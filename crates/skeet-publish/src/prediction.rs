use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// Quantiles of the predicted wait until the next match, as absolute wall-clock
/// instants the next match is expected by (see [`predict_next_match`]).
///
/// Matches are modelled as a homogeneous Poisson process, so the time to the
/// next one is `Exponential(λ)`; these three points of that distribution's
/// inverse CDF bracket when it is likely to appear.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NextMatchPrediction {
    /// `T(0.025)` — a 2.5% chance of a match by here (an optimistic early bound).
    pub lower: DateTime<Utc>,
    /// `T(0.5)` — the median wait; an even chance of a match by here.
    pub middle: DateTime<Utc>,
    /// `T(0.975)` — a 97.5% chance of a match by here (the "95% chance by" point).
    pub upper: DateTime<Utc>,
}

/// Predict when the next match will arrive, modelling matches over the window
/// `[start, now]` as a homogeneous Poisson process with rate `λ = count / window`
/// and predicting forward from `now`.
///
/// Returns `None` when there's nothing to extrapolate from — `count == 0`
/// (`λ = 0` ⇒ an infinite, undefined wait) or a non-positive window — so the
/// caller shows no countdown rather than a bogus one.
pub fn predict_next_match(
    now: DateTime<Utc>,
    start: DateTime<Utc>,
    count: u64,
) -> Option<NextMatchPrediction> {
    let window_seconds = (now - start).num_seconds();
    if count == 0 || window_seconds <= 0 {
        return None;
    }
    // Exponential inverse-CDF: the time by which there's probability `p` of a
    // match is T(p) = −ln(1 − p) / λ, with λ = count / window.
    let by = |p: f64| {
        let wait_seconds = -(1.0 - p).ln() * window_seconds as f64 / count as f64;
        now + Duration::milliseconds((wait_seconds * 1000.0).round() as i64)
    };
    Some(NextMatchPrediction {
        lower: by(0.025),
        middle: by(0.5),
        upper: by(0.975),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use proptest::prelude::*;

    /// A base instant plus `hours`, so windows wider than a day are easy to write.
    fn at(hours: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap() + Duration::hours(hours)
    }

    #[test]
    fn quantiles_are_the_expected_waits_for_two_matches_per_day() {
        // 24h window, 2 matches → λ = 2 per day. The waits below are −ln(1−p)/λ
        // worked out once (with p = 0.025 / 0.5 / 0.975) and frozen as concrete
        // seconds: ~18m, the ~8h19m median, and ~44h16m. Using count > 1 keeps the
        // `/ count` honest — at count = 1 a `*`-vs-`/` slip would be invisible.
        let now = at(24);
        let prediction = predict_next_match(now, at(0), 2).expect("predictable");
        let wait = |t: DateTime<Utc>| (t - now).num_seconds();
        assert_eq!(wait(prediction.lower), 1_093);
        assert_eq!(wait(prediction.middle), 29_943);
        assert_eq!(wait(prediction.upper), 159_359);
    }

    proptest! {
        /// For any operational window (an hour up to a year) with at least one
        /// match, the quantiles are well-formed: each is strictly after the origin
        /// `now`, and they are strictly ordered `lower < middle < upper`. The range
        /// is bounded to realistic bucket sizes/counts; far outside it the
        /// millisecond rounding of sub-millisecond waits would collapse them.
        #[test]
        fn quantiles_are_ordered_and_after_now(
            window_seconds in 3_600i64..=366 * 24 * 3_600,
            count in 1u64..=10_000,
        ) {
            let now = at(0);
            let prediction = predict_next_match(now, now - Duration::seconds(window_seconds), count)
                .expect("a positive window with a match is always predictable");
            prop_assert!(now < prediction.lower);
            prop_assert!(prediction.lower < prediction.middle);
            prop_assert!(prediction.middle < prediction.upper);
        }
    }

    #[test]
    fn no_prediction_without_a_match() {
        assert_eq!(predict_next_match(at(48), at(0), 0), None);
    }

    #[test]
    fn no_prediction_for_a_non_positive_window() {
        assert_eq!(predict_next_match(at(0), at(0), 5), None);
        assert_eq!(predict_next_match(at(0), at(24), 5), None);
    }
}
