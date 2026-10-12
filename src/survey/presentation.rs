//! Fixed report styles retained by portable survey recipes.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurveyPresentation {
    #[default]
    Report,
    HighContrast,
}

use super::{SurveyColumnReview, SurveyReview};
use crate::{
    analysis::CompletedAnalysis,
    data::{AllocationCounter, Budget, Dataset},
    error::AppResult,
    figures::{FigureKey, FigureKind, FigurePalette, FigureSettings},
    operation::JobControl,
    project::{AnalysisRun, Project},
};
use std::fmt::Write;

pub(super) fn present(
    original: &Project,
    mut candidate: Project,
    review: &SurveyReview,
    style: SurveyPresentation,
    control: &JobControl,
    retained: &[&Dataset],
    extra: usize,
) -> AppResult<Project> {
    for index in original.runs().len()..candidate.runs().len() {
        for column in &review.columns {
            control.check_cancelled()?;
            let run = &candidate.runs()[index];
            let Some(key) = key_for(run, column.column) else {
                continue;
            };
            let name = original.dataset().column(column.column)?.name();
            let mut storage = AllocationCounter::default();
            storage.add(extra)?;
            let combined = Project::retained_owned_bytes_iter_controlled(
                [original, &candidate].into_iter(),
                Some(control),
            )?;
            storage.add(combined)?;
            storage.add(name.len().min(4096))?;
            storage.add(256)?;
            Budget::context_bytes_controlled(
                original.dataset(),
                retained,
                None,
                storage.bytes,
                control,
            )?;
            let settings = settings(
                name,
                column,
                original.dataset().row_count(),
                key.kind,
                style,
            );
            let mut apply_extra = AllocationCounter::default();
            apply_extra.add(extra)?;
            apply_extra.add(combined - candidate.owned_bytes_controlled(control)?)?;
            apply_extra.add(settings.heap_bytes())?;
            candidate = candidate.with_figure_with_budget_controlled(
                &key,
                settings,
                retained,
                apply_extra.bytes,
                Some(control),
            )?;
            control.check_cancelled()?;
        }
    }
    Ok(candidate)
}

fn key_for(run: &AnalysisRun, column: crate::data::ColumnId) -> Option<FigureKey> {
    let kind = match &run.analysis {
        CompletedAnalysis::Frequencies(a) if a.tables.iter().any(|t| t.column == column) => {
            FigureKind::Frequency(column.0)
        }
        CompletedAnalysis::Describe(a) if a.plots.iter().any(|p| p.column == column) => {
            FigureKind::Distribution(column.0)
        }
        CompletedAnalysis::Histogram(a) if a.request.variable == column => FigureKind::Histogram,
        _ => return None,
    };
    Some(FigureKey {
        run: run.id.0,
        kind,
        variant: 0,
    })
}

fn settings(
    name: &str,
    column: &SurveyColumnReview,
    rows: usize,
    kind: FigureKind,
    style: SurveyPresentation,
) -> FigureSettings {
    let mut title = String::with_capacity(name.len().min(4096));
    for ch in name.chars() {
        if title.len() + ch.len_utf8() > 4096 {
            break;
        }
        title.push(if ch.is_control() { ' ' } else { ch });
    }
    let mut caption = String::with_capacity(256);
    write!(
        caption,
        "{} valid responses; {} missing. ",
        column.valid, column.missing
    )
    .expect("writing to String");
    if matches!(kind, FigureKind::Frequency(_)) {
        write!(caption, "Counts use complete answers. Valid percentages use {} valid responses; all-response percentages use all {} responses.", column.valid, rows).expect("writing to String");
    } else {
        caption.push_str("Missing responses are excluded from the distribution.");
    }
    FigureSettings {
        title,
        caption,
        palette: match style {
            SurveyPresentation::Report => FigurePalette::Report,
            SurveyPresentation::HighContrast => FigurePalette::Accessible,
        },
        ..Default::default()
    }
}
