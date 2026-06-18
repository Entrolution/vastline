//! Burn-rate maths. The key honesty point (your call-out): "running burn" and "total burn" are
//! different numbers and both matter, so we compute and surface them separately.
//!
//!   * running burn — what the live, compute-billing instances cost per hour.
//!   * total burn   — every instance's `dph_total`, *including storage on stopped instances*.
//!
//! Runway (time until the balance is gone) is computed from **total** burn, because that is what
//! actually drains the wallet — an idle-but-not-destroyed fleet still bleeds storage cost, and a
//! runway based on running burn alone would read as falsely infinite while money leaks away.

/// Hours of balance left at a given burn rate. `None` when balance is unknown or burn is ~zero
/// (nothing is draining, so "time left" is not meaningful / would be infinite).
pub fn runway_hours(balance: Option<f64>, burn_per_hr: f64) -> Option<f64> {
    let bal = balance?;
    if burn_per_hr <= 1e-9 {
        return None;
    }
    Some((bal.max(0.0)) / burn_per_hr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runway_basic() {
        // $47.20 at $1.89/hr ≈ 24.97h
        let h = runway_hours(Some(47.20), 1.89).unwrap();
        assert!((h - 24.97).abs() < 0.05, "got {h}");
    }

    #[test]
    fn runway_none_when_no_burn() {
        assert_eq!(runway_hours(Some(10.0), 0.0), None);
    }

    #[test]
    fn runway_none_when_no_balance() {
        assert_eq!(runway_hours(None, 1.0), None);
    }

    #[test]
    fn runway_clamps_negative_balance_to_zero() {
        assert_eq!(runway_hours(Some(-5.0), 2.0), Some(0.0));
    }
}
