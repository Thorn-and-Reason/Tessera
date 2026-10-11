//! Physical-row independent ANOVA with exact cell sums and private carried state.
use super::{arithmetic::*, cohort::AnovaCohort, *};
use crate::analysis::Estimate;
use crate::{
    analysis::{
        model::checked_sum,
        moments::{BoundedSum, Scaled},
        validation::invalid_analysis,
    },
    data::{Budget, Cell, Dataset},
    error::AppResult,
    operation::JobControl,
};
use std::mem::size_of;

pub(crate) struct CellState {
    pub(crate) sum: BoundedSum,
    pub(crate) count: u64,
}
pub(crate) struct IndependentState {
    pub(crate) cells: Vec<CellState>,
    pub(crate) error: Energy,
}
impl IndependentState {
    pub(crate) fn owned_bytes(&self) -> AppResult<usize> {
        checked_sum([
            size_of::<Self>(),
            super::accounting::array::<CellState>(self.cells.capacity())?,
        ])
    }
}
pub(crate) struct IndependentResult {
    pub(crate) outcome: AnovaOutcome,
    pub(crate) cell_means: Vec<Estimate>,
    pub(crate) numerical: Option<IndependentState>,
}
/// extra_bytes contains every retained-data and caller owner except this dataset,
/// borrowed request/cohort and the result/workspace charged below.
pub(crate) fn compute(
    dataset: &Dataset,
    request: &AnovaRequest,
    cohort: &AnovaCohort,
    control: &JobControl,
    extra_bytes: usize,
) -> AppResult<IndependentResult> {
    let quiet = control.without_progress();
    let c = &quiet;
    c.checkpoint(0, None)?;
    let outcome = match &request.design {
        AnovaDesign::OneWay { outcome, .. } | AnovaDesign::Factorial { outcome, .. } => *outcome,
        _ => return Err(invalid_analysis()),
    };
    request.admit()?;
    workspace_bounds()?;
    if cohort.cells.len() > MAX_GRID {
        return Err(invalid_analysis());
    }
    let budget = checked_sum([
        extra_bytes,
        request.heap_bytes()?,
        size_of::<AnovaRequest>(),
        cohort.owned_bytes()?,
        2 * 1024 * 1024,
        result_bound()?,
    ])?;
    Budget::check_datasets(&[dataset], budget)?;
    let mut cells = Vec::with_capacity(cohort.cells.len());
    for cell in &cohort.cells {
        cells.push(CellState {
            sum: BoundedSum::default(),
            count: cell.frequency,
        });
    }
    let column = dataset.column(outcome)?;
    let mut grand = BoundedSum::default();
    for row in &cohort.rows {
        c.checkpoint(row.row, None)?;
        let Cell::Number(value) = column.cell(row.row)? else {
            return Err(invalid_analysis());
        };
        let cell = cells.get_mut(row.cell).ok_or_else(invalid_analysis)?;
        cell.sum.push(value, row.weight)?;
        grand.push(value, row.weight)?;
    }
    let mut cell_means = Vec::with_capacity(cells.len());
    for (i, cell) in cells.iter().enumerate() {
        c.checkpoint(i, None)?;
        cell_means.push(if cell.count == 0 {
            Estimate::unavailable("The realised cell is empty.")
        } else {
            cell.sum.scaled(cell.count)?.estimate()
        });
    }
    if let Some(f) = &cohort.failure {
        return Ok(IndependentResult {
            outcome: AnovaOutcome::Unavailable(f.clone()),
            cell_means,
            numerical: None,
        });
    }
    let (mut error, mut total, mut model) = (
        EnergySum::default(),
        EnergySum::default(),
        EnergySum::default(),
    );
    for row in &cohort.rows {
        c.checkpoint(row.row, None)?;
        let Cell::Number(value) = column.cell(row.row)? else {
            return Err(invalid_analysis());
        };
        let cell = &cells[row.cell];
        let grand_residual = grand.residual(value, cohort.frequency)?;
        if let Err(f) = error
            .add(cell.sum.residual(value, cell.count)?, row.weight)
            .and_then(|_| total.add(grand_residual, row.weight))
        {
            return Ok(unavailable(f, cell_means));
        }
    }
    for (i, cell) in cells.iter().enumerate() {
        c.checkpoint(i, None)?;
        if let Err(f) = model.add(
            cell.sum
                .mean_difference(cell.count, &grand, cohort.frequency)?,
            cell.count,
        ) {
            return Ok(unavailable(f, cell_means));
        }
    }
    let (error, total, model) = match (error.finish(), total.finish(), model.finish()) {
        (Ok(a), Ok(b), Ok(d)) => (a, b, d),
        (Err(f), _, _) | (_, Err(f), _) | (_, _, Err(f)) => return Ok(unavailable(f, cell_means)),
    };
    if !decomposition(total, &[error, model]) {
        return Ok(unavailable(
            failure("Total, error and direct model energies are inconsistent."),
            cell_means,
        ));
    }
    let residual_df = cohort
        .frequency
        .checked_sub(cells.len() as u64)
        .ok_or_else(invalid_analysis)?;
    let balanced = cells.iter().all(|cell| cell.count == cells[0].count);
    let levels = cohort
        .factors
        .iter()
        .map(|f| f.values.len())
        .collect::<Vec<_>>();
    let counts = cells.iter().map(|cell| cell.count).collect::<Vec<_>>();
    let mut effects = Vec::with_capacity((1 << levels.len()) - 1);
    let mut effect_energies = Vec::with_capacity(effects.capacity());
    for mask in 1..1 << levels.len() {
        c.checkpoint(mask, None)?;
        let b = basis(&levels, mask, c)?;
        let energy = effect_scores(&cells, &b, balanced, c)?;
        let energy = match energy {
            Ok((scores, exponent, input)) => {
                super::solver::solve(&b, &counts, &scores, exponent, input, c)?
            }
            Err(f) => Err(f),
        };
        let effect = match energy {
            Ok(e) => {
                effect_energies.push(e);
                effect(mask, b.rank, e, error, total, residual_df, c)?
            }
            Err(f) => unavailable_effect(mask, b.rank, error, residual_df, f),
        };
        effects.push(effect);
    }
    if (balanced || levels.len() == 1)
        && effect_energies.len() == effects.len()
        && !decomposition(model, &effect_energies)
    {
        return Ok(unavailable(
            failure("Balanced effect energies do not partition direct model energy."),
            cell_means,
        ));
    }
    c.checkpoint(0, None)?;
    Ok(IndependentResult {
        outcome: AnovaOutcome::Fitted(AnovaModelResult {
            effects,
            error_ss: error.captured(),
            model_ss: model.captured(),
            total_ss: total.captured(),
            subject_ss: None,
            residual_df,
            total_df: cohort.frequency - 1,
            balanced,
        }),
        cell_means,
        numerical: Some(IndependentState { cells, error }),
    })
}
fn workspace_bounds() -> AppResult<()> {
    let accumulation = checked_sum([
        64 * size_of::<CellState>(),
        size_of::<BoundedSum>(),
        64 * size_of::<Scaled>(),
        4096,
    ])?;
    let solver = 8 * (4 * 64 * 63 + 2 * 63 * 63 + 16 * 64 + 16 * 63);
    let caller = checked_sum([
        64 * 63 * size_of::<i16>(),
        63 * size_of::<f64>(),
        64 * size_of::<u64>(),
        64 * size_of::<Scaled>(),
        4 * 63 * size_of::<f64>(),
        64 * size_of::<f64>(),
        6 * size_of::<usize>(),
        8192,
    ])?;
    if accumulation > 1024 * 1024 || solver > 256 * 1024 || caller > 768 * 1024 {
        return Err(crate::error::AppError::SizeLimit {
            limit: 2 * 1024 * 1024,
        });
    }
    Ok(())
}
fn result_bound() -> AppResult<usize> {
    // Fixed maximum includes scalar reason strings and all 63 factor-subset owners.
    checked_sum([
        size_of::<IndependentResult>(),
        size_of::<IndependentState>(),
        64 * size_of::<CellState>(),
        63 * (size_of::<AnovaEffect>() + 6 * size_of::<usize>() + 24 * 256),
        64 * (size_of::<Estimate>() + 256),
    ])
}
fn unavailable(f: AnovaFailure, cell_means: Vec<Estimate>) -> IndependentResult {
    IndependentResult {
        outcome: AnovaOutcome::Unavailable(f),
        cell_means,
        numerical: None,
    }
}
fn decomposition(total: Energy, parts: &[Energy]) -> bool {
    if total.value.fraction == 0. {
        return parts.iter().all(|p| p.value.fraction == 0.);
    }
    let sum = parts
        .iter()
        .fold(Scaled::default(), |sum, p| sum.plus(p.value));
    let relative = (sum.ratio(total.value).value() - 1.).abs();
    relative.is_finite() && relative <= MAX_RELATIVE + 16. * f64::EPSILON * parts.len() as f64
}
fn effect_scores(
    cells: &[CellState],
    b: &Basis,
    balanced: bool,
    c: &JobControl,
) -> AppResult<Result<(Vec<f64>, i32, f64), AnovaFailure>> {
    let mut scores = Vec::with_capacity(b.rank);
    if balanced {
        for i in 0..b.rank {
            let mut sum = BoundedSum::default();
            for (j, cell) in cells.iter().enumerate() {
                c.checkpoint(i * b.cells + j, None)?;
                sum.add_sum(&cell.sum, b.rows[i * b.cells + j])?;
            }
            scores.push(sum.scaled(cells[0].count)?.times(1. / b.norms[i]));
        }
        let exponent = scores
            .iter()
            .filter(|s| s.fraction != 0.)
            .map(|s| s.exponent)
            .max()
            .unwrap_or(0);
        let values = scores
            .iter()
            .map(|s| at(*s, exponent))
            .collect::<Result<Vec<_>, _>>();
        return Ok(values.map(|v| {
            let norm = v.iter().fold(0f64, |norm, score| norm.hypot(*score));
            (v, exponent, 16. * U * b.cells as f64 * norm)
        }));
    }
    let anchor = &cells[0];
    let mut differences = Vec::with_capacity(cells.len());
    for (i, cell) in cells.iter().enumerate() {
        c.checkpoint(i, None)?;
        differences.push(
            cell.sum
                .mean_difference(cell.count, &anchor.sum, anchor.count)?,
        );
    }
    let exponent = differences
        .iter()
        .filter(|s| s.fraction != 0.)
        .map(|s| s.exponent)
        .max()
        .unwrap_or(0);
    let values = match differences
        .iter()
        .map(|s| at(*s, exponent))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(v) => v,
        Err(f) => return Ok(Err(f)),
    };
    let mut error_norm = 0f64;
    for i in 0..b.rank {
        c.checkpoint(i, None)?;
        let row = &b.rows[i * b.cells..(i + 1) * b.cells];
        let coordinate = (|| {
            let mut products = [0.; MAX_GRID];
            for (j, (value, coefficient)) in values.iter().zip(row).enumerate() {
                products[j] = normalized_product(*value, f64::from(*coefficient), 1.)?;
            }
            let products = &products[..b.cells];
            let score = normalized_product(compensated(products.iter().copied()), 1., b.norms[i])?;
            let absolute = normalized_product(
                compensated(products.iter().map(|v| v.abs())),
                1.,
                b.norms[i],
            )?;
            let allowance = normalized_product(absolute, 16. * U * b.cells as f64, 1.)?;
            Ok::<_, AnovaFailure>((score, allowance))
        })();
        let (score, allowance) = match coordinate {
            Ok(values) => values,
            Err(f) => return Ok(Err(f)),
        };
        scores.push(Scaled::new(score));
        error_norm = error_norm.hypot(allowance);
    }
    Ok(Ok((
        scores.iter().map(|s| s.value()).collect(),
        exponent,
        error_norm,
    )))
}
fn factors(mask: usize) -> Vec<usize> {
    (0..6).filter(|i| mask & (1 << i) != 0).collect()
}
fn effect(
    mask: usize,
    q: usize,
    ss: Energy,
    error: Energy,
    total: Energy,
    df: u64,
    c: &JobControl,
) -> AppResult<AnovaEffect> {
    let test = integer_f_test(ss, error, q as u64, df, c)?;
    let failure = if df == 0 {
        Some(AnovaFailure {
            kind: AnovaFailureKind::InsufficientErrorDf,
            reason: "There are no error degrees of freedom.".into(),
        })
    } else if error.value.fraction == 0. {
        Some(AnovaFailure {
            kind: AnovaFailureKind::ZeroError,
            reason: "Within-cell error is exactly zero.".into(),
        })
    } else {
        None
    };
    let partial = Energy {
        value: ss.value.plus(error.value),
        relative_error: ss.relative_error + error.relative_error,
    };
    Ok(AnovaEffect {
        factors: factors(mask),
        dimension: q,
        ss: ss.captured(),
        ms: ss.mean_square(q as u64),
        error_ss: error.captured(),
        error_ms: error.mean_square(df),
        test,
        gg: None,
        eta_squared: energy_ratio(ss, total),
        partial_eta_squared: energy_ratio(ss, partial),
        failure,
    })
}
fn unavailable_effect(
    mask: usize,
    q: usize,
    error: Energy,
    df: u64,
    f: AnovaFailure,
) -> AnovaEffect {
    let missing = || Estimate::unavailable(&f.reason);
    AnovaEffect {
        factors: factors(mask),
        dimension: q,
        ss: AnovaEnergy {
            state: AnovaQuantityState::Unavailable(f.kind),
            value: missing(),
        },
        ms: missing(),
        error_ss: error.captured(),
        error_ms: error.mean_square(df),
        test: AnovaTest {
            df1: Estimate::available(q as f64),
            df2: Estimate::available(df as f64),
            f: missing(),
            p: missing(),
        },
        gg: None,
        eta_squared: missing(),
        partial_eta_squared: missing(),
        failure: Some(f),
    }
}
#[cfg(test)]
mod tests;
