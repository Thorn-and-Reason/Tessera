//! Preparation provenance and data-only undo snapshots share immutable records.
use super::*;
use crate::{
    data::{Column, ColumnId, RevisionId},
    operation::JobControl,
    preparation::{Step, MAX_STEPS},
    project_io::wire,
};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedStep {
    #[serde(with = "wire::revision_id")]
    pub source_revision: RevisionId,
    #[serde(with = "wire::revision_id")]
    pub result_revision: RevisionId,
    pub step: Step,
}
impl RecordedStep {
    pub fn owned_bytes(&self) -> AppResult<usize> {
        let mut n = AllocationCounter::default();
        n.add(size_of::<Self>() + 2 * size_of::<usize>())?;
        n.add(self.step.owned_bytes()?.saturating_sub(size_of::<Step>()))?;
        Ok(n.bytes)
    }
}
#[derive(Debug, Clone)]
pub struct DataSnapshot {
    dataset: Arc<Dataset>,
    steps: Vec<Arc<RecordedStep>>,
}
impl DataSnapshot {
    pub fn dataset(&self) -> &Dataset {
        &self.dataset
    }
    pub fn steps(&self) -> &[Arc<RecordedStep>] {
        &self.steps
    }
    /// Shell ownership only: callers account dataset chunks and unique record Arcs separately.
    pub fn owned_bytes(&self) -> AppResult<usize> {
        let mut n = AllocationCounter::default();
        n.add(size_of::<Self>() + 2 * size_of::<usize>())?;
        n.array::<Arc<RecordedStep>>(self.steps.capacity())?;
        Ok(n.bytes)
    }
}
impl Project {
    pub fn steps(&self) -> &[Arc<RecordedStep>] {
        &self.steps
    }
    pub fn next_column_id(&self) -> ColumnId {
        self.next_column
    }
    pub fn preparation_bytes(&self) -> AppResult<usize> {
        let mut n = AllocationCounter::default();
        n.array::<Arc<RecordedStep>>(self.steps.capacity())?;
        for r in &self.steps {
            n.add(r.owned_bytes()?)?;
        }
        Ok(n.bytes)
    }
    pub fn data_snapshot_with_budget(
        &self,
        retained: &[&Dataset],
        extra: usize,
    ) -> AppResult<DataSnapshot> {
        let mut n = AllocationCounter::default();
        n.add(size_of::<DataSnapshot>() + 2 * size_of::<usize>())?;
        n.array::<Arc<RecordedStep>>(self.steps.len())?;
        self.check_update_budget(None, retained, extra, n.bytes, 0)?;
        Ok(DataSnapshot {
            dataset: Arc::clone(&self.dataset),
            steps: self.steps.clone(),
        })
    }
    pub(crate) fn admit_preparation(&self, step: &Step) -> AppResult<()> {
        if self.steps.len() >= MAX_STEPS
            && !(matches!(step, Step::DatasetEdit)
                && self
                    .steps
                    .last()
                    .is_some_and(|r| matches!(r.step, Step::DatasetEdit)))
        {
            return Err(AppError::invalid(
                "preparation",
                "A project can retain at most 256 preparation records.",
            ));
        }
        step.validate_shape()
    }
    pub(crate) fn with_prepared_dataset(
        &self,
        dataset: Dataset,
        step: &Step,
        control: &JobControl,
        retained: &[&Dataset],
        extra: usize,
    ) -> AppResult<Self> {
        self.with_prepared_dataset_impl(dataset, step, Some(control), retained, extra)
    }
    pub(super) fn with_prepared_dataset_impl(
        &self,
        dataset: Dataset,
        step: &Step,
        control: Option<&JobControl>,
        retained: &[&Dataset],
        extra: usize,
    ) -> AppResult<Self> {
        if let Some(control) = control {
            control.check_cancelled()?;
        }
        self.admit_preparation(step)?;
        if dataset.revision() <= self.dataset.revision() {
            return Err(AppError::Conflict(
                "A replacement dataset must advance the active data revision.".into(),
            ));
        }
        for column in dataset.columns() {
            if column.id() < self.next_column
                && !self
                    .dataset
                    .columns()
                    .iter()
                    .any(|old| old.id() == column.id())
            {
                return Err(AppError::Conflict(
                    "New columns must use an unissued column identity.".into(),
                ));
            }
        }
        let next_column = self.next_column.max(dataset.next_column_id()?);
        let coalesce = matches!(step, Step::DatasetEdit)
            && self
                .steps
                .last()
                .is_some_and(|r| matches!(r.step, Step::DatasetEdit));
        let len = self.steps.len() + usize::from(!coalesce);
        let mut n = AllocationCounter::default();
        n.add(
            self.copied_shell_bytes(self.runs.len())?
                + self.copied_provenance_bytes()
                + 2 * size_of::<usize>(),
        )?;
        n.array::<Arc<RecordedStep>>(len)?;
        n.add(step.owned_bytes()? + size_of::<RecordedStep>() + 2 * size_of::<usize>())?;
        self.check_update_budget_controlled(Some(&dataset), retained, extra, n.bytes, 0, control)?;
        let source_revision = if coalesce {
            self.steps.last().unwrap().source_revision
        } else {
            self.dataset.revision()
        };
        let record = RecordedStep {
            source_revision,
            result_revision: dataset.revision(),
            step: step.clone(),
        };
        let mut steps = Vec::with_capacity(len);
        steps.extend(
            self.steps
                .iter()
                .take(self.steps.len() - usize::from(coalesce))
                .cloned(),
        );
        steps.push(Arc::new(record));
        let result = Self {
            dataset: Arc::new(dataset),
            steps,
            next_column,
            title: self.title.clone(),
            runs: self.runs.clone(),
            next_run: self.next_run,
            provenance: self.provenance.clone(),
            figures: self.figures.clone(),
            presets: self.presets.clone(),
        };
        if let Some(control) = control {
            control.check_cancelled()?;
        }
        Ok(result)
    }
    pub fn with_restored_dataset_and_steps(
        &self,
        snapshot: &DataSnapshot,
        control: &JobControl,
        retained: &[&Dataset],
        extra: usize,
    ) -> AppResult<Self> {
        control.check_cancelled()?;
        let revision = self.dataset.revision().next()?;
        let mut n = AllocationCounter::default();
        n.add(
            self.copied_shell_bytes(self.runs.len())?
                + self.copied_provenance_bytes()
                + 2 * size_of::<usize>(),
        )?;
        n.array::<Column>(snapshot.dataset.column_count())?;
        n.array::<Arc<RecordedStep>>(snapshot.steps.len())?;
        n.add(snapshot.dataset.column_count().saturating_mul(512))?;
        for col in snapshot.dataset.columns() {
            n.add(col.metadata().validation_work()?)?;
            n.array::<Arc<[usize]>>(col.len().div_ceil(crate::data::CHUNK_SIZE))?;
        }
        self.check_update_budget_controlled(
            Some(snapshot.dataset()),
            retained,
            extra,
            n.bytes,
            0,
            Some(control),
        )?;
        let dataset = Dataset::from_revision(snapshot.dataset.columns().to_vec(), revision)?;
        let result = Self {
            dataset: Arc::new(dataset),
            steps: snapshot.steps.clone(),
            next_column: self.next_column.max(snapshot.dataset.next_column_id()?),
            title: self.title.clone(),
            runs: self.runs.clone(),
            next_run: self.next_run,
            provenance: self.provenance.clone(),
            figures: self.figures.clone(),
            presets: self.presets.clone(),
        };
        control.check_cancelled()?;
        Ok(result)
    }
    pub(crate) fn with_saved_preparation(
        self,
        steps: Vec<Arc<RecordedStep>>,
        next_column: ColumnId,
        control: &JobControl,
    ) -> AppResult<Self> {
        if steps.len() > MAX_STEPS
            || next_column.0 == 0
            || self
                .dataset
                .columns()
                .iter()
                .any(|col| col.id() >= next_column)
        {
            return Err(AppError::invalid(
                "preparation",
                "Invalid record count or next column identity.",
            ));
        }
        let mut previous = RevisionId(0);
        for record in &steps {
            control.check_cancelled()?;
            record.step.validate_shape()?;
            if record.source_revision.0 == 0
                || record.source_revision < previous
                || record.result_revision <= record.source_revision
                || record.result_revision > self.dataset.revision()
                || record.step.source_ids().any(|id| id >= next_column)
            {
                return Err(AppError::invalid("preparation","Preparation records must refer to ordered prior data revisions and issued column identities."));
            }
            previous = record.result_revision;
        }
        let result = Self {
            steps,
            next_column,
            ..self
        };
        Budget::check_datasets(&[result.dataset()], result.owned_bytes()?)?;
        control.check_cancelled()?;
        Ok(result)
    }
}
