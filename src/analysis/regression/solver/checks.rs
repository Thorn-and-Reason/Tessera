//! Accepted Task 1 gates, evaluated before dependent quantities are published.
use super::*;
pub(super) const SHAPE: &str = "The decomposition returned factors with incompatible dimensions.";
const NONFINITE: &str = "The decomposition returned a nonfinite factor.";
const SINGULAR: &str = "The decomposition returned an invalid singular value.";
const NORM: &str = "A normalized covariance-factor row norm is not positive and finite.";
const COVFINITE: &str =
    "The covariance basis or correlation representation contains a nonfinite value.";
const REASONS:[&str;16]=[
    "The decomposition does not reconstruct the scaled design within the numerical tolerance.",
    "The left decomposition factors are not sufficiently orthogonal.",
    "The right decomposition factors are not sufficiently orthogonal.",
    "The coefficients do not satisfy the triangular system within the numerical tolerance.",
    "The full-cohort residuals do not satisfy the least-squares equations within the numerical tolerance.",
    "The retained residuals disagree with independently evaluated fitted values.",
    "The independently accumulated residual norm disagrees with the QR residual norm.",
    "The weighted residual mean is inconsistent with the fitted intercept.",
    "The response, explained and residual energies are numerically inconsistent.",
    "The frequency-weighted per-copy leverages do not sum to the model rank within the numerical tolerance.",
    "A per-copy leverage is outside its admissible numerical range.",
    "The covariance basis disagrees with the decomposition factors.",
    "The covariance correlation representation is not sufficiently symmetric.",
    "The covariance correlations disagree with the covariance-factor row products.",
    "A covariance correlation diagonal is inconsistent with a squared row norm.",
    "A standard error disagrees with its covariance-factor row norm.",
];
fn ratio(a: f64, b: f64) -> f64 {
    if b == 0. {
        if a == 0. {
            0.
        } else {
            f64::INFINITY
        }
    } else {
        a / b
    }
}
fn gate(metric: usize, value: f64, limit: f64) -> Result<(), Abort> {
    if !value.is_finite() || !limit.is_finite() || value > limit {
        Err(Abort::Numerical(REASONS[metric]))
    } else {
        Ok(())
    }
}
pub(super) fn factors(
    r: &DMatrix<f64>,
    u: &DMatrix<f64>,
    vt: &DMatrix<f64>,
    s: &[f64],
    ck: &mut Check<'_>,
) -> Result<(), Abort> {
    ck.now()?;
    let p = r.nrows();
    if p < 2 || r.ncols() != p || u.shape() != (p, p) || vt.shape() != (p, p) || s.len() != p {
        return Err(Abort::Numerical(SHAPE));
    }
    for v in r.iter().chain(u.iter()).chain(vt.iter()) {
        ck.tick()?;
        if !v.is_finite() {
            return Err(Abort::Numerical(NONFINITE));
        }
    }
    for v in s {
        ck.tick()?;
        if !v.is_finite() || *v < 0. {
            return Err(Abort::Numerical(SINGULAR));
        }
    }
    let rn = ck.norm(r.iter().copied())?;
    let (mut rec, mut ou, mut ov) = (0f64, 0f64, 0f64);
    for i in 0..p {
        for j in 0..p {
            let mut totals = [Sum::default(), Sum::default(), Sum::default()];
            for k in 0..p {
                ck.tick()?;
                totals[0].add(if rn == 0. {
                    0.
                } else {
                    u[(i, k)] * (s[k] / rn) * vt[(k, j)]
                });
                totals[1].add(u[(k, i)] * u[(k, j)]);
                totals[2].add(vt[(i, k)] * vt[(j, k)]);
            }
            rec = rec.hypot(if rn == 0. {
                0.
            } else {
                r[(i, j)] / rn - totals[0].value()
            });
            let target = f64::from(i == j);
            ou = ou.hypot(totals[1].value() - target);
            ov = ov.hypot(totals[2].value() - target);
        }
    }
    if rn == 0. && s.iter().any(|v| *v != 0.) {
        rec = f64::INFINITY;
    }
    let limit = 64. * p as f64 * f64::EPSILON;
    gate(0, rec, limit)?;
    gate(1, ou, limit)?;
    gate(2, ov, limit)?;
    ck.now()
}
pub(super) struct Evidence<'a> {
    pub r: &'a DMatrix<f64>,
    pub rhs: &'a [f64],
    pub h: &'a [f64],
    pub vt: &'a DMatrix<f64>,
    pub s: &'a [f64],
    pub corr: &'a [f64],
    pub norms: &'a [f64],
    pub tail: f64,
}
pub(super) fn fit(
    mut fit: ScaledFit,
    input: &Input<'_>,
    terms: &[RegressionTerm],
    ev: Evidence<'_>,
    ck: &mut Check<'_>,
) -> Result<ScaledFit, Abort> {
    let p = fit.p;
    if !(2..=MAX_TERMS).contains(&p) {
        return Err(Abort::Numerical(SHAPE));
    }
    let mut m = 0;
    for record in input.eligible {
        ck.tick()?;
        m += usize::from(record.complete);
    }
    if fit.gamma.len() != p
        || fit.basis.len() != p * p
        || ev.r.shape() != (p, p)
        || ev.rhs.len() != p
        || ev.h.len() != m
        || fit.residuals.len() != m
        || ev.vt.shape() != (p, p)
        || ev.s.len() != p
        || ev.corr.len() != p * p
        || ev.norms.len() != p
        || fit.scales.len() != p - 1
    {
        return Err(Abort::Numerical(SHAPE));
    }
    let mut leverage_domain = 0f64;
    for v in ev.h {
        ck.tick()?;
        if !v.is_finite() || *v < 0. {
            return Err(Abort::Numerical(
                "A per-copy leverage is negative or nonfinite.",
            ));
        }
        leverage_domain = leverage_domain.max(-v).max(v - 1.);
    }
    for v in ev.norms {
        ck.tick()?;
        if !v.is_finite() || *v <= 0. {
            return Err(Abort::Numerical(NORM));
        }
    }
    for v in fit.basis.iter().chain(ev.corr) {
        ck.tick()?;
        if !v.is_finite() {
            return Err(Abort::Numerical(COVFINITE));
        }
    }
    for v in fit
        .gamma
        .iter()
        .chain(&fit.residuals)
        .chain(ev.rhs)
        .chain(std::iter::once(&ev.tail))
    {
        ck.tick()?;
        if !v.is_finite() {
            return Err(Abort::Numerical(
                "The fitted calculation contains a nonfinite value.",
            ));
        }
    }
    let rn = ck.norm(ev.r.iter().copied())?;
    let gn = ck.norm(fit.gamma.iter().copied())?;
    let bound = rn * gn + ck.norm(ev.rhs.iter().copied())?.hypot(ev.tail);
    let mut solve = 0f64;
    for i in 0..p {
        solve = solve.hypot(ck.sum((0..p).map(|j| ev.r[(i, j)] * fit.gamma[j]))? - ev.rhs[i]);
    }
    let mut row = vec![0.; p];
    let mut station = (0..p).map(|_| Sum::default()).collect::<Vec<_>>();
    let (mut e, mut pred, mut response, mut record_error) = (0f64, 0f64, 0f64, 0f64);
    let mut mean = Sum::default();
    let mut i = 0;
    for record in input.eligible {
        ck.tick()?;
        if !record.complete {
            continue;
        }
        fill_row(input.dataset, terms, &fit.scales, record.row, &mut row, ck)?;
        let weight = record.weight as f64 / input.n as f64;
        let root = weight.sqrt();
        let y = fit.outcome.at(input.value(record.row, None, ck)?);
        let f = ck.sum(row.iter().zip(&fit.gamma).map(|(a, b)| a * b))?;
        record_error = record_error.hypot((y - f - fit.residuals[i]) * root);
        e = e.hypot(fit.residuals[i] * root);
        pred = pred.hypot(f * root);
        response = response.hypot(y * root);
        mean.add(fit.residuals[i] * weight);
        for j in 0..p {
            ck.tick()?;
            station[j].add(row[j] * fit.residuals[i] * weight);
        }
        i += 1;
    }
    let energy_scale = response.hypot(pred).hypot(e);
    let identity = if energy_scale == 0. {
        0.
    } else {
        ratio(
            ((response / energy_scale).powi(2)
                - (pred / energy_scale).powi(2)
                - (e / energy_scale).powi(2))
            .abs()
                * energy_scale,
            bound,
        )
    };
    let mut trace = Sum::default();
    let mut i = 0;
    for record in input.eligible {
        ck.tick()?;
        if record.complete {
            trace.add(ev.h[i] * record.weight as f64);
            i += 1;
        }
    }
    let trace = trace.value();
    let (mut basis_error, mut symmetry, mut diagonal, mut se_error, mut correlation_error) =
        (0f64, 0f64, 0f64, 0f64, 0f64);
    for j in 0..p {
        let norm = ck.norm(fit.basis[j * p..(j + 1) * p].iter().copied())?;
        if !norm.is_finite() || norm <= 0. {
            return Err(Abort::Numerical(NORM));
        }
        se_error = se_error.max(ratio((ev.norms[j] - norm).abs(), norm));
        diagonal = diagonal.max((ev.corr[j * p + j] - 1.).abs());
        for k in 0..p {
            ck.tick()?;
            basis_error = basis_error.hypot(fit.basis[j * p + k] * ev.s[k] - ev.vt[(k, j)]);
            symmetry = symmetry.hypot(ev.corr[j * p + k] - ev.corr[k * p + j]);
            let other = ck.norm(fit.basis[k * p..(k + 1) * p].iter().copied())?;
            if !other.is_finite() || other <= 0. {
                return Err(Abort::Numerical(NORM));
            }
            let expected = ck.sum(
                (0..p).map(|a| (fit.basis[j * p + a] / norm) * (fit.basis[k * p + a] / other)),
            )?;
            correlation_error = correlation_error.max((ev.corr[j * p + k] - expected).abs());
        }
    }
    let local = 64. * p as f64 * f64::EPSILON;
    let me = m as f64 * f64::EPSILON;
    let pe = p as f64 * f64::EPSILON;
    if me >= 1. || pe >= 1. {
        return Err(Abort::Numerical(
            "The numerical accumulation bound is outside its supported range.",
        ));
    }
    let row_limit = local + 8. * me / (1. - me);
    let delta = pe / (1. - pe) * (p as f64).sqrt() * fit.condition;
    let leverage_limit = row_limit + 2. * delta + delta * delta;
    for (metric, value, limit) in [
        (3, ratio(solve, bound), row_limit),
        (
            4,
            ratio(ck.norm(station.iter().map(Sum::value))?, rn * bound),
            row_limit,
        ),
        (5, ratio(record_error, bound), row_limit),
        (6, ratio((e - ev.tail.hypot(solve)).abs(), bound), row_limit),
        (7, ratio(mean.value().abs(), bound), row_limit),
        (8, identity, row_limit),
        (9, (trace - p as f64).abs() / p as f64, leverage_limit),
        (10, leverage_domain, leverage_limit),
        (11, basis_error / (p as f64).sqrt(), local),
        (12, symmetry / p as f64, local),
        (13, correlation_error, local),
        (14, diagonal, local),
        (15, se_error, local),
    ] {
        gate(metric, value, limit)?;
    }
    fit.e = e;
    fit.explained = pred;
    fit.total = response;
    ck.now()?;
    Ok(fit)
}

#[cfg(test)]
#[path = "checks_tests.rs"]
mod tests;
