use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use backon::RetryableWithContext;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::firehose::SkeetCandidate;
use crate::pipeline::{self, PipelineCounters};

/// Why a reconnect cycle needs to back off and retry. Both variants are
/// retryable; a healthy session ending is an `Ok` (see [`SessionOutcome`]).
#[derive(Debug, thiserror::Error)]
enum ReconnectError {
    #[error("failed to connect to firehose: {0}")]
    Connect(String),
    #[error("firehose session flapped after {0:?}")]
    Flap(Duration),
}

/// How a connected session ended, when it ended without needing a retry.
enum SessionOutcome {
    /// A session that stayed up long enough to count as healthy ended; the
    /// reconnect backoff should reset to its base delay.
    Stable,
    /// Shutdown was requested (token cancelled or downstream closed); the
    /// stage should stop entirely.
    ShutDown,
}

pub async fn run(
    tx: mpsc::Sender<SkeetCandidate>,
    counters: Arc<PipelineCounters>,
    token: CancellationToken,
) {
    let recv_timeout = Duration::from_secs(30);

    // The resume cursor lives only in memory: it survives reconnects within
    // this process, but a restart resumes at live-tail (the redeploy/crash gap
    // is intentionally accepted as lost).
    let mut last_time_us: Option<u64> = None;

    loop {
        // Each iteration starts a fresh backoff schedule, so a healthy session
        // ending resets the escalating, jittered delay back to its base. The
        // cursor (`last_time_us`) threads across attempts as the retry context.
        let op = |last: Option<u64>| {
            let tx = tx.clone();
            let counters = counters.clone();
            let token = token.clone();
            async move { run_session(last, recv_timeout, tx, counters, token).await }
        };

        let (last, outcome) = op
            .retry(crate::firehose::reconnect_backoff())
            .context(last_time_us)
            .notify(|err: &ReconnectError, dur| {
                warn!(?dur, %err, "firehose reconnect backing off");
            })
            .await;
        last_time_us = last;

        match outcome {
            // Unreachable in practice — the never-give-up backoff only returns
            // once the operation succeeds — but stopping is the safe fallback.
            Ok(SessionOutcome::ShutDown) | Err(_) => return,
            Ok(SessionOutcome::Stable) => {}
        }
    }
}

/// Connect once and pump events until the session ends, advancing the resume
/// cursor on every observed event. Returns the (possibly advanced) cursor plus
/// either how the session ended cleanly or the error that should back off.
async fn run_session(
    mut last_time_us: Option<u64>,
    recv_timeout: Duration,
    tx: mpsc::Sender<SkeetCandidate>,
    counters: Arc<PipelineCounters>,
    token: CancellationToken,
) -> (Option<u64>, Result<SessionOutcome, ReconnectError>) {
    let cursor = last_time_us.and_then(crate::firehose::cursor_from);
    let receiver = match crate::firehose::connect(cursor).await {
        Ok(r) => r,
        Err(e) => return (last_time_us, Err(ReconnectError::Connect(e.to_string()))),
    };
    info!("firehose connected, listening for posts...");

    let started = Instant::now();
    loop {
        let event = tokio::select! {
            () = token.cancelled() => return (last_time_us, Ok(SessionOutcome::ShutDown)),
            result = tokio::time::timeout(recv_timeout, receiver.recv_async()) => match result {
                Ok(Ok(event)) => event,
                Ok(Err(_)) => {
                    warn!("firehose channel closed");
                    break;
                }
                Err(_) => {
                    warn!("no message received in {recv_timeout:?}, reconnecting");
                    break;
                }
            },
        };

        last_time_us = Some(crate::firehose::event_time_us(&event));

        if let Some(candidate) = crate::firehose::extract_skeet_candidate(&event) {
            counters.firehose.fetch_add(1, Ordering::Relaxed);
            if pipeline::forward(&tx, candidate, &token).await.is_err() {
                return (last_time_us, Ok(SessionOutcome::ShutDown));
            }
        }
    }

    let up_for = started.elapsed();
    if crate::firehose::session_was_stable(up_for) {
        (last_time_us, Ok(SessionOutcome::Stable))
    } else {
        (last_time_us, Err(ReconnectError::Flap(up_for)))
    }
}
