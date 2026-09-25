use crate::infrai_client::{InfraiClient, InfraiError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofOfDelivery {
    pub object_key: String,
    pub media_type: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShipmentEvent {
    LabelCreated,
    InTransit { facility: String },
    Delivered { proof: ProofOfDelivery },
    Exception { code: String, detail: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShipmentDisposition {
    Tracking,
    DeliveredWithProof(String),
    ManualReview { code: String },
}

pub fn disposition(events: &[ShipmentEvent]) -> ShipmentDisposition {
    if let Some(code) = events.iter().rev().find_map(|event| match event {
        ShipmentEvent::Exception { code, .. } => Some(code.clone()),
        _ => None,
    }) {
        return ShipmentDisposition::ManualReview { code };
    }
    events
        .iter()
        .rev()
        .find_map(|event| match event {
            ShipmentEvent::Delivered { proof } => Some(ShipmentDisposition::DeliveredWithProof(
                proof.object_key.clone(),
            )),
            _ => None,
        })
        .unwrap_or(ShipmentDisposition::Tracking)
}

#[derive(Debug, Clone)]
pub struct SignupRequest {
    pub email: String,
    pub password: String,
    pub name: String,
    pub shipment_id: String,
    pub verification_origin: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignupReceipt {
    pub user_id: String,
    pub message_id: String,
    pub shipment_id: String,
}

#[derive(Debug)]
pub enum SignupError {
    InvalidInput(&'static str),
    Infrai(InfraiError),
}

impl From<InfraiError> for SignupError {
    fn from(value: InfraiError) -> Self {
        Self::Infrai(value)
    }
}

impl std::fmt::Display for SignupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(message) => write!(f, "invalid signup: {message}"),
            Self::Infrai(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for SignupError {}

pub async fn register_shipment_contact(
    client: &InfraiClient,
    request: SignupRequest,
) -> Result<SignupReceipt, SignupError> {
    if !request.email.contains('@') {
        return Err(SignupError::InvalidInput("email must contain @"));
    }
    if request.shipment_id.trim().is_empty() {
        return Err(SignupError::InvalidInput("shipment_id is required"));
    }
    let signup_key = format!("shipment-signup:{}:{}", request.shipment_id, request.email);
    let metadata = format!(
        "{{\"shipment_id\":\"{}\"}}",
        escape_json(&request.shipment_id)
    );
    let user_id = client
        .create_user(
            &request.email,
            &request.password,
            &request.name,
            &metadata,
            &signup_key,
        )
        .await?;

    let link = format!(
        "{}/verify?user_id={}&shipment_id={}",
        request.verification_origin.trim_end_matches('/'),
        user_id,
        request.shipment_id
    );
    let html = format!("<p>Confirm tracking access for shipment <strong>{}</strong>.</p><p><a href=\"{}\">Verify email</a></p>", request.shipment_id, link);
    let mail_key = format!("shipment-verification:{user_id}");
    let email_result = client
        .send_verification_email(
            &request.email,
            "Verify shipment tracking email",
            &html,
            &mail_key,
        )
        .await;
    let cleanup_result = client.delete_user(&user_id).await;
    let message_id = email_result?;
    cleanup_result?;
    Ok(SignupReceipt {
        user_id,
        message_id,
        shipment_id: request.shipment_id,
    })
}

fn escape_json(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exception_routes_a_delivered_shipment_to_manual_review() {
        let events = vec![
            ShipmentEvent::Delivered {
                proof: ProofOfDelivery {
                    object_key: "pod/SHP-2048/photo.jpg".into(),
                    media_type: "image/jpeg".into(),
                    sha256: "9f86d081884c7d659a2feaa0c55ad015".into(),
                },
            },
            ShipmentEvent::Exception {
                code: "RECIPIENT_DISPUTE".into(),
                detail: "recipient disputed signature".into(),
            },
        ];
        assert_eq!(
            disposition(&events),
            ShipmentDisposition::ManualReview {
                code: "RECIPIENT_DISPUTE".into()
            }
        );
    }
}
