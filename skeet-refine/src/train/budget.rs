use eval::Usd;

use crate::train::TrainError;

/// Sample size per iteration. Reserves cost for one full test-set evaluation
/// then divides the residual budget evenly across `max_iterations`.
pub fn per_iteration_sample_size(
    per_image_cost: Usd,
    test_count: usize,
    budget: Usd,
    max_iterations: u32,
) -> Result<usize, TrainError> {
    let reserved_for_final_eval = per_image_cost * test_count as u64;
    let residual = if budget > reserved_for_final_eval {
        budget - reserved_for_final_eval
    } else {
        Usd::zero()
    };
    let per_iter_budget = residual / u64::from(max_iterations);
    let size = per_iter_budget.ratio_floor(per_image_cost) as usize;
    if size == 0 {
        return Err(TrainError::BudgetTooSmall);
    }
    Ok(size)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usd(s: &str) -> Usd {
        s.parse().expect("valid Usd")
    }

    #[test]
    fn sample_size_is_reduced_by_final_eval_reservation() {
        // $0.01/image, budget $2.00, 100 test images, 5 iterations:
        //   reserve $1.00 for final eval → residual $1.00
        //   $1.00 / 5 iterations / $0.01 = 20 images/iter
        //   (without reservation: floor($2.00 / 5 / $0.01) = 40)
        let n = per_iteration_sample_size(usd("0.01"), 100, usd("2.00"), 5)
            .expect("budget covers it");
        assert_eq!(n, 20);
    }

    #[test]
    fn per_iteration_size_errors_when_budget_too_small() {
        let err = per_iteration_sample_size(usd("0.01"), 100, usd("0.0"), 5);
        assert!(matches!(err, Err(TrainError::BudgetTooSmall)));
    }
}
