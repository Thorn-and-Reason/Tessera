//! Actual accepted direct-API captures, kept separate from structural shape fixtures.
use super::*;

fn number(v: &Value) -> f64 {
    v.as_f64().unwrap()
}
fn request_case(case: &Value) -> (Dataset, RegressionRequest) {
    let data = &case["input"];
    let x = data["x"].as_array().unwrap();
    let mut cols = vec![Column::numeric(
        ColumnId(1),
        "y",
        data["y"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| Some(number(v)))
            .collect(),
    )
    .unwrap()];
    let mut predictors = Vec::new();
    let coding = &case["coding"];
    if coding["kind"] == "treatment" {
        let raw = coding["source_rows"].as_array().unwrap();
        for j in 0..4 {
            let id = ColumnId(j as u64 + 2);
            let factor = coding["factors"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["column"] == j);
            let text = raw[0][j].is_string();
            cols.push(if text {
                Column::text(
                    id,
                    &format!("group{j}"),
                    raw.iter()
                        .map(|r| Some(r[j].as_str().unwrap().into()))
                        .collect(),
                )
                .unwrap()
            } else {
                Column::numeric(
                    id,
                    &format!("x{j}"),
                    raw.iter().map(|r| Some(number(&r[j]))).collect(),
                )
                .unwrap()
            });
            predictors.push(RegressionPredictor {
                column: id,
                role: factor.map_or(PredictorRole::Numeric, |f| PredictorRole::Categorical {
                    reference: if text {
                        Cell::Text(f["reference"].as_str().unwrap().into())
                    } else {
                        Cell::Number(number(&f["reference"]))
                    },
                }),
            });
        }
    } else {
        for j in 0..x[0].as_array().unwrap().len() {
            let id = ColumnId(j as u64 + 2);
            cols.push(
                Column::numeric(
                    id,
                    &format!("x{j}"),
                    x.iter().map(|r| Some(number(&r[j]))).collect(),
                )
                .unwrap(),
            );
            predictors.push(RegressionPredictor {
                column: id,
                role: PredictorRole::Numeric,
            });
        }
    }
    let weight = ColumnId(cols.len() as u64 + 1);
    cols.push(
        Column::numeric(
            weight,
            "frequency",
            data["w"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| Some(v.as_u64().unwrap() as f64))
                .collect(),
        )
        .unwrap(),
    );
    (
        Dataset::new(cols).unwrap(),
        RegressionRequest {
            outcome: ColumnId(1),
            predictors,
            confidence: ConfidenceLevel::P95,
            filter: RowFilter::default(),
            frequency_weight: Some(weight),
        },
    )
}

fn run(dataset: &Dataset, request: &RegressionRequest) -> ComputedRegression {
    regression_with_budget(
        dataset,
        request,
        &JobControl::default(),
        &[],
        request.heap_bytes().unwrap(),
    )
    .unwrap()
}
fn decode_capture(text: &str, headroom: usize) -> AppResult<ComputedRegression> {
    let payload: Payload = guarded_checked(text, headroom, |p: &Payload, state| {
        state.borrow_mut().add(p.0.validation_scratch_bytes()?)?;
        p.0.validate()
    })?;
    Ok(payload.0)
}
fn saved(result: &ComputedRegression) -> Value {
    serde_json::to_value(Payload(result.clone())).unwrap()
}
fn reference_cases() -> Vec<Value> {
    serde_json::from_str::<Value>(include_str!(
        "../../../../tests/fixtures/regression-fit.json"
    ))
    .unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
fn ordinary() -> ComputedRegression {
    let case = reference_cases()
        .into_iter()
        .find(|c| c["name"] == "simple")
        .unwrap();
    let (data, request) = request_case(&case);
    run(&data, &request)
}
#[test]
fn actual_reference_captures_roundtrip_all_confidences() {
    let mut count = 0;
    let mut documents = Vec::new();
    for case in reference_cases() {
        let (dataset, request) = request_case(&case);
        for confidence in [
            ConfidenceLevel::P90,
            ConfidenceLevel::P95,
            ConfidenceLevel::P99,
        ] {
            let request = RegressionRequest {
                confidence,
                ..request.clone()
            };
            if case["policy"]["status"] == "unsupported" {
                assert!(regression_with_budget(
                    &dataset,
                    &request,
                    &JobControl::default(),
                    &[],
                    request.heap_bytes().unwrap()
                )
                .is_err());
                continue;
            }
            let result = run(&dataset, &request);
            let text = serde_json::to_string(&Payload(result.clone())).unwrap();
            assert_eq!(
                decode_capture(&text, MAX_OWNED_BYTES).unwrap(),
                result,
                "{}",
                case["name"]
            );
            if let RegressionOutcome::Fitted(f) = &result.outcome {
                let s = &f.statistics;
                println!(
                    "CAPTURE {}",
                    json!({"name":case["name"],"confidence":format!("{:?}",confidence),"M":result.fitted_rows,"N":result.frequency,"p":result.terms.len(),"condition":f.condition.value,"sse":s.sse.value,"sst":s.sst.value,"model_ss":s.model_ss.value,"r_squared":s.r_squared.value,"adjusted":s.adjusted_r_squared.value,"spread":s.residual_standard_error.value,"f":s.f.value})
                );
            }
            documents.push(json!({"name":case["name"],"confidence":format!("{:?}",confidence),"payload":saved(&result)}));
            count += 1;
        }
    }
    assert_eq!(count, 84);
    let scratch = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".build/regression-task6");
    std::fs::create_dir_all(&scratch).unwrap();
    std::fs::write(
        scratch.join("actual-captures.json"),
        serde_json::to_vec(&documents).unwrap(),
    )
    .unwrap();
}
#[test]
fn captured_energy_corruption_is_refused() {
    let good = ordinary();
    let mut bad = saved(&good);
    let s = &mut bad["outcome"]["fitted"]["statistics"];
    s["sse"] = estimate(0.);
    s["sst"] = estimate(20.);
    s["model_ss"] = estimate(200.);
    s["r_squared"] = estimate(1.);
    assert!(decode_capture(&bad.to_string(), MAX_OWNED_BYTES).is_err());
}

mod mutations;

mod states;
