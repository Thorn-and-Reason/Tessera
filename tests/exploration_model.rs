use tessera::{analysis::*, data::*};
fn e(value: f64) -> Estimate {
    Estimate {
        value: Some(value),
        reason: None,
    }
}
fn unavailable() -> Estimate {
    Estimate {
        value: None,
        reason: Some("No eligible observations.".into()),
    }
}
fn info(id: u64, kind: ColumnKind) -> RunColumnInfo {
    RunColumnInfo {
        id: ColumnId(id),
        name: format!("V{id}"),
        kind,
        description: String::new(),
        missing_rules: MissingRules::None,
        value_labels: vec![],
    }
}
fn frequencies() -> ComputedFrequencies {
    ComputedFrequencies {
        revision: RevisionId(1),
        request: FrequenciesRequest {
            columns: vec![ColumnId(1)],
            filter: RowFilter::default(),
            frequency_weight: None,
        },
        columns: vec![info(1, ColumnKind::Number)],
        source_rows: 3,
        diagnostics: RunDiagnostics::default(),
        tables: vec![FrequencyTable {
            column: ColumnId(1),
            valid_rows: 2,
            missing_rows: 1,
            valid_n: 2,
            missing_n: 1,
            missing_percent: e(100.0 / 3.0),
            categories: vec![
                FrequencyCategory {
                    value: Cell::Number(0.0),
                    value_label_index: None,
                    rows: 1,
                    frequency: 1,
                    percent: e(100.0 / 3.0),
                    valid_percent: e(50.0),
                    cumulative_frequency: Some(1),
                    cumulative_percent: Some(e(50.0)),
                },
                FrequencyCategory {
                    value: Cell::Number(2.0),
                    value_label_index: None,
                    rows: 1,
                    frequency: 1,
                    percent: e(100.0 / 3.0),
                    valid_percent: e(50.0),
                    cumulative_frequency: Some(2),
                    cumulative_percent: Some(e(100.0)),
                },
            ],
        }],
    }
}
fn histogram() -> ComputedHistogram {
    ComputedHistogram {
        revision: RevisionId(1),
        request: HistogramRequest {
            variable: ColumnId(1),
            bins: HistogramBins::Count(2),
            filter: RowFilter::default(),
            frequency_weight: None,
        },
        columns: vec![info(1, ColumnKind::Number)],
        source_rows: 3,
        diagnostics: RunDiagnostics::default(),
        valid_rows: 2,
        n: 2,
        missing_rows: 1,
        missing_n: 1,
        range: [e(0.0), e(2.0)],
        bins: vec![
            HistogramBin {
                left: 0.0,
                right: 1.0,
                rows: 1,
                frequency: 1,
            },
            HistogramBin {
                left: 1.0,
                right: 2.0,
                rows: 1,
                frequency: 1,
            },
        ],
        bin_adjustment: None,
    }
}
fn scatter() -> ComputedScatter {
    ComputedScatter {
        revision: RevisionId(1),
        request: ScatterRequest {
            x: ColumnId(1),
            y: ColumnId(2),
            filter: RowFilter::default(),
            frequency_weight: None,
        },
        columns: vec![info(1, ColumnKind::Number), info(2, ColumnKind::Number)],
        source_rows: 3,
        diagnostics: RunDiagnostics::default(),
        valid_rows: 2,
        n: 2,
        missing_rows: 1,
        missing_n: 1,
        ranges: [[e(0.0), e(2.0)], [e(3.0), e(4.0)]],
        points: vec![
            ScatterPoint {
                row: 0,
                x: 0.0,
                y: 3.0,
                frequency: 1,
            },
            ScatterPoint {
                row: 2,
                x: 2.0,
                y: 4.0,
                frequency: 1,
            },
        ],
        reduced: false,
    }
}
fn boxplot() -> ComputedBoxplot {
    ComputedBoxplot {
        revision: RevisionId(1),
        request: BoxplotRequest {
            outcome: ColumnId(1),
            group: None,
            filter: RowFilter::default(),
            frequency_weight: None,
        },
        columns: vec![info(1, ColumnKind::Number)],
        source_rows: 2,
        diagnostics: RunDiagnostics::default(),
        missing_group_rows: 0,
        missing_group_n: 0,
        missing_outcome_rows: 0,
        missing_outcome_n: 0,
        groups: vec![BoxplotGroup {
            value: None,
            value_label_index: None,
            rows: 2,
            n: 2,
            minimum: e(0.0),
            q1: e(0.5),
            median: e(1.0),
            q3: e(1.5),
            maximum: e(2.0),
            whiskers: [e(0.0), e(2.0)],
            outlier_rows: 0,
            outlier_n: 0,
        }],
        points: vec![],
        reduced: false,
        empty_reason: None,
    }
}
#[test]
fn current_roles_bins_shared_filters_and_budget() {
    let dataset = Dataset::new(vec![
        Column::numeric(ColumnId(1), "X", vec![Some(1.0)]).unwrap(),
        Column::text(ColumnId(2), "Group", vec![Some("A".into())]).unwrap(),
    ])
    .unwrap();
    let base = frequencies().request;
    for columns in [
        vec![],
        vec![ColumnId(0)],
        vec![ColumnId(1), ColumnId(1)],
        vec![ColumnId(99)],
    ] {
        assert!(FrequenciesRequest {
            columns,
            ..base.clone()
        }
        .validate_current(&dataset, &[], 0)
        .is_err());
    }
    assert!(FrequenciesRequest {
        columns: vec![ColumnId(1), ColumnId(2)],
        ..base.clone()
    }
    .validate_current(&dataset, &[], 0)
    .is_ok());
    for bins in [HistogramBins::Count(0), HistogramBins::Count(129)] {
        assert!(HistogramRequest {
            bins,
            ..histogram().request
        }
        .validate_current(&dataset, &[], 0)
        .is_err());
    }
    assert!(HistogramRequest {
        variable: ColumnId(2),
        ..histogram().request
    }
    .validate_current(&dataset, &[], 0)
    .is_err());
    assert!(ScatterRequest {
        y: ColumnId(1),
        ..scatter().request
    }
    .validate_current(&dataset, &[], 0)
    .is_err());
    assert!(BoxplotRequest {
        group: Some(ColumnId(1)),
        ..boxplot().request
    }
    .validate_current(&dataset, &[], 0)
    .is_err());
    assert!(BoxplotRequest {
        group: Some(ColumnId(2)),
        ..boxplot().request
    }
    .validate_current(&dataset, &[], 0)
    .is_ok());
    for filter in [
        RowFilter {
            predicates: vec![RowPredicate::Number {
                column: ColumnId(2),
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
            predicates: vec![RowPredicate::Missing {
                column: ColumnId(0),
                missing: true,
            }],
        },
    ] {
        assert!(FrequenciesRequest {
            filter,
            ..base.clone()
        }
        .validate_current(&dataset, &[], 0)
        .is_err());
    }
    assert!(FrequenciesRequest {
        frequency_weight: Some(ColumnId(2)),
        ..base.clone()
    }
    .validate_current(&dataset, &[], 0)
    .is_err());
    assert!(base
        .validate_current(&dataset, &[], MAX_OWNED_BYTES)
        .is_err());
    assert!(base.validate_current(&dataset, &[], usize::MAX).is_err());
    assert!(base.heap_bytes().unwrap() > 0);
    assert!(base.validation_scratch_bytes().unwrap() > 0);
    assert_eq!(
        base.validate_current(&dataset, &[&dataset], 0)
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn frequencies_exact_identity_denominators_and_captured_labels() {
    let base = frequencies();
    assert!(base.validate().is_ok());
    for mode in 0..12 {
        let mut bad = base.clone();
        match mode {
            0 => bad.tables[0].categories[1].value = Cell::Number(-0.0),
            1 => bad.tables[0].categories.reverse(),
            2 => bad.tables[0].valid_n = 3,
            3 => bad.tables[0].categories[0].percent = e(50.0),
            4 => bad.tables[0].categories[0].valid_percent = e(100.0 / 3.0),
            5 => bad.tables[0].missing_percent = e(50.0),
            6 => bad.tables[0].categories[1].cumulative_frequency = Some(1),
            7 => bad.tables[0].categories[0].cumulative_percent = None,
            8 => bad.tables[0].categories[0].value_label_index = Some(0),
            9 => bad.tables[0].categories[0].value = Cell::Missing,
            10 => bad.source_rows = 4,
            _ => bad.tables[0].categories[0].rows = 0,
        }
        assert!(
            bad.validate().is_err(),
            "accepted frequencies corruption {mode}"
        );
    }
    let mut multi = base.clone();
    multi.request.columns.push(ColumnId(2));
    multi.columns.push(info(2, ColumnKind::Number));
    multi.tables.push(FrequencyTable {
        column: ColumnId(2),
        ..multi.tables[0].clone()
    });
    assert!(multi.validate().is_ok());
    multi.request.frequency_weight = Some(ColumnId(2));
    multi.tables[1].valid_n = 3;
    multi.tables[1].categories[0].frequency = 2;
    multi.tables[1].missing_percent = e(25.0);
    for (i, c) in multi.tables[1].categories.iter_mut().enumerate() {
        c.percent = e(100.0 * c.frequency as f64 / 4.0);
        c.valid_percent = e(100.0 * c.frequency as f64 / 3.0);
        c.cumulative_frequency = Some(2 + i as u64);
        c.cumulative_percent = Some(e(100.0 * (2 + i) as f64 / 3.0));
    }
    assert!(multi.validate().is_err());
    let mut labeled = base.clone();
    labeled.columns[0].value_labels = vec![
        ValueLabel {
            value: Cell::Number(-0.0),
            label: "Same caption".into(),
        },
        ValueLabel {
            value: Cell::Number(2.0),
            label: "Same caption".into(),
        },
    ];
    assert!(labeled.validate().is_err());
    labeled.tables[0].categories[0].value_label_index = Some(0);
    labeled.tables[0].categories[1].value_label_index = Some(1);
    assert!(labeled.validate().is_ok());
    labeled.tables[0].categories[1].value_label_index = Some(0);
    assert!(labeled.validate().is_err());
    let mut text = base;
    text.columns[0].kind = ColumnKind::Text;
    for (category, value) in text.tables[0].categories.iter_mut().zip(["", " A\t"]) {
        category.value = Cell::Text(value.into());
        category.cumulative_frequency = None;
        category.cumulative_percent = None;
    }
    assert!(text.validate().is_ok());
    text.tables[0].categories[1].value = Cell::Text(String::new());
    assert!(text.validate().is_err());
}
#[test]
fn histogram_boundaries_totals_and_empty_constant_shapes() {
    let base = histogram();
    assert!(base.validate().is_ok());
    for mode in 0..8 {
        let mut bad = base.clone();
        match mode {
            0 => bad.bins[0].left = f64::NEG_INFINITY,
            1 => bad.bins[0].right = -1.0,
            2 => bad.bins[1].left = 1.1,
            3 => bad.bins[1].right = 3.0,
            4 => bad.bins[0].frequency = 2,
            5 => bad.range[0] = unavailable(),
            6 => bad.bin_adjustment = Some(" ".into()),
            _ => bad.bins[0].rows = 0,
        }
        assert!(
            bad.validate().is_err(),
            "accepted histogram corruption {mode}"
        );
    }
    let mut constant = base.clone();
    constant.range = [e(2.0), e(2.0)];
    constant.bins = vec![HistogramBin {
        left: 2.0,
        right: 2.0,
        rows: 2,
        frequency: 2,
    }];
    assert!(constant.validate().is_ok());
    let mut empty = base;
    empty.source_rows = 0;
    empty.valid_rows = 0;
    empty.n = 0;
    empty.missing_rows = 0;
    empty.missing_n = 0;
    empty.range = [unavailable(), unavailable()];
    empty.bins.clear();
    assert!(empty.validate().is_ok());
}
#[test]
fn scatter_points_source_identity_ranges_and_reduction() {
    let base = scatter();
    assert!(base.validate().is_ok());
    for mode in 0..8 {
        let mut bad = base.clone();
        match mode {
            0 => bad.points[1].row = 0,
            1 => bad.points[1].row = 3,
            2 => bad.points[1].x = 3.0,
            3 => bad.points[0].frequency = 0,
            4 => bad.points[0].y = f64::NAN,
            5 => bad.reduced = true,
            6 => bad.ranges[0][1] = e(3.0),
            _ => bad.points.reverse(),
        }
        assert!(
            bad.validate().is_err(),
            "accepted scatter corruption {mode}"
        );
    }
    let mut empty = base.clone();
    empty.source_rows = 0;
    empty.valid_rows = 0;
    empty.n = 0;
    empty.missing_rows = 0;
    empty.missing_n = 0;
    empty.points.clear();
    empty.ranges = [
        [unavailable(), unavailable()],
        [unavailable(), unavailable()],
    ];
    assert!(empty.validate().is_ok());
    let mut singleton = base;
    singleton.source_rows = 1;
    singleton.valid_rows = 1;
    singleton.n = 1;
    singleton.missing_rows = 0;
    singleton.missing_n = 0;
    singleton.points.truncate(1);
    singleton.ranges = [[e(0.0), e(0.0)], [e(3.0), e(3.0)]];
    assert!(singleton.validate().is_ok());
}
#[test]
fn boxplot_order_exclusions_groups_and_empty_shapes() {
    let base = boxplot();
    assert!(base.validate().is_ok());
    for mode in 0..9 {
        let mut bad = base.clone();
        match mode {
            0 => bad.groups[0].q1 = e(1.1),
            1 => bad.groups[0].whiskers[0] = e(0.6),
            2 => bad.groups[0].maximum = e(1.0),
            3 => bad.groups.clear(),
            4 => bad.groups[0].value = Some(Cell::Number(1.0)),
            5 => bad.missing_group_rows = 1,
            6 => bad.missing_outcome_rows = 1,
            7 => bad.groups[0].outlier_n = 1,
            _ => bad.empty_reason = Some("Empty".into()),
        }
        assert!(
            bad.validate().is_err(),
            "accepted boxplot corruption {mode}"
        );
    }
    let mut grouped = base.clone();
    grouped.request.group = Some(ColumnId(2));
    grouped.columns.push(info(2, ColumnKind::Text));
    grouped.groups[0].value = Some(Cell::Text("A".into()));
    assert!(grouped.validate().is_ok());
    grouped.groups[0].rows = 0;
    grouped.groups[0].n = 0;
    assert!(grouped.validate().is_err());
    let mut empty = base;
    empty.source_rows = 0;
    let g = &mut empty.groups[0];
    g.rows = 0;
    g.n = 0;
    g.minimum = unavailable();
    g.q1 = unavailable();
    g.median = unavailable();
    g.q3 = unavailable();
    g.maximum = unavailable();
    g.whiskers = [unavailable(), unavailable()];
    empty.empty_reason = Some("No eligible outcomes.".into());
    assert!(empty.validate().is_ok());
    let mut empty_grouped = empty.clone();
    empty_grouped.request.group = Some(ColumnId(2));
    empty_grouped.columns.push(info(2, ColumnKind::Text));
    empty_grouped.groups.clear();
    assert!(empty_grouped.validate().is_ok());
    empty_grouped.empty_reason = None;
    assert!(empty_grouped.validate().is_err());
    let mut singleton = boxplot();
    singleton.source_rows = 1;
    let g = &mut singleton.groups[0];
    g.rows = 1;
    g.n = 1;
    g.minimum = e(2.0);
    g.q1 = e(2.0);
    g.median = e(2.0);
    g.q3 = e(2.0);
    g.maximum = e(2.0);
    g.whiskers = [e(2.0), e(2.0)];
    assert!(singleton.validate().is_ok());
}
#[test]
fn weighted_large_n_and_unsupported_diagnostics() {
    for mode in 0..3 {
        let diagnostics = match mode {
            0 => RunDiagnostics {
                missing_weight_rows: 1,
                ..RunDiagnostics::default()
            },
            1 => RunDiagnostics {
                zero_weight_rows: 1,
                ..RunDiagnostics::default()
            },
            _ => RunDiagnostics {
                complete_case_rows: 1,
                ..RunDiagnostics::default()
            },
        };
        assert!(ComputedFrequencies {
            diagnostics: diagnostics.clone(),
            ..frequencies()
        }
        .validate()
        .is_err());
        assert!(ComputedHistogram {
            diagnostics: diagnostics.clone(),
            ..histogram()
        }
        .validate()
        .is_err());
        assert!(ComputedScatter {
            diagnostics: diagnostics.clone(),
            ..scatter()
        }
        .validate()
        .is_err());
        assert!(ComputedBoxplot {
            diagnostics,
            ..boxplot()
        }
        .validate()
        .is_err());
    }
    let mut weighted = histogram();
    weighted.request.frequency_weight = Some(ColumnId(2));
    weighted.columns.push(info(2, ColumnKind::Number));
    weighted.n = MAX_FREQUENCY - 1;
    weighted.bins[0].frequency = MAX_FREQUENCY - 2;
    weighted.missing_n = 1;
    assert!(weighted.validate().is_ok());
    weighted.n = MAX_FREQUENCY;
    weighted.bins[0].frequency = MAX_FREQUENCY - 1;
    assert!(weighted.validate().is_err());
}
#[test]
fn capacities_strings_reasons_and_snapshot_scratch_are_charged() {
    let base = frequencies();
    let bytes = base.owned_bytes().unwrap();
    let mut large = base.clone();
    large.columns[0].description = "Long captured description".repeat(100);
    assert!(large.owned_bytes().unwrap() > bytes + 1000);
    let mut reserved = base.clone();
    reserved.tables.reserve(20);
    reserved.tables[0].categories.reserve(20);
    assert!(reserved.owned_bytes().unwrap() > bytes);
    let scratch = base.validation_scratch_bytes().unwrap();
    let mut labeled = base;
    labeled.columns[0].value_labels.push(ValueLabel {
        value: Cell::Number(0.0),
        label: "Long label".repeat(100),
    });
    assert!(labeled.validation_scratch_bytes().unwrap() > scratch);
    assert!(histogram().owned_bytes().is_ok());
    assert!(scatter().owned_bytes().is_ok());
    assert!(boxplot().owned_bytes().is_ok());
}
#[test]
fn quartiles_outside_whiskers_and_interpolated_missing_intervals_are_valid() {
    let mut valid = boxplot();
    valid.source_rows = 4;
    valid.groups[0] = BoxplotGroup {
        value: None,
        value_label_index: None,
        rows: 4,
        n: 4,
        minimum: e(0.0),
        q1: e(0.75),
        median: e(1.5),
        q3: e(26.5),
        maximum: e(100.0),
        whiskers: [e(0.0), e(2.0)],
        outlier_rows: 1,
        outlier_n: 1,
    };
    valid.points = vec![BoxplotPoint {
        group_index: 0,
        row: 3,
        value: 100.0,
        frequency: 1,
    }];
    valid.columns[0].missing_rules = MissingRules::Number {
        values: vec![26.5],
        ranges: vec![(0.25, 0.9)],
    };
    assert!(valid.validate().is_ok());
    for mode in 0..6 {
        let mut bad = valid.clone();
        match mode {
            0 => bad.points[0].value = 99.0,
            1 => bad.points[0].value = 1.0,
            2 => bad.points[0].row = 4,
            3 => bad.groups[0].whiskers[1] = e(100.0),
            4 => bad.groups[0].outlier_rows = 2,
            _ => bad.points[0].group_index = 1,
        }
        assert!(
            bad.validate().is_err(),
            "accepted outlier corruption {mode}"
        );
    }
    // Reflection gives a lower whisker above Q1; both type-7 shapes remain valid.
    let mut lower = valid;
    lower.columns[0].missing_rules = MissingRules::None;
    let g = &mut lower.groups[0];
    g.q1 = e(73.5);
    g.median = e(98.5);
    g.q3 = e(99.25);
    g.whiskers = [e(98.0), e(100.0)];
    lower.points[0].value = 0.0;
    assert!(lower.validate().is_ok());
}
#[test]
fn captured_missing_rules_refuse_raw_values_but_not_bin_edges() {
    let mut f = frequencies();
    f.columns[0].missing_rules = MissingRules::Number {
        values: vec![-0.0],
        ranges: vec![],
    };
    assert!(f.validate().is_err());
    let mut h = histogram();
    h.columns[0].missing_rules = MissingRules::Number {
        values: vec![1.0],
        ranges: vec![(0.8, 1.2)],
    };
    assert!(h.validate().is_ok());
    h.columns[0].missing_rules = MissingRules::Number {
        values: vec![],
        ranges: vec![(-1.0, 3.0), (0.5, 1.0)],
    };
    assert!(h.validate().is_err());
    let mut s = scatter();
    s.columns[1].missing_rules = MissingRules::Number {
        values: vec![4.0],
        ranges: vec![],
    };
    assert!(s.validate().is_err());
    let mut b = boxplot();
    b.request.group = Some(ColumnId(2));
    b.columns.push(info(2, ColumnKind::Text));
    b.groups[0].value = Some(Cell::Text(String::new()));
    assert!(b.validate().is_ok());
    b.columns[1].missing_rules = MissingRules::Text(vec![String::new()]);
    assert!(b.validate().is_err());
    let mut bad = boxplot();
    bad.columns[0].missing_rules = MissingRules::Number {
        values: vec![2.0],
        ranges: vec![],
    };
    assert!(bad.validate().is_err());
}

#[test]
fn reduced_scatter_keeps_full_bounds_and_replication_counts() {
    let mut valid = scatter();
    valid.source_rows = 2001;
    valid.valid_rows = 2001;
    valid.n = 2001;
    valid.missing_rows = 0;
    valid.missing_n = 0;
    valid.reduced = true;
    valid.ranges = [[e(-10.0), e(3000.0)], [e(2.0), e(2.0)]];
    valid.points = (0..MAX_PLOT_POINTS)
        .map(|i| ScatterPoint {
            row: i * 2000 / 1999,
            x: i as f64,
            y: 2.0,
            frequency: 1,
        })
        .collect();
    assert!(valid.validate().is_ok());
    valid.request.frequency_weight = Some(ColumnId(3));
    valid.columns.push(info(3, ColumnKind::Number));
    valid.n = MAX_FREQUENCY;
    valid.points[0].frequency = MAX_FREQUENCY - 2000;
    assert!(valid.validate().is_ok());
    valid.points[0].frequency += 1;
    assert!(valid.validate().is_err());
}

#[test]
fn weighted_maximum_constants_and_disjoint_exclusions() {
    let mut h = histogram();
    h.source_rows = 4;
    h.valid_rows = 1;
    h.n = MAX_FREQUENCY;
    h.missing_rows = 0;
    h.missing_n = 0;
    h.range = [e(2.0), e(2.0)];
    h.bins = vec![HistogramBin {
        left: 2.0,
        right: 2.0,
        rows: 1,
        frequency: MAX_FREQUENCY,
    }];
    h.request.frequency_weight = Some(ColumnId(2));
    h.columns.push(info(2, ColumnKind::Number));
    h.diagnostics = RunDiagnostics {
        filtered_rows: 1,
        missing_weight_rows: 1,
        zero_weight_rows: 1,
        complete_case_rows: 0,
    };
    assert!(h.validate().is_ok());
    h.diagnostics.filtered_rows += 1;
    assert!(h.validate().is_err());
    let mut b = boxplot();
    b.source_rows = 3;
    b.request.frequency_weight = Some(ColumnId(2));
    b.columns.push(info(2, ColumnKind::Number));
    b.groups[0].n = MAX_FREQUENCY - 1;
    b.missing_outcome_rows = 1;
    b.missing_outcome_n = 1;
    assert!(b.validate().is_ok());
    b.missing_group_rows = 1;
    b.missing_group_n = 1;
    assert!(b.validate().is_err());
    b.request.group = Some(ColumnId(3));
    b.columns.push(info(3, ColumnKind::Text));
    b.groups[0].value = Some(Cell::Text("A".into()));
    assert!(b.validate().is_err());
    b.source_rows = 4;
    b.groups[0].n -= 1;
    assert!(b.validate().is_ok());
    let mut f = frequencies();
    f.request.frequency_weight = Some(ColumnId(2));
    f.columns.push(info(2, ColumnKind::Number));
    let t = &mut f.tables[0];
    t.valid_n = MAX_FREQUENCY - 1;
    t.categories[0].frequency = MAX_FREQUENCY - 2;
    for (i, c) in t.categories.iter_mut().enumerate() {
        c.percent = e(100.0 * c.frequency as f64 / MAX_FREQUENCY as f64);
        c.valid_percent = e(100.0 * c.frequency as f64 / (MAX_FREQUENCY - 1) as f64);
        let cumulative = MAX_FREQUENCY - 2 + i as u64;
        c.cumulative_frequency = Some(cumulative);
        c.cumulative_percent = Some(e(100.0 * cumulative as f64 / (MAX_FREQUENCY - 1) as f64));
    }
    t.missing_percent = e(100.0 / MAX_FREQUENCY as f64);
    assert!(f.validate().is_ok());
}

#[test]
fn live_request_is_charged_once_and_missing_indexes_are_precharged() {
    let dataset = Dataset::new(vec![
        Column::numeric(ColumnId(1), "X", vec![Some(1.0)]).unwrap()
    ])
    .unwrap();
    let mut request = frequencies().request;
    request.columns.reserve(10000);
    let datasets = Budget::dataset_bytes(&[&dataset], 0).unwrap();
    let extra = MAX_OWNED_BYTES - datasets - request.validation_scratch_bytes().unwrap() - 512;
    assert!(request.heap_bytes().unwrap() > 512);
    assert!(request.validate_current(&dataset, &[], extra).is_ok());
    let original = frequencies();
    let scratch = original.validation_scratch_bytes().unwrap();
    let mut missing = original.clone();
    missing.columns[0].missing_rules = MissingRules::Number {
        values: vec![88.0; 10],
        ranges: vec![(88.0, 89.0); 20],
    };
    assert!(missing.validation_scratch_bytes().unwrap() >= scratch + 30 * 1024);
    missing.columns[0].value_labels = (0..1000)
        .map(|i| ValueLabel {
            value: Cell::Number(i as f64),
            label: "Repeated label".repeat(100),
        })
        .collect();
    missing.tables[0].categories[0].value_label_index = Some(0);
    missing.tables[0].categories[1].value_label_index = Some(2);
    assert!(missing.validate().is_ok());
    let mut empty = histogram();
    empty.source_rows = 0;
    empty.valid_rows = 0;
    empty.n = 0;
    empty.missing_rows = 0;
    empty.missing_n = 0;
    empty.bins.clear();
    empty.range = [unavailable(), unavailable()];
    let bytes = empty.owned_bytes().unwrap();
    empty.range[0].reason = Some("A long unavailable reason.".repeat(100));
    assert!(empty.owned_bytes().unwrap() > bytes + 1000);
}
#[path = "exploration_model/boundaries.rs"]
mod boundaries;
