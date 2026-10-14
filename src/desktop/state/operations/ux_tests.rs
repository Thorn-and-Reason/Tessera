use super::*;
use eframe::egui;
#[test]
fn clean_recovery_does_not_replace_successful_save_receipt_or_status() {
    let mut state = DesktopState::new().unwrap();
    let token = state.token();
    let mut outcome = crate::files::SaveOutcome {
        publication: crate::files::Publication::Committed,
        revision: Some(token.revision),
        committed_stamp: Some(crate::files::FileStamp {
            identity: crate::files::FileIdentity { volume: 1, file: 1 },
            digest: [0; 32],
            size: 40,
        }),
        destination: PathBuf::from("/tmp/user-project.tessera"),
        recovery: None,
        warning: None,
    };
    state
        .handle_event(JobEvent::Completed {
            token,
            cancelled: false,
            result: Ok(JobOutput::Saved(outcome.clone())),
        })
        .unwrap();
    let saved_status = state.status.clone();
    outcome.destination = PathBuf::from("/tmp/recovery/checkpoint.tessera");
    state.operation_name = "Recovery checkpoint";
    state
        .handle_event(JobEvent::Completed {
            token,
            cancelled: false,
            result: Ok(JobOutput::Checkpoint(outcome)),
        })
        .unwrap();
    assert_eq!(
        state.file_outcome.as_ref().unwrap().destination,
        PathBuf::from("/tmp/user-project.tessera")
    );
    assert_eq!(state.status, saved_status);
}
#[test]
fn closing_active_import_rejects_late_success_without_releasing_worker_ownership() {
    fn draw(
        ctx: &egui::Context,
        state: &mut DesktopState,
        jobs: &mut JobRunner,
        events: Vec<egui::Event>,
    ) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(760., 520.),
                )),
                events,
                ..Default::default()
            },
            |_| super::super::super::dialogs::show(ctx, state, jobs),
        );
        output.textures_delta.clear();
    }
    for (dialog, operation) in [
        (Dialog::Import, "Import"),
        (Dialog::ExcelImport, "Excel import"),
        (Dialog::Import, "Import preview"),
    ] {
        for action in ["Escape", "Close dialog", "Cancel import"] {
            let ctx = egui::Context::default();
            let mut state = DesktopState::new().unwrap();
            state.dialog = Some(dialog);
            state.active = true;
            state.active_recipe_bytes = 456;
            state.operation_name = operation;
            let token = state.token();
            let mut jobs = JobRunner::default();
            draw(&ctx, &mut state, &mut jobs, vec![]);
            draw(&ctx, &mut state, &mut jobs, vec![]);
            if action == "Escape" {
                draw(
                    &ctx,
                    &mut state,
                    &mut jobs,
                    vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
            } else {
                let pos = state.control_rect(action).unwrap().center();
                for pressed in [true, false] {
                    draw(
                        &ctx,
                        &mut state,
                        &mut jobs,
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
            }
            assert!(state.active);
            assert_eq!(state.active_recipe_bytes, 456);
            assert!(state.cancelled, "{operation}: {action}");
            assert_eq!(
                state.dialog,
                if action == "Cancel import" {
                    Some(dialog)
                } else {
                    None
                }
            );
            let output = if operation == "Import preview" {
                JobOutput::Preview(Arc::new(
                    crate::import::preview(
                        Arc::from(&b"A\n1\n"[..]),
                        Delimiter::Comma,
                        &Default::default(),
                    )
                    .unwrap(),
                ))
            } else {
                JobOutput::Imported(
                    Dataset::new(vec![crate::data::Column::numeric(
                        ColumnId(1),
                        "late",
                        vec![Some(1.)],
                    )
                    .unwrap()])
                    .unwrap(),
                )
            };
            state
                .handle_event(JobEvent::Completed {
                    token,
                    cancelled: false,
                    result: Ok(output),
                })
                .unwrap();
            assert_eq!(state.project().dataset().column_count(), 0);
            assert_eq!(state.document_id(), token.document);
            assert!(!state.active);
            assert!(state.preview.is_none());
            assert_eq!(
                state.dialog,
                if action == "Cancel import" {
                    Some(dialog)
                } else {
                    None
                }
            );
        }
    }
}
#[test]
fn cancelled_feedback_is_not_an_error() {
    let mut state = DesktopState::new().unwrap();
    state.fail(AppError::Cancelled);
    assert!(state.error().is_none());
    assert!(state.status.contains("cancelled"));
}
#[test]
fn error_retains_actionable_tail_after_a_long_destination() {
    let mut state = DesktopState::new().unwrap();
    state.fail(AppError::invalid(
        "figure export",
        format!(
            "Destination: {}. Disk full; choose another drive.",
            "long-folder/".repeat(100)
        ),
    ));
    assert!(state
        .error()
        .unwrap()
        .contains("Disk full; choose another drive."));
}
