//! Exhaustive checks for algebraically exact fits, conditional on computed energy.
use super::*;
use serde_json::json;

fn published(e: &Estimate) -> Value {
    json!({"status":if e.value.is_some(){"available"}else{"unavailable"},"value":e.value,"reason":e.reason})
}
fn available(v: f64) -> Value {
    assert!(v.is_finite());
    json!({"status":"available","value":v,"reason":null})
}
fn unavailable(reason: &str) -> Value {
    json!({"status":"unavailable","value":null,"reason":reason})
}
fn record(fit: &ScaledFit, case: &Value, confidence: ConfidenceLevel, corrupt: bool) -> Value {
    let control = JobControl::default();
    let (coefficients, mut s) =
        super::super::super::inference::infer(fit, confidence, &control).unwrap();
    if corrupt {
        s.sst = Estimate::unavailable("corrupted");
    }
    json!({
        "coefficients":coefficients.iter().map(|c|json!({"estimate":published(&c.estimate),"standard_error":published(&c.standard_error),"t":published(&c.t),"p":published(&c.p),"interval":c.interval.iter().map(published).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        "statistics":{"sse":published(&s.sse),"sst":published(&s.sst),"model_ss":published(&s.model_ss),"residual_standard_error":published(&s.residual_standard_error),"r_squared":published(&s.r_squared),"adjusted_r_squared":published(&s.adjusted_r_squared),"f":published(&s.f),"f_p":published(&s.f_p)},
        "covariance":(0..fit.rank()).map(|j|(0..fit.rank()).map(|k|published(&fit.covariance(j,k,&control).unwrap())).collect::<Vec<_>>()).collect::<Vec<_>>(),
        "fitted":case["input"]["y"].as_array().unwrap().iter().zip(fit.residuals()).map(|(y,e)|published(&fit.fitted_estimate(fit.normalised_outcome(number(y))-e))).collect::<Vec<_>>(),
        "residual":fit.residuals().iter().map(|e|published(&fit.residual_estimate(*e))).collect::<Vec<_>>()
    })
}
fn primary(fit: &ScaledFit, case: &Value) -> Value {
    let design = case["input"]["x"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            std::iter::once(1.)
                .chain(
                    x.as_array()
                        .unwrap()
                        .iter()
                        .zip(&fit.scales)
                        .map(|(x, s)| ((number(x) - s.anchor) / s.delta - s.mean) / s.sd),
                )
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    json!({"scales":fit.scales.iter().chain(std::iter::once(&fit.outcome)).map(|s|json!([s.anchor,s.delta,s.mean,s.sd])).collect::<Vec<_>>(),"design":design,"outcome":case["input"]["y"].as_array().unwrap().iter().map(|y|fit.normalised_outcome(number(y))).collect::<Vec<_>>(),"gamma":fit.gamma,"residual":fit.residuals()})
}
fn tail(t: f64, df: u64, reference: &Value) -> f64 {
    // Student's leading large-t term; the relative omitted term is bounded by
    // df²(df+1)/(2(df+2)t²), from (1+df/t²)^(-(df+1)/2).
    let t = t.abs();
    let d = df as f64;
    assert!(d * d * (d + 1.) / (2. * (d + 2.) * t * t) < 1e-12);
    number(&reference["student_tail_limit"]["value"]) * t.powf(-d)
}
fn expectation(fit: &ScaledFit, case: &Value, key: &str) -> Value {
    let truth = &case["reference"]["fit"];
    let route = &case["reference"]["route_reference"];
    let df = fit.residual_df();
    let n = fit.frequency() as f64;
    let unit = fit.outcome.delta * fit.outcome.sd;
    let rows = case["input"]["w"].as_array().unwrap();
    let sse = fit
        .residuals()
        .iter()
        .zip(rows)
        .map(|(e, w)| number(w) * (e * unit).powi(2))
        .sum::<f64>();
    let variance = if df > 0 { sse / df as f64 } else { 0. };
    let sst = number(&truth["statistics"]["sst"]["value"]);
    let predicted = primary(fit, case)["design"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            row.as_array()
                .unwrap()
                .iter()
                .zip(&fit.gamma)
                .map(|(z, g)| number(z) * g)
                .sum::<f64>()
        })
        .collect::<Vec<_>>();
    let model = predicted
        .iter()
        .zip(rows)
        .map(|(v, w)| number(w) * (v * unit).powi(2))
        .sum::<f64>();
    let reason = if df == 0 {
        "There are no residual degrees of freedom."
    } else {
        "Residual variance is exactly zero."
    };
    let coefficients=truth["coefficients"].as_array().unwrap().iter().enumerate().map(|(j,c)|{
        let b=number(&c["estimate"]["value"]);
        let se=(variance*number(&route["inverse_gram"][j][j]["value"])).sqrt();
        let critical=if df>0{number(&route["critical_values"][key]["value"])}else{0.};
        let t=if se>0.{b/se}else{0.};
        json!({"estimate":available(b),"standard_error":if df>0{available(se)}else{unavailable(reason)},"t":if df>0&&sse>0.{available(t)}else{unavailable(reason)},"p":if df>0&&sse>0.{available(if b==0.{1.}else{tail(t,df,route)})}else{unavailable(reason)},"interval":if df>0{vec![available(b-critical*se),available(b+critical*se)]}else{vec![unavailable(reason);2]}})
    }).collect::<Vec<_>>();
    let f = if sse > 0. {
        model * df as f64 / sse / (fit.rank() - 1) as f64
    } else {
        0.
    };
    json!({"coefficients":coefficients,
        "statistics":{"sse":available(sse),"sst":available(sst),"model_ss":available(model),"residual_standard_error":if df>0{available(variance.sqrt())}else{unavailable(reason)},"r_squared":if sst>0.{available(model/sst)}else{unavailable("The outcome has no variation.")},"adjusted_r_squared":if df==0{unavailable(reason)}else if sst==0.{unavailable("The outcome has no variation.")}else{available(1.-variance*(n-1.)/sst)},"f":if df>0&&sse>0.{available(f)}else{unavailable(reason)},"f_p":if df>0&&sse>0.{assert_eq!(fit.rank(),2);available(tail(f.sqrt(),df,route))}else{unavailable(reason)}},
        "covariance":(0..fit.rank()).map(|j|(0..fit.rank()).map(|k|if df>0{available(variance*number(&route["inverse_gram"][j][k]["value"]))}else{unavailable(reason)}).collect::<Vec<_>>()).collect::<Vec<_>>(),
        "fitted":truth["fitted"].as_array().unwrap().iter().map(|v|available(number(&v["value"]))).collect::<Vec<_>>(),
        "residual":fit.residuals().iter().map(|e|available(e*unit)).collect::<Vec<_>>()
    })
}
fn compare(actual: &Value, expected: &Value) -> bool {
    if let Some(e) = expected.as_object() {
        let Some(a) = actual.as_object() else {
            return false;
        };
        if a.len() != e.len() || a.keys().ne(e.keys()) {
            return false;
        }
        if e.contains_key("status") {
            if a["status"] != e["status"] || a["reason"] != e["reason"] {
                return false;
            }
            if e["status"] == "unavailable" {
                return a["value"].is_null();
            }
            let Some(v) = a["value"].as_f64() else {
                return false;
            };
            let w = number(&e["value"]);
            return v.is_finite() && (w == 0. || v != 0.) && (v - w).abs() <= w.abs() * 1e-10;
        }
        return e.iter().all(|(k, v)| compare(&a[k], v));
    }
    if let Some(e) = expected.as_array() {
        return actual
            .as_array()
            .is_some_and(|a| a.len() == e.len() && a.iter().zip(e).all(|(a, e)| compare(a, e)));
    }
    actual == expected
}
fn mutation_checks(actual: &Value, expected: &Value) -> usize {
    fn paths(v: &Value, path: String, out: &mut Vec<String>) {
        if v.get("status").is_some() {
            out.push(path);
            return;
        }
        if let Some(a) = v.as_array() {
            for (i, v) in a.iter().enumerate() {
                paths(v, format!("{path}/{i}"), out);
            }
        }
        if let Some(a) = v.as_object() {
            for (k, v) in a {
                paths(v, format!("{path}/{k}"), out);
            }
        }
    }
    let mut all = vec![];
    paths(actual, String::new(), &mut all);
    for path in &all {
        for kind in 0..4 {
            let mut bad = actual.clone();
            let v = bad.pointer_mut(path).unwrap();
            match kind {
                0 => v["status"] = json!("corrupted"),
                1 => v["reason"] = json!("corrupted"),
                2 => {
                    v.as_object_mut().unwrap().remove("value");
                }
                _ => {
                    v["value"] = json!(v["value"].as_f64().map_or(1., |n| if n == 0. {
                        1.
                    } else {
                        n * 1.01
                    }))
                }
            }
            assert!(!compare(&bad, expected), "accepted {path}/{kind}");
        }
    }
    all.len() * 4
}
pub(super) fn check(case: &Value, fit: &ScaledFit, corrupt: bool) {
    // The independent fixture explicitly permits route-dependent computed zero.
    // Never require a platform to invent positive rounding residue. The checks
    // below still compare every scalar, availability state and reason for the
    // actually computed energy, and exercise corruption rejection.
    assert!(fit.residual_norm().is_finite() && fit.residual_norm() >= 0.);
    let mut outputs = serde_json::Map::new();
    let mut mutations = 0;
    for (level, key) in [
        (ConfidenceLevel::P90, "90"),
        (ConfidenceLevel::P95, "95"),
        (ConfidenceLevel::P99, "99"),
    ] {
        let actual = record(fit, case, level, corrupt);
        let expected = expectation(fit, case, key);
        assert!(
            compare(&actual, &expected),
            "route fields {} at {key}: actual={actual} expected={expected}",
            case["name"]
        );
        mutations += mutation_checks(&actual, &expected);
        outputs.insert(key.into(), actual);
    }
    println!(
        "FIT_ROUTE_RESULT {}",
        json!({"name":case["name"],"primary":primary(fit,case),"outputs":outputs})
    );
    println!("ROUTE_MUTATIONS {} {mutations}", case["name"]);
}
