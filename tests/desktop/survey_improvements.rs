use super::*;

#[test]
fn loaded_recipe_shows_saved_settings_before_execution() {
    use tessera::survey::{
        cleaning::CleaningSpec,
        recipe::{encode_with_budget, RecipeColumn, SurveyRecipe},
        SurveyPresentation, SurveyRole,
    };
    let mut state = state();
    let mut jobs = JobRunner::default();
    state.enter_survey().unwrap();
    state.service_survey(&mut jobs).unwrap();
    while jobs.is_running() {
        state.poll(&mut jobs);
        std::thread::yield_now();
    }
    let recipe = SurveyRecipe {
        name: "Saved survey".into(),
        presentation: SurveyPresentation::HighContrast,
        columns: vec![
            RecipeColumn {
                name: "Sgôr ŵ".into(),
                kind: tessera::data::ColumnKind::Number,
                role: SurveyRole::NumericMeasure,
                cleaning: None,
            },
            RecipeColumn {
                name: "Enw".into(),
                kind: tessera::data::ColumnKind::Text,
                role: SurveyRole::Categories,
                cleaning: Some(CleaningSpec {
                    trim_whitespace: true,
                    rules: std::iter::once(tessera::preparation::RecodeRule {
                        from: Cell::Text("Carys".into()),
                        to: Cell::Missing,
                    })
                    .chain((0..12).map(|index| tessera::preparation::RecodeRule {
                        from: Cell::Text(format!("Old label {index}")),
                        to: Cell::Text(format!("Reviewed label {index}")),
                    }))
                    .collect(),
                }),
            },
        ],
    };
    let path = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "tessera-recipe-summary-{}.json",
        std::process::id()
    ));
    std::fs::write(
        &path,
        encode_with_budget(&recipe, &JobControl::default(), &[], 0).unwrap(),
    )
    .unwrap();
    state.load_survey_recipe(&mut jobs, path.clone()).unwrap();
    while jobs.is_running() {
        state.poll(&mut jobs);
        std::thread::yield_now();
    }
    std::fs::remove_file(path).unwrap();
    assert!(state.error().is_none(), "{:?}", state.error());
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let size = Vec2::new(1000., 900.);
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            ..Default::default()
        },
        |ui| render(ui, &mut state, &mut jobs),
    );
    output.textures_delta.clear();
    let mut labels: Vec<_> = output
        .platform_output
        .accesskit_update
        .unwrap()
        .nodes
        .into_iter()
        .filter_map(|(_, n)| n.label().or_else(|| n.value()).map(str::to_owned))
        .collect();
    click(
        &ctx,
        &mut state,
        &mut jobs,
        "Next saved recipe variables page",
        size,
    );
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            ..Default::default()
        },
        |ui| render(ui, &mut state, &mut jobs),
    );
    output.textures_delta.clear();
    labels.extend(
        output
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .into_iter()
            .filter_map(|(_, n)| n.label().or_else(|| n.value()).map(str::to_owned)),
    );
    let text = labels.join("\n");
    for expected in [
        "Saved survey",
        "Apply recipe uses these saved settings.",
        "Saved report style: High contrast",
        "Enw: Count responses",
        "Remove surrounding spaces",
        "Carys",
        "Missing",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    let mut paged_text = String::new();
    for _ in 0..20 {
        click(
            &ctx,
            &mut state,
            &mut jobs,
            "Next saved replacements page",
            size,
        );
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                ..Default::default()
            },
            |ui| render(ui, &mut state, &mut jobs),
        );
        output.textures_delta.clear();
        for shape in output.shapes {
            if let egui::Shape::Text(text) = shape.shape {
                paged_text.push_str(text.galley.text());
            }
        }
    }
    assert!(paged_text.contains("Reviewed label 11"), "{paged_text}");
    assert_eq!(state.survey_presentation(), SurveyPresentation::Report);
    assert!(!jobs.is_running());
    assert!(state.project().runs().is_empty());
    frame(&ctx, &mut state, &mut jobs, Vec2::new(640., 440.), vec![]);
    assert!(
        state
            .control_rect("Apply recipe and create report")
            .unwrap()
            .max
            .y
            <= 440.
    );
}

#[test]
fn preparation_offers_staged_replacements() {
    let mut state = state();
    let mut jobs = JobRunner::default();
    state.enter_survey().unwrap();
    state.service_survey(&mut jobs).unwrap();
    while jobs.is_running() {
        state.poll(&mut jobs);
        std::thread::yield_now();
    }
    let ctx = egui::Context::default();
    frame(&ctx, &mut state, &mut jobs, Vec2::new(1000., 900.), vec![]);
    click(
        &ctx,
        &mut state,
        &mut jobs,
        "Prepare response labels",
        Vec2::new(1000., 900.),
    );
    frame(&ctx, &mut state, &mut jobs, Vec2::new(1000., 900.), vec![]);
    assert!(state.control_rect("Add replacement").is_some());
    for (from, to, missing) in [("Carys", "Shared", false), ("", "", true)] {
        let mut draft = state.copy_survey_cleaning_draft().unwrap();
        draft.column = Some(ColumnId(2));
        draft.replacement = true;
        draft.from = from.into();
        draft.to = to.into();
        draft.missing = missing;
        state.set_survey_cleaning_draft(draft).unwrap();
        frame(&ctx, &mut state, &mut jobs, Vec2::new(1000., 900.), vec![]);
        let pos = state.control_rect("Add replacement").unwrap().center();
        frame(
            &ctx,
            &mut state,
            &mut jobs,
            Vec2::new(1000., 900.),
            vec![Event::PointerMoved(pos)],
        );
        click(
            &ctx,
            &mut state,
            &mut jobs,
            "Add replacement",
            Vec2::new(1000., 900.),
        );
        assert!(state.error().is_none(), "{:?}", state.error());
    }
    assert_eq!(state.survey_cleaning_draft().rules.len(), 2);
    frame(&ctx, &mut state, &mut jobs, Vec2::new(1000., 900.), vec![]);
    click(
        &ctx,
        &mut state,
        &mut jobs,
        "Remove replacement 1",
        Vec2::new(1000., 900.),
    );
    assert_eq!(state.survey_cleaning_draft().rules.len(), 1);
    frame(&ctx, &mut state, &mut jobs, Vec2::new(1000., 900.), vec![]);
    click(
        &ctx,
        &mut state,
        &mut jobs,
        "Preview label changes",
        Vec2::new(1000., 900.),
    );
    while jobs.is_running() {
        state.poll(&mut jobs);
        std::thread::yield_now();
    }
    assert!(state.error().is_none(), "{:?}", state.error());
    assert_eq!(state.survey_cleaning_preview().unwrap().changed, 1);
}

#[test]
fn improved_review_reaches_recipes_and_keeps_choices_stable() {
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
    assert!(state.control_rect("Load report recipe").is_some());
    let order = state.survey_order().to_vec();
    assert_eq!(order.first(), Some(&0));
    state
        .set_survey_role(ColumnId(1), tessera::survey::SurveyRole::NumericMeasure)
        .unwrap();
    state
        .set_survey_role(ColumnId(2), tessera::survey::SurveyRole::Exclude)
        .unwrap();
    assert_eq!(state.survey_order(), order);
    frame(&ctx, &mut state, &mut jobs, Vec2::new(640., 440.), vec![]);
    assert!(state.control_rect("Save report recipe").is_some());
    click(
        &ctx,
        &mut state,
        &mut jobs,
        "Create report",
        Vec2::new(640., 440.),
    );
    while jobs.is_running() {
        state.poll(&mut jobs);
        std::thread::yield_now();
    }
    assert!(state.error().is_none(), "{:?}", state.error());
    assert_eq!(state.survey_run_ids().len(), 2);
}
