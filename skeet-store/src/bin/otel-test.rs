#![warn(clippy::all, clippy::nursery)]

use std::time::Duration;

use tracing::{info, info_span};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _guard = shared::tracing::init_with_file("info", "otel-test");

    let span = info_span!("otel_test_root", test_run = %uuid::Uuid::new_v4());
    let _enter = span.enter();

    info!("otel-test: sending sample spans");

    for i in 0..3 {
        let child = info_span!("sample_operation", iteration = i);
        let _enter = child.enter();
        info!("processing iteration {i}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    info!("otel-test: all spans sent, shutting down");
    Ok(())
}
