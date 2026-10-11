use super::*;
mod fixture {
    use crate::{analysis::*, data::*};
    pub fn input(design: usize) -> (Dataset, AnovaRequest) {
        let mut columns = vec![
            Column::numeric(
                ColumnId(1),
                "Y",
                vec![Some(1.), Some(2.), Some(3.), Some(4.)],
            )
            .unwrap(),
            Column::text(
                ColumnId(2),
                "A",
                vec![
                    Some(" A ".into()),
                    Some(" A ".into()),
                    Some("B".into()),
                    Some("B".into()),
                ],
            )
            .unwrap(),
            Column::text(
                ColumnId(3),
                "B",
                vec![
                    Some("X".into()),
                    Some("Y".into()),
                    Some("X".into()),
                    Some("Y".into()),
                ],
            )
            .unwrap(),
            Column::text(
                ColumnId(4),
                "ID",
                (0..4).map(|i| Some(i.to_string())).collect(),
            )
            .unwrap(),
        ];
        for i in 5..9 {
            columns.push(
                Column::numeric(
                    ColumnId(i),
                    &format!("M{i}"),
                    vec![Some(1.), Some(2.), Some(4.), Some(8.)],
                )
                .unwrap(),
            );
        }
        let repeated = design == 2;
        let design = match design {
            0 => AnovaDesign::OneWay {
                outcome: ColumnId(1),
                factor: ColumnId(2),
            },
            1 => AnovaDesign::Factorial {
                outcome: ColumnId(1),
                factors: vec![ColumnId(2), ColumnId(3)],
            },
            _ => AnovaDesign::Repeated {
                subject: ColumnId(4),
                factors: vec![
                    AnovaWithinFactor {
                        name: " Time ".into(),
                        levels: vec![" First ".into(), "Second".into()],
                    },
                    AnovaWithinFactor {
                        name: "Condition".into(),
                        levels: vec!["X".into(), "Y".into()],
                    },
                ],
                measurements: (5..9).map(ColumnId).collect(),
            },
        };
        (
            Dataset::new(columns).unwrap(),
            AnovaRequest {
                design,
                confidence: ConfidenceLevel::P95,
                filter: RowFilter::default(),
                frequency_weight: None,
                planned_pairs: vec![AnovaPairRequest {
                    factor: 0,
                    first: Cell::Text(if repeated { " First " } else { " A " }.into()),
                    second: Cell::Text(if repeated { "Second" } else { "B" }.into()),
                    scope: AnovaScope::Marginal,
                }],
                all_pairs: vec![AnovaFamilyRequest {
                    factor: 0,
                    scope: AnovaScope::Marginal,
                }],
            },
        )
    }
}

#[test]
fn all_designs_roundtrip_ordered_exact_recipes_and_match_without_allocation() {
    for design in 0..3 {
        let (data, mut request) = fixture::input(design);
        for confidence in [
            ConfidenceLevel::P90,
            ConfidenceLevel::P95,
            ConfidenceLevel::P99,
        ] {
            request.confidence = confidence;
            if design > 0 {
                request.planned_pairs[0].scope = AnovaScope::Fixed(vec![Cell::Text("Y".into())]);
                request.all_pairs[0].scope = AnovaScope::Fixed(vec![Cell::Text("X".into())]);
            }
            let d = AnovaDraft::from_request(request.clone());
            let (same, allocated) =
                crate::data::measure_test_allocations(|| d.matches_saved(&request));
            assert!(same);
            assert_eq!(allocated, 0);
            assert_eq!(d.build_request(&data).unwrap(), request);
            let mut changed = request.clone();
            changed.planned_pairs[0].first = Cell::Text("different".into());
            assert!(!d.matches_saved(&changed));
            if let AnovaDesign::Repeated { measurements, .. } = &mut changed.design {
                measurements.swap(0, 1);
                assert!(!d.matches_saved(&changed));
            }
        }
    }
}
#[test]
fn repeated_mapping_and_exact_declarations_refuse_invalid_roles_before_clone() {
    let (data, mut r) = fixture::input(2);
    if let AnovaDesign::Repeated { factors, .. } = &mut r.design {
        factors[0].name = "long declared name ".repeat(1000);
    }
    let base = AnovaDraft::from_request(r);
    for mutation in 0..8 {
        let mut d = base.clone();
        let AnovaDesignDraft::Repeated {
            subject,
            factors,
            measurements,
        } = &mut d.design
        else {
            panic!()
        };
        match mutation {
            0 => measurements[0] = None,
            1 => measurements[1] = measurements[0],
            2 => measurements[0] = *subject,
            3 => measurements[0] = Some(ColumnId(2)),
            4 => {
                measurements.pop();
            }
            5 => factors[0].name = "\u{2003}".into(),
            6 => factors[1].name = factors[0].name.clone(),
            _ => factors[0].levels[1] = factors[0].levels[0].clone(),
        }
        let (result, largest) = crate::data::measure_test_allocations(|| d.build_request(&data));
        assert!(result.is_err());
        assert!(
            largest < 512,
            "copied vectors before rejecting setup: {largest}"
        );
    }
    let mut d = base.clone();
    d.frequency_weight = Some(ColumnId(1));
    assert!(d.build_request(&data).is_err());
    let AnovaDesignDraft::Repeated { factors, .. } = &mut d.design else {
        panic!()
    };
    factors[0].levels = vec!["ŵ".into(), "w\u{302}".into()];
    d.frequency_weight = None;
    d.clear_comparisons();
    assert!(d.build_request(&data).is_ok());
    let AnovaDesignDraft::Repeated { factors, .. } = &mut d.design else {
        panic!()
    };
    factors[0].levels = vec!["A".into(), " A ".into()];
    assert!(d.build_request(&data).is_ok());
}
#[test]
fn numeric_factors_and_subject_ids_are_explicit_and_conversion_moves_labels() {
    let (data, mut r) = fixture::input(0);
    r.design = AnovaDesign::OneWay {
        outcome: ColumnId(1),
        factor: ColumnId(5),
    };
    r.planned_pairs.clear();
    r.all_pairs.clear();
    assert!(AnovaDraft::from_request(r).build_request(&data).is_ok());
    let (data, mut r) = fixture::input(2);
    let AnovaDesign::Repeated {
        subject, factors, ..
    } = &mut r.design
    else {
        panic!()
    };
    *subject = ColumnId(1);
    factors[0].name = "ŵ".repeat(10000);
    let pointer = factors[0].name.as_ptr();
    let bound = AnovaDraft::conversion_bytes(&r).unwrap();
    let (d, largest) = crate::data::measure_test_allocations(|| AnovaDraft::from_request(r));
    let AnovaDesignDraft::Repeated { factors, .. } = &d.design else {
        panic!()
    };
    assert_eq!(factors[0].name.as_ptr(), pointer);
    assert!(largest <= bound);
    assert!(d.build_request(&data).is_ok());
}
#[test]
fn saved_absent_pairs_keep_typed_identity_and_duplicates_are_rejected() {
    let (data, mut r) = fixture::input(0);
    r.planned_pairs[0].first = Cell::Text("absent \u{2003}".into());
    let d = AnovaDraft::from_request(r.clone());
    assert_eq!(d.build_request(&data).unwrap(), r);
    let mut duplicate = r.planned_pairs[0].clone();
    std::mem::swap(&mut duplicate.first, &mut duplicate.second);
    r.planned_pairs.push(duplicate);
    assert!(AnovaDraft::from_request(r.clone())
        .build_request(&data)
        .is_err());
    r.planned_pairs.pop();
    r.all_pairs.push(r.all_pairs[0].clone());
    assert!(AnovaDraft::from_request(r).build_request(&data).is_err());
}
#[test]
fn capacity_and_scratch_include_editor_strings_and_refuse_oversized_draft() {
    let (data, r) = fixture::input(0);
    let mut d = AnovaDraft::from_request(r);
    let baseline = d.heap_bytes().unwrap();
    d.endpoints[0] = Some(Cell::Text(String::with_capacity(9000)));
    d.fixed = Some(vec![Some(Cell::Text(String::with_capacity(10000)))]);
    assert!(d.heap_bytes().unwrap() >= baseline + 19000);
    d.endpoints[0] = Some(Cell::Text(String::with_capacity(600000)));
    let (result, largest) = crate::data::measure_test_allocations(|| d.build_request(&data));
    assert!(matches!(result, Err(AppError::SizeLimit { .. })));
    assert_eq!(largest, 0);
}
