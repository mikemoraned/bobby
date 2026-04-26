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
    use opentelemetry_sdk::metrics::{
        InMemoryMetricExporter, SdkMeterProvider,
        data::{AggregatedMetrics, MetricData},
    };

    fn make_test_metrics() -> (StoreMetrics, SdkMeterProvider, InMemoryMetricExporter) {
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_periodic_exporter(exporter.clone())
            .build();
        let meter = provider.meter("lance");
        let metrics = StoreMetrics::new(meter);
        (metrics, provider, exporter)
    }

    fn gauge_values_for(
        metric_name: &str,
        exporter: &InMemoryMetricExporter,
        provider: &SdkMeterProvider,
    ) -> Vec<(String, u64)> {
        provider.force_flush().unwrap();
        let metrics = exporter.get_finished_metrics().unwrap();
        metrics
            .iter()
            .flat_map(|rm| rm.scope_metrics())
            .flat_map(|sm| sm.metrics())
            .filter(|m| m.name() == metric_name)
            .flat_map(|m| {
                if let AggregatedMetrics::U64(MetricData::Gauge(g)) = m.data() {
                    g.data_points()
                        .map(|dp| {
                            let table = dp
                                .attributes()
                                .find(|kv| kv.key.as_str() == "table")
                                .map(|kv| kv.value.as_str().to_string())
                                .unwrap_or_default();
                            (table, dp.value())
                        })
                        .collect::<Vec<_>>()
                } else {
                    vec![]
                }
            })
            .collect()
    }

    #[test]
    fn record_fragment_counts_emits_gauge_per_table() {
        let (metrics, provider, exporter) = make_test_metrics();

        metrics.record_fragment_counts(&[("images_v6", 64), ("images_score_v2", 4)]);

        let values = gauge_values_for("lance.table.fragments", &exporter, &provider);
        assert!(
            values.contains(&("images_v6".to_string(), 64)),
            "expected images_v6=64, got {values:?}"
        );
        assert!(
            values.contains(&("images_score_v2".to_string(), 4)),
            "expected images_score_v2=4, got {values:?}"
        );
    }

    #[test]
    fn record_table_versions_emits_gauge_per_table() {
        let (metrics, provider, exporter) = make_test_metrics();

        metrics.record_table_versions(&[("images_v6", 42), ("images_score_v2", 7)]);

        let values = gauge_values_for("lance.table.version", &exporter, &provider);
        assert!(
            values.contains(&("images_v6".to_string(), 42)),
            "expected images_v6=42, got {values:?}"
        );
        assert!(
            values.contains(&("images_score_v2".to_string(), 7)),
            "expected images_score_v2=7, got {values:?}"
        );
    }
}
