use tessera::{
    data::{Column, ColumnId, Dataset},
    operation::JobControl,
    survey::{
        recipe::{decode_with_budget, encode_with_budget, SurveyRecipe},
        SurveyChoice, SurveyPresentation, SurveyRole,
    },
};
fn data(id: u64) -> Dataset {
    Dataset::new(vec![Column::text(
        ColumnId(id),
        "Choice",
        vec![Some("Yes".into())],
    )
    .unwrap()])
    .unwrap()
}
#[test]
fn portable_recipe_rebinds_ids_and_rejects_shape_changes() {
    let d = data(1);
    let c = JobControl::default();
    let recipe = SurveyRecipe::capture(
        &d,
        &[SurveyChoice {
            column: ColumnId(1),
            role: SurveyRole::Categories,
        }],
        &[],
        SurveyPresentation::default(),
        "Survey",
        &c,
        &[],
        0,
    )
    .unwrap();
    let bytes = encode_with_budget(&recipe, &c, &[], 0).unwrap();
    let decoded = decode_with_budget(&bytes, &c, &[], 0).unwrap();
    assert_eq!(
        decoded.bind(&data(99), &c).unwrap().choices[0].column,
        ColumnId(99)
    );
    let changed = Dataset::new(vec![
        Column::numeric(ColumnId(2), "Choice", vec![Some(1.)]).unwrap()
    ])
    .unwrap();
    assert!(decoded.bind(&changed, &c).is_err());
}
#[test]
fn untrusted_recipe_input_and_cancellation_are_rejected() {
    let c = JobControl::default();
    for input in [b"{}".as_slice(),b"{\"version\":99}",b"{\"version\":1,\"name\":\"x\",\"columns\":[],\"presentation\":\"report\",\"execute\":\"evil\"}"] {assert!(decode_with_budget(input,&c,&[],0).is_err());}
    c.cancel();
    assert!(decode_with_budget(b"{}", &c, &[], 0).is_err());
}
#[test]
fn duplicate_names_and_unapproved_roles_are_rejected() {
    let c = JobControl::default();
    let d = Dataset::new(vec![
        Column::text(ColumnId(1), "Choice", vec![Some("Yes".into())]).unwrap(),
        Column::text(ColumnId(2), "Other", vec![Some("No".into())]).unwrap(),
    ])
    .unwrap();
    let choices = [
        SurveyChoice {
            column: ColumnId(1),
            role: SurveyRole::Categories,
        },
        SurveyChoice {
            column: ColumnId(2),
            role: SurveyRole::Exclude,
        },
    ];
    let valid = SurveyRecipe::capture(
        &d,
        &choices,
        &[],
        SurveyPresentation::default(),
        "Survey",
        &c,
        &[],
        0,
    )
    .unwrap();
    let duplicate = SurveyRecipe {
        columns: vec![valid.columns[0].clone(), valid.columns[0].clone()],
        ..valid
    };
    assert!(duplicate.validate(&c).is_err());
    for role in [
        SurveyRole::Exclude,
        SurveyRole::Unresolved,
        SurveyRole::NumericMeasure,
    ] {
        assert!(SurveyRecipe::capture(
            &data(1),
            &[SurveyChoice {
                column: ColumnId(1),
                role
            }],
            &[],
            SurveyPresentation::default(),
            "Survey",
            &c,
            &[],
            0
        )
        .is_err());
    }
}
#[test]
fn recipe_size_budget_and_file_conflicts_are_guarded() {
    use tessera::{
        data::MAX_OWNED_BYTES,
        survey::recipe::{load_with_budget, save_with_budget, MAX_RECIPE_BYTES},
    };
    let c = JobControl::default();
    let d = data(1);
    let recipe = SurveyRecipe::capture(
        &d,
        &[SurveyChoice {
            column: ColumnId(1),
            role: SurveyRole::Categories,
        }],
        &[],
        SurveyPresentation::default(),
        "Survey",
        &c,
        &[],
        0,
    )
    .unwrap();
    assert!(encode_with_budget(&recipe, &c, &[], MAX_OWNED_BYTES).is_err());
    assert!(decode_with_budget(&vec![b' '; MAX_RECIPE_BYTES + 1], &c, &[], 0).is_err());
    let directory = std::fs::canonicalize(std::env::temp_dir())
        .unwrap()
        .join(format!("tessera-recipe-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("survey.tessera-survey");
    let saved = save_with_budget(&path, &recipe, None, &c, &[], 0).unwrap();
    assert!(saved.committed_stamp.is_some());
    let (loaded, stamp) = load_with_budget(&path, &c, &[], 0).unwrap();
    assert_eq!(loaded, recipe);
    assert!(save_with_budget(&path, &recipe, None, &c, &[], 0).is_err());
    std::fs::write(&path, b"changed").unwrap();
    assert!(save_with_budget(&path, &recipe, Some(&stamp), &c, &[], 0).is_err());
    std::fs::remove_dir_all(directory).unwrap();
}
#[test]
fn replay_preserves_source_and_never_cleans_derived_output_again() {
    use tessera::{
        project::Project,
        survey::{cleaning::CleaningSpec, recipe::replay_with_budget},
    };
    let c = JobControl::default();
    let d = Dataset::new(vec![Column::text(
        ColumnId(1),
        "Choice",
        vec![Some(" Yes ".into()), Some("Yes".into())],
    )
    .unwrap()])
    .unwrap();
    let recipe = SurveyRecipe::capture(
        &d,
        &[SurveyChoice {
            column: ColumnId(1),
            role: SurveyRole::Categories,
        }],
        &[(
            ColumnId(1),
            CleaningSpec {
                trim_whitespace: true,
                rules: vec![],
            },
        )],
        SurveyPresentation::default(),
        "Survey",
        &c,
        &[],
        0,
    )
    .unwrap();
    let source = Project::new("Source", d).unwrap();
    let output = replay_with_budget(&source, &recipe, &c, &[], 0).unwrap();
    assert_eq!(source.dataset().columns().len(), 1);
    assert_eq!(
        output.dataset().columns()[0].text_value(0).unwrap(),
        Some(" Yes ")
    );
    assert_eq!(
        output.dataset().columns()[1].text_value(0).unwrap(),
        Some("Yes")
    );
    assert!(replay_with_budget(&output, &recipe, &c, &[], 0).is_err());
    assert!(!output.runs().is_empty());
}
#[test]
fn valid_envelopes_refuse_unknown_versions_fields_roles_and_nested_rules() {
    let c = JobControl::default();
    let d = data(1);
    let recipe = SurveyRecipe::capture(
        &d,
        &[SurveyChoice {
            column: ColumnId(1),
            role: SurveyRole::Categories,
        }],
        &[],
        SurveyPresentation::default(),
        "Survey",
        &c,
        &[],
        0,
    )
    .unwrap();
    let good: serde_json::Value =
        serde_json::from_slice(&encode_with_budget(&recipe, &c, &[], 0).unwrap()).unwrap();
    for case in 0..5 {
        let mut bad = good.clone();
        match case {
            0 => bad["version"] = 2.into(),
            1 => bad["recipe"]["execute"] = "script".into(),
            2 => bad["recipe"]["columns"][0]["role"] = "unresolved".into(),
            3 => bad["recipe"]["columns"][0]["kind"] = "script".into(),
            _ => {
                bad["recipe"]["columns"][0]["cleaning"] =
                    serde_json::json!({"trim_whitespace":true,"rules":[],"execute":"script"})
            }
        };
        assert!(decode_with_budget(&serde_json::to_vec(&bad).unwrap(), &c, &[], 0).is_err());
    }
}
#[test]
fn cancellation_rejects_bind_encode_capture_and_file_operations() {
    use tessera::survey::recipe::{load_with_budget, save_with_budget};
    let d = data(1);
    let c = JobControl::default();
    let choices = [SurveyChoice {
        column: ColumnId(1),
        role: SurveyRole::Categories,
    }];
    let recipe = SurveyRecipe::capture(
        &d,
        &choices,
        &[],
        SurveyPresentation::default(),
        "Survey",
        &c,
        &[],
        0,
    )
    .unwrap();
    c.cancel();
    assert!(recipe.bind(&d, &c).is_err());
    assert!(encode_with_budget(&recipe, &c, &[], 0).is_err());
    assert!(SurveyRecipe::capture(
        &d,
        &choices,
        &[],
        SurveyPresentation::default(),
        "Survey",
        &c,
        &[],
        0
    )
    .is_err());
    let path = std::path::Path::new("/nonexistent/survey.tessera-survey");
    assert!(save_with_budget(path, &recipe, None, &c, &[], 0).is_err());
    assert!(load_with_budget(path, &c, &[], 0).is_err());
}
#[test]
fn numeric_cleaning_is_explicit_and_types_must_match() {
    use tessera::{data::Cell, preparation::RecodeRule, survey::cleaning::CleaningSpec};
    let c = JobControl::default();
    let d = Dataset::new(vec![
        Column::numeric(ColumnId(1), "Score", vec![Some(99.)]).unwrap()
    ])
    .unwrap();
    let choices = [SurveyChoice {
        column: ColumnId(1),
        role: SurveyRole::NumericMeasure,
    }];
    let missing = CleaningSpec {
        trim_whitespace: false,
        rules: vec![RecodeRule {
            from: Cell::Number(99.),
            to: Cell::Missing,
        }],
    };
    assert!(SurveyRecipe::capture(
        &d,
        &choices,
        &[(ColumnId(1), missing)],
        SurveyPresentation::default(),
        "Survey",
        &c,
        &[],
        0
    )
    .is_ok());
    for bad in [
        CleaningSpec {
            trim_whitespace: true,
            rules: vec![],
        },
        CleaningSpec {
            trim_whitespace: false,
            rules: vec![RecodeRule {
                from: Cell::Text("99".into()),
                to: Cell::Missing,
            }],
        },
    ] {
        assert!(SurveyRecipe::capture(
            &d,
            &choices,
            &[(ColumnId(1), bad)],
            SurveyPresentation::default(),
            "Survey",
            &c,
            &[],
            0
        )
        .is_err());
    }
}
#[test]
fn long_unicode_header_replay_keeps_signature_and_uses_valid_derived_name() {
    use tessera::{
        project::Project,
        survey::{cleaning::CleaningSpec, recipe::replay_with_budget},
    };
    let name = "£".repeat(510);
    let d = Dataset::new(vec![Column::text(
        ColumnId(1),
        &name,
        vec![Some(" Yes ".into())],
    )
    .unwrap()])
    .unwrap();
    let c = JobControl::default();
    let recipe = SurveyRecipe::capture(
        &d,
        &[SurveyChoice {
            column: ColumnId(1),
            role: SurveyRole::Categories,
        }],
        &[(
            ColumnId(1),
            CleaningSpec {
                trim_whitespace: true,
                rules: vec![],
            },
        )],
        SurveyPresentation::default(),
        "Survey",
        &c,
        &[],
        0,
    )
    .unwrap();
    let output =
        replay_with_budget(&Project::new("Source", d).unwrap(), &recipe, &c, &[], 0).unwrap();
    assert_eq!(recipe.columns[0].name, name);
    assert_eq!(output.dataset().columns()[0].name(), name);
    assert!(output.dataset().columns()[1].name().len() <= 1024);
}
#[test]
fn reviewed_name_is_bounded_and_resolves_collisions() {
    use tessera::survey::cleaning::reviewed_name;
    let name = "£".repeat(510);
    let base = format!("{} (reviewed)", &name[..990]);
    let d = Dataset::new(vec![
        Column::text(ColumnId(1), &name, vec![Some("x".into())]).unwrap(),
        Column::text(ColumnId(2), &base, vec![Some("x".into())]).unwrap(),
        Column::text(ColumnId(3), &format!("{base} 2"), vec![Some("x".into())]).unwrap(),
    ])
    .unwrap();
    let proposed = reviewed_name(&d, ColumnId(1)).unwrap();
    assert_eq!(proposed, format!("{base} 3"));
    assert!(proposed.len() <= 1024);
    assert!(reviewed_name(&d, ColumnId(99)).is_err());
}
