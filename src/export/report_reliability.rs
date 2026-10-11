//! Stream captured reliability values through the controlled historical report writer.
use super::{
    report::{missing, text},
    report_analyses::{cell, estimate},
};
use crate::{
    analysis::{Estimate, NumericComparison, ReliabilityEnergyState, ReliabilityRun, RowPredicate},
    data::ColumnId,
    error::AppResult,
    operation::{JobControl, RunId},
};
use std::io::Write;
#[path = "report_regression/cancellation.rs"]
mod cancellation;

pub(crate) fn write(
    w: &mut dyn Write,
    id: RunId,
    r: &ReliabilityRun,
    c: &JobControl,
) -> AppResult<()> {
    cancellation::write(w, c, |w, c| output(w, id, r, c))
}
fn column(w: &mut dyn Write, r: &ReliabilityRun, id: ColumnId) -> AppResult<()> {
    if let Some(info) = r.columns.iter().find(|info| info.id == id) {
        text(w, &info.name)
    } else {
        write!(w, "Column {}", id.0)?;
        Ok(())
    }
}
fn row(w: &mut dyn Write, label: &str, value: &Estimate) -> AppResult<()> {
    w.write_all(b"<tr><th scope=\"row\">")?;
    text(w, label)?;
    w.write_all(b"</th><td>")?;
    estimate(w, value)?;
    w.write_all(b"</td></tr>")?;
    Ok(())
}
fn state(w: &mut dyn Write, state: ReliabilityEnergyState) -> AppResult<()> {
    text(
        w,
        match state {
            ReliabilityEnergyState::Unavailable => "Unavailable",
            ReliabilityEnergyState::ExactZero => "Exact zero",
            ReliabilityEnergyState::Positive => "Positive",
            ReliabilityEnergyState::Unresolved => "Unresolved",
        },
    )
}
fn output(w: &mut dyn Write, id: RunId, r: &ReliabilityRun, c: &JobControl) -> AppResult<()> {
    write!(w,"<section aria-labelledby=\"run-{}\"><h2 id=\"run-{}\">Analysis run {} · Reliability</h2><p>Calculated from data revision {}. Names, definitions and results were captured when this analysis ran.</p>",id.0,id.0,id.0,r.revision.0)?;
    w.write_all(b"<p>Cronbach's raw alpha uses scored item covariances; standardized alpha uses scored item correlations. All summaries and item-deletion results use the same complete-case cohort. Alpha does not establish unidimensionality.</p><p>Numerical method: exact centered interval, version 1.</p>")?;
    write!(w,"<h3>Cohort</h3><p>Source physical rows: {}. Complete physical rows: {}. Complete-case frequency N: {}. Incomplete-case frequency: {}.</p><p>Excluded physical rows: {} by filter; {} with missing weights; {} with valid zero weights; {} incomplete cases.</p>",r.source_rows,r.complete_rows,r.frequency,r.incomplete_case_frequency,r.diagnostics.filtered_rows,r.diagnostics.missing_weight_rows,r.diagnostics.zero_weight_rows,r.diagnostics.complete_case_rows)?;
    w.write_all(b"<h3>Captured settings</h3><p>Frequency weight: ")?;
    if let Some(id) = r.request.frequency_weight {
        column(w, r, id)?;
        text(w, "; literal repeated observations")?;
    } else {
        text(w, "none (each physical row has frequency one)")?;
    }
    w.write_all(b".</p>")?;
    filters(w, r, c)?;
    w.write_all(b"<h4>Ordered items and scoring</h4><ol>")?;
    for (i, id) in r.request.items.iter().enumerate() {
        c.checkpoint(i, None)?;
        w.write_all(b"<li>")?;
        column(w, r, *id)?;
        if let Some(reverse) = r
            .request
            .reversals
            .iter()
            .find(|reverse| reverse.column == *id)
        {
            text(w, "; reverse scored as lower + upper - value; lower bound ")?;
            super::number(w, reverse.lower)?;
            text(w, "; upper bound ")?;
            super::number(w, reverse.upper)?;
        } else {
            text(w, "; original scoring")?;
        }
        w.write_all(b"</li>")?;
    }
    w.write_all(b"</ol><table><caption>Scale reliability</caption><tbody>")?;
    row(w, "Raw alpha", &r.raw_alpha)?;
    row(w, "Standardized alpha", &r.standardized_alpha)?;
    w.write_all(b"<tr><th scope=\"row\">Raw total energy state</th><td>")?;
    state(w, r.total_energy)?;
    w.write_all(b"</td></tr><tr><th scope=\"row\">Standardized total energy state</th><td>")?;
    state(w, r.standardized_total_energy)?;
    w.write_all(b"</td></tr></tbody></table><table><caption>Scored item summaries and deletion diagnostics</caption><thead><tr><th scope=\"col\">Item</th><th scope=\"col\">Scored mean</th><th scope=\"col\">Scored sample SD</th><th scope=\"col\">Corrected item-total correlation</th><th scope=\"col\">Raw alpha if item removed</th><th scope=\"col\">Standardized alpha if item removed</th><th scope=\"col\">Item energy state</th><th scope=\"col\">Remaining raw energy state</th><th scope=\"col\">Remaining standardized energy state</th></tr></thead><tbody>")?;
    for (i, item) in r.items.iter().enumerate() {
        c.checkpoint(i, None)?;
        w.write_all(b"<tr><th scope=\"row\">")?;
        column(w, r, item.column)?;
        w.write_all(b"</th>")?;
        for value in [
            &item.mean,
            &item.sample_sd,
            &item.corrected_item_total,
            &item.raw_alpha_if_removed,
            &item.standardized_alpha_if_removed,
        ] {
            w.write_all(b"<td>")?;
            estimate(w, value)?;
            w.write_all(b"</td>")?;
        }
        for value in [
            item.energy,
            item.remaining_energy,
            item.remaining_standardized_energy,
        ] {
            w.write_all(b"<td>")?;
            state(w, value)?;
            w.write_all(b"</td>")?;
        }
        w.write_all(b"</tr>")?;
    }
    w.write_all(b"</tbody></table><p>Corrected item-total correlation relates each scored item to the sum of the remaining scored items. Item deletion does not select a new cohort. Undefined or numerically unresolved estimates retain their captured reasons.</p>")?;
    definitions(w, r, c)?;
    w.write_all(b"</section>")?;
    c.check_cancelled()
}
fn filters(w: &mut dyn Write, r: &ReliabilityRun, c: &JobControl) -> AppResult<()> {
    w.write_all(b"<p>Captured filter: ")?;
    if r.request.filter.predicates.is_empty() {
        text(w, "all rows")?;
    }
    for (i, p) in r.request.filter.predicates.iter().enumerate() {
        c.checkpoint(i, None)?;
        if i > 0 {
            text(w, " AND ")?;
        }
        column(w, r, p.column())?;
        match p {
            RowPredicate::Number {
                comparison, value, ..
            } => {
                text(
                    w,
                    match comparison {
                        NumericComparison::Equal => " = ",
                        NumericComparison::NotEqual => " != ",
                        NumericComparison::Less => " < ",
                        NumericComparison::LessOrEqual => " <= ",
                        NumericComparison::Greater => " > ",
                        NumericComparison::GreaterOrEqual => " >= ",
                    },
                )?;
                super::number(w, *value)?;
            }
            RowPredicate::Text { value, equal, .. } => {
                text(
                    w,
                    if *equal {
                        " equals "
                    } else {
                        " does not equal "
                    },
                )?;
                text(w, "\"")?;
                text(w, value)?;
                text(w, "\"")?;
            }
            RowPredicate::Missing { missing, .. } => text(
                w,
                if *missing {
                    " is missing"
                } else {
                    " is present"
                },
            )?,
        }
    }
    w.write_all(b"</p>")?;
    Ok(())
}
fn definitions(w: &mut dyn Write, r: &ReliabilityRun, c: &JobControl) -> AppResult<()> {
    w.write_all(b"<h3>Captured column definitions</h3><dl>")?;
    for info in &r.columns {
        c.checkpoint(0, None)?;
        w.write_all(b"<dt>")?;
        text(w, &info.name)?;
        w.write_all(b"</dt><dd>")?;
        text(w, &info.description)?;
        write!(
            w,
            "<p>Column {}; captured kind {}.</p>",
            info.id.0,
            match info.kind {
                crate::data::ColumnKind::Number => "Number",
                crate::data::ColumnKind::Text => "Text",
            }
        )?;
        text(w, "Captured missing rules: ")?;
        missing(w, &info.missing_rules)?;
        for label in &info.value_labels {
            c.checkpoint(0, None)?;
            w.write_all(b"<p>")?;
            cell(w, &label.value)?;
            text(w, " → ")?;
            text(w, &label.label)?;
            w.write_all(b"</p>")?;
        }
        w.write_all(b"</dd>")?;
    }
    w.write_all(b"</dl>")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        analysis::{reliability_with_budget, ReliabilityRequest, ReliabilityReverse},
        data::{Column, ColumnMetadata, Dataset},
        error::AppError,
        operation::CHECKPOINT_HOOK,
    };
    use std::{cell::Cell, rc::Rc};
    fn captured() -> (Dataset, ReliabilityRun) {
        let columns = [1, 2, 3]
            .into_iter()
            .map(|id| {
                let column = Column::numeric(
                    ColumnId(id),
                    &format!("Item {id} <script>"),
                    vec![Some(0.), Some(1.), Some(2.)],
                )
                .unwrap();
                let metadata = ColumnMetadata {
                    description: "Description <b>&\"".into(),
                    ..column.metadata().clone()
                };
                column.with_metadata(metadata).unwrap()
            })
            .collect();
        let data = Dataset::new(columns).unwrap();
        let request = ReliabilityRequest {
            items: vec![ColumnId(1), ColumnId(2), ColumnId(3)],
            filter: Default::default(),
            frequency_weight: None,
            reversals: vec![ReliabilityReverse {
                column: ColumnId(1),
                lower: 0.,
                upper: 2.,
            }],
        };
        let owner = std::mem::size_of::<ReliabilityRequest>() + request.heap_bytes().unwrap();
        let run =
            reliability_with_budget(&data, &request, &JobControl::default(), &[], owner).unwrap();
        (data, run)
    }
    #[test]
    fn captured_report_escapes_text_and_distinguishes_scoring_and_deletion() {
        let (_, captured) = captured();
        let run = ReliabilityRun {
            raw_alpha: Estimate {
                value: None,
                reason: Some("Reason <script>&\"".into()),
            },
            ..captured
        };
        let mut out = Vec::new();
        write(&mut out, RunId(7), &run, &JobControl::default()).unwrap();
        let html = String::from_utf8(out).unwrap();
        for expected in [
            "run-7",
            "Item 1 &lt;script&gt;",
            "Description &lt;b&gt;&amp;&quot;",
            "Reason &lt;script&gt;&amp;&quot;",
            "Complete physical rows: 3",
            "Complete-case frequency N: 3",
            "lower bound 0; upper bound 2",
            "Scored mean",
            "Raw alpha if item removed",
            "Standardized alpha if item removed",
            "Alpha does not establish unidimensionality.",
        ] {
            assert!(html.contains(expected), "missing {expected}");
        }
        assert!(!html.contains("<script>"));
    }
    #[test]
    fn historical_publication_uses_captured_values_and_preserves_destination_on_refusal() {
        let (data, run) = captured();
        let before = run.clone();
        let project = crate::project::Project::new("Reliability <report>", data)
            .unwrap()
            .with_run(crate::analysis::CompletedAnalysis::Reliability(run))
            .unwrap();
        let current = project
            .dataset()
            .edit_cell(ColumnId(1), 0, crate::data::Cell::Number(99.))
            .unwrap();
        let project = project.with_dataset(current).unwrap();
        let path = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "tessera-reliability-report-{}.html",
            std::process::id()
        ));
        std::fs::write(&path, "original").unwrap();
        let cancelled = JobControl::default();
        cancelled.cancel();
        assert!(matches!(
            super::super::export_report(&path, &project, &[RunId(1)], &cancelled),
            Err(AppError::Cancelled)
        ));
        assert!(super::super::export_report_with_budget(
            &path,
            &project,
            &[RunId(1)],
            &JobControl::default(),
            &[],
            crate::data::MAX_OWNED_BYTES
        )
        .is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        std::fs::remove_file(&path).unwrap();
        super::super::export_report(&path, &project, &[RunId(1)], &JobControl::default()).unwrap();
        let html = std::fs::read_to_string(&path).unwrap();
        assert!(html.contains("Historical results: the current data revision differs."));
        assert!(html.contains("Item 1 &lt;script&gt;"));
        assert!(html.contains("Reliability &lt;report&gt;"));
        assert_eq!(
            project.runs()[0].analysis,
            crate::analysis::CompletedAnalysis::Reliability(before)
        );
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn export_owner_precharge_observes_control_before_selection_validation() {
        let (data, run) = captured();
        let project = crate::project::Project::new("Precharge", data)
            .unwrap()
            .with_run(crate::analysis::CompletedAnalysis::Reliability(run))
            .unwrap();
        let control = JobControl::default();
        let cancel = control.clone();
        CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = Some(Box::new(move |_| cancel.cancel())));
        let result = super::super::export_report(
            std::path::Path::new("unused.html"),
            &project,
            &[RunId(999)],
            &control,
        );
        CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
        assert!(matches!(result, Err(AppError::Cancelled)), "{result:?}");
    }
    #[test]
    fn supplied_control_cancels_before_output_and_inside_long_text() {
        let (_, captured) = captured();
        let control = JobControl::default();
        control.cancel();
        let mut out = Vec::new();
        assert!(matches!(
            write(&mut out, RunId(1), &captured, &control),
            Err(AppError::Cancelled)
        ));
        assert!(out.is_empty());
        let mut columns = captured.columns.clone();
        columns[0].description = "x".repeat(100_000);
        let run = ReliabilityRun {
            columns,
            ..captured
        };
        let control = JobControl::default();
        let cancel = control.clone();
        let calls = Rc::new(Cell::new(0));
        let seen = calls.clone();
        CHECKPOINT_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |work| {
                seen.set(seen.get() + 1);
                if work.done >= 4096 {
                    cancel.cancel();
                }
            }))
        });
        let result = write(&mut out, RunId(1), &run, &control);
        CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
        assert!(matches!(result, Err(AppError::Cancelled)), "{result:?}");
        assert!(calls.get() > 1);
        assert!(out.len() < 100_000);
    }
}
