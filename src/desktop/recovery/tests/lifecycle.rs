use super::*;

fn modal_frame(
    ctx: &egui::Context,
    manager: &mut RecoveryManager,
    state: &mut DesktopState,
    jobs: &mut JobRunner,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640., 440.),
            )),
            events,
            ..Default::default()
        },
        |ui| manager.show(ui.ctx(), state, jobs),
    );
    output.textures_delta.clear();
    output
}

#[test]
fn compact_cleanup_modal_keeps_copy_and_keep_reachable_with_long_paths() {
    for zoom in [1., 1.5] {
        let temp = Temp::new();
        let root = temp
            .0
            .join("long-path-component-".repeat(4))
            .join("another-long-component-".repeat(4))
            .join("last-long-component-".repeat(4));
        let store = RecoveryStore::open_at(&root).unwrap();
        let mut outcome = store
            .checkpoint(
                &Project::new("Private checkpoint", Dataset::new(vec![]).unwrap()).unwrap(),
                1,
                &JobControl::default(),
            )
            .unwrap();
        let retained = store
            .path()
            .parent()
            .unwrap()
            .join("retained-cleanup-failure");
        std::fs::create_dir(&retained).unwrap();
        let displaced = retained.join("prepared");
        std::fs::write(&displaced, b"private previous version").unwrap();
        outcome.recovery = Some(crate::files::RecoveryPaths {
            directory: retained.clone(),
            displaced: Some(displaced.clone()),
            frozen_revision: Some(displaced.clone()),
            prepared: Some(displaced.clone()),
        });
        outcome.warning = Some("Recovery cleanup could not be confirmed.".into());
        let mut manager = RecoveryManager::with_store(store).unwrap();
        manager.prompt = false;
        manager.discovery = false;
        manager.checkpoint_owned = true;
        manager.failed = true;
        manager.cleanup_decision = true;
        let mut state = DesktopState::new().unwrap();
        state.recovery_outcome = Some(outcome);
        let mut jobs = JobRunner::default();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        ctx.set_zoom_factor(zoom);
        ctx.all_styles_mut(|style| style.animation_time = 0.);
        for _ in 0..3 {
            modal_frame(&ctx, &mut manager, &mut state, &mut jobs, vec![]);
        }
        let mut copied = false;
        let mut keep_bounds = None;
        for _ in 0..40 {
            let output = modal_frame(&ctx, &mut manager, &mut state, &mut jobs, vec![]);
            let mut copy_bounds = None;
            for (_, node) in output
                .platform_output
                .accesskit_update
                .into_iter()
                .flat_map(|update| update.nodes)
            {
                match node.label() {
                    Some("Copy Recovery") => copy_bounds = node.bounds(),
                    Some("Keep recovery files and continue") => keep_bounds = node.bounds(),
                    _ => {}
                }
            }
            let keep = keep_bounds.expect("Keep must remain accessible in the compact modal");
            assert!(
                ctx.content_rect()
                    .contains(egui::pos2(keep.x0 as f32, keep.y0 as f32))
                    && ctx
                        .content_rect()
                        .contains(egui::pos2(keep.x1 as f32, keep.y1 as f32)),
                "Keep is clipped at {zoom}x: {keep:?}"
            );
            if let Some(bounds) = copy_bounds {
                let pos = egui::pos2(
                    ((bounds.x0 + bounds.x1) / 2.) as f32,
                    ((bounds.y0 + bounds.y1) / 2.) as f32,
                );
                if ctx.content_rect().contains(pos) {
                    for pressed in [true, false] {
                        let out = modal_frame(
                            &ctx,
                            &mut manager,
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
                        copied |= out.platform_output.commands.iter().any(|command| matches!(command, egui::OutputCommand::CopyText(text) if text == &retained.to_string_lossy()));
                    }
                }
            }
            if copied {
                break;
            }
            let pos = ctx.content_rect().center() - egui::vec2(0., 30.);
            modal_frame(
                &ctx,
                &mut manager,
                &mut state,
                &mut jobs,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        phase: egui::TouchPhase::Move,
                        delta: egui::vec2(0., -80.),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(
            copied,
            "The full long recovery path must be copyable at {zoom}x"
        );
        let bounds = keep_bounds.unwrap();
        let pos = egui::pos2(
            ((bounds.x0 + bounds.x1) / 2.) as f32,
            ((bounds.y0 + bounds.y1) / 2.) as f32,
        );
        for pressed in [true, false] {
            modal_frame(
                &ctx,
                &mut manager,
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
        assert!(!manager.blocks_ui());
        assert_eq!(
            std::fs::read(displaced).unwrap(),
            b"private previous version"
        );
    }
}

#[test]
fn automatic_cleanup_failure_offers_keep_without_a_pending_transition() {
    let temp = Temp::new();
    let store = RecoveryStore::open_at(&temp.0).unwrap();
    let old_path = store.path().to_owned();
    let mut manager = RecoveryManager::with_store(store).unwrap();
    manager.prompt = false;
    manager.discovery = false;
    manager.checkpoint_owned = true;
    manager.failed = true;
    manager.cleanup_decision = true;
    let mut state = DesktopState::new().unwrap();
    let project = state.project().clone();
    assert!(state.pending().is_none());
    let mut jobs = JobRunner::default();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        manager.show(ui.ctx(), &mut state, &mut jobs)
    });
    output.textures_delta.clear();
    assert!(output
        .platform_output
        .accesskit_update
        .into_iter()
        .flat_map(|update| update.nodes)
        .any(|(_, node)| node.label() == Some("Keep recovery files and continue")));
    manager.keep_and_continue(&mut state);
    assert!(!manager.blocks_ui());
    assert!(manager.before_transition(&mut state, &mut jobs));
    assert_ne!(manager.store.path(), old_path);
    assert!(Arc::ptr_eq(&project, state.project()));
    assert!(!state.shutdown);
}

#[test]
fn cleanup_modal_copies_surviving_path_before_keep_and_quit() {
    let temp = Temp::new();
    let store = RecoveryStore::open_at(&temp.0).unwrap();
    let mut outcome = store
        .checkpoint(
            &Project::new("Private checkpoint", Dataset::new(vec![]).unwrap()).unwrap(),
            1,
            &JobControl::default(),
        )
        .unwrap();
    let retained = store
        .path()
        .parent()
        .unwrap()
        .join("retained-cleanup-failure");
    std::fs::create_dir(&retained).unwrap();
    let displaced = retained.join("prepared");
    std::fs::write(&displaced, b"private earlier complete version").unwrap();
    outcome.recovery = Some(crate::files::RecoveryPaths {
        directory: retained.clone(),
        displaced: Some(displaced.clone()),
        frozen_revision: None,
        prepared: None,
    });
    outcome.warning = Some("The prior complete version remains in recovery.".into());
    let mut manager = RecoveryManager::with_store(store).unwrap();
    manager.prompt = false;
    manager.discovery = false;
    manager.checkpoint_owned = true;
    manager.failed = true;
    manager.cleanup_decision = true;
    let mut state = DesktopState::new().unwrap();
    state.act(super::super::super::UiAction::Request(
        super::super::super::PendingAction::Close,
    ));
    state.recovery_outcome = Some(outcome);
    let mut jobs = JobRunner::default();
    assert!(!manager.before_transition(&mut state, &mut jobs));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let frame =
        |events, manager: &mut RecoveryManager, state: &mut DesktopState, jobs: &mut JobRunner| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180., 780.),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| manager.show(ui.ctx(), state, jobs),
            );
            output.textures_delta.clear();
            output
        };
    let mut copy_bounds = None;
    for _ in 0..3 {
        let output = frame(vec![], &mut manager, &mut state, &mut jobs);
        for (_, node) in output
            .platform_output
            .accesskit_update
            .into_iter()
            .flat_map(|update| update.nodes)
        {
            if node.label() == Some("Copy Recovery") {
                copy_bounds = node.bounds();
            }
        }
    }
    let bounds = copy_bounds
        .expect("The blocking cleanup modal must expose a usable recovery-path copy control");
    let pos = egui::pos2(
        ((bounds.x0 + bounds.x1) / 2.) as f32,
        ((bounds.y0 + bounds.y1) / 2.) as f32,
    );
    let mut commands = Vec::new();
    for pressed in [true, false] {
        commands.extend(
            frame(
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                &mut manager,
                &mut state,
                &mut jobs,
            )
            .platform_output
            .commands,
        );
    }
    assert!(commands.iter().any(|command| matches!(command, egui::OutputCommand::CopyText(text) if text == &retained.to_string_lossy())));
    assert!(!state.shutdown);
    manager.keep_and_continue(&mut state);
    assert!(manager.before_transition(&mut state, &mut jobs));
    state.process_pending(&mut jobs);
    assert!(state.shutdown);
    assert_eq!(
        std::fs::read(displaced).unwrap(),
        b"private earlier complete version"
    );
}
