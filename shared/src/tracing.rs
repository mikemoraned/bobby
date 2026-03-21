use opentelemetry::trace::TracerProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;
use tracing_subscriber::Layer;

fn env_filter(default_filter: &str) -> tracing_subscriber::EnvFilter {
    tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| default_filter.parse().expect("valid filter"))
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
pub fn init(default_filter: &str) {
    tracing_subscriber::fmt()
        .with_env_filter(env_filter(default_filter))
        .init();
}

/// Initialize tracing with a daily rolling file appender and optional OpenTelemetry.
///
/// Logs are written to `logs/{filename}` with daily rotation.
/// The returned guards must be held for the lifetime of the program.
pub fn init_with_file(default_filter: &str, filename: &str) -> TracingGuard {
    let file_appender = tracing_appender::rolling::daily("logs", filename);
    let (non_blocking, file_guard) = tracing_appender::non_blocking(file_appender);

    let (otel_layer, otel_guard) = match try_otel_layer() {
        Some((layer, guard)) => (Some(layer.with_filter(env_filter(default_filter))), Some(guard)),
        None => (None, None),
    };

    let (console_layer, console_server) = console_subscriber::ConsoleLayer::builder()
        .with_default_env()
        .build();

    tracing_subscriber::registry()
        .with(console_layer)
        .with(fmt::layer().with_writer(non_blocking).with_filter(env_filter(default_filter)))
        .with(otel_layer)
        .init();

    tokio::spawn(console_server.serve());

    TracingGuard {
        _file_guard: file_guard,
        _otel_guard: otel_guard,
    }
}

/// Initialize tracing with both a daily rolling file appender, stderr output,
/// and optional OpenTelemetry.
///
/// The returned guards must be held for the lifetime of the program.
pub fn init_with_file_and_stderr(default_filter: &str, filename: &str) -> TracingGuard {
    let file_appender = tracing_appender::rolling::daily("logs", filename);
    let (non_blocking, file_guard) = tracing_appender::non_blocking(file_appender);

    let (otel_layer, otel_guard) = match try_otel_layer() {
        Some((layer, guard)) => (Some(layer.with_filter(env_filter(default_filter))), Some(guard)),
        None => (None, None),
    };

    let (console_layer, console_server) = console_subscriber::ConsoleLayer::builder()
        .with_default_env()
        .build();

    tracing_subscriber::registry()
        .with(console_layer)
        .with(fmt::layer().with_writer(non_blocking).with_filter(env_filter(default_filter)))
        .with(fmt::layer().with_writer(std::io::stderr).with_filter(env_filter(default_filter)))
        .with(otel_layer)
        .init();

    tokio::spawn(console_server.serve());

    TracingGuard {
        _file_guard: file_guard,
        _otel_guard: otel_guard,
    }
}

/// Holds guards for tracing infrastructure (file appender + optional OpenTelemetry).
///
/// Must be held for the lifetime of the program to ensure logs are flushed
/// and the OTLP exporter shuts down cleanly.
pub struct TracingGuard {
    _file_guard: WorkerGuard,
    _otel_guard: Option<OtelGuard>,
}
