//! One owned operation at a time. UI code polls; only finished workers are joined.
pub use crate::operation::JobControl;
mod excel;
mod execute;
mod figures;
mod frequency_view;
#[cfg(test)]
mod recovery;
use crate::import::excel::{
    self as excel_import, ExcelDecisions, ExcelPreview, ExcelSelection, ExcelWorkbook,
};
use crate::{
    analysis::{
        self, AnalysisRequest, CompletedAnalysis, FrequencySearchRequest, FrequencySearchResult,
        GroupLookupRequest, GroupLookupResult,
    },
    data::{Budget, ColumnId, ColumnKind, Dataset, RevisionId, MAX_OWNED_BYTES},
    error::{AppError, AppResult},
    export,
    files::{self, FileStamp, Publication, SaveOutcome},
    import::{self, Delimiter, ImportColumnChoice, ImportPreview, MAX_INPUT_BYTES},
    operation::{JobToken, Progress, RunId},
    preparation::{self, Step},
    project::Project,
    recovery::{RecoveryEntry, RecoveryStore},
};
pub use frequency_view::{FrequencyMetadata, FrequencyRowsOutput};
use std::{
    fs::File,
    io::Read,
    mem::size_of,
    path::PathBuf,
    sync::{
        mpsc::{self, Receiver},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
};

// Covers control/channel bookkeeping, one terminal slot and four Vec slots for inline events.
// Thread stacks and allocator/RSS overhead are outside the owned-work accounting policy.
const JOB_WORK_BYTES: usize =
    4 * size_of::<JobEvent>() + size_of::<AppResult<JobOutput>>() + 4 * 1024;

#[derive(Debug, Default)]
pub struct RetainedWork {
    pub projects: Vec<Arc<Project>>,
    pub datasets: Vec<Arc<Dataset>>,
    pub recovery_stores: Vec<RecoveryStore>,
    pub previews: Vec<Arc<ImportPreview>>,
    pub excel_workbooks: Vec<Arc<ExcelWorkbook>>,
    pub excel_previews: Vec<Arc<ExcelPreview>>,
    pub group_values: Vec<Arc<GroupLookupResult>>,
    pub frequency_rows: Vec<Arc<FrequencySearchResult>>,
    pub frequency_metadata: Vec<Arc<FrequencyMetadata>>,
    pub prepare_frequency_metadata: bool,
    /// Other live UI/pending inputs not represented above. Do not include the task's input twice.
    pub extra_bytes: usize,
}
#[derive(Debug)]
pub struct JobTask {
    pub operation: JobOperation,
    pub retained: RetainedWork,
}
impl JobTask {
    pub fn new(operation: JobOperation) -> Self {
        Self {
            operation,
            retained: RetainedWork::default(),
        }
    }
}
#[derive(Debug)]
pub enum JobOperation {
    DiscoverRecovery {
        store: RecoveryStore,
    },
    RetireCheckpoint {
        store: RecoveryStore,
    },
    DiscardRecovery {
        store: RecoveryStore,
        entry: RecoveryEntry,
    },
    Prepare {
        project: Arc<Project>,
        step: Step,
    },
    Checkpoint {
        project: Arc<Project>,
        store: RecoveryStore,
        sequence: u64,
    },
    Recover {
        store: RecoveryStore,
        entry: RecoveryEntry,
    },
    InspectExcelPath {
        path: PathBuf,
    },
    PreviewExcel {
        workbook: Arc<ExcelWorkbook>,
        selection: ExcelSelection,
    },
    CommitExcel {
        preview: Arc<ExcelPreview>,
        choices: Vec<ImportColumnChoice>,
        decisions: ExcelDecisions,
    },
    PreviewPath {
        path: PathBuf,
        delimiter: Delimiter,
    },
    CommitImport {
        preview: Arc<ImportPreview>,
        choices: Vec<ImportColumnChoice>,
    },
    Open {
        path: PathBuf,
    },
    Analyze {
        project: Arc<Project>,
        request: AnalysisRequest,
    },
    FrequencySearch {
        project: Arc<Project>,
        request: FrequencySearchRequest,
    },
    GroupLookup {
        project: Arc<Project>,
        request: GroupLookupRequest,
    },
    ConvertColumn {
        project: Arc<Project>,
        id: ColumnId,
        kind: ColumnKind,
    },
    Save {
        project: Arc<Project>,
        path: PathBuf,
        expected: Option<FileStamp>,
    },
    ExportData {
        project: Arc<Project>,
        path: PathBuf,
        delimiter: Delimiter,
    },
    ExportFigures {
        project: Arc<Project>,
        directory: PathBuf,
        keys: Vec<crate::figures::FigureKey>,
        format: crate::export::figures::FigureFormat,
    },
    ExportFigure {
        project: Arc<Project>,
        path: PathBuf,
        key: crate::figures::FigureKey,
        format: crate::export::figures::FigureFormat,
    },
    ExportReport {
        project: Arc<Project>,
        path: PathBuf,
        runs: Vec<RunId>,
    },
}
// One completed result stays inline in the single-worker channel; its storage is precharged.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum JobOutput {
    RecoveryEntries(Vec<RecoveryEntry>),
    RecoveryRetired,
    RecoveryDiscarded,
    Prepared(Arc<Project>),
    Checkpoint(SaveOutcome),
    Recovered(Arc<Project>),
    ExcelWorkbook(Arc<ExcelWorkbook>),
    ExcelPreview(Arc<ExcelPreview>),
    Preview(Arc<ImportPreview>),
    Imported(Dataset),
    Opened {
        project: Arc<Project>,
        path: PathBuf,
        stamp: FileStamp,
    },
    Analyzed(CompletedAnalysis),
    GroupValues(Arc<GroupLookupResult>),
    FrequencyRows(FrequencyRowsOutput),
    Converted(Dataset),
    Saved(SaveOutcome),
    Exported(SaveOutcome),
    FiguresExported {
        outcomes: Vec<SaveOutcome>,
    },
}
impl JobOutput {
    pub fn file_outcome(&self) -> Option<&SaveOutcome> {
        match self {
            Self::Saved(outcome) | Self::Exported(outcome) => Some(outcome),
            _ => None,
        }
    }
}
// At most three events per poll; keeping the terminal value inline avoids a separate allocation.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum JobEvent {
    Progress {
        token: JobToken,
        progress: Progress,
    },
    CancellationRequested {
        token: JobToken,
    },
    Completed {
        token: JobToken,
        cancelled: bool,
        result: AppResult<JobOutput>,
    },
}
pub fn may_publish(
    token: JobToken,
    active_document: u64,
    active_revision: RevisionId,
    generation: u64,
) -> bool {
    token.document == active_document
        && token.revision == active_revision
        && token.generation == generation
}
impl JobEvent {
    pub fn token(&self) -> JobToken {
        match self {
            Self::Progress { token, .. }
            | Self::CancellationRequested { token }
            | Self::Completed { token, .. } => *token,
        }
    }
    pub fn may_publish(&self, document: u64, revision: RevisionId, generation: u64) -> bool {
        matches!(
            self,
            Self::Completed {
                cancelled: false,
                result: Ok(_),
                ..
            }
        ) && may_publish(self.token(), document, revision, generation)
    }
    /// Publication is a completed external fact, independent of current UI identity/cancellation.
    pub fn published_file(&self) -> Option<&SaveOutcome> {
        match self {
            Self::Completed {
                result: Ok(output), ..
            } => output
                .file_outcome()
                .filter(|outcome| outcome.committed_stamp.is_some()),
            _ => None,
        }
    }
    /// Only this condition may clear dirty state or adopt the saved document path.
    pub fn confirms_current_save(
        &self,
        document: u64,
        revision: RevisionId,
        generation: u64,
    ) -> bool {
        self.may_publish(document, revision, generation)
            && matches!(self,
            Self::Completed { result: Ok(JobOutput::Saved(outcome)), .. }
            if outcome.publication == Publication::Committed && outcome.committed_stamp.is_some() && outcome.revision == Some(revision))
    }
}
struct Active {
    token: JobToken,
    control: JobControl,
    latest: Arc<Mutex<Option<Progress>>>,
    result: Receiver<AppResult<JobOutput>>,
    worker: JoinHandle<()>,
    cancelled: bool,
    cancellation_pending: bool,
}
#[derive(Default)]
#[must_use = "Poll and report completion, including file outcomes, before closing the application."]
pub struct JobRunner {
    active: Option<Active>,
    closing: bool,
    completion_control: Option<JobControl>,
}
impl JobRunner {
    pub fn start(&mut self, task: JobTask, token: JobToken) -> AppResult<()> {
        if task
            .operation
            .project()
            .is_some_and(|project| project.dataset().revision() != token.revision)
        {
            return Err(AppError::Conflict(
                "The operation token does not match its input revision.".into(),
            ));
        }
        self.ensure_available()?;
        task.preflight()?;
        self.start_work(token, move |control| task.execute_preflighted(control))
    }
    fn start_work(
        &mut self,
        token: JobToken,
        work: impl FnOnce(&JobControl) -> AppResult<JobOutput> + Send + 'static,
    ) -> AppResult<()> {
        self.ensure_available()?;
        let latest = Arc::new(Mutex::new(None));
        let control = JobControl::coalesced(latest.clone());
        let worker_control = control.clone();
        let (sender, result) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("tessera-operation".into())
            .spawn(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    worker_control.checkpoint(0, None)?;
                    work(&worker_control)
                }))
                .unwrap_or_else(|_| Err(worker_failed()));
                let _ = sender.send(outcome);
            })?;
        self.active = Some(Active {
            token,
            control,
            latest,
            result,
            worker,
            cancelled: false,
            cancellation_pending: false,
        });
        Ok(())
    }
    fn ensure_available(&self) -> AppResult<()> {
        if self.closing || self.active.is_some() {
            Err(AppError::Conflict(
                "Wait for the current operation to finish before starting another.".into(),
            ))
        } else {
            Ok(())
        }
    }
    pub fn cancel(&mut self) {
        if let Some(active) = &mut self.active {
            if !active.cancelled {
                active.cancellation_pending = true;
            }
            active.cancelled = true;
            active.control.cancel();
        }
    }
    pub fn is_running(&self) -> bool {
        self.active.is_some()
    }
    /// Keep the window/event loop alive and poll until is_shutdown_complete(), reporting all events.
    pub fn begin_shutdown(&mut self) {
        self.closing = true;
        self.cancel();
    }
    pub fn is_shutdown_complete(&self) -> bool {
        self.closing && self.active.is_none()
    }
    /// At most one progress value and one terminal result are retained while the UI is idle.
    pub(crate) fn completion_control(&self) -> Option<JobControl> {
        self.completion_control.clone()
    }
    pub fn poll(&mut self) -> Vec<JobEvent> {
        self.completion_control = None;
        let mut events = Vec::new();
        let Some(active) = &mut self.active else {
            return events;
        };
        if active.cancellation_pending {
            active.cancellation_pending = false;
            events.push(JobEvent::CancellationRequested {
                token: active.token,
            });
        }
        if let Ok(mut latest) = active.latest.try_lock() {
            if let Some(progress) = latest.take() {
                events.push(JobEvent::Progress {
                    token: active.token,
                    progress,
                });
            }
        }
        if active.worker.is_finished() {
            let active = self.active.take().expect("active worker checked");
            self.completion_control = Some(active.control.clone());
            let joined = active.worker.join();
            let result = if joined.is_err() {
                Err(worker_failed())
            } else {
                active
                    .result
                    .try_recv()
                    .unwrap_or_else(|_| Err(worker_failed()))
            };
            // Keep file reconciliation/recovery outcomes intact even when cancellation arrived late.
            let result = if active.cancelled
                && result
                    .as_ref()
                    .is_ok_and(|output| output.file_outcome().is_none())
            {
                Err(AppError::Cancelled)
            } else {
                result
            };
            events.push(JobEvent::Completed {
                token: active.token,
                cancelled: active.cancelled,
                result,
            });
        }
        events
    }
}
impl Drop for JobRunner {
    fn drop(&mut self) {
        // Emergency teardown only. Normal GUI closure uses begin_shutdown/poll and stays responsive.
        if let Some(active) = self.active.take() {
            active.control.cancel();
            let _ = active.worker.join();
            if let Ok(result) = active.result.try_recv() {
                match result {
                    Ok(output) => {
                        if let Some(outcome) = output.file_outcome() {
                            eprintln!("Tessera file operation at shutdown: {outcome:?}");
                        }
                    }
                    Err(error) => eprintln!("Tessera operation at shutdown: {error}"),
                }
            }
        }
    }
}
fn worker_failed() -> AppError {
    AppError::invalid(
        "operation",
        "The background operation stopped unexpectedly. Check any file outcome before retrying.",
    )
}

impl JobOperation {
    fn project(&self) -> Option<&Project> {
        match self {
            Self::Prepare { project, .. }
            | Self::Checkpoint { project, .. }
            | Self::FrequencySearch { project, .. }
            | Self::GroupLookup { project, .. }
            | Self::Analyze { project, .. }
            | Self::ConvertColumn { project, .. }
            | Self::Save { project, .. }
            | Self::ExportData { project, .. }
            | Self::ExportReport { project, .. }
            | Self::ExportFigure { project, .. }
            | Self::ExportFigures { project, .. } => Some(project),
            _ => None,
        }
    }
    #[cfg(test)]
    fn pending_bytes(&self) -> AppResult<usize> {
        self.pending_bytes_controlled(None)
    }
    fn pending_bytes_controlled(&self, control: Option<&JobControl>) -> AppResult<usize> {
        Ok(match self {
            Self::InspectExcelPath { path }
            | Self::PreviewPath { path, .. }
            | Self::Open { path }
            | Self::Save { path, .. }
            | Self::ExportData { path, .. }
            | Self::ExportFigure { path, .. } => path.capacity(),
            Self::Analyze { request, .. } => match control {
                Some(c) => request.heap_bytes_controlled(c)?,
                None => request.heap_bytes()?,
            },
            Self::GroupLookup { request, .. } => request.heap_bytes()?,
            Self::FrequencySearch { request, .. } => request.heap_bytes()?,
            Self::ExportFigures {
                directory, keys, ..
            } => figures::pending_bytes(directory, keys)?,
            Self::ExportReport { path, runs, .. } => {
                path.capacity() + runs.capacity() * size_of::<RunId>()
            }
            // The import domain counts choices and its own preview before allocation.
            Self::CommitImport { choices, .. } | Self::CommitExcel { choices, .. } => {
                (choices.capacity() - choices.len()) * size_of::<ImportColumnChoice>()
            }
            Self::Checkpoint { store, .. }
            | Self::DiscoverRecovery { store }
            | Self::RetireCheckpoint { store } => store.owned_bytes(),
            Self::Recover { store, entry } | Self::DiscardRecovery { store, entry } => {
                add(store.owned_bytes(), entry.owned_bytes())?
            }
            Self::Prepare { .. } | Self::ConvertColumn { .. } | Self::PreviewExcel { .. } => 0,
        })
    }
}
impl JobTask {
    fn input_excel_workbook(&self) -> Option<&Arc<ExcelWorkbook>> {
        match &self.operation {
            JobOperation::PreviewExcel { workbook, .. } => Some(workbook),
            JobOperation::CommitExcel { preview, .. } => Some(preview.workbook()),
            _ => None,
        }
    }
    fn retained_excel_workbooks(&self) -> impl Iterator<Item = &Arc<ExcelWorkbook>> + Clone {
        self.retained.excel_workbooks.iter().chain(
            self.retained
                .excel_previews
                .iter()
                .map(|preview| preview.workbook()),
        )
    }
    fn projects(&self) -> impl Iterator<Item = &Project> + Clone {
        self.operation
            .project()
            .into_iter()
            .chain(self.retained.projects.iter().map(Arc::as_ref))
    }
    fn datasets(&self) -> impl Iterator<Item = &Dataset> + Clone {
        self.projects()
            .map(Project::dataset)
            .chain(self.retained.datasets.iter().map(Arc::as_ref))
    }
    fn dataset_count(&self) -> usize {
        self.retained.projects.len()
            + usize::from(self.operation.project().is_some())
            + self.retained.datasets.len()
    }
    // No references are collected until the complete live baseline and their exact capacity fit.
    fn preflight(&self) -> AppResult<usize> {
        self.preflight_controlled(None)
    }
    fn preflight_controlled(&self, control: Option<&JobControl>) -> AppResult<usize> {
        if let Some(c) = control {
            c.check_cancelled()?;
        }
        let reference_bytes = (self.dataset_count()
            + self.retained.projects.capacity()
            + self.retained.datasets.capacity()
            + self.retained.recovery_stores.capacity()
            + self.retained.previews.capacity()
            + self.retained.excel_workbooks.capacity()
            + self.retained.excel_previews.capacity()
            + self.retained.group_values.capacity()
            + self.retained.frequency_rows.capacity()
            + self.retained.frequency_metadata.capacity())
            * size_of::<usize>();
        let mut extra = add(
            self.retained.extra_bytes,
            self.operation.pending_bytes_controlled(control)?,
        )?;
        extra = add(
            extra,
            add(size_of::<Self>() + JOB_WORK_BYTES, reference_bytes)?,
        )?;
        extra = add(
            extra,
            Project::retained_owned_bytes_iter_controlled(self.projects(), control)?,
        )?;
        for (index, store) in self.retained.recovery_stores.iter().enumerate() {
            let own = match &self.operation {
                JobOperation::Checkpoint { store: input, .. }
                | JobOperation::DiscoverRecovery { store: input }
                | JobOperation::RetireCheckpoint { store: input }
                | JobOperation::DiscardRecovery { store: input, .. }
                | JobOperation::Recover { store: input, .. } => store.same_owner(input),
                _ => false,
            };
            if !own
                && !self.retained.recovery_stores[..index]
                    .iter()
                    .any(|old| store.same_owner(old))
            {
                extra = add(extra, store.owned_bytes())?;
            }
        }
        for (index, preview) in self.retained.previews.iter().enumerate() {
            let own = matches!(&self.operation, JobOperation::CommitImport { preview: input, .. } if Arc::ptr_eq(input, preview));
            if !own
                && !self.retained.previews[..index]
                    .iter()
                    .any(|prior| Arc::ptr_eq(prior, preview))
            {
                extra = add(extra, preview.owned_bytes())?;
            }
        }
        for (index, workbook) in self.retained_excel_workbooks().enumerate() {
            let own = self
                .input_excel_workbook()
                .is_some_and(|input| Arc::ptr_eq(input, workbook));
            if !own
                && !self
                    .retained_excel_workbooks()
                    .take(index)
                    .any(|prior| Arc::ptr_eq(prior, workbook))
            {
                extra = add(extra, workbook.owned_bytes())?;
            }
        }
        for (index, preview) in self.retained.excel_previews.iter().enumerate() {
            let own = matches!(&self.operation, JobOperation::CommitExcel { preview: input, .. } if Arc::ptr_eq(input, preview));
            if !own
                && !self.retained.excel_previews[..index]
                    .iter()
                    .any(|prior| Arc::ptr_eq(prior, preview))
            {
                extra = add(extra, preview.preview_only_bytes())?;
            }
        }
        for (index, result) in self.retained.group_values.iter().enumerate() {
            if !self.retained.group_values[..index]
                .iter()
                .any(|prior| Arc::ptr_eq(prior, result))
            {
                extra = add(extra, add(result.owned_bytes()?, 2 * size_of::<usize>())?)?;
            }
        }
        for (index, result) in self.retained.frequency_rows.iter().enumerate() {
            if !self.retained.frequency_rows[..index]
                .iter()
                .any(|prior| Arc::ptr_eq(prior, result))
            {
                extra = add(extra, add(result.owned_bytes()?, 2 * size_of::<usize>())?)?;
            }
        }
        for (index, cache) in self.retained.frequency_metadata.iter().enumerate() {
            if !self.retained.frequency_metadata[..index]
                .iter()
                .any(|prior| Arc::ptr_eq(prior, cache))
            {
                extra = add(
                    extra,
                    add(cache.owned_bytes()?, cache.orphaned_owner_bytes())?,
                )?;
            }
        }
        // Commit counts its input itself later; include it now while checking accounting scratch.
        let input = match &self.operation {
            JobOperation::Prepare { step, .. } => step.owned_bytes()?,
            JobOperation::CommitImport { preview, choices } => {
                add(preview.owned_bytes(), import::choice_bytes(choices)?)?
            }
            JobOperation::PreviewExcel { workbook, .. } => workbook.owned_bytes(),
            JobOperation::CommitExcel {
                preview, choices, ..
            } => add(preview.owned_bytes(), import::choice_bytes(choices)?)?,
            _ => 0,
        };
        Budget::dataset_bytes_iter(self.datasets(), add(extra, input)?)?;
        Ok(extra)
    }
}

fn add(bytes: usize, extra: usize) -> AppResult<usize> {
    bytes
        .checked_add(extra)
        .filter(|n| *n <= MAX_OWNED_BYTES)
        .ok_or(AppError::SizeLimit {
            limit: MAX_OWNED_BYTES,
        })
}
fn read_import(
    path: &PathBuf,
    control: &JobControl,
    retained: &[&Dataset],
    extra: usize,
) -> AppResult<Arc<[u8]>> {
    control.checkpoint(0, None)?;
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(AppError::invalid(
            "import",
            "Choose a regular CSV or TSV file.",
        ));
    }
    let length = usize::try_from(metadata.len())
        .ok()
        .filter(|n| *n <= MAX_INPUT_BYTES)
        .ok_or(AppError::SizeLimit {
            limit: MAX_INPUT_BYTES,
        })?;
    // Vec-to-Arc conversion temporarily owns both buffers. Reject before either allocation.
    Budget::check_datasets(retained, add(extra, length * 2 + 32 * 1024 + 64)?)?;
    let mut bytes = Vec::with_capacity(length);
    let mut buffer = [0; 32 * 1024];
    loop {
        control.checkpoint(bytes.len(), Some(length))?;
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        if bytes.len() + count > length {
            return Err(AppError::Conflict(
                "The import file changed while it was read. Preview it again.".into(),
            ));
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    if bytes.len() != length {
        return Err(AppError::Conflict(
            "The import file changed while it was read. Preview it again.".into(),
        ));
    }
    Ok(Arc::from(bytes))
}

#[cfg(test)]
mod tests;
