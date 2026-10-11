//! Streamed residual-Gram Greenhouse–Geisser policy.
use super::arithmetic::{failure, MAX_RELATIVE, U};
use super::AnovaFailure;
pub(crate) struct Covariance {
    pub(crate) epsilon: f64,
    pub(crate) allowance: f64,
    pub(crate) df1: f64,
    pub(crate) df2: f64,
    pub(crate) lower: f64,
    pub(crate) upper: f64,
}
pub(crate) fn evaluate(
    gram: &[f64],
    d: usize,
    n: usize,
    k: usize,
) -> Result<Covariance, AnovaFailure> {
    let bad = || failure("Residual covariance or GG allowance is unresolved.");
    if n < 2
        || d == 0
        || d > 63
        || k < d + 1
        || k > 64
        || gram.len() != d * d
        || gram.iter().any(|x| !x.is_finite())
    {
        return Err(bad());
    }
    if (0..d).any(|i| gram[i * d + i] < 0.) {
        return Err(bad());
    }
    use crate::analysis::inference::arithmetic::TailFloat;
    let trace = (0..d).fold(TailFloat(0., 0.), |sum, i| {
        sum.add(TailFloat(gram[i * d + i], 0.))
    });
    let frobenius = gram.iter().fold(TailFloat(0., 0.), |sum, v| {
        sum.add(TailFloat(*v, 0.).mul(TailFloat(*v, 0.)))
    });
    let (trace, frobenius) = (trace.0 + trace.1, frobenius.0 + frobenius.1);
    if trace <= 0.
        || !trace.is_finite()
        || frobenius <= 0.
        || !frobenius.is_finite()
        || gram.iter().any(|x| *x != 0. && x * x == 0.)
    {
        return Err(bad());
    }
    let rho = 64. * U * (n + k * d + d * d + 16) as f64;
    let z = rho * (d as f64).sqrt();
    let r_fr = 2. * z + z * z;
    let allowance = (-2. * (-rho).ln_1p() - (-r_fr).ln_1p()).exp_m1() + 16. * U;
    if !rho.is_finite() || !allowance.is_finite() || rho > MAX_RELATIVE || allowance > MAX_RELATIVE
    {
        return Err(bad());
    }
    let lower = 1. / d as f64;
    let upper = 1f64.min((n - 1) as f64 / d as f64);
    let candidate = trace * trace / (d as f64 * frobenius);
    let epsilon = if n == 2 {
        // Two actual complete profiles prove rank one; screen the Gram identity, not noisy epsilon.
        let identity = trace * trace / frobenius;
        if !identity.is_finite() || (identity - 1.).abs() > allowance {
            return Err(bad());
        }
        lower
    } else if d == 1 {
        1.
    } else {
        if !candidate.is_finite() || candidate < lower || candidate > upper {
            return Err(bad());
        }
        candidate
    };
    let df1 = if n == 2 || d == 1 {
        1.
    } else {
        epsilon * d as f64
    };
    Ok(Covariance {
        epsilon,
        allowance,
        df1,
        df2: (n - 1) as f64 * df1,
        lower,
        upper,
    })
}
#[cfg(test)]
mod tests;
