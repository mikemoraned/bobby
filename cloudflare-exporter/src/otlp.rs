use opentelemetry::{
    KeyValue,
    metrics::{Counter, Gauge},
};

use crate::cloudflare::R2Metrics;

pub struct CloudflareMetrics {
    operations: Counter<u64>,
    storage_bytes: Gauge<u64>,
    storage_objects: Gauge<u64>,
}

impl CloudflareMetrics {
    pub fn new(meter: opentelemetry::metrics::Meter) -> Self {
        Self {
            operations: meter
                .u64_counter("cloudflare.r2.operations")
                .with_description("R2 operation request count from Cloudflare Analytics API")
                .with_unit("requests")
                .build(),
            storage_bytes: meter
                .u64_gauge("cloudflare.r2.storage.bytes")
                .with_description("R2 stored payload size from Cloudflare Analytics API")
                .with_unit("bytes")
                .build(),
            storage_objects: meter
                .u64_gauge("cloudflare.r2.storage.objects")
                .with_description("R2 object count from Cloudflare Analytics API")
                .with_unit("objects")
                .build(),
        }
    }

    pub fn record(&self, metrics: &R2Metrics) {
        for op in &metrics.operations {
            self.operations.add(
                op.requests,
                &[
                    KeyValue::new("bucket", op.bucket_name.clone()),
                    KeyValue::new("action_type", op.action_type.clone()),
                ],
            );
        }
        for s in &metrics.storage {
            let bucket = KeyValue::new("bucket", s.bucket_name.clone());
            self.storage_bytes
                .record(s.payload_size, std::slice::from_ref(&bucket));
            self.storage_objects
                .record(s.object_count, std::slice::from_ref(&bucket));
        }
    }
}
