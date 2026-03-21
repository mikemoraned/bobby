use opentelemetry::trace::TracerProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;
use tracing_subscriber::Layer;

fn targets_filter(default_filter: &str) -> Targets {
    std::env::var("RUST_LOG")
        .unwrap_or_else(|_| default_filter.to_string())
        .parse()
        .expect("valid filter")
}

/// Controls whether tokio-console support is enabled.
pub enum TokioConsoleSupport {
    Disabled,
    Enabled { port: u16 },
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
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| default_filter.parse().expect("valid filter")),
        )
        .init();
}

/// Initialize tracing with a daily rolling file appender and optional OpenTelemetry.
///
/// When tokio-console is enabled, file and OTEL layers are replaced by
/// console-subscriber's own stderr output to avoid a known incompatibility
/// between `ConsoleLayer` and `fmt::Layer` span tracking.
///
/// The returned guards must be held for the lifetime of the program.
pub fn init_with_file(
    default_filter: &str,
    filename: &str,
    console: TokioConsoleSupport,
) -> TracingGuard {
    if let TokioConsoleSupport::Enabled { port } = console {
        return init_with_console(port);
    }

    let file_appender = tracing_appender::rolling::daily("logs", filename);
    let (non_blocking, file_guard) = tracing_appender::non_blocking(file_appender);

    let (otel_layer, otel_guard) = match try_otel_layer() {
        Some((layer, guard)) => (Some(layer.with_filter(targets_filter(default_filter))), Some(guard)),
        None => (None, None),
    };

    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_ansi(false)
                .with_writer(non_blocking)
                .with_filter(targets_filter(default_filter)),
        )
        .with(otel_layer)
        .init();

    TracingGuard {
        _file_guard: Some(file_guard),
        _otel_guard: otel_guard,
    }
}

/// Initialize tracing with both a daily rolling file appender, stderr output,
/// and optional OpenTelemetry.
///
/// When tokio-console is enabled, file and OTEL layers are replaced by
/// console-subscriber's own stderr output to avoid a known incompatibility
/// between `ConsoleLayer` and `fmt::Layer` span tracking.
///
/// The returned guards must be held for the lifetime of the program.
pub fn init_with_file_and_stderr(
    default_filter: &str,
    filename: &str,
    console: TokioConsoleSupport,
) -> TracingGuard {
    if let TokioConsoleSupport::Enabled { port } = console {
        return init_with_console(port);
    }

    let file_appender = tracing_appender::rolling::daily("logs", filename);
    let (non_blocking, file_guard) = tracing_appender::non_blocking(file_appender);

    let (otel_layer, otel_guard) = match try_otel_layer() {
        Some((layer, guard)) => (Some(layer.with_filter(targets_filter(default_filter))), Some(guard)),
        None => (None, None),
    };

    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_ansi(false)
                .with_writer(non_blocking)
                .with_filter(targets_filter(default_filter)),
        )
        .with(fmt::layer().with_writer(std::io::stderr).with_filter(targets_filter(default_filter)))
        .with(otel_layer)
        .init();

    TracingGuard {
        _file_guard: Some(file_guard),
        _otel_guard: otel_guard,
    }
}

/// Initialize tracing with tokio-console support.
///
/// Uses console-subscriber's built-in subscriber which includes a stderr
/// fmt layer compatible with the ConsoleLayer. File and OTEL layers are
/// not available in this mode.
fn init_with_console(port: u16) -> TracingGuard {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    console_subscriber::ConsoleLayer::builder()
        .server_addr(addr)
        .with_default_env()
        .init();

    tracing::info!("tokio-console enabled: run `tokio-console http://127.0.0.1:{port}`");
    tracing::info!("file and OTEL logging disabled while tokio-console is active");

    TracingGuard {
        _file_guard: None,
        _otel_guard: None,
    }
}

/// Holds guards for tracing infrastructure (file appender + optional OpenTelemetry).
///
/// Must be held for the lifetime of the program to ensure logs are flushed
/// and the OTLP exporter shuts down cleanly.
pub struct TracingGuard {
    _file_guard: Option<WorkerGuard>,
    _otel_guard: Option<OtelGuard>,
}
