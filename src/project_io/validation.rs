//! Relationships between persisted result fields beyond the shared model validator.
use crate::{
    analysis::{CompletedAnalysis, MissingPolicy},
    data::MAX_CELLS,
    error::{AppError, AppResult},
    project::AnalysisRun,
};
use std::{collections::HashSet, sync::Arc};

pub(super) fn runs(
    runs: &[Arc<AnalysisRun>],
    control: &crate::operation::JobControl,
) -> AppResult<()> {
    for run in runs {
        control.checkpoint(0, None)?;
        let d = run.analysis.diagnostics();
        if [
            d.filtered_rows,
            d.missing_weight_rows,
            d.zero_weight_rows,
            d.complete_case_rows,
        ]
        .into_iter()
        .try_fold(0_usize, usize::checked_add)
        .is_none_or(|n| n > MAX_CELLS)
        {
            return Err(invalid());
        }
        let mut names = HashSet::new();
        if run
            .analysis
            .columns()
            .iter()
            .any(|info| !names.insert(&info.name))
        {
            return Err(invalid());
        }
        let CompletedAnalysis::Describe(a) = &run.analysis else {
            continue;
        };
        if a.request.missing_policy == MissingPolicy::PerVariable
            && (d.complete_case_rows != 0 || a.rows.iter().any(|r| r.complete_case_excluded_n != 0))
        {
            return Err(invalid());
        }
        if a.request.frequency_weight.is_none()
            && (d.missing_weight_rows != 0 || d.zero_weight_rows != 0)
        {
            return Err(invalid());
        }
        let mut total = None;
        for (row, plot) in a.rows.iter().zip(&a.plots) {
            let count = row
                .valid_n
                .checked_add(row.missing_n)
                .and_then(|n| n.checked_add(row.complete_case_excluded_n))
                .ok_or_else(invalid)?;
            if total.is_some_and(|total| total != count) || plot.source_rows as u64 > row.valid_n {
                return Err(invalid());
            }
            total = Some(count);
            let frequency = plot
                .points
                .iter()
                .try_fold(0_u64, |n, p| n.checked_add(p.frequency))
                .ok_or_else(invalid)?;
            if !plot.reduced && frequency != row.valid_n {
                return Err(invalid());
            }
            if a.request.frequency_weight.is_none()
                && (plot.points.iter().any(|p| p.frequency != 1)
                    || plot.source_rows as u64 != row.valid_n)
            {
                return Err(invalid());
            }
        }
    }
    Ok(())
}
fn invalid() -> AppError {
    AppError::invalid(
        "analysis",
        "Saved observation counts, labels or frequencies are inconsistent.",
    )
}
