//! Project updates share immutable prior runs.
use crate::{
    analysis::CompletedAnalysis,
    data::{AllocationCounter, Budget, Dataset},
    error::{AppError, AppResult},
    operation::RunId,
};
use std::{mem::size_of, sync::Arc};
mod figures;
mod preparation;
pub use preparation::{DataSnapshot, RecordedStep};
#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisRun {
    pub id: RunId,
    pub analysis: CompletedAnalysis,
}
#[derive(Debug, Clone)]
pub struct Project {
    title: String,
    dataset: Arc<Dataset>,
    runs: Vec<Arc<AnalysisRun>>,
    next_run: RunId,
    provenance: Vec<String>,
    figures: Vec<crate::figures::FigureRecord>,
    presets: Vec<crate::figures::FigurePreset>,
    steps: Vec<Arc<RecordedStep>>,
    next_column: crate::data::ColumnId,
}
impl Project {
    pub fn new(title: &str, dataset: Dataset) -> AppResult<Self> {
        if title.trim().is_empty() || title.chars().any(char::is_control) {
            return Err(AppError::invalid(
                "title",
                "Enter a nonblank title without control characters.",
            ));
        }
        Budget::check_datasets(
            &[&dataset],
            size_of::<Self>() + title.len() + 2 * size_of::<usize>(),
        )?;
        let next_column = dataset.next_column_id()?;
        Ok(Self {
            title: title.into(),
            dataset: Arc::new(dataset),
            runs: vec![],
            next_run: RunId(1),
            provenance: vec![],
            figures: vec![],
            presets: vec![],
            steps: vec![],
            next_column,
        })
    }
    pub fn from_parts(
        title: &str,
        dataset: Dataset,
        runs: Vec<Arc<AnalysisRun>>,
        next_run: RunId,
    ) -> AppResult<Self> {
        Self::from_parts_impl(title, dataset, runs, next_run, None)
    }
    pub fn from_parts_controlled(
        title: &str,
        dataset: Dataset,
        runs: Vec<Arc<AnalysisRun>>,
        next_run: RunId,
        control: &crate::operation::JobControl,
    ) -> AppResult<Self> {
        Self::from_parts_impl(title, dataset, runs, next_run, Some(control))
    }
    fn from_parts_impl(
        title: &str,
        dataset: Dataset,
        runs: Vec<Arc<AnalysisRun>>,
        next_run: RunId,
        control: Option<&crate::operation::JobControl>,
    ) -> AppResult<Self> {
        if let Some(c) = control {
            c.check_cancelled()?;
        }
        let mut input = AllocationCounter::default();
        input.array::<Arc<AnalysisRun>>(runs.capacity())?;
        for run in &runs {
            if let Some(c) = control {
                c.check_cancelled()?;
            }
            if let CompletedAnalysis::Reliability(v) = &run.analysis {
                match control {
                    Some(c) => v.request.admit_controlled(c)?,
                    None => v.request.admit()?,
                };
            }
            if let CompletedAnalysis::Anova(v) = &run.analysis {
                v.request.admit()?;
            }
            input.add(
                match control {
                    Some(c) => run.analysis.owned_bytes_controlled(c)?,
                    None => run.analysis.owned_bytes()?,
                } + size_of::<RunId>()
                    + 2 * size_of::<usize>(),
            )?;
        }
        input.add(size_of::<Self>() + title.len() + 2 * size_of::<usize>())?;
        Budget::check_datasets(&[&dataset], input.bytes)?;
        let project = Self {
            runs,
            next_run,
            ..Self::new(title, dataset)?
        };
        let owned = match control {
            Some(c) => project.owned_bytes_controlled(c)?,
            None => project.owned_bytes()?,
        };
        Budget::check_datasets(&[&project.dataset], owned)?;
        let mut previous = 0;
        for run in &project.runs {
            if run.id.0 <= previous || run.analysis.revision().0 > project.dataset.revision().0 {
                return Err(AppError::invalid(
                    "runs",
                    "Retained run IDs must increase and refer to an existing data revision.",
                ));
            }
            let validation = owned
                .checked_add(match control {
                    Some(c) => run.analysis.validation_scratch_bytes_controlled(c)?,
                    None => run.analysis.validation_scratch_bytes()?,
                })
                .and_then(|n| {
                    n.checked_add(if control.is_none() {
                        crate::operation::JobControl::DEFAULT_HEAP_BYTES
                    } else {
                        0
                    })
                })
                .ok_or(AppError::SizeLimit {
                    limit: crate::data::MAX_OWNED_BYTES,
                })?;
            Budget::check_datasets(&[&project.dataset], validation)?;
            match control {
                Some(c) => run.analysis.validate_controlled(c)?,
                None => run.analysis.validate()?,
            }
            previous = run.id.0;
        }
        if next_run.0 <= previous {
            return Err(AppError::invalid(
                "runs",
                "Next run ID must follow every retained run.",
            ));
        }
        if let Some(c) = control {
            c.check_cancelled()?;
        }
        Ok(project)
    }
    /// Searches only the named stored frequency result. Caller extra excludes this project.
    pub fn frequency_rows_with_budget(
        &self,
        request: &crate::analysis::FrequencySearchRequest,
        control: &crate::operation::JobControl,
        retained: &[&Dataset],
        extra_bytes: usize,
    ) -> AppResult<crate::analysis::FrequencySearchResult> {
        let run = self
            .runs
            .binary_search_by_key(&request.run, |r| r.id)
            .ok()
            .and_then(|i| self.runs.get(i))
            .ok_or_else(|| {
                AppError::invalid(
                    "frequency search",
                    "The saved frequency run does not exist.",
                )
            })?;
        let CompletedAnalysis::Frequencies(result) = &run.analysis else {
            return Err(AppError::invalid(
                "frequency search",
                "Select a saved Frequencies run.",
            ));
        };
        let mut counter = AllocationCounter::default();
        counter.add(extra_bytes)?;
        counter.add(self.owned_bytes()?)?;
        counter.add(request.heap_bytes()?)?;
        let baseline = Budget::dataset_bytes_iter(
            std::iter::once(self.dataset()).chain(retained.iter().copied()),
            counter.bytes,
        )?;
        crate::analysis::frequency_rows_with_budget(result, request, control, baseline)
    }
    pub fn next_run_id(&self) -> RunId {
        self.next_run
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn dataset(&self) -> &Dataset {
        &self.dataset
    }
    pub fn runs(&self) -> &[Arc<AnalysisRun>] {
        &self.runs
    }
    /// Descriptive text only: entries are never opened, fetched or executed.
    pub fn provenance(&self) -> &[String] {
        &self.provenance
    }
    pub fn with_provenance(&self, provenance: Vec<String>) -> AppResult<Self> {
        self.with_provenance_with_budget(provenance, &[], 0)
    }
    /// `extra_bytes` accounts for live UI/input/other-project metadata, excluding self-owned bytes.
    pub fn with_provenance_with_budget(
        &self,
        provenance: Vec<String>,
        retained: &[&Dataset],
        extra_bytes: usize,
    ) -> AppResult<Self> {
        self.with_provenance_impl(provenance, retained, extra_bytes, None)
    }
    pub fn with_provenance_with_budget_controlled(
        &self,
        provenance: Vec<String>,
        retained: &[&Dataset],
        extra_bytes: usize,
        control: &crate::operation::JobControl,
    ) -> AppResult<Self> {
        self.with_provenance_impl(provenance, retained, extra_bytes, Some(control))
    }
    fn with_provenance_impl(
        &self,
        provenance: Vec<String>,
        retained: &[&Dataset],
        extra_bytes: usize,
        control: Option<&crate::operation::JobControl>,
    ) -> AppResult<Self> {
        let output = self.copied_shell_bytes(self.runs.len())?
            + provenance_bytes(&provenance, provenance.capacity())?;
        self.check_update_budget_controlled(None, retained, extra_bytes, output, 0, control)?;
        let next = Self {
            title: copy_string(&self.title, control)?,
            dataset: Arc::clone(&self.dataset),
            runs: self.runs.clone(),
            next_run: self.next_run,
            provenance,
            figures: self.figures.clone(),
            presets: self.presets.clone(),
            steps: self.steps.clone(),
            next_column: self.next_column,
        };
        if let Some(c) = control {
            c.check_cancelled()?;
        }
        Ok(next)
    }
    /// Counts non-dataset owned storage: project/title/provenance, run references/results
    /// and the dataset Arc control block. Shared run results are counted once within
    /// this project; summing multiple project estimates counts shared results again.
    /// Use Budget separately to count unique storage across dataset revisions.
    pub fn owned_bytes(&self) -> AppResult<usize> {
        self.owned_bytes_impl(None)
    }
    pub(crate) fn owned_bytes_controlled(
        &self,
        control: &crate::operation::JobControl,
    ) -> AppResult<usize> {
        self.owned_bytes_impl(Some(control))
    }
    fn owned_bytes_impl(&self, control: Option<&crate::operation::JobControl>) -> AppResult<usize> {
        if let Some(c) = control {
            c.check_cancelled()?;
        }
        let bytes = self.preparation_bytes()?
            + self.figure_bytes()?
            + self.history_bytes_impl(control)?
            + provenance_bytes(&self.provenance, self.provenance.capacity())?
            + self.title.capacity()
            + size_of::<Self>()
            + 2 * size_of::<usize>();
        Budget::check_datasets(&[], bytes)?;
        Ok(bytes)
    }
    /// Counts unique non-dataset storage across retained snapshots without scratch allocation.
    /// Project aliases, shared run Arcs and dataset Arc control blocks count once by identity.
    /// Dataset contents are excluded; use Budget to count their shared storage.
    pub fn retained_owned_bytes(projects: &[&Project]) -> AppResult<usize> {
        Self::retained_owned_bytes_iter(projects.iter().copied())
    }
    /// Iterator form for callers accounting before constructing reference vectors.
    /// Use borrowing iterators whose Clone does not allocate, such as slice/option chains.
    pub fn retained_owned_bytes_iter<'a>(
        projects: impl Iterator<Item = &'a Project> + Clone,
    ) -> AppResult<usize> {
        Self::retained_owned_bytes_iter_controlled(projects, None)
    }
    pub(crate) fn retained_owned_bytes_iter_controlled<'a>(
        projects: impl Iterator<Item = &'a Project> + Clone,
        control: Option<&crate::operation::JobControl>,
    ) -> AppResult<usize> {
        let mut counter = AllocationCounter::default();
        // ponytail: prior-project scans are O(p²); use a fully budgeted identity index if large retained histories become a measured bottleneck.
        if let Some(c) = control {
            c.check_cancelled()?;
        }
        for (index, project) in projects.clone().enumerate() {
            if projects
                .clone()
                .take(index)
                .any(|previous| std::ptr::eq(previous, project))
            {
                continue;
            }
            counter.add(project.figure_bytes()?)?;
            counter.array::<Arc<RecordedStep>>(project.steps.capacity())?;
            for (step_index, step) in project.steps.iter().enumerate() {
                if !projects.clone().take(index).any(|p| {
                    p.steps
                        .get(step_index)
                        .is_some_and(|s| Arc::ptr_eq(s, step))
                }) {
                    counter.add(step.owned_bytes()?)?;
                }
            }
            counter.add(size_of::<Self>())?;
            counter.add(project.title.capacity())?;
            counter.add(provenance_bytes(
                &project.provenance,
                project.provenance.capacity(),
            )?)?;
            counter.array::<Arc<AnalysisRun>>(project.runs.capacity())?;
            if !projects
                .clone()
                .take(index)
                .any(|previous| Arc::ptr_eq(&previous.dataset, &project.dataset))
            {
                counter.add(2 * size_of::<usize>())?;
            }
            for run in &project.runs {
                let shared = projects.clone().take(index).any(|previous| {
                    previous
                        .runs
                        .binary_search_by_key(&run.id, |candidate| candidate.id)
                        .ok()
                        .is_some_and(|position| Arc::ptr_eq(&previous.runs[position], run))
                });
                if !shared {
                    counter.add(
                        match control {
                            Some(c) => run.analysis.owned_bytes_controlled(c)?,
                            None => run.analysis.owned_bytes()?,
                        } + size_of::<RunId>()
                            + 2 * size_of::<usize>(),
                    )?;
                }
            }
        }
        Ok(counter.bytes)
    }
    fn copied_provenance_bytes(&self) -> usize {
        self.provenance.len() * size_of::<String>()
            + self.provenance.iter().map(String::len).sum::<usize>()
    }
    pub fn with_dataset(&self, dataset: Dataset) -> AppResult<Self> {
        self.with_dataset_with_budget(dataset, &[], 0)
    }
    /// Includes source/output and retained dataset storage plus caller-owned work before copying.
    pub fn with_dataset_with_budget(
        &self,
        dataset: Dataset,
        retained: &[&Dataset],
        extra_bytes: usize,
    ) -> AppResult<Self> {
        self.with_prepared_dataset_impl(
            dataset,
            &crate::preparation::Step::DatasetEdit,
            None,
            retained,
            extra_bytes,
        )
    }
    pub fn with_run(&self, analysis: impl Into<CompletedAnalysis>) -> AppResult<Self> {
        self.with_run_with_budget(analysis, &[], 0)
    }
    /// Charges caller-owned work and validation indexes before validating or appending a result.
    pub fn with_run_with_budget(
        &self,
        analysis: impl Into<CompletedAnalysis>,
        retained: &[&Dataset],
        extra_bytes: usize,
    ) -> AppResult<Self> {
        self.with_run_impl(analysis.into(), retained, extra_bytes, None)
    }
    pub fn with_run_with_budget_controlled(
        &self,
        analysis: impl Into<CompletedAnalysis>,
        retained: &[&Dataset],
        extra_bytes: usize,
        control: &crate::operation::JobControl,
    ) -> AppResult<Self> {
        self.with_run_impl(analysis.into(), retained, extra_bytes, Some(control))
    }
    fn with_run_impl(
        &self,
        analysis: CompletedAnalysis,
        retained: &[&Dataset],
        extra_bytes: usize,
        control: Option<&crate::operation::JobControl>,
    ) -> AppResult<Self> {
        if let Some(c) = control {
            c.check_cancelled()?;
        }
        if analysis.revision() != self.dataset.revision() {
            return Err(AppError::Conflict(
                "The data changed before this analysis completed. Run it again.".into(),
            ));
        }
        let output = self.copied_shell_bytes(self.runs.len() + 1)?
            + self.copied_provenance_bytes()
            + match control {
                Some(c) => analysis.owned_bytes_controlled(c)?,
                None => analysis.owned_bytes()?,
            }
            + size_of::<RunId>()
            + 2 * size_of::<usize>();
        self.check_update_budget_controlled(
            None,
            retained,
            extra_bytes,
            output,
            (match control {
                Some(c) => analysis.validation_scratch_bytes_controlled(c)?,
                None => analysis.validation_scratch_bytes()?,
            })
            .checked_add(if control.is_none() {
                crate::operation::JobControl::DEFAULT_HEAP_BYTES
            } else {
                0
            })
            .ok_or(AppError::SizeLimit {
                limit: crate::data::MAX_OWNED_BYTES,
            })?,
            control,
        )?;
        match control {
            Some(c) => analysis.validate_controlled(c)?,
            None => analysis.validate()?,
        }
        if let Some(c) = control {
            c.check_cancelled()?;
        }
        let next_run = self.next_run.next()?;
        let mut runs = Vec::with_capacity(self.runs.len() + 1);
        runs.extend(self.runs.iter().cloned());
        runs.push(Arc::new(AnalysisRun {
            id: self.next_run,
            analysis,
        }));
        let next = Self {
            title: copy_string(&self.title, control)?,
            dataset: Arc::clone(&self.dataset),
            runs,
            next_run,
            provenance: copy_provenance(&self.provenance, control)?,
            figures: self.figures.clone(),
            presets: self.presets.clone(),
            steps: self.steps.clone(),
            next_column: self.next_column,
        };
        if let Some(c) = control {
            c.check_cancelled()?;
        }
        Ok(next)
    }
    fn copied_shell_bytes(&self, runs: usize) -> AppResult<usize> {
        let mut counter = AllocationCounter::default();
        counter.add(self.figure_bytes()?)?;
        counter.array::<Arc<RecordedStep>>(self.steps.len())?;
        counter.add(size_of::<Self>())?;
        counter.add(self.title.len())?;
        counter.array::<Arc<AnalysisRun>>(runs)?;
        Ok(counter.bytes)
    }
    fn check_update_budget(
        &self,
        next: Option<&Dataset>,
        retained: &[&Dataset],
        extra_bytes: usize,
        output: usize,
        scratch: usize,
    ) -> AppResult<()> {
        self.check_update_budget_controlled(next, retained, extra_bytes, output, scratch, None)
    }
    fn check_update_budget_controlled(
        &self,
        next: Option<&Dataset>,
        retained: &[&Dataset],
        extra_bytes: usize,
        output: usize,
        scratch: usize,
        control: Option<&crate::operation::JobControl>,
    ) -> AppResult<()> {
        let mut counter = AllocationCounter::default();
        counter.add(extra_bytes)?;
        counter.add(match control {
            Some(c) => self.owned_bytes_controlled(c)?,
            None => self.owned_bytes()?,
        })?;
        counter.add(output)?;
        counter.add(scratch)?;
        Budget::dataset_bytes_iter_controlled(
            std::iter::once(self.dataset())
                .chain(next)
                .chain(retained.iter().copied()),
            counter.bytes,
            control,
        )
        .map(|_| ())
    }
    /// Borrowed accounting for retained runs and their reference vector.
    /// Dataset storage and the project title are excluded; callers account for those separately.
    pub fn history_bytes(&self) -> AppResult<usize> {
        self.history_bytes_impl(None)
    }
    fn history_bytes_impl(
        &self,
        control: Option<&crate::operation::JobControl>,
    ) -> AppResult<usize> {
        let mut bytes = self.runs.capacity() * size_of::<Arc<AnalysisRun>>();
        for run in &self.runs {
            bytes += match control {
                Some(c) => run.analysis.owned_bytes_controlled(c)?,
                None => run.analysis.owned_bytes()?,
            } + size_of::<RunId>()
                + 2 * size_of::<usize>();
            Budget::check_datasets(&[], bytes)?;
        }
        Ok(bytes)
    }
}

fn provenance_bytes(entries: &[String], capacity: usize) -> AppResult<usize> {
    let bytes = capacity
        .checked_mul(size_of::<String>())
        .and_then(|bytes| {
            entries
                .iter()
                .try_fold(bytes, |total, text| total.checked_add(text.capacity()))
        })
        .ok_or(AppError::SizeLimit {
            limit: crate::data::MAX_OWNED_BYTES,
        })?;
    Budget::check_datasets(&[], bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod reliability_tests {
    use super::*;
    use crate::{
        analysis::*,
        data::{Column, ColumnId, Dataset},
        operation::{JobControl, CHECKPOINT_HOOK},
    };
    fn fixture() -> (Project, CompletedAnalysis) {
        let d = Dataset::new(vec![
            Column::numeric(ColumnId(1), "First", vec![Some(1.), Some(2.), Some(3.)]).unwrap(),
            Column::numeric(ColumnId(2), "Second", vec![Some(2.), Some(4.), Some(6.)]).unwrap(),
        ])
        .unwrap();
        let r = ReliabilityRequest {
            items: vec![ColumnId(1), ColumnId(2)],
            filter: RowFilter::default(),
            frequency_weight: None,
            reversals: vec![],
        };
        let mut a = reliability_with_budget(
            &d,
            &r,
            &JobControl::default(),
            &[],
            size_of::<ReliabilityRequest>() + r.heap_bytes().unwrap(),
        )
        .unwrap();
        a.columns[0].description = "captured description ".repeat(4096);
        (
            Project::new("History", d).unwrap(),
            CompletedAnalysis::Reliability(a),
        )
    }
    #[test]
    fn supplied_project_control_cancels_inside_captured_string_validation() {
        for restore in [false, true] {
            let (p, a) = fixture();
            let c = JobControl::default();
            let cancel = c.clone();
            CHECKPOINT_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |progress| {
                    if progress.done >= 4096 {
                        cancel.cancel();
                    }
                }))
            });
            let result = if restore {
                Project::from_parts_controlled(
                    "History",
                    p.dataset().clone(),
                    vec![Arc::new(AnalysisRun {
                        id: RunId(1),
                        analysis: a,
                    })],
                    RunId(2),
                    &c,
                )
            } else {
                p.with_run_with_budget_controlled(a, &[], 0, &c)
            };
            CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
            assert!(matches!(result, Err(AppError::Cancelled)));
            assert!(p.runs().is_empty());
        }
    }
    #[test]
    fn actual_project_precharge_uses_supplied_control_before_capacity_refusal() {
        let (p, mut a) = fixture();
        let CompletedAnalysis::Reliability(r) = &mut a else {
            unreachable!()
        };
        r.columns[0].description = String::with_capacity(crate::data::MAX_OWNED_BYTES);
        let c = JobControl::default();
        let cancel = c.clone();
        CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = Some(Box::new(move |_| cancel.cancel())));
        let result = p.with_run_with_budget_controlled(a, &[], 0, &c);
        CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
        assert!(matches!(result, Err(AppError::Cancelled)), "{result:?}");
    }
    #[test]
    fn supplied_precancelled_project_control_allocates_no_fallback() {
        let (p, a) = fixture();
        let c = JobControl::default();
        c.cancel();
        let (result, allocated) = crate::data::measure_test_allocations(|| {
            p.with_run_with_budget_controlled(a, &[], usize::MAX, &c)
        });
        assert!(matches!(result, Err(AppError::Cancelled)));
        assert_eq!(allocated, 0);
    }
}

#[cfg(test)]
mod survey_controlled_tests {
    use super::*;
    use crate::operation::{JobControl, CHECKPOINT_HOOK};
    #[test]
    fn controlled_provenance_matches_existing_and_cancels_during_copy() {
        let p = Project::new(&"a".repeat(20000), Dataset::new(vec![]).unwrap()).unwrap();
        let entries = vec!["Survey audit".into()];
        let old = p.with_provenance(entries.clone()).unwrap();
        let new = p
            .with_provenance_with_budget_controlled(entries, &[], 0, &JobControl::default())
            .unwrap();
        assert_eq!(old.owned_bytes().unwrap(), new.owned_bytes().unwrap());
        assert_eq!(old.provenance(), new.provenance());
        let c = JobControl::default();
        let cancel = c.clone();
        CHECKPOINT_HOOK.with(|h| {
            *h.borrow_mut() = Some(Box::new(move |progress| {
                if progress.done == 4096 {
                    cancel.cancel();
                }
            }))
        });
        let result = p.with_provenance_with_budget_controlled(vec![], &[], 0, &c);
        CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
        assert!(matches!(result, Err(AppError::Cancelled)));
        assert!(p.provenance().is_empty());
    }
}

// Copies long owned text in bounded chunks so cancellation remains responsive.
pub(crate) fn copy_string(
    value: &str,
    control: Option<&crate::operation::JobControl>,
) -> AppResult<String> {
    let Some(control) = control else {
        return Ok(value.to_owned());
    };
    let quiet = control.without_progress();
    quiet.checkpoint(0, None)?;
    let mut copied = String::with_capacity(value.len());
    let mut start = 0;
    while start < value.len() {
        quiet.checkpoint(start, None)?;
        let mut end = (start + 4096).min(value.len());
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        copied.push_str(&value[start..end]);
        start = end;
    }
    quiet.check_cancelled()?;
    Ok(copied)
}
fn copy_provenance(
    values: &[String],
    control: Option<&crate::operation::JobControl>,
) -> AppResult<Vec<String>> {
    let mut copied = Vec::with_capacity(values.len());
    for value in values {
        copied.push(copy_string(value, control)?);
    }
    Ok(copied)
}
