use super::*;
fn identity(r: ComputedAnova) {
    assert_eq!(
        decode(&encode(&r), MAX_OWNED_BYTES, &JobControl::default()).unwrap(),
        r
    );
}
fn groups(v: &[&str]) -> Vec<Option<String>> {
    v.iter().map(|v| Some(v.to_string())).collect()
}
#[test]
fn retained_unavailable_designs_zero_error_and_unbalanced_fits() {
    for (values, group, task) in [
        (vec![], vec![], None),
        (vec![Some(1.), Some(2.)], groups(&["a", "a"]), None),
        (vec![Some(1.), Some(2.)], groups(&["a", "b"]), None),
        (
            vec![Some(1.), Some(1.), Some(3.), Some(3.)],
            groups(&["a", "a", "b", "b"]),
            None,
        ),
        (
            vec![Some(1.), Some(2.), Some(3.), Some(4.)],
            groups(&["a", "a", "b", "b"]),
            Some(groups(&["x", "x", "y", "y"])),
        ),
        (
            vec![
                Some(1.),
                Some(2.),
                Some(3.),
                Some(4.),
                Some(5.),
                Some(6.),
                Some(8.),
                Some(9.),
                Some(11.),
            ],
            groups(&["a", "a", "a", "a", "b", "b", "b", "b", "b"]),
            Some(groups(&["x", "x", "y", "y", "x", "x", "y", "y", "y"])),
        ),
    ] {
        identity(captures::independent_rows(values, group, task));
    }
}
#[test]
fn carried_positive_publication_extremes_and_structural_gg() {
    for design in 0..4 {
        for scale in [1e160, 1e-170] {
            identity(captures::capture(design, scale, false));
        }
    }
    for profiles in [
        vec![vec![1., 3.], vec![2., 5.], vec![4., 5.]],
        vec![vec![1., 3.], vec![2., 4.], vec![3., 5.]],
        vec![vec![1., 3.]],
        vec![],
    ] {
        identity(captures::repeated_rows(&profiles, &[2]));
    }
    identity(captures::repeated_rows(
        &[vec![1., 2., 4.], vec![3., 5., 7.]],
        &[3],
    ));
    identity(captures::repeated_rows(
        &[vec![1., 2., 4., 7.], vec![3., 5., 8., 10.]],
        &[2, 2],
    ));
}
#[test]
fn absent_fixed_slices_keep_all_family_members_and_exact_text() {
    let d = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Y",
            (0..12).map(|i| Some((i * i % 17) as f64)).collect(),
        )
        .unwrap(),
        Column::text(
            ColumnId(2),
            "group",
            (0..12)
                .map(|i| Some(if i % 2 == 0 { " a" } else { "a " }.into()))
                .collect(),
        )
        .unwrap(),
        Column::text(
            ColumnId(3),
            "task",
            (0..12)
                .map(|i| Some(if i % 4 < 2 { "é" } else { "e\u{301}" }.into()))
                .collect(),
        )
        .unwrap(),
    ])
    .unwrap();
    let request = AnovaRequest {
        design: AnovaDesign::Factorial {
            outcome: ColumnId(1),
            factors: vec![ColumnId(2), ColumnId(3)],
        },
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
        planned_pairs: vec![AnovaPairRequest {
            factor: 0,
            first: Cell::Text(" a".into()),
            second: Cell::Text("a ".into()),
            scope: AnovaScope::Fixed(vec![Cell::Text("missing".into())]),
        }],
        all_pairs: vec![AnovaFamilyRequest {
            factor: 0,
            scope: AnovaScope::Fixed(vec![Cell::Text("missing".into())]),
        }],
    };
    let r = captures::assemble(&d, request);
    for f in &r.families {
        assert_eq!(f.m, 1);
        assert!(matches!(
            f.members[0].state,
            AnovaComparisonState::AbsentRequestedLevel(_)
        ));
        assert_eq!(
            f.adjustment_failure.as_ref().unwrap().kind,
            AnovaFailureKind::IncompleteFamily
        );
    }
    identity(r);
}

#[test]
fn subnormal_publication_capture_must_roundtrip() {
    // Observed accepted-engine/validator boundary regression; root owns its fix.
    identity(captures::capture(0, 1e-160, false));
}
