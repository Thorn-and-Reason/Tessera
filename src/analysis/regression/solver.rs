//! Precharged streaming QR followed by a checked, bounded square SVD.
use super::*;
use crate::{
    analysis::{moments::Scaled, selection::Eligible, Estimate},
    data::{Cell, Dataset},
    error::{AppError, AppResult},
    operation::JobControl,
};
use nalgebra::{linalg::SVD, DMatrix};
mod checks;
pub(super) mod scaling;
use scaling::{estimate, multiply, Scale};
#[cfg(test)]
pub(super) mod tests;

pub(super) enum KernelOutcome {
    Fitted(ScaledFit),
    Unavailable(RegressionFailure),
}
/// Every vector is in captured term/cohort order. No dependency object escapes.
/// B is row-major V Sigma^-1 for the sqrt(w/N) normalised design. Residuals are
/// in outcome-standard-deviation units; E/P/Y are sqrt(w/N)-weighted norms.
pub(super) struct ScaledFit {
    n: u64,
    p: usize,
    scales: Vec<Scale>,
    outcome: Scale,
    gamma: Vec<f64>,
    basis: Vec<f64>,
    original_basis: Vec<Scaled>,
    original_norms: Vec<Scaled>,
    coefficients: Vec<Scaled>,
    residuals: Vec<f64>,
    e: f64,
    explained: f64,
    total: f64,
    condition: f64,
}
impl ScaledFit {
    pub(super) fn rank(&self) -> usize {
        self.p
    }
    pub(super) fn frequency(&self) -> u64 {
        self.n
    }
    pub(super) fn residual_df(&self) -> u64 {
        self.n - self.p as u64
    }
    pub(super) fn condition(&self) -> f64 {
        self.condition
    }
    pub(super) fn condition_warned(&self) -> bool {
        self.condition > 1e8
    }
    pub(super) fn residual_norm(&self) -> f64 {
        self.e
    }
    pub(super) fn residuals(&self) -> &[f64] {
        &self.residuals
    }
    pub(super) fn coefficients(&self) -> &[Scaled] {
        &self.coefficients
    }
    pub(super) fn outcome_unit(&self) -> Scaled {
        self.outcome.unit()
    }
    pub(super) fn term_unit(&self, j: usize) -> Scaled {
        if j == 0 {
            Scaled::new(1.)
        } else {
            self.scales[j - 1].unit()
        }
    }
    #[cfg(test)]
    pub(super) fn normalised_outcome(&self, y: f64) -> f64 {
        self.outcome.at(y)
    }
    pub(super) fn fitted_estimate(&self, z: f64) -> Estimate {
        estimate(
            self.outcome
                .centre()
                .plus(multiply(self.outcome.unit(), Scaled::new(z))),
        )
    }
    pub(super) fn residual_estimate(&self, e: f64) -> Estimate {
        estimate(multiply(self.outcome.unit(), Scaled::new(e)))
    }
    pub(super) fn standard_error(&self, j: usize) -> Scaled {
        multiply(
            self.original_norms[j],
            Scaled::new(self.e).ratio(Scaled::new((self.residual_df() as f64).sqrt())),
        )
    }
    #[cfg(test)]
    pub(super) fn covariance(
        &self,
        j: usize,
        k: usize,
        control: &JobControl,
    ) -> AppResult<Estimate> {
        control.checkpoint(0, None)?;
        if j >= self.p || k >= self.p {
            return Err(AppError::invalid(
                "regression",
                "The covariance index is outside the model.",
            ));
        }
        if self.residual_df() == 0 {
            return Ok(Estimate::unavailable(
                "There are no residual degrees of freedom.",
            ));
        }
        let mut ck = Check::new(control);
        let factor = Scaled::new(self.e).ratio(Scaled::new((self.residual_df() as f64).sqrt()));
        let product = scaling::sum(
            (0..self.p).map(|a| {
                multiply(
                    self.original_basis[j * self.p + a],
                    self.original_basis[k * self.p + a],
                )
            }),
            &mut ck,
        )
        .map_err(Abort::operational)?;
        control.checkpoint(0, None)?;
        Ok(estimate(multiply(multiply(product, factor), factor)))
    }
    pub(super) fn energy(&self, norm: f64) -> Scaled {
        let v = multiply(self.outcome.unit(), Scaled::new(norm));
        multiply(multiply(v, v), Scaled::new(self.n as f64))
    }
    pub(super) fn total_norm(&self) -> f64 {
        self.total
    }
    pub(super) fn explained_norm(&self) -> f64 {
        self.explained
    }
    /// Caller precharges `row` once and reuses it for full-cohort diagnostics.
    pub(super) fn row(
        &self,
        dataset: &Dataset,
        terms: &[RegressionTerm],
        row: usize,
        buffer: &mut [f64],
        control: &JobControl,
    ) -> AppResult<()> {
        let mut ck = Check::new(control);
        let result = fill_row(dataset, terms, &self.scales, row, buffer, &mut ck);
        control.checkpoint(0, None)?;
        result.map_err(Abort::operational)
    }
    pub(super) fn predict(&self, row: &[f64], control: &JobControl) -> AppResult<f64> {
        control.checkpoint(0, None)?;
        if row.len() != self.p {
            return Err(AppError::invalid(
                "regression",
                "The design row has incompatible dimensions.",
            ));
        }
        let mut ck = Check::new(control);
        let result = ck.sum(row.iter().zip(&self.gamma).map(|(a, b)| a * b));
        control.checkpoint(0, None)?;
        result.map_err(Abort::operational)
    }
    pub(super) fn leverage(&self, row: &[f64], control: &JobControl) -> AppResult<f64> {
        let mut ck = Check::new(control);
        let result = leverage(row, &self.basis, self.p, self.n, &mut ck);
        control.checkpoint(0, None)?;
        result.map_err(Abort::operational)
    }
}
#[derive(Default)]
struct Sum {
    sum: f64,
    correction: f64,
}
impl Sum {
    fn add(&mut self, v: f64) {
        let x = v - self.correction;
        let t = self.sum + x;
        self.correction = (t - self.sum) - x;
        self.sum = t;
    }
    fn value(&self) -> f64 {
        self.sum
    }
}
struct Check<'a> {
    control: &'a JobControl,
    ops: usize,
}
impl<'a> Check<'a> {
    fn new(control: &'a JobControl) -> Self {
        Self { control, ops: 0 }
    }
    fn tick(&mut self) -> Result<(), Abort> {
        self.ops += 1;
        if self.ops.is_multiple_of(64) {
            self.now()?;
        }
        Ok(())
    }
    fn now(&self) -> Result<(), Abort> {
        self.control.checkpoint(self.ops, None)?;
        Ok(())
    }
    fn sum(&mut self, values: impl Iterator<Item = f64>) -> Result<f64, Abort> {
        let mut sum = Sum::default();
        for v in values {
            self.tick()?;
            sum.add(v);
        }
        Ok(sum.value())
    }
    fn norm(&mut self, values: impl Iterator<Item = f64>) -> Result<f64, Abort> {
        let mut norm = 0f64;
        for v in values {
            self.tick()?;
            norm = norm.hypot(v);
        }
        Ok(norm)
    }
}
enum Abort {
    Operational(AppError),
    Numerical(&'static str),
    Rank(usize),
    Nonconvergence,
}
impl From<AppError> for Abort {
    fn from(e: AppError) -> Self {
        Self::Operational(e)
    }
}
impl Abort {
    fn operational(self) -> AppError {
        match self {
            Self::Operational(e) => e,
            _ => AppError::invalid("regression", "Invalid internal numerical state."),
        }
    }
}
struct Input<'a> {
    dataset: &'a Dataset,
    request: &'a RegressionRequest,
    eligible: &'a [Eligible],
    n: u64,
}
impl Input<'_> {
    fn value(
        &self,
        row: usize,
        term: Option<&RegressionTerm>,
        ck: &mut Check<'_>,
    ) -> Result<f64, Abort> {
        term_value(
            self.dataset,
            row,
            term.unwrap_or(&RegressionTerm::Numeric {
                column: self.request.outcome,
            }),
            ck,
        )
    }
}
fn term_value(
    dataset: &Dataset,
    row: usize,
    term: &RegressionTerm,
    ck: &mut Check<'_>,
) -> Result<f64, Abort> {
    ck.tick()?;
    let (column, level) = match term {
        RegressionTerm::Intercept => return Ok(1.),
        RegressionTerm::Numeric { column } => (*column, None),
        RegressionTerm::Contrast { column, level, .. } => (*column, Some(level)),
    };
    let col = dataset.column(column)?;
    match level {
        Some(Cell::Text(level)) => {
            let value = col.text_value(row)?.ok_or_else(|| {
                AppError::invalid(
                    "regression",
                    "A complete model row contains a missing value.",
                )
            })?;
            if value.len() != level.len() {
                return Ok(0.);
            }
            for (a, b) in value
                .as_bytes()
                .chunks(1024)
                .zip(level.as_bytes().chunks(1024))
            {
                ck.now()?;
                if a != b {
                    return Ok(0.);
                }
            }
            Ok(1.)
        }
        _ => match col.cell(row)? {
            Cell::Number(v) if v.is_finite() => Ok(if let Some(Cell::Number(level)) = level {
                f64::from(v == *level)
            } else {
                v
            }),
            _ => Err(AppError::invalid(
                "regression",
                "A complete model row contains an invalid number.",
            )
            .into()),
        },
    }
}
fn fill_row(
    dataset: &Dataset,
    terms: &[RegressionTerm],
    scales: &[Scale],
    row: usize,
    buffer: &mut [f64],
    ck: &mut Check<'_>,
) -> Result<(), Abort> {
    if terms.len() != buffer.len() || terms.len() != scales.len() + 1 {
        return Err(Abort::Numerical(
            "The design row has incompatible dimensions.",
        ));
    }
    buffer[0] = 1.;
    for j in 1..terms.len() {
        buffer[j] = scales[j - 1].at(term_value(dataset, row, &terms[j], ck)?);
    }
    Ok(())
}
pub(super) fn solve(
    dataset: &Dataset,
    request: &RegressionRequest,
    eligible: &[Eligible],
    _levels: &[RegressionLevels],
    terms: &[RegressionTerm],
    control: &JobControl,
) -> AppResult<KernelOutcome> {
    let quiet = control.without_progress();
    let mut ck = Check::new(&quiet);
    ck.control.checkpoint(0, None)?;
    let result = solve_inner(dataset, request, eligible, terms, &mut ck);
    ck.control.checkpoint(0, None)?;
    match result {
        Ok(fit) => Ok(KernelOutcome::Fitted(fit)),
        Err(Abort::Operational(e)) => Err(e),
        Err(error) => {
            let (kind, reason, rank) = match error {
                Abort::Numerical(reason) => (RegressionFailureKind::Numerical, reason, None),
                Abort::Rank(rank) => (
                    RegressionFailureKind::Unidentified,
                    "The requested model is not identified at the declared rank threshold.",
                    Some(rank),
                ),
                Abort::Nonconvergence => (
                    RegressionFailureKind::Nonconvergence,
                    "The bounded decomposition did not converge.",
                    None,
                ),
                Abort::Operational(_) => unreachable!(),
            };
            Ok(KernelOutcome::Unavailable(RegressionFailure {
                kind,
                reason: reason.into(),
                expected_terms: Some(terms.len()),
                rank,
                implicated_terms: vec![],
            }))
        }
    }
}
fn solve_inner(
    dataset: &Dataset,
    request: &RegressionRequest,
    eligible: &[Eligible],
    terms: &[RegressionTerm],
    ck: &mut Check<'_>,
) -> Result<ScaledFit, Abort> {
    let p = terms.len();
    if p > MAX_TERMS {
        return Err(super::coding::oversized_model().into());
    }
    if p < 2 || !matches!(terms.first(), Some(RegressionTerm::Intercept)) {
        return Err(AppError::invalid(
            "regression",
            "An intercept and at least one predictor are required.",
        )
        .into());
    }
    let mut n = 0u64;
    let mut m = 0usize;
    let mut zero_weight = false;
    for row in eligible {
        ck.tick()?;
        if row.complete {
            zero_weight |= row.weight == 0;
            n = n
                .checked_add(row.weight)
                .filter(|n| *n <= 1 << 53)
                .ok_or_else(|| AppError::invalid("regression", "The frequency exceeds 2^53."))?;
            m += 1;
        }
    }
    if n == 0 || zero_weight {
        return Err(AppError::invalid(
            "regression",
            "A positive-frequency complete cohort is required.",
        )
        .into());
    }
    let input = Input {
        dataset,
        request,
        eligible,
        n,
    };
    let mut scales = Vec::with_capacity(p - 1);
    for term in &terms[1..] {
        scales.push(scaling::scale(&input, Some(term), ck)?);
    }
    let outcome = scaling::scale(&input, None, ck)?;
    let mut r = DMatrix::<f64>::zeros(p, p);
    let mut rhs = vec![0.; p];
    let mut row = vec![0.; p];
    let mut tail = 0f64;
    for record in eligible {
        ck.tick()?;
        if !record.complete {
            continue;
        }
        fill_row(dataset, terms, &scales, record.row, &mut row, ck)?;
        let root = (record.weight as f64 / n as f64).sqrt();
        for value in &mut row {
            *value *= root;
            ck.tick()?;
        }
        let mut y = outcome.at(input.value(record.row, None, ck)?) * root;
        for j in 0..p {
            ck.tick()?;
            if row[j] == 0. {
                continue;
            }
            let h = r[(j, j)].hypot(row[j]);
            let a = r[(j, j)] / h;
            let b = row[j] / h;
            for k in j..p {
                ck.tick()?;
                let old = r[(j, k)];
                r[(j, k)] = a * old + b * row[k];
                row[k] = -b * old + a * row[k];
            }
            let old = rhs[j];
            rhs[j] = a * old + b * y;
            y = -b * old + a * y;
        }
        tail = tail.hypot(y);
    }
    let retained = r.clone();
    let svd = decompose(r, 5. * f64::EPSILON, 512, ck)?;
    let u = svd.u.ok_or(Abort::Numerical(checks::SHAPE))?;
    let vt = svd.v_t.ok_or(Abort::Numerical(checks::SHAPE))?;
    let s = svd.singular_values;
    checks::factors(&retained, &u, &vt, s.as_slice(), ck)?;
    let (rank, condition) = rank_and_condition(s.as_slice());
    if rank < p {
        return Err(Abort::Rank(rank));
    }
    let mut q = vec![0.; p];
    let mut gamma = vec![0.; p];
    for k in 0..p {
        q[k] = ck.sum((0..p).map(|i| u[(i, k)] * rhs[i]))? / s[k];
    }
    for j in 0..p {
        gamma[j] = ck.sum((0..p).map(|k| vt[(k, j)] * q[k]))?;
    }
    drop(q);
    drop(u);
    let mut basis = vec![0.; p * p];
    let mut norms = vec![0.; p];
    for j in 0..p {
        for k in 0..p {
            ck.tick()?;
            basis[j * p + k] = vt[(k, j)] / s[k];
        }
        norms[j] = ck.norm(basis[j * p..(j + 1) * p].iter().copied())?;
    }
    let mut corr = vec![0.; p * p];
    for j in 0..p {
        for k in 0..p {
            corr[j * p + k] = ck.sum(
                (0..p).map(|a| (basis[j * p + a] / norms[j]) * (basis[k * p + a] / norms[k])),
            )?;
        }
    }
    let mut residuals = Vec::with_capacity(m);
    let mut h = Vec::with_capacity(m);
    for record in eligible {
        ck.tick()?;
        if !record.complete {
            continue;
        }
        fill_row(dataset, terms, &scales, record.row, &mut row, ck)?;
        residuals.push(
            outcome.at(input.value(record.row, None, ck)?)
                - ck.sum(row.iter().zip(&gamma).map(|(a, b)| a * b))?,
        );
        h.push(leverage(&row, &basis, p, n, ck)?);
    }
    let fit = ScaledFit {
        n,
        p,
        scales,
        outcome,
        gamma,
        basis,
        original_basis: vec![],
        original_norms: vec![],
        coefficients: vec![],
        residuals,
        e: 0.,
        explained: 0.,
        total: 0.,
        condition,
    };
    let mut fit = checks::fit(
        fit,
        &input,
        terms,
        checks::Evidence {
            r: &retained,
            rhs: &rhs,
            h: &h,
            vt: &vt,
            s: s.as_slice(),
            corr: &corr,
            norms: &norms,
            tail,
        },
        ck,
    )?;
    drop(retained);
    drop(rhs);
    drop(h);
    drop(vt);
    drop(s);
    drop(corr);
    drop(norms);
    drop(row);
    transform(&mut fit, ck)?;
    ck.now()?;
    Ok(fit)
}
fn rank_and_condition(singular: &[f64]) -> (usize, f64) {
    let hi = singular.iter().copied().fold(0f64, f64::max);
    let lo = singular.iter().copied().fold(f64::INFINITY, f64::min);
    (
        singular.iter().filter(|v| **v > hi * 1e-12).count(),
        hi / lo,
    )
}
fn decompose(
    r: DMatrix<f64>,
    epsilon: f64,
    iterations: usize,
    ck: &Check<'_>,
) -> Result<SVD<f64, nalgebra::Dyn, nalgebra::Dyn>, Abort> {
    ck.now()?;
    let result = SVD::try_new_unordered(r, true, true, epsilon, iterations);
    ck.now()?;
    result.ok_or(Abort::Nonconvergence)
}
fn leverage(
    row: &[f64],
    basis: &[f64],
    p: usize,
    n: u64,
    ck: &mut Check<'_>,
) -> Result<f64, Abort> {
    if row.len() != p || basis.len() != p * p {
        return Err(Abort::Numerical(checks::SHAPE));
    }
    let mut norm = 0f64;
    for k in 0..p {
        norm = norm.hypot(ck.sum((0..p).map(|j| row[j] * basis[j * p + k]))?);
    }
    Ok((norm / (n as f64).sqrt()).powi(2))
}
fn transform(fit: &mut ScaledFit, ck: &mut Check<'_>) -> Result<(), Abort> {
    let p = fit.p;
    let unit = fit.outcome.unit();
    let mut coefficients = vec![Scaled::default(); p];
    for (j, coefficient) in coefficients.iter_mut().enumerate().skip(1) {
        ck.tick()?;
        *coefficient = multiply(unit, Scaled::new(fit.gamma[j])).ratio(fit.scales[j - 1].unit());
    }
    coefficients[0] = scaling::sum(
        std::iter::once(fit.outcome.centre())
            .chain(std::iter::once(multiply(unit, Scaled::new(fit.gamma[0]))))
            .chain(
                (1..p).map(|j| multiply(coefficients[j], fit.scales[j - 1].centre()).times(-1.)),
            ),
        ck,
    )?;
    for b in &coefficients {
        ck.tick()?;
        if estimate(*b).value.is_none() {
            return Err(Abort::Numerical(
                "An original-unit coefficient is outside finite Float64 range.",
            ));
        }
    }
    let mut original = vec![Scaled::default(); p * p];
    for j in 1..p {
        for k in 0..p {
            ck.tick()?;
            original[j * p + k] =
                multiply(unit, Scaled::new(fit.basis[j * p + k])).ratio(fit.scales[j - 1].unit());
        }
    }
    for k in 0..p {
        original[k] = scaling::sum(
            std::iter::once(multiply(unit, Scaled::new(fit.basis[k]))).chain(
                (1..p)
                    .map(|j| multiply(original[j * p + k], fit.scales[j - 1].centre()).times(-1.)),
            ),
            ck,
        )?;
    }
    let mut norms = Vec::with_capacity(p);
    for j in 0..p {
        norms.push(scaling::norm(
            original[j * p..(j + 1) * p].iter().copied(),
            ck,
        )?);
    }
    fit.coefficients = coefficients;
    fit.original_basis = original;
    fit.original_norms = norms;
    Ok(())
}
