use super::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, Ordering};
static WATCH_RENDER: AtomicBool = AtomicBool::new(false);
struct RenderAllocator;
#[global_allocator]
static ALLOCATOR: RenderAllocator = RenderAllocator;
fn check_allocation(bytes: usize) {
    if WATCH_RENDER.load(Ordering::Relaxed) && bytes > tessera::desktop::UI_WORK_BYTES {
        WATCH_RENDER.store(false, Ordering::Relaxed);
        eprintln!(
            "oversized {bytes} allocation: {}",
            std::backtrace::Backtrace::force_capture()
        );
        std::process::exit(77);
    }
}
unsafe impl GlobalAlloc for RenderAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        check_allocation(layout.size());
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        check_allocation(size);
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[test]
fn pending_save_finishes_before_new_and_conflict_remains_recoverable() {
    let dir = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("tessera-pending-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("project.tessera");
    let _ = std::fs::remove_file(&path);
    let mut state = DesktopState::from_project(
        Project::new(
            "Pending",
            Dataset::new(vec![Column::numeric(
                ColumnId(1),
                "Score",
                vec![Some(17.0); 300000],
            )
            .unwrap()])
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let mut jobs = JobRunner::default();
    let project = Arc::clone(state.project());
    run(
        &mut state,
        &mut jobs,
        JobOperation::Save {
            project,
            path: path.clone(),
            expected: None,
        },
    );
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    assert!(access_labels(&ctx, &mut state, &mut jobs)
        .iter()
        .any(|label| label == "Saved project.tessera"));
    let other_path = dir.join("other.tessera");
    let other = Project::new("Other", Dataset::new(vec![]).unwrap()).unwrap();
    let mut bytes = Vec::new();
    tessera::project_io::encode(&other, &mut bytes, &JobControl::default()).unwrap();
    std::fs::write(&other_path, bytes).unwrap();
    let mut adopted = state.clone();
    let old_document = adopted.token().document;
    run(
        &mut adopted,
        &mut jobs,
        JobOperation::Open { path: other_path },
    );
    assert_eq!(adopted.project().title(), "Other");
    assert_ne!(adopted.token().document, old_document);
    assert!(!access_labels(&ctx, &mut adopted, &mut jobs)
        .iter()
        .any(|label| label == "Saved project.tessera"));
    state = state
        .apply_action(UiAction::EditCell {
            column: ColumnId(1),
            row: 0,
            value: Cell::Number(88.0),
        })
        .unwrap()
        .apply_action(UiAction::Request(PendingAction::New))
        .unwrap()
        .apply_action(UiAction::ResolveUnsaved(UnsavedChoice::Save))
        .unwrap();
    let size = Vec2::new(760.0, 520.0);
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    let deadline = Instant::now() + Duration::from_secs(20);
    while jobs.is_running() {
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    assert!(state.error().is_none(), "{:?}", state.error());
    assert_eq!(state.project().dataset().row_count(), 0);
    run(
        &mut state,
        &mut jobs,
        JobOperation::Open { path: path.clone() },
    );
    assert_eq!(
        state.project().dataset().cell(ColumnId(1), 0).unwrap(),
        Cell::Number(88.0)
    );
    state = state
        .apply_action(UiAction::EditCell {
            column: ColumnId(1),
            row: 0,
            value: Cell::Number(99.0),
        })
        .unwrap();
    std::fs::write(&path, b"external version").unwrap();
    state = state
        .apply_action(UiAction::Request(PendingAction::New))
        .unwrap()
        .apply_action(UiAction::ResolveUnsaved(UnsavedChoice::Save))
        .unwrap();
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    while jobs.is_running() {
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    assert!(state.is_dirty());
    assert!(state.pending().is_some());
    assert_eq!(state.project().dataset().row_count(), 300000);
    assert_eq!(std::fs::read(&path).unwrap(), b"external version");
    state = state
        .apply_action(UiAction::ResolveUnsaved(UnsavedChoice::Cancel))
        .unwrap();
    assert!(state.pending().is_none());
    std::fs::remove_dir_all(dir).unwrap();
}
pub(super) fn access_labels(
    ctx: &egui::Context,
    state: &mut DesktopState,
    jobs: &mut JobRunner,
) -> Vec<String> {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(760.0, 520.0))),
            ..Default::default()
        },
        |ui| render(ui, state, jobs),
    );
    output.textures_delta.clear();
    output
        .platform_output
        .accesskit_update
        .into_iter()
        .flat_map(|update| update.nodes)
        .flat_map(|(_, node)| {
            [
                node.label().map(str::to_owned),
                node.value().map(str::to_owned),
            ]
            .into_iter()
            .flatten()
        })
        .collect()
}
#[test]
fn cell_accessibility_values_and_click_enter() {
    let mut state = state();
    let mut jobs = JobRunner::default();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let labels = access_labels(&ctx, &mut state, &mut jobs);
    for expected in [
        "Sgôr ŵ, row 1: 17",
        "Enw, row 1: Empty text",
        "Enw, row 2: Missing",
        "Enw, row 3: Carys",
    ] {
        assert!(
            labels.iter().any(|s| s == expected),
            "missing AX label {expected}; got {labels:?}"
        );
    }
    let size = Vec2::new(760.0, 520.0);
    click(&ctx, &mut state, &mut jobs, "Cell 1:0", size);
    frame(
        &ctx,
        &mut state,
        &mut jobs,
        size,
        vec![Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    assert!(state.control_rect("Cell editor").is_some());
    frame(
        &ctx,
        &mut state,
        &mut jobs,
        size,
        vec![Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    state.set_cell_text("19.125".into()).unwrap();
    frame(
        &ctx,
        &mut state,
        &mut jobs,
        size,
        vec![Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert_eq!(
        state.project().dataset().cell(ColumnId(1), 0).unwrap(),
        Cell::Number(19.125)
    );
}
#[test]
fn near_ceiling_render_bounds_imported_presentation_allocations() {
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "review::near_ceiling_render_child",
            "--ignored",
            "--nocapture",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "allocation child {:?}: {}{}",
        result.status,
        std::str::from_utf8(&result.stdout).unwrap(),
        std::str::from_utf8(&result.stderr).unwrap()
    );
}
#[test]
#[ignore = "Runs in an isolated child with an allocation sentinel"]
fn near_ceiling_render_child() {
    use tessera::data::{Budget, ColumnMetadata, MAX_OWNED_BYTES};
    let ctx = egui::Context::default();
    let mut jobs = JobRunner::default();
    let mut state = DesktopState::new().unwrap();
    frame(&ctx, &mut state, &mut jobs, Vec2::new(760.0, 520.0), vec![]);
    let near_ceiling = |headroom| {
        let title = "a".repeat(5 * 1024 * 1024);
        let name = "b".repeat(5 * 1024 * 1024);
        let column = Column::numeric(ColumnId(1), &name, vec![Some(17.0)]).unwrap();
        let bare = Project::new(&title, Dataset::new(vec![column.clone()]).unwrap()).unwrap();
        let baseline =
            Budget::dataset_bytes(&[bare.dataset()], bare.owned_bytes().unwrap()).unwrap();
        let capacity = MAX_OWNED_BYTES - baseline - tessera::desktop::UI_WORK_BYTES - headroom;
        drop(bare);
        let mut description = String::with_capacity(capacity);
        description.push('x');
        let project = Project::new(
            &title,
            Dataset::new(vec![column
                .with_metadata(ColumnMetadata {
                    description,
                    ..column.metadata().clone()
                })
                .unwrap()])
            .unwrap(),
        )
        .unwrap();
        drop(title);
        drop(name);
        drop(column);
        DesktopState::from_project(project).unwrap()
    };
    state = near_ceiling(512);
    let owner = Arc::clone(state.project());
    WATCH_RENDER.store(true, Ordering::Relaxed);
    // The transient work reserve does not cover newly retained controls or state copies.
    frame(&ctx, &mut state, &mut jobs, Vec2::new(760.0, 520.0), vec![]);
    WATCH_RENDER.store(true, Ordering::Relaxed);
    let exhausted = state.apply_action(UiAction::Navigate(Workspace::Variables));
    WATCH_RENDER.store(false, Ordering::Relaxed);
    assert!(matches!(
        exhausted,
        Err(tessera::error::AppError::SizeLimit { .. })
    ));
    assert!(Arc::ptr_eq(&owner, state.project()));
    assert_eq!(state.workspace(), Workspace::Data);
    drop(owner);
    drop(state);
    // Retained controls and overlapping navigation copies receive separate headroom.
    state = near_ceiling(256 * 1024);
    for view in [
        Workspace::Data,
        Workspace::Prepare,
        Workspace::Variables,
        Workspace::Analyses,
        Workspace::Results,
    ] {
        state = state.apply_action(UiAction::Navigate(view)).unwrap();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(760.0, 520.0))),
                ..Default::default()
            },
            |ui| {
                WATCH_RENDER.store(true, Ordering::Relaxed);
                render(ui, &mut state, &mut jobs);
                WATCH_RENDER.store(false, Ordering::Relaxed);
            },
        );
        output.textures_delta.clear();
    }

    drop(state);
    let mut metadata = ColumnMetadata::new(&"n".repeat(5 * 1024 * 1024));
    metadata.description = "d".repeat(5 * 1024 * 1024);
    metadata.value_labels = vec![tessera::data::ValueLabel {
        value: Cell::Number(17.0),
        label: "l".repeat(5 * 1024 * 1024),
    }];
    let dataset = Dataset::new(vec![Column::numeric(
        ColumnId(1),
        "Original",
        vec![Some(17.0)],
    )
    .unwrap()
    .with_metadata(metadata)
    .unwrap()])
    .unwrap();
    let analysis = describe(&dataset, &request(), &JobControl::default()).unwrap();
    state = DesktopState::from_project(
        Project::new("Large provenance", dataset)
            .unwrap()
            .with_provenance(vec!["p".repeat(5 * 1024 * 1024)])
            .unwrap()
            .with_run(analysis)
            .unwrap(),
    )
    .unwrap();
    state = state
        .apply_action(UiAction::SelectRun(RunId(1)))
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Results))
        .unwrap();
    let size = Vec2::new(1180.0, 1000.0);
    WATCH_RENDER.store(true, Ordering::Relaxed);
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    click(
        &ctx,
        &mut state,
        &mut jobs,
        "Run settings and provenance",
        size,
    );
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    click(&ctx, &mut state, &mut jobs, "Analyses", size);
    click(&ctx, &mut state, &mut jobs, "Add variables…", size);
    WATCH_RENDER.store(false, Ordering::Relaxed);
}
#[test]
fn click_then_enter_opens_the_clicked_cell() {
    let ctx = egui::Context::default();
    let mut state = state();
    let mut jobs = JobRunner::default();
    let size = Vec2::new(760.0, 520.0);
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    click(&ctx, &mut state, &mut jobs, "Cell 1:0", size);
    frame(
        &ctx,
        &mut state,
        &mut jobs,
        size,
        vec![Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    assert!(state.control_rect("Cell editor").is_some());
}
#[test]
fn compact_results_keep_full_precision_accessible() {
    let dataset = Dataset::new(vec![Column::numeric(
        ColumnId(1),
        "Score",
        [17.0, 22.0, 25.0, 20.0, 25.0, 31.0].map(Some).to_vec(),
    )
    .unwrap()])
    .unwrap();
    let computed = describe(&dataset, &request(), &JobControl::default()).unwrap();
    let exact = computed.rows[0].estimates[&Statistic::Mean].value.unwrap();
    let mut state = DesktopState::from_project(
        Project::new("Results", dataset)
            .unwrap()
            .with_run(computed)
            .unwrap(),
    )
    .unwrap();
    state = state
        .apply_action(UiAction::SelectRun(RunId(1)))
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Results))
        .unwrap();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut jobs = JobRunner::default();
    let labels = access_labels(&ctx, &mut state, &mut jobs);
    assert!(
        labels
            .iter()
            .any(|label| label == &format!("Mean: {exact}")),
        "{labels:?}"
    );
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1180.0, 780.0))),
            ..Default::default()
        },
        |ui| render(ui, &mut state, &mut jobs),
    );
    for expected in ["23.33", "4.84"] {
        assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Text(text) if text.galley.text()==expected)),"missing visible {expected}");
    }
    output.textures_delta.clear();
    assert_eq!(
        {
            let tessera::analysis::CompletedAnalysis::Describe(a) =
                &state.selected_run().unwrap().analysis
            else {
                panic!("expected descriptives")
            };
            &a.rows[0]
        }
        .estimates[&Statistic::Mean]
            .value,
        Some(exact)
    );
}
#[test]
fn inactive_navigation_is_unfilled_and_selection_uses_brand_selection_fill() {
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    ctx.set_theme(egui::Theme::Light);
    let mut state = state();
    let mut jobs = JobRunner::default();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(760.0, 520.0))),
            ..Default::default()
        },
        |ui| render(ui, &mut state, &mut jobs),
    );
    let data = state.control_rect("Data").unwrap();
    let variables = state.control_rect("Variables").unwrap();
    output.textures_delta.clear();
    let has_fill = |rect: Rect, colour: Option<egui::Color32>| {
        output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Rect(shape) if shape.rect.center().distance(rect.center())<1.0&&shape.fill!=egui::Color32::TRANSPARENT&&colour.is_none_or(|colour|shape.fill==colour)))
    };
    assert!(!has_fill(variables, None));
    assert!(has_fill(data, Some(egui::Color32::from_rgb(228, 237, 247))));
}

fn popup_access_frame(
    ctx: &egui::Context,
    state: &mut DesktopState,
    jobs: &mut JobRunner,
    events: Vec<Event>,
) -> egui::accesskit::TreeUpdate {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1180.0, 780.0))),
            events,
            ..Default::default()
        },
        |ui| render(ui, state, jobs),
    );
    output.textures_delta.clear();
    output.platform_output.accesskit_update.unwrap()
}
fn reachable_access_labels(update: &egui::accesskit::TreeUpdate) -> Vec<String> {
    let mut stack = vec![update.tree.as_ref().unwrap().root];
    let mut labels = Vec::new();
    while let Some(id) = stack.pop() {
        if let Some((_, node)) = update.nodes.iter().find(|(node_id, _)| *node_id == id) {
            if let Some(label) = node.label().or_else(|| node.value()) {
                labels.push(label.to_owned());
            }
            stack.extend_from_slice(node.children());
        }
    }
    labels
}
#[test]
fn dirty_new_with_open_file_menu_exposes_modal_and_escape_cancels() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut state = state();
    let project = Arc::clone(state.project());
    let mut jobs = JobRunner::default();
    let tree = popup_access_frame(&ctx, &mut state, &mut jobs, vec![]);
    let bounds = tree
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("File"))
        .unwrap()
        .1
        .bounds()
        .unwrap();
    let position = Pos2::new(
        ((bounds.x0 + bounds.x1) / 2.0) as f32,
        ((bounds.y0 + bounds.y1) / 2.0) as f32,
    );
    for pressed in [true, false] {
        popup_access_frame(
            &ctx,
            &mut state,
            &mut jobs,
            vec![
                Event::PointerMoved(position),
                Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
    }
    assert!(egui::Popup::is_any_open(&ctx));
    popup_access_frame(
        &ctx,
        &mut state,
        &mut jobs,
        vec![Event::Key {
            key: Key::N,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        }],
    );
    assert_eq!(state.dialog(), Some(Dialog::Unsaved));
    let tree = popup_access_frame(&ctx, &mut state, &mut jobs, vec![]);
    let labels = reachable_access_labels(&tree);
    for expected in ["Save changes", "Discard", "Cancel"] {
        assert!(
            labels.iter().any(|label| label == expected),
            "missing {expected}: {labels:?}"
        );
    }
    assert!(
        !egui::Popup::is_any_open(&ctx),
        "background menu must yield to the modal"
    );
    let file_id = tree
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("File"))
        .unwrap()
        .0;
    popup_access_frame(
        &ctx,
        &mut state,
        &mut jobs,
        vec![Event::AccessKitActionRequest(
            egui::accesskit::ActionRequest {
                action: egui::accesskit::Action::Click,
                target_tree: egui::accesskit::TreeId::ROOT,
                target_node: file_id,
                data: None,
            },
        )],
    );
    let tree = popup_access_frame(&ctx, &mut state, &mut jobs, vec![]);
    let labels = reachable_access_labels(&tree);
    assert!(!egui::Popup::is_any_open(&ctx));
    assert!(labels.iter().any(|label| label == "Cancel"));
    assert!(!labels.iter().any(|label| label == "Open…"));
    popup_access_frame(
        &ctx,
        &mut state,
        &mut jobs,
        vec![Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert!(state.pending().is_none());
    assert!(state.dialog().is_none());
    assert!(Arc::ptr_eq(&project, state.project()));
}

#[test]
fn only_matching_cancelled_completion_clears_cancelling_status() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut state = state();
    let mut jobs = JobRunner::default();
    let token = state.token();
    state
        .handle_event(JobEvent::CancellationRequested { token })
        .unwrap();
    let stale = tessera::operation::JobToken {
        generation: token.generation + 1,
        ..token
    };
    assert!(state
        .handle_event(JobEvent::Completed {
            token: stale,
            cancelled: true,
            result: Err(tessera::error::AppError::Cancelled)
        })
        .is_err());
    assert!(access_labels(&ctx, &mut state, &mut jobs)
        .iter()
        .any(|label| label == "Cancelling…"));
    assert!(state
        .handle_event(JobEvent::Completed {
            token,
            cancelled: true,
            result: Err(tessera::error::AppError::Cancelled)
        })
        .is_err());
    assert!(!access_labels(&ctx, &mut state, &mut jobs)
        .iter()
        .any(|label| label == "Cancelling…"));
}

#[test]
fn repeated_close_keeps_protective_save_and_reopens_exact_edit() {
    let dir = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("tessera-repeated-close-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("project.tessera");
    let _ = std::fs::remove_file(&path);
    let mut state = DesktopState::from_project(
        Project::new(
            "Close",
            Dataset::new(vec![Column::numeric(
                ColumnId(1),
                "Score",
                vec![Some(17.0); 300000],
            )
            .unwrap()])
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let mut jobs = JobRunner::default();
    let project = Arc::clone(state.project());
    run(
        &mut state,
        &mut jobs,
        JobOperation::Save {
            project,
            path: path.clone(),
            expected: None,
        },
    );
    state = state
        .apply_action(UiAction::EditCell {
            column: ColumnId(1),
            row: 0,
            value: Cell::Number(88.0),
        })
        .unwrap()
        .apply_action(UiAction::Request(PendingAction::Close))
        .unwrap()
        .apply_action(UiAction::ResolveUnsaved(UnsavedChoice::Save))
        .unwrap();
    let ctx = egui::Context::default();
    let size = Vec2::new(760.0, 520.0);
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    assert!(jobs.is_running());
    for _ in 0..3 {
        state = state
            .apply_action(UiAction::Request(PendingAction::Close))
            .unwrap();
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    while jobs.is_running() {
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    assert!(state.error().is_none(), "{:?}", state.error());
    assert!(!state.is_dirty());
    let reopened =
        tessera::project_io::decode(std::fs::File::open(&path).unwrap(), &JobControl::default())
            .unwrap();
    assert_eq!(
        reopened.dataset().cell(ColumnId(1), 0).unwrap(),
        Cell::Number(88.0)
    );
    std::fs::remove_dir_all(dir).unwrap();
}
