use super::*;

fn excel_workbook() -> Arc<import::excel::ExcelWorkbook> {
    use tessera_workbook::{
        metadata::{WorkbookInfo, Worksheet},
        supervisor::WorkbookSnapshot,
    };
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("crates/tessera-workbook/fixtures/tiny.xlsx");
    let snapshot =
        WorkbookSnapshot::prepare(&path, 0, &std::sync::atomic::AtomicBool::new(false)).unwrap();
    let info = WorkbookInfo {
        source_identity: snapshot.source_identity(),
        epoch_1904: false,
        sheets: vec![Worksheet {
            index: 0,
            name: "Data".into(),
            kind: 0,
            visibility: 0,
            dimensions: Some([0, 0, 1, 0]),
        }],
    };
    Arc::new(
        import::excel::ExcelWorkbook::from_snapshot(snapshot, info, &JobControl::default(), &[], 0)
            .unwrap(),
    )
}
fn excel_preview(workbook: Arc<import::excel::ExcelWorkbook>) -> Arc<import::excel::ExcelPreview> {
    use tessera_workbook::protocol::{Cell, Value};
    Arc::new(
        import::excel::ExcelPreview::from_cells(
            workbook,
            import::excel::ExcelSelection {
                worksheet_index: 0,
                range: [0, 0, 0, 0],
                header: false,
            },
            vec![Cell {
                row: 0,
                column: 0,
                flags: 0,
                value: Value::Text("00123".into()),
            }],
            &JobControl::default(),
            &[],
            0,
        )
        .unwrap(),
    )
}
#[test]
fn excel_commit_publication_failure_cancellation_and_budget_preserve_active_project() {
    let active = project();
    let mut before = Vec::new();
    crate::project_io::encode(&active, &mut before, &JobControl::default()).unwrap();
    let preview = excel_preview(excel_workbook());
    let choices = preview.default_choices().unwrap();
    let mut task = JobTask::new(JobOperation::CommitExcel {
        preview: preview.clone(),
        choices,
        decisions: import::excel::ExcelDecisions::default(),
    });
    task.retained.projects.push(active.clone());
    let mut runner = JobRunner::default();
    runner.start(task, token()).unwrap();
    let event = complete(&mut runner);
    assert!(event.may_publish(1, RevisionId(1), 1));
    assert!(!event.may_publish(2, RevisionId(1), 1));
    assert!(
        matches!(event, JobEvent::Completed { result: Ok(JobOutput::Imported(ref dataset)), .. } if dataset.row_count() == 1)
    );
    let mut invalid = preview.default_choices().unwrap();
    invalid[0].name.clear();
    let failure = JobTask::new(JobOperation::CommitExcel {
        preview: preview.clone(),
        choices: invalid,
        decisions: import::excel::ExcelDecisions::default(),
    });
    assert!(failure.execute(&JobControl::default()).is_err());
    let control = JobControl::default();
    control.cancel();
    assert!(matches!(
        failure.execute(&control),
        Err(AppError::Cancelled)
    ));
    let mut over = JobTask::new(JobOperation::InspectExcelPath {
        path: "not-opened.xlsx".into(),
    });
    over.retained.extra_bytes = MAX_OWNED_BYTES;
    assert!(matches!(
        runner.start(over, token()),
        Err(AppError::SizeLimit { .. })
    ));
    assert!(!runner.is_running());
    let mut after = Vec::new();
    crate::project_io::encode(&active, &mut after, &JobControl::default()).unwrap();
    assert_eq!(after, before);
}
#[test]
fn excel_retained_accounting_deduplicates_shared_workbooks_and_previews() {
    let workbook = excel_workbook();
    let first = excel_preview(workbook.clone());
    let second = excel_preview(workbook.clone());
    let mut task = JobTask::new(JobOperation::Open {
        path: "unused.tessera".into(),
    });
    let empty = task.preflight().unwrap();
    task.retained.excel_workbooks = vec![workbook.clone()];
    let once = task.preflight().unwrap();
    assert!(once - empty >= workbook.owned_bytes());
    task.retained.excel_previews = vec![first.clone(), second.clone()];
    let both = task.preflight().unwrap();
    assert_eq!(
        both - once,
        first.preview_only_bytes() + second.preview_only_bytes() + 2 * size_of::<usize>()
    );
    task.retained.excel_workbooks.push(workbook);
    task.retained.excel_previews.push(first);
    assert!(task.preflight().unwrap() - both < 128);
}
#[test]
fn excel_input_is_counted_once_and_worker_locator_is_executable_sibling() {
    let workbook = excel_workbook();
    let preview = excel_preview(workbook.clone());
    let mut task = JobTask::new(JobOperation::CommitExcel {
        preview: preview.clone(),
        choices: preview.default_choices().unwrap(),
        decisions: import::excel::ExcelDecisions::default(),
    });
    let baseline = task.preflight().unwrap();
    task.retained.excel_workbooks = vec![workbook];
    task.retained.excel_previews = vec![preview];
    assert_eq!(task.preflight().unwrap() - baseline, 2 * size_of::<usize>());
    let control = JobControl::default();
    let config = super::super::excel::config(&control).unwrap();
    assert_eq!(
        config.worker.parent(),
        std::env::current_exe().unwrap().parent()
    );
    assert_eq!(
        config.worker.file_name().unwrap(),
        if cfg!(windows) {
            "tessera-workbook-worker.exe"
        } else {
            "tessera-workbook-worker"
        }
    );
    assert!(config.probe.is_none());
    control.cancel();
    assert!(config.cancel.load(std::sync::atomic::Ordering::SeqCst));
}
#[test]
fn excel_inspection_and_invalid_selection_fail_without_replacing_active_data() {
    let active = project();
    let mut runner = JobRunner::default();
    let mut missing = JobTask::new(JobOperation::InspectExcelPath {
        path: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("crates/tessera-workbook/fixtures/not-a-workbook.xlsx"),
    });
    missing.retained.projects.push(active.clone());
    runner.start(missing, token()).unwrap();
    assert!(matches!(
        complete(&mut runner),
        JobEvent::Completed { result: Err(_), .. }
    ));
    let mut invalid = JobTask::new(JobOperation::PreviewExcel {
        workbook: excel_workbook(),
        selection: import::excel::ExcelSelection {
            worksheet_index: 999,
            range: [0, 0, 0, 0],
            header: false,
        },
    });
    invalid.retained.projects.push(active.clone());
    runner.start(invalid, token()).unwrap();
    assert!(matches!(
        complete(&mut runner),
        JobEvent::Completed { result: Err(_), .. }
    ));
    assert_eq!(active.dataset().row_count(), 10_000);
    assert_eq!(
        active.dataset().cell(ColumnId(1), 0).unwrap(),
        crate::data::Cell::Number(2.0)
    );
}
