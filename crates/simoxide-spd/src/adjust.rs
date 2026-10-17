//! The size change of an adjustment, as `spd/common/AdjustmentCalculator.qvto` computes it
//! (`DESIGN-SPD.md` §1.3). The interpreter uses it for the fixed Q9 (group-size constraints
//! checked in the chain); the adjustment itself (P2) runs the transformation.

use crate::model::Adjustment;

/// `Interprete_adjustmentType`: the desired change for a group of `size` elements. `Err`: the
/// transformation's `assert fatal` (absolute goal value not positive).
///
/// Relative: `x = size * percentage / 100.0`; with `q4` (Slingshot) `max(x, min)` if `x > 0`,
/// else `min(x, min)` (so 0 % gives 0 or a negative `min`, ignoring a positive `min`). Fixed
/// (provisional): 0 stays 0; otherwise the magnitude is at least `min` in the direction of `x`.
pub fn desired_change(adj: &Adjustment, size: i64, q4: bool) -> Result<f64, String> {
    Ok(match *adj {
        Adjustment::Step { step } => step as f64,
        Adjustment::Absolute { goal } => {
            if goal <= 0 {
                return Err("The specified goal value has to be greater then 0.".into());
            }
            (goal - size) as f64
        }
        Adjustment::Relative { percentage, min } => {
            // Integer * Integer / Real (OCL): the product is an Integer
            let x = size.wrapping_mul(percentage) as f64 / 100.0;
            let min = min as f64;
            if q4 {
                if x > 0.0 { x.max(min) } else { x.min(min) }
            } else if x > 0.0 {
                x.max(min)
            } else if x < 0.0 {
                x.min(-min)
            } else {
                0.0
            }
        }
        Adjustment::Missing => 0.0,
    })
}

/// `Interprete_GroupSizeConstraints`: the change bounded to `[min - size, max - size]`. `Err`:
/// the constraint's `assert fatal`s (`min >= 1`, `max > min`).
pub fn clamp(change: f64, size: i64, min: i64, max: i64) -> Result<f64, String> {
    if min < 1 {
        return Err("A minimum below 1 is not permitted for a group size constraints.".into());
    }
    if max <= min {
        return Err("The maximum constraint lower or equal to the minimum is not permitted in a group size constraint.".into());
    }
    Ok(change.min((max - size) as f64).max((min - size) as f64))
}

/// The number of elements an enactment adds (> 0) or removes (< 0): scale-out appends
/// `ceil(change)` copies, scale-in keeps `max(1, size - round(|change|))` elements.
pub fn effective_change(change: f64, size: i64) -> i64 {
    if change > 0.0 {
        change.ceil() as i64
    } else if change < 0.0 {
        let remove = (-change + 0.5).floor() as i64;
        -remove.min((size - 1).max(0))
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_absolute_relative() {
        assert_eq!(
            desired_change(&Adjustment::Step { step: -2 }, 5, true),
            Ok(-2.0)
        );
        assert_eq!(
            desired_change(&Adjustment::Absolute { goal: 3 }, 5, true),
            Ok(-2.0)
        );
        assert!(desired_change(&Adjustment::Absolute { goal: 0 }, 5, true).is_err());
        let r = |p, m| Adjustment::Relative {
            percentage: p,
            min: m,
        };
        // 50 % of 3 = 1.5, at least 1
        assert_eq!(desired_change(&r(50, 1), 3, true), Ok(1.5));
        assert_eq!(desired_change(&r(10, 1), 3, true), Ok(1.0));
        // Q4: 0 % with min 1 gives min(0, 1) = 0 in Slingshot (the min is ignored) ...
        assert_eq!(desired_change(&r(0, 1), 3, true), Ok(0.0));
        // ... and a negative percentage ignores the min magnitude
        assert_eq!(desired_change(&r(-10, 1), 3, true), Ok(-0.3));
        // fixed: zero stays zero, otherwise at least `min` in the direction of the percentage
        assert_eq!(desired_change(&r(0, 1), 3, false), Ok(0.0));
        assert_eq!(desired_change(&r(-10, 1), 3, false), Ok(-1.0));
        assert_eq!(desired_change(&r(10, 1), 3, false), Ok(1.0));
    }

    #[test]
    fn clamping_and_effect() {
        assert_eq!(clamp(3.0, 2, 1, 4), Ok(2.0));
        assert_eq!(clamp(-3.0, 2, 1, 4), Ok(-1.0));
        assert_eq!(clamp(1.0, 4, 1, 4), Ok(0.0));
        assert!(clamp(1.0, 4, 0, 4).is_err());
        assert!(clamp(1.0, 4, 2, 2).is_err());
        assert_eq!(effective_change(0.3, 2), 1);
        assert_eq!(effective_change(-0.3, 2), 0);
        assert_eq!(effective_change(-0.5, 2), -1);
        assert_eq!(effective_change(-5.0, 2), -1, "at least one element stays");
        assert_eq!(effective_change(-1.0, 1), 0);
        assert_eq!(effective_change(0.0, 1), 0);
    }
}
