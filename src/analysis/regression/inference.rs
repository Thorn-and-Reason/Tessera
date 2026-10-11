//! I_z(df2/2, df1/2), increasing the second beta shape by positive terms.
use crate::{
    analysis::{
        inference::{
            arithmetic::TailFloat as D, precise_normalization, student_t_log_tail_precise,
            StudentLogTail,
        },
        Estimate,
    },
    error::{AppError, AppResult},
    operation::JobControl,
};

pub(crate) fn f_upper_tail(
    log_f: f64,
    df1: u64,
    df2: u64,
    control: &JobControl,
) -> AppResult<Estimate> {
    let quiet = control.without_progress();
    quiet.checkpoint(0, None)?;
    let result = if !log_f.is_finite() || !(1..=63).contains(&df1) || !(1..=1 << 53).contains(&df2)
    {
        Err(AppError::invalid("F tail", "A finite log statistic, 1–63 numerator degrees and 1–2^53 denominator degrees are required."))
    } else {
        calculate(log_f, df1, df2, &quiet)
    };
    quiet.checkpoint(0, None)?;
    result
}

fn calculate(log_f: f64, df1: u64, df2: u64, control: &JobControl) -> AppResult<Estimate> {
    let b = D(df2 as f64 * 0.5, 0.0);
    let log_numerator = D(log_f, 0.0).add(D(df1 as f64, 0.0).ln());
    let ratio = log_numerator.sub(D(df2 as f64, 0.0).ln());
    let log_z = softplus(ratio).neg();
    let log_q = softplus(ratio.neg()).neg();
    let odd = df1 % 2 == 1;
    let mut shape = if odd { 0.5 } else { 1.0 };
    let base = if odd {
        student_t_log_tail_precise(log_numerator.mul(D(0.5, 0.0)), None, df2 as f64, control)?
    } else {
        let log = b.mul(log_z);
        Ok(StudentLogTail {
            log_hi: log.0,
            log_lo: log.1,
            uncertainty: rounding(log.0),
        })
    };
    control.checkpoint(0, None)?;
    let mut sum = match base {
        Ok(base) => base,
        Err(reason) => return Ok(reason),
    };
    let coefficient = if odd {
        precise_normalization(df2 as f64)
            .add(D(df2 as f64, 0.0).ln().mul(D(0.5, 0.0)))
            .add(D::LN_2)
    } else {
        b.ln()
    };
    let mut increment = b.mul(log_z).add(D(shape, 0.0).mul(log_q)).add(coefficient);
    let mut uncertainty = rounding(increment.0) + if odd { 2e-19 } else { 0.0 };
    let mut bound = LowerTailBound::new(b, odd);
    while shape < df1 as f64 * 0.5 {
        control.checkpoint(0, None)?;
        sum = positive_sum(sum, increment, uncertainty);
        if log_q.0 < -f64::MAX / (shape + 1.0) {
            // All remaining positive increments are far below rounding range.
            break;
        }
        let numerator = b.add(D(shape, 0.0)).ln();
        let denominator = D(shape + 1.0, 0.0).ln();
        increment = increment.add(log_q).add(numerator).sub(denominator);
        bound = bound.advance(numerator, denominator);
        uncertainty += rounding(increment.0);
        shape += 1.0;
    }
    if bound.rounds_to_one(df1 as f64 * 0.5, b, log_q, log_z) {
        return Ok(Estimate::available(1.0));
    }
    let estimate = sum.probability();
    if estimate.value.is_some_and(|value| {
        value < f64::MIN_POSITIVE && sum.uncertainty > 0.5 * (f64::from_bits(1) / value)
    }) {
        return Ok(Estimate::unavailable(
            "F-tail uncertainty exceeds the subnormal precision limit.",
        ));
    }
    Ok(estimate)
}

struct LowerTailBound {
    coefficient: D,
    scale: f64,
}
impl LowerTailBound {
    fn new(b: D, odd: bool) -> Self {
        // Jensen gives Gamma(b+1/2)/Gamma(b) <= sqrt(b); 2/sqrt(pi) < 2.
        let coefficient = if odd {
            D::LN_2.add(b.ln().mul(D(0.5, 0.0)))
        } else {
            b.ln()
        };
        Self {
            coefficient,
            scale: 1.0 + coefficient.0.abs(),
        }
    }
    fn advance(self, numerator: D, denominator: D) -> Self {
        Self {
            coefficient: self.coefficient.add(numerator).sub(denominator),
            scale: self.scale + numerator.0.abs() + denominator.0.abs(),
        }
    }
    fn log_bound(&self, a: f64, b: D, log_q: D, log_z: D) -> (D, f64) {
        // Integrate t^(a-1)(1-t)^(b-1) on [0,q], bounding the second factor.
        let power = log_q.mul(D(a, 0.0));
        let endpoint = log_z.mul(D((b.0 - 1.0).min(0.0), 0.0));
        let bound = power.add(self.coefficient).add(endpoint);
        let margin = 1024.0 * f64::EPSILON * (self.scale + power.0.abs() + endpoint.0.abs());
        (bound, margin)
    }
    fn rounds_to_one(&self, a: f64, b: D, log_q: D, log_z: D) -> bool {
        let (bound, margin) = self.log_bound(a, b, log_q, log_z);
        bound.0.is_finite()
            && margin.is_finite()
            && bound.add(D(margin, 0.0)).sub(D::LN_2.mul(D(-54.0, 0.0))).0 < 0.0
    }
}

fn softplus(x: D) -> D {
    if x.0 > 0.0 {
        x.add(x.neg().exp().ln_1p())
    } else {
        x.exp().ln_1p()
    }
}
fn rounding(log: f64) -> f64 {
    128.0 * f64::EPSILON.powi(2) * (1.0 + log.abs())
}
fn positive_sum(left: StudentLogTail, right: D, right_error: f64) -> StudentLogTail {
    let left_log = D(left.log_hi, left.log_lo);
    let (large, small) = if left_log.0 > right.0 {
        (left_log, right)
    } else {
        (right, left_log)
    };
    let log = large.add(small.sub(large).exp().ln_1p());
    // Weight relative uncertainty by each positive contribution.
    let left_weight = left_log.sub(log).exp().0;
    let right_weight = right.sub(log).exp().0;
    StudentLogTail {
        log_hi: log.0,
        log_lo: log.1,
        uncertainty: left_weight * left.uncertainty + right_weight * right_error + rounding(log.0),
    }
}

#[cfg(test)]
mod tests;

use super::{
    solver::{
        scaling::{estimate as scaled_estimate, log as scaled_log},
        ScaledFit,
    },
    RegressionCoefficient, RegressionStatistics,
};
use crate::analysis::{inference::student_t_critical, moments::Scaled, ConfidenceLevel};
const NO_DF: &str = "There are no residual degrees of freedom.";
const ZERO_VARIANCE: &str = "Residual variance is exactly zero.";
/// The caller precharges kernel_work_bytes, including the single shared workspace.
pub(super) fn infer(
    fit: &ScaledFit,
    confidence: ConfidenceLevel,
    control: &JobControl,
) -> AppResult<(Vec<RegressionCoefficient>, RegressionStatistics)> {
    let quiet = control.without_progress();
    quiet.checkpoint(0, None)?;
    let result = infer_inner(fit, confidence, &quiet);
    quiet.checkpoint(0, None)?;
    result
}
fn infer_inner(
    fit: &ScaledFit,
    confidence: ConfidenceLevel,
    control: &JobControl,
) -> AppResult<(Vec<RegressionCoefficient>, RegressionStatistics)> {
    let df = fit.residual_df();
    let critical = if df == 0 {
        Estimate::unavailable(NO_DF)
    } else if fit.residual_norm() == 0. {
        Estimate::available(0.)
    } else {
        student_t_critical(df as f64, confidence, control)?
    };
    let mut coefficients = Vec::with_capacity(fit.rank());
    for (j, b) in fit.coefficients().iter().copied().enumerate() {
        control.checkpoint(j, None)?;
        coefficients.push(coefficient(
            b,
            (df > 0).then(|| fit.standard_error(j)),
            &critical,
            df,
            control,
        )?);
    }
    let e = fit.residual_norm();
    let y = fit.total_norm();
    let p = fit.explained_norm();
    let mut statistics = RegressionStatistics {
        sse: scaled_estimate(fit.energy(e)),
        sst: scaled_estimate(fit.energy(y)),
        model_ss: scaled_estimate(fit.energy(p)),
        residual_standard_error: Estimate::unavailable(NO_DF),
        r_squared: Estimate::unavailable("The outcome has no variation."),
        adjusted_r_squared: Estimate::unavailable("The outcome has no variation."),
        f: Estimate::unavailable(if df == 0 { NO_DF } else { ZERO_VARIANCE }),
        f_p: Estimate::unavailable(if df == 0 { NO_DF } else { ZERO_VARIANCE }),
    };
    if y > 0. {
        statistics.r_squared = scaled_estimate(super::solver::scaling::multiply(
            Scaled::new(p).ratio(Scaled::new(y)),
            Scaled::new(p).ratio(Scaled::new(y)),
        ));
    }
    if df == 0 {
        statistics.adjusted_r_squared = Estimate::unavailable(NO_DF);
    } else {
        statistics.residual_standard_error = scaled_estimate(super::solver::scaling::multiply(
            fit.outcome_unit(),
            Scaled::new(e * (fit.frequency() as f64 / df as f64).sqrt()),
        ));
        if y > 0. {
            statistics.adjusted_r_squared = Estimate::available(
                1. - (e / y).powi(2) * ((fit.frequency() - 1) as f64 / df as f64),
            );
        }
        (statistics.f, statistics.f_p) = f_inference(p, e, (fit.rank() - 1) as u64, df, control)?;
    }
    Ok((coefficients, statistics))
}

fn coefficient(
    b: Scaled,
    se: Option<Scaled>,
    critical: &Estimate,
    df: u64,
    control: &JobControl,
) -> AppResult<RegressionCoefficient> {
    control.checkpoint(0, None)?;
    let missing = || Estimate::unavailable(if df == 0 { NO_DF } else { ZERO_VARIANCE });
    let mut c = RegressionCoefficient {
        estimate: scaled_estimate(b),
        standard_error: missing(),
        t: missing(),
        p: missing(),
        interval: [missing(), missing()],
    };
    if let Some(se) = se {
        c.standard_error = scaled_estimate(se);
        if let Some(critical) = critical.value {
            let margin = se.times(critical);
            c.interval = [
                scaled_estimate(b.plus(margin.times(-1.))),
                scaled_estimate(b.plus(margin)),
            ];
        } else {
            c.interval = [critical.clone(), critical.clone()];
        }
        if se.fraction != 0. {
            let t = b.ratio(se);
            c.t = scaled_estimate(t);
            c.p = if b.fraction == 0. {
                Estimate::available(1.)
            } else {
                match student_t_log_tail_precise(scaled_log(t), None, df as f64, control)? {
                    Ok(log) => log.probability(),
                    Err(reason) => reason,
                }
            };
        }
    }
    control.checkpoint(0, None)?;
    Ok(c)
}
fn f_inference(
    explained: f64,
    residual: f64,
    model_df: u64,
    df: u64,
    control: &JobControl,
) -> AppResult<(Estimate, Estimate)> {
    control.checkpoint(0, None)?;
    let result = if df == 0 || residual == 0. {
        let reason = if df == 0 { NO_DF } else { ZERO_VARIANCE };
        (Estimate::unavailable(reason), Estimate::unavailable(reason))
    } else if explained == 0. {
        (Estimate::available(0.), Estimate::available(1.))
    } else {
        let ratio = Scaled::new(explained).ratio(Scaled::new(residual));
        let f = super::solver::scaling::multiply(ratio, ratio).times(df as f64 / model_df as f64);
        let log = scaled_log(f);
        (
            scaled_estimate(f),
            f_upper_tail(log.0 + log.1, model_df, df, control)?,
        )
    };
    control.checkpoint(0, None)?;
    Ok(result)
}
