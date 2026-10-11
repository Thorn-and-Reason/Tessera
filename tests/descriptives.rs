mod common;
use tessera::{analysis::*, data::*, error::AppError, operation::JobControl};

fn request(ids: &[u64]) -> DescribeRequest {
    DescribeRequest {
        columns: ids.iter().copied().map(ColumnId).collect(),
        statistics: vec![
            Statistic::Mean,
            Statistic::SampleStdDev,
            Statistic::Variance,
            Statistic::Minimum,
            Statistic::Maximum,
            Statistic::Sum,
            Statistic::Range,
            Statistic::StdErrorMean,
        ],
        missing_policy: MissingPolicy::PerVariable,
        filter: RowFilter::default(),
        frequency_weight: None,
    }
}
fn dataset(values: Vec<Option<f64>>) -> Dataset {
    Dataset::new(vec![Column::numeric(ColumnId(1), "Value", values).unwrap()]).unwrap()
}
fn run(data: &Dataset, req: &DescribeRequest) -> ComputedAnalysis {
    describe(data, req, &JobControl::default()).unwrap()
}
fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= expected.abs() * 3e-15 + f64::from_bits(1),
        "{actual:?} != {expected:?}"
    );
}
fn value(analysis: &ComputedAnalysis, row: usize, stat: Statistic) -> f64 {
    analysis.rows[row].estimates[&stat].value.unwrap()
}

// Catches population denominators, rounding before calculation and collapsed duplicate points.
#[test]
fn approved_sample_matches_independent_values() {
    let analysis = run(&common::sample_dataset(), &request(&[6, 2]));
    close(value(&analysis, 0, Statistic::Mean), 23.333333333333332);
    close(
        value(&analysis, 0, Statistic::SampleStdDev),
        4.844240566555986,
    );
    close(value(&analysis, 1, Statistic::Mean), 20.333333333333332);
    close(
        value(&analysis, 1, Statistic::SampleStdDev),
        2.581988897471611,
    );
    assert_eq!(
        analysis.rows[0].estimates[&Statistic::Range].value,
        Some(14.0)
    );
    assert_eq!(
        analysis.plots[0]
            .points
            .iter()
            .filter(|p| p.value == 25.0)
            .count(),
        2
    );
    assert_eq!(analysis.rows[0].valid_n, 6);
}
// Catches treating integer frequency weights as reliability weights or ignoring weights.
#[test]
fn weights_match_expanded_observations() {
    let data = Dataset::new(vec![
        Column::numeric(ColumnId(1), "X", vec![Some(1.0), Some(3.0), Some(8.0)]).unwrap(),
        Column::numeric(ColumnId(2), "W", vec![Some(2.0), Some(1.0), Some(3.0)]).unwrap(),
    ])
    .unwrap();
    let mut req = request(&[1]);
    req.frequency_weight = Some(ColumnId(2));
    let weighted = run(&data, &req);
    close(value(&weighted, 0, Statistic::Mean), 4.833333333333333);
    assert_eq!(value(&weighted, 0, Statistic::Sum), 29.0);
    let expanded = run(
        &dataset(vec![
            Some(1.0),
            Some(1.0),
            Some(3.0),
            Some(8.0),
            Some(8.0),
            Some(8.0),
        ]),
        &request(&[1]),
    );
    for stat in &req.statistics {
        close(value(&weighted, 0, *stat), value(&expanded, 0, *stat));
    }
    assert_eq!(weighted.rows[0].valid_n, 6);
    assert_eq!(weighted.plots[0].points[2].frequency, 3);
}
// Catches applying complete case before filtering, ignoring user missing rules and mislabelling exclusions.
#[test]
fn missing_policies_and_filters() {
    let x = Column::numeric(
        ColumnId(1),
        "X",
        vec![Some(1.0), Some(-99.0), Some(3.0), None],
    )
    .unwrap();
    let x = x
        .with_metadata(ColumnMetadata {
            missing_rules: MissingRules::Number {
                values: vec![-99.0],
                ranges: vec![],
            },
            ..x.metadata().clone()
        })
        .unwrap();
    let data = Dataset::new(vec![
        x,
        Column::numeric(
            ColumnId(2),
            "Y",
            vec![None, Some(2.0), Some(4.0), Some(5.0)],
        )
        .unwrap(),
        Column::text(
            ColumnId(3),
            "Group",
            vec![
                Some("yes".into()),
                Some("yes".into()),
                Some("yes".into()),
                Some("no".into()),
            ],
        )
        .unwrap(),
    ])
    .unwrap();
    let mut req = request(&[1, 2]);
    req.filter.predicates.push(RowPredicate::Text {
        column: ColumnId(3),
        value: "yes".into(),
        equal: true,
    });
    let per = run(&data, &req);
    assert_eq!((per.rows[0].valid_n, per.rows[0].missing_n), (2, 1));
    assert_eq!(per.diagnostics.filtered_rows, 1);
    req.missing_policy = MissingPolicy::CompleteCase;
    let complete = run(&data, &req);
    assert_eq!(
        (
            complete.rows[0].valid_n,
            complete.rows[0].missing_n,
            complete.rows[0].complete_case_excluded_n
        ),
        (1, 1, 1)
    );
    assert_eq!(
        (
            complete.rows[1].missing_n,
            complete.rows[1].complete_case_excluded_n
        ),
        (1, 1)
    );
    assert_eq!(value(&complete, 0, Statistic::Mean), 3.0);
}
// Catches tiny variance underflow, large-offset cancellation and accidental all-estimate failure.
#[test]
fn constant_small_n_and_extreme_scale() {
    for values in [vec![1e308, 1e308], vec![4.0, 4.0]] {
        let a = run(
            &dataset(values.into_iter().map(Some).collect()),
            &request(&[1]),
        );
        assert_eq!(value(&a, 0, Statistic::SampleStdDev), 0.0);
    }
    let one = run(&dataset(vec![Some(7.0)]), &request(&[1]));
    assert_eq!(one.rows[0].estimates[&Statistic::SampleStdDev].value, None);
    assert!(one.rows[0].estimates[&Statistic::SampleStdDev]
        .reason
        .is_some());
    let a = run(&dataset(vec![Some(1e-200), Some(2e-200)]), &request(&[1]));
    close(value(&a, 0, Statistic::Mean), 1.5e-200);
    close(value(&a, 0, Statistic::Sum), 3e-200);
    close(
        value(&a, 0, Statistic::SampleStdDev),
        7.071067811865475e-201,
    );
    close(value(&a, 0, Statistic::StdErrorMean), 5e-201);
    assert_eq!(a.rows[0].estimates[&Statistic::Variance].value, None);
    for (magnitude, sd, sem) in [
        (1e308, Some(1.4142135623730951e308), 1e308),
        (f64::MAX, None, f64::MAX),
    ] {
        let a = run(
            &dataset(vec![Some(-magnitude), Some(magnitude)]),
            &request(&[1]),
        );
        assert_eq!(value(&a, 0, Statistic::Mean), 0.0);
        assert_eq!(value(&a, 0, Statistic::Sum), 0.0);
        match sd {
            Some(sd) => close(value(&a, 0, Statistic::SampleStdDev), sd),
            None => assert!(a.rows[0].estimates[&Statistic::SampleStdDev]
                .reason
                .is_some()),
        }
        close(value(&a, 0, Statistic::StdErrorMean), sem);
    }
    let a = run(
        &dataset(vec![Some(1e12 + 1.0), Some(1e12 + 2.0), Some(1e12 + 3.0)]),
        &request(&[1]),
    );
    close(value(&a, 0, Statistic::SampleStdDev), 1.0);
    close(value(&a, 0, Statistic::StdErrorMean), 0.5773502691896258);
    for values in [
        [1e308, -1e308, 1e-200],
        [1e308, 1e-200, -1e308],
        [1e-200, -1e308, 1e308],
    ] {
        let a = run(
            &dataset(values.into_iter().map(Some).collect()),
            &request(&[1]),
        );
        close(value(&a, 0, Statistic::Mean), 3.333333333333333e-201);
        close(value(&a, 0, Statistic::Sum), 1e-200);
        close(value(&a, 0, Statistic::SampleStdDev), 1e308);
        close(value(&a, 0, Statistic::StdErrorMean), 5.773502691896258e307);
    }
    let missing = run(&dataset(vec![None, None]), &request(&[1]));
    assert_eq!((missing.rows[0].valid_n, missing.rows[0].missing_n), (0, 2));
    assert!(missing.rows[0]
        .estimates
        .values()
        .all(|e| e.value.is_none() && e.reason.is_some()));
}
// Catches invalid typed assignments, invalid eligible weights, integer count loss and cancellation.
#[test]
fn invalid_requests_and_overflow() {
    let data = common::sample_dataset();
    for ids in [&[6, 6][..], &[999][..], &[1][..], &[][..]] {
        assert!(describe(&data, &request(ids), &JobControl::default()).is_err());
    }
    let mut req = request(&[6]);
    req.statistics.push(Statistic::Mean);
    assert!(describe(&data, &req, &JobControl::default()).is_err());
    let control = JobControl::default();
    control.cancel();
    assert!(matches!(
        describe(&data, &request(&[6]), &control),
        Err(AppError::Cancelled)
    ));
    for weight in [-1.0, 0.5, 9007199254740994.0] {
        let data = Dataset::new(vec![
            Column::numeric(ColumnId(1), "X", vec![Some(1.0)]).unwrap(),
            Column::numeric(ColumnId(2), "W", vec![Some(weight)]).unwrap(),
        ])
        .unwrap();
        let mut req = request(&[1]);
        req.frequency_weight = Some(ColumnId(2));
        assert!(describe(&data, &req, &JobControl::default()).is_err());
    }
    let a = run(&dataset(vec![Some(1e308), Some(1e308)]), &request(&[1]));
    assert_eq!(a.rows[0].estimates[&Statistic::Sum].value, None);
    assert_eq!(value(&a, 0, Statistic::Mean), 1e308);
}
#[test]
fn zero_missing_weights_and_filter_eligibility() {
    let data = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "X",
            vec![Some(1.0), Some(2.0), Some(3.0), Some(4.0)],
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "W",
            vec![Some(0.0), None, Some(2.0), Some(-1.0)],
        )
        .unwrap(),
    ])
    .unwrap();
    let mut req = request(&[1]);
    req.frequency_weight = Some(ColumnId(2));
    req.filter.predicates.push(RowPredicate::Number {
        column: ColumnId(1),
        comparison: NumericComparison::Less,
        value: 4.0,
    });
    let a = run(&data, &req);
    assert_eq!(a.rows[0].valid_n, 2);
    assert_eq!(value(&a, 0, Statistic::Mean), 3.0);
    assert_eq!(
        (
            a.diagnostics.zero_weight_rows,
            a.diagnostics.missing_weight_rows,
            a.diagnostics.filtered_rows
        ),
        (1, 1, 1)
    );
}
#[test]
fn plot_reduction_leaves_full_statistics() {
    let a = run(
        &dataset((0..5000).map(|n| Some(n as f64)).collect()),
        &request(&[1]),
    );
    assert_eq!(a.plots[0].points.len(), 2000);
    assert!(a.plots[0].reduced);
    assert_eq!(a.plots[0].source_rows, 5000);
    assert_eq!(value(&a, 0, Statistic::Mean), 2499.5);
    assert_eq!(a.rows[0].valid_n, 5000);
}
// Catches variance about a rounded raw mean instead of the mathematical centre.
#[test]
fn adjacent_large_values_and_subnormal_spread() {
    let a = run(&dataset(vec![Some(1e16), Some(1e16 + 2.0)]), &request(&[1]));
    close(
        value(&a, 0, Statistic::SampleStdDev),
        std::f64::consts::SQRT_2,
    );
    let tiny = f64::from_bits(1);
    let a = run(&dataset(vec![Some(tiny), Some(2.0 * tiny)]), &request(&[1]));
    assert_eq!(value(&a, 0, Statistic::SampleStdDev), tiny);
    assert!(a.rows[0].estimates[&Statistic::StdErrorMean]
        .reason
        .is_some());
}
#[test]
fn numeric_and_missing_filter_contracts() {
    let data = dataset(vec![None, Some(1.0), Some(2.0), Some(3.0)]);
    for (comparison, expected) in [
        (NumericComparison::Equal, 1),
        (NumericComparison::NotEqual, 2),
        (NumericComparison::Less, 1),
        (NumericComparison::LessOrEqual, 2),
        (NumericComparison::Greater, 1),
        (NumericComparison::GreaterOrEqual, 2),
    ] {
        let mut req = request(&[1]);
        req.filter.predicates.push(RowPredicate::Number {
            column: ColumnId(1),
            comparison,
            value: 2.0,
        });
        assert_eq!(run(&data, &req).rows[0].valid_n, expected);
    }
    let mut req = request(&[1]);
    req.filter.predicates.push(RowPredicate::Missing {
        column: ColumnId(1),
        missing: true,
    });
    let a = run(&data, &req);
    assert_eq!((a.rows[0].valid_n, a.rows[0].missing_n), (0, 1));
    req.filter.predicates = vec![RowPredicate::Missing {
        column: ColumnId(1),
        missing: false,
    }];
    assert_eq!(run(&data, &req).rows[0].valid_n, 3);
    req.filter.predicates = vec![RowPredicate::Number {
        column: ColumnId(1),
        comparison: NumericComparison::Equal,
        value: f64::NAN,
    }];
    assert!(describe(&data, &req, &JobControl::default()).is_err());
    req.filter.predicates = vec![RowPredicate::Text {
        column: ColumnId(1),
        value: "2".into(),
        equal: true,
    }];
    assert!(describe(&data, &req, &JobControl::default()).is_err());
}
#[test]
fn missing_ranges_text_tokens_and_integer_frequency_ceiling() {
    let x = Column::numeric(
        ColumnId(1),
        "X",
        vec![None, Some(1.0), Some(2.0), Some(3.0)],
    )
    .unwrap();
    let x = x
        .with_metadata(ColumnMetadata {
            missing_rules: MissingRules::Number {
                values: vec![],
                ranges: vec![(1.0, 2.0)],
            },
            ..x.metadata().clone()
        })
        .unwrap();
    let group = Column::text(
        ColumnId(2),
        "Group",
        vec![None, Some("a".into()), Some("".into()), Some("b".into())],
    )
    .unwrap();
    let group = group
        .with_metadata(ColumnMetadata {
            missing_rules: MissingRules::Text(vec!["".into()]),
            ..group.metadata().clone()
        })
        .unwrap();
    let data = Dataset::new(vec![x, group]).unwrap();
    let mut req = request(&[1]);
    req.filter.predicates.push(RowPredicate::Text {
        column: ColumnId(2),
        value: "a".into(),
        equal: false,
    });
    let a = run(&data, &req);
    assert_eq!((a.rows[0].valid_n, a.rows[0].missing_n), (1, 0));
    assert_eq!(value(&a, 0, Statistic::Mean), 3.0);
    let data = Dataset::new(vec![
        Column::numeric(ColumnId(1), "X", vec![Some(2.0), None]).unwrap(),
        Column::numeric(
            ColumnId(2),
            "Weight",
            vec![Some(9007199254740992.0), Some(0.0)],
        )
        .unwrap(),
    ])
    .unwrap();
    let mut req = request(&[1]);
    req.frequency_weight = Some(ColumnId(2));
    let a = run(&data, &req);
    assert_eq!(a.rows[0].valid_n, 9007199254740992);
    assert_eq!(value(&a, 0, Statistic::Mean), 2.0);
    let over = data.edit_cell(ColumnId(2), 1, Cell::Number(1.0)).unwrap();
    assert!(describe(&over, &req, &JobControl::default()).is_err());
    assert!(Column::numeric(ColumnId(1), "Invalid weight", vec![Some(f64::INFINITY)]).is_err());
}
#[test]
fn retained_history_budget_rejects_before_selection_work() {
    use std::sync::{atomic::AtomicBool, mpsc, Arc};
    let data = common::sample_dataset();
    let retained = data.edit_cell(ColumnId(6), 0, Cell::Number(18.0)).unwrap();
    let already = Budget::dataset_bytes(&[&data, &retained], 0).unwrap();
    let (sender, receiver) = mpsc::channel();
    let control = JobControl::new(Arc::new(AtomicBool::new(false)), Some(sender));
    let result = describe_with_budget(
        &data,
        &request(&[6]),
        &control,
        &[&retained],
        MAX_OWNED_BYTES - already - 1,
    );
    assert!(matches!(result, Err(AppError::SizeLimit { .. })));
    assert!(receiver.try_iter().all(|progress| progress.done == 0));
    assert!(describe_with_budget(
        &data,
        &request(&[6]),
        &JobControl::default(),
        &[],
        usize::MAX
    )
    .is_err());
}
#[test]
fn progress_remains_monotonic_across_selected_columns() {
    use std::sync::{atomic::AtomicBool, mpsc, Arc};
    let (sender, receiver) = mpsc::channel();
    let control = JobControl::new(Arc::new(AtomicBool::new(false)), Some(sender));
    describe(&common::sample_dataset(), &request(&[6, 2]), &control).unwrap();
    let progress = receiver.try_iter().collect::<Vec<_>>();
    assert!(progress.windows(2).all(|pair| pair[0].done <= pair[1].done));
    assert_eq!(
        progress.last().unwrap().total,
        Some(progress.last().unwrap().done)
    );
}

struct AllocationTracker;
std::thread_local! {
    static MEASURING_ALLOCATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static LARGEST_ALLOCATION: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[global_allocator]
static ALLOCATION_TRACKER: AllocationTracker = AllocationTracker;
// Tests observe allocation sizes on their own thread; System owns the memory.
unsafe impl std::alloc::GlobalAlloc for AllocationTracker {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        record_allocation(layout.size());
        unsafe { std::alloc::GlobalAlloc::alloc(&std::alloc::System, layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::GlobalAlloc::dealloc(&std::alloc::System, pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: std::alloc::Layout, size: usize) -> *mut u8 {
        record_allocation(size);
        unsafe { std::alloc::GlobalAlloc::realloc(&std::alloc::System, pointer, layout, size) }
    }
}
fn record_allocation(bytes: usize) {
    let _ = MEASURING_ALLOCATION.try_with(|active| {
        if active.get() {
            let _ = LARGEST_ALLOCATION.try_with(|largest| largest.set(largest.get().max(bytes)));
        }
    });
}
#[test]
fn validation_trees_are_reserved_before_allocation() {
    let data = dataset(vec![]);
    let mut req = request(&[1]);
    req.statistics = vec![Statistic::Mean];
    let control = JobControl::default();
    let headroom = 80;
    let reference_bytes = std::mem::size_of::<&Dataset>();
    let existing = Budget::dataset_bytes(&[&data], reference_bytes).unwrap();
    let extra_bytes = MAX_OWNED_BYTES - existing - headroom;
    // Existing data and the temporary reference vector fit; a validation tree does not.
    assert!(Budget::check_datasets(&[&data], extra_bytes + reference_bytes).is_ok());
    LARGEST_ALLOCATION.with(|largest| largest.set(0));
    MEASURING_ALLOCATION.with(|active| active.set(true));
    let result = describe_with_budget(&data, &req, &control, &[], extra_bytes);
    MEASURING_ALLOCATION.with(|active| active.set(false));
    let largest = LARGEST_ALLOCATION.with(std::cell::Cell::get);
    assert!(matches!(result, Err(AppError::SizeLimit { .. })));
    assert!(
        largest <= headroom,
        "Validation allocated {largest} bytes despite only {headroom} bytes of headroom."
    );
}
