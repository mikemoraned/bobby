#![warn(clippy::all, clippy::nursery)]

//! One-off: reconstruct historical `PruneStats` from saved-image timestamps for
//! the period before the pruner started recording statistics itself. Resumable —
//! it picks up after the latest already-recorded interval — and intended to be
//! deleted once the backfill has run. The `Images`/`Statistics` port methods it
//! relies on are kept.

use chrono::{Duration, DurationRound};
use clap::Parser;
use skeet_store::{Images, PruneStats, Statistics, StoreArgs};
use tracing::info;

/// The pruner's measured save rate, as a percentage of examined images that get
/// saved as candidates. Mirrors skeet-publish's `SAVE_RATE_PERCENT`; duplicated
/// here because this is a throwaway and we don't want to couple the store to it.
const SAVE_RATE_PERCENT: f64 = 0.2;

/// Invert the save rate: how many images must have been examined to yield
/// `saved` saves. We have no separate record of skeets-seen vs images-examined
/// for the backfilled period, so the two are taken as equal (see slice doc).
fn estimate_examined(saved: u64) -> u64 {
    (saved as f64 * 100.0 / SAVE_RATE_PERCENT).round() as u64
}

#[derive(Parser)]
#[command(about = "One-off: backfill prune statistics from saved-image timestamps")]
struct Args {
    #[command(flatten)]
    store: StoreArgs,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _guard = shared::tracing::init_with_file("info", "backfill-statistics");
    info!(
        git_hash = env!("BUILD_GIT_HASH"),
        "backfill-statistics starting"
    );

    let args = Args::parse();
    let store = args.store.open_store("backfill-statistics").await?;

    let Some(oldest) = store.oldest_discovered_at().await? else {
        info!("no saved images; nothing to backfill");
        return Ok(());
    };
    // `newest` is `Some` whenever `oldest` is, but fall back defensively.
    let newest = store
        .newest_discovered_at()
        .await?
        .map_or_else(|| oldest.as_datetime(), |d| d.as_datetime());

    // Resume after whatever's already recorded; otherwise start at the oldest
    // image. The max handles both, and aligning to the hour keeps every recorded
    // interval on a clean boundary (so resume never straddles one).
    let resume_from = store.latest_interval_end().await?;
    let start = resume_from.map_or_else(|| oldest.as_datetime(), |end| oldest.as_datetime().max(end));
    let mut interval_start = start.duration_trunc(Duration::hours(1))?;

    info!(
        oldest = %oldest,
        newest = %newest,
        ?resume_from,
        first_interval_start = %interval_start,
        "computed backfill range"
    );

    let mut recorded = 0_u64;
    while interval_start < newest {
        let interval_end = interval_start + Duration::hours(1);
        let images_saved = store.count_in_interval(interval_start, interval_end).await?;
        if images_saved > 0 {
            let examined = estimate_examined(images_saved);
            store
                .record(&PruneStats {
                    interval_start,
                    interval_end,
                    skeets_seen: examined,
                    images_examined: examined,
                    images_saved,
                })
                .await?;
            recorded += 1;
            info!(%interval_start, %interval_end, images_saved, examined, "recorded interval");
        }
        interval_start = interval_end;
    }

    info!(recorded, "backfill complete");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_examined_inverts_the_save_rate() {
        // Mirrors skeet-publish's estimate_processed: saved × 500.
        assert_eq!(estimate_examined(0), 0);
        assert_eq!(estimate_examined(1), 500);
        assert_eq!(estimate_examined(43243), 21_621_500);
    }
}
