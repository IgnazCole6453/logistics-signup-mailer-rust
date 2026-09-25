use logistics_signup_mailer::infrai_client::InfraiClient;
use logistics_signup_mailer::shipment_signup::{register_shipment_contact, SignupRequest};
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Wake, Waker};

fn main() {
    if let Err(error) = block_on(run()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let request = SignupRequest {
        email: args
            .next()
            .ok_or("usage: shipment-signup EMAIL PASSWORD NAME SHIPMENT_ID VERIFY_ORIGIN")?,
        password: args.next().ok_or("PASSWORD is required")?,
        name: args.next().ok_or("NAME is required")?,
        shipment_id: args.next().ok_or("SHIPMENT_ID is required")?,
        verification_origin: args.next().ok_or("VERIFY_ORIGIN is required")?,
    };
    let receipt = register_shipment_contact(&InfraiClient::from_env()?, request).await?;
    println!(
        "user_id={} message_id={} shipment_id={}",
        receipt.user_id, receipt.message_id, receipt.shipment_id
    );
    Ok(())
}

fn block_on<F: Future>(future: F) -> F::Output {
    struct Noop;
    impl Wake for Noop {
        fn wake(self: std::sync::Arc<Self>) {}
    }
    let waker = Waker::from(std::sync::Arc::new(Noop));
    let mut context = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}
