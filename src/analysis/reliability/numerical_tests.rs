use crate::{
    analysis::reliability::*,
    data::{Column, ColumnId, Dataset},
    operation::JobControl,
};
fn source(rows: &[Vec<f64>]) -> (Dataset, ReliabilityRequest) {
    let k = rows.first().map_or(2, Vec::len);
    let d = Dataset::new(
        (0..k)
            .map(|i| {
                Column::numeric(
                    ColumnId(i as u64 + 1),
                    &format!("item {i}"),
                    rows.iter().map(|row| Some(row[i])).collect(),
                )
                .unwrap()
            })
            .collect(),
    )
    .unwrap();
    let r = ReliabilityRequest {
        items: (1..=k as u64).map(ColumnId).collect(),
        filter: Default::default(),
        frequency_weight: None,
        reversals: vec![],
    };
    (d, r)
}
fn run(rows: &[Vec<f64>]) -> ReliabilityComputed {
    let (d, r) = source(rows);
    let control = JobControl::default();
    let cohort = capture_cohort(&d, &r, &control).unwrap();
    compute(&d, &r, &cohort, &control).unwrap()
}
#[test]
fn analytical_identical_items_have_alpha_one() {
    let result = run(&[vec![0., 0.], vec![1., 1.]]);
    close(&result.raw_alpha, 1.);
    close(&result.standardized_alpha, 1.);
}

fn close(p: &crate::analysis::Estimate, truth: f64) {
    let value = p
        .value
        .unwrap_or_else(|| panic!("Unavailable {:?}", p.reason));
    // Oracle truth is rounded once: half-ulp <= 2^-53 max(1,abs(truth)).
    let bound = 2f64.powi(-40) * value.abs().max(1.) + 2f64.powi(-53) * truth.abs().max(1.);
    assert!(
        (value - truth).abs() <= bound,
        "{value} != {truth}, radius {bound}"
    );
}

fn hex(s: &str) -> f64 {
    let (sign, s) = s.strip_prefix('-').map_or((0, s), |s| (1u64 << 63, s));
    let (mantissa, exponent) = s.strip_prefix("0x").unwrap().split_once('p').unwrap();
    let exponent: i32 = exponent.parse().unwrap();
    let (integer, fraction) = mantissa.split_once('.').unwrap();
    let fraction = u64::from_str_radix(&format!("{fraction:0<13}"), 16).unwrap();
    let bits = if integer == "0" {
        fraction
    } else {
        (((exponent + 1023) as u64) << 52) | fraction
    };
    f64::from_bits(sign | bits)
}
fn fixture_source(case: &serde_json::Value) -> (Dataset, ReliabilityRequest) {
    use crate::analysis::{NumericComparison, RowPredicate};
    let input = &case["input"];
    let k = input["k"].as_u64().unwrap() as usize;
    let rows = input["rows"].as_array().unwrap();
    let hexes = case["input_float64_hex"].as_array().unwrap();
    let mut columns: Vec<_> = (0..k)
        .map(|i| {
            Column::numeric(
                ColumnId(i as u64 + 1),
                &format!("fixture {i}"),
                hexes.iter().map(|row| row[i].as_str().map(hex)).collect(),
            )
            .unwrap()
        })
        .collect();
    let weighted = rows.iter().any(|r| r.get("weight").is_some());
    let filtered = rows.iter().any(|r| r.get("include").is_some());
    if weighted {
        columns.push(
            Column::numeric(
                ColumnId(k as u64 + 1),
                "frequency",
                rows.iter()
                    .map(|r| r.get("weight").map_or(Some(1.), |v| v.as_f64()))
                    .collect(),
            )
            .unwrap(),
        );
    }
    if filtered {
        columns.push(
            Column::numeric(
                ColumnId(k as u64 + 2),
                "include",
                rows.iter()
                    .map(|r| {
                        Some(if r["include"].as_bool().unwrap_or(true) {
                            1.
                        } else {
                            0.
                        })
                    })
                    .collect(),
            )
            .unwrap(),
        );
    }
    let reversals = input["reversals"].as_object().map_or(vec![], |m| {
        m.iter()
            .map(|(i, b)| ReliabilityReverse {
                column: ColumnId(i.parse::<u64>().unwrap() + 1),
                lower: if input["id"] == "reverse_overflow" {
                    hex("0x1.fffffffffffffp+1022")
                } else {
                    b[0].as_f64().unwrap()
                },
                upper: if input["id"] == "reverse_overflow" {
                    hex("0x1.fffffffffffffp+1023")
                } else {
                    b[1].as_f64().unwrap()
                },
            })
            .collect()
    });
    let request = ReliabilityRequest {
        items: (1..=k as u64).map(ColumnId).collect(),
        filter: crate::analysis::RowFilter {
            predicates: if filtered {
                vec![RowPredicate::Number {
                    column: ColumnId(k as u64 + 2),
                    comparison: NumericComparison::Equal,
                    value: 1.,
                }]
            } else {
                vec![]
            },
        },
        frequency_weight: weighted.then_some(ColumnId(k as u64 + 1)),
        reversals,
    };
    (Dataset::new(columns).unwrap(), request)
}
#[test]
fn all_independent_fixtures_use_real_sources_and_exact_float64_inputs() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/reliability.json")).unwrap();
    let mut published = 0;
    let mut undefined = 0;
    let mut refusals = 0;
    let mut errors = 0;
    for case in fixture["cases"].as_array().unwrap() {
        let id = case["input"]["id"].as_str().unwrap();
        // The oracle's abstract integer 2^53+1 cannot be a Float64 source cell.
        // Exercise the same base ceiling with two exact weights rather than round it away.
        if id == "base_ceiling_incomplete" {
            base_ceiling_counterpart();
            errors += 1;
            continue;
        }
        let (d, r) = fixture_source(case);
        let c = JobControl::default();
        let cohort = capture_cohort(&d, &r, &c);
        if case.get("expected_error").is_some() {
            assert!(cohort.is_err(), "{id}");
            errors += 1;
            continue;
        }
        let cohort = cohort.unwrap();
        let expected = &case["exact"];
        assert_eq!(cohort.frequency, expected["n"].as_u64().unwrap(), "{id}");
        assert_eq!(
            cohort.complete_rows,
            expected["physical_rows"].as_u64().unwrap() as usize,
            "{id}"
        );
        let result = compute(&d, &r, &cohort, &c).unwrap();
        let controls = &case["float64_publication_controls"];
        let standardized = case["precision_controls"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["rounded_float64"]
            .as_array()
            .unwrap();
        let mut check =
            |metric: &str, index: usize, p: &crate::analysis::Estimate, v: &serde_json::Value| {
                let refused = (id == "huge_sd" && metric == "sd")
                    || (id == "signed_cancellation"
                        && (metric == "raw" || (metric == "corrected" && index < 2)));
                if refused {
                    assert!(p.value.is_none(), "{id} {metric}[{index}] must refuse");
                    refusals += 1;
                } else if v.is_null() {
                    assert!(p.value.is_none(), "{id} {metric}[{index}] is undefined");
                    undefined += 1;
                } else {
                    let truth = hex(v.as_str().unwrap());
                    if metric == "mean" {
                        assert_eq!(
                            p.value.unwrap().to_bits(),
                            truth.to_bits(),
                            "{id} mean[{index}]"
                        );
                    } else {
                        close(p, truth);
                    }
                    published += 1;
                }
                if p.value.is_none() {
                    assert!(p
                        .reason
                        .as_ref()
                        .is_some_and(|s| !s.trim().is_empty() && s.len() <= 1024));
                }
            };
        check("raw", 0, &result.raw_alpha, &controls["raw_alpha"]);
        check(
            "standardized",
            0,
            &result.standardized_alpha,
            &standardized[0],
        );
        for (i, item) in result.items.iter().enumerate() {
            check("mean", i, &item.mean, &controls["means"][i]);
            check("sd", i, &item.sample_sd, &controls["sample_sd"][i]);
            check(
                "corrected",
                i,
                &item.corrected_item_total,
                &controls["corrected"][i],
            );
            check(
                "raw_deleted",
                i,
                &item.raw_alpha_if_removed,
                &controls["raw_deletion"][i],
            );
            check(
                "standardized_deleted",
                i,
                &item.standardized_alpha_if_removed,
                &standardized[i + 1],
            );
        }
    }
    assert_eq!((published, undefined, refusals, errors), (292, 118, 5, 3));
}
#[test]
fn analytical_boundaries_and_energy_dependencies() {
    use ReliabilityEnergyState::*;
    let scaled = run(&[vec![0., 0.], vec![1., 2.]]);
    close(&scaled.raw_alpha, 8. / 9.);
    close(&scaled.standardized_alpha, 1.);
    assert_eq!(scaled.items[0].mean.value, Some(0.5));
    assert_eq!(scaled.items[1].mean.value, Some(1.));
    for item in scaled.items {
        assert_eq!(item.corrected_item_total.value, Some(1.));
        assert!(item.raw_alpha_if_removed.value.is_none());
        assert_eq!(item.remaining_standardized_energy, Unavailable);
    }
    let negative = run(&[vec![0., 0.], vec![1., -2.]]);
    close(&negative.raw_alpha, -8.);
    for item in negative.items {
        assert_eq!(item.corrected_item_total.value, Some(-1.));
    }
    let constant = run(&[vec![0., 5.], vec![1., 5.]]);
    close(&constant.raw_alpha, 0.);
    assert_eq!(constant.items[1].sample_sd.value, Some(0.));
    assert_eq!(constant.items[1].energy, ExactZero);
    assert_eq!(constant.standardized_total_energy, Unavailable);
    let opposite = run(&[vec![0., 0.], vec![1., -1.]]);
    assert_eq!(opposite.total_energy, ExactZero);
    assert!(opposite.raw_alpha.value.is_none());
    let removed = run(&[vec![0., 0., 5.], vec![1., 1., 5.]]);
    assert!(removed.standardized_alpha.value.is_none());
    close(&removed.items[2].standardized_alpha_if_removed, 1.);
    let single = run(&[vec![2., 3.]]);
    assert_eq!(single.items[0].mean.value, Some(2.));
    assert_eq!(single.total_energy, Unavailable);
    let empty = run(&[]);
    assert!(empty.items[0].mean.value.is_none());
    assert_eq!(empty.total_energy, Unavailable);
}
#[test]
fn compressed_frequency_matches_literal_replication_and_zero_means_round_correctly() {
    let d = Dataset::new(vec![
        Column::numeric(ColumnId(1), "a", vec![Some(0.), Some(1.), Some(3.)]).unwrap(),
        Column::numeric(ColumnId(2), "b", vec![Some(1.), Some(2.), Some(2.)]).unwrap(),
        Column::numeric(ColumnId(3), "weights", vec![Some(2.), Some(3.), Some(1.)]).unwrap(),
    ])
    .unwrap();
    let mut r = source(&[]).1;
    r.frequency_weight = Some(ColumnId(3));
    let c = JobControl::default();
    let cohort = capture_cohort(&d, &r, &c).unwrap();
    let compressed = compute(&d, &r, &cohort, &c).unwrap();
    let literal = run(&[
        vec![0., 1.],
        vec![0., 1.],
        vec![1., 2.],
        vec![1., 2.],
        vec![1., 2.],
        vec![3., 2.],
    ]);
    close(&compressed.raw_alpha, literal.raw_alpha.value.unwrap());
    close(
        &compressed.standardized_alpha,
        literal.standardized_alpha.value.unwrap(),
    );
    for (a, b) in compressed.items.iter().zip(literal.items) {
        assert_eq!(a.mean, b.mean);
        close(&a.sample_sd, b.sample_sd.value.unwrap());
        close(
            &a.corrected_item_total,
            b.corrected_item_total.value.unwrap(),
        );
    }
    let sub = run(&[vec![0., 0.], vec![f64::from_bits(1), f64::from_bits(1)]]);
    assert_eq!(sub.items[0].mean.value.unwrap().to_bits(), 0);
}
#[test]
fn deletion_retains_missing_rule_cohort_and_source_is_immutable() {
    use crate::data::{ColumnMetadata, MissingRules};
    let col = Column::numeric(ColumnId(3), "c", vec![Some(5.), Some(5.), Some(-999.)]).unwrap();
    let col = col
        .with_metadata(ColumnMetadata {
            missing_rules: MissingRules::Number {
                values: vec![-999.],
                ranges: vec![],
            },
            ..col.metadata().clone()
        })
        .unwrap();
    let d = Dataset::new(vec![
        Column::numeric(ColumnId(1), "a", vec![Some(0.), Some(1.), Some(100.)]).unwrap(),
        Column::numeric(ColumnId(2), "b", vec![Some(0.), Some(1.), Some(-100.)]).unwrap(),
        col,
    ])
    .unwrap();
    let mut r = source(&[vec![0.; 3]]).1;
    r.reversals = vec![ReliabilityReverse {
        column: ColumnId(1),
        lower: 0.,
        upper: 100.,
    }];
    let before = d
        .columns()
        .iter()
        .map(|col| {
            (
                col.metadata().clone(),
                (0..3).map(|i| col.cell(i).unwrap()).collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    let revision = d.revision();
    let c = JobControl::default();
    let cohort = capture_cohort(&d, &r, &c).unwrap();
    assert_eq!((cohort.complete_rows, cohort.frequency), (2, 2));
    let result = compute(&d, &r, &cohort, &c).unwrap();
    assert!(result.standardized_alpha.value.is_none());
    assert!(result.items[2]
        .standardized_alpha_if_removed
        .value
        .is_none()); // remaining reversed pair cancels exactly
    assert_eq!(result.items[0].mean.value, Some(99.5));
    assert_eq!(d.revision(), revision);
    for (col, (metadata, cells)) in d.columns().iter().zip(before) {
        assert_eq!(col.metadata(), &metadata);
        assert_eq!(
            (0..3).map(|i| col.cell(i).unwrap()).collect::<Vec<_>>(),
            cells
        );
    }
}
#[test]
fn mutated_cohort_counts_and_precancellation_are_rejected() {
    let (d, r) = source(&[vec![0., 0.], vec![1., 2.]]);
    let c = JobControl::default();
    let mut cohort = capture_cohort(&d, &r, &c).unwrap();
    cohort.frequency += 1;
    assert!(compute(&d, &r, &cohort, &c).is_err());
    c.cancel();
    let (result, largest) = crate::data::measure_test_allocations(|| compute(&d, &r, &cohort, &c));
    assert!(matches!(result, Err(crate::error::AppError::Cancelled)));
    assert_eq!(largest, 0);
}

fn wide_source(k: usize, rows: usize) -> (Dataset, ReliabilityRequest) {
    let d = Dataset::new(
        (0..k)
            .map(|i| {
                Column::numeric(
                    ColumnId(i as u64 + 1),
                    &format!("wide {i}"),
                    (0..rows).map(|r| Some((r % 7) as f64)).collect(),
                )
                .unwrap()
            })
            .collect(),
    )
    .unwrap();
    let r = ReliabilityRequest {
        items: (1..=k as u64).map(ColumnId).collect(),
        filter: Default::default(),
        frequency_weight: None,
        reversals: vec![],
    };
    (d, r)
}
#[test]
fn cancellation_reaches_all_four_passes_and_terminal_empty_singleton() {
    use crate::{error::AppError, operation::CHECKPOINT_HOOK};
    use std::{cell::Cell, rc::Rc};
    let (d, r) = wide_source(64, 128);
    let original = capture_cohort(&d, &r, &JobControl::default()).unwrap();
    for threshold in [3208, 19848, 52744, 90312] {
        let c = JobControl::default();
        let cancel = c.clone();
        let done = Rc::new(Cell::new(0));
        let observed = done.clone();
        CHECKPOINT_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |p| {
                if p.done >= threshold {
                    observed.set(p.done);
                    cancel.cancel()
                }
            }))
        });
        let result = compute(&d, &r, &original, &c);
        CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
        assert!(
            matches!(result, Err(AppError::Cancelled)),
            "threshold {threshold}"
        );
        assert!(done.get() >= threshold && done.get() <= threshold + 64);
    }
    for rows in [0, 1] {
        let (d, r) = wide_source(2, rows);
        let cohort = capture_cohort(&d, &r, &JobControl::default()).unwrap();
        let count = Rc::new(Cell::new(0));
        let observed = count.clone();
        CHECKPOINT_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |_| observed.set(observed.get() + 1)))
        });
        compute(&d, &r, &cohort, &JobControl::default()).unwrap();
        CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
        let terminal = count.get();
        let seen = Rc::new(Cell::new(0));
        let observed = seen.clone();
        let c = JobControl::default();
        let cancel = c.clone();
        CHECKPOINT_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |_| {
                observed.set(observed.get() + 1);
                if observed.get() == terminal {
                    cancel.cancel()
                }
            }))
        });
        let result = compute(&d, &r, &cohort, &c);
        CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
        assert!(
            matches!(result, Err(AppError::Cancelled)),
            "N={rows} terminal {terminal}"
        );
        assert_eq!(seen.get(), terminal);
    }
}
#[test]
fn maximum_cells_use_linear_workspace_and_exact_bounded_work_counts() {
    use crate::operation::CHECKPOINT_HOOK;
    use std::{cell::Cell, rc::Rc};
    let k = 64;
    let rows = 31250;
    let (d, r) = wide_source(k, rows);
    let c = JobControl::default();
    let cohort = capture_cohort(&d, &r, &c).unwrap();
    let checkpoints = Rc::new(Cell::new(0));
    let observed = checkpoints.clone();
    let last = Rc::new(Cell::new(0));
    let tracked = last.clone();
    let maximum_gap = Rc::new(Cell::new(0));
    let gap = maximum_gap.clone();
    let admission = super::accounting::numerical_admission(&d, &r, &cohort, &c).unwrap();
    let expected_total = admission.work;
    let totals_match = Rc::new(Cell::new(true));
    let totals = totals_match.clone();
    CHECKPOINT_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |p| {
            observed.set(observed.get() + 1);
            gap.set(gap.get().max(p.done.saturating_sub(tracked.get())));
            tracked.set(p.done);
            totals.set(totals.get() && p.total == Some(expected_total));
        }))
    });
    let start = std::time::Instant::now();
    let ((result, work), largest) = crate::data::measure_test_allocations(|| {
        super::compute::compute_core(&d, &r, &cohort, &c).unwrap()
    });
    let elapsed = start.elapsed();
    CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
    close(&result.raw_alpha, 1.);
    close(&result.standardized_alpha, 1.);
    assert_eq!(work.numerical, 9 * rows * k + 3 * k);
    assert_eq!(work.classification, 2 * k * k);
    assert_eq!(work.source, 5 * rows + 4 * rows * k + k * (k + 1) / 2);
    assert_eq!(work.finalization, 10 * k);
    assert!(maximum_gap.get() <= 64);
    assert!(checkpoints.get() > work.total().unwrap() / 64);
    let admission = super::accounting::numerical_admission(&d, &r, &cohort, &c).unwrap();
    assert!(work.total().unwrap() <= admission.work);
    assert!(largest < admission.bytes);
    assert!(largest < rows * k * 8);
    assert_eq!(last.get(), work.total().unwrap());
    assert!(totals_match.get());
    eprintln!(
        "2M cells: {:?}; work {:?}; total {}; checkpoints {}; max gap {}; largest allocation {}; admitted owned bytes {}",
        elapsed,
        work,
        work.total().unwrap(),
        checkpoints.get(),
        maximum_gap.get(),
        largest,
        admission.bytes
    );
}

fn base_ceiling_counterpart() {
    let d = Dataset::new(vec![
        Column::numeric(ColumnId(1), "missing", vec![None, None]).unwrap(),
        Column::numeric(ColumnId(2), "present", vec![Some(1.), Some(1.)]).unwrap(),
        Column::numeric(
            ColumnId(3),
            "weight",
            vec![Some((1u64 << 53) as f64), Some(1.)],
        )
        .unwrap(),
    ])
    .unwrap();
    let r = ReliabilityRequest {
        frequency_weight: Some(ColumnId(3)),
        ..source(&[]).1
    };
    assert!(capture_cohort(&d, &r, &JobControl::default()).is_err());
}
#[test]
fn representable_weights_overflow_base_frequency_before_completeness() {
    base_ceiling_counterpart();
}

#[test]
fn final_state_scans_check_cancellation_between_the_two_bounded_scans() {
    use crate::{error::AppError, operation::CHECKPOINT_HOOK};
    use std::{cell::Cell, rc::Rc};
    let (d, r) = wide_source(64, 3);
    let cohort = capture_cohort(&d, &r, &JobControl::default()).unwrap();
    let (_, work) = super::compute::compute_core(&d, &r, &cohort, &JobControl::default()).unwrap();
    let threshold = work.total().unwrap() - work.classification - r.items.len() + 64;
    let c = JobControl::default();
    let cancel = c.clone();
    let last = Rc::new(Cell::new(0));
    let observed = last.clone();
    CHECKPOINT_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |p| {
            if p.done >= threshold {
                observed.set(p.done);
                cancel.cancel()
            }
        }))
    });
    let result = compute(&d, &r, &cohort, &c);
    CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
    assert!(matches!(result, Err(AppError::Cancelled)));
    assert_eq!(last.get(), threshold);
}
