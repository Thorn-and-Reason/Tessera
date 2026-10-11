use super::*;
use crate::{
    analysis::{ConfidenceLevel, RowFilter},
    data::{Column, ColumnId},
};
fn run(offset: f64, delta: f64, reverse: bool, unequal: bool) -> Vec<AnovaContrastFamily> {
    let values = [1., 3., 3., 5., 2., 4., 2. + delta, 4. + delta];
    let d = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "y",
            values.iter().map(|v| Some(v + offset)).collect(),
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "A",
            (0..8).map(|i| Some((i / 4) as f64)).collect(),
        )
        .unwrap(),
        Column::numeric(
            ColumnId(3),
            "B",
            (0..8).map(|i| Some(((i / 2) % 2) as f64)).collect(),
        )
        .unwrap(),
        Column::numeric(
            ColumnId(4),
            "w",
            (0..8)
                .map(|i| Some(if unequal { (i / 2 + 1) as f64 } else { 1. }))
                .collect(),
        )
        .unwrap(),
    ])
    .unwrap();
    let r = AnovaRequest {
        design: AnovaDesign::Factorial {
            outcome: ColumnId(1),
            factors: vec![ColumnId(2), ColumnId(3)],
        },
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: Some(ColumnId(4)),
        planned_pairs: vec![AnovaPairRequest {
            factor: 0,
            first: Cell::Number(if reverse { 1. } else { 0. }),
            second: Cell::Number(if reverse { 0. } else { 1. }),
            scope: AnovaScope::Marginal,
        }],
        all_pairs: vec![AnovaFamilyRequest {
            factor: 0,
            scope: AnovaScope::Marginal,
        }],
    };
    let c = JobControl::default();
    let s = cohort::select(&d, &r, &c, 0).unwrap();
    let result = super::super::independent::compute(&d, &r, &s, &c, 0).unwrap();
    assert!(matches!(result.outcome, AnovaOutcome::Fitted(_)));
    let families = compute(
        &d,
        &r,
        &s,
        &result.outcome,
        Input::Independent(result.numerical.as_ref()),
        &c,
        0,
    )
    .unwrap();
    let ids = validation::request(&r, |id| Ok(d.column(id)?.kind()), &c).unwrap();
    let captured = ComputedAnova {
        revision: d.revision(),
        request: r,
        columns: crate::analysis::support::capture_columns(&d, &ids, &c).unwrap(),
        source_rows: d.row_count(),
        diagnostics: s.diagnostics.clone(),
        fitted_rows: s.rows.len(),
        frequency: s.frequency,
        incomplete_case_frequency: s.incomplete_case_frequency,
        observed_measurements: s.observed_measurements,
        grid: AnovaGrid {
            factors: s.factors.clone(),
            cells: s
                .cells
                .iter()
                .zip(result.cell_means)
                .map(|(count, mean)| AnovaCell {
                    physical_rows: count.physical_rows,
                    frequency: count.frequency,
                    mean,
                })
                .collect(),
            complete: true,
        },
        method: AnovaMethod {
            numerical_version: 1,
            basis_version: 1,
        },
        outcome: result.outcome,
        families: families.clone(),
    };
    validation::completed(&captured, &c).unwrap();
    families
}
#[test]
fn balanced_marginal_exact_zero_preserves_full_inference() {
    for offset in [0., 2f64.powi(45)] {
        for reverse in [false, true] {
            let families = run(offset, 0., reverse, false);
            for family in &families {
                assert_eq!(family.m, 1);
                let m = &family.members[0];
                assert_eq!(m.ordinal, 0);
                assert!(matches!(m.state, AnovaComparisonState::Available));
                assert_eq!(m.estimate.value, Some(0.));
                assert_eq!(m.standard_error.value, Some(1.));
                assert_eq!(m.df.value, Some(4.));
                assert_eq!(m.t.value, Some(0.));
                assert_eq!(m.raw_p.value, Some(1.));
                assert_eq!(m.adjusted_p.value, Some(1.));
                assert!((m.interval[0].value.unwrap() + 2.7764451051977987).abs() < 1e-9);
                assert_eq!(m.interval[0].value.map(|v| -v), m.interval[1].value);
            }
        }
    }
}
#[test]
fn exact_nonzero_near_cancellation_is_never_relabelled_zero() {
    let delta = 2f64.powi(-48);
    for reverse in [false, true] {
        let families = run(0., delta, reverse, false);
        let m = &families[0].members[0];
        assert_eq!(
            m.estimate.value,
            Some(if reverse { delta / 2. } else { -delta / 2. })
        );
        assert!(m.t.value.is_some_and(|t| t != 0.));
        assert!(m.standard_error.value.is_some_and(|v| v > 0.));
    }
}
#[test]
fn unequal_count_global_cancellation_keeps_conservative_refusal() {
    let families = run(0., 0., false, true);
    for family in families {
        assert_eq!(family.m, 1);
        let m = &family.members[0];
        assert!(matches!(m.state, AnovaComparisonState::Unavailable(_)));
        assert!(m.estimate.value.is_none());
        assert!(m.raw_p.value.is_none());
        assert!(family.adjustment_failure.is_some());
    }
}
