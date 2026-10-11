//! Streamed exports preserve existing destinations. Project files retain typed metadata and missingness.
pub mod figures;
mod report;
pub(crate) mod report_analyses;
pub(crate) mod report_anova;
mod report_charts;
pub(crate) mod report_exploration;
pub(crate) mod report_regression;
pub(crate) mod report_reliability;
use crate::{
    data::{Budget, Cell, ColumnKind, Dataset, MAX_OWNED_BYTES},
    error::{AppError, AppResult},
    files::{self, SaveOutcome},
    import::Delimiter,
    operation::{JobControl, RunId},
    project::Project,
};
use std::{io::Write, mem::size_of, path::Path};
pub fn export_data(
    path: &Path,
    dataset: &Dataset,
    delimiter: Delimiter,
    control: &JobControl,
) -> AppResult<SaveOutcome> {
    export_data_with_budget(path, dataset, delimiter, control, &[], 0)
}
pub fn export_data_with_budget(
    path: &Path,
    dataset: &Dataset,
    delimiter: Delimiter,
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<SaveOutcome> {
    check_budget(dataset, retained, extra_bytes)?;
    files::publish(path, Some(dataset.revision()), None, control, |writer| {
        data(writer, dataset, delimiter, control)
    })
}
pub fn export_report(
    path: &Path,
    project: &Project,
    selected_runs: &[RunId],
    control: &JobControl,
) -> AppResult<SaveOutcome> {
    export_report_with_budget(path, project, selected_runs, control, &[], 0)
}
pub fn export_report_with_budget(
    path: &Path,
    project: &Project,
    selected_runs: &[RunId],
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<SaveOutcome> {
    control.check_cancelled()?;
    let extra = extra_bytes
        .checked_add(project.owned_bytes_controlled(control)?)
        .ok_or(AppError::SizeLimit {
            limit: MAX_OWNED_BYTES,
        })?;
    check_budget(project.dataset(), retained, extra)?;
    for (index, id) in selected_runs.iter().enumerate() {
        if selected_runs[..index].contains(id) || !project.runs().iter().any(|run| run.id == *id) {
            return Err(AppError::invalid(
                "runs",
                "Choose unique retained analysis runs.",
            ));
        }
    }
    for id in selected_runs {
        let run = project
            .runs()
            .iter()
            .find(|r| r.id == *id)
            .expect("selected IDs checked above");
        let scratch = run.analysis.validation_scratch_bytes_controlled(control)?;
        let peak = extra.checked_add(scratch).ok_or(AppError::SizeLimit {
            limit: MAX_OWNED_BYTES,
        })?;
        check_budget(project.dataset(), retained, peak)?;
        run.analysis.validate_controlled(control)?;
        let draws = match &run.analysis {
            crate::analysis::CompletedAnalysis::Describe(v) => !v.plots.is_empty(),
            crate::analysis::CompletedAnalysis::Frequencies(v) => !v.tables.is_empty(),
            crate::analysis::CompletedAnalysis::Histogram(_)
            | crate::analysis::CompletedAnalysis::Scatter(_)
            | crate::analysis::CompletedAnalysis::Boxplot(_)
            | crate::analysis::CompletedAnalysis::Regression(_) => true,
            _ => false,
        };
        if draws {
            let peak =
                extra
                    .checked_add(crate::figures::SVG_WORK_BYTES)
                    .ok_or(AppError::SizeLimit {
                        limit: MAX_OWNED_BYTES,
                    })?;
            check_budget(project.dataset(), retained, peak)?;
        }
    }
    files::publish(
        path,
        Some(project.dataset().revision()),
        None,
        control,
        |writer| report::write(writer, project, selected_runs, control),
    )
}
fn check_budget(dataset: &Dataset, retained: &[&Dataset], extra: usize) -> AppResult<()> {
    let refs = retained
        .len()
        .checked_add(1)
        .and_then(|n| n.checked_mul(size_of::<&Dataset>()))
        .ok_or(AppError::SizeLimit {
            limit: MAX_OWNED_BYTES,
        })?;
    let extra = extra.checked_add(refs).ok_or(AppError::SizeLimit {
        limit: MAX_OWNED_BYTES,
    })?;
    let extra = files::work_budget(retained, extra)?;
    let datasets: Vec<_> = std::iter::once(dataset)
        .chain(retained.iter().copied())
        .collect();
    Budget::check_datasets(&datasets, extra)
}
fn data(
    writer: &mut dyn Write,
    dataset: &Dataset,
    delimiter: Delimiter,
    control: &JobControl,
) -> AppResult<()> {
    let delimiter = match delimiter {
        Delimiter::Comma => b',',
        Delimiter::Tab => b'\t',
    };
    for (index, column) in dataset.columns().iter().enumerate() {
        if index != 0 {
            writer.write_all(&[delimiter])?;
        }
        field(writer, column.name(), delimiter)?;
    }
    writer.write_all(b"\r\n")?;
    for row in 0..dataset.row_count() {
        control.checkpoint(row, Some(dataset.row_count()))?;
        for (index, column) in dataset.columns().iter().enumerate() {
            if index != 0 {
                writer.write_all(&[delimiter])?;
            }
            match column.kind() {
                ColumnKind::Text => {
                    if let Some(value) = column.text_value(row)? {
                        field(writer, value, delimiter)?;
                    }
                }
                ColumnKind::Number => {
                    if let Cell::Number(value) = column.cell(row)? {
                        number(writer, value)?;
                    }
                }
            }
        }
        writer.write_all(b"\r\n")?;
    }
    Ok(())
}
// An empty present text field is quoted; null is blank. Spreadsheet programs may still merge these meanings.
fn field(writer: &mut dyn Write, text: &str, delimiter: u8) -> AppResult<()> {
    if !text.is_empty()
        && !text
            .bytes()
            .any(|c| matches!(c, b'"' | b'\r' | b'\n') || c == delimiter)
    {
        writer.write_all(text.as_bytes())?;
        return Ok(());
    }
    writer.write_all(b"\"")?;
    for (index, part) in text.split('"').enumerate() {
        if index != 0 {
            writer.write_all(b"\"\"")?;
        }
        writer.write_all(part.as_bytes())?;
    }
    writer.write_all(b"\"")?;
    Ok(())
}
fn number(writer: &mut dyn Write, value: f64) -> AppResult<()> {
    if value.fract() == 0.0 && value.abs() >= 9_007_199_254_740_992.0 {
        write!(writer, "{value:.0}")?;
    } else {
        write!(writer, "{value}")?;
    }
    Ok(())
}

#[cfg(test)]
mod regression_publication_tests {
    use super::*;
    use crate::{analysis::CompletedAnalysis, operation::CHECKPOINT_HOOK, project::Project};
    use std::{cell::Cell, rc::Rc};
    #[test]
    fn regression_report_cancels_at_last_prepublication_checkpoint() {
        let (data, capture) = super::report_regression::controls::run("ordinary");
        let project = Project::new("Publication", data)
            .unwrap()
            .with_run(CompletedAnalysis::Regression(capture))
            .unwrap();
        let before = project.runs()[0].analysis.clone();
        let dir = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("tessera-task9-late-report-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let calls = Rc::new(Cell::new(0));
        let seen = calls.clone();
        let prepublication = Rc::new(Cell::new(0));
        let pre = prepublication.clone();
        let baseline_path = dir.join("baseline.html");
        CHECKPOINT_HOOK.with(|h| {
            *h.borrow_mut() = Some(Box::new(move |_| {
                seen.set(seen.get() + 1);
                if !baseline_path.exists() {
                    pre.set(seen.get());
                }
            }))
        });
        let baseline = export_report(
            &dir.join("baseline.html"),
            &project,
            &[RunId(1)],
            &JobControl::default(),
        );
        CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
        baseline.unwrap();
        let last = prepublication.get();
        assert!(last > 5);
        calls.set(0);
        let control = JobControl::default();
        let cancel = control.clone();
        let seen = calls.clone();
        CHECKPOINT_HOOK.with(|h| {
            *h.borrow_mut() = Some(Box::new(move |_| {
                seen.set(seen.get() + 1);
                if seen.get() == last {
                    cancel.cancel();
                }
            }))
        });
        let result = export_report(&dir.join("cancelled.html"), &project, &[RunId(1)], &control);
        CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
        assert!(matches!(result, Err(AppError::Cancelled)), "{result:?}");
        assert_eq!(calls.get(), last);
        assert!(!dir.join("cancelled.html").exists());
        assert!(dir.join("baseline.html").exists());
        assert_eq!(project.runs()[0].analysis, before);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod cancellation_first_tests {
    use super::*;
    #[test]
    fn report_cancelled_before_invalid_selection_and_owner_accounting() {
        let data = Dataset::new(vec![crate::data::Column::numeric(
            crate::data::ColumnId(1),
            "x",
            vec![Some(1.)],
        )
        .unwrap()])
        .unwrap();
        let project = Project::new("Cancelled", data).unwrap();
        let control = JobControl::default();
        control.cancel();
        let result = export_report_with_budget(
            Path::new("unused.html"),
            &project,
            &[RunId(999)],
            &control,
            &[],
            usize::MAX,
        );
        assert!(matches!(result, Err(AppError::Cancelled)), "{result:?}");
    }
}
