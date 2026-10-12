//! Atomic survey batches built from the existing descriptive engines.
use super::{inspect_with_budget, SurveyRequest, SurveyRole};
use crate::{
    analysis::{
        self, AnalysisRequest, DescribeRequest, FrequenciesRequest, HistogramBins,
        HistogramRequest, MissingPolicy, RowFilter, Statistic,
    },
    data::{AllocationCounter, Budget, ColumnId, Dataset},
    error::AppResult,
    operation::JobControl,
    project::Project,
};
use std::{fmt::Write, mem::size_of};

/// Builds a complete candidate without changing the input project. `extra_bytes`
/// covers caller storage outside `project`, `request`, and the retained datasets.
pub fn generate_with_budget(
    project: &Project,
    request: &SurveyRequest,
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<Project> {
    generate_presented_with_budget(
        project,
        request,
        super::SurveyPresentation::default(),
        control,
        retained,
        extra_bytes,
    )
}

pub fn generate_presented_with_budget(
    project: &Project,
    request: &SurveyRequest,
    presentation: super::SurveyPresentation,
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<Project> {
    request.validate(project.dataset(), control)?;
    let mut held = AllocationCounter::default();
    held.add(extra_bytes)?;
    held.add(request.owned_bytes()?)?;
    let original_bytes = project.owned_bytes_controlled(control)?;
    let review = inspect_with_budget(
        project.dataset(),
        control,
        retained,
        sum(&[held.bytes, original_bytes])?,
    )?;
    held.add(review.owned_bytes()?)?;
    // Reserve both role lists before allocating either.
    let mut role_storage = AllocationCounter::default();
    role_storage.array::<ColumnId>(request.choices.len().checked_mul(2).ok_or(
        crate::error::AppError::SizeLimit {
            limit: crate::data::MAX_OWNED_BYTES,
        },
    )?)?;
    let audit_capacity = sum(&[
        128,
        review
            .columns
            .len()
            .checked_mul(512)
            .ok_or(crate::error::AppError::SizeLimit {
                limit: crate::data::MAX_OWNED_BYTES,
            })?,
    ])?;
    let mut audit_storage = AllocationCounter::default();
    audit_storage.add(audit_capacity)?;
    audit_storage.array::<String>(project.provenance().len() + 1)?;
    for entry in project.provenance() {
        control.check_cancelled()?;
        audit_storage.add(entry.len())?;
    }
    Budget::context_bytes_controlled(
        project.dataset(),
        retained,
        None,
        sum(&[
            held.bytes,
            role_storage.bytes,
            original_bytes,
            audit_storage.bytes,
        ])?,
        control,
    )?;
    let mut categories = Vec::with_capacity(request.choices.len());
    let mut measures = Vec::with_capacity(request.choices.len());
    for choice in &request.choices {
        control.check_cancelled()?;
        match choice.role {
            SurveyRole::Categories => categories.push(choice.column),
            SurveyRole::NumericMeasure => measures.push(choice.column),
            _ => {}
        }
    }
    let total_runs = usize::from(!categories.is_empty())
        + usize::from(!measures.is_empty())
        + review
            .columns
            .iter()
            .filter(|column| column.valid != 0 && measures.contains(&column.column))
            .count();
    let mut audit = String::with_capacity(audit_capacity);
    write!(audit, "Instant survey report. Complete cells counted as categories; no filter or weight. Duplicate rows retained: {}. Variables:", review.duplicate_rows).expect("writing to String");
    for column in &review.columns {
        control.check_cancelled()?;
        let role = request
            .choices
            .iter()
            .find(|c| c.column == column.column)
            .map_or(SurveyRole::Exclude, |c| c.role);
        write!(audit, " {}={}", column.column.0, role.label()).expect("writing to String");
        if !column.findings.is_empty() {
            audit.push_str(" (review findings:");
            for finding in &column.findings {
                write!(audit, " {};", finding.label()).expect("writing to String");
            }
            audit.push(')');
        }
        audit.push(';');
    }
    let mut provenance = Vec::with_capacity(project.provenance().len() + 1);
    for entry in project.provenance() {
        control.check_cancelled()?;
        provenance.push(crate::project::copy_string(entry, Some(control))?);
    }
    provenance.push(audit);
    control.check_cancelled()?;
    let mut candidate = project.with_provenance_with_budget_controlled(
        provenance,
        retained,
        sum(&[held.bytes, role_storage.bytes])?,
        control,
    )?;
    if !categories.is_empty() {
        candidate = append(
            project,
            &candidate,
            AnalysisRequest::Frequencies(FrequenciesRequest {
                group: None,
                columns: categories,
                filter: RowFilter::default(),
                frequency_weight: None,
            }),
            control,
            retained,
            sum(&[held.bytes, measures.capacity() * size_of::<ColumnId>()])?,
        )?;
        control.checkpoint(
            candidate.runs().len() - project.runs().len(),
            Some(total_runs),
        )?;
    } else {
        drop(categories);
    }
    if !measures.is_empty() {
        // Charge the statistic vector before constructing the Describe recipe;
        // its column list is the already-live measures vector, transferred below.
        let mut recipe_storage = AllocationCounter::default();
        recipe_storage.array::<ColumnId>(measures.capacity())?;
        recipe_storage.array::<Statistic>(4)?;
        recipe_storage.add(size_of::<AnalysisRequest>())?;
        let combined = Project::retained_owned_bytes_iter_controlled(
            [project, &candidate].into_iter(),
            Some(control),
        )?;
        Budget::context_bytes_controlled(
            project.dataset(),
            retained,
            None,
            sum(&[held.bytes, recipe_storage.bytes, combined])?,
            control,
        )?;
        candidate = append(
            project,
            &candidate,
            AnalysisRequest::Describe(DescribeRequest {
                columns: measures,
                statistics: vec![
                    Statistic::Mean,
                    Statistic::SampleStdDev,
                    Statistic::Minimum,
                    Statistic::Maximum,
                ],
                missing_policy: MissingPolicy::PerVariable,
                filter: RowFilter::default(),
                frequency_weight: None,
            }),
            control,
            retained,
            held.bytes,
        )?;
        control.checkpoint(
            candidate.runs().len() - project.runs().len(),
            Some(total_runs),
        )?;
    } else {
        drop(measures);
    }
    for column in &review.columns {
        control.check_cancelled()?;
        if column.valid != 0
            && request
                .choices
                .iter()
                .any(|c| c.column == column.column && c.role == SurveyRole::NumericMeasure)
        {
            candidate = append(
                project,
                &candidate,
                AnalysisRequest::Histogram(HistogramRequest {
                    variable: column.column,
                    bins: HistogramBins::Automatic,
                    filter: RowFilter::default(),
                    frequency_weight: None,
                }),
                control,
                retained,
                held.bytes,
            )?;
            control.checkpoint(
                candidate.runs().len() - project.runs().len(),
                Some(total_runs),
            )?;
        }
    }
    control.check_cancelled()?;
    super::presentation::present(
        project,
        candidate,
        &review,
        presentation,
        control,
        retained,
        held.bytes,
    )
}

fn sum(values: &[usize]) -> AppResult<usize> {
    let mut bytes = AllocationCounter::default();
    for value in values {
        bytes.add(*value)?;
    }
    Ok(bytes.bytes)
}
fn append(
    original: &Project,
    candidate: &Project,
    request: AnalysisRequest,
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<Project> {
    let extra_bytes = sum(&[
        extra_bytes,
        size_of::<AnalysisRequest>(),
        request.heap_bytes()?,
    ])?;
    let combined = Project::retained_owned_bytes_iter_controlled(
        [original, candidate].into_iter(),
        Some(control),
    )?;
    let all_live = sum(&[extra_bytes, combined])?;
    Budget::context_bytes_controlled(candidate.dataset(), retained, None, all_live, control)?;
    let result =
        analysis::analyze_with_budget(candidate.dataset(), &request, control, retained, all_live)?;
    // The append method charges the candidate and result itself. Only the
    // original's distinct shell and caller storage belong in its extra context.
    let extra = sum(&[
        extra_bytes,
        combined - candidate.owned_bytes_controlled(control)?,
    ])?;
    candidate.with_run_with_budget_controlled(result, retained, extra, control)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{data::Column, operation::CHECKPOINT_HOOK, survey::SurveyChoice};
    #[test]
    fn cancellation_after_a_completed_run_discards_the_batch() {
        let p = Project::new(
            "Synthetic",
            Dataset::new(vec![
                Column::numeric(ColumnId(1), "Measure", vec![Some(1.), Some(2.), Some(3.)])
                    .unwrap(),
                Column::text(ColumnId(2), "Choice", vec![Some("Yes".into()); 3]).unwrap(),
            ])
            .unwrap(),
        )
        .unwrap();
        let r = SurveyRequest {
            revision: p.dataset().revision(),
            choices: vec![
                SurveyChoice {
                    column: ColumnId(1),
                    role: SurveyRole::NumericMeasure,
                },
                SurveyChoice {
                    column: ColumnId(2),
                    role: SurveyRole::Categories,
                },
            ],
        };
        let c = JobControl::default();
        let cancel = c.clone();
        CHECKPOINT_HOOK.with(|h| {
            *h.borrow_mut() = Some(Box::new(move |progress| {
                if progress.done == 1 && progress.total == Some(3) {
                    cancel.cancel();
                }
            }))
        });
        let result = generate_with_budget(&p, &r, &c, &[], 0);
        CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
        assert!(matches!(result, Err(crate::error::AppError::Cancelled)));
        assert!(p.runs().is_empty());
        assert!(p.provenance().is_empty());
    }
}

#[cfg(test)]
mod reservation_tests {
    use super::*;
    use crate::{
        data::{Column, MAX_OWNED_BYTES},
        survey::SurveyChoice,
    };
    #[test]
    fn batch_boundary_excludes_the_consumed_category_recipe() {
        let control = JobControl::default();
        let p = Project::new(
            "Boundary",
            Dataset::new(vec![
                Column::text(ColumnId(1), "Choice", vec![Some("Yes".into()); 2000]).unwrap(),
                Column::numeric(
                    ColumnId(2),
                    "Measure",
                    (0..2000).map(|i| Some(i as f64)).collect(),
                )
                .unwrap(),
            ])
            .unwrap(),
        )
        .unwrap();
        let request = SurveyRequest {
            revision: p.dataset().revision(),
            choices: vec![
                SurveyChoice {
                    column: ColumnId(1),
                    role: SurveyRole::Categories,
                },
                SurveyChoice {
                    column: ColumnId(2),
                    role: SurveyRole::NumericMeasure,
                },
            ],
        };
        let full = generate_with_budget(&p, &request, &control, &[], 0).unwrap();
        let candidate = p
            .with_provenance(full.provenance().to_vec())
            .unwrap()
            .with_run(full.runs()[0].analysis.clone())
            .unwrap();
        let review = inspect_with_budget(p.dataset(), &control, &[], 0).unwrap();
        let mut columns = Vec::with_capacity(request.choices.len());
        columns.push(ColumnId(2));
        let recipe = AnalysisRequest::Describe(DescribeRequest {
            columns,
            statistics: vec![
                Statistic::Mean,
                Statistic::SampleStdDev,
                Statistic::Minimum,
                Statistic::Maximum,
            ],
            missing_policy: MissingPolicy::PerVariable,
            filter: RowFilter::default(),
            frequency_weight: None,
        });
        let live = request.owned_bytes().unwrap()
            + review.owned_bytes().unwrap()
            + size_of::<AnalysisRequest>()
            + recipe.heap_bytes().unwrap()
            + Project::retained_owned_bytes(&[&p, &candidate]).unwrap();
        // The dominating peak is Project's Describe append: its new shell and
        // result coexist with both old projects and per-point validation indexes.
        let result_storage = full.runs()[1].analysis.owned_bytes().unwrap();
        let output = size_of::<Project>()
            + "Boundary".len()
            + 2 * size_of::<std::sync::Arc<crate::project::AnalysisRun>>()
            + size_of::<String>()
            + candidate.provenance()[0].len()
            + result_storage
            + size_of::<crate::operation::RunId>()
            + 2 * size_of::<usize>();
        let baseline = Budget::dataset_bytes(
            &[p.dataset()],
            live + output + full.runs()[1].analysis.validation_scratch_bytes().unwrap(),
        )
        .unwrap();
        // Five dataset/metadata/chunk identities require 4-to-8 pointer growth;
        // both ownership-index buffers remain live at that checkpoint.
        let peak = baseline + 12 * size_of::<*const ()>();
        let extra = MAX_OWNED_BYTES - peak;
        let result = generate_with_budget(&p, &request, &control, &[], extra);
        assert!(
            result.is_ok(),
            "A batch at the independently calculated peak must fit: {result:?}"
        );
        assert!(matches!(
            generate_with_budget(&p, &request, &control, &[], extra + 1),
            Err(crate::error::AppError::SizeLimit { .. })
        ));
    }
}
