//! Local workbook workflow probe.
//!
//! Usage:
//!   cargo run --example workflow_probe -- <xlsx-path> <output-dir>
//!
//! The probe exercises the workbook worker path, imports the selected sheet,
//! runs two analyses through DesktopState, saves and reopens a project, and
//! exports an HTML report. It prints counts and artifact paths, not cell values.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use tessera::{
    analysis::{
        AnalysisRequest, CompletedAnalysis, CorrelationMissingPolicy, DescribeRequest,
        MissingPolicy, PearsonRequest, RowFilter, Statistic,
    },
    data::{ColumnId, ColumnKind, Dataset, RevisionId},
    desktop::{draft::AnalysisDraft, DesktopState, UiAction, Workspace},
    error::{AppError, AppResult},
    files::{self, Publication},
    import::excel::{ExcelDecisions, ExcelPreview, ExcelSelection, ExcelWorkbook},
    jobs::{JobEvent, JobOperation, JobOutput, JobRunner, JobTask},
    operation::{JobControl, JobToken},
    project::Project,
};

fn usage() -> AppError {
    AppError::invalid("arguments", "Use workflow_probe XLSX_PATH OUTPUT_DIR.")
}

fn token(revision: RevisionId, generation: u64) -> JobToken {
    JobToken {
        document: 1,
        generation,
        revision,
    }
}

fn run_task(
    operation: JobOperation,
    revision: RevisionId,
    generation: u64,
) -> AppResult<JobOutput> {
    let mut runner = JobRunner::default();
    runner.start(JobTask::new(operation), token(revision, generation))?;
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        for event in runner.poll() {
            if let JobEvent::Completed { result, .. } = event {
                return result;
            }
        }
        if !runner.is_running() {
            return Err(AppError::invalid(
                "workflow probe",
                "Worker stopped without a completion event.",
            ));
        }
        if Instant::now() >= deadline {
            runner.begin_shutdown();
            return Err(AppError::invalid(
                "workflow probe",
                "Worker did not finish within 120 seconds.",
            ));
        }
        thread::sleep(Duration::from_millis(2));
    }
}

fn inspect(path: &Path) -> AppResult<Arc<ExcelWorkbook>> {
    match run_task(
        JobOperation::InspectExcelPath {
            path: path.to_owned(),
        },
        RevisionId(0),
        1,
    )? {
        JobOutput::ExcelWorkbook(workbook) => Ok(workbook),
        _ => Err(AppError::invalid(
            "workflow probe",
            "Inspect did not return a workbook.",
        )),
    }
}

fn selection(workbook: &ExcelWorkbook) -> AppResult<ExcelSelection> {
    let sheet = workbook
        .metadata()
        .sheets
        .iter()
        .find(|sheet| sheet.kind == 0 && sheet.visibility == 0)
        .or_else(|| {
            workbook
                .metadata()
                .sheets
                .iter()
                .find(|sheet| sheet.kind == 0)
        })
        .ok_or_else(|| AppError::invalid("workflow probe", "Workbook has no worksheet."))?;
    Ok(ExcelSelection {
        worksheet_index: sheet.index,
        range: sheet.dimensions.unwrap_or([0, 0, 0, 0]),
        header: true,
    })
}

fn preview(
    workbook: Arc<ExcelWorkbook>,
    selection: ExcelSelection,
) -> AppResult<Arc<ExcelPreview>> {
    match run_task(
        JobOperation::PreviewExcel {
            workbook,
            selection,
        },
        RevisionId(0),
        2,
    )? {
        JobOutput::ExcelPreview(preview) => Ok(preview),
        _ => Err(AppError::invalid(
            "workflow probe",
            "Preview did not return an Excel preview.",
        )),
    }
}

fn imported_dataset(preview: Arc<ExcelPreview>) -> AppResult<Dataset> {
    let mut choices = preview.default_choices()?;
    for choice in &mut choices {
        choice.acknowledge_precision_loss = true;
    }
    let decisions = ExcelDecisions {
        acknowledge_cached_formulas: true,
        allow_missing_formula_caches: true,
        allow_fictitious_dates_as_missing: true,
    };
    match run_task(
        JobOperation::CommitExcel {
            preview,
            choices,
            decisions,
        },
        RevisionId(0),
        3,
    )? {
        JobOutput::Imported(dataset) => Ok(dataset),
        _ => Err(AppError::invalid(
            "workflow probe",
            "Commit did not return an imported dataset.",
        )),
    }
}

fn useful_number(data: &Dataset, id: ColumnId) -> AppResult<bool> {
    let column = data.column(id)?;
    let mut present = 0usize;
    for row in 0..data.row_count() {
        if !column.is_missing(row)? {
            present += 1;
            if present >= 2 {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn ranked_numeric_columns(data: &Dataset) -> AppResult<Vec<ColumnId>> {
    let mut columns = Vec::new();
    for column in data
        .columns()
        .iter()
        .filter(|column| column.kind() == ColumnKind::Number)
    {
        if useful_number(data, column.id())? {
            columns.push(column.id());
        }
    }
    columns.sort_by_key(|id| {
        let name = data
            .column(*id)
            .map(|column| column.name().to_ascii_lowercase())
            .unwrap_or_default();
        let rank = if name.contains("average") || name.contains("mean") {
            0
        } else if name.contains("score") {
            1
        } else if name.contains("scale") || name.contains("item") {
            2
        } else {
            3
        };
        (rank, id.0)
    });
    if columns.len() < 2 {
        return Err(AppError::invalid(
            "workflow probe",
            "Imported data needs at least two numeric variables with two present values.",
        ));
    }
    Ok(columns)
}

fn wait_desktop(state: &mut DesktopState, jobs: &mut JobRunner) -> AppResult<()> {
    let deadline = Instant::now() + Duration::from_secs(120);
    while jobs.is_running() {
        state.poll(jobs);
        if let Some(error) = state.error() {
            return Err(AppError::invalid("desktop worker", error));
        }
        if Instant::now() >= deadline {
            jobs.begin_shutdown();
            return Err(AppError::invalid(
                "desktop worker",
                "Desktop worker did not finish within 120 seconds.",
            ));
        }
        thread::sleep(Duration::from_millis(2));
    }
    state.poll(jobs);
    if let Some(error) = state.error() {
        return Err(AppError::invalid("desktop worker", error));
    }
    Ok(())
}

fn analyze(state: &mut DesktopState, request: AnalysisRequest) -> AppResult<()> {
    let mut jobs = JobRunner::default();
    let before = state.project().runs().len();
    let expected = match &request {
        AnalysisRequest::Describe(_) => "describe",
        AnalysisRequest::Pearson(_) => "pearson",
        _ => "analysis",
    };
    *state = state.apply_action(UiAction::Navigate(Workspace::Analyses))?;
    *state = state.apply_action(UiAction::SetDraft(AnalysisDraft::from_request(request)))?;
    let request = state.checked_draft_request()?;
    state.start_operation(
        &mut jobs,
        JobOperation::Analyze {
            project: Arc::clone(state.project()),
            request,
        },
    )?;
    wait_desktop(state, &mut jobs)?;
    assert_eq!(state.project().runs().len(), before + 1);
    assert_eq!(state.workspace(), Workspace::Results);
    let run = state
        .selected_run()
        .ok_or_else(|| AppError::invalid("workflow probe", "No selected result after analysis."))?;
    match (&run.analysis, expected) {
        (CompletedAnalysis::Describe(result), "describe") => {
            assert!(!result.rows.is_empty());
        }
        (CompletedAnalysis::Pearson(result), "pearson") => {
            assert!(!result.rows.is_empty());
        }
        _ => {
            return Err(AppError::invalid(
                "workflow probe",
                "Completed analysis did not match the submitted request.",
            ));
        }
    }
    Ok(())
}

fn save_project(state: &DesktopState, path: &Path) -> AppResult<()> {
    let output = run_task(
        JobOperation::Save {
            project: Arc::clone(state.project()),
            path: path.to_owned(),
            expected: None,
        },
        state.project().dataset().revision(),
        10,
    )?;
    let JobOutput::Saved(outcome) = output else {
        return Err(AppError::invalid(
            "workflow probe",
            "Save did not return a save outcome.",
        ));
    };
    if !matches!(outcome.publication, Publication::Committed)
        || outcome.committed_stamp.is_none()
        || outcome.warning.is_some()
        || outcome.recovery.is_some()
    {
        return Err(AppError::invalid(
            "workflow probe",
            "Project save was not cleanly confirmed.",
        ));
    }
    Ok(())
}

fn export_report(state: &DesktopState, path: &Path) -> AppResult<()> {
    let runs = state.project().runs().iter().map(|run| run.id).collect();
    let output = run_task(
        JobOperation::ExportReport {
            project: Arc::clone(state.project()),
            path: path.to_owned(),
            runs,
        },
        state.project().dataset().revision(),
        11,
    )?;
    let JobOutput::Exported(outcome) = output else {
        return Err(AppError::invalid(
            "workflow probe",
            "Export did not return a file outcome.",
        ));
    };
    if !matches!(outcome.publication, Publication::Committed)
        || outcome.committed_stamp.is_none()
        || outcome.warning.is_some()
        || outcome.recovery.is_some()
    {
        return Err(AppError::invalid(
            "workflow probe",
            "Report export was not cleanly confirmed.",
        ));
    }
    Ok(())
}

fn run() -> AppResult<()> {
    let mut args = std::env::args_os().skip(1);
    let workbook_path = PathBuf::from(args.next().ok_or_else(usage)?);
    let output_dir = PathBuf::from(args.next().ok_or_else(usage)?);
    if args.next().is_some() {
        return Err(usage());
    }
    fs::create_dir_all(&output_dir)?;

    let workbook = inspect(&workbook_path)?;
    let selection = selection(&workbook)?;
    let preview = preview(workbook, selection)?;
    let preview_rows = preview.row_count();
    let preview_columns = preview.columns().len();
    let dataset = imported_dataset(preview)?;
    assert_eq!(dataset.row_count(), preview_rows);
    assert_eq!(dataset.column_count(), preview_columns);

    let numeric = ranked_numeric_columns(&dataset)?;
    let describe_columns = numeric.iter().take(2).copied().collect::<Vec<_>>();
    let pearson_columns = describe_columns.clone();
    let mut state = DesktopState::from_project(Project::new("Workflow probe", dataset)?)?;

    analyze(
        &mut state,
        AnalysisRequest::Describe(DescribeRequest {
            columns: describe_columns,
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
    )?;
    analyze(
        &mut state,
        AnalysisRequest::Pearson(PearsonRequest {
            columns: pearson_columns,
            missing_policy: CorrelationMissingPolicy::PairwiseComplete,
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
    )?;

    let project_path = output_dir.join("workflow_probe.tessera");
    save_project(&mut state, &project_path)?;
    let (loaded, _) = files::read_project(&project_path, &JobControl::default())?;
    assert_eq!(
        loaded.dataset().row_count(),
        state.project().dataset().row_count()
    );
    assert_eq!(
        loaded.dataset().column_count(),
        state.project().dataset().column_count()
    );
    assert_eq!(loaded.runs().len(), state.project().runs().len());

    let report_path = output_dir.join("workflow_probe_report.html");
    export_report(&mut state, &report_path)?;
    let report = fs::read_to_string(&report_path)?;
    assert!(report.contains("Descriptive"));
    assert!(report.contains("Pearson"));

    println!(
        "workflow_probe_ok rows={} variables={} numeric_candidates={} runs={} project={} report={}",
        state.project().dataset().row_count(),
        state.project().dataset().column_count(),
        numeric.len(),
        state.project().runs().len(),
        project_path.display(),
        report_path.display()
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
