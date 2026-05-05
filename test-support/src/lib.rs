use opentelemetry_sdk::metrics::{
    InMemoryMetricExporter, SdkMeterProvider,
    data::{AggregatedMetrics, MetricData},
};

/// A flushed snapshot of OTel metrics that can be queried multiple times.
pub struct Snapshot(Vec<opentelemetry_sdk::metrics::data::ResourceMetrics>);

impl Snapshot {
    fn named(&self, metric: &str) -> impl Iterator<Item = &opentelemetry_sdk::metrics::data::Metric> {
        self.0
            .iter()
            .flat_map(|rm| rm.scope_metrics())
            .flat_map(|sm| sm.metrics())
            .filter(move |m| m.name() == metric)
    }

    pub fn sum_counter(&self, metric: &str, attr: Option<(&str, &str)>) -> u64 {
        self.named(metric)
            .flat_map(|m| {
                if let AggregatedMetrics::U64(MetricData::Sum(s)) = m.data() {
                    s.data_points()
                        .filter(|dp| {
                            attr.is_none_or(|(k, v)| {
                                dp.attributes().any(|kv| kv.key.as_str() == k && kv.value.as_str() == v)
                            })
                        })
                        .map(|dp| dp.value())
                        .collect::<Vec<_>>()
                } else {
                    vec![]
                }
            })
            .sum()
    }

    pub fn last_gauge_u64(&self, metric: &str, attr: Option<(&str, &str)>) -> u64 {
        self.named(metric)
            .flat_map(|m| {
                if let AggregatedMetrics::U64(MetricData::Gauge(g)) = m.data() {
                    g.data_points()
                        .filter(|dp| {
                            attr.is_none_or(|(k, v)| {
                                dp.attributes().any(|kv| kv.key.as_str() == k && kv.value.as_str() == v)
                            })
                        })
                        .map(|dp| dp.value())
                        .collect::<Vec<_>>()
                } else {
                    vec![]
                }
            })
            .sum()
    }

    pub fn histogram_observation_count(&self, metric: &str, attr: Option<(&str, &str)>) -> u64 {
        self.named(metric)
            .flat_map(|m| match m.data() {
                AggregatedMetrics::U64(MetricData::Histogram(h)) => h
                    .data_points()
                    .filter(|dp| {
                        attr.is_none_or(|(k, v)| {
                            dp.attributes().any(|kv| kv.key.as_str() == k && kv.value.as_str() == v)
                        })
                    })
                    .map(|dp| dp.count())
                    .collect::<Vec<_>>(),
                AggregatedMetrics::F64(MetricData::Histogram(h)) => h
                    .data_points()
                    .filter(|dp| {
                        attr.is_none_or(|(k, v)| {
                            dp.attributes().any(|kv| kv.key.as_str() == k && kv.value.as_str() == v)
                        })
                    })
                    .map(|dp| dp.count())
                    .collect::<Vec<_>>(),
                _ => vec![],
            })
            .sum()
    }
}

/// Flush the provider, collect all emitted metrics, reset the exporter, and return a
/// queryable snapshot. Use this when a single emit needs multiple assertions.
///
/// # Panics
///
/// Panics if `force_flush` or `get_finished_metrics` fails — test helper, panicking on
/// failure is correct behaviour in tests.
#[allow(clippy::unwrap_used)]
pub fn flush_and_collect(
    provider: &SdkMeterProvider,
    exporter: &InMemoryMetricExporter,
) -> Snapshot {
    provider.force_flush().unwrap();
    let metrics = exporter.get_finished_metrics().unwrap();
    exporter.reset();
    Snapshot(metrics)
}

/// Convenience: flush, query a single u64 counter value, and reset.
pub fn sum_counter(
    provider: &SdkMeterProvider,
    exporter: &InMemoryMetricExporter,
    metric: &str,
    attr: Option<(&str, &str)>,
) -> u64 {
    flush_and_collect(provider, exporter).sum_counter(metric, attr)
}

/// Convenience: flush, query a single u64 gauge value, and reset.
pub fn last_gauge_u64(
    provider: &SdkMeterProvider,
    exporter: &InMemoryMetricExporter,
    metric: &str,
    attr: Option<(&str, &str)>,
) -> u64 {
    flush_and_collect(provider, exporter).last_gauge_u64(metric, attr)
}

/// Convenience: flush, query histogram observation count, and reset.
pub fn histogram_observation_count(
    provider: &SdkMeterProvider,
    exporter: &InMemoryMetricExporter,
    metric: &str,
    attr: Option<(&str, &str)>,
) -> u64 {
    flush_and_collect(provider, exporter).histogram_observation_count(metric, attr)
}
