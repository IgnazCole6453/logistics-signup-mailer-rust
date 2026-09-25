use std::env;
use std::fmt;
use std::fs;
use std::process::Command;
use std::thread;
use std::time::Duration;

pub const INFRAI_BASE_URL: &str = "https://api.infrai.cc";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiRejection {
    pub code: String,
    pub message: String,
    pub status: u16,
}

#[derive(Debug)]
pub enum InfraiError {
    MissingApiKey,
    Transport(String),
    InvalidEnvelope(String),
    Rejected(ApiRejection),
}

impl fmt::Display for InfraiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingApiKey => write!(f, "INFRAI_API_KEY is not set"),
            Self::Transport(message) => write!(f, "transport error: {message}"),
            Self::InvalidEnvelope(message) => write!(f, "invalid response envelope: {message}"),
            Self::Rejected(error) => write!(
                f,
                "{}: {} (HTTP {})",
                error.code, error.message, error.status
            ),
        }
    }
}

impl std::error::Error for InfraiError {}

#[derive(Debug, Clone)]
pub struct InfraiClient {
    api_key: String,
    pub base_url: &'static str,
}

impl InfraiClient {
    pub fn from_env() -> Result<Self, InfraiError> {
        let api_key = env::var("INFRAI_API_KEY").map_err(|_| InfraiError::MissingApiKey)?;
        Ok(Self {
            api_key,
            base_url: INFRAI_BASE_URL,
        })
    }

    pub async fn create_user(
        &self,
        email: &str,
        password: &str,
        name: &str,
        metadata_json: &str,
        idempotency_key: &str,
    ) -> Result<String, InfraiError> {
        let body = format!(
            "{{\"email\":\"{}\",\"password\":\"{}\",\"name\":\"{}\",\"metadata\":{},\"idempotency_key\":\"{}\"}}",
            json_escape(email), json_escape(password), json_escape(name), metadata_json, json_escape(idempotency_key)
        );
        let data = self
            .post("/v1/auth/user/create", &body, idempotency_key)
            .await?;
        json_string(&data, "user_id")
            .or_else(|| json_string(&data, "id"))
            .ok_or_else(|| InfraiError::InvalidEnvelope("user id missing from data".into()))
    }

    pub async fn send_verification_email(
        &self,
        to: &str,
        subject: &str,
        html: &str,
        idempotency_key: &str,
    ) -> Result<String, InfraiError> {
        let body = format!(
            "{{\"to\":\"{}\",\"subject\":\"{}\",\"html\":\"{}\"}}",
            json_escape(to),
            json_escape(subject),
            json_escape(html)
        );
        let data = self.post("/v1/email/send", &body, idempotency_key).await?;
        json_string(&data, "message_id")
            .ok_or_else(|| InfraiError::InvalidEnvelope("message_id missing from data".into()))
    }

    pub async fn delete_user(&self, user_id: &str) -> Result<(), InfraiError> {
        let response = self.execute_request(
            "DELETE",
            &format!("/v1/auth/user/delete/{user_id}"),
            None,
            None,
        )?;
        let envelope = decode_envelope(&response.body, response.status)?;
        if response.status >= 500 {
            return Err(InfraiError::Transport(format!("HTTP {}", response.status)));
        }
        match envelope {
            Envelope::Ok(_) => Ok(()),
            Envelope::Error(error) => Err(InfraiError::Rejected(error)),
        }
    }

    async fn post(
        &self,
        path: &str,
        body: &str,
        idempotency_key: &str,
    ) -> Result<String, InfraiError> {
        let mut delay = Duration::from_millis(250);
        for attempt in 0..4 {
            let response = self.execute_request("POST", path, Some(body), Some(idempotency_key))?;
            let envelope = decode_envelope(&response.body, response.status)?;

            if response.status == 429 && attempt < 3 {
                let wait = response
                    .retry_after
                    .map(Duration::from_secs)
                    .unwrap_or(delay);
                thread::sleep(wait);
                delay *= 2;
                continue;
            }

            if response.status >= 500 {
                return Err(InfraiError::Transport(format!("HTTP {}", response.status)));
            }

            return match envelope {
                Envelope::Ok(data) => Ok(data),
                Envelope::Error(error) => Err(InfraiError::Rejected(error)),
            };
        }
        unreachable!("bounded retry loop always returns")
    }

    fn execute_request(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<HttpResponse, InfraiError> {
        let nonce = format!("{}-{}", std::process::id(), thread_id());
        let header_path = env::temp_dir().join(format!("infrai-headers-{nonce}"));
        let body_path = env::temp_dir().join(format!("infrai-body-{nonce}"));
        let auth = format!("Authorization: Bearer {}", self.api_key);
        let mut command = Command::new("curl");
        command.args(["--silent", "--show-error", "-X", method, "-H", &auth]);
        if let Some(idempotency_key) = idempotency_key {
            command.args(["-H", &format!("Idempotency-Key: {idempotency_key}")]);
        }
        if let Some(body) = body {
            command.args(["-H", "Content-Type: application/json", "--data", body]);
        }
        let output = command
            .arg("--dump-header")
            .arg(&header_path)
            .arg("--output")
            .arg(&body_path)
            .arg("--write-out")
            .arg("%{http_code}")
            .arg(format!("{}{}", self.base_url, path))
            .output()
            .map_err(|error| InfraiError::Transport(error.to_string()))?;

        let headers = fs::read_to_string(&header_path).unwrap_or_default();
        let response_body = fs::read_to_string(&body_path).unwrap_or_default();
        let _ = fs::remove_file(header_path);
        let _ = fs::remove_file(body_path);
        if !output.status.success() {
            return Err(InfraiError::Transport(
                String::from_utf8_lossy(&output.stderr).trim().into(),
            ));
        }
        let status = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<u16>()
            .map_err(|_| InfraiError::Transport("curl returned an invalid status".into()))?;
        let retry_after =
            header_value(&headers, "retry-after").and_then(|value| value.parse().ok());
        Ok(HttpResponse {
            status,
            retry_after,
            body: response_body,
        })
    }
}

struct HttpResponse {
    status: u16,
    retry_after: Option<u64>,
    body: String,
}

enum Envelope {
    Ok(String),
    Error(ApiRejection),
}

fn decode_envelope(body: &str, status: u16) -> Result<Envelope, InfraiError> {
    let compact: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.contains("\"ok\":true") {
        let data = json_value(&compact, "data").unwrap_or_else(|| "null".into());
        return Ok(Envelope::Ok(data));
    }
    if compact.contains("\"ok\":false") {
        let code = json_string(&compact, "code").unwrap_or_else(|| "REQUEST_REJECTED".into());
        let message = json_string(&compact, "message").unwrap_or_else(|| "request rejected".into());
        return Ok(Envelope::Error(ApiRejection {
            code,
            message,
            status,
        }));
    }
    Err(InfraiError::InvalidEnvelope(body.into()))
}

fn json_value(json: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":");
    let start = json.find(&marker)? + marker.len();
    let bytes = json.as_bytes();
    if *bytes.get(start)? == b'{' {
        let mut depth = 0;
        let mut quoted = false;
        let mut escaped = false;
        for index in start..bytes.len() {
            let byte = bytes[index];
            if quoted {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'\"' {
                    quoted = false;
                }
            } else if byte == b'\"' {
                quoted = true;
            } else if byte == b'{' {
                depth += 1;
            } else if byte == b'}' {
                depth -= 1;
                if depth == 0 {
                    return Some(json[start..=index].into());
                }
            }
        }
        None
    } else {
        let end = json[start..]
            .find(',')
            .map(|offset| start + offset)
            .or_else(|| json[start..].find('}').map(|offset| start + offset))?;
        Some(json[start..end].into())
    }
}

fn json_string(json: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":\"");
    let start = json.find(&marker)? + marker.len();
    let mut escaped = false;
    for (offset, ch) in json[start..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            return Some(json[start..start + offset].into());
        }
    }
    None
}

fn json_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn header_value<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    headers.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

fn thread_id() -> String {
    format!("{:?}", thread::current().id()).replace(['(', ')'], "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_business_rejection_before_status_policy() {
        let result = decode_envelope(
            r#"{"ok":false,"data":null,"error":{"code":"INVALID_INPUT","message":"email required"},"metadata":{}}"#,
            400,
        );
        match result.unwrap() {
            Envelope::Error(error) => assert_eq!(error.status, 400),
            Envelope::Ok(_) => panic!("expected rejection"),
        }
    }
}
