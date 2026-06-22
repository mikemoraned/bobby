use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Gauge, Meter};

/// OTel metrics emitted by `skeet-publish` each publish cycle.
pub struct PublishMetrics {
    cycles: Counter<u64>,
    list_size: Gauge<u64>,
}

impl PublishMetrics {
    pub fn new(meter: &Meter) -> Self {
        Self {
            cycles: meter
                .u64_counter("skeet_publish.cycles")
                .with_description("Publish cycles, by outcome")
                .build(),
            list_size: meter
                .u64_gauge("skeet_publish.list_size")
                .with_description("Pairs in the most recently published list")
                .with_unit("pairs")
                .build(),
        }
    }

    pub fn record_published(&self) {
        self.cycles.add(1, &[KeyValue::new("outcome", "published")]);
    }

    pub fn record_unchanged(&self) {
        self.cycles.add(1, &[KeyValue::new("outcome", "unchanged")]);
    }

    pub fn record_failed(&self) {
        self.cycles.add(1, &[KeyValue::new("outcome", "failed")]);
    }

    /// Record the size of a published list (one observation per list name).
    pub fn record_list_size(&self, list: &str, size: u64) {
        self.list_size
            .record(size, &[KeyValue::new("list", list.to_string())]);
    }
}
