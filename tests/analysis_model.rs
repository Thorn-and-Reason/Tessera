use std::sync::{atomic::AtomicBool, mpsc, Arc};
use tessera::{analysis::*, data::*, operation::JobControl};

fn data() -> Dataset {
    Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "X",
            vec![None, Some(2.0), Some(3.0), Some(4.0)],
        )
        .unwrap(),
        Column::numeric(ColumnId(2), "Y", vec![Some(1.0); 4]).unwrap(),
        Column::numeric(
            ColumnId(3),
            "Weight",
            vec![Some(-1.0), None, Some(0.0), Some(2.0)],
        )
        .unwrap(),
        Column::text(ColumnId(4), "Group", vec![Some("A".into()); 4]).unwrap(),
    ])
    .unwrap()
}
fn one() -> AnalysisRequest {
    AnalysisRequest::OneSampleT(OneSampleTRequest {
        variable: ColumnId(1),
        reference_mean: 0.0,
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
    })
}
fn estimate(value: f64) -> Estimate {
    Estimate {
        value: Some(value),
        reason: None,
    }
}
fn unavailable() -> Estimate {
    Estimate {
        value: None,
        reason: Some("Unavailable".into()),
    }
}
fn summary() -> SampleSummary {
    SampleSummary {
        n: 3,
        mean: estimate(2.0),
        sample_sd: unavailable(),
    }
}
fn statistics() -> TStatistics {
    TStatistics {
        difference: estimate(2.0),
        standard_error: unavailable(),
        t: estimate(2.0),
        df: estimate(2.0),
        p: estimate(0.183503419072274),
        interval: [estimate(-2.0), estimate(6.0)],
        effect_size: unavailable(),
    }
}
fn snapshots() -> Vec<RunColumnInfo> {
    vec![RunColumnInfo {
        id: ColumnId(1),
        name: "X".into(),
        kind: ColumnKind::Number,
        description: String::new(),
        missing_rules: MissingRules::None,
        value_labels: vec![],
    }]
}
// Catches missing role checks, conflating current and historical schemas, and accepting invalid inference.
#[test]
fn typed_roles_and_historical_result_validation() {
    let data = data();
    let pearson = AnalysisRequest::Pearson(PearsonRequest {
        columns: vec![ColumnId(1), ColumnId(1)],
        missing_policy: CorrelationMissingPolicy::PairwiseComplete,
        filter: RowFilter::default(),
        frequency_weight: None,
    });
    assert!(pearson.validate_current(&data, &[], 0).is_err());
    let paired = AnalysisRequest::PairedT(PairedTRequest {
        first: ColumnId(1),
        second: ColumnId(1),
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
    });
    assert!(paired.validate_current(&data, &[], 0).is_err());
    for groups in [
        [Cell::Missing, Cell::Text("B".into())],
        [Cell::Number(1.0), Cell::Text("B".into())],
        [Cell::Text("A".into()), Cell::Text("A".into())],
    ] {
        let request = AnalysisRequest::IndependentT(IndependentTRequest {
            outcome: ColumnId(1),
            group: ColumnId(4),
            groups,
            method: IndependentMethod::Welch,
            confidence: ConfidenceLevel::P95,
            filter: RowFilter::default(),
            frequency_weight: None,
        });
        assert!(request.validate_current(&data, &[], 0).is_err());
    }
    if let AnalysisRequest::OneSampleT(mut request) = one() {
        for confidence in [
            ConfidenceLevel::P90,
            ConfidenceLevel::P95,
            ConfidenceLevel::P99,
        ] {
            request.confidence = confidence;
            assert!(AnalysisRequest::OneSampleT(request.clone())
                .validate_current(&data, &[], 0)
                .is_ok());
        }
        request.reference_mean = f64::INFINITY;
        assert!(AnalysisRequest::OneSampleT(request)
            .validate_current(&data, &[], 0)
            .is_err());
    }
    let AnalysisRequest::OneSampleT(request) = one() else {
        unreachable!()
    };
    let result = ComputedOneSampleT {
        revision: RevisionId(1),
        request,
        columns: snapshots(),
        diagnostics: RunDiagnostics::default(),
        sample: summary(),
        missing_rows: 1,
        statistics: statistics(),
    };
    assert!(CompletedAnalysis::OneSampleT(result.clone())
        .validate()
        .is_ok());
    let changed = Dataset::new(vec![Column::text(
        ColumnId(1),
        "X",
        vec![Some("changed".into())],
    )
    .unwrap()])
    .unwrap();
    assert!(one().validate_current(&changed, &[], 0).is_err());
    assert!(CompletedAnalysis::OneSampleT(result.clone())
        .validate()
        .is_ok());
    let mut invalid = result.clone();
    invalid.statistics.p = estimate(0.0);
    assert!(CompletedAnalysis::OneSampleT(invalid).validate().is_err());
    let mut invalid = result.clone();
    invalid.statistics.df = estimate(3.0);
    assert!(CompletedAnalysis::OneSampleT(invalid).validate().is_err());
    let mut invalid = result.clone();
    invalid.statistics.interval = [estimate(6.0), estimate(-2.0)];
    assert!(CompletedAnalysis::OneSampleT(invalid).validate().is_err());
    let mut invalid = result.clone();
    invalid.sample.n = MAX_FREQUENCY + 1;
    assert!(CompletedAnalysis::OneSampleT(invalid).validate().is_err());
    let mut invalid = result;
    invalid.statistics.t.reason = Some("contradiction".into());
    assert!(CompletedAnalysis::OneSampleT(invalid).validate().is_err());
    let request = PearsonRequest {
        columns: vec![ColumnId(1), ColumnId(2)],
        missing_policy: CorrelationMissingPolicy::PairwiseComplete,
        filter: RowFilter::default(),
        frequency_weight: None,
    };
    let mut columns = snapshots();
    columns.push(RunColumnInfo {
        id: ColumnId(2),
        name: "Y".into(),
        ..columns[0].clone()
    });
    let perfect = ComputedPearson {
        revision: RevisionId(1),
        request,
        columns,
        diagnostics: RunDiagnostics::default(),
        rows: vec![CorrelationRow {
            first: ColumnId(1),
            second: ColumnId(2),
            n: 3,
            r: estimate(1.0),
            p: estimate(0.0),
            missing_rows: 1,
            complete_case_excluded_rows: 0,
        }],
    };
    assert!(CompletedAnalysis::Pearson(perfect.clone())
        .validate()
        .is_ok());
    let mut invalid = perfect.clone();
    invalid.rows[0].r = estimate(0.99);
    assert!(CompletedAnalysis::Pearson(invalid).validate().is_err());
    let mut invalid = perfect;
    invalid.rows[0].n = 2;
    assert!(CompletedAnalysis::Pearson(invalid).validate().is_err());
}
// Catches cancellation checks publishing/resetting progress and eligibility preceding weight validation.
#[test]
fn weight_order_and_neutral_cancel_check() {
    let (tx, rx) = mpsc::channel();
    let control = JobControl::new(Arc::new(AtomicBool::new(false)), Some(tx));
    control.checkpoint(7, Some(10)).unwrap();
    assert_eq!(rx.recv().unwrap().done, 7);
    control.check_cancelled().unwrap();
    assert!(rx.try_recv().is_err());
    control.cancel();
    assert!(control.check_cancelled().is_err());
    assert!(rx.try_recv().is_err());
    let data = data();
    let request = DescribeRequest {
        columns: vec![ColumnId(1)],
        statistics: vec![Statistic::Mean],
        missing_policy: MissingPolicy::PerVariable,
        frequency_weight: Some(ColumnId(3)),
        filter: RowFilter {
            predicates: vec![RowPredicate::Missing {
                column: ColumnId(1),
                missing: false,
            }],
        },
    };
    let result = describe(&data, &request, &JobControl::default()).unwrap();
    assert_eq!(result.diagnostics.filtered_rows, 1);
    assert_eq!(result.diagnostics.missing_weight_rows, 1);
    assert_eq!(result.diagnostics.zero_weight_rows, 1);
    assert_eq!(result.rows[0].valid_n, 2);
    let unfiltered = DescribeRequest {
        filter: RowFilter::default(),
        ..request
    };
    assert!(describe(&data, &unfiltered, &JobControl::default()).is_err());
}

// Catches flushing representable tails to zero or rejecting independently retained intervals.
#[test]
fn subnormal_t_tail_and_independent_endpoint_survive_validation() {
    let AnalysisRequest::OneSampleT(request) = one() else {
        unreachable!()
    };
    let result = ComputedOneSampleT {
        revision: RevisionId(1),
        request,
        columns: snapshots(),
        diagnostics: RunDiagnostics::default(),
        sample: summary(),
        missing_rows: 0,
        statistics: TStatistics {
            t: estimate(1e160),
            p: estimate(f64::from_bits(1)),
            interval: [unavailable(), estimate(6.0)],
            ..statistics()
        },
    };
    assert!(CompletedAnalysis::OneSampleT(result).validate().is_ok());
}
// Catches omissions in request references, group identity, capacity accounting and preflight.
#[test]
fn recipe_views_and_reference_bounds() {
    let data = data();
    let recipes = vec![
        one(),
        AnalysisRequest::Pearson(PearsonRequest {
            columns: vec![ColumnId(1), ColumnId(2)],
            missing_policy: CorrelationMissingPolicy::CompleteCases,
            filter: RowFilter::default(),
            frequency_weight: Some(ColumnId(3)),
        }),
        AnalysisRequest::PairedT(PairedTRequest {
            first: ColumnId(1),
            second: ColumnId(2),
            confidence: ConfidenceLevel::P90,
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        AnalysisRequest::IndependentT(IndependentTRequest {
            outcome: ColumnId(1),
            group: ColumnId(4),
            groups: [Cell::Text("A".into()), Cell::Text("absent".into())],
            method: IndependentMethod::Pooled,
            confidence: ConfidenceLevel::P99,
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        AnalysisRequest::Describe(DescribeRequest {
            columns: vec![ColumnId(1)],
            statistics: vec![Statistic::Mean],
            missing_policy: MissingPolicy::PerVariable,
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
    ];
    for request in recipes {
        assert!(request
            .validate_current(&data, &[], request.heap_bytes().unwrap())
            .is_ok());
        assert_eq!(request.borrowed(), request);
        assert_eq!(request.borrowed().clone_owned(), request);
        assert!(request.borrowed().validation_scratch_bytes().unwrap() > 0);
        assert!(request
            .validate_current(&data, &[], MAX_OWNED_BYTES)
            .is_err());
        assert!(request.validate_current(&data, &[], usize::MAX).is_err());
    }
    let request = AnalysisRequest::IndependentT(IndependentTRequest {
        outcome: ColumnId(1),
        group: ColumnId(2),
        groups: [Cell::Number(0.0), Cell::Number(-0.0)],
        method: IndependentMethod::Welch,
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
    });
    assert!(request.validate_current(&data, &[], 0).is_err());
    for filter in [
        RowFilter {
            predicates: vec![RowPredicate::Number {
                column: ColumnId(4),
                comparison: NumericComparison::Equal,
                value: 1.0,
            }],
        },
        RowFilter {
            predicates: vec![RowPredicate::Number {
                column: ColumnId(1),
                comparison: NumericComparison::Equal,
                value: f64::NAN,
            }],
        },
        RowFilter {
            predicates: vec![RowPredicate::Text {
                column: ColumnId(1),
                value: "A".into(),
                equal: true,
            }],
        },
        RowFilter {
            predicates: vec![RowPredicate::Missing {
                column: ColumnId(99),
                missing: true,
            }],
        },
    ] {
        let AnalysisRequest::OneSampleT(request) = one() else {
            unreachable!()
        };
        assert!(
            AnalysisRequest::OneSampleT(OneSampleTRequest { filter, ..request })
                .validate_current(&data, &[], 0)
                .is_err()
        );
    }
    let AnalysisRequest::OneSampleT(request) = one() else {
        unreachable!()
    };
    let result = CompletedAnalysis::OneSampleT(ComputedOneSampleT {
        revision: RevisionId(1),
        request,
        columns: snapshots(),
        diagnostics: RunDiagnostics::default(),
        sample: summary(),
        missing_rows: 0,
        statistics: statistics(),
    });
    assert_eq!(result.revision(), RevisionId(1));
    assert_eq!(result.columns().len(), 1);
    assert_eq!(result.diagnostics(), &RunDiagnostics::default());
    assert_eq!(result.request(), one());
    assert!(result.owned_bytes().unwrap() >= std::mem::size_of::<CompletedAnalysis>());
    assert!(result.validation_scratch_bytes().unwrap() > 0);
}

// Catches lost paired identity, pooled/Welch df rules, snapshot corruption and incomplete accounting.
#[test]
fn paired_and_independent_historical_invariants() {
    let mut columns = snapshots();
    columns.push(RunColumnInfo {
        id: ColumnId(2),
        name: "Y".into(),
        ..columns[0].clone()
    });
    let paired = ComputedPairedT {
        revision: RevisionId(1),
        request: PairedTRequest {
            first: ColumnId(1),
            second: ColumnId(2),
            confidence: ConfidenceLevel::P95,
            filter: RowFilter::default(),
            frequency_weight: None,
        },
        columns: columns.clone(),
        diagnostics: RunDiagnostics::default(),
        first: summary(),
        second: summary(),
        difference_sd: unavailable(),
        statistics: statistics(),
    };
    assert!(CompletedAnalysis::PairedT(paired.clone())
        .validate()
        .is_ok());
    assert!(CompletedAnalysis::PairedT(paired.clone())
        .owned_bytes()
        .is_ok());
    let mut invalid = paired.clone();
    invalid.second.n = 4;
    assert!(CompletedAnalysis::PairedT(invalid).validate().is_err());
    let mut invalid = paired;
    invalid.request.second = ColumnId(1);
    assert!(CompletedAnalysis::PairedT(invalid).validate().is_err());
    columns[1].kind = ColumnKind::Text;
    let independent = ComputedIndependentT {
        revision: RevisionId(1),
        request: IndependentTRequest {
            outcome: ColumnId(1),
            group: ColumnId(2),
            groups: [Cell::Text("A".into()), Cell::Text("B".into())],
            method: IndependentMethod::Welch,
            confidence: ConfidenceLevel::P95,
            filter: RowFilter::default(),
            frequency_weight: None,
        },
        columns,
        diagnostics: RunDiagnostics::default(),
        groups: [summary(), summary()],
        exclusions: GroupExclusions::default(),
        statistics: TStatistics {
            df: estimate(3.5),
            ..statistics()
        },
    };
    // Welch df cannot fall below min(N1 - 1, N2 - 1), even if displayed SD/SE are unavailable.
    for df in [0.5, 2.0 - 1e-10] {
        let invalid = ComputedIndependentT {
            statistics: TStatistics {
                df: estimate(df),
                ..independent.statistics.clone()
            },
            ..independent.clone()
        };
        assert!(
            CompletedAnalysis::IndependentT(invalid).validate().is_err(),
            "accepted impossible Welch df={df}"
        );
    }
    for df in [2.0, 2.0 - 2.0 * f64::EPSILON] {
        let valid = ComputedIndependentT {
            statistics: TStatistics {
                df: estimate(df),
                ..independent.statistics.clone()
            },
            ..independent.clone()
        };
        assert!(
            CompletedAnalysis::IndependentT(valid).validate().is_ok(),
            "rejected boundary Welch df={df}"
        );
    }
    let completed = CompletedAnalysis::IndependentT(independent.clone());
    assert!(completed.validate().is_ok());
    assert!(completed.owned_bytes().is_ok());
    let mut invalid = independent.clone();
    invalid.request.method = IndependentMethod::Pooled;
    assert!(CompletedAnalysis::IndependentT(invalid).validate().is_err());
    let mut valid = independent.clone();
    valid.request.method = IndependentMethod::Pooled;
    valid.statistics.df = estimate(4.0);
    assert!(CompletedAnalysis::IndependentT(valid).validate().is_ok());
    let mut invalid = independent.clone();
    invalid.statistics.df = estimate(4.1);
    assert!(CompletedAnalysis::IndependentT(invalid).validate().is_err());
    let mut invalid = independent.clone();
    invalid.groups[0].n = 1;
    assert!(CompletedAnalysis::IndependentT(invalid).validate().is_err());
    let mut invalid = independent.clone();
    invalid.request.group = ColumnId(1);
    invalid.request.groups = [Cell::Number(1.0), Cell::Number(2.0)];
    assert!(CompletedAnalysis::IndependentT(invalid).validate().is_err());
    let mut invalid = independent.clone();
    invalid.exclusions.missing_group_rows = MAX_CELLS + 1;
    assert!(CompletedAnalysis::IndependentT(invalid).validate().is_err());
    let mut invalid = independent.clone();
    invalid.columns[0].value_labels = vec![
        ValueLabel {
            value: Cell::Number(0.0),
            label: "zero".into(),
        },
        ValueLabel {
            value: Cell::Number(-0.0),
            label: "duplicate".into(),
        },
    ];
    assert!(CompletedAnalysis::IndependentT(invalid).validate().is_err());
    let mut invalid = independent.clone();
    invalid.columns[1].missing_rules = MissingRules::Number {
        values: vec![],
        ranges: vec![],
    };
    assert!(CompletedAnalysis::IndependentT(invalid).validate().is_err());
    let mut invalid = independent.clone();
    invalid.columns.push(invalid.columns[0].clone());
    assert!(CompletedAnalysis::IndependentT(invalid).validate().is_err());
    let mut invalid = independent.clone();
    invalid.columns[0].name = "\n".into();
    assert!(CompletedAnalysis::IndependentT(invalid).validate().is_err());
    let mut invalid = independent.clone();
    invalid.statistics.difference = Estimate {
        value: None,
        reason: Some(" ".into()),
    };
    assert!(CompletedAnalysis::IndependentT(invalid).validate().is_err());
    let baseline = completed.owned_bytes().unwrap();
    let mut with_reason = independent;
    with_reason.statistics.effect_size = Estimate {
        value: None,
        reason: Some("A deliberately much longer unavailable estimate reason.".repeat(20)),
    };
    assert!(
        CompletedAnalysis::IndependentT(with_reason)
            .owned_bytes()
            .unwrap()
            > baseline + 500
    );
}

// Catches requiring a displayable t when the engine independently retains its positive tail.
#[test]
fn engine_tail_survives_unrepresentable_t_in_historical_validation() {
    let dataset = Dataset::new(vec![Column::numeric(
        ColumnId(1),
        "Outcome",
        vec![Some(0.01), Some(0.03)],
    )
    .unwrap()])
    .unwrap();
    let request = OneSampleTRequest {
        variable: ColumnId(1),
        reference_mean: 1e308,
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
    };
    let result = one_sample_t_with_budget(
        &dataset,
        &request,
        &JobControl::default(),
        &[],
        AnalysisRequestRef::OneSampleT(&request)
            .heap_bytes()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(result.sample.n, 2);
    assert!(result.statistics.t.value.is_none());
    assert!(result.statistics.t.reason.is_some());
    assert_eq!(result.statistics.df.value, Some(1.0));
    let p = result.statistics.p.value.unwrap();
    assert!((p / 6.3661977236758e-311 - 1.0).abs() < 1e-12);
    assert!(CompletedAnalysis::OneSampleT(result.clone())
        .validate()
        .is_ok());
    let missing_df = ComputedOneSampleT {
        statistics: TStatistics {
            df: unavailable(),
            ..result.statistics.clone()
        },
        ..result.clone()
    };
    assert!(CompletedAnalysis::OneSampleT(missing_df)
        .validate()
        .is_err());
    let zero_se = ComputedOneSampleT {
        statistics: TStatistics {
            standard_error: estimate(0.0),
            ..result.statistics
        },
        ..result
    };
    assert!(CompletedAnalysis::OneSampleT(zero_se).validate().is_err());
}

fn count_result(kind: usize, n: u64, weighted: bool) -> CompletedAnalysis {
    let mut columns = snapshots();
    columns.push(RunColumnInfo {
        id: ColumnId(2),
        name: "Y".into(),
        ..columns[0].clone()
    });
    let frequency_weight = weighted.then_some(ColumnId(2));
    let filter = RowFilter::default();
    let diagnostics = RunDiagnostics {
        filtered_rows: 1,
        ..RunDiagnostics::default()
    };
    let confidence = ConfidenceLevel::P95;
    let sample = SampleSummary { n, ..summary() };
    let statistics = TStatistics {
        df: estimate(n.saturating_sub(1) as f64),
        ..statistics()
    };
    match kind {
        0 => CompletedAnalysis::Pearson(ComputedPearson {
            revision: RevisionId(1),
            request: PearsonRequest {
                columns: vec![ColumnId(1), ColumnId(2)],
                missing_policy: CorrelationMissingPolicy::CompleteCases,
                filter,
                frequency_weight,
            },
            columns,
            diagnostics: RunDiagnostics {
                complete_case_rows: 3,
                ..diagnostics
            },
            rows: vec![CorrelationRow {
                first: ColumnId(1),
                second: ColumnId(2),
                n,
                r: estimate(1.0),
                p: estimate(0.0),
                missing_rows: 1,
                complete_case_excluded_rows: 2,
            }],
        }),
        1 => {
            if !weighted {
                columns.pop();
            }
            CompletedAnalysis::OneSampleT(ComputedOneSampleT {
                revision: RevisionId(1),
                request: OneSampleTRequest {
                    variable: ColumnId(1),
                    reference_mean: 0.0,
                    confidence,
                    filter,
                    frequency_weight,
                },
                columns,
                diagnostics,
                sample,
                missing_rows: 3,
                statistics,
            })
        }
        2 => CompletedAnalysis::PairedT(ComputedPairedT {
            revision: RevisionId(1),
            request: PairedTRequest {
                first: ColumnId(1),
                second: ColumnId(2),
                confidence,
                filter,
                frequency_weight,
            },
            columns,
            diagnostics: RunDiagnostics {
                complete_case_rows: 3,
                ..diagnostics
            },
            first: sample.clone(),
            second: sample,
            difference_sd: unavailable(),
            statistics,
        }),
        _ => CompletedAnalysis::IndependentT(ComputedIndependentT {
            revision: RevisionId(1),
            request: IndependentTRequest {
                outcome: ColumnId(1),
                group: ColumnId(2),
                groups: [Cell::Number(1.0), Cell::Number(2.0)],
                method: IndependentMethod::Welch,
                confidence,
                filter,
                frequency_weight,
            },
            columns,
            diagnostics,
            groups: [
                SampleSummary {
                    n: n / 2,
                    ..sample.clone()
                },
                SampleSummary {
                    n: n - n / 2,
                    ..sample
                },
            ],
            exclusions: GroupExclusions {
                missing_group_rows: 1,
                other_group_rows: 1,
                missing_outcome_rows: 1,
            },
            statistics: TStatistics {
                df: estimate(n.saturating_sub(2) as f64),
                ..statistics
            },
        }),
    }
}
fn diagnostics_mut(result: &mut CompletedAnalysis) -> &mut RunDiagnostics {
    match result {
        CompletedAnalysis::Describe(v) => &mut v.diagnostics,
        CompletedAnalysis::Pearson(v) => &mut v.diagnostics,
        CompletedAnalysis::OneSampleT(v) => &mut v.diagnostics,
        CompletedAnalysis::PairedT(v) => &mut v.diagnostics,
        CompletedAnalysis::IndependentT(v) => &mut v.diagnostics,
        CompletedAnalysis::Frequencies(v) => &mut v.diagnostics,
        CompletedAnalysis::Histogram(v) => &mut v.diagnostics,
        CompletedAnalysis::Scatter(v) => &mut v.diagnostics,
        CompletedAnalysis::Boxplot(v) => &mut v.diagnostics,
        CompletedAnalysis::Anova(v) => &mut v.diagnostics,
        CompletedAnalysis::Reliability(v) => &mut v.diagnostics,
        CompletedAnalysis::Regression(v) => &mut v.diagnostics,
    }
}
// Catches independent validators admitting weight diagnostics without a frequency role.
#[test]
fn unweighted_weight_diagnostics_are_rejected_by_model_and_project() {
    use tessera::project::Project;
    let dataset = data();
    let describe_request = DescribeRequest {
        columns: vec![ColumnId(1)],
        statistics: vec![Statistic::Mean],
        missing_policy: MissingPolicy::PerVariable,
        filter: RowFilter::default(),
        frequency_weight: None,
    };
    let descriptive: CompletedAnalysis =
        describe(&dataset, &describe_request, &JobControl::default())
            .unwrap()
            .into();
    let project = Project::new("Intrinsic counts", dataset).unwrap();
    for original in
        std::iter::once(descriptive).chain((0..4).map(|kind| count_result(kind, 4, false)))
    {
        assert!(original.validate().is_ok());
        for missing in [false, true] {
            let mut invalid = original.clone();
            if missing {
                diagnostics_mut(&mut invalid).missing_weight_rows = 1;
            } else {
                diagnostics_mut(&mut invalid).zero_weight_rows = 1;
            }
            assert!(
                invalid.validate().is_err(),
                "model accepted weight exclusion without weights"
            );
            assert!(
                project.with_run(invalid).is_err(),
                "project accepted weight exclusion without weights"
            );
        }
    }
}
// Catches using frequency-sized unweighted N and adding complete-case exclusions twice.
#[test]
fn unweighted_source_totals_have_checked_exact_boundaries_and_weighted_controls() {
    use tessera::operation::RunId;
    use tessera::project::{AnalysisRun, Project};
    let project = Project::new("Intrinsic counts", data()).unwrap();
    for kind in 0..4 {
        let exact = count_result(kind, MAX_CELLS as u64 - 4, false);
        assert!(
            exact.validate().is_ok(),
            "rejected exact source ceiling for variant {kind}"
        );
        assert!(project.with_run(exact).is_ok());
        for n in [MAX_CELLS as u64 - 3, MAX_CELLS as u64 + 1, MAX_FREQUENCY] {
            let invalid = count_result(kind, n, false);
            assert!(
                invalid.validate().is_err(),
                "accepted source total over ceiling for variant {kind}"
            );
            assert!(project.with_run(invalid.clone()).is_err());
            assert!(Project::from_parts(
                "Invalid",
                data(),
                vec![Arc::new(AnalysisRun {
                    id: RunId(1),
                    analysis: invalid
                })],
                RunId(2)
            )
            .is_err());
        }
        let mut weighted = count_result(kind, MAX_FREQUENCY, true);
        diagnostics_mut(&mut weighted).missing_weight_rows = 1;
        diagnostics_mut(&mut weighted).zero_weight_rows = 1;
        assert!(
            weighted.validate().is_ok(),
            "frequency replication was source-capped for variant {kind}"
        );
        assert!(project.with_run(weighted).is_ok());
    }
}
