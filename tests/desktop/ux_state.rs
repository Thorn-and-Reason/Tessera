use super::*;
use tessera::files::Publication;
#[cfg(unix)]
use tessera::recovery::RecoveryStore;

struct Temp(std::path::PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "tessera-ux-state-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn visible_text(ctx: &egui::Context, state: &mut DesktopState, jobs: &mut JobRunner) -> String {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000., 760.))),
            ..Default::default()
        },
        |ui| render(ui, state, jobs),
    );
    output.textures_delta.clear();
    output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text.galley.text()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn save_fixture(state: &mut DesktopState, jobs: &mut JobRunner, path: &std::path::Path) {
    let project = Arc::clone(state.project());
    run(
        state,
        jobs,
        JobOperation::Save {
            project,
            path: path.to_owned(),
            expected: None,
        },
    );
}

#[test]
#[cfg(unix)]
fn durable_checkpoint_keeps_explicit_save_receipt_and_file_identity() {
    let temp = Temp::new();
    let path = temp.0.join("reviewed-project.tessera");
    let mut state = state();
    let mut jobs = JobRunner::default();
    save_fixture(&mut state, &mut jobs, &path);
    let project = Arc::clone(state.project());
    let store = RecoveryStore::open_at(&temp.0.join("private-recovery")).unwrap();
    run(
        &mut state,
        &mut jobs,
        JobOperation::Checkpoint {
            project,
            store,
            sequence: 1,
        },
    );
    assert_eq!(state.file_receipt().unwrap().destination, path);
    assert!(state.recovery_receipt().is_none());
    assert!(!state.is_dirty());
    let ctx = egui::Context::default();
    for _ in 0..2 {
        visible_text(&ctx, &mut state, &mut jobs);
    }
    click(
        &ctx,
        &mut state,
        &mut jobs,
        "File details",
        Vec2::new(1000., 760.),
    );
    let text = visible_text(&ctx, &mut state, &mut jobs);
    assert!(text.contains("Project save"), "{text}");
    assert!(text.contains("Current project file:"), "{text}");
    assert!(text.contains("reviewed-project.tessera"), "{text}");
    assert!(!text.contains("checkpoint.tessera"), "{text}");
}

#[test]
fn recovery_warning_keeps_explicit_receipt_and_surviving_copy_paths() {
    let temp = Temp::new();
    let path = temp.0.join("reviewed-project.tessera");
    let mut state = state();
    let mut jobs = JobRunner::default();
    save_fixture(&mut state, &mut jobs, &path);
    let mut outcome = state.file_receipt().unwrap().clone();
    outcome.destination = temp.0.join("checkpoint.tessera");
    outcome.warning =
        Some("Recovery cleanup needs review; retained copies remain available.".into());
    outcome.recovery = Some(tessera::files::RecoveryPaths {
        directory: temp.0.join("retained"),
        frozen_revision: Some(temp.0.join("retained/frozen")),
        displaced: None,
        prepared: None,
    });
    let token = state.token();
    assert!(state
        .handle_event(JobEvent::Completed {
            token,
            cancelled: false,
            result: Ok(JobOutput::Checkpoint(outcome)),
        })
        .is_err());
    assert_eq!(state.file_receipt().unwrap().destination, path);
    assert!(state.recovery_receipt().unwrap().recovery.is_some());
    assert_eq!(state.dialog(), Some(Dialog::FileOutcome));
    let ctx = egui::Context::default();
    visible_text(&ctx, &mut state, &mut jobs);
    let text = visible_text(&ctx, &mut state, &mut jobs);
    assert!(text.contains("Recovery update"), "{text}");
    assert!(text.contains("Recovery cleanup needs review"), "{text}");
    assert!(text.contains("Copy Saved revision"), "{text}");
}

#[test]
fn export_receipt_identifies_export_and_preserves_current_project_path() {
    let temp = Temp::new();
    let path = temp.0.join("project.tessera");
    let export = temp.0.join("data.csv");
    let mut state = state();
    let mut jobs = JobRunner::default();
    save_fixture(&mut state, &mut jobs, &path);
    let project = Arc::clone(state.project());
    run(
        &mut state,
        &mut jobs,
        JobOperation::ExportData {
            project,
            path: export.clone(),
            delimiter: tessera::import::Delimiter::Comma,
            acknowledge_formula_risk: false,
        },
    );
    assert_eq!(
        state.file_receipt().unwrap().publication,
        Publication::Committed
    );
    assert_eq!(state.file_receipt().unwrap().destination, export);
    assert!(!state.is_dirty());
    let warning = state.file_receipt().unwrap().warning.clone();
    if warning.is_some() {
        assert_eq!(state.dialog(), Some(Dialog::FileOutcome));
    } else {
        assert_eq!(state.dialog(), None);
    }
    let ctx = egui::Context::default();
    if warning.is_none() {
        for _ in 0..2 {
            visible_text(&ctx, &mut state, &mut jobs);
        }
        click(
            &ctx,
            &mut state,
            &mut jobs,
            "File details",
            Vec2::new(1000., 760.),
        );
    }
    assert_eq!(state.dialog(), Some(Dialog::FileOutcome));
    let text = visible_text(&ctx, &mut state, &mut jobs);
    assert!(text.contains("File export"), "{text}");
    assert!(text.contains("Destination:"), "{text}");
    if let Some(warning) = warning {
        assert!(text.contains(&warning), "{text}");
    }
    assert!(text.contains("Current project file:"), "{text}");
    assert!(!text.contains("File saved"), "{text}");
}
