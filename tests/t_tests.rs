use tessera::{analysis::*, data::*, operation::JobControl};
fn data(a: &[f64], b: &[f64]) -> Dataset {
    Dataset::new(vec![
        Column::numeric(ColumnId(1), "First", a.iter().copied().map(Some).collect()).unwrap(),
        Column::numeric(ColumnId(2), "Second", b.iter().copied().map(Some).collect()).unwrap(),
    ])
    .unwrap()
}
fn req() -> OneSampleTRequest {
    OneSampleTRequest {
        variable: ColumnId(1),
        reference_mean: 0.,
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
    }
}
fn near(e: &Estimate, x: f64) {
    assert!(
        (e.value.unwrap() - x).abs() <= 2e-12 * x.abs().max(1.),
        "{e:?} != {x}"
    );
}
#[test]
fn analytical_one_sample_and_paired() {
    let d = data(&[1., 2., 3.], &[0.; 3]);
    let r = one_sample_t_with_budget(&d, &req(), &JobControl::default(), &[], 0).unwrap();
    assert_eq!(r.sample.n, 3);
    near(&r.sample.mean, 2.);
    near(&r.sample.sample_sd, 1.);
    near(&r.statistics.standard_error, 1. / 3f64.sqrt());
    near(&r.statistics.t, 12f64.sqrt());
    near(&r.statistics.df, 2.);
    near(&r.statistics.p, 1. - (12f64 / 14.).sqrt());
    near(&r.statistics.effect_size, 2.);
    let critical = (2. * (1. / 0.05f64 / 1.95 - 1.)).sqrt();
    near(&r.statistics.interval[0], 2. - critical / 3f64.sqrt());
    near(&r.statistics.interval[1], 2. + critical / 3f64.sqrt());
    let d = data(&[2., 4., 7.], &[1., 2., 4.]);
    let q = PairedTRequest {
        first: ColumnId(1),
        second: ColumnId(2),
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
    };
    let p = paired_t_with_budget(&d, &q, &JobControl::default(), &[], 0).unwrap();
    near(&p.difference_sd, 1.);
    assert_eq!(p.statistics, r.statistics);
}
#[test]
fn overflowing_differences_keep_rows_and_ratios() {
    let d = data(&[f64::MAX, -f64::MAX], &[-f64::MAX, f64::MAX]);
    let q = PairedTRequest {
        first: ColumnId(1),
        second: ColumnId(2),
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
    };
    let r = paired_t_with_budget(&d, &q, &JobControl::default(), &[], 0).unwrap();
    assert_eq!(r.first.n, 2);
    near(&r.statistics.difference, 0.);
    near(&r.statistics.t, 0.);
    near(&r.statistics.p, 1.);
    near(&r.statistics.effect_size, 0.);
    assert!(r.difference_sd.value.is_none());
    let d = data(&[0., f64::from_bits(1)], &[0.; 2]);
    let r = one_sample_t_with_budget(&d, &req(), &JobControl::default(), &[], 0).unwrap();
    near(&r.statistics.t, 1.);
    near(&r.statistics.p, 0.5);
    assert!(r.statistics.standard_error.value.is_none());
}
#[test]
fn zero_se_retains_defined_outputs() {
    let d = data(&[2., 2.], &[0.; 2]);
    let r = one_sample_t_with_budget(&d, &req(), &JobControl::default(), &[], 0).unwrap();
    near(&r.statistics.difference, 2.);
    near(&r.statistics.df, 1.);
    near(&r.statistics.interval[0], 2.);
    assert!(r.statistics.t.value.is_none());
    assert!(r.statistics.p.value.is_none());
    assert!(r.statistics.effect_size.value.is_none());
}
fn independent(method: IndependentMethod) -> IndependentTRequest {
    IndependentTRequest {
        outcome: ColumnId(1),
        group: ColumnId(2),
        groups: [Cell::Number(0.), Cell::Number(1.)],
        method,
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
    }
}
#[test]
fn analytical_independent() {
    let d = data(&[1., 2., 3., 4., 6., 8.], &[0., 0., 0., 1., 1., 1.]);
    for method in [IndependentMethod::Welch, IndependentMethod::Pooled] {
        let r = independent_t_with_budget(&d, &independent(method), &JobControl::default(), &[], 0)
            .unwrap();
        near(&r.groups[0].mean, 2.);
        near(&r.groups[1].mean, 6.);
        near(&r.groups[0].sample_sd, 1.);
        near(&r.groups[1].sample_sd, 2.);
        near(&r.statistics.difference, -4.);
        near(&r.statistics.standard_error, (5f64 / 3.).sqrt());
        near(&r.statistics.t, -4. / (5f64 / 3.).sqrt());
        near(&r.statistics.effect_size, -4. / 2.5f64.sqrt());
        // Independent mpmath 80-digit incomplete-beta and inverse-tail references.
        let (df, p, critical) = if method == IndependentMethod::Welch {
            (50. / 17., 0.05478676604107642, 3.218758614532884)
        } else {
            (4., 0.03627782452189909, 2.776445105197794)
        };
        near(&r.statistics.df, df);
        near(&r.statistics.p, p);
        near(
            &r.statistics.interval[0],
            -4. - critical * (5f64 / 3.).sqrt(),
        );
        near(
            &r.statistics.interval[1],
            -4. + critical * (5f64 / 3.).sqrt(),
        );
    }
}
#[test]
fn log_tail_and_cancellation() {
    let d = data(&[0.01, 0.03], &[0.; 2]);
    let q = OneSampleTRequest {
        reference_mean: 1e308,
        ..req()
    };
    let r = one_sample_t_with_budget(&d, &q, &JobControl::default(), &[], 0).unwrap();
    assert!(r.statistics.t.value.is_none());
    let p = r.statistics.p.value.unwrap();
    assert!((p / 6.3661977236758e-311 - 1.).abs() < 1e-12);
    let d = data(&[1e16, 1e16 + 2., -1e16, -1e16], &[0.; 4]);
    let r = one_sample_t_with_budget(&d, &req(), &JobControl::default(), &[], 0).unwrap();
    near(&r.sample.mean, 0.5);
}
#[test]
fn missing_pairs_and_reversal() {
    let d = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "a",
            vec![Some(2.), Some(4.), Some(7.), Some(100.)],
        )
        .unwrap(),
        Column::numeric(ColumnId(2), "b", vec![Some(1.), Some(2.), Some(4.), None]).unwrap(),
    ])
    .unwrap();
    let q = PairedTRequest {
        first: ColumnId(1),
        second: ColumnId(2),
        confidence: ConfidenceLevel::P90,
        filter: RowFilter::default(),
        frequency_weight: None,
    };
    let a = paired_t_with_budget(&d, &q, &JobControl::default(), &[], 0).unwrap();
    let b = paired_t_with_budget(
        &d,
        &PairedTRequest {
            first: q.second,
            second: q.first,
            ..q
        },
        &JobControl::default(),
        &[],
        0,
    )
    .unwrap();
    assert_eq!(a.diagnostics.complete_case_rows, 1);
    near(&a.first.mean, 13. / 3.);
    near(&a.second.mean, 7. / 3.);
    near(&b.statistics.t, -a.statistics.t.value.unwrap());
    near(
        &b.statistics.interval[0],
        -a.statistics.interval[1].value.unwrap(),
    );
    assert_eq!(a.statistics.p, b.statistics.p);
}
#[test]
fn disappeared_and_filtered_groups_are_empty() {
    let d = data(&[1., 2., 3.], &[0., 0., 0.]);
    let q = independent(IndependentMethod::Welch);
    let r = independent_t_with_budget(&d, &q, &JobControl::default(), &[], 0).unwrap();
    assert_eq!(r.groups[1].n, 0);
    assert!(r.statistics.difference.value.is_none());
    let q = IndependentTRequest {
        filter: RowFilter {
            predicates: vec![RowPredicate::Number {
                column: ColumnId(2),
                comparison: NumericComparison::Equal,
                value: 1.,
            }],
        },
        ..q
    };
    let r = independent_t_with_budget(
        &d,
        &q,
        &JobControl::default(),
        &[],
        AnalysisRequestRef::IndependentT(&q).heap_bytes().unwrap(),
    )
    .unwrap();
    assert_eq!(r.groups[0].n, 0);
    assert_eq!(r.diagnostics.filtered_rows, 3);
}
#[test]
fn independent_zero_variances() {
    let d = data(&[2., 2., 4., 4.], &[0., 0., 1., 1.]);
    for method in [IndependentMethod::Welch, IndependentMethod::Pooled] {
        let r = independent_t_with_budget(&d, &independent(method), &JobControl::default(), &[], 0)
            .unwrap();
        near(&r.statistics.difference, -2.);
        near(&r.statistics.standard_error, 0.);
        assert!(r.statistics.t.value.is_none());
        assert_eq!(
            r.statistics.df.value.is_some(),
            method == IndependentMethod::Pooled
        );
        assert_eq!(
            r.statistics.interval[0].value.is_some(),
            method == IndependentMethod::Pooled
        );
    }
    let d = data(&[2., 2., 1., 2., 3.], &[0., 0., 1., 1., 1.]);
    let r = independent_t_with_budget(
        &d,
        &independent(IndependentMethod::Welch),
        &JobControl::default(),
        &[],
        0,
    )
    .unwrap();
    near(&r.statistics.df, 2.);
    near(&r.statistics.p, 1.);
    near(&r.statistics.standard_error, 1. / 3f64.sqrt());
}
#[test]
fn invalid_budget_and_cancel() {
    let d = data(&[1., 2., 3.], &[0., 0., 1.]);
    let c = JobControl::default();
    c.cancel();
    assert!(one_sample_t_with_budget(&d, &req(), &c, &[], 0).is_err());
    assert!(one_sample_t_with_budget(
        &d,
        &req(),
        &JobControl::default(),
        &[],
        MAX_OWNED_BYTES - 8192
    )
    .is_err());
    let q = OneSampleTRequest {
        reference_mean: f64::NAN,
        ..req()
    };
    assert!(one_sample_t_with_budget(&d, &q, &JobControl::default(), &[], 0).is_err());
    let q = IndependentTRequest {
        group: ColumnId(1),
        ..independent(IndependentMethod::Welch)
    };
    assert!(independent_t_with_budget(&d, &q, &JobControl::default(), &[], 0).is_err());
    let text = Dataset::new(vec![
        Column::text(ColumnId(1), "a", vec![Some("x".into())]).unwrap()
    ])
    .unwrap();
    assert!(one_sample_t_with_budget(&text, &req(), &JobControl::default(), &[], 0).is_err());
}
#[test]
fn weighted_tests_match_literal_replication() {
    let d = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "a",
            vec![Some(1.), Some(3.), Some(4.), Some(8.)],
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "b",
            vec![Some(0.), Some(0.), Some(1.), Some(1.)],
        )
        .unwrap(),
        Column::numeric(
            ColumnId(3),
            "w",
            vec![Some(2.), Some(1.), Some(1.), Some(2.)],
        )
        .unwrap(),
    ])
    .unwrap();
    let expanded = data(&[1., 1., 3., 4., 8., 8.], &[0., 0., 0., 1., 1., 1.]);
    let a = one_sample_t_with_budget(
        &d,
        &OneSampleTRequest {
            frequency_weight: Some(ColumnId(3)),
            ..req()
        },
        &JobControl::default(),
        &[],
        0,
    )
    .unwrap();
    let b = one_sample_t_with_budget(&expanded, &req(), &JobControl::default(), &[], 0).unwrap();
    assert_eq!(a.statistics, b.statistics);
    for method in [IndependentMethod::Welch, IndependentMethod::Pooled] {
        let q = independent(method);
        let a = independent_t_with_budget(
            &d,
            &IndependentTRequest {
                frequency_weight: Some(ColumnId(3)),
                ..q.clone()
            },
            &JobControl::default(),
            &[],
            0,
        )
        .unwrap();
        let b = independent_t_with_budget(&expanded, &q, &JobControl::default(), &[], 0).unwrap();
        assert_eq!(a.statistics, b.statistics);
    }
    let q = PairedTRequest {
        first: ColumnId(1),
        second: ColumnId(2),
        confidence: ConfidenceLevel::P99,
        filter: RowFilter::default(),
        frequency_weight: None,
    };
    let a = paired_t_with_budget(
        &d,
        &PairedTRequest {
            frequency_weight: Some(ColumnId(3)),
            ..q.clone()
        },
        &JobControl::default(),
        &[],
        0,
    )
    .unwrap();
    let b = paired_t_with_budget(&expanded, &q, &JobControl::default(), &[], 0).unwrap();
    assert_eq!(a.statistics, b.statistics);
}
#[test]
fn independent_extremes_and_rescaling() {
    let base = [1., 2., 3., 4., 6., 8.];
    let g = [0., 0., 0., 1., 1., 1.];
    let plain = independent_t_with_budget(
        &data(&base, &g),
        &independent(IndependentMethod::Welch),
        &JobControl::default(),
        &[],
        0,
    )
    .unwrap();
    for factor in [1e-300, 1e300] {
        let values = base.map(|x| x * factor);
        let r = independent_t_with_budget(
            &data(&values, &g),
            &independent(IndependentMethod::Welch),
            &JobControl::default(),
            &[],
            0,
        )
        .unwrap();
        near(&r.statistics.t, plain.statistics.t.value.unwrap());
        near(&r.statistics.df, plain.statistics.df.value.unwrap());
        near(
            &r.statistics.effect_size,
            plain.statistics.effect_size.value.unwrap(),
        );
        near(&r.statistics.p, plain.statistics.p.value.unwrap());
    }
    let d = data(
        &[f64::MAX, f64::MAX / 2., -f64::MAX, -f64::MAX / 2.],
        &[0., 0., 1., 1.],
    );
    let r = independent_t_with_budget(
        &d,
        &independent(IndependentMethod::Welch),
        &JobControl::default(),
        &[],
        0,
    )
    .unwrap();
    assert!(r.statistics.difference.value.is_none());
    near(&r.statistics.t, 18f64.sqrt());
    near(&r.statistics.df, 2.);
    near(&r.statistics.effect_size, 18f64.sqrt());
}
#[test]
fn ordered_exclusions_and_weight_validation() {
    let d = Dataset::new(vec![
        Column::numeric(ColumnId(1), "a", vec![None, None, None, Some(1.), Some(3.)]).unwrap(),
        Column::text(
            ColumnId(2),
            "g",
            vec![
                None,
                Some("other".into()),
                Some("A".into()),
                Some("A".into()),
                Some("A".into()),
            ],
        )
        .unwrap(),
    ])
    .unwrap();
    let q = IndependentTRequest {
        groups: [Cell::Text("A".into()), Cell::Text("B".into())],
        ..independent(IndependentMethod::Welch)
    };
    let r = independent_t_with_budget(
        &d,
        &q,
        &JobControl::default(),
        &[],
        AnalysisRequestRef::IndependentT(&q).heap_bytes().unwrap(),
    )
    .unwrap();
    assert_eq!(
        r.exclusions,
        GroupExclusions {
            missing_group_rows: 1,
            other_group_rows: 1,
            missing_outcome_rows: 1
        }
    );
    assert_eq!(r.groups[0].n, 2);
    assert_eq!(r.diagnostics.complete_case_rows, 0);
    let bad = data(&[1., 2.], &[-1., 2.]);
    let q = OneSampleTRequest {
        frequency_weight: Some(ColumnId(2)),
        ..req()
    };
    assert!(one_sample_t_with_budget(&bad, &q, &JobControl::default(), &[], 0).is_err());
    let q = OneSampleTRequest {
        filter: RowFilter {
            predicates: vec![RowPredicate::Number {
                column: ColumnId(2),
                comparison: NumericComparison::Greater,
                value: 0.,
            }],
        },
        ..q
    };
    assert_eq!(
        one_sample_t_with_budget(
            &bad,
            &q,
            &JobControl::default(),
            &[],
            AnalysisRequestRef::OneSampleT(&q).heap_bytes().unwrap()
        )
        .unwrap()
        .sample
        .n,
        2
    );
}
#[test]
fn every_confidence_interval_and_small_samples() {
    for (confidence, alpha) in [
        (ConfidenceLevel::P90, 0.1_f64),
        (ConfidenceLevel::P95, 0.05),
        (ConfidenceLevel::P99, 0.01),
    ] {
        let r = one_sample_t_with_budget(
            &data(&[1., 2., 3.], &[0.; 3]),
            &OneSampleTRequest {
                confidence,
                ..req()
            },
            &JobControl::default(),
            &[],
            0,
        )
        .unwrap();
        let critical = (2. * (1. / (alpha * (2. - alpha)) - 1.)).sqrt();
        near(&r.statistics.interval[0], 2. - critical / 3f64.sqrt());
        near(&r.statistics.interval[1], 2. + critical / 3f64.sqrt());
    }
    for values in [vec![], vec![4.]] {
        let r = one_sample_t_with_budget(
            &data(&values, &vec![0.; values.len()]),
            &req(),
            &JobControl::default(),
            &[],
            0,
        )
        .unwrap();
        assert!(r.sample.sample_sd.value.is_none());
        assert!(r.statistics.t.value.is_none());
        assert!(r.statistics.df.value.is_none());
        if values.len() == 1 {
            near(&r.statistics.difference, 4.);
        }
    }
}
#[test]
fn paired_and_independent_guards() {
    let d = data(&[1., 2., 3., 4.], &[0., 0., 1., 1.]);
    let q = PairedTRequest {
        first: ColumnId(1),
        second: ColumnId(2),
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
    };
    let c = JobControl::default();
    c.cancel();
    assert!(paired_t_with_budget(&d, &q, &c, &[], 0).is_err());
    assert!(
        independent_t_with_budget(&d, &independent(IndependentMethod::Welch), &c, &[], 0).is_err()
    );
    assert!(paired_t_with_budget(
        &d,
        &q,
        &JobControl::default(),
        &[&d],
        MAX_OWNED_BYTES - 8192
    )
    .is_err());
    assert!(independent_t_with_budget(
        &d,
        &independent(IndependentMethod::Welch),
        &JobControl::default(),
        &[&d],
        MAX_OWNED_BYTES - 8192
    )
    .is_err());
    assert!(paired_t_with_budget(
        &d,
        &PairedTRequest {
            second: ColumnId(99),
            ..q
        },
        &JobControl::default(),
        &[],
        0
    )
    .is_err());
    let q = IndependentTRequest {
        groups: [Cell::Text("A".into()), Cell::Text("B".into())],
        ..independent(IndependentMethod::Welch)
    };
    assert!(independent_t_with_budget(
        &d,
        &q,
        &JobControl::default(),
        &[],
        AnalysisRequestRef::IndependentT(&q).heap_bytes().unwrap()
    )
    .is_err());
}
#[test]
fn unequal_n_independent_references() {
    let d = data(&[1., 3., 4., 6., 8.], &[0., 0., 1., 1., 1.]);
    for (method, df, se, p, critical) in [
        (
            IndependentMethod::Welch,
            49. / 17.,
            (7f64 / 3.).sqrt(),
            0.08244065846409346,
            3.2572081254789047,
        ),
        (
            IndependentMethod::Pooled,
            3.,
            5. / 3.,
            0.09587448227183103,
            3.1824463052837096,
        ),
    ] {
        let r = independent_t_with_budget(&d, &independent(method), &JobControl::default(), &[], 0)
            .unwrap();
        near(&r.statistics.df, df);
        near(&r.statistics.standard_error, se);
        near(&r.statistics.t, -4. / se);
        near(&r.statistics.p, p);
        near(&r.statistics.interval[0], -4. - critical * se);
        near(&r.statistics.interval[1], -4. + critical * se);
        near(&r.statistics.effect_size, -4. / (10f64 / 3.).sqrt());
        CompletedAnalysis::IndependentT(r).validate().unwrap();
    }
}
