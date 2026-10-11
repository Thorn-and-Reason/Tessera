use std::{
    io::Write,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};
use tessera::{
    data::{Cell, ColumnId, ColumnKind, Dataset, MAX_OWNED_BYTES},
    error::AppError,
    import::{
        excel::{self, ExcelDecisions, ExcelPreview, ExcelSelection, ExcelWorkbook},
        ImportWarning,
    },
    operation::JobControl,
};
use tessera_workbook::{
    metadata::{WorkbookInfo, Worksheet},
    protocol::{Cell as WorkbookCell, Value},
    supervisor::WorkbookSnapshot,
};

struct TemporarySource(PathBuf);
impl TemporarySource {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let nonce = NEXT
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    value.checked_add(1)
                })
                .expect("test source nonce exhausted");
            let path = std::env::temp_dir().join(format!(
                "tessera-excel-source-{}-{nonce}.xlsx",
                std::process::id()
            ));
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(mut file) => {
                    let source = Self(path);
                    let written = file.write_all(b"original immutable workbook bytes");
                    drop(file);
                    written.unwrap();
                    return source;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("could not create test source: {error}"),
            }
        }
    }
}
impl Drop for TemporarySource {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn workbook() -> Arc<ExcelWorkbook> {
    workbook_epoch(false)
}
fn workbook_epoch(epoch_1904: bool) -> Arc<ExcelWorkbook> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("crates/tessera-workbook/fixtures/tiny.xlsx");
    let snapshot = WorkbookSnapshot::prepare(&path, 0, &AtomicBool::new(false)).unwrap();
    let info = WorkbookInfo {
        source_identity: snapshot.source_identity(),
        epoch_1904,
        sheets: vec![Worksheet {
            index: 0,
            name: "Data".into(),
            kind: 0,
            visibility: 0,
            dimensions: Some([0, 0, 20, 5]),
        }],
    };
    Arc::new(ExcelWorkbook::from_snapshot(snapshot, info, &JobControl::default(), &[], 0).unwrap())
}
fn cell(row: u32, column: u32, flags: u8, value: Value) -> WorkbookCell {
    WorkbookCell {
        row,
        column,
        flags,
        value,
    }
}
fn text(row: u32, column: u32, value: &str) -> WorkbookCell {
    cell(row, column, 0, Value::Text(value.into()))
}
fn preview(range: [u32; 4], header: bool, cells: Vec<WorkbookCell>) -> ExcelPreview {
    ExcelPreview::from_cells(
        workbook(),
        ExcelSelection {
            worksheet_index: 0,
            range,
            header,
        },
        cells,
        &JobControl::default(),
        &[],
        0,
    )
    .unwrap()
}
fn commit(p: &ExcelPreview, decisions: ExcelDecisions) -> Dataset {
    let mut choices = p.default_choices().unwrap();
    for choice in &mut choices {
        choice.acknowledge_precision_loss = true;
    }
    excel::commit_preview_with_budget(p, &choices, decisions, &JobControl::default(), &[], 0)
        .unwrap()
}
#[test]
fn missing_empty_text_and_text_identifiers_remain_distinct() {
    let p = preview(
        [0, 0, 2, 2],
        true,
        vec![
            text(0, 0, "ID"),
            text(0, 1, "Text"),
            text(0, 2, "N"),
            text(1, 0, "00123"),
            text(1, 1, ""),
            cell(1, 2, 32, Value::Number(1.0)),
            text(2, 0, "00456"),
        ],
    );
    let data = commit(&p, ExcelDecisions::default());
    assert_eq!(data.row_count(), 2);
    assert_eq!(
        data.cell(ColumnId(1), 0).unwrap(),
        Cell::Text("00123".into())
    );
    assert_eq!(
        data.cell(ColumnId(2), 0).unwrap(),
        Cell::Text(String::new())
    );
    assert_eq!(data.cell(ColumnId(2), 1).unwrap(), Cell::Missing);
    assert_eq!(data.cell(ColumnId(3), 1).unwrap(), Cell::Missing);
}
#[test]
fn optional_header_and_offset_rectangle_preserve_every_selected_row() {
    let p = preview(
        [3, 2, 4, 2],
        false,
        vec![text(3, 2, "001"), text(4, 2, "002")],
    );
    assert_eq!(p.columns()[0].header, "Variable 1");
    assert_eq!(p.row_count(), 2);
    assert_eq!(
        commit(&p, ExcelDecisions::default())
            .cell(ColumnId(1), 0)
            .unwrap(),
        Cell::Text("001".into())
    );
    let p = preview(
        [3, 2, 4, 2],
        true,
        vec![text(3, 2, "ID"), text(4, 2, "001")],
    );
    assert_eq!(p.columns()[0].header, "ID");
    assert_eq!(p.row_count(), 1);
}
#[test]
fn numeric_source_requires_acknowledgement_even_when_selected_as_text() {
    let p = preview(
        [0, 0, 2, 0],
        true,
        vec![
            text(0, 0, "N"),
            cell(1, 0, 32, Value::Number(9007199254740992.0)),
            cell(2, 0, 32, Value::Number(-0.0)),
        ],
    );
    assert!(p.columns()[0]
        .warnings
        .contains(&ImportWarning::PrecisionRisk));
    let mut choices = p.default_choices().unwrap();
    for kind in [ColumnKind::Number, ColumnKind::Text] {
        choices[0].kind = kind;
        choices[0].acknowledge_precision_loss = false;
        assert!(excel::commit_preview_with_budget(
            &p,
            &choices,
            ExcelDecisions::default(),
            &JobControl::default(),
            &[],
            0
        )
        .is_err());
        choices[0].acknowledge_precision_loss = true;
        assert!(excel::commit_preview_with_budget(
            &p,
            &choices,
            ExcelDecisions::default(),
            &JobControl::default(),
            &[],
            0
        )
        .is_ok());
    }
    choices[0].kind = ColumnKind::Number;
    let data = excel::commit_preview_with_budget(
        &p,
        &choices,
        ExcelDecisions::default(),
        &JobControl::default(),
        &[],
        0,
    )
    .unwrap();
    let Cell::Number(value) = data.cell(ColumnId(1), 1).unwrap() else {
        panic!()
    };
    assert_eq!(value.to_bits(), (-0.0_f64).to_bits());
}
#[test]
fn mixed_boolean_and_numeric_looking_text_default_to_text() {
    let p = preview(
        [0, 0, 2, 2],
        true,
        vec![
            text(0, 0, "Mixed"),
            text(0, 1, "Boolean"),
            text(0, 2, "ID"),
            cell(1, 0, 32, Value::Number(2.0)),
            text(1, 1, "true"),
            text(1, 2, "00123"),
            text(2, 0, "other"),
            text(2, 1, "false"),
        ],
    );
    assert!(p
        .columns()
        .iter()
        .all(|column| column.inferred_kind == ColumnKind::Text));
    assert_eq!(
        commit(&p, ExcelDecisions::default())
            .cell(ColumnId(2), 1)
            .unwrap(),
        Cell::Text("false".into())
    );
}
#[test]
fn formula_cache_decisions_are_explicit_and_independent() {
    let p = preview(
        [0, 0, 2, 0],
        true,
        vec![
            text(0, 0, "Formula"),
            cell(1, 0, 1, Value::Text("cached".into())),
            cell(2, 0, 3, Value::Missing),
        ],
    );
    assert_eq!(p.warnings().cached_formula_cells, 1);
    assert_eq!(p.warnings().missing_formula_cache_cells, 1);
    let choices = p.default_choices().unwrap();
    for decisions in [
        ExcelDecisions::default(),
        ExcelDecisions {
            acknowledge_cached_formulas: true,
            ..Default::default()
        },
        ExcelDecisions {
            allow_missing_formula_caches: true,
            ..Default::default()
        },
    ] {
        assert!(excel::commit_preview_with_budget(
            &p,
            &choices,
            decisions,
            &JobControl::default(),
            &[],
            0
        )
        .is_err());
    }
    let data = commit(
        &p,
        ExcelDecisions {
            acknowledge_cached_formulas: true,
            allow_missing_formula_caches: true,
            ..Default::default()
        },
    );
    assert_eq!(
        data.cell(ColumnId(1), 0).unwrap(),
        Cell::Text("cached".into())
    );
    assert_eq!(data.cell(ColumnId(1), 1).unwrap(), Cell::Missing);
}
#[test]
fn dates_use_iso_text_and_fictitious_day_requires_missing_decision() {
    let p = preview(
        [0, 0, 2, 0],
        true,
        vec![
            text(0, 0, "Date"),
            cell(
                1,
                0,
                0,
                Value::ExcelDate {
                    serial: 61.0,
                    epoch_1904: false,
                    duration: false,
                },
            ),
            cell(
                2,
                0,
                16,
                Value::ExcelDate {
                    serial: 60.0,
                    epoch_1904: false,
                    duration: false,
                },
            ),
        ],
    );
    assert_eq!(p.columns()[0].inferred_kind, ColumnKind::Text);
    assert_eq!(p.warnings().fictitious_date_cells, 1);
    assert!(excel::commit_preview_with_budget(
        &p,
        &p.default_choices().unwrap(),
        ExcelDecisions::default(),
        &JobControl::default(),
        &[],
        0
    )
    .is_err());
    let data = commit(
        &p,
        ExcelDecisions {
            allow_fictitious_dates_as_missing: true,
            ..Default::default()
        },
    );
    assert_eq!(
        data.cell(ColumnId(1), 0).unwrap(),
        Cell::Text("1900-03-01T00:00:00.000".into())
    );
    assert_eq!(data.cell(ColumnId(1), 1).unwrap(), Cell::Missing);
}
#[test]
fn malformed_range_cells_flags_and_sheet_are_rejected() {
    for (selection, cells) in [
        (
            ExcelSelection {
                worksheet_index: 1,
                range: [0, 0, 1, 0],
                header: false,
            },
            vec![],
        ),
        (
            ExcelSelection {
                worksheet_index: 0,
                range: [0, 0, 1_048_576, 0],
                header: false,
            },
            vec![],
        ),
        (
            ExcelSelection {
                worksheet_index: 0,
                range: [0, 0, 999_999, 2],
                header: false,
            },
            vec![],
        ),
        (
            ExcelSelection {
                worksheet_index: 0,
                range: [0, 0, 1, 0],
                header: false,
            },
            vec![cell(1, 0, 0, Value::Number(1.0))],
        ),
        (
            ExcelSelection {
                worksheet_index: 0,
                range: [0, 0, 1, 0],
                header: false,
            },
            vec![text(2, 0, "outside")],
        ),
        (
            ExcelSelection {
                worksheet_index: 0,
                range: [0, 0, 1, 0],
                header: false,
            },
            vec![text(1, 0, "first"), text(0, 0, "second")],
        ),
    ] {
        assert!(ExcelPreview::from_cells(
            workbook(),
            selection,
            cells,
            &JobControl::default(),
            &[],
            0
        )
        .is_err());
    }
}
#[test]
fn cancellation_and_retained_budget_fail_without_consuming_preview() {
    let p = preview([0, 0, 0, 0], false, vec![text(0, 0, "001")]);
    let choices = p.default_choices().unwrap();
    let control = JobControl::default();
    control.cancel();
    assert!(matches!(
        excel::commit_preview_with_budget(
            &p,
            &choices,
            ExcelDecisions::default(),
            &control,
            &[],
            MAX_OWNED_BYTES
        ),
        Err(AppError::Cancelled)
    ));
    assert!(matches!(
        excel::commit_preview_with_budget(
            &p,
            &choices,
            ExcelDecisions::default(),
            &JobControl::default(),
            &[],
            MAX_OWNED_BYTES
        ),
        Err(AppError::SizeLimit { .. })
    ));
    assert_eq!(commit(&p, ExcelDecisions::default()).row_count(), 1);
}
#[test]
fn worker_quota_overflow_fails_before_process_launch() {
    use std::sync::atomic::AtomicBool;
    use tessera_workbook::supervisor::Config;
    let config = Config {
        worker: "/nonexistent-worker".into(),
        source: Default::default(),
        range: [0; 4],
        worksheet_index: 0,
        quota: usize::MAX,
        retained_bytes: 0,
        timeout: std::time::Duration::from_secs(1),
        cancel: Arc::new(AtomicBool::new(false)),
        probe: None,
    };
    assert!(matches!(
        excel::preview_with_budget(
            workbook(),
            ExcelSelection {
                worksheet_index: 0,
                range: [0; 4],
                header: false
            },
            &config,
            &JobControl::default(),
            &[],
            0
        ),
        Err(AppError::SizeLimit { .. })
    ));
}
#[test]
fn preview_retains_the_same_workbook_owner_after_external_source_replacement() {
    let source_owner = TemporarySource::new();
    let source = &source_owner.0;
    let snapshot = WorkbookSnapshot::prepare(&source, 0, &AtomicBool::new(false)).unwrap();
    let identity = snapshot.source_identity();
    let info = WorkbookInfo {
        source_identity: identity,
        epoch_1904: false,
        sheets: vec![Worksheet {
            index: 0,
            name: "Data".into(),
            kind: 0,
            visibility: 0,
            dimensions: None,
        }],
    };
    let workbook = Arc::new(
        ExcelWorkbook::from_snapshot(snapshot, info, &JobControl::default(), &[], 0).unwrap(),
    );
    let p = ExcelPreview::from_cells(
        Arc::clone(&workbook),
        ExcelSelection {
            worksheet_index: 0,
            range: [0; 4],
            header: false,
        },
        vec![text(0, 0, "00123")],
        &JobControl::default(),
        &[],
        0,
    )
    .unwrap();
    assert!(Arc::ptr_eq(p.workbook(), &workbook));
    assert_eq!(
        p.owned_bytes(),
        workbook.owned_bytes() + p.preview_only_bytes()
    );
    std::fs::write(&source, b"replacement source").unwrap();
    std::fs::remove_file(source).unwrap();
    drop(workbook);
    assert_eq!(p.workbook().source_identity(), identity);
    assert_eq!(
        commit(&p, ExcelDecisions::default())
            .cell(ColumnId(1), 0)
            .unwrap(),
        Cell::Text("00123".into())
    );
}
#[test]
fn supplied_metadata_cannot_bypass_worksheet_name_validation() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("crates/tessera-workbook/fixtures/tiny.xlsx");
    for names in [["Data", "Data"], ["Data", "bad\nname"]] {
        let snapshot = WorkbookSnapshot::prepare(&path, 0, &AtomicBool::new(false)).unwrap();
        let info = WorkbookInfo {
            source_identity: snapshot.source_identity(),
            epoch_1904: false,
            sheets: names
                .iter()
                .enumerate()
                .map(|(index, name)| Worksheet {
                    index: index as u32,
                    name: (*name).into(),
                    kind: 0,
                    visibility: 0,
                    dimensions: None,
                })
                .collect(),
        };
        assert!(
            ExcelWorkbook::from_snapshot(snapshot, info, &JobControl::default(), &[], 0).is_err()
        );
    }
}
#[test]
fn real_1904_dates_and_durations_remain_iso_text() {
    let p = ExcelPreview::from_cells(
        workbook_epoch(true),
        ExcelSelection {
            worksheet_index: 0,
            range: [0, 0, 1, 0],
            header: false,
        },
        vec![
            cell(
                0,
                0,
                8,
                Value::ExcelDate {
                    serial: 60.0,
                    epoch_1904: true,
                    duration: false,
                },
            ),
            cell(
                1,
                0,
                8,
                Value::ExcelDate {
                    serial: 1.5,
                    epoch_1904: true,
                    duration: true,
                },
            ),
        ],
        &JobControl::default(),
        &[],
        0,
    )
    .unwrap();
    let data = commit(&p, ExcelDecisions::default());
    assert_eq!(
        data.cell(ColumnId(1), 0).unwrap(),
        Cell::Text("1904-03-01T00:00:00.000".into())
    );
    assert_eq!(
        data.cell(ColumnId(1), 1).unwrap(),
        Cell::Text("PT129600.000S".into())
    );
}
#[test]
fn unsafe_numeric_text_stays_exact_by_default_and_can_be_converted_with_approval() {
    for token in [
        "9007199254740993",
        "9007199254740994",
        "-9007199254740994",
        "9.007199254740993e15",
        "1e23",
    ] {
        let p = preview([0; 4], false, vec![text(0, 0, token)]);
        assert!(
            p.columns()[0]
                .warnings
                .contains(&ImportWarning::PrecisionRisk),
            "numeric conversion warning missing for {token}"
        );
        let mut choices = p.default_choices().unwrap();
        assert_eq!(choices[0].kind, ColumnKind::Text);
        let data = excel::commit_preview_with_budget(
            &p,
            &choices,
            ExcelDecisions::default(),
            &JobControl::default(),
            &[],
            0,
        )
        .unwrap();
        assert_eq!(data.cell(ColumnId(1), 0).unwrap(), Cell::Text(token.into()));
        choices[0].kind = ColumnKind::Number;
        assert!(excel::commit_preview_with_budget(
            &p,
            &choices,
            ExcelDecisions::default(),
            &JobControl::default(),
            &[],
            0
        )
        .is_err());
        choices[0].acknowledge_precision_loss = true;
        let data = excel::commit_preview_with_budget(
            &p,
            &choices,
            ExcelDecisions::default(),
            &JobControl::default(),
            &[],
            0,
        )
        .unwrap();
        assert_eq!(
            data.cell(ColumnId(1), 0).unwrap(),
            Cell::Number(token.parse::<f64>().unwrap())
        );
    }
}
#[test]
fn header_precision_rules_preserve_numeric_source_policy_without_classifying_text_headers_as_data()
{
    let text_header = preview(
        [0, 0, 1, 0],
        true,
        vec![text(0, 0, "9007199254740993"), text(1, 0, "00123")],
    );
    let choices = text_header.default_choices().unwrap();
    assert!(!text_header.columns()[0]
        .warnings
        .contains(&ImportWarning::PrecisionRisk));
    assert!(excel::commit_preview_with_budget(
        &text_header,
        &choices,
        ExcelDecisions::default(),
        &JobControl::default(),
        &[],
        0
    )
    .is_ok());
    let numeric_header = preview(
        [0, 0, 1, 0],
        true,
        vec![
            cell(0, 0, 32, Value::Number(9007199254740992.0)),
            text(1, 0, "00123"),
        ],
    );
    let mut choices = numeric_header.default_choices().unwrap();
    assert_eq!(choices[0].kind, ColumnKind::Text);
    assert!(excel::commit_preview_with_budget(
        &numeric_header,
        &choices,
        ExcelDecisions::default(),
        &JobControl::default(),
        &[],
        0
    )
    .is_err());
    choices[0].acknowledge_precision_loss = true;
    let data = excel::commit_preview_with_budget(
        &numeric_header,
        &choices,
        ExcelDecisions::default(),
        &JobControl::default(),
        &[],
        0,
    )
    .unwrap();
    assert_eq!(
        data.cell(ColumnId(1), 0).unwrap(),
        Cell::Text("00123".into())
    );
}
