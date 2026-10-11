//! Calibrated GG tail service; callers validate the captured model first.
use crate::{
    analysis::Estimate,
    error::{AppError, AppResult},
    operation::JobControl,
};
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProbabilityRange {
    pub(crate) lower: f64,
    pub(crate) upper: f64,
}
impl ProbabilityRange {
    fn valid(self) -> bool {
        self.lower.is_finite()
            && self.upper.is_finite()
            && self.lower >= f64::MIN_POSITIVE
            && self.lower <= self.upper
            && self.upper <= 1.
    }
}
/// Working calibration data, never a serialized or certified interval.
pub(crate) struct EvaluatedTail {
    pub(crate) estimate: Estimate,
    pub(crate) range: Option<ProbabilityRange>,
}
pub(crate) fn f_upper_from_log(
    log_f: f64,
    df1: f64,
    df2: f64,
    control: &JobControl,
) -> AppResult<Estimate> {
    Ok(evaluate(log_f, df1, df2, control)?.estimate)
}
pub(crate) fn evaluate(
    log_f: f64,
    df1: f64,
    df2: f64,
    control: &JobControl,
) -> AppResult<EvaluatedTail> {
    evaluate_with_limit(log_f, df1, df2, 10_000, control)
}
fn evaluate_with_limit(
    log_f: f64,
    df1: f64,
    df2: f64,
    limit: usize,
    control: &JobControl,
) -> AppResult<EvaluatedTail> {
    control.check_cancelled()?;
    if !log_f.is_finite()
        || !df1.is_finite()
        || !df2.is_finite()
        || !(1. - f64::EPSILON / 2. ..=63.).contains(&df1)
        || !(df1..=2_000_000.).contains(&df2)
    {
        return Err(AppError::invalid(
            "ANOVA F tail",
            "Finite log F and validated corrected degrees are required.",
        ));
    }
    let evaluated = kernel(log_f, df1, df2, limit, control);
    control.check_cancelled()?;
    Ok(match evaluated {
        Ok((p, range)) => EvaluatedTail {
            estimate: Estimate::available(p),
            range: Some(range),
        },
        Err(reason) => EvaluatedTail {
            estimate: Estimate::unavailable(match reason {
                "work_limit" => "The fractional F tail exceeded its numerical work limit.",
                "subnormal_uncertain" => {
                    "The fractional F probability crosses the Float64 publication boundary."
                }
                "precision_uncertain" => {
                    "The fractional F probability has excessive calibration uncertainty."
                }
                _ => "The fractional F tail could not be evaluated with finite stable arithmetic.",
            }),
            range: None,
        },
    })
}
fn probability_range(low: f64, g: f64, flipped: bool) -> ProbabilityRange {
    if flipped {
        ProbabilityRange {
            lower: -(low + g).min(0.).exp_m1(),
            upper: -(low - g).exp_m1(),
        }
    } else {
        ProbabilityRange {
            lower: (low - g).exp(),
            upper: (low + g).min(0.).exp(),
        }
    }
}
/// The upstream sensitivity and merged calibration width are separate screens.
/// Endpoints must already preserve physical epsilon bounds and linked degrees.
pub(crate) fn combined_probability(
    center: &EvaluatedTail,
    endpoints: &[EvaluatedTail],
    upstream_relative: Option<f64>,
) -> Estimate {
    let unavailable = || {
        Estimate::unavailable("The dependent F probability has unavailable or excessive upstream/calibration uncertainty.")
    };
    let Some(upstream) = upstream_relative else {
        return unavailable();
    };
    if !upstream.is_finite() || !(0. ..=2e-9).contains(&upstream) || endpoints.len() != 4 {
        return unavailable();
    }
    let Some(candidate) = center.estimate.value else {
        return unavailable();
    };
    let Some(center_range) = center.range else {
        return unavailable();
    };
    if !center_range.valid()
        || !candidate.is_finite()
        || candidate < center_range.lower
        || candidate > center_range.upper
    {
        return unavailable();
    }
    let mut lower = center_range.lower;
    let mut upper = center_range.upper;
    let mut min_p = f64::INFINITY;
    let mut max_p = 0_f64;
    for endpoint in endpoints {
        let (Some(range), Some(p)) = (endpoint.range, endpoint.estimate.value) else {
            return unavailable();
        };
        if !p.is_finite() || !range.valid() || p < range.lower || p > range.upper {
            return unavailable();
        }
        lower = lower.min(range.lower);
        upper = upper.max(range.upper);
        min_p = min_p.min(p);
        max_p = max_p.max(p);
    }
    if (max_p - min_p) / min_p > 2e-9
        || (upper - lower) / lower > 1e-8
        || candidate < lower
        || candidate > upper
    {
        return unavailable();
    }
    Estimate::available(candidate)
}
fn lb(a: f64, b: f64) -> f64 {
    let (a, b) = (a.min(b), a.max(b));
    if b < 16. {
        return statrs::function::gamma::ln_gamma(a) + statrs::function::gamma::ln_gamma(b)
            - statrs::function::gamma::ln_gamma(a + b);
    }
    let c = |x: f64| {
        let r = 1. / x;
        let r2 = r * r;
        r * (1. / 12.
            + r2 * (-1. / 360.
                + r2 * (1. / 1260.
                    + r2 * (-1. / 1680. + r2 * (1. / 1188. + r2 * (-691. / 360360. + r2 / 156.))))))
    };
    statrs::function::gamma::ln_gamma(a) - a * b.ln() - (a + b - 0.5) * (a / b).ln_1p() + a + c(b)
        - c(a + b)
}
fn sp(x: f64) -> f64 {
    x.max(0.) + (-x.abs()).exp().ln_1p()
}
fn kernel(
    lf: f64,
    d1: f64,
    d2: f64,
    limit: usize,
    ck: &JobControl,
) -> Result<(f64, ProbabilityRange), &'static str> {
    ck.checkpoint(0, None).map_err(|_| "cancelled")?;
    if !(d1 >= 1. - f64::EPSILON / 2. && d1 <= 63. && d2 >= d1 && d2 <= 2_000_000.)
        || !d1.is_finite()
        || !d2.is_finite()
    {
        return Err("domain");
    };
    if !lf.is_finite() {
        return Err("domain");
    };
    let r = lf + d1.ln() - d2.ln();
    let (mut lx, mut ly) = (-sp(r), -sp(-r));
    let (mut a, mut b) = (d2 / 2., d1 / 2.);
    let flipped = lx > ((a + 1.) / (a + b + 2.)).ln();
    if flipped {
        std::mem::swap(&mut a, &mut b);
        std::mem::swap(&mut lx, &mut ly)
    }
    let x = lx.exp();
    let protect = |v: f64| {
        if v.abs() < 1e-300 {
            1e-300_f64.copysign(v)
        } else {
            v
        }
    };
    let (mut c, mut d) = (1., 1. / protect(1. - (a + b) * x / (a + 1.)));
    let (mut h, mut settled) = (d, 0);
    for m in 1..=limit.min(10_000) {
        ck.checkpoint(0, None).map_err(|_| "cancelled")?;
        ck.checkpoint(m, Some(10_000)).map_err(|_| "cancelled")?;
        let mf = m as f64;
        let previous = h;
        for aa in [
            mf * (b - mf) * x / ((a + 2. * mf - 1.) * (a + 2. * mf)),
            -(a + mf) * (a + b + mf) * x / ((a + 2. * mf) * (a + 2. * mf + 1.)),
        ] {
            d = 1. / protect(1. + aa * d);
            c = protect(1. + aa / c);
            let delta = c * d;
            h *= delta
        }
        if !h.is_finite() || h <= 0. {
            return Err("arithmetic");
        };
        settled = if (h / previous - 1.).abs() <= 4. * f64::EPSILON {
            settled + 1
        } else {
            0
        };
        if settled >= 4 {
            ck.checkpoint(0, None).map_err(|_| "cancelled")?;
            let log_beta = lb(a, b);
            let low = a * lx + b * ly - log_beta + h.ln() - a.ln();
            // Calibration allowance, not a mathematically certified error bound.
            let guard = 2e-9
                + 256.
                    * f64::EPSILON
                    * ((a * lx).abs()
                        + (b * ly).abs()
                        + log_beta.abs()
                        + h.ln().abs()
                        + a.ln().abs());
            ck.checkpoint(0, None).map_err(|_| "cancelled")?;
            if !low.is_finite() || low > 32. * f64::EPSILON {
                return Err("arithmetic");
            };
            let p = if flipped { -low.exp_m1() } else { low.exp() };
            if p >= f64::MIN_POSITIVE && (!guard.is_finite() || guard > 1e-8) {
                return Err("precision_uncertain");
            }
            if !(0. ..=1.).contains(&p) {
                return Err("arithmetic");
            };
            if p < f64::MIN_POSITIVE || (!flipped && low <= f64::MIN_POSITIVE.ln() + guard) {
                return Err("subnormal_uncertain");
            }
            let range = probability_range(low, guard, flipped);
            if !p.is_finite() || !range.valid() || p < range.lower || p > range.upper {
                return Err("precision_uncertain");
            }
            return Ok((p, range));
        }
    }
    Err("work_limit")
}

#[cfg(test)]
mod tests;
