#![warn(clippy::all, clippy::nursery)]

use opentelemetry::{
    KeyValue,
    metrics::{Counter, Gauge, Meter},
};

pub struct SyncMetrics {
    run_total: Counter<u64>,
    datapoints_fetched: Gauge<u64>,
}

impl SyncMetrics {
    pub fn new(meter: &Meter) -> Self {
        Self {
            run_total: meter
                .u64_counter("cloudflare_exporter_run_total")
                .with_description("Cumulative sync runs by outcome")
                .with_unit("runs")
                .build(),
            datapoints_fetched: meter
                .u64_gauge("cloudflare_exporter_datapoints_fetched")
                .with_description("Number of Cloudflare datapoints fetched in this run")
                .with_unit("datapoints")
                .build(),
        }
    }

    pub fn record_success(&self, datapoints: u64) {
        self.run_total.add(1, &[KeyValue::new("status", "success")]);
        self.datapoints_fetched.record(datapoints, &[]);
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
        let metrics = SyncMetrics::new(&provider.meter("cloudflare_exporter"));
        (metrics, provider, exporter)
    }

    #[test]
    fn success_increments_run_counter_and_records_datapoints() {
        let (metrics, provider, exporter) = make_test_metrics();
        metrics.record_success(42);
        assert_eq!(
            sum_counter(
                &provider,
                &exporter,
                "cloudflare_exporter_run_total",
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
                "cloudflare_exporter_run_total",
                Some(("status", "failure"))
            ),
            1
        );
    }

    #[test]
    fn datapoints_gauge_records_count() {
        let (metrics, provider, exporter) = make_test_metrics();
        metrics.record_success(7);
        assert_eq!(
            last_gauge_u64(
                &provider,
                &exporter,
                "cloudflare_exporter_datapoints_fetched",
                None
            ),
            7
        );
    }
}
