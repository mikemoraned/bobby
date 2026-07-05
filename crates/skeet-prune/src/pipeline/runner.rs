//! Assembly and supervision for the staged pipeline: wire the bounded channels,
//! spawn each stage into a supervised `JoinSet`, install the shutdown handler,
//! and await completion bounded by the drain deadline.
use std::sync::Arc;
use std::time::Duration;

use shared::{ModelVersion, PruneConfig};
use skeet_store::{Images, Statistics};
use tokio::signal::unix::{Signal, SignalKind, signal};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::firehose::SkeetCandidate;
use crate::pipeline::prune_image_stage::ClassifyConfig;
use crate::pipeline::{
    ChannelMonitors, ImageMessage, MetaMessage, PipelineCounters, StatsMessage,
    content_statistics_stage, firehose_stage, prune_image_stage, prune_meta_stage, save_stage,
};

/// A configured pipeline: what it processes with, plus its runtime tunables.
///
/// Holds the data (store, HTTP client, prune config) and the knobs (worker
/// counts, intervals, drain timeout). Construct one and call [`Pipeline::run`];
/// the assembly — channels, worker pools, shutdown wiring — stays private to
/// this module.
pub struct Pipeline<S> {
    pub store: Arc<S>,
    pub http: reqwest::Client,
    pub prune_config: PruneConfig,
    pub config_version: ModelVersion,
    pub meta_workers: usize,
    pub image_workers: usize,
    pub status_interval: Duration,
    pub statistics_flush_interval: Duration,
    /// How long the channels may drain into the store after a shutdown signal
    /// before exit is forced. Kept below the k8s grace period so the drain
    /// finishes ahead of SIGKILL.
    pub drain_timeout: Duration,
}

impl<S> Pipeline<S>
where
    S: Images + Statistics + Send + Sync + 'static,
{
    /// Assemble and run the pipeline until it completes.
    ///
    /// Under normal operation this never returns — the firehose is an unbounded
    /// source. It returns on shutdown: a reactive abort (a stage's downstream
    /// closed) or a deliberate drain (a termination signal), the latter bounded
    /// by [`drain_timeout`](Self::drain_timeout).
    pub async fn run(self) {
        let Self {
            store,
            http,
            prune_config,
            config_version,
            meta_workers,
            image_workers,
            status_interval,
            statistics_flush_interval,
            drain_timeout,
        } = self;

        // Pipeline: firehose → meta prune → image prune → save → stats. The
        // firehose→meta and meta→image channels are MPMC so each stage's worker
        // pool shares one input; the image→save and save→stats channels each
        // have a single consumer but use the same channel type.
        let (firehose_tx, firehose_rx) = async_channel::bounded::<SkeetCandidate>(16);
        let (meta_tx, meta_rx) = async_channel::bounded::<MetaMessage>(16);
        let (image_tx, image_rx) = async_channel::bounded::<ImageMessage>(100);
        let (stats_tx, stats_rx) = async_channel::bounded::<StatsMessage>(100);

        let counters = Arc::new(PipelineCounters::default());
        let channels = ChannelMonitors::new(&firehose_tx, &meta_tx, &image_tx);

        // Two shutdown signals with distinct intent:
        // - `abort` (reactive): a stage whose downstream closes cancels it, so
        //   every other stage unwinds at once through the same seam — in-flight
        //   work is dropped because there's nowhere for it to go.
        // - `drain` (deliberate): a shutdown signal trips this to stop *only* the
        //   firehose source. The bounded channels then close-cascade so buffered
        //   and in-flight items finish into the idempotent store before exit.
        let abort = CancellationToken::new();
        let drain = CancellationToken::new();
        spawn_signal_handler(abort.clone(), drain.clone());

        // Supervise every stage so shutdown awaits their completion, not just the
        // sink's: the drain finishes only once the sink stage returns, which
        // happens after everything upstream has drained through it into the store.
        let mut stages = JoinSet::new();

        {
            let counters = Arc::clone(&counters);
            let abort = abort.clone();
            let drain = drain.clone();
            stages.spawn(async move {
                firehose_stage::run(firehose_tx, counters, abort, drain).await;
            });
        }

        {
            let http = http.clone();
            let counters = Arc::clone(&counters);
            let abort = abort.clone();
            stages.spawn(async move {
                prune_meta_stage::run_workers(
                    firehose_rx,
                    meta_tx,
                    http,
                    counters,
                    meta_workers,
                    abort,
                )
                .await;
            });
        }

        {
            let counters = Arc::clone(&counters);
            let abort = abort.clone();
            let classify_config = ClassifyConfig {
                http,
                prune_config,
                config_version,
            };
            stages.spawn(async move {
                prune_image_stage::run_workers(
                    meta_rx,
                    image_tx,
                    classify_config,
                    counters,
                    image_workers,
                    abort,
                )
                .await;
            });
        }

        {
            let store = Arc::clone(&store);
            let abort = abort.clone();
            stages.spawn(async move {
                save_stage::run(&image_rx, store.as_ref(), stats_tx, abort).await;
            });
        }

        {
            let abort = abort.clone();
            stages.spawn(async move {
                content_statistics_stage::run(
                    &stats_rx,
                    store.as_ref(),
                    counters,
                    channels,
                    status_interval,
                    statistics_flush_interval,
                    abort,
                )
                .await;
            });
        }

        tokio::select! {
            () = await_stages(&mut stages) => info!("pipeline stages completed, exiting"),
            () = drain_deadline(&drain, drain_timeout) => {
                warn!(
                    timeout_secs = drain_timeout.as_secs(),
                    "drain deadline exceeded; exiting with items possibly still in flight"
                );
            }
        }
    }
}

/// Spawn the shutdown handler: the first termination signal starts a drain, a
/// second escalates to a reactive abort.
fn spawn_signal_handler(abort: CancellationToken, drain: CancellationToken) {
    tokio::spawn(async move {
        match install_shutdown_signals() {
            Ok((mut sigterm, mut sigint)) => {
                wait_either(&mut sigterm, &mut sigint).await;
                info!("shutdown signal received, draining pipeline (signal again to abort)");
                drain.cancel();
                wait_either(&mut sigterm, &mut sigint).await;
                warn!("second shutdown signal received, aborting drain");
                abort.cancel();
            }
            Err(e) => {
                warn!(error = %e, "failed to install signal handlers; SIGTERM will fall back to SIGKILL");
            }
        }
    });
}

/// Install the process-termination signal streams: SIGTERM (k8s redeploy, sent
/// before the grace-period SIGKILL) and SIGINT (Ctrl-C during local iteration).
fn install_shutdown_signals() -> std::io::Result<(Signal, Signal)> {
    Ok((
        signal(SignalKind::terminate())?,
        signal(SignalKind::interrupt())?,
    ))
}

/// Resolve when either signal stream next fires.
async fn wait_either(sigterm: &mut Signal, sigint: &mut Signal) {
    tokio::select! {
        _ = sigterm.recv() => {}
        _ = sigint.recv() => {}
    }
}

/// Await every supervised stage to completion, logging any that failed to join.
async fn await_stages(stages: &mut JoinSet<()>) {
    while let Some(res) = stages.join_next().await {
        if let Err(e) = res {
            warn!(error = %e, "pipeline stage task failed");
        }
    }
}

/// Bound the post-signal drain: resolves `timeout` after `drain` is tripped, and
/// never before. Normal running and reactive abort leave `drain` untripped, so
/// this future stays pending and the caller waits on the stages with no timer —
/// only a deliberate drain is deadline-bounded.
async fn drain_deadline(drain: &CancellationToken, timeout: Duration) {
    drain.cancelled().await;
    tokio::time::sleep(timeout).await;
}
