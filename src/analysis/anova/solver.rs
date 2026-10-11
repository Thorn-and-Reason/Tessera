//! Reviewed tall square-root solve; no inverse, rank truncation or fallback.
use super::arithmetic::*;
use super::{AnovaFailure, AnovaFailureKind};
use crate::{analysis::moments::Scaled, error::AppResult, operation::JobControl};
use nalgebra::{linalg::SVD, DMatrix};

pub(crate) fn solve(
    basis: &Basis,
    counts: &[u64],
    scores: &[f64],
    exponent: i32,
    input_error: f64,
    control: &JobControl,
) -> AppResult<Result<Energy, AnovaFailure>> {
    solve_with_iterations(basis, counts, scores, exponent, input_error, 512, control)
}
fn solve_with_iterations(
    b: &Basis,
    counts: &[u64],
    scores: &[f64],
    exponent: i32,
    input_error: f64,
    iterations: usize,
    c: &JobControl,
) -> AppResult<Result<Energy, AnovaFailure>> {
    c.checkpoint(0, None)?;
    let (q, cells) = (b.rank, b.cells);
    let total = counts
        .iter()
        .try_fold(0u64, |sum, count| sum.checked_add(*count));
    if total.is_none_or(|n| n > 1 << 53)
        || b.rows.len() != q * cells
        || b.norms.len() != q
        || b.norms.iter().any(|n| !n.is_finite() || *n <= 0.)
        || q == 0
        || q >= cells
        || cells > 64
        || counts.len() != cells
        || scores.len() != q
        || counts.contains(&0)
        || scores.iter().any(|v| !v.is_finite())
        || !input_error.is_finite()
        || input_error < 0.
        || iterations == 0
        || iterations > 512
    {
        return Ok(Err(failure("Invalid admitted hypothesis solve.")));
    }
    let m = *counts.iter().min().unwrap() as f64;
    let maximum = *counts.iter().max().unwrap() as f64;
    if scores.iter().all(|v| *v == 0.) {
        return Ok(if input_error == 0. {
            Ok(Energy::default())
        } else {
            Err(failure("A zero score has unresolved input uncertainty."))
        });
    }
    let mut a = DMatrix::zeros(cells, q);
    for i in 0..q {
        for j in 0..cells {
            c.checkpoint(i * cells + j, None)?;
            a[(j, i)] =
                f64::from(b.rows[i * cells + j]) / b.norms[i] * (m / counts[j] as f64).sqrt();
        }
    }
    if q == 1 {
        let denominator = compensated((0..cells).map(|j| a[(j, 0)] * a[(j, 0)]));
        let x = scores[0] / denominator.sqrt();
        return Ok(finish(
            x.abs(),
            16. * f64::EPSILON * cells as f64,
            input_error,
            (m / maximum).sqrt(),
            m,
            exponent,
        ));
    }
    c.checkpoint(0, None)?;
    let decomposition =
        SVD::try_new_unordered(a.clone(), true, true, 5. * f64::EPSILON, iterations);
    c.checkpoint(0, None)?;
    let Some(svd) = decomposition else {
        return Ok(Err(AnovaFailure {
            kind: AnovaFailureKind::Nonconvergence,
            reason: "The bounded hypothesis decomposition did not converge.".into(),
        }));
    };
    let (Some(u), Some(vt)) = (svd.u, svd.v_t) else {
        return Ok(Err(failure(
            "Hypothesis decomposition vectors are unavailable.",
        )));
    };
    let singular = svd.singular_values;
    if singular.len() != q || singular.iter().any(|s| !s.is_finite() || *s <= 0.) {
        return Ok(Err(failure(
            "The full hypothesis dimension is numerically unresolved.",
        )));
    }
    let mut residual = 0f64;
    let mut left = 0f64;
    let mut right = 0f64;
    for i in 0..cells {
        for j in 0..q {
            c.checkpoint(i * q + j, None)?;
            let reconstructed = compensated((0..q).map(|k| u[(i, k)] * singular[k] * vt[(k, j)]));
            residual = residual.hypot(reconstructed - a[(i, j)]);
        }
    }
    for i in 0..q {
        for j in 0..q {
            c.checkpoint(i * q + j, None)?;
            let identity = if i == j { 1. } else { 0. };
            left = left.hypot(compensated((0..cells).map(|k| u[(k, i)] * u[(k, j)])) - identity);
            right = right.hypot(compensated((0..q).map(|k| vt[(i, k)] * vt[(j, k)])) - identity);
        }
    }
    let reconstruction_limit = 64. * f64::EPSILON * cells as f64 * (cells as f64).sqrt();
    let orthogonality_limit = 64. * f64::EPSILON * cells as f64;
    let minimum = singular.iter().copied().fold(f64::INFINITY, f64::min);
    let delta = residual + 8. * f64::EPSILON * (q as f64).sqrt();
    if !residual.is_finite()
        || !left.is_finite()
        || !right.is_finite()
        || residual > reconstruction_limit
        || left > orthogonality_limit
        || right > orthogonality_limit
        || minimum <= delta
    {
        return Ok(Err(failure(
            "Hypothesis reconstruction or orthogonality is unresolved.",
        )));
    }
    let factor_error =
        2. * delta / (minimum - delta) + 2. * (left + right) + 8. * f64::EPSILON * q as f64;
    let mut norm = 0f64;
    for k in 0..q {
        c.checkpoint(k, None)?;
        let component = compensated((0..q).map(|i| vt[(k, i)] * scores[i])) / singular[k];
        norm = norm.hypot(component);
    }
    Ok(finish(
        norm,
        factor_error,
        input_error,
        (m / maximum).sqrt(),
        m,
        exponent,
    ))
}
fn finish(
    norm: f64,
    factor_error: f64,
    input_error: f64,
    count_lower: f64,
    minimum_count: f64,
    exponent: i32,
) -> Result<Energy, AnovaFailure> {
    if !norm.is_finite()
        || norm <= 0.
        || !factor_error.is_finite()
        || factor_error >= 1.
        || count_lower <= 0.
    {
        return Err(failure(
            "A positive hypothesis energy norm cannot be resolved.",
        ));
    }
    // Reduce the computed norm before using it to propagate input error.
    let lower = norm * (1. - factor_error).sqrt() - input_error / count_lower;
    if !lower.is_finite() || lower <= 0. {
        return Err(failure(
            "Hypothesis input uncertainty reaches its energy norm.",
        ));
    }
    let relative_error = factor_error + 2. * input_error / (count_lower * lower);
    if !relative_error.is_finite() || relative_error > MAX_RELATIVE {
        return Err(failure(
            "Hypothesis energy allowance exceeds the admitted bound.",
        ));
    }
    let part = Scaled::new(norm);
    let value = multiply(part, part).times(minimum_count);
    Ok(Energy {
        value: Scaled {
            fraction: value.fraction,
            exponent: value.exponent + 2 * exponent,
        },
        relative_error,
    })
}
#[cfg(test)]
mod tests;
