use super::*;
use crate::{
    analysis::{ConfidenceLevel, RowFilter},
    data::{Column, ColumnId, Dataset},
};

#[test]
fn original_unit_publication_boundaries() {
    for v in [
        f64::from_bits(1),
        f64::from_bits(f64::MIN_POSITIVE.to_bits() - 1),
    ] {
        let e = Energy {
            value: Scaled::new(v),
            relative_error: 0.,
        };
        assert_eq!(e.captured().state, AnovaQuantityState::Positive);
        assert!(e.captured().value.value.is_none());
        assert!(e.captured().value.reason.is_some());
        assert!(e.mean_square(1).value.is_none());
    }
    for v in [0., f64::MIN_POSITIVE, 1., f64::MAX] {
        let e = Energy {
            value: Scaled::new(v),
            relative_error: 0.,
        };
        assert_eq!(e.captured().value.value, Some(v));
        assert_eq!(e.mean_square(1).value, Some(v));
        assert_eq!(
            e.captured().state,
            if v == 0. {
                AnovaQuantityState::ExactZero
            } else {
                AnovaQuantityState::Positive
            }
        );
    }
    let e = Energy {
        value: Scaled::new(f64::MIN_POSITIVE),
        relative_error: 0.,
    };
    assert!(e.mean_square(2).value.is_none());
    assert!(e.mean_square(0).value.is_none());
    for value in [
        Scaled {
            fraction: 0.5,
            exponent: 1025,
        },
        Scaled {
            fraction: 0.5,
            exponent: -1075,
        },
    ] {
        let e = Energy {
            value,
            relative_error: 0.,
        };
        assert_eq!(e.captured().state, AnovaQuantityState::Positive);
        assert!(e.captured().value.value.is_none());
        assert!(e.mean_square(1).value.is_none());
    }
}

fn model(repeated_design: bool, scale: f64) -> AnovaModelResult {
    let mut cols = vec![];
    let design = if repeated_design {
        cols.push(
            Column::numeric(ColumnId(1), "id", (0..5).map(|i| Some(i as f64)).collect()).unwrap(),
        );
        for j in 0..3 {
            cols.push(
                Column::numeric(
                    ColumnId(j + 2),
                    &format!("y{j}"),
                    (0..5)
                        .map(|i| Some(((i * i + j * j + i * j) as f64 + 1.) * scale))
                        .collect(),
                )
                .unwrap(),
            );
        }
        AnovaDesign::Repeated {
            subject: ColumnId(1),
            factors: vec![AnovaWithinFactor {
                name: "time".into(),
                levels: vec!["a".into(), "b".into(), "c".into()],
            }],
            measurements: vec![ColumnId(2), ColumnId(3), ColumnId(4)],
        }
    } else {
        cols.push(
            Column::numeric(
                ColumnId(1),
                "y",
                (0..12)
                    .map(|i| Some(((i * i % 17) as f64 + 1.) * scale))
                    .collect(),
            )
            .unwrap(),
        );
        cols.push(
            Column::numeric(
                ColumnId(2),
                "group",
                (0..12).map(|i| Some((i % 2) as f64)).collect(),
            )
            .unwrap(),
        );
        AnovaDesign::OneWay {
            outcome: ColumnId(1),
            factor: ColumnId(2),
        }
    };
    let d = Dataset::new(cols).unwrap();
    let r = AnovaRequest {
        design,
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
        planned_pairs: vec![],
        all_pairs: vec![],
    };
    let c = JobControl::default();
    let cohort = cohort::select(&d, &r, &c, 0).unwrap();
    let outcome = if repeated_design {
        repeated::compute(&d, &r, &cohort, &c, 0).unwrap().outcome
    } else {
        independent::compute(&d, &r, &cohort, &c, 0)
            .unwrap()
            .outcome
    };
    match outcome {
        AnovaOutcome::Fitted(m) => m,
        other => panic!("Expected fitted model: {other:?}"),
    }
}

#[test]
fn independent_subnormal_publication_keeps_carried_inference() {
    check_engine(false);
}
#[test]
fn repeated_subnormal_publication_keeps_carried_inference() {
    check_engine(true);
}
fn check_engine(repeated_design: bool) {
    let normal = model(repeated_design, 1.);
    let tiny = model(repeated_design, 1e-160);
    for (a, b) in normal.effects.iter().zip(&tiny.effects) {
        for ss in [&b.ss, &b.error_ss] {
            assert_eq!(ss.state, AnovaQuantityState::Positive);
            assert!(ss.value.value.is_none());
            assert!(ss.value.reason.is_some());
        }
        assert!(b.ms.value.is_none());
        assert!(b.error_ms.value.is_none());
        if repeated_design {
            assert_eq!(a.eta_squared, b.eta_squared);
        } else {
            assert!((a.eta_squared.value.unwrap() - b.eta_squared.value.unwrap()).abs() < 1e-10);
        }
        for (x, y) in [
            (&a.test.f, &b.test.f),
            (&a.test.p, &b.test.p),
            (&a.partial_eta_squared, &b.partial_eta_squared),
        ] {
            let (x, y) = (
                x.value.expect(&format!("normal {x:?}")),
                y.value.expect(&format!("tiny {y:?}")),
            );
            assert!((x - y).abs() <= 1e-10 * x.abs().max(1.));
        }
    }
}
