#![warn(clippy::all, clippy::nursery)]

use std::time::Duration;

use opentelemetry::KeyValue;
use tracing::{info, info_span};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _guard = shared::tracing::init_with_file("info", "otel-test");
    info!(git_hash = env!("BUILD_GIT_HASH"), "otel-test starting");

    // --- traces ---
    let span = info_span!("otel_test_root", test_run = %uuid::Uuid::new_v4());
    let _enter = span.enter();

    info!("otel-test: sending sample spans");

    for i in 0..3 {
        let child = info_span!("sample_operation", iteration = i);
        let _enter = child.enter();
        info!("processing iteration {i}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    info!("otel-test: all spans sent");

    // --- metrics ---
    info!("otel-test: sending sample r2.operations metric");

    let meter = opentelemetry::global::meter("r2");
    let counter = meter
        .u64_counter("r2.operations")
        .with_description("Count of R2 object store operations by type and CLI")
        .with_unit("operations")
        .build();

    for (operation, r2_class) in [
        ("get", "B"),
        ("head", "B"),
        ("put", "A"),
        ("list", "A"),
        ("delete", "A"),
    ] {
        counter.add(
            1,
            &[
                KeyValue::new("operation", operation),
                KeyValue::new("r2_class", r2_class),
                KeyValue::new("cli", "otel-test"),
                KeyValue::new("store_prefix", "test://"),
            ],
        );
    }

    // wait for the periodic reader to export; default interval is 60s so force a flush
    // by dropping the guard (which calls shutdown on the provider)
    info!("otel-test: metrics recorded, shutting down (will flush on drop)");
    Ok(())
}
