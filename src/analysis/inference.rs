//! Student-specific inference. Callers must precharge INFERENCE_WORK_BYTES in
//! their aggregate retained-work budget before entering these bounded-stack routines.
pub(crate) mod arithmetic;
use super::{quadrature, ConfidenceLevel, Estimate};
use crate::{
    error::{AppError, AppResult},
    operation::JobControl,
};
use arithmetic::TailFloat;
use std::f64::consts::{LN_2, PI};
pub(crate) const INFERENCE_WORK_BYTES: usize = 8192;
const UNDERFLOW: &str = "The probability is below Float64 range.";
const LIMIT: &str = "Student-t integration exceeded its numerical work or precision limit.";
pub(crate) fn student_t_two_sided_p(t: f64, df: f64, control: &JobControl) -> AppResult<Estimate> {
    let result = (|| {
        validate(df, control)?;
        if !t.is_finite() {
            return Err(AppError::invalid(
                "Student-t",
                "The statistic must be finite.",
            ));
        }
        if t == 0.0 {
            return Ok(Estimate::available(1.0));
        }
        tail(t.abs().ln(), Some(t.abs()), df, control)
    })();
    control.check_cancelled()?;
    result
}
pub(crate) fn student_t_two_sided_p_from_log(
    log_abs_t: f64,
    df: f64,
    control: &JobControl,
) -> AppResult<Estimate> {
    let result = (|| {
        validate(df, control)?;
        if !log_abs_t.is_finite() {
            return Err(AppError::invalid(
                "Student-t",
                "The log magnitude must be finite.",
            ));
        }
        tail(log_abs_t, None, df, control)
    })();
    control.check_cancelled()?;
    result
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct StudentLogTail {
    pub(crate) log_hi: f64,
    pub(crate) log_lo: f64,
    pub(crate) uncertainty: f64,
}
impl StudentLogTail {
    pub(crate) fn probability(self) -> Estimate {
        let log_p = TailFloat(self.log_hi, self.log_lo);
        let boundary = TailFloat::LN_2.mul(TailFloat(-1075.0, 0.0));
        let distance = log_p.sub(boundary);
        if self.log_hi == f64::NEG_INFINITY {
            return Estimate::unavailable(UNDERFLOW);
        }
        if !self.log_hi.is_finite()
            || !self.log_lo.is_finite()
            || !self.uncertainty.is_finite()
            || self.uncertainty < 0.0
        {
            return Estimate::unavailable(LIMIT);
        }
        if distance.0 + self.uncertainty < 0.0 {
            return Estimate::unavailable(UNDERFLOW);
        }
        if distance.0 - self.uncertainty <= 0.0 {
            return Estimate::unavailable(
                "The probability is too close to the Float64 rounding boundary.",
            );
        }
        if self.uncertainty > 1e-10 {
            return Estimate::unavailable(LIMIT);
        }
        if self.log_hi > 0.0 || (self.log_hi == 0.0 && self.log_lo > 0.0) {
            return Estimate::unavailable(LIMIT);
        }
        let value = log_p.exp().0;
        if value > 0.0 && value <= 1.0 {
            Estimate::available(value)
        } else {
            Estimate::unavailable(LIMIT)
        }
    }
}
fn tail(
    log_abs_t: f64,
    exact_t: Option<f64>,
    df: f64,
    control: &JobControl,
) -> AppResult<Estimate> {
    let result = evaluate_tail(TailFloat(log_abs_t, 0.0), exact_t, df, control)?;
    control.check_cancelled()?;
    Ok(match result {
        Ok(evaluation) => evaluation.probability,
        Err(reason) => reason,
    })
}
#[cfg(test)]
pub(crate) fn student_t_log_tail(
    log_abs_t: f64,
    exact_t: Option<f64>,
    df: f64,
    control: &JobControl,
) -> AppResult<Result<StudentLogTail, Estimate>> {
    student_t_log_tail_precise(TailFloat(log_abs_t, 0.0), exact_t, df, control)
}
pub(crate) fn student_t_log_tail_precise(
    log_t: TailFloat,
    exact_t: Option<f64>,
    df: f64,
    control: &JobControl,
) -> AppResult<Result<StudentLogTail, Estimate>> {
    evaluate_tail(log_t, exact_t, df, control)
        .map(|evaluation| evaluation.map(|evaluation| evaluation.log))
}
struct TailEvaluation {
    log: StudentLogTail,
    probability: Estimate,
}
fn evaluate_tail(
    log_t: TailFloat,
    exact_t: Option<f64>,
    df: f64,
    control: &JobControl,
) -> AppResult<Result<TailEvaluation, Estimate>> {
    let result = (|| {
        validate(df, control)?;
        if !log_t.0.is_finite()
            || !log_t.1.is_finite()
            || exact_t.is_some_and(|t| !t.is_finite() || t <= 0.0)
        {
            return Err(AppError::invalid(
                "Student-t",
                "The log magnitude and optional positive statistic must be finite.",
            ));
        }
        log_tail_inner(log_t, exact_t, df, control)
    })();
    control.check_cancelled()?;
    result.map(|tail| {
        tail.and_then(|evaluation| {
            let log = evaluation.log;
            if log.log_hi.is_finite()
                && log.log_lo.is_finite()
                && log.uncertainty.is_finite()
                && log.uncertainty >= 0.0
            {
                Ok(evaluation)
            } else {
                Err(Estimate::unavailable(LIMIT))
            }
        })
    })
}
fn log_tail_inner(
    log_t: TailFloat,
    exact_t: Option<f64>,
    df: f64,
    control: &JobControl,
) -> AppResult<Result<TailEvaluation, Estimate>> {
    let log_c = normalization(df);
    let a = (df + 1.0) * 0.5;
    if log_t.0 < LN_2 {
        let x_precise = exact_t.map_or_else(|| log_t.exp(), |t| TailFloat(t, 0.0));
        let x = exact_t.unwrap_or_else(|| log_t.0.exp());
        let integral =
            quadrature::integrate(|z| (-a * (x * x * z * z / df).ln_1p()).exp(), 1.0, control)?;
        return Ok(match integral {
            Some(j) => {
                let mass = precise_normalization(df)
                    .exp()
                    .mul(TailFloat(2.0, 0.0))
                    .mul(x_precise)
                    .mul(TailFloat(j.value, 0.0));
                let log = TailFloat::ONE.sub(mass).ln();
                Ok(TailEvaluation {
                    probability: Estimate::available(1.0 - 2.0 * log_c.exp() * x * j.value),
                    log: StudentLogTail {
                        log_hi: log.0,
                        log_lo: log.1,
                        uncertainty: mass.0 * (j.error / j.value + 32.0 * f64::EPSILON + 2e-19)
                            / (1.0 - mass.0),
                    },
                })
            }
            None => Err(Estimate::unavailable(LIMIT)),
        });
    }
    let q = 2.0 * log_t.0 - df.ln();
    let h = if q >= 0.0 {
        1.0 / (1.0 + (-q).exp())
    } else {
        q.exp() / (1.0 + q.exp())
    };
    let k = (df + 1.0) * h - 1.0;
    let integral = quadrature::integrate(
        |s| {
            let u = 2.0 * s / k;
            (s / k - a * (h * u.exp_m1()).ln_1p()).exp()
        },
        40.0,
        control,
    )?;
    let Some(j) = integral else {
        return Ok(Err(Estimate::unavailable(LIMIT)));
    };
    let softplus = if q > 0.0 {
        q + (-q).exp().ln_1p()
    } else {
        q.exp().ln_1p()
    };
    let prefactor = LN_2 + log_c + log_t.0 - k.ln();
    let exponent = a * softplus;
    let log_scale = LN_2 + log_c - k.ln() + j.value.ln();
    let uncertainty =
        j.error / j.value + 32.0 * f64::EPSILON * (1.0 + prefactor.abs() + exponent.abs());
    let ordinary = probability(log_t.0 - exponent, log_scale, uncertainty);
    let refine = ordinary.value.is_some() && log_t.0 - exponent + log_scale < -700.0;
    let log_t = exact_t.map_or(log_t, |t| TailFloat(t, 0.0).ln());
    let v = TailFloat(df, 0.0);
    let a = v.add(TailFloat::ONE).mul(TailFloat(0.5, 0.0));
    let q = log_t.mul(TailFloat(2.0, 0.0)).sub(v.ln());
    let softplus = if q.0 > 0.0 {
        q.add(q.neg().exp().ln_1p())
    } else {
        q.exp().ln_1p()
    };
    let log_p = TailFloat::LN_2
        .add(precise_normalization(df))
        .add(log_t)
        .sub(TailFloat(k, 0.0).ln())
        .sub(a.mul(softplus))
        .add(TailFloat(j.value, 0.0).ln());
    let log = StudentLogTail {
        log_hi: log_p.0,
        log_lo: log_p.1,
        uncertainty: j.error / j.value
            + 32.0 * f64::EPSILON
            + 2e-19
            + 128.0 * f64::EPSILON.powi(2) * (1.0 + a.0 * softplus.0.abs()),
    };
    let probability = if refine { log.probability() } else { ordinary };
    Ok(Ok(TailEvaluation { log, probability }))
}
pub(crate) fn precise_normalization(df: f64) -> TailFloat {
    let mut v = TailFloat(df, 0.0);
    let mut correction = TailFloat(0.0, 0.0);
    // C(v)=C(v+2)*sqrt(1+2/v)/(1+1/v); at most 64 shifts.
    while v.0 < 128.0 {
        correction = correction
            .add(TailFloat(2.0, 0.0).div(v).ln_1p().mul(TailFloat(0.5, 0.0)))
            .sub(TailFloat::ONE.div(v).ln_1p());
        v = v.add(TailFloat(2.0, 0.0));
    }
    let r = TailFloat::ONE.div(v);
    let r2 = r.mul(r);
    let expansion = TailFloat(17.0, 0.0)
        .div(TailFloat(112.0, 0.0))
        .mul(r2)
        .sub(TailFloat::ONE.div(TailFloat(20.0, 0.0)))
        .mul(r2)
        .add(TailFloat::ONE.div(TailFloat(24.0, 0.0)))
        .mul(r2)
        .sub(TailFloat(0.25, 0.0))
        .mul(r);
    TailFloat::PI
        .mul(TailFloat(2.0, 0.0))
        .ln()
        .mul(TailFloat(-0.5, 0.0))
        .add(expansion)
        .add(correction)
}

// Preserve the historical conversion for existing Student callers; the log API
// carries the compensated result from the same integration into F recurrence.
fn probability(log_density: f64, log_scale: f64, uncertainty: f64) -> Estimate {
    let log_p = log_density + log_scale;
    let boundary = -1075.0 * LN_2;
    if log_p == f64::NEG_INFINITY {
        return Estimate::unavailable(UNDERFLOW);
    }
    if !log_p.is_finite() || !uncertainty.is_finite() {
        return Estimate::unavailable(LIMIT);
    }
    if log_p + uncertainty < boundary {
        return Estimate::unavailable(UNDERFLOW);
    }
    if log_p - uncertainty <= boundary {
        return Estimate::unavailable(
            "The probability is too close to the Float64 rounding boundary.",
        );
    }
    let half = (log_density * 0.5).exp();
    let value = half * (half * log_scale.exp());
    if value > 0.0 && value <= 1.0 {
        Estimate::available(value)
    } else {
        Estimate::unavailable(LIMIT)
    }
}
fn normalization(df: f64) -> f64 {
    if df < 128.0 {
        statrs::function::gamma::ln_gamma((df + 1.0) * 0.5)
            - statrs::function::gamma::ln_gamma(df * 0.5)
            - 0.5 * (df * PI).ln()
    } else {
        let r = 1.0 / df;
        let r2 = r * r;
        -0.5 * (2.0 * PI).ln()
            + r * (-0.25 + r2 * (1.0 / 24.0 + r2 * (-1.0 / 20.0 + r2 * 17.0 / 112.0)))
    }
}
fn validate(df: f64, control: &JobControl) -> AppResult<()> {
    control.check_cancelled()?;
    if !df.is_finite() || !(1.0..=9007199254740992.0).contains(&df) {
        return Err(AppError::invalid(
            "Student-t",
            "Degrees of freedom must be between 1 and 2^53.",
        ));
    }
    Ok(())
}
pub(crate) fn student_t_critical(
    df: f64,
    confidence: ConfidenceLevel,
    control: &JobControl,
) -> AppResult<Estimate> {
    let result = (|| {
        validate(df, control)?;
        let alpha = match confidence {
            ConfidenceLevel::P90 => 0.1,
            ConfidenceLevel::P95 => 0.05,
            ConfidenceLevel::P99 => 0.01,
        };
        if df == 1.0 {
            return Ok(Estimate::available(1.0 / (PI * alpha * 0.5).tan()));
        }
        let (mut lower, mut upper) = (0.0, 128.0);
        for _ in 0..80 {
            let middle = (lower + upper) * 0.5;
            let estimate = student_t_two_sided_p(middle, df, control)?;
            let Some(p) = estimate.value else {
                if estimate.reason.as_deref() == Some(UNDERFLOW) {
                    upper = middle;
                    continue;
                }
                return Ok(estimate);
            };
            if upper - lower <= 1e-12 && (p - alpha).abs() <= 2e-14 {
                return Ok(Estimate::available(middle));
            }
            if p > alpha {
                lower = middle;
            } else {
                upper = middle;
            }
        }
        Ok(Estimate::unavailable(
            "Student-t critical-value bisection exceeded its numerical limit.",
        ))
    })();
    control.check_cancelled()?;
    result
}
#[cfg(test)]
mod tests;
