use super::*;

#[test]
fn guided_question_opens_form_without_running_and_retains_manual_access() {
    let mut state = state()
        .apply_action(UiAction::Navigate(Workspace::Analyses))
        .unwrap();
    let ctx = egui::Context::default();
    let mut jobs = JobRunner::default();
    let size = Vec2::new(640., 440.);
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    click(&ctx, &mut state, &mut jobs, "Help choose an analysis", size);
    assert!(state.control_rect("Open recommended analysis").is_some());
    click(&ctx, &mut state, &mut jobs, "Compare measurements", size);
    click(
        &ctx,
        &mut state,
        &mut jobs,
        "Separate people or observations",
        size,
    );
    click(
        &ctx,
        &mut state,
        &mut jobs,
        "Two groups or measurements",
        size,
    );
    click(
        &ctx,
        &mut state,
        &mut jobs,
        "Open recommended analysis",
        size,
    );
    assert_eq!(
        state.draft().procedure(),
        tessera::desktop::draft::Procedure::IndependentT
    );
    assert!(!jobs.is_running());
    assert!(state.project().runs().is_empty());
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    assert!(state.control_rect("Help choose an analysis").is_some());
}

#[test]
fn guidance_keyboard_and_small_window_keep_actions_reachable() {
    let mut state = state()
        .apply_action(UiAction::Navigate(Workspace::Analyses))
        .unwrap();
    let ctx = egui::Context::default();
    let mut jobs = JobRunner::default();
    let size = Vec2::new(640., 440.);
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    let id = state.control_id("Help choose an analysis").unwrap();
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
    for label in ["Open recommended analysis", "Choose manually"] {
        let rect = state.control_rect(label).unwrap();
        assert!(rect.max.y <= size.y && rect.min.y >= 0.);
    }
    assert!(!jobs.is_running());
}

#[test]
fn guided_repeated_design_opens_mapping_and_unsupported_category_stays_blocked() {
    let mut state = state()
        .apply_action(UiAction::Navigate(Workspace::Analyses))
        .unwrap();
    let ctx = egui::Context::default();
    let mut jobs = JobRunner::default();
    let size = Vec2::new(900., 700.);
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    click(&ctx, &mut state, &mut jobs, "Help choose an analysis", size);
    for label in [
        "Compare measurements",
        "Same people measured repeatedly",
        "Three or more groups or measurements",
    ] {
        click(&ctx, &mut state, &mut jobs, label, size);
    }
    click(
        &ctx,
        &mut state,
        &mut jobs,
        "Categories or response labels",
        size,
    );
    click(
        &ctx,
        &mut state,
        &mut jobs,
        "Open recommended analysis",
        size,
    );
    assert!(state.control_rect("Choose manually").is_some());
    assert_eq!(
        state.draft().procedure(),
        tessera::desktop::draft::Procedure::Describe
    );
    click(&ctx, &mut state, &mut jobs, "Numeric measurements", size);
    click(
        &ctx,
        &mut state,
        &mut jobs,
        "Open recommended analysis",
        size,
    );
    assert!(matches!(
        state.draft(),
        tessera::desktop::draft::AnalysisDraft::Anova(tessera::desktop::draft::anova::AnovaDraft {
            design: tessera::desktop::draft::anova::AnovaDesignDraft::Repeated {
                subject: None,
                ..
            },
            ..
        })
    ));
    assert!(!jobs.is_running());
    assert!(state.project().runs().is_empty());
}

#[test]
fn independent_several_groups_replaces_prior_anova_design_through_guide() {
    use tessera::desktop::draft::{
        anova::{AnovaDesignDraft, AnovaDraft},
        AnalysisDraft,
    };
    for previous in [
        AnovaDesignDraft::Repeated {
            subject: None,
            factors: vec![],
            measurements: vec![],
        },
        AnovaDesignDraft::Factorial {
            outcome: None,
            factors: vec![],
        },
    ] {
        let mut state = state()
            .apply_action(UiAction::SetDraft(AnalysisDraft::Anova(AnovaDraft {
                design: previous,
                ..Default::default()
            })))
            .unwrap()
            .apply_action(UiAction::Navigate(Workspace::Analyses))
            .unwrap();
        let ctx = egui::Context::default();
        let mut jobs = JobRunner::default();
        let size = Vec2::new(900., 700.);
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
        click(&ctx, &mut state, &mut jobs, "Help choose an analysis", size);
        for label in [
            "Compare measurements",
            "Separate people or observations",
            "Three or more groups or measurements",
            "Open recommended analysis",
        ] {
            click(&ctx, &mut state, &mut jobs, label, size);
        }
        assert!(matches!(
            state.draft(),
            AnalysisDraft::Anova(AnovaDraft {
                design: AnovaDesignDraft::OneWay {
                    outcome: None,
                    factor: None
                },
                ..
            })
        ));
        assert!(!jobs.is_running());
        assert!(state.project().runs().is_empty());
    }
}
