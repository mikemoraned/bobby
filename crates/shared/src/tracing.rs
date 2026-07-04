use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::MetricExporter;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::SdkTracerProvider;
use std::time::Duration;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::Layer;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;

// `default_filter` is a compile-time constant supplied by each binary; a parse
// failure is a programming error in our own directive, caught immediately at startup.
#[allow(clippy::expect_used)]
fn targets_filter(default_filter: &str) -> Targets {
    std::env::var("RUST_LOG")
        .ok()
        .as_deref()
        .unwrap_or(default_filter)
        .parse()
        .expect("valid filter")
}

/// Ensure the binary's own log target is captured by the shipped default filter.
///
/// A `Targets` filter with specific `crate=level` directives drops every target
/// it doesn't list, and a binary's log target is its crate name — trivially
/// omitted (and then silently lost) because it differs from the library crates
/// the directives usually name. Prepending `{bin_target}=info` guarantees the
/// binary's own logs survive regardless of what it's named. Prepended (not
/// appended) so an explicit later directive for the same target still wins.
///
/// Only applied to the compiled-in default; an explicit `RUST_LOG` stays a full
/// override so a developer can still narrow or silence everything on purpose.
fn with_bin_target(bin_target: &str, default_filter: &str) -> String {
    format!("{bin_target}=info,{default_filter}")
}

/// Guard that shuts down the OpenTelemetry tracer provider on drop.
pub struct OtelGuard {
    provider: SdkTracerProvider,
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        if let Err(e) = self.provider.shutdown() {
            eprintln!("OpenTelemetry shutdown error: {e}");
        }
    }
}

/// Guard that shuts down the OpenTelemetry metrics provider on drop.
pub struct MetricsGuard {
    provider: SdkMeterProvider,
}

impl Drop for MetricsGuard {
    fn drop(&mut self) {
        if let Err(e) = self.provider.shutdown() {
            eprintln!("OpenTelemetry metrics shutdown error: {e}");
        }
    }
}

/// Try to initialize an OpenTelemetry OTLP metrics provider.
///
/// Sets the global meter provider. Returns `None` (with a warning) if
/// `OTEL_EXPORTER_OTLP_ENDPOINT` is not set.
pub fn try_init_metrics() -> Option<MetricsGuard> {
    if std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_err() {
        tracing::warn!("OTEL_EXPORTER_OTLP_ENDPOINT not set, OpenTelemetry metrics disabled");
        return None;
    }

    let exporter = match MetricExporter::builder().with_http().build() {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("Failed to build OTel metric exporter: {e}");
            return None;
        }
    };
    let reader = PeriodicReader::builder(exporter)
        .with_interval(Duration::from_secs(5))
        .build();
    let provider = SdkMeterProvider::builder().with_reader(reader).build();

    opentelemetry::global::set_meter_provider(provider.clone());
    Some(MetricsGuard { provider })
}

/// Try to initialize an OpenTelemetry OTLP tracing layer.
///
/// Returns `None` (with a warning) if `OTEL_EXPORTER_OTLP_ENDPOINT` is not set.
fn try_otel_layer<S>() -> Option<(
    tracing_opentelemetry::OpenTelemetryLayer<S, opentelemetry_sdk::trace::Tracer>,
    OtelGuard,
)>
where
    S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
{
    if std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_err() {
        tracing::warn!("OTEL_EXPORTER_OTLP_ENDPOINT not set, OpenTelemetry disabled");
        return None;
    }

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .build()
        .ok()?;

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .build();

    let tracer = provider.tracer("bobby");
    let layer = tracing_opentelemetry::layer().with_tracer(tracer);

    Some((layer, OtelGuard { provider }))
}

/// Initialize a stderr tracing subscriber with `RUST_LOG` env support.
///
/// Falls back to `default_filter` if `RUST_LOG` is not set (e.g. `"info"`).
// `default_filter` is a compile-time constant supplied by each binary; a parse
// failure is a programming error in our own directive, caught immediately at startup.
#[allow(clippy::expect_used)]
pub fn init(default_filter: &str) {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| default_filter.parse().expect("valid filter")),
        )
        .init();
}

/// Initialize tracing with a daily rolling file appender, stderr output,
/// and optional OpenTelemetry (traces + metrics).
///
/// `bin_target` is the calling binary's log target — pass `env!("CARGO_CRATE_NAME")`
/// so it tracks the crate name automatically — and is always captured at `info`
/// (see [`with_bin_target`]) so a binary can't silently filter out its own logs.
///
/// The returned guards must be held for the lifetime of the program.
pub fn init_with_file(bin_target: &str, default_filter: &str, filename: &str) -> TracingGuard {
    let file_appender = tracing_appender::rolling::daily("logs", filename);
    let (non_blocking, file_guard) = tracing_appender::non_blocking(file_appender);

    let filter = targets_filter(&with_bin_target(bin_target, default_filter));
    let (otel_layer, otel_guard) = match try_otel_layer() {
        Some((layer, guard)) => (Some(layer.with_filter(filter.clone())), Some(guard)),
        None => (None, None),
    };

    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_ansi(false)
                .with_writer(non_blocking)
                .with_filter(filter.clone()),
        )
        .with(
            fmt::layer()
                .with_writer(std::io::stderr)
                .with_filter(filter),
        )
        .with(otel_layer)
        .init();

    let metrics_guard = try_init_metrics();

    TracingGuard {
        _file_guard: Some(file_guard),
        _otel_guard: otel_guard,
        _metrics_guard: metrics_guard,
    }
}

/// Holds guards for tracing infrastructure (file appender + optional OpenTelemetry).
///
/// Must be held for the lifetime of the program to ensure logs are flushed
/// and the OTLP exporters shut down cleanly.
pub struct TracingGuard {
    _file_guard: Option<WorkerGuard>,
    _otel_guard: Option<OtelGuard>,
    _metrics_guard: Option<MetricsGuard>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::Level;

    /// The bin target is captured at info even when the default directives — the
    /// library crates a binary usually names — omit it entirely.
    #[test]
    fn bin_target_is_captured_when_absent_from_default() {
        let filter: Targets = with_bin_target("some_bin", "shared=info,skeet_store=info")
            .parse()
            .expect("valid filter");
        assert!(filter.would_enable("some_bin", &Level::INFO));
        // An unrelated, unlisted target stays filtered out — we widened the filter
        // by exactly the binary's own target, nothing more.
        assert!(!filter.would_enable("some_other_crate", &Level::INFO));
    }

    /// A later explicit directive for the same target overrides the prepended
    /// default, so a binary can still raise or lower its own level.
    #[test]
    fn explicit_directive_overrides_prepended_default() {
        let filter: Targets = with_bin_target("some_bin", "some_bin=warn")
            .parse()
            .expect("valid filter");
        assert!(!filter.would_enable("some_bin", &Level::INFO));
        assert!(filter.would_enable("some_bin", &Level::WARN));
    }
}
