use super::*;
use crate::operation::CHECKPOINT_HOOK;
#[test]
fn guarded_allocation_and_semantic_scratch_refuse_without_mutating_capture() {
    let r = captures::capture(3, 1., false);
    let text = encode(&r);
    let original = r.clone();
    for room in [
        0,
        text.len(),
        text.len() + 3 * MAX_DEPTH + size_of::<Payload>(),
    ] {
        assert!(matches!(
            decode(&text, room, &JobControl::default()),
            Err(AppError::SizeLimit { .. })
        ));
    }
    assert_eq!(r, original);
    let oversized = " ".repeat(MAX_PROJECT_BYTES + 1);
    let input_control = JobControl::default();
    let (result, largest) = crate::data::measure_test_allocations(|| {
        decode(&oversized, MAX_OWNED_BYTES, &input_control)
    });
    assert!(matches!(
        result,
        Err(AppError::SizeLimit {
            limit: MAX_PROJECT_BYTES
        })
    ));
    assert_eq!(largest, 0);
    drop(oversized);
    let c = JobControl::default();
    c.cancel();
    assert!(matches!(
        decode(&text, MAX_OWNED_BYTES, &c),
        Err(AppError::Cancelled)
    ));
    let deep = format!(
        "{}0{}",
        "[".repeat(MAX_DEPTH + 1),
        "]".repeat(MAX_DEPTH + 1)
    );
    assert!(decode(&deep, MAX_OWNED_BYTES, &JobControl::default()).is_err());
    assert!(decode(
        &(text.clone() + " null"),
        MAX_OWNED_BYTES,
        &JobControl::default()
    )
    .is_err());
}
#[test]
fn nested_request_limit_and_semantic_cancellation() {
    let r = captures::capture(0, 1., false);
    let mut p: Value = serde_json::from_str(&encode(&r)).unwrap();
    p["request"]["planned_pairs"][0]["first"] = json!("ŵ".repeat(anova::MAX_REQUEST_BYTES));
    assert!(matches!(
        decode(&p.to_string(), MAX_OWNED_BYTES, &JobControl::default()),
        Err(AppError::SizeLimit {
            limit: anova::MAX_REQUEST_BYTES
        })
    ));
    let c = JobControl::default();
    let cancel = c.clone();
    let text = encode(&r);
    CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = Some(Box::new(move |_| cancel.cancel())));
    let result = decode(&text, MAX_OWNED_BYTES, &c);
    CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
    assert!(matches!(result, Err(AppError::Cancelled)));
}

#[test]
fn global_guard_refuses_strings_and_vector_growth_before_allocation() {
    let mut value: Value = serde_json::from_str(&encode(&captures::capture(0, 1., false))).unwrap();
    value["columns"][0]["description"] = json!("r".repeat(8192));
    let input = value.to_string();
    let (result, largest) = crate::data::measure_test_allocations(|| {
        decode(&input, input.len() + 2048, &JobControl::default())
    });
    assert!(matches!(result, Err(AppError::SizeLimit { .. })));
    assert!(largest < 8192, "allocated {largest}");
    let c = JobControl::default();
    for (headroom, maximum) in [
        (31, 64),
        (210, 4 * size_of::<Cell>()),
        (26 * size_of::<Cell>(), 8 * size_of::<Cell>()),
    ] {
        let state = RefCell::new(Accounting {
            bytes: MAX_OWNED_BYTES - headroom,
            control: &c,
            error: None,
            steps: 0,
        });
        let mut j = serde_json::Deserializer::from_str("[0,1,2,3,4,5,6,7,8]");
        let (result, largest) = crate::data::measure_test_allocations(|| {
            cells::deserialize(Guard {
                inner: &mut j,
                state: &state,
                depth: 0,
            })
        });
        assert!(result.is_err());
        assert!(matches!(
            state.borrow().error,
            Some(AppError::SizeLimit { .. })
        ));
        assert!(largest <= maximum, "allocated {largest}");
    }
}
#[test]
fn request_quota_precedes_semantic_scratch_and_scratch_precedes_indexes() {
    let mut r = captures::capture(3, 1., false);
    r.request.planned_pairs[0].first = Cell::Text("ŵ".repeat(anova::MAX_REQUEST_BYTES));
    let c = JobControl::default();
    let state = RefCell::new(Accounting {
        bytes: r.owned_bytes().unwrap(),
        control: &c,
        error: None,
        steps: 0,
    });
    let before = state.borrow().bytes;
    let (result, largest) = crate::data::measure_test_allocations(|| validate_decoded(&r, &state));
    assert!(matches!(
        result,
        Err(AppError::SizeLimit {
            limit: anova::MAX_REQUEST_BYTES
        })
    ));
    assert_eq!(state.borrow().bytes, before);
    assert_eq!(largest, 0);
    let r = captures::capture(3, 1., false);
    let state = RefCell::new(Accounting {
        bytes: MAX_OWNED_BYTES - r.validation_scratch_bytes().unwrap() + 1,
        control: &c,
        error: None,
        steps: 0,
    });
    let (result, largest) = crate::data::measure_test_allocations(|| validate_decoded(&r, &state));
    assert!(matches!(
        result,
        Err(AppError::SizeLimit {
            limit: MAX_OWNED_BYTES
        })
    ));
    assert!(largest <= 64, "fixed error allocation {largest}");
}
#[test]
fn controlled_semantic_text_mapping_grid_effect_family_scans_cancel() {
    for design in 0..4 {
        let mut r = captures::capture(design, 1., false);
        r.columns[0].description = "semantic metadata ŵ".repeat(2000);
        let calls = std::rc::Rc::new(std::cell::Cell::new(0));
        let observed = calls.clone();
        CHECKPOINT_HOOK
            .with(|h| *h.borrow_mut() = Some(Box::new(move |_| observed.set(observed.get() + 1))));
        let result = r.validate(&JobControl::default());
        CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
        result.unwrap();
        let count = calls.get();
        assert!(count > 8);
        for target in [1, 5, count / 2, count - 1] {
            let c = JobControl::default();
            let cancel = c.clone();
            let scanned = std::cell::Cell::new(0);
            CHECKPOINT_HOOK.with(|h| {
                *h.borrow_mut() = Some(Box::new(move |_| {
                    scanned.set(scanned.get() + 1);
                    if scanned.get() >= target {
                        cancel.cancel()
                    }
                }))
            });
            let state = RefCell::new(Accounting {
                bytes: r.owned_bytes().unwrap(),
                control: &c,
                error: None,
                steps: 0,
            });
            let result = validate_decoded(&r, &state);
            CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
            assert!(
                matches!(result, Err(AppError::Cancelled)),
                "design {design}, checkpoint {target}"
            );
        }
    }
}

#[test]
fn every_nested_request_owner_is_charged_before_semantic_indexes() {
    for owner in 0..4 {
        let mut r = captures::capture(3, 1., false);
        let text = "a".repeat(anova::MAX_REQUEST_BYTES);
        match owner {
            0 => r.request.filter.predicates.push(RowPredicate::Text {
                column: ColumnId(1),
                value: text,
                equal: true,
            }),
            1 => r.request.planned_pairs[0].scope = AnovaScope::Fixed(vec![Cell::Text(text)]),
            2 => {
                if let AnovaDesign::Repeated { factors, .. } = &mut r.request.design {
                    factors[0].levels[0] = text
                }
            }
            3 => {
                if let AnovaDesign::Repeated { measurements, .. } = &mut r.request.design {
                    *measurements =
                        vec![ColumnId(2); anova::MAX_REQUEST_BYTES / size_of::<ColumnId>() + 1]
                }
            }
            _ => unreachable!(),
        }
        let c = JobControl::default();
        let state = RefCell::new(Accounting {
            bytes: r.owned_bytes().unwrap(),
            control: &c,
            error: None,
            steps: 0,
        });
        let before = state.borrow().bytes;
        let (result, largest) =
            crate::data::measure_test_allocations(|| validate_decoded(&r, &state));
        assert!(matches!(
            result,
            Err(AppError::SizeLimit {
                limit: anova::MAX_REQUEST_BYTES
            })
        ));
        assert_eq!(state.borrow().bytes, before);
        assert_eq!(largest, 0);
    }
}
