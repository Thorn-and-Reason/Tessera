//! Captured-scalar consistency only; no active data, factors or inference are rerun.
use super::*;
use crate::analysis::{moments::Scaled, regression::solver::scaling::multiply};
const DIRECT: f64 = 1e-10;

pub(super) fn relationships(r: &ComputedRegression, f: &RegressionFit) -> AppResult<()> {
    let condition = f.condition.value.ok_or_else(invalid_analysis)?;
    // The threshold product and singular-value ratio each round once; eight eps is conservative.
    if condition > (1. / r.method.rank_threshold) * (1. + 8. * f64::EPSILON)
        || f.term_scales[0].value != Some(1.)
    {
        return Err(invalid_analysis());
    }
    // Empirical saved-output allowance, not the unavailable internal T*D bound or an accuracy theorem.
    let energy = DIRECT.max(8. * f.rank as f64 * f64::EPSILON * condition);
    exact_states(r, f)?;
    energies(f, energy)?;
    variance_inference(f)?;
    coefficients_and_scales(r, f)?;
    summary(r, f, energy)
}
fn energies(f: &RegressionFit, energy: f64) -> AppResult<()> {
    let s = &f.statistics;
    if let (Some(sse), Some(sst), Some(model)) = (s.sse.value, s.sst.value, s.model_ss.value) {
        let tolerance = if sse == 0. { DIRECT } else { energy };
        positive_energy(
            Scaled::new(sse).plus(Scaled::new(model)),
            sst,
            tolerance,
            rounding(sse).max(rounding(model)),
        )?;
    }
    ratio(&s.r_squared, &s.model_ss, &s.sst, 1.)?;
    if let (Some(r2), Some(sse), Some(sst)) = (s.r_squared.value, s.sse.value, s.sst.value) {
        if sst > 0. {
            offset(r2, sse, sst, 1., energy)?;
        }
    }
    Ok(())
}
fn variance_inference(f: &RegressionFit) -> AppResult<()> {
    let s = &f.statistics;
    if f.residual_df > 0 {
        ratio(
            &s.f,
            &s.model_ss,
            &s.sse,
            f.residual_df as f64 / f.model_df as f64,
        )?;
        if let (Some(adjusted), Some(sse), Some(sst)) =
            (s.adjusted_r_squared.value, s.sse.value, s.sst.value)
        {
            if sst > 0. {
                offset(
                    adjusted,
                    sse,
                    sst,
                    f.total_df as f64 / f.residual_df as f64,
                    DIRECT,
                )?;
            }
        }
        if let (Some(spread), Some(sse)) = (s.residual_standard_error.value, s.sse.value) {
            let squared =
                multiply(Scaled::new(spread), Scaled::new(spread)).times(f.residual_df as f64);
            positive_energy(squared, sse, DIRECT, squared_rounding(spread))?;
        }
    }
    Ok(())
}
fn coefficients_and_scales(r: &ComputedRegression, f: &RegressionFit) -> AppResult<()> {
    let s = &f.statistics;
    if let (Some(unit), Some(sst)) = (f.outcome_scale.value, s.sst.value) {
        if sst > 0. {
            let energy = multiply(Scaled::new(unit), Scaled::new(unit)).times(r.frequency as f64);
            positive_energy(energy, sst, DIRECT, squared_rounding(unit))?;
        } else {
            check(unit == 1.)?;
        }
    }
    for c in &f.coefficients {
        if c.standard_error.value == Some(0.) {
            check(s.sse.value == Some(0.))?;
        }
        ratio(&c.t, &c.estimate, &c.standard_error, 1.)?;
        if let (Some(lo), Some(hi), Some(b)) =
            (c.interval[0].value, c.interval[1].value, c.estimate.value)
        {
            check(interval_centre_close(lo, hi, b))?;
        }
    }
    Ok(())
}

fn check(valid: bool) -> AppResult<()> {
    if valid {
        Ok(())
    } else {
        Err(invalid_analysis())
    }
}
fn positive_energy(
    expected: Scaled,
    published: f64,
    tolerance: f64,
    input_rounding: f64,
) -> AppResult<()> {
    if expected.fraction == 0. || published == 0. {
        return check(expected.fraction == 0. && published == 0.);
    }
    check(expected.fraction > 0. && published > 0.)?;
    let observed = Scaled::new(published).ratio(expected).value();
    let bound = 1. + tolerance + 2. * (input_rounding + rounding(published));
    check(observed >= 1. / bound && observed <= bound)
}
fn squared_rounding(value: f64) -> f64 {
    if value == 0. {
        return 0.;
    }
    let r = rounding(value);
    2. * r + r * r
}
fn interval_centre_close(lo: f64, hi: f64, coefficient: f64) -> bool {
    let lower = Scaled::new(lo);
    let upper = Scaled::new(hi);
    let sum = lower.plus(upper);
    let centre = Scaled::new(coefficient).times(2.);
    let exponent = lower
        .exponent
        .max(upper.exponent)
        .max(sum.exponent)
        .max(centre.exponent)
        .max(-1074);
    let at = |s: Scaled| s.fraction * 2_f64.powi(s.exponent - exponent);
    // Preserve endpoint publication rounding before their signed sum cancels.
    let endpoints = 2. * (rounding(lo) * at(lower).abs() + rounding(hi) * at(upper).abs());
    let relative = DIRECT * at(sum).abs().max(at(centre).abs());
    let smallest = 4. * 2_f64.powi(-1074 - exponent);
    (at(sum) - at(centre)).abs() <= relative + endpoints + smallest
}
fn rounding(value: f64) -> f64 {
    if value == 0. || value.abs() >= f64::MIN_POSITIVE {
        f64::EPSILON
    } else {
        f64::from_bits(1) / value.abs()
    }
}
fn ratio(
    value: &Estimate,
    numerator: &Estimate,
    denominator: &Estimate,
    factor: f64,
) -> AppResult<()> {
    if let (Some(value), Some(a), Some(b)) = (value.value, numerator.value, denominator.value) {
        if b > 0. {
            check(
                (value == 0.) == (a == 0.)
                    && (a == 0. || value.is_sign_positive() == a.is_sign_positive()),
            )?;
            if a != 0. {
                let expected = Scaled::new(a).ratio(Scaled::new(b)).times(factor);
                let observed = Scaled::new(value).ratio(expected).value();
                // Finite factor bounds cannot be disabled by a one-ULP denominator.
                let bound = 1. + DIRECT + 2. * (rounding(a) + rounding(b) + rounding(value));
                check(observed >= 1. / bound && observed <= bound)?;
            }
        }
    }
    Ok(())
}
fn offset(
    value: f64,
    numerator: f64,
    denominator: f64,
    factor: f64,
    tolerance: f64,
) -> AppResult<()> {
    let quotient = Scaled::new(numerator)
        .ratio(Scaled::new(denominator))
        .times(factor);
    // A dimensionless subtraction from one has an absolute unit-rounding allowance.
    let expected = Scaled::new(1.).plus(quotient.times(-1.));
    let value = Scaled::new(value);
    let exponent = value
        .exponent
        .max(expected.exponent)
        .max(quotient.exponent)
        .max(0);
    let at = |s: Scaled| s.fraction * 2_f64.powi(s.exponent - exponent);
    let scale = at(quotient).abs().max(2_f64.powi(-exponent));
    let allowance = tolerance + 2. * (rounding(numerator) + rounding(denominator));
    check((at(value) - at(expected)).abs() <= allowance * scale)
}
fn exact_states(r: &ComputedRegression, f: &RegressionFit) -> AppResult<()> {
    let s = &f.statistics;
    let d = &f.residual_diagnostics;
    if s.sst.value == Some(0.) {
        check(s.sse.value == Some(0.) && s.model_ss.value == Some(0.))?;
    }
    if s.sse.value == Some(0.) {
        if f.residual_df > 0 {
            check(s.residual_standard_error.value == Some(0.))?;
        }
        check(d.residual_mean.value == Some(0.) && d.residual_sample_sd.value == Some(0.))?;
        for e in d
            .residual_range
            .iter()
            .chain(d.residual_points.iter().map(|p| &p.residual))
            .chain(d.qq_points.iter().map(|p| &p.residual))
        {
            check(e.value == Some(0.))?;
        }
        if s.sst.value != Some(0.) {
            check(s.r_squared.value == Some(1.))?;
            if f.residual_df > 0 {
                check(s.adjusted_r_squared.value == Some(1.))?;
            }
        }
    }
    if f.residual_df == 0 || s.sse.value == Some(0.) {
        for extent in [&d.studentised, &d.cooks_distance] {
            check(
                extent.undefined_rows == r.fitted_rows && extent.undefined_frequency == r.frequency,
            )?;
        }
    }
    Ok(())
}
fn summary(r: &ComputedRegression, f: &RegressionFit, tolerance: f64) -> AppResult<()> {
    let d = &f.residual_diagnostics;
    inside(&d.residual_mean, &d.residual_range)?;
    if let (Some(sd), Some(mean), Some(sse)) = (
        d.residual_sample_sd.value,
        d.residual_mean.value,
        f.statistics.sse.value,
    ) {
        check(sse == 0. || sd != 0. || mean != 0.)?;
        let centred = multiply(Scaled::new(sd), Scaled::new(sd)).times((r.frequency - 1) as f64);
        let centre = multiply(Scaled::new(mean), Scaled::new(mean)).times(r.frequency as f64);
        positive_energy(
            centred.plus(centre),
            sse,
            tolerance,
            squared_rounding(sd).max(squared_rounding(mean)),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::ConfidenceLevel;
    fn near_zero_capture(signal: f64, confidence: ConfidenceLevel) -> ComputedRegression {
        use crate::{
            analysis::{regression_with_budget, RowFilter},
            data::{Column, Dataset},
            operation::JobControl,
        };
        let x = [-2., -1., 1., 2.];
        let y = [1., -1., -1., 1.];
        let data = Dataset::new(vec![
            Column::numeric(
                ColumnId(1),
                "Y",
                y.into_iter()
                    .zip(x)
                    .map(|(y, x)| Some(y + signal * x))
                    .collect(),
            )
            .unwrap(),
            Column::numeric(ColumnId(2), "X", x.map(Some).to_vec()).unwrap(),
        ])
        .unwrap();
        let request = RegressionRequest {
            outcome: ColumnId(1),
            predictors: vec![RegressionPredictor {
                column: ColumnId(2),
                role: PredictorRole::Numeric,
            }],
            confidence,
            filter: RowFilter::default(),
            frequency_weight: None,
        };
        regression_with_budget(
            &data,
            &request,
            &JobControl::default(),
            &[],
            request.heap_bytes().unwrap(),
        )
        .unwrap_or_else(|error| panic!("signal={signal}, confidence={confidence:?}: {error:?}"))
    }
    #[test]
    fn actual_near_zero_coefficients_preserve_rounded_interval_centres() {
        for confidence in [
            ConfidenceLevel::P90,
            ConfidenceLevel::P95,
            ConfidenceLevel::P99,
        ] {
            for signal in [0., 1e-14, 1e-10, 1e-6, 0.01] {
                let capture = near_zero_capture(signal, confidence);
                assert!(capture.validate().is_ok());
                let RegressionOutcome::Fitted(fit) = &capture.outcome else {
                    panic!("expected fitted signal={signal}")
                };
                assert!(fit
                    .coefficients
                    .iter()
                    .all(|c| c.interval.iter().all(|e| e.value.is_some())));
            }
        }
    }
    #[test]
    fn actual_interval_with_deliberately_displaced_centre_is_refused() {
        let mut capture = near_zero_capture(0., ConfidenceLevel::P95);
        let RegressionOutcome::Fitted(fit) = &mut capture.outcome else {
            panic!()
        };
        let c = &mut fit.coefficients[1];
        for endpoint in &mut c.interval {
            *endpoint = Estimate::available(endpoint.value.unwrap() + 1e-6);
        }
        assert!(capture.validate().is_err());
    }
    #[test]
    fn actual_rounded_qq_ties_still_refuse_descending_published_residuals() {
        let mut capture = near_zero_capture(0.01, ConfidenceLevel::P95);
        let RegressionOutcome::Fitted(fit) = &mut capture.outcome else {
            panic!()
        };
        // The live residuals may round a few ULPs apart on another OS.
        // Publish the rounded tie explicitly and retain each source identity.
        let rows = [
            fit.residual_diagnostics.qq_points[2].row,
            fit.residual_diagnostics.qq_points[3].row,
        ];
        for point in &mut fit.residual_diagnostics.residual_points {
            if rows.contains(&point.row) {
                point.residual = Estimate::available(1.);
            }
        }
        let q = &mut fit.residual_diagnostics.qq_points;
        q[2].row = rows[0].max(rows[1]);
        q[3].row = rows[0].min(rows[1]);
        q[2].residual = Estimate::available(1.);
        q[3].residual = Estimate::available(1.);
        fit.residual_diagnostics.residual_range[1] = Estimate::available(1.);
        assert!(capture.validate().is_ok());
        let RegressionOutcome::Fitted(fit) = &mut capture.outcome else {
            panic!()
        };
        let q = &mut fit.residual_diagnostics.qq_points;
        assert!(q[0].residual.value < q[3].residual.value);
        // Keep each rank/theoretical position, but invert its captured source and residual.
        let first = q[0].clone();
        q[0].row = q[3].row;
        q[0].residual = q[3].residual.clone();
        q[3].row = first.row;
        q[3].residual = first.residual;
        assert!(capture.validate().is_err());
    }
    #[test]
    fn actual_exact_zero_qq_ties_still_require_source_row_order() {
        let mut capture = scaled_capture(0.);
        let RegressionOutcome::Fitted(fit) = &mut capture.outcome else {
            panic!()
        };
        assert_eq!(fit.statistics.sse.value, Some(0.));
        let q = &mut fit.residual_diagnostics.qq_points;
        assert_eq!(q[0].residual.value, Some(0.));
        assert_eq!(q[1].residual.value, Some(0.));
        let first_row = q[0].row;
        q[0].row = q[1].row;
        q[1].row = first_row;
        assert!(capture.validate().is_err());
    }
    fn scaled_capture(unit: f64) -> ComputedRegression {
        use crate::{
            analysis::{regression_with_budget, ConfidenceLevel, RowFilter},
            data::{Column, Dataset},
            operation::JobControl,
        };
        let data = Dataset::new(vec![
            Column::numeric(
                ColumnId(1),
                "Y",
                [1., 2., 2., 4.].map(|y| Some(y * unit)).to_vec(),
            )
            .unwrap(),
            Column::numeric(ColumnId(2), "X", [0., 1., 2., 3.].map(Some).to_vec()).unwrap(),
        ])
        .unwrap();
        let request = RegressionRequest {
            outcome: ColumnId(1),
            predictors: vec![RegressionPredictor {
                column: ColumnId(2),
                role: PredictorRole::Numeric,
            }],
            confidence: ConfidenceLevel::P95,
            filter: RowFilter::default(),
            frequency_weight: None,
        };
        regression_with_budget(
            &data,
            &request,
            &JobControl::default(),
            &[],
            request.heap_bytes().unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn available_scaling_unit_cannot_be_false_zero() {
        let mut bad = scaled_capture(1e-200);
        let RegressionOutcome::Fitted(f) = &mut bad.outcome else {
            panic!()
        };
        assert!(f.statistics.sst.value.is_none());
        f.outcome_scale = Estimate::available(0.);
        assert!(bad.validate().is_err());
    }
    #[test]
    fn positive_sse_cannot_have_zero_mean_and_sample_spread() {
        let mut bad = scaled_capture(3e-162);
        let RegressionOutcome::Fitted(f) = &mut bad.outcome else {
            panic!()
        };
        assert_eq!(f.statistics.sse.value, Some(f64::from_bits(1)));
        f.residual_diagnostics.residual_mean = Estimate::available(0.);
        f.residual_diagnostics.residual_sample_sd = Estimate::available(0.);
        assert!(bad.validate().is_err());
    }
    #[test]
    fn positive_subnormal_sst_cannot_have_a_minimal_scaling_unit() {
        let mut bad = scaled_capture(1e-162);
        let RegressionOutcome::Fitted(f) = &mut bad.outcome else {
            panic!()
        };
        assert_eq!(f.statistics.sst.value, Some(f64::from_bits(1)));
        f.outcome_scale = Estimate::available(f64::from_bits(1));
        assert!(bad.validate().is_err());
    }
    #[test]
    fn positive_subnormal_sse_cannot_have_a_minimal_spread_and_zero_mean() {
        let mut bad = scaled_capture(3e-162);
        let RegressionOutcome::Fitted(f) = &mut bad.outcome else {
            panic!()
        };
        assert_eq!(f.statistics.sse.value, Some(f64::from_bits(1)));
        f.residual_diagnostics.residual_mean = Estimate::available(0.);
        f.residual_diagnostics.residual_sample_sd = Estimate::available(f64::from_bits(1));
        assert!(bad.validate().is_err());
    }
    #[test]
    fn forged_negative_adjustment_cannot_inflate_its_own_offset_allowance() {
        let tiny = f64::from_bits(1);
        assert!(offset(-f64::MAX, tiny, tiny, 1., DIRECT).is_err());
        assert!(offset(0., tiny, tiny, 1., DIRECT).is_ok());
    }
    #[test]
    fn extreme_ratio_cannot_use_subnormal_rounding_to_admit_arbitrary_magnitude() {
        let large = Estimate::available(f64::MAX);
        let tiny = Estimate::available(f64::from_bits(1));
        assert!(ratio(&large, &large, &tiny, 1.).is_err());
        assert!(ratio(&tiny, &tiny, &large, 1.).is_err());
        assert!(ratio(
            &Estimate::unavailable("The estimate is outside finite Float64 range."),
            &large,
            &tiny,
            1.
        )
        .is_ok());
    }
    #[test]
    fn rounded_ratio_allowance_never_changes_known_zero_or_sign() {
        let unit = Estimate::available(1.);
        let tiny = f64::from_bits(1);
        for (published, numerator) in [(0., tiny), (tiny, 0.), (tiny, -tiny)] {
            assert!(ratio(
                &Estimate::available(published),
                &Estimate::available(numerator),
                &unit,
                1.
            )
            .is_err());
        }
        for (published, numerator) in [(0., 0.), (tiny, tiny), (-tiny, -tiny)] {
            assert!(ratio(
                &Estimate::available(published),
                &Estimate::available(numerator),
                &unit,
                1.
            )
            .is_ok());
        }
        assert!(ratio(
            &Estimate::unavailable("A nonzero estimate is below Float64 range."),
            &Estimate::available(tiny),
            &Estimate::available(f64::MAX),
            1.
        )
        .is_ok());
    }
}
