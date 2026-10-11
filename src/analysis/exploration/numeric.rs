//! Checked integer decisions and finite plotting arithmetic.
use crate::{
    analysis::{moments::convex_interpolation, validation::invalid_analysis, MAX_FREQUENCY},
    error::AppResult,
};

pub(crate) fn interpolate(
    left: f64,
    right: f64,
    numerator: u64,
    denominator: u64,
) -> AppResult<f64> {
    if !left.is_finite() || !right.is_finite() || denominator == 0 || numerator > denominator {
        return Err(invalid_analysis());
    }
    if numerator == denominator {
        return Ok(right);
    }
    if numerator == 0 || left == right {
        return Ok(left);
    }
    let value = convex_interpolation(left, right, numerator, denominator);
    if !value.is_finite() || value < left.min(right) || value > left.max(right) {
        return Err(invalid_analysis());
    }
    Ok(value)
}

pub(crate) fn quartile_rank(n: u64, quarter: u8) -> AppResult<(u64, u64, u64)> {
    if n == 0 || n > MAX_FREQUENCY || !(1..=3).contains(&quarter) {
        return Err(invalid_analysis());
    }
    let rank = u128::from(n - 1)
        .checked_mul(u128::from(quarter))
        .ok_or_else(invalid_analysis)?;
    Ok((
        u64::try_from(rank / 4).map_err(|_| invalid_analysis())?,
        (rank % 4) as u64,
        4,
    ))
}

/// A single displayed point selects the first eligible ordinal.
pub(crate) fn display_index(total: usize, shown: usize, index: usize) -> AppResult<usize> {
    if shown == 0 || shown > total || index >= shown {
        return Err(invalid_analysis());
    }
    if shown == 1 {
        return Ok(0);
    }
    index
        .checked_mul(total - 1)
        .map(|product| product / (shown - 1))
        .ok_or_else(invalid_analysis)
}

pub(crate) fn sturges_bins(n: u64) -> AppResult<usize> {
    if n > MAX_FREQUENCY {
        return Err(invalid_analysis());
    }
    Ok(if n == 0 {
        0
    } else {
        (1 + u64::BITS - (n - 1).leading_zeros()) as usize
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::MAX_FREQUENCY;
    use crate::charts::ChartScale;

    #[test]
    fn exact_reference_interpolation_and_ranks() {
        let reference: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/chart-arithmetic-reference.json"
        ))
        .unwrap();
        for case in reference["interpolation"].as_array().unwrap() {
            let value = |key: &str| f64::from_bits(case[key].as_u64().unwrap());
            let left = value("left_bits");
            let right = value("right_bits");
            let actual = interpolate(
                left,
                right,
                case["numerator"].as_u64().unwrap(),
                case["denominator"].as_u64().unwrap(),
            )
            .unwrap();
            assert!(actual.is_finite() && actual >= left.min(right) && actual <= left.max(right));
            // Exact rational reference: zero ULP tolerance (signed zeros compare equal).
            assert_eq!(actual, value("expected_bits"), "{case}");
        }
        for case in reference["ranks"].as_array().unwrap() {
            let actual = quartile_rank(
                case["n"].as_u64().unwrap(),
                case["quarter"].as_u64().unwrap() as u8,
            )
            .unwrap();
            assert_eq!(
                actual,
                (
                    case["lower"].as_u64().unwrap(),
                    case["numerator"].as_u64().unwrap(),
                    4
                ),
                "{case}"
            );
        }
        let (mut max_scale_ulps, mut max_scale_absolute) = (0, 0.0f64);
        for case in reference["scale"].as_array().unwrap() {
            let value = |key: &str| f64::from_bits(case[key].as_u64().unwrap());
            let actual = ChartScale::new(value("lo_bits"), value("hi_bits"))
                .unwrap()
                .position(value("value_bits"))
                .unwrap();
            let expected = value("expected_bits");
            max_scale_ulps = max_scale_ulps.max(actual.to_bits().abs_diff(expected.to_bits()));
            max_scale_absolute = max_scale_absolute.max((actual - expected).abs());
            // Subtraction and division may each round: at most two ULPs in unit space.
            assert!(actual.is_finite() && (0.0..=1.0).contains(&actual));
            assert!(
                actual.to_bits().abs_diff(expected.to_bits()) <= 2,
                "scale {case}: {actual} vs {expected}"
            );
        }
        println!("1812 interpolation cases: 0 ULP / 0 absolute error; 30 exact ranks; 1800 scale cases: max {max_scale_ulps} ULP / {max_scale_absolute:e} absolute error");
    }

    #[test]
    fn convex_endpoints_extremes_and_adjacent_values() {
        assert_eq!(interpolate(-f64::MAX, f64::MAX, 1, 2).unwrap(), 0.0);
        for (lo, hi) in [
            (f64::MAX.next_down(), f64::MAX),
            (f64::from_bits(1), f64::from_bits(2)),
            (-17.0, -17.0),
        ] {
            assert_eq!(interpolate(lo, hi, 0, 4).unwrap().to_bits(), lo.to_bits());
            assert_eq!(interpolate(lo, hi, 4, 4).unwrap().to_bits(), hi.to_bits());
            let values = (0..=4)
                .map(|n| interpolate(lo, hi, n, 4).unwrap())
                .collect::<Vec<_>>();
            assert!(values.windows(2).all(|p| p[0] <= p[1]));
        }
        assert_eq!(
            interpolate(-0.0, 0.0, 0, 1).unwrap().to_bits(),
            (-0.0f64).to_bits()
        );
        assert_eq!(interpolate(1.0, 1.0, u64::MAX - 1, u64::MAX).unwrap(), 1.0);
        assert_eq!(
            interpolate(-f64::MAX, f64::MAX, u64::MAX, u64::MAX).unwrap(),
            f64::MAX
        );
    }

    #[test]
    fn signed_zero_endpoints_negative_to_positive() {
        for (numerator, expected) in [(0, -0.0f64), (1, 0.0)] {
            assert_eq!(
                interpolate(-0.0, 0.0, numerator, 1).unwrap().to_bits(),
                expected.to_bits()
            );
        }
    }

    #[test]
    fn signed_zero_endpoints_positive_to_negative() {
        for (numerator, expected) in [(0, 0.0f64), (1, -0.0)] {
            assert_eq!(
                interpolate(0.0, -0.0, numerator, 1).unwrap().to_bits(),
                expected.to_bits()
            );
        }
    }

    #[test]
    fn exact_integer_bin_rank_and_display_decisions() {
        assert_eq!(sturges_bins(0).unwrap(), 0);
        assert_eq!(sturges_bins(1).unwrap(), 1);
        for power in 1..=53 {
            let n = 1u64 << power;
            assert_eq!(sturges_bins(n).unwrap(), power as usize + 1);
            if n < MAX_FREQUENCY {
                assert_eq!(sturges_bins(n + 1).unwrap(), power as usize + 2);
            }
            assert_eq!(
                sturges_bins(n - 1).unwrap(),
                if power == 1 { 1 } else { power as usize + 1 }
            );
        }
        assert_eq!(
            quartile_rank(MAX_FREQUENCY, 3).unwrap(),
            (6_755_399_441_055_743, 1, 4)
        );
        assert_eq!(display_index(2001, 2000, 0).unwrap(), 0);
        assert_eq!(display_index(2001, 2000, 1999).unwrap(), 2000);
        assert_eq!(display_index(5, 3, 1).unwrap(), 2);
        assert_eq!(display_index(5, 1, 0).unwrap(), 0);
        // Dimensionless values may exceed the owned-byte ceiling.
        assert_eq!(display_index(usize::MAX, 2, 1).unwrap(), usize::MAX - 1);
        assert!(display_index(usize::MAX, 3, 2).is_err());
    }

    #[test]
    fn unit_scales_preserve_small_spreads_and_overflowing_spans() {
        for (lo, mid, hi) in [
            (-f64::MAX, 0.0, f64::MAX),
            (f64::from_bits(1), f64::from_bits(2), f64::from_bits(3)),
            (1e300, 1e300f64.next_up(), 1e300f64.next_up().next_up()),
            (0.0, 0.5, 1.0),
        ] {
            let scale = ChartScale::new(lo, hi).unwrap();
            assert_eq!(scale.position(lo).unwrap(), 0.0);
            assert_eq!(scale.position(mid).unwrap(), 0.5);
            assert_eq!(scale.position(hi).unwrap(), 1.0);
            assert!(scale.position(lo.next_down()).is_err());
            assert!(scale.position(hi.next_up()).is_err());
        }
        assert_eq!(
            ChartScale::new(7.0, 7.0).unwrap().position(7.0).unwrap(),
            0.5
        );
        assert_eq!(
            ChartScale::new(f64::MAX.next_down(), f64::MAX)
                .unwrap()
                .position(f64::MAX)
                .unwrap(),
            1.0
        );
    }

    #[test]
    fn impossible_arithmetic_arguments_are_rejected() {
        for (lo, hi, n, d) in [
            (0.0, 1.0, 1, 0),
            (0.0, 1.0, 2, 1),
            (f64::NAN, 1.0, 0, 1),
            (0.0, f64::INFINITY, 0, 1),
        ] {
            assert!(interpolate(lo, hi, n, d).is_err());
        }
        for (n, q) in [(0, 1), (1, 0), (1, 4), (MAX_FREQUENCY + 1, 1)] {
            assert!(quartile_rank(n, q).is_err());
        }
        assert!(sturges_bins(MAX_FREQUENCY + 1).is_err());
        for args in [(0, 0, 0), (1, 0, 0), (1, 2, 0), (5, 3, 3)] {
            assert!(display_index(args.0, args.1, args.2).is_err());
        }
        for (lo, hi) in [(2.0, 1.0), (f64::NAN, 1.0), (0.0, f64::INFINITY)] {
            assert!(ChartScale::new(lo, hi).is_err());
        }
        let scale = ChartScale::new(1.0, 1.0).unwrap();
        assert!(scale.position(2.0).is_err());
        assert!(scale.position(f64::NAN).is_err());
    }
}
