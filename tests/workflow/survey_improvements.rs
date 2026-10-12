use super::*;
use tessera::survey::{
    cleaning::{CleaningRequest, CleaningSpec},
    SurveyRole,
};

#[test]
fn draft_preparation_handles_long_unicode_source_header() {
    let name = "ŵ".repeat(8000);
    let dataset = tessera::data::Dataset::new(vec![tessera::data::Column::text(
        ColumnId(1),
        &name,
        vec![Some(" A ".into())],
    )
    .unwrap()])
    .unwrap();
    let mut state =
        DesktopState::from_project(tessera::project::Project::new("Long header", dataset).unwrap())
            .unwrap();
    let mut jobs = JobRunner::default();
    state.enter_survey().unwrap();
    state.service_survey(&mut jobs).unwrap();
    finish(&mut state, &mut jobs);
    let mut draft = state.survey_cleaning_draft().clone();
    draft.column = Some(ColumnId(1));
    draft.trim = true;
    state.set_survey_cleaning_draft(draft).unwrap();
    state.preview_survey_cleaning_draft(&mut jobs).unwrap();
    finish(&mut state, &mut jobs);
    assert!(state.error().is_none(), "{:?}", state.error());
    state.apply_survey_cleaning(&mut jobs).unwrap();
    finish(&mut state, &mut jobs);
    assert!(state.error().is_none(), "{:?}", state.error());
    assert_eq!(state.project().dataset().columns()[0].name(), name);
    assert!(state.project().dataset().columns()[1].name().len() <= 1024);
}

#[test]
fn staged_replacements_merge_labels_and_define_multiple_missing_tokens() {
    let dataset = tessera::data::Dataset::new(vec![tessera::data::Column::text(
        ColumnId(1),
        "Activity",
        vec![
            Some(" reading ".into()),
            Some("READING".into()),
            Some("N/A".into()),
            Some("?".into()),
            Some("Walking".into()),
        ],
    )
    .unwrap()])
    .unwrap();
    let mut state = DesktopState::from_project(
        tessera::project::Project::new("Multiple labels", dataset).unwrap(),
    )
    .unwrap();
    let mut jobs = JobRunner::default();
    let temp = Temp::new();
    state.enter_survey().unwrap();
    state.service_survey(&mut jobs).unwrap();
    finish(&mut state, &mut jobs);
    state
        .set_survey_role(ColumnId(1), SurveyRole::Categories)
        .unwrap();
    for (from, to, missing) in [
        ("reading", "Reading", false),
        ("READING", "Reading", false),
        ("N/A", "", true),
        ("?", "", true),
    ] {
        let mut draft = state.copy_survey_cleaning_draft().unwrap();
        draft.column = Some(ColumnId(1));
        draft.trim = true;
        draft.replacement = true;
        draft.from = from.into();
        draft.to = to.into();
        draft.missing = missing;
        state.set_survey_cleaning_draft(draft).unwrap();
        state.stage_survey_replacement().unwrap();
    }
    assert_eq!(state.survey_cleaning_draft().rules.len(), 4);
    state.preview_survey_cleaning_draft(&mut jobs).unwrap();
    finish(&mut state, &mut jobs);
    assert!(state.error().is_none(), "{:?}", state.error());
    assert_eq!(state.survey_cleaning_preview().unwrap().changed, 4);
    state.apply_survey_cleaning(&mut jobs).unwrap();
    finish(&mut state, &mut jobs);
    assert!(state.error().is_none(), "{:?}", state.error());
    let id = state.project().dataset().columns()[1].id();
    for (row, value) in [
        Cell::Text("Reading".into()),
        Cell::Text("Reading".into()),
        Cell::Missing,
        Cell::Missing,
        Cell::Text("Walking".into()),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(state.project().dataset().cell(id, row).unwrap(), value);
    }
    let path = temp.0.join("multiple.tessera-recipe");
    state.save_survey_recipe(&mut jobs, path.clone()).unwrap();
    finish(&mut state, &mut jobs);
    assert!(state.error().is_none(), "{:?}", state.error());
    let (recipe, _) =
        tessera::survey::recipe::load_with_budget(&path, &JobControl::default(), &[], 0).unwrap();
    assert_eq!(recipe.columns[0].cleaning.as_ref().unwrap().rules.len(), 4);
}

#[test]
fn cleaning_preview_requires_explicit_apply_and_preserves_source() {
    let mut state = imported();
    let mut jobs = JobRunner::default();
    state.enter_survey().unwrap();
    state.service_survey(&mut jobs).unwrap();
    finish(&mut state, &mut jobs);
    let before = Arc::clone(state.project());
    let request = CleaningRequest {
        revision: before.dataset().revision(),
        source: ColumnId(1),
        name: "Reviewed labels".into(),
        spec: CleaningSpec {
            trim_whitespace: true,
            rules: vec![],
        },
    };
    state.preview_survey_cleaning(&mut jobs, request).unwrap();
    finish(&mut state, &mut jobs);
    assert!(Arc::ptr_eq(&before, state.project()));
    assert!(state.survey_cleaning_preview().is_some());
    state.apply_survey_cleaning(&mut jobs).unwrap();
    finish(&mut state, &mut jobs);
    assert!(state.error().is_none(), "{:?}", state.error());
    for row in 0..before.dataset().row_count() {
        assert_eq!(
            state.project().dataset().cell(ColumnId(1), row).unwrap(),
            before.dataset().cell(ColumnId(1), row).unwrap()
        );
    }
    assert_eq!(
        state.project().dataset().column_count(),
        before.dataset().column_count() + 1
    );
    assert_eq!(
        state
            .survey_choices()
            .iter()
            .find(|c| c.column == ColumnId(1))
            .unwrap()
            .role,
        SurveyRole::Exclude
    );
    state = state.apply_action(UiAction::Undo).unwrap();
    assert_eq!(
        state.project().dataset().column_count(),
        before.dataset().column_count()
    );
}

#[test]
fn recipe_saved_after_cleaning_replays_original_structure_once() {
    let mut state = imported();
    let mut jobs = JobRunner::default();
    let temp = Temp::new();
    state.enter_survey().unwrap();
    state.service_survey(&mut jobs).unwrap();
    finish(&mut state, &mut jobs);
    state
        .set_survey_role(ColumnId(1), SurveyRole::Categories)
        .unwrap();
    state
        .set_survey_role(ColumnId(2), SurveyRole::NumericMeasure)
        .unwrap();
    state
        .preview_survey_cleaning(
            &mut jobs,
            CleaningRequest {
                revision: state.project().dataset().revision(),
                source: ColumnId(1),
                name: "Reviewed labels".into(),
                spec: CleaningSpec {
                    trim_whitespace: true,
                    rules: vec![],
                },
            },
        )
        .unwrap();
    finish(&mut state, &mut jobs);
    state.apply_survey_cleaning(&mut jobs).unwrap();
    finish(&mut state, &mut jobs);
    assert!(state.survey_ready(), "{:?}", state.error());
    let path = temp.0.join("survey.tessera-recipe");
    state.save_survey_recipe(&mut jobs, path.clone()).unwrap();
    finish(&mut state, &mut jobs);
    assert!(state.error().is_none(), "{:?}", state.error());
    let mut fresh = imported();
    let before = Arc::clone(fresh.project());
    fresh.enter_survey().unwrap();
    fresh.service_survey(&mut jobs).unwrap();
    finish(&mut fresh, &mut jobs);
    fresh.load_survey_recipe(&mut jobs, path.clone()).unwrap();
    finish(&mut fresh, &mut jobs);
    assert!(fresh.error().is_none(), "{:?}", fresh.error());
    assert!(Arc::ptr_eq(&before, fresh.project()));
    fresh.replay_survey_recipe(&mut jobs).unwrap();
    finish(&mut fresh, &mut jobs);
    assert!(fresh.error().is_none(), "{:?}", fresh.error());
    assert_eq!(fresh.project().dataset().column_count(), 3);
    assert_eq!(fresh.survey_run_ids().len(), 3);
    fresh.load_survey_recipe(&mut jobs, path).unwrap();
    finish(&mut fresh, &mut jobs);
    assert!(fresh.error().is_some());
    assert_eq!(fresh.project().dataset().column_count(), 3);
    fresh = fresh.apply_action(UiAction::Undo).unwrap();
    assert_eq!(fresh.project().dataset().column_count(), 2);
}

#[test]
fn changed_revision_invalidates_preview_and_stable_unresolved_order() {
    let mut state = imported();
    let mut jobs = JobRunner::default();
    state.enter_survey().unwrap();
    state.service_survey(&mut jobs).unwrap();
    finish(&mut state, &mut jobs);
    let order = state.survey_order().to_vec();
    state
        .set_survey_role(ColumnId(2), SurveyRole::NumericMeasure)
        .unwrap();
    assert_eq!(state.survey_order(), order);
    state
        .preview_survey_cleaning(
            &mut jobs,
            CleaningRequest {
                revision: state.project().dataset().revision(),
                source: ColumnId(1),
                name: "Prepared".into(),
                spec: CleaningSpec {
                    trim_whitespace: true,
                    rules: vec![],
                },
            },
        )
        .unwrap();
    finish(&mut state, &mut jobs);
    state = state
        .apply_action(UiAction::EditCell {
            column: ColumnId(1),
            row: 0,
            value: Cell::Text("changed".into()),
        })
        .unwrap();
    assert!(state.survey_cleaning_preview().is_none());
    assert!(state.apply_survey_cleaning(&mut jobs).is_err());
}

#[test]
fn cleaning_guard_rejects_unfinished_edits_and_cancelled_preview() {
    let mut state = imported();
    let mut jobs = JobRunner::default();
    state.enter_survey().unwrap();
    state.service_survey(&mut jobs).unwrap();
    finish(&mut state, &mut jobs);
    let before = Arc::clone(state.project());
    let request = CleaningRequest {
        revision: before.dataset().revision(),
        source: ColumnId(1),
        name: "Prepared".into(),
        spec: CleaningSpec {
            trim_whitespace: true,
            rules: vec![],
        },
    };
    state = state
        .apply_action(UiAction::BeginCell {
            column: ColumnId(2),
            row: 0,
        })
        .unwrap();
    state.set_cell_text("invalid".into()).unwrap();
    assert!(state.preview_survey_cleaning(&mut jobs, request).is_err());
    assert!(Arc::ptr_eq(&before, state.project()));
    let mut clean = imported();
    clean.enter_survey().unwrap();
    clean.service_survey(&mut jobs).unwrap();
    finish(&mut clean, &mut jobs);
    let before = Arc::clone(clean.project());
    clean
        .preview_survey_cleaning(
            &mut jobs,
            CleaningRequest {
                revision: before.dataset().revision(),
                source: ColumnId(1),
                name: "Prepared".into(),
                spec: CleaningSpec {
                    trim_whitespace: true,
                    rules: vec![],
                },
            },
        )
        .unwrap();
    jobs.cancel();
    finish(&mut clean, &mut jobs);
    assert!(Arc::ptr_eq(&before, clean.project()));
    assert!(clean.survey_cleaning_preview().is_none());
}
