use std::sync::Arc;

use image::{DynamicImage, ImageBuffer, Rgb};
use shared::refine_model::{ModelName, ModelProvider, RefinePrompt};
use shared::{RefineModel, RefineModels, Score, Threshold};

/// Build a 1-pixel-tall RGB image whose width encodes `marker + 1`, so the
/// caller can later recover `marker` from the image alone via `marker_of`.
/// Useful for asynchronous tests where requests and responses need to be
/// paired without sharing other state.
pub fn marker_image(marker: u32) -> DynamicImage {
    let width = marker + 1;
    DynamicImage::ImageRgb8(ImageBuffer::from_pixel(width, 1, Rgb([0u8, 0, 0])))
}

/// Recover the marker an image was built with via `marker_image`.
pub fn marker_of(image: &DynamicImage) -> u32 {
    image.width() - 1
}

/// Deterministic `Score` for a marker — `marker / 100`, clamped by the
/// `0u32..=100` range of valid markers. Pair with `marker_image` to let an
/// async scoring stub return a score the caller can independently predict.
pub fn score_for(marker: u32) -> Score {
    Score::new(marker as f32 / 100.0).expect("marker in 0..=100 yields valid score")
}

/// A `RefineModels` registry for use in tests, containing a single entry
/// keyed by `"test"` (a synthetic version string, not a real hash) that
/// accepts any score ≥ 0.5.
pub fn test_models() -> Arc<RefineModels> {
    let mut models = RefineModels::new();
    models.insert_unverified(
        "test",
        RefineModel {
            model_provider: ModelProvider::openai(),
            model_name: ModelName::gpt_4o(),
            prompt: RefinePrompt::new("test prompt"),
            decision_threshold: Threshold::new(0.5).expect("valid"),
        },
    );
    Arc::new(models)
}

use opentelemetry_sdk::metrics::{
    InMemoryMetricExporter, SdkMeterProvider,
    data::{AggregatedMetrics, MetricData},
};

/// A flushed snapshot of OTel metrics that can be queried multiple times.
pub struct Snapshot(Vec<opentelemetry_sdk::metrics::data::ResourceMetrics>);

impl Snapshot {
    fn named(
        &self,
        metric: &str,
    ) -> impl Iterator<Item = &opentelemetry_sdk::metrics::data::Metric> {
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
                                dp.attributes()
                                    .any(|kv| kv.key.as_str() == k && kv.value.as_str() == v)
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
                                dp.attributes()
                                    .any(|kv| kv.key.as_str() == k && kv.value.as_str() == v)
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
                            dp.attributes()
                                .any(|kv| kv.key.as_str() == k && kv.value.as_str() == v)
                        })
                    })
                    .map(|dp| dp.count())
                    .collect::<Vec<_>>(),
                AggregatedMetrics::F64(MetricData::Histogram(h)) => h
                    .data_points()
                    .filter(|dp| {
                        attr.is_none_or(|(k, v)| {
                            dp.attributes()
                                .any(|kv| kv.key.as_str() == k && kv.value.as_str() == v)
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
