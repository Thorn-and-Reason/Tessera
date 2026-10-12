use tessera::{
    analysis::NumericComparison,
    data::{Cell, Column, ColumnId, ColumnKind, Dataset, MissingRules},
    operation::JobControl,
    preparation::{
        prepare_with_budget, Arithmetic, FilterPredicate, Operand, RecodeRule, Step, Unmatched,
    },
    project::Project,
};
fn sample() -> Project {
    Project::new(
        "Prepare",
        Dataset::new(vec![
            Column::numeric(ColumnId(1), "x", vec![Some(2.), None, Some(2.), Some(1.)]).unwrap(),
            Column::text(
                ColumnId(2),
                "id",
                vec![
                    Some("a".into()),
                    Some("b".into()),
                    Some("c".into()),
                    Some("d".into()),
                ],
            )
            .unwrap(),
        ])
        .unwrap(),
    )
    .unwrap()
}
fn run(p: &Project, s: Step) -> Project {
    prepare_with_budget(p, &s, &JobControl::default(), &[], 0).unwrap()
}
fn compute(op: Arithmetic, n: f64) -> Step {
    Step::Compute {
        name: "y".into(),
        left: Operand::Column(ColumnId(1)),
        operator: op,
        right: Operand::Constant(n),
    }
}
#[test]
fn arithmetic_records_and_preserves_source_missingness() {
    let p = sample();
    let q = run(&p, compute(Arithmetic::Add, 3.));
    assert_eq!(q.dataset().cell(ColumnId(3), 0).unwrap(), Cell::Number(5.));
    assert_eq!(q.dataset().cell(ColumnId(3), 1).unwrap(), Cell::Missing);
    assert_eq!(p.dataset().column_count(), 2);
    assert_eq!(q.steps().len(), 1);
    assert_eq!(q.next_column_id(), ColumnId(4));
}
#[test]
fn arithmetic_failure_and_cancellation_are_atomic() {
    let p = sample();
    for s in [
        compute(Arithmetic::Divide, 0.),
        compute(Arithmetic::Multiply, f64::MAX),
        compute(Arithmetic::Add, f64::NAN),
    ] {
        assert!(prepare_with_budget(&p, &s, &JobControl::default(), &[], 0).is_err());
    }
    let c = JobControl::default();
    c.cancel();
    assert!(prepare_with_budget(&p, &compute(Arithmetic::Add, 1.), &c, &[], 0).is_err());
    assert!(p.steps().is_empty());
}
#[test]
fn recode_typed_exact_pairs_preserve_missing() {
    let q = run(
        &sample(),
        Step::Recode {
            source: ColumnId(1),
            name: "group".into(),
            kind: ColumnKind::Text,
            rules: vec![RecodeRule {
                from: Cell::Number(2.),
                to: Cell::Text("two".into()),
            }],
            unmatched: Unmatched::Missing,
        },
    );
    assert_eq!(
        q.dataset().cell(ColumnId(3), 0).unwrap(),
        Cell::Text("two".into())
    );
    assert_eq!(q.dataset().cell(ColumnId(3), 1).unwrap(), Cell::Missing);
    assert_eq!(q.dataset().cell(ColumnId(3), 3).unwrap(), Cell::Missing);
}
#[test]
fn recode_collisions_and_type_mismatches_are_rejected() {
    let p = sample();
    let rules = vec![
        RecodeRule {
            from: Cell::Number(2.),
            to: Cell::Text("two".into())
        };
        2
    ];
    let s = Step::Recode {
        source: ColumnId(1),
        name: "group".into(),
        kind: ColumnKind::Text,
        rules,
        unmatched: Unmatched::Missing,
    };
    assert!(prepare_with_budget(&p, &s, &JobControl::default(), &[], 0).is_err());
}
#[test]
fn stable_sort_missing_last_moves_whole_rows() {
    let q = run(
        &sample(),
        Step::Sort {
            source: ColumnId(1),
            descending: false,
        },
    );
    for (i, v) in ["d", "a", "c", "b"].iter().enumerate() {
        assert_eq!(
            q.dataset().cell(ColumnId(2), i).unwrap(),
            Cell::Text((*v).into())
        );
    }
    let q = run(
        &sample(),
        Step::Sort {
            source: ColumnId(1),
            descending: true,
        },
    );
    assert_eq!(
        q.dataset().cell(ColumnId(2), 3).unwrap(),
        Cell::Text("b".into())
    );
}
#[test]
fn filter_keeps_matching_rows_and_zero_matches_are_valid() {
    let q = run(
        &sample(),
        Step::Filter {
            source: ColumnId(1),
            predicate: FilterPredicate::Compare {
                comparison: NumericComparison::Greater,
                value: Cell::Number(99.),
            },
        },
    );
    assert_eq!(q.dataset().row_count(), 0);
    assert_eq!(q.dataset().column_count(), 2);
    let q = run(
        &sample(),
        Step::Filter {
            source: ColumnId(1),
            predicate: FilterPredicate::Missing,
        },
    );
    assert_eq!(
        q.dataset().cell(ColumnId(2), 0).unwrap(),
        Cell::Text("b".into())
    );
}
#[test]
fn user_missing_rules_apply_to_compute_and_survive_row_operations() {
    let p = sample();
    let old = p.dataset().column(ColumnId(1)).unwrap();
    let mut m = old.metadata().clone();
    m.missing_rules = MissingRules::Number {
        values: vec![2.],
        ranges: vec![],
    };
    let d = p
        .dataset()
        .with_column_metadata_with_budget(ColumnId(1), m, &[], 0)
        .unwrap();
    let p = p.with_dataset(d).unwrap();
    let q = run(&p, compute(Arithmetic::Add, 1.));
    assert_eq!(q.dataset().cell(ColumnId(3), 0).unwrap(), Cell::Missing);
    let q = run(
        &p,
        Step::Sort {
            source: ColumnId(1),
            descending: false,
        },
    );
    assert_eq!(
        q.dataset().column(ColumnId(1)).unwrap().metadata(),
        p.dataset().column(ColumnId(1)).unwrap().metadata()
    );
}
#[test]
fn budget_failure_preserves_input() {
    let p = sample();
    assert!(prepare_with_budget(
        &p,
        &compute(Arithmetic::Add, 1.),
        &JobControl::default(),
        &[],
        tessera::data::MAX_OWNED_BYTES
    )
    .is_err());
    assert_eq!(p.dataset().column_count(), 2);
}
#[test]
fn restored_data_is_fresh_and_column_ids_never_rewind() {
    let p = sample();
    let snap = p.data_snapshot_with_budget(&[], 0).unwrap();
    let q = run(&p, compute(Arithmetic::Add, 1.));
    let restored = q
        .with_restored_dataset_and_steps(&snap, &JobControl::default(), &[], 0)
        .unwrap();
    assert!(restored.dataset().revision() > q.dataset().revision());
    assert_eq!(restored.next_column_id(), ColumnId(4));
    let r = run(&restored, compute(Arithmetic::Add, 1.));
    assert_eq!(r.dataset().columns()[2].id(), ColumnId(4));
}
#[test]
fn direct_edits_coalesce_and_saved_steps_are_passive() {
    let p = sample();
    let q = p
        .with_dataset(
            p.dataset()
                .edit_cell(ColumnId(1), 0, Cell::Number(5.))
                .unwrap(),
        )
        .unwrap();
    let q = q
        .with_dataset(
            q.dataset()
                .edit_cell(ColumnId(1), 0, Cell::Number(6.))
                .unwrap(),
        )
        .unwrap();
    assert_eq!(q.steps().len(), 1);
    let mut bytes = vec![];
    tessera::project_io::encode(&q, &mut bytes, &JobControl::default()).unwrap();
    let r = tessera::project_io::decode(bytes.as_slice(), &JobControl::default()).unwrap();
    assert_eq!(r.steps(), q.steps());
    assert_eq!(r.next_column_id(), q.next_column_id());
    assert_eq!(r.dataset().cell(ColumnId(1), 0).unwrap(), Cell::Number(6.));
}
#[test]
fn every_structured_recipe_roundtrips_without_execution() {
    for step in [
        compute(Arithmetic::Subtract, 1.),
        compute(Arithmetic::Multiply, 2.),
        compute(Arithmetic::Divide, 2.),
        Step::Recode {
            source: ColumnId(1),
            name: "copy".into(),
            kind: ColumnKind::Number,
            rules: vec![RecodeRule {
                from: Cell::Number(2.),
                to: Cell::Number(8.),
            }],
            unmatched: Unmatched::Preserve,
        },
        Step::Sort {
            source: ColumnId(2),
            descending: true,
        },
        Step::Filter {
            source: ColumnId(2),
            predicate: FilterPredicate::Compare {
                comparison: NumericComparison::NotEqual,
                value: Cell::Text("a".into()),
            },
        },
        Step::Filter {
            source: ColumnId(1),
            predicate: FilterPredicate::Present,
        },
    ] {
        let p = run(&sample(), step);
        let mut bytes = vec![];
        tessera::project_io::encode(&p, &mut bytes, &JobControl::default()).unwrap();
        let r = tessera::project_io::decode(bytes.as_slice(), &JobControl::default()).unwrap();
        assert_eq!(r.steps(), p.steps());
        assert_eq!(r.next_column_id(), p.next_column_id());
        assert_eq!(r.dataset().row_count(), p.dataset().row_count());
    }
}
#[test]
fn malformed_saved_records_and_watermarks_are_rejected() {
    let p = run(&sample(), compute(Arithmetic::Add, 1.));
    let mut bytes = vec![];
    tessera::project_io::encode(&p, &mut bytes, &JobControl::default()).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for (path, bad) in [
        ("/next_column_id", serde_json::json!("3")),
        ("/next_column_id", serde_json::json!("04")),
        ("/preparation/0/source_revision", serde_json::json!("2")),
        ("/preparation/0/result_revision", serde_json::json!("9")),
        (
            "/preparation/0/step/compute/left",
            serde_json::json!({"column":"999"}),
        ),
        (
            "/preparation/0/step/compute/operator",
            serde_json::json!("eval"),
        ),
        ("/preparation/0/step/compute/name", serde_json::json!("")),
        (
            "/preparation/0/step/compute/left",
            serde_json::json!({"constant":null}),
        ),
    ] {
        let mut v = value.clone();
        *v.pointer_mut(path).unwrap() = bad;
        assert!(
            tessera::project_io::decode(
                serde_json::to_vec(&v).unwrap().as_slice(),
                &JobControl::default()
            )
            .is_err(),
            "{path}"
        );
    }
    for field in ["next_column_id", "preparation"] {
        let mut v = value.clone();
        v.as_object_mut().unwrap().remove(field);
        assert!(tessera::project_io::decode(
            serde_json::to_vec(&v).unwrap().as_slice(),
            &JobControl::default()
        )
        .is_err());
    }
}
#[test]
fn recode_preserves_unmatched_exact_text_and_signed_zero_matches() {
    let p = sample();
    let q = run(
        &p,
        Step::Recode {
            source: ColumnId(2),
            name: "new".into(),
            kind: ColumnKind::Text,
            rules: vec![RecodeRule {
                from: Cell::Text("a".into()),
                to: Cell::Text("".into()),
            }],
            unmatched: Unmatched::Preserve,
        },
    );
    assert_eq!(
        q.dataset().cell(ColumnId(3), 0).unwrap(),
        Cell::Text("".into())
    );
    assert_eq!(
        q.dataset().cell(ColumnId(3), 1).unwrap(),
        Cell::Text("b".into())
    );
    let p = Project::new(
        "Zero",
        Dataset::new(vec![Column::numeric(
            ColumnId(1),
            "x",
            vec![Some(-0.), Some(0.)],
        )
        .unwrap()])
        .unwrap(),
    )
    .unwrap();
    let q = run(
        &p,
        Step::Recode {
            source: ColumnId(1),
            name: "z".into(),
            kind: ColumnKind::Number,
            rules: vec![RecodeRule {
                from: Cell::Number(0.),
                to: Cell::Number(7.),
            }],
            unmatched: Unmatched::Preserve,
        },
    );
    assert_eq!(q.dataset().cell(ColumnId(2), 0).unwrap(), Cell::Number(7.));
}
#[test]
fn invalid_preparation_requests_fail_before_data_changes() {
    let p = sample();
    for step in [
        Step::Compute {
            name: "x".into(),
            left: Operand::Column(ColumnId(1)),
            operator: Arithmetic::Add,
            right: Operand::Constant(1.),
        },
        Step::Compute {
            name: "y".into(),
            left: Operand::Column(ColumnId(2)),
            operator: Arithmetic::Add,
            right: Operand::Constant(1.),
        },
        Step::Filter {
            source: ColumnId(1),
            predicate: FilterPredicate::Compare {
                comparison: NumericComparison::Equal,
                value: Cell::Text("2".into()),
            },
        },
        Step::Filter {
            source: ColumnId(1),
            predicate: FilterPredicate::Compare {
                comparison: NumericComparison::Equal,
                value: Cell::Missing,
            },
        },
        Step::Sort {
            source: ColumnId(0),
            descending: false,
        },
        Step::DatasetEdit,
        Step::Recode {
            source: ColumnId(1),
            name: "new".into(),
            kind: ColumnKind::Text,
            rules: vec![RecodeRule {
                from: Cell::Number(2.),
                to: Cell::Text("two".into()),
            }],
            unmatched: Unmatched::Preserve,
        },
    ] {
        assert!(prepare_with_budget(&p, &step, &JobControl::default(), &[], 0).is_err());
    }
}
#[test]
fn both_numeric_column_operands_and_all_comparisons_work() {
    let p = sample();
    let q = run(
        &p,
        Step::Compute {
            name: "twice".into(),
            left: Operand::Column(ColumnId(1)),
            operator: Arithmetic::Add,
            right: Operand::Column(ColumnId(1)),
        },
    );
    assert_eq!(q.dataset().cell(ColumnId(3), 0).unwrap(), Cell::Number(4.));
    for (comparison, n) in [
        (NumericComparison::Equal, 2),
        (NumericComparison::NotEqual, 1),
        (NumericComparison::Less, 1),
        (NumericComparison::LessOrEqual, 3),
        (NumericComparison::Greater, 0),
        (NumericComparison::GreaterOrEqual, 2),
    ] {
        let q = run(
            &p,
            Step::Filter {
                source: ColumnId(1),
                predicate: FilterPredicate::Compare {
                    comparison,
                    value: Cell::Number(2.),
                },
            },
        );
        assert_eq!(q.dataset().row_count(), n);
    }
}
#[test]
fn cancelled_restore_and_budget_refusal_leave_current_data_unchanged() {
    let p = sample();
    let snap = p.data_snapshot_with_budget(&[], 0).unwrap();
    let q = run(&p, compute(Arithmetic::Add, 1.));
    let control = JobControl::default();
    control.cancel();
    assert!(q
        .with_restored_dataset_and_steps(&snap, &control, &[], 0)
        .is_err());
    assert!(q
        .with_restored_dataset_and_steps(
            &snap,
            &JobControl::default(),
            &[],
            tessera::data::MAX_OWNED_BYTES
        )
        .is_err());
    assert_eq!(q.dataset().column_count(), 3);
    assert_eq!(q.next_column_id(), ColumnId(4));
}
#[test]
fn identity_exhaustion_and_record_limit_fail_without_adoption() {
    let p = Project::new(
        "Last ID",
        Dataset::new(vec![Column::numeric(
            ColumnId(u64::MAX - 1),
            "x",
            vec![Some(1.)],
        )
        .unwrap()])
        .unwrap(),
    )
    .unwrap();
    assert!(prepare_with_budget(
        &p,
        &Step::Compute {
            name: "y".into(),
            left: Operand::Column(ColumnId(u64::MAX - 1)),
            operator: Arithmetic::Add,
            right: Operand::Constant(1.)
        },
        &JobControl::default(),
        &[],
        0
    )
    .is_err());
    let mut p = sample();
    for _ in 0..tessera::preparation::MAX_STEPS {
        p = run(
            &p,
            Step::Sort {
                source: ColumnId(1),
                descending: false,
            },
        );
    }
    assert!(prepare_with_budget(
        &p,
        &Step::Sort {
            source: ColumnId(1),
            descending: false
        },
        &JobControl::default(),
        &[],
        0
    )
    .is_err());
    assert_eq!(p.steps().len(), tessera::preparation::MAX_STEPS);
}
#[test]
fn public_dataset_replacement_cannot_reuse_a_branch_column_identity() {
    let p = sample();
    let old = p.data_snapshot_with_budget(&[], 0).unwrap();
    let q = run(&p, compute(Arithmetic::Add, 1.));
    let q = q
        .with_restored_dataset_and_steps(&old, &JobControl::default(), &[], 0)
        .unwrap();
    let d = q
        .dataset()
        .append_column(Column::numeric(ColumnId(3), "reused", vec![None; 4]).unwrap())
        .unwrap();
    assert!(q.with_dataset(d).is_err());
    let d = q
        .dataset()
        .append_column(Column::numeric(q.next_column_id(), "fresh", vec![None; 4]).unwrap())
        .unwrap();
    assert!(q.with_dataset(d).is_ok());
}

fn domain_project() -> Project {
    let a = Column::numeric(
        ColumnId(1),
        "a",
        vec![Some(1.), None, Some(99.), Some(f64::MAX), Some(f64::MAX)],
    )
    .unwrap();
    let mut metadata = a.metadata().clone();
    metadata.missing_rules = MissingRules::Number {
        values: vec![99.],
        ranges: vec![],
    };
    Project::new(
        "Domain",
        Dataset::new(vec![
            a.with_metadata(metadata).unwrap(),
            Column::numeric(
                ColumnId(2),
                "b",
                vec![
                    Some(3.),
                    Some(2.),
                    Some(2.),
                    Some(f64::MAX),
                    Some(-f64::MAX),
                ],
            )
            .unwrap(),
            Column::numeric(
                ColumnId(3),
                "c",
                vec![Some(5.), Some(2.), Some(2.), Some(f64::MAX), Some(0.)],
            )
            .unwrap(),
        ])
        .unwrap(),
    )
    .unwrap()
}
fn domain_step() -> Step {
    Step::Mean {
        name: "domain".into(),
        columns: vec![ColumnId(1), ColumnId(2), ColumnId(3)],
    }
}
#[test]
fn domain_average_requires_complete_answers_and_is_overflow_safe() {
    let p = domain_project();
    let q = run(&p, domain_step());
    for (row, expected) in [
        Cell::Number(3.),
        Cell::Missing,
        Cell::Missing,
        Cell::Number(f64::MAX),
        Cell::Number(0.),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(q.dataset().cell(ColumnId(4), row).unwrap(), expected);
    }
    assert_eq!(
        q.dataset()
            .column(ColumnId(4))
            .unwrap()
            .metadata()
            .measurement,
        tessera::data::Measurement::Scale
    );
    assert_eq!(p.dataset().column_count(), 3);
    let mut bytes = vec![];
    tessera::project_io::encode(&q, &mut bytes, &JobControl::default()).unwrap();
    let saved = tessera::project_io::decode(bytes.as_slice(), &JobControl::default()).unwrap();
    assert_eq!(saved.steps(), q.steps());
    let replay = run(&p, saved.steps()[0].step.clone());
    assert_eq!(
        replay.dataset().cell(ColumnId(4), 0).unwrap(),
        Cell::Number(3.)
    );
    let snapshot = p.data_snapshot_with_budget(&[], 0).unwrap();
    let restored = q
        .with_restored_dataset_and_steps(&snapshot, &JobControl::default(), &[], 0)
        .unwrap();
    assert_eq!(restored.dataset().column_count(), 3);
    assert!(restored.steps().is_empty());
}
#[test]
fn invalid_domain_requests_and_budget_fail_atomically() {
    let p = domain_project();
    for columns in [
        vec![],
        vec![ColumnId(1)],
        vec![ColumnId(1); 2],
        vec![ColumnId(1); 65],
        vec![ColumnId(1), ColumnId(99)],
        vec![ColumnId(0), ColumnId(1)],
    ] {
        assert!(prepare_with_budget(
            &p,
            &Step::Mean {
                name: "domain".into(),
                columns
            },
            &JobControl::default(),
            &[],
            0
        )
        .is_err());
    }
    assert!(prepare_with_budget(
        &sample(),
        &Step::Mean {
            name: "domain".into(),
            columns: vec![ColumnId(1), ColumnId(2)]
        },
        &JobControl::default(),
        &[],
        0
    )
    .is_err());
    assert!(prepare_with_budget(
        &p,
        &domain_step(),
        &JobControl::default(),
        &[],
        tessera::data::MAX_OWNED_BYTES
    )
    .is_err());
    assert!(p.steps().is_empty());
    assert_eq!(p.dataset().column_count(), 3);
}

#[test]
fn domain_name_limits_and_cancellation_preserve_source() {
    let p = domain_project();
    for name in [String::new(), "a".into(), "x".repeat(1025)] {
        let step = Step::Mean {
            name,
            columns: vec![ColumnId(1), ColumnId(2)],
        };
        assert!(prepare_with_budget(&p, &step, &JobControl::default(), &[], 0).is_err());
    }
    let control = JobControl::default();
    control.cancel();
    assert!(prepare_with_budget(&p, &domain_step(), &control, &[], 0).is_err());
    assert_eq!(p.dataset().column_count(), 3);
}

#[test]
fn domain_average_accepts_64_distinct_items() {
    let p = Project::new(
        "Wide",
        Dataset::new(
            (1..=64)
                .map(|id| {
                    Column::numeric(ColumnId(id), &format!("item{id}"), vec![Some(2.)]).unwrap()
                })
                .collect(),
        )
        .unwrap(),
    )
    .unwrap();
    let q = run(
        &p,
        Step::Mean {
            name: "domain".into(),
            columns: (1..=64).map(ColumnId).collect(),
        },
    );
    assert_eq!(q.dataset().cell(ColumnId(65), 0).unwrap(), Cell::Number(2.));
}
