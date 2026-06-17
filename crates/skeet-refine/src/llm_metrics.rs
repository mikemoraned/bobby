// GenAI semconv is in Development status; opt-in via OTEL_SEMCONV_STABILITY_OPT_IN=gen_ai_latest_experimental.
// Names may shift as the spec evolves.
//
// `gen_ai.provider.name` has no constant in opentelemetry-semantic-conventions 0.31 — that crate
// still uses the older `GEN_AI_SYSTEM` name. The rename landed after 0.31; use a string literal
// until the crate catches up.
use std::time::Duration;

use opentelemetry::{
    KeyValue,
    metrics::{Histogram, Meter},
};
use opentelemetry_semantic_conventions::{
    attribute::{ERROR_TYPE, GEN_AI_OPERATION_NAME, GEN_AI_REQUEST_MODEL, GEN_AI_TOKEN_TYPE},
    metric::{GEN_AI_CLIENT_OPERATION_DURATION, GEN_AI_CLIENT_TOKEN_USAGE},
};
use rig::completion::Usage;

const GEN_AI_PROVIDER_NAME: &str = "gen_ai.provider.name";

pub struct LlmMetrics {
    token_usage: Histogram<u64>,
    operation_duration: Histogram<f64>,
}

impl LlmMetrics {
    pub fn new(meter: &Meter) -> Self {
        Self {
            token_usage: meter
                .u64_histogram(GEN_AI_CLIENT_TOKEN_USAGE)
                .with_description("Token counts for GenAI completion requests")
                .with_unit("token")
                .with_boundaries(vec![
                    1.0, 4.0, 16.0, 64.0, 256.0, 1024.0, 4096.0, 16384.0, 65536.0, 262144.0,
                    1048576.0, 4194304.0, 16777216.0, 67108864.0,
                ])
                .build(),
            operation_duration: meter
                .f64_histogram(GEN_AI_CLIENT_OPERATION_DURATION)
                .with_description("Duration of GenAI completion operations")
                .with_unit("s")
                .with_boundaries(vec![
                    0.01, 0.02, 0.04, 0.08, 0.16, 0.32, 0.64, 1.28, 2.56, 5.12, 10.24, 20.48,
                    40.96, 81.92,
                ])
                .build(),
        }
    }

    pub fn record_success(&self, usage: &Usage, duration: Duration, model: &str) {
        let base = [
            KeyValue::new(GEN_AI_PROVIDER_NAME, "openai"),
            KeyValue::new(GEN_AI_REQUEST_MODEL, model.to_string()),
            KeyValue::new(GEN_AI_OPERATION_NAME, "chat"),
        ];
        self.token_usage.record(
            usage.input_tokens,
            &[
                base[0].clone(),
                base[1].clone(),
                base[2].clone(),
                KeyValue::new(GEN_AI_TOKEN_TYPE, "input"),
            ],
        );
        self.token_usage.record(
            usage.output_tokens,
            &[
                base[0].clone(),
                base[1].clone(),
                base[2].clone(),
                KeyValue::new(GEN_AI_TOKEN_TYPE, "output"),
            ],
        );
        self.operation_duration.record(duration.as_secs_f64(), &base);
    }

    pub fn record_error(&self, duration: Duration, error_type: &str, model: &str) {
        self.operation_duration.record(
            duration.as_secs_f64(),
            &[
                KeyValue::new(GEN_AI_PROVIDER_NAME, "openai"),
                KeyValue::new(GEN_AI_REQUEST_MODEL, model.to_string()),
                KeyValue::new(GEN_AI_OPERATION_NAME, "chat"),
                KeyValue::new(ERROR_TYPE, error_type.to_string()),
            ],
        );
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

    fn make_provider() -> (LlmMetrics, SdkMeterProvider, InMemoryMetricExporter) {
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_periodic_exporter(exporter.clone())
            .build();
        let metrics = LlmMetrics::new(&provider.meter("gen_ai"));
        (metrics, provider, exporter)
    }

    fn histogram_count(
        provider: &SdkMeterProvider,
        exporter: &InMemoryMetricExporter,
        name: &str,
        attr: Option<(&str, &str)>,
    ) -> u64 {
        provider.force_flush().unwrap();
        let finished = exporter.get_finished_metrics().unwrap();
        let mut count = 0;
        for rm in &finished {
            for sm in rm.scope_metrics() {
                for m in sm.metrics() {
                    if m.name() != name {
                        continue;
                    }
                    match m.data() {
                        AggregatedMetrics::U64(MetricData::Histogram(h)) => {
                            for dp in h.data_points() {
                                let matches = attr.is_none_or(|(k, v)| {
                                    dp.attributes()
                                        .any(|kv| kv.key.as_str() == k && kv.value.as_str() == v)
                                });
                                if matches {
                                    count += dp.count();
                                }
                            }
                        }
                        AggregatedMetrics::F64(MetricData::Histogram(h)) => {
                            for dp in h.data_points() {
                                let matches = attr.is_none_or(|(k, v)| {
                                    dp.attributes()
                                        .any(|kv| kv.key.as_str() == k && kv.value.as_str() == v)
                                });
                                if matches {
                                    count += dp.count();
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        exporter.reset();
        count
    }

    fn usage(input: u64, output: u64) -> Usage {
        Usage {
            input_tokens: input,
            output_tokens: output,
            total_tokens: input + output,
            cached_input_tokens: 0,
        }
    }

    #[test]
    fn record_success_emits_two_token_observations_and_one_duration() {
        let (m, provider, exporter) = make_provider();
        m.record_success(&usage(100, 50), Duration::from_millis(500), "gpt-4o");
        // one input + one output observation
        assert_eq!(
            histogram_count(&provider, &exporter, GEN_AI_CLIENT_TOKEN_USAGE, None),
            2
        );
        let (m2, p2, e2) = make_provider();
        m2.record_success(&usage(100, 50), Duration::from_millis(500), "gpt-4o");
        assert_eq!(
            histogram_count(&p2, &e2, GEN_AI_CLIENT_OPERATION_DURATION, None),
            1
        );
    }

    #[test]
    fn record_success_labels_token_types() {
        let (m, provider, exporter) = make_provider();
        m.record_success(&usage(100, 50), Duration::from_millis(500), "gpt-4o");
        assert_eq!(
            histogram_count(
                &provider,
                &exporter,
                GEN_AI_CLIENT_TOKEN_USAGE,
                Some((GEN_AI_TOKEN_TYPE, "input"))
            ),
            1
        );
        let (m2, p2, e2) = make_provider();
        m2.record_success(&usage(100, 50), Duration::from_millis(500), "gpt-4o");
        assert_eq!(
            histogram_count(
                &p2,
                &e2,
                GEN_AI_CLIENT_TOKEN_USAGE,
                Some((GEN_AI_TOKEN_TYPE, "output"))
            ),
            1
        );
    }

    #[test]
    fn record_error_emits_one_duration_with_error_type() {
        let (m, provider, exporter) = make_provider();
        m.record_error(Duration::from_millis(200), "Completion", "gpt-4o");
        assert_eq!(
            histogram_count(
                &provider,
                &exporter,
                GEN_AI_CLIENT_OPERATION_DURATION,
                Some((ERROR_TYPE, "Completion"))
            ),
            1
        );
    }
}
