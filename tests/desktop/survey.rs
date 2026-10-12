use super::*;
#[test]
fn survey_entry_empty_and_both_modes() {
    for mode in [
        tessera::desktop::AnalysisMode::Guided,
        tessera::desktop::AnalysisMode::Advanced,
    ] {
        for workspace in [Workspace::Data, Workspace::Analyses] {
            for full in [false, true] {
                let state = if full {
                    state()
                } else {
                    DesktopState::new().unwrap()
                };
                let mut state = state
                    .apply_action(UiAction::SetAnalysisMode(mode))
                    .unwrap()
                    .apply_action(UiAction::Navigate(workspace))
                    .unwrap();
                let ctx = egui::Context::default();
                frame(
                    &ctx,
                    &mut state,
                    &mut JobRunner::default(),
                    Vec2::new(640., 440.),
                    vec![],
                );
                assert!(state.control_rect("Instant survey report").is_some());
            }
        }
    }
}
#[test]
fn survey_review_compact_footer_and_roles() {
    let mut state = state();
    let mut jobs = JobRunner::default();
    state.enter_survey().unwrap();
    state.service_survey(&mut jobs).unwrap();
    while jobs.is_running() {
        state.poll(&mut jobs);
        std::thread::yield_now();
    }
    let ctx = egui::Context::default();
    frame(&ctx, &mut state, &mut jobs, Vec2::new(640., 440.), vec![]);
    for label in [
        "Create report",
        "Manual analysis",
        "Prepare data",
        "Variable definitions",
        "Survey role 1",
    ] {
        let rect = state
            .control_rect(label)
            .unwrap_or_else(|| panic!("missing {label}"));
        assert!(rect.max.y <= 440., "{label}: {rect:?}");
    }
    assert!(!state.survey_ready());
    click(
        &ctx,
        &mut state,
        &mut jobs,
        "Create report",
        Vec2::new(640., 440.),
    );
    assert!(!jobs.is_running());
    assert!(state.error().is_none());
    assert!(state.project().runs().is_empty());
}

#[test]
fn survey_keyboard_paging_modes_and_results_edit_access() {
    for mode in [
        tessera::desktop::AnalysisMode::Guided,
        tessera::desktop::AnalysisMode::Advanced,
    ] {
        let dataset = Dataset::new(
            (1..=100)
                .map(|id| {
                    Column::numeric(
                        ColumnId(id),
                        &format!("Question {id}"),
                        vec![Some(1.), Some(2.)],
                    )
                    .unwrap()
                })
                .collect(),
        )
        .unwrap();
        let mut state = DesktopState::from_project(Project::new("Survey", dataset).unwrap())
            .unwrap()
            .apply_action(UiAction::SetAnalysisMode(mode))
            .unwrap();
        let mut jobs = JobRunner::default();
        state.enter_survey().unwrap();
        state.service_survey(&mut jobs).unwrap();
        while jobs.is_running() {
            state.poll(&mut jobs);
            std::thread::yield_now();
        }
        let ctx = egui::Context::default();
        let size = Vec2::new(640., 440.);
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
        assert!(state.control_rect("Survey role 100").is_none());
        let next = "Next survey variables page";
        let id = state.control_id(next).unwrap();
        ctx.memory_mut(|m| m.request_focus(id));
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
        assert!(state.control_rect("Survey role 1").is_none());
        for id in 1..=100 {
            state
                .set_survey_role(
                    ColumnId(id),
                    if id == 1 {
                        tessera::survey::SurveyRole::Categories
                    } else {
                        tessera::survey::SurveyRole::Exclude
                    },
                )
                .unwrap();
        }
        state.submit_survey(&mut jobs).unwrap();
        while jobs.is_running() {
            state.poll(&mut jobs);
            std::thread::yield_now();
        }
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
        assert!(state.control_rect("Export survey report").unwrap().max.y <= 440.);
        // egui hit-tests against the previous frame; wait for the current Results target.
        let mut hovered = false;
        let mut targets = Vec::new();
        for _ in 0..8 {
            let edit = state.control_rect("Edit selected analysis").unwrap();
            let id = state.control_id("Edit selected analysis").unwrap();
            assert!(
                ctx.content_rect().contains_rect(edit),
                "edit outside {edit:?}"
            );
            frame(
                &ctx,
                &mut state,
                &mut jobs,
                size,
                vec![Event::PointerMoved(edit.center())],
            );
            hovered = ctx.interaction_snapshot(|snapshot| snapshot.hovered.contains(&id))
                && state.control_rect("Edit selected analysis") == Some(edit)
                && state.control_id("Edit selected analysis") == Some(id);
            targets.push((id, edit, hovered));
            if hovered {
                break;
            }
        }
        assert!(
            hovered,
            "Results edit target did not become interactive: {targets:?}"
        );
        click(&ctx, &mut state, &mut jobs, "Edit selected analysis", size);
        assert_eq!(
            state.workspace(),
            Workspace::Analyses,
            "mode {mode:?}, error {:?}, active {}, review {}",
            state.error(),
            jobs.is_running(),
            state.survey_visible()
        );
        assert!(!state.survey_visible());
    }
}

#[test]
fn survey_zoom_footer_stays_reachable() {
    let mut state = state();
    let mut jobs = JobRunner::default();
    state.enter_survey().unwrap();
    state.service_survey(&mut jobs).unwrap();
    while jobs.is_running() {
        state.poll(&mut jobs);
        std::thread::yield_now();
    }
    let ctx = egui::Context::default();
    ctx.set_zoom_factor(1.5);
    frame(&ctx, &mut state, &mut jobs, Vec2::new(640., 440.), vec![]);
    for label in [
        "Create report",
        "Manual analysis",
        "Prepare data",
        "Variable definitions",
    ] {
        let rect = state.control_rect(label).unwrap();
        assert!(ctx.content_rect().contains_rect(rect), "{label} {rect:?}");
    }
}

#[test]
fn survey_text_role_menu_has_categories_but_no_numeric_measure() {
    let mut state = state();
    let mut jobs = JobRunner::default();
    state.enter_survey().unwrap();
    state.service_survey(&mut jobs).unwrap();
    while jobs.is_running() {
        state.poll(&mut jobs);
        std::thread::yield_now();
    }
    let ctx = egui::Context::default();
    let size = Vec2::new(640., 440.);
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    // Page the sparse review rather than rendering every variable at once.
    if state.control_rect("Survey role 2").is_none() {
        click(
            &ctx,
            &mut state,
            &mut jobs,
            "Next survey variables page",
            size,
        );
    }
    let id = state.control_id("Survey role 2").unwrap();
    ctx.memory_mut(|memory| memory.request_focus(id));
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
    assert!(state
        .control_rect("Survey role 2: Count responses")
        .is_some());
    assert!(state
        .control_rect("Survey role 2: Summarise a measurement")
        .is_none());
}
