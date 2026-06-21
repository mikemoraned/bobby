use opentelemetry::{KeyValue, metrics::Gauge};

pub struct StoreMetrics {
    fragments: Gauge<u64>,
    version: Gauge<u64>,
}

impl StoreMetrics {
    pub fn new(meter: opentelemetry::metrics::Meter) -> Self {
        Self {
            fragments: meter
                .u64_gauge("lance.table.fragments")
                .with_description("Number of fragments per lance table")
                .with_unit("fragments")
                .build(),
            version: meter
                .u64_gauge("lance.table.version")
                .with_description("Current version counter per lance table")
                .build(),
        }
    }

    pub fn record_fragment_counts(&self, counts: &[(&str, u64)]) {
        for (table, count) in counts {
            self.fragments
                .record(*count, &[KeyValue::new("table", table.to_string())]);
        }
    }

    pub fn record_table_versions(&self, versions: &[(&str, u64)]) {
        for (table, version) in versions {
            self.version
                .record(*version, &[KeyValue::new("table", table.to_string())]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::metrics::MeterProvider;
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, SdkMeterProvider};
    use test_support::flush_and_collect;

    fn make_test_metrics() -> (StoreMetrics, SdkMeterProvider, InMemoryMetricExporter) {
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_periodic_exporter(exporter.clone())
            .build();
        let meter = provider.meter("lance");
        let metrics = StoreMetrics::new(meter);
        (metrics, provider, exporter)
    }

    #[test]
    fn record_fragment_counts_emits_gauge_per_table() {
        let (metrics, provider, exporter) = make_test_metrics();
        metrics.record_fragment_counts(&[("images_v6", 64), ("images_score_v2", 4)]);
        let snap = flush_and_collect(&provider, &exporter);
        assert_eq!(
            snap.last_gauge_u64("lance.table.fragments", Some(("table", "images_v6"))),
            64
        );
        assert_eq!(
            snap.last_gauge_u64("lance.table.fragments", Some(("table", "images_score_v2"))),
            4
        );
    }

    #[test]
    fn record_table_versions_emits_gauge_per_table() {
        let (metrics, provider, exporter) = make_test_metrics();
        metrics.record_table_versions(&[("images_v6", 42), ("images_score_v2", 7)]);
        let snap = flush_and_collect(&provider, &exporter);
        assert_eq!(
            snap.last_gauge_u64("lance.table.version", Some(("table", "images_v6"))),
            42
        );
        assert_eq!(
            snap.last_gauge_u64("lance.table.version", Some(("table", "images_score_v2"))),
            7
        );
    }
}
