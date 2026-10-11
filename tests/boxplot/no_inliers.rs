use super::*;

fn no_inliers() -> ComputedBoxplot {
    let a = f64::from_bits(1);
    compute(&dataset(vec![Some(-a), Some(a)]), &request(None))
}

#[test]
fn rounded_zero_quartiles_preserve_finite_all_outlier_summary() {
    let result = no_inliers();
    let g = &result.groups[0];
    assert_eq!((g.rows, g.n, g.outlier_rows, g.outlier_n), (2, 2, 2, 2));
    assert_eq!(
        [&g.minimum, &g.q1, &g.median, &g.q3, &g.maximum].map(|e| e.value.unwrap().to_bits()),
        [(1u64 << 63) | 1, 1u64 << 63, 0, 0, 1]
    );
    assert!(g
        .whiskers
        .iter()
        .all(|e| e.value.is_none() && e.reason.as_ref().is_some_and(|s| !s.trim().is_empty())));
    assert_eq!(
        result
            .points
            .iter()
            .map(|p| (p.row, p.value.to_bits(), p.frequency))
            .collect::<Vec<_>>(),
        [(0, (1u64 << 63) | 1, 1), (1, 1, 1)]
    );
    assert!(result.empty_reason.is_none() && !result.reduced);
    assert!(result.owned_bytes().unwrap() > 0);
}

#[test]
fn mixed_grouped_no_inlier_and_ordinary_groups_keep_all_identities() {
    let a = f64::from_bits(1);
    let d = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Value",
            vec![
                Some(-a),
                Some(0.),
                Some(a),
                Some(2.),
                Some(1.),
                Some(3.),
                Some(100.),
            ],
        )
        .unwrap(),
        Column::text(
            ColumnId(2),
            "Group",
            ["A", "B", "A", "B", "B", "B", "B"]
                .map(|v| Some(v.into()))
                .to_vec(),
        )
        .unwrap(),
    ])
    .unwrap();
    let result = compute(&d, &request(Some(ColumnId(2))));
    assert_eq!(result.groups.len(), 2);
    let g = &result.groups[0];
    assert_eq!(
        (g.value.as_ref(), g.rows, g.n, g.outlier_rows, g.outlier_n),
        (Some(&Cell::Text("A".into())), 2, 2, 2, 2)
    );
    assert!(g
        .whiskers
        .iter()
        .all(|e| e.value.is_none() && e.reason.is_some()));
    assert_eq!(
        [
            g.q1.value.unwrap().to_bits(),
            g.median.value.unwrap().to_bits(),
            g.q3.value.unwrap().to_bits()
        ],
        [1u64 << 63, 0, 0]
    );
    let g = &result.groups[1];
    assert_eq!((g.rows, g.n, g.outlier_rows, g.outlier_n), (5, 5, 1, 1));
    assert_eq!(g.whiskers.clone().map(|e| e.value), [Some(0.), Some(3.)]);
    assert_eq!(
        result
            .points
            .iter()
            .map(|p| (p.group_index, p.row, p.value.to_bits(), p.frequency))
            .collect::<Vec<_>>(),
        [
            (0, 0, (1u64 << 63) | 1, 1),
            (0, 2, 1, 1),
            (1, 6, 100f64.to_bits(), 1)
        ]
    );
}

#[test]
fn all_outlier_relaxation_rejects_malformed_summaries_and_points() {
    let base = no_inliers();
    for mode in 0..11 {
        let mut bad = base.clone();
        match mode {
            0 => {
                bad.groups[0].whiskers[0] = Estimate {
                    value: Some(0.),
                    reason: None,
                }
            }
            1 => bad.groups[0].outlier_rows = 1,
            2 => bad.groups[0].outlier_n = 1,
            3 => {
                bad.groups[0].q1 = Estimate {
                    value: Some(f64::from_bits(1)),
                    reason: None,
                }
            }
            4 => {
                bad.groups[0].minimum = Estimate {
                    value: None,
                    reason: Some("Unavailable".into()),
                }
            }
            5 => bad.points[0].value = 0.,
            6 => bad.points[1].value = -f64::from_bits(1),
            7 => bad.points[0].value = f64::from_bits(1),
            8 => bad.points[1].row = 0,
            9 => bad.points[1].row = 2,
            _ => bad.groups[0].whiskers[0].reason = Some(" ".into()),
        }
        assert!(
            bad.validate().is_err(),
            "accepted malformed all-outlier mode {mode}"
        );
    }
    let mut ordinary = compute(
        &dataset([0., 1., 2., 3., 100.].map(Some).to_vec()),
        &request(None),
    );
    let absent = Estimate {
        value: None,
        reason: Some("No observed in-fence values".into()),
    };
    ordinary.groups[0].whiskers = [absent.clone(), absent];
    assert!(ordinary.validate().is_err());
    ordinary.groups[0].outlier_rows = 5;
    ordinary.groups[0].outlier_n = 5;
    assert!(ordinary.validate().is_err());
}

#[test]
fn nearby_subnormal_ties_normal_and_reflected_controls_keep_observed_whiskers() {
    let a = f64::from_bits(1);
    for values in [
        vec![-2. * a, 2. * a],
        vec![-a, -a, a, a],
        vec![a, 2. * a],
        vec![-2. * a, -a],
        vec![-1., 1.],
    ] {
        let result = compute(
            &dataset(values.iter().copied().map(Some).collect()),
            &request(None),
        );
        let g = &result.groups[0];
        assert_eq!(
            (g.rows, g.n, g.outlier_rows, g.outlier_n),
            (values.len(), values.len() as u64, 0, 0)
        );
        assert_eq!(
            g.whiskers.clone().map(|e| e.value.unwrap().to_bits()),
            [values[0].to_bits(), values[values.len() - 1].to_bits()]
        );
    }
    let with_zero = compute(&dataset(vec![Some(-a), Some(0.), Some(a)]), &request(None));
    let g = &with_zero.groups[0];
    assert_eq!((g.rows, g.n, g.outlier_rows, g.outlier_n), (3, 3, 2, 2));
    assert_eq!(
        g.whiskers.clone().map(|e| e.value.unwrap().to_bits()),
        [0, 0]
    );
    let reflected = compute(&dataset(vec![Some(a), Some(-a)]), &request(None));
    assert_eq!(
        reflected
            .points
            .iter()
            .map(|p| (p.row, p.value.to_bits()))
            .collect::<Vec<_>>(),
        [(1, (1u64 << 63) | 1), (0, 1)]
    );
}
