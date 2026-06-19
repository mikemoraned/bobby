#![warn(clippy::all, clippy::nursery)]

use opentelemetry::{
    KeyValue,
    metrics::{Counter, Gauge, Meter},
};

pub struct SyncMetrics {
    run_total: Counter<u64>,
    entries_pushed: Gauge<u64>,
}

impl SyncMetrics {
    pub fn new(meter: &Meter) -> Self {
        Self {
            run_total: meter
                .u64_counter("openai_exporter_run_total")
                .with_description("Cumulative sync runs by outcome")
                .with_unit("runs")
                .build(),
            entries_pushed: meter
                .u64_gauge("openai_exporter_entries_pushed")
                .with_description("Number of cost entries pushed in this run")
                .with_unit("entries")
                .build(),
        }
    }

    pub fn record_success(&self, entries: u64) {
        self.run_total.add(1, &[KeyValue::new("status", "success")]);
        self.entries_pushed.record(entries, &[]);
    }

    pub fn record_failure(&self) {
        self.run_total.add(1, &[KeyValue::new("status", "failure")]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::metrics::MeterProvider;
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, SdkMeterProvider};
    use test_support::{last_gauge_u64, sum_counter};

    fn make_test_metrics() -> (SyncMetrics, SdkMeterProvider, InMemoryMetricExporter) {
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_periodic_exporter(exporter.clone())
            .build();
        let metrics = SyncMetrics::new(&provider.meter("openai_exporter"));
        (metrics, provider, exporter)
    }

    #[test]
    fn success_increments_run_counter_and_records_entries() {
        let (metrics, provider, exporter) = make_test_metrics();
        metrics.record_success(4);
        assert_eq!(
            sum_counter(
                &provider,
                &exporter,
                "openai_exporter_run_total",
                Some(("status", "success"))
            ),
            1
        );
    }

    #[test]
    fn failure_increments_failure_counter() {
        let (metrics, provider, exporter) = make_test_metrics();
        metrics.record_failure();
        assert_eq!(
            sum_counter(
                &provider,
                &exporter,
                "openai_exporter_run_total",
                Some(("status", "failure"))
            ),
            1
        );
    }

    #[test]
    fn entries_gauge_records_count() {
        let (metrics, provider, exporter) = make_test_metrics();
        metrics.record_success(4);
        assert_eq!(
            last_gauge_u64(&provider, &exporter, "openai_exporter_entries_pushed", None),
            4
        );
    }
}
