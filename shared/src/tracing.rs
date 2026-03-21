use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;

fn env_filter(default_filter: &str) -> tracing_subscriber::EnvFilter {
    tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| default_filter.parse().expect("valid filter"))
}

/// Initialize a stderr tracing subscriber with `RUST_LOG` env support.
///
/// Falls back to `default_filter` if `RUST_LOG` is not set (e.g. `"info"`).
pub fn init(default_filter: &str) {
    tracing_subscriber::fmt()
        .with_env_filter(env_filter(default_filter))
        .init();
}

/// Initialize tracing with a daily rolling file appender.
///
/// Logs are written to `logs/{filename}` with daily rotation.
/// The returned `WorkerGuard` must be held for the lifetime of the program
/// to ensure all logs are flushed.
pub fn init_with_file(default_filter: &str, filename: &str) -> WorkerGuard {
    let file_appender = tracing_appender::rolling::daily("logs", filename);
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::registry()
        .with(env_filter(default_filter))
        .with(fmt::layer().with_writer(non_blocking))
        .init();

    guard
}

/// Initialize tracing with both a daily rolling file appender and stderr output.
///
/// The returned `WorkerGuard` must be held for the lifetime of the program
/// to ensure all logs are flushed.
pub fn init_with_file_and_stderr(default_filter: &str, filename: &str) -> WorkerGuard {
    let file_appender = tracing_appender::rolling::daily("logs", filename);
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::registry()
        .with(env_filter(default_filter))
        .with(fmt::layer().with_writer(non_blocking))
        .with(fmt::layer().with_writer(std::io::stderr))
        .init();

    guard
}
