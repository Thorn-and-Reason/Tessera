use tessera::{analysis::*, data::*, operation::JobControl};
fn dataset(values: &[Vec<Option<f64>>]) -> Dataset {
    Dataset::new(
        values
            .iter()
            .enumerate()
            .map(|(i, v)| {
                Column::numeric(ColumnId(i as u64 + 1), &format!("V{i}"), v.clone()).unwrap()
            })
            .collect(),
    )
    .unwrap()
}
fn request(count: u64) -> PearsonRequest {
    PearsonRequest {
        columns: (1..=count).map(ColumnId).collect(),
        missing_policy: CorrelationMissingPolicy::PairwiseComplete,
        filter: RowFilter::default(),
        frequency_weight: None,
    }
}
fn run(d: &Dataset, r: &PearsonRequest) -> ComputedPearson {
    pearson_with_budget(
        d,
        r,
        &JobControl::default(),
        &[],
        AnalysisRequestRef::Pearson(r).heap_bytes().unwrap(),
    )
    .unwrap()
}
fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 2e-14, "{a} != {b}");
}
#[test]
fn ordered_pairs_and_pair_specific_samples() {
    let d = dataset(&[
        vec![Some(1.), Some(2.), Some(3.), Some(4.), None],
        vec![Some(2.), Some(4.), Some(6.), None, Some(10.)],
        vec![Some(3.), Some(6.), Some(9.), Some(12.), Some(15.)],
    ]);
    let mut r = request(3);
    r.columns = vec![ColumnId(3), ColumnId(1), ColumnId(2)];
    let pair = run(&d, &r);
    assert_eq!(
        pair.rows.iter().map(|v| v.n).collect::<Vec<_>>(),
        vec![4, 4, 3]
    );
    assert_eq!(
        (pair.rows[0].first, pair.rows[0].second),
        (ColumnId(3), ColumnId(1))
    );
    for row in &pair.rows {
        close(row.r.value.unwrap(), 1.);
        assert_eq!(row.p.value, Some(0.));
    }
    r.missing_policy = CorrelationMissingPolicy::CompleteCases;
    let result = run(&d, &r);
    assert_eq!(result.diagnostics.complete_case_rows, 2);
    assert!(result
        .rows
        .iter()
        .all(|v| v.n == 3 && v.missing_rows + v.complete_case_excluded_rows == 2));
    CompletedAnalysis::Pearson(result).validate().unwrap();
}
#[test]
fn weighted_pairs_match_replication() {
    let d = dataset(&[
        vec![Some(1.), Some(2.), Some(4.)],
        vec![Some(4.), Some(1.), Some(3.)],
        vec![Some(2.), Some(3.), Some(1.)],
    ]);
    let mut r = request(2);
    r.frequency_weight = Some(ColumnId(3));
    let weighted = run(&d, &r);
    let replicated = dataset(&[
        vec![Some(1.), Some(1.), Some(2.), Some(2.), Some(2.), Some(4.)],
        vec![Some(4.), Some(4.), Some(1.), Some(1.), Some(1.), Some(3.)],
    ]);
    let simple = run(&replicated, &request(2));
    assert_eq!(weighted.rows[0].n, 6);
    close(
        weighted.rows[0].r.value.unwrap(),
        simple.rows[0].r.value.unwrap(),
    );
    close(
        weighted.rows[0].p.value.unwrap(),
        simple.rows[0].p.value.unwrap(),
    );
}
#[test]
fn large_offset_and_extreme_rescaling() {
    let x = [0., 1., 2., 3., 4.];
    let y = [1., 4., 2., 8., 5.];
    let base = run(
        &dataset(&[x.map(Some).to_vec(), y.map(Some).to_vec()]),
        &request(2),
    )
    .rows[0]
        .r
        .value
        .unwrap();
    for (offset, scale) in [(1e15, 1.), (0., 1e-300), (0., 1e300)] {
        let d = dataset(&[
            x.map(|v| Some(offset + v * scale)).to_vec(),
            y.map(|v| Some(offset + v * scale)).to_vec(),
        ]);
        close(run(&d, &request(2)).rows[0].r.value.unwrap(), base);
    }
    close(base, 12. / 300_f64.sqrt());
}
#[test]
fn constant_and_n2_reasons() {
    let constant = run(
        &dataset(&[vec![Some(1.); 3], vec![Some(1.), Some(2.), Some(3.)]]),
        &request(2),
    );
    assert!(constant.rows[0].r.reason.is_some());
    assert!(constant.rows[0].p.reason.is_some());
    let two = run(
        &dataset(&[vec![Some(1.), Some(2.)], vec![Some(3.), Some(1.)]]),
        &request(2),
    );
    assert_eq!(two.rows[0].r.value, Some(-1.));
    assert!(two.rows[0].p.reason.is_some());
}
#[test]
fn pair_budget_and_cancel() {
    let d = dataset(&[vec![Some(1.), Some(2.)], vec![Some(1.), Some(2.)]]);
    let r = request(2);
    let c = JobControl::default();
    assert!(pearson_with_budget(&d, &r, &c, &[], MAX_OWNED_BYTES).is_err());
    c.cancel();
    assert!(pearson_with_budget(&d, &r, &c, &[], 0).is_err());
    let many = dataset(&(0..1800).map(|_| vec![]).collect::<Vec<_>>());
    assert!(pearson_with_budget(&many, &request(1800), &JobControl::default(), &[], 0).is_err());
}

#[test]
fn nonperfect_reference_tail_and_empty_cohort() {
    let d = dataset(&[
        vec![Some(-1.), Some(0.), Some(1.)],
        vec![Some(0.), Some(-2.), Some(2.)],
    ]);
    let result = run(&d, &request(2));
    close(result.rows[0].r.value.unwrap(), 0.5);
    close(result.rows[0].p.value.unwrap(), 2. / 3.);
    let empty = run(&dataset(&[vec![None], vec![Some(1.)]]), &request(2));
    assert_eq!(empty.rows[0].n, 0);
    assert_eq!(empty.rows[0].missing_rows, 1);
    assert!(empty.rows[0].r.reason.is_some());
}
#[test]
fn weight_validation_precedes_pair_missingness_and_respects_filter() {
    let d = dataset(&[
        vec![None, Some(1.), Some(2.)],
        vec![Some(3.), Some(2.), Some(4.)],
        vec![Some(-1.), Some(1.), Some(1.)],
    ]);
    let mut r = request(2);
    r.frequency_weight = Some(ColumnId(3));
    assert!(pearson_with_budget(&d, &r, &JobControl::default(), &[], 0).is_err());
    r.filter.predicates.push(RowPredicate::Missing {
        column: ColumnId(1),
        missing: false,
    });
    let result = run(&d, &r);
    assert_eq!(result.rows[0].n, 2);
    assert_eq!(result.diagnostics.filtered_rows, 1);
}

#[test]
fn subnormal_and_maximum_adjacent_spreads() {
    let tiny = f64::from_bits(1);
    let maximum = f64::MAX;
    let adjacent = f64::from_bits(maximum.to_bits() - 1);
    for (lo, hi) in [(0., tiny), (adjacent, maximum)] {
        let d = dataset(&[
            vec![Some(lo), Some(hi), Some(hi)],
            vec![Some(hi), Some(lo), Some(hi)],
        ]);
        let result = run(&d, &request(2));
        close(result.rows[0].r.value.unwrap(), -0.5);
        close(result.rows[0].p.value.unwrap(), 2. / 3.);
    }
    let d = dataset(&[
        vec![Some(-maximum), Some(0.), Some(maximum)],
        vec![Some(maximum), Some(0.), Some(-maximum)],
    ]);
    assert_eq!(run(&d, &request(2)).rows[0].r.value, Some(-1.));
}

#[test]
fn irregular_perfect_pairs_and_genuine_near_perfect_inference() {
    for direction in [1., -1.] {
        for weights in [None, Some(vec![Some(2.), Some(3.), Some(4.)])] {
            let mut columns = vec![
                vec![Some(0.), Some(1.), Some(13.)],
                vec![Some(0.), Some(direction), Some(13. * direction)],
            ];
            let mut r = request(2);
            if let Some(weights) = weights {
                columns.push(weights);
                r.frequency_weight = Some(ColumnId(3));
            }
            let result = run(&dataset(&columns), &r);
            assert_eq!(result.rows[0].r.value, Some(direction));
            assert_eq!(result.rows[0].p.value, Some(0.));
        }
        let near = run(
            &dataset(&[
                vec![Some(0.), Some(1.), Some(13.)],
                vec![Some(0.), Some(direction * 1.000001), Some(direction * 13.)],
            ]),
            &request(2),
        );
        assert!(near.rows[0].r.value.unwrap().abs() < 1.);
        assert!(near.rows[0].p.value.unwrap() > 0.);
        assert!((1. - near.rows[0].r.value.unwrap().abs()) < 32. * f64::EPSILON);
        let two = run(
            &dataset(&[
                vec![Some(1.), Some(13.)],
                vec![Some(direction), Some(direction * 13.)],
            ]),
            &request(2),
        );
        assert_eq!(two.rows[0].r.value, Some(direction));
        assert!(two.rows[0].p.reason.is_some());
    }
}
