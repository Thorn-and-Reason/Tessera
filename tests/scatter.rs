use tessera::{analysis::*, data::*, error::AppError, operation::JobControl};
fn request() -> ScatterRequest {
    ScatterRequest {
        x: ColumnId(1),
        y: ColumnId(2),
        filter: RowFilter::default(),
        frequency_weight: None,
    }
}
fn dataset(x: Vec<Option<f64>>, y: Vec<Option<f64>>) -> Dataset {
    Dataset::new(vec![
        Column::numeric(ColumnId(1), "X", x).unwrap(),
        Column::numeric(ColumnId(2), "Y", y).unwrap(),
    ])
    .unwrap()
}
fn compute(d: &Dataset, r: &ScatterRequest) -> ComputedScatter {
    let result =
        scatter_with_budget(d, r, &JobControl::default(), &[], r.heap_bytes().unwrap()).unwrap();
    result.validate().unwrap();
    result
}
#[test]
fn independently_selected_ordinals_keep_exact_paired_source_identity() {
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/scatter-ordinal-reference.json")).unwrap();
    for case in reference["cases"].as_array().unwrap() {
        let m = case["eligible_rows"].as_u64().unwrap() as usize;
        // Every odd source row is role-missing; reduction must select eligible ordinals.
        let x = (0..2 * m)
            .map(|row| row.is_multiple_of(2).then_some(row as f64))
            .collect();
        let y = (0..2 * m).map(|row| Some(-(row as f64))).collect();
        let result = compute(&dataset(x, y), &request());
        assert_eq!(
            (result.valid_rows, result.missing_rows, result.n),
            (m, m, m as u64)
        );
        assert_eq!(
            result.points.len(),
            case["shown"].as_u64().unwrap() as usize
        );
        for (point, ordinal) in result
            .points
            .iter()
            .zip(case["indices"].as_array().unwrap())
        {
            let row = 2 * ordinal.as_u64().unwrap() as usize;
            assert_eq!(
                (point.row, point.x, point.y, point.frequency),
                (row, row as f64, -(row as f64), 1)
            );
        }
        assert_eq!(result.reduced, m > 2000);
    }
}
#[test]
fn omitted_sole_extreme_remains_in_full_ranges() {
    let mut x = (0..2001).map(|i| Some(i as f64)).collect::<Vec<_>>();
    let mut y = x.clone();
    // The exact integer formula omits ordinal 1999 for M=2001, K=2000.
    x[1999] = Some(-f64::MAX);
    y[1999] = Some(f64::MAX);
    let result = compute(&dataset(x, y), &request());
    assert_eq!(
        (result.valid_rows, result.n, result.points.len()),
        (2001, 2001, 2000)
    );
    assert!(!result.points.iter().any(|p| p.row == 1999));
    assert_eq!(
        [result.ranges[0][0].value, result.ranges[0][1].value],
        [Some(-f64::MAX), Some(2000.)]
    );
    assert_eq!(
        [result.ranges[1][0].value, result.ranges[1][1].value],
        [Some(0.), Some(f64::MAX)]
    );
}
#[test]
fn missing_roles_partition_once_and_frequency_means_replication() {
    let d = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "X",
            vec![Some(1.), None, Some(2.), None, Some(4.), None, Some(5.)],
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "Y",
            vec![Some(3.), Some(3.), None, None, Some(9.), Some(7.), Some(8.)],
        )
        .unwrap(),
        Column::numeric(
            ColumnId(3),
            "W",
            vec![
                Some(2.),
                Some(3.),
                Some(4.),
                Some(5.),
                Some(6.),
                Some(0.),
                None,
            ],
        )
        .unwrap(),
    ])
    .unwrap();
    let mut r = request();
    r.frequency_weight = Some(ColumnId(3));
    let result = compute(&d, &r);
    assert_eq!(
        (
            result.valid_rows,
            result.n,
            result.missing_rows,
            result.missing_n
        ),
        (2, 8, 3, 12)
    );
    assert_eq!(
        result
            .points
            .iter()
            .map(|p| (p.row, p.frequency))
            .collect::<Vec<_>>(),
        [(0, 2), (4, 6)]
    );
    assert_eq!(
        (
            result.diagnostics.zero_weight_rows,
            result.diagnostics.missing_weight_rows,
            result.diagnostics.complete_case_rows
        ),
        (1, 1, 0)
    );
    let replicated = compute(
        &dataset(
            vec![Some(1.); 2]
                .into_iter()
                .chain(vec![Some(4.); 6])
                .collect(),
            vec![Some(3.); 2]
                .into_iter()
                .chain(vec![Some(9.); 6])
                .collect(),
        ),
        &request(),
    );
    assert_eq!(result.ranges, replicated.ranges);
    assert_eq!(result.n, replicated.n);
}
#[test]
fn empty_constant_and_extreme_axes_keep_numeric_tables() {
    for (x, y) in [(vec![], vec![]), (vec![None], vec![Some(1.)])] {
        let result = compute(&dataset(x, y), &request());
        assert!(
            result.points.is_empty() && result.ranges.iter().flatten().all(|e| e.value.is_none())
        );
    }
    let result = compute(
        &dataset(
            vec![Some(5.); 3],
            vec![Some(-f64::MAX), Some(0.), Some(f64::MAX)],
        ),
        &request(),
    );
    assert_eq!(
        [result.ranges[0][0].value, result.ranges[0][1].value],
        [Some(5.), Some(5.)]
    );
    assert_eq!(result.points.len(), 3);
}
#[test]
fn invalid_roles_weight_order_cancellation_and_retained_budget() {
    let d = dataset(vec![None, Some(1.)], vec![None, Some(2.)]);
    let mut r = request();
    r.y = r.x;
    assert!(scatter_with_budget(&d, &r, &JobControl::default(), &[], 0).is_err());
    let control = JobControl::default();
    control.cancel();
    assert!(matches!(
        scatter_with_budget(&d, &request(), &control, &[], 0),
        Err(AppError::Cancelled)
    ));
    let retained = Dataset::new(vec![Column::text(
        ColumnId(9),
        "Old",
        vec![Some("r".repeat(2_000_000))],
    )
    .unwrap()])
    .unwrap();
    assert!(matches!(
        scatter_with_budget(
            &d,
            &request(),
            &JobControl::default(),
            &[&retained],
            MAX_OWNED_BYTES - 1_000_000
        ),
        Err(AppError::SizeLimit { .. })
    ));
    let bad = Dataset::new(vec![
        Column::numeric(ColumnId(1), "X", vec![None, Some(1.)]).unwrap(),
        Column::numeric(ColumnId(2), "Y", vec![None, Some(2.)]).unwrap(),
        Column::numeric(ColumnId(3), "W", vec![Some(-1.), Some(2.)]).unwrap(),
    ])
    .unwrap();
    let mut r = request();
    r.frequency_weight = Some(ColumnId(3));
    assert!(scatter_with_budget(&bad, &r, &JobControl::default(), &[], 0).is_err());
    r.filter.predicates.push(RowPredicate::Missing {
        column: ColumnId(1),
        missing: false,
    });
    assert_eq!(compute(&bad, &r).n, 2);
}
#[test]
fn caller_extra_is_charged_once_and_output_growth_is_precharged() {
    let d = dataset(vec![Some(0.), Some(4.)], vec![Some(5.), Some(7.)]);
    let r = request();
    let live = Budget::dataset_bytes(&[&d], 0).unwrap();
    let result = scatter_with_budget(
        &d,
        &r,
        &JobControl::default(),
        &[],
        MAX_OWNED_BYTES - live - 16_384,
    )
    .unwrap();
    result.validate().unwrap();
    let extra = MAX_OWNED_BYTES - live - r.validation_scratch_bytes().unwrap();
    assert!(matches!(
        scatter_with_budget(&d, &r, &JobControl::default(), &[], extra),
        Err(AppError::SizeLimit { .. })
    ));
}
#[test]
fn full_frequency_boundary_and_captured_data_remain_exact() {
    let d = Dataset::new(vec![
        Column::numeric(ColumnId(1), "X", vec![Some(-0.), Some(1.)]).unwrap(),
        Column::numeric(ColumnId(2), "Y", vec![Some(3.), Some(4.)]).unwrap(),
        Column::numeric(
            ColumnId(3),
            "W",
            vec![Some(1.), Some((MAX_FREQUENCY - 1) as f64)],
        )
        .unwrap(),
    ])
    .unwrap();
    let mut r = request();
    r.frequency_weight = Some(ColumnId(3));
    let result = compute(&d, &r);
    assert_eq!(result.n, MAX_FREQUENCY);
    assert_eq!(result.points[0].x.to_bits(), (-0.0f64).to_bits());
    assert_eq!(result.points[1].frequency, MAX_FREQUENCY - 1);
    let changed = d.edit_cell(ColumnId(1), 1, Cell::Number(20.)).unwrap();
    assert_ne!(changed.revision(), result.revision);
    assert_eq!(result.points[1].x, 1.);
    let excess = d.edit_cell(ColumnId(3), 0, Cell::Number(2.)).unwrap();
    assert!(scatter_with_budget(&excess, &r, &JobControl::default(), &[], 0).is_err());
}
