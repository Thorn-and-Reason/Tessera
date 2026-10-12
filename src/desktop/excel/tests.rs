use super::*;
use crate::{
    jobs::{JobEvent, JobOutput},
    operation::{JobControl, JobToken},
};
use tessera_workbook::{
    metadata::{WorkbookInfo, Worksheet},
    protocol::{Cell as SourceCell, Value},
    supervisor::WorkbookSnapshot,
};
fn workbook() -> Arc<ExcelWorkbook> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/tessera-workbook/fixtures/tiny.xlsx");
    let snapshot =
        WorkbookSnapshot::prepare(&path, 0, &std::sync::atomic::AtomicBool::new(false)).unwrap();
    let metadata = WorkbookInfo {
        source_identity: snapshot.source_identity(),
        epoch_1904: false,
        sheets: vec![Worksheet {
            index: 0,
            name: "Data".into(),
            kind: 0,
            visibility: 0,
            dimensions: Some([0, 0, 1, 2]),
        }],
    };
    Arc::new(
        ExcelWorkbook::from_snapshot(snapshot, metadata, &JobControl::default(), &[], 0).unwrap(),
    )
}
fn ready_state() -> DesktopState {
    let mut state = DesktopState::new().unwrap();
    state.import_path = Some(PathBuf::from("original.xlsx"));
    let workbook = workbook();
    state.accept_excel_workbook(Arc::clone(&workbook)).unwrap();
    let selection = ExcelSelection {
        worksheet_index: 0,
        range: [0, 0, 0, 2],
        header: false,
    };
    let cells = vec![
        SourceCell {
            row: 0,
            column: 0,
            flags: 33,
            value: Value::Number(9007199254740992.),
        },
        SourceCell {
            row: 0,
            column: 1,
            flags: 3,
            value: Value::Missing,
        },
        SourceCell {
            row: 0,
            column: 2,
            flags: 16,
            value: Value::ExcelDate {
                serial: 60.,
                epoch_1904: false,
                duration: false,
            },
        },
    ];
    let preview =
        ExcelPreview::from_cells(workbook, selection, cells, &JobControl::default(), &[], 0)
            .unwrap();
    state.accept_excel_preview(Arc::new(preview)).unwrap();
    state
}
fn approve(state: &mut DesktopState) {
    state.excel.decisions = ExcelDecisions {
        acknowledge_cached_formulas: true,
        allow_missing_formula_caches: true,
        allow_fictitious_dates_as_missing: true,
    };
    let mut choice = state.choices[0].clone();
    choice.acknowledge_precision_loss = true;
    state.edit_import_choice(0, choice).unwrap();
}
#[test]
fn grouped_stored_numeric_approval_requires_an_explicit_click() {
    let mut state = DesktopState::new().unwrap();
    let workbook = workbook();
    state.accept_excel_workbook(Arc::clone(&workbook)).unwrap();
    let preview = ExcelPreview::from_cells(
        workbook,
        ExcelSelection {
            worksheet_index: 0,
            range: [0, 0, 0, 2],
            header: false,
        },
        vec![
            SourceCell {
                row: 0,
                column: 0,
                flags: 32,
                value: Value::Number(9007199254740992.0),
            },
            SourceCell {
                row: 0,
                column: 1,
                flags: 32,
                value: Value::Number(9007199254740992.0),
            },
            SourceCell {
                row: 0,
                column: 2,
                flags: 0,
                value: Value::Text("9007199254740993".into()),
            },
        ],
        &JobControl::default(),
        &[],
        0,
    )
    .unwrap();
    state.accept_excel_preview(Arc::new(preview)).unwrap();
    let mut choice = state.choices[1].clone();
    choice.kind = ColumnKind::Text;
    state.edit_import_choice(1, choice).unwrap();
    let mut choice = state.choices[2].clone();
    choice.kind = ColumnKind::Number;
    state.edit_import_choice(2, choice).unwrap();
    let ctx = egui::Context::default();
    super::super::super::configure_context(&ctx);
    let mut jobs = JobRunner::default();
    let mut frame = |state: &mut DesktopState, events| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 800.0),
                )),
                events,
                ..Default::default()
            },
            |ui| render(ui, state, &mut jobs),
        );
        output.textures_delta.clear();
    };
    frame(&mut state, vec![]);
    assert!(!state.choices[0].acknowledge_precision_loss);
    assert!(state.excel_commit_operation().is_err());
    let pos = state
        .control_rect("Accept Excel stored numeric rounding")
        .expect("one explicit acknowledgement should cover stored numeric sources")
        .center();
    for pressed in [true, false] {
        frame(
            &mut state,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    assert!(state.choices[0].acknowledge_precision_loss);
    assert!(state.choices[1].acknowledge_precision_loss);
    assert!(
        !state.choices[2].acknowledge_precision_loss,
        "stored-number consent must not approve text conversion"
    );
    assert!(state.excel_commit_operation().is_err());
    let mut choice = state.choices[2].clone();
    choice.acknowledge_precision_loss = true;
    state.edit_import_choice(2, choice).unwrap();
    assert!(state.excel_commit_operation().is_ok());
}

#[test]
fn numeric_source_still_needs_precision_approval_when_imported_as_text() {
    let mut state = ready_state();
    state.excel.decisions = ExcelDecisions {
        acknowledge_cached_formulas: true,
        allow_missing_formula_caches: true,
        allow_fictitious_dates_as_missing: true,
    };
    let mut choice = state.choices[0].clone();
    choice.kind = ColumnKind::Text;
    state.edit_import_choice(0, choice).unwrap();
    assert!(state.excel_commit_operation().is_err());
    approve(&mut state);
    let JobOperation::CommitExcel { choices, .. } = state.excel_commit_operation().unwrap() else {
        panic!()
    };
    assert_eq!(choices[0].kind, ColumnKind::Text);
}
#[test]
fn every_formula_and_date_decision_is_required_and_range_edits_need_new_preview() {
    let mut state = ready_state();
    approve(&mut state);
    assert!(state.excel_commit_operation().is_ok());
    for index in 0..3 {
        let original = state.excel.decisions;
        match index {
            0 => state.excel.decisions.acknowledge_cached_formulas = false,
            1 => state.excel.decisions.allow_missing_formula_caches = false,
            _ => state.excel.decisions.allow_fictitious_dates_as_missing = false,
        }
        assert!(state.excel_commit_operation().is_err());
        state.excel.decisions = original;
    }
    state.excel.selection.as_mut().unwrap().header = true;
    assert!(state.excel_commit_operation().is_err());
    state.excel.selection = Some(state.excel.preview.as_ref().unwrap().selection());
    assert!(state.excel_commit_operation().is_ok());
}
#[test]
fn workbook_and_preview_share_one_source_charge_and_clones_refuse_before_allocation() {
    let mut state = ready_state();
    let preview = state.excel.preview.as_ref().unwrap();
    let workbook = state.excel.workbook.as_ref().unwrap();
    let controls = 4 * std::mem::size_of::<usize>();
    assert_eq!(
        state.excel.owned_bytes(),
        workbook.owned_bytes() + preview.preview_only_bytes() + controls
    );
    approve(&mut state);
    state.active_recipe_bytes = crate::data::MAX_OWNED_BYTES;
    let (result, allocated) =
        crate::data::measure_test_allocations(|| state.excel_commit_operation());
    assert!(matches!(result, Err(AppError::SizeLimit { .. })));
    assert_eq!(allocated, 0);
}
#[test]
fn stale_failed_and_cancelled_events_keep_project_and_workbook_retryable() {
    let mut state = ready_state();
    approve(&mut state);
    let project = Arc::clone(state.project());
    let workbook = Arc::clone(state.excel.workbook.as_ref().unwrap());
    let preview = Arc::clone(state.excel.preview.as_ref().unwrap());
    state.active = true;
    state.operation_name = "Excel import";
    let token = state.token();
    let stale = JobToken {
        generation: token.generation + 1,
        ..token
    };
    state
        .handle_event(JobEvent::Completed {
            token: stale,
            cancelled: false,
            result: Err(AppError::invalid("Excel import", "stale error")),
        })
        .unwrap();
    assert!(state.active);
    assert!(state.error().is_none());
    state
        .handle_event(JobEvent::Completed {
            token,
            cancelled: true,
            result: Ok(JobOutput::Imported(
                crate::data::Dataset::new(vec![]).unwrap(),
            )),
        })
        .unwrap();
    assert!(!state.active);
    assert!(Arc::ptr_eq(&project, state.project()));
    assert!(Arc::ptr_eq(
        &workbook,
        state.excel.workbook.as_ref().unwrap()
    ));
    assert!(Arc::ptr_eq(&preview, state.excel.preview.as_ref().unwrap()));
    assert!(state.excel_commit_operation().is_ok());
    state
        .handle_event(JobEvent::Completed {
            token: state.token(),
            cancelled: false,
            result: Err(AppError::invalid("Excel import", "retryable failure")),
        })
        .unwrap_err();
    assert!(Arc::ptr_eq(&project, state.project()));
    assert!(state.excel_commit_operation().is_ok());
}
#[test]
fn successful_import_adoption_releases_source_only_after_validation() {
    let mut state = ready_state();
    approve(&mut state);
    let source = Arc::downgrade(state.excel.workbook.as_ref().unwrap());
    state.operation_name = "Excel import";
    state.import_path = Some(PathBuf::from("failed-later-selection.xlsx"));
    state
        .handle_event(JobEvent::Completed {
            token: state.token(),
            cancelled: false,
            result: Ok(JobOutput::Imported(
                crate::data::Dataset::new(vec![]).unwrap(),
            )),
        })
        .unwrap();
    assert_eq!(state.project().title(), "original");
    assert!(state.project().provenance()[0].contains("original.xlsx"));
    assert!(source.upgrade().is_none());
    assert!(state.excel.preview.is_none());
}
#[test]
fn rendered_excel_preview_exposes_numeric_text_approval_and_active_cancel() {
    let mut state = ready_state();
    let mut choice = state.choices[0].clone();
    choice.kind = ColumnKind::Text;
    state.edit_import_choice(0, choice).unwrap();
    let ctx = egui::Context::default();
    super::super::super::configure_context(&ctx);
    let mut jobs = JobRunner::default();
    let frame = |state: &mut DesktopState, jobs: &mut JobRunner| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900., 800.),
                )),
                ..Default::default()
            },
            |ui| {
                render(ui, state, jobs);
                commit_button(ui, state, jobs);
            },
        );
        output.textures_delta.clear();
    };
    frame(&mut state, &mut jobs);
    assert!(state.control_rect("Preview selected range").is_some());
    assert!(state
        .control_rect("Excel numeric rounding variable 1")
        .is_some());
    state.active = true;
    frame(&mut state, &mut jobs);
    assert!(state.control_rect("Cancel import").is_some());
}
#[test]
fn excel_request_respects_property_and_unsaved_decisions() {
    use super::super::{MetadataEditor, PendingAction, PropertyChoice, UiAction};
    use crate::data::{Column, ColumnId, ColumnMetadata, Dataset};
    let project = crate::project::Project::new(
        "Prior",
        Dataset::new(vec![
            Column::numeric(ColumnId(1), "Value", vec![Some(1.)]).unwrap()
        ])
        .unwrap(),
    )
    .unwrap();
    let mut state = DesktopState::from_project(project).unwrap();
    state.metadata_editor = Some(MetadataEditor {
        column: ColumnId(1),
        metadata: ColumnMetadata::new("Edited"),
        kind: ColumnKind::Number,
    });
    let state = state
        .apply_action(UiAction::Request(PendingAction::ImportExcel(
            PathBuf::from("new.xlsx"),
        )))
        .unwrap();
    assert_eq!(state.dialog, Some(Dialog::PropertyChanges));
    assert!(state.pending().is_none());
    let state = state
        .apply_action(UiAction::ResolveProperties(PropertyChoice::Discard))
        .unwrap();
    assert_eq!(state.dialog, Some(Dialog::Unsaved));
    assert!(matches!(
        state.pending(),
        Some(PendingAction::ImportExcel(_))
    ));
    assert_eq!(
        state
            .project()
            .dataset()
            .column(ColumnId(1))
            .unwrap()
            .name(),
        "Value"
    );
}
#[test]
fn numeric_text_conversion_exposes_approval_only_when_number_is_selected() {
    let mut state = DesktopState::new().unwrap();
    let workbook = workbook();
    state.accept_excel_workbook(Arc::clone(&workbook)).unwrap();
    let preview = ExcelPreview::from_cells(
        workbook,
        ExcelSelection {
            worksheet_index: 0,
            range: [0; 4],
            header: false,
        },
        vec![SourceCell {
            row: 0,
            column: 0,
            flags: 0,
            value: Value::Text("9007199254740993".into()),
        }],
        &JobControl::default(),
        &[],
        0,
    )
    .unwrap();
    state.accept_excel_preview(Arc::new(preview)).unwrap();
    let ctx = egui::Context::default();
    super::super::super::configure_context(&ctx);
    let mut jobs = JobRunner::default();
    let frame = |state: &mut DesktopState, jobs: &mut JobRunner| {
        state.controls.clear();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900., 800.),
                )),
                ..Default::default()
            },
            |ui| render(ui, state, jobs),
        );
        output.textures_delta.clear();
    };
    assert!(state.excel_commit_operation().is_ok());
    frame(&mut state, &mut jobs);
    assert!(state
        .control_rect("Excel numeric rounding variable 1")
        .is_none());
    let mut choice = state.choices[0].clone();
    choice.kind = ColumnKind::Number;
    state.edit_import_choice(0, choice).unwrap();
    assert!(state.excel_commit_operation().is_err());
    frame(&mut state, &mut jobs);
    assert!(state
        .control_rect("Excel numeric rounding variable 1")
        .is_some());
    let mut choice = state.choices[0].clone();
    choice.acknowledge_precision_loss = true;
    state.edit_import_choice(0, choice).unwrap();
    assert!(state.excel_commit_operation().is_ok());
    let mut choice = state.choices[0].clone();
    choice.kind = ColumnKind::Text;
    choice.acknowledge_precision_loss = false;
    state.edit_import_choice(0, choice).unwrap();
    assert!(state.excel_commit_operation().is_ok());
}
