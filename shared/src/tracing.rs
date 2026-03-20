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
