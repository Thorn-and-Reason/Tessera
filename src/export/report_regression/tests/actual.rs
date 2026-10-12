use super::*;
#[test]
fn every_actual_regression_table_has_its_named_focusable_region() {
    for case in controls::cases() {
        let (_, result) = controls::run(case);
        let out = html(&result);
        let tables = out.matches("<table>").count();
        assert!(tables > 0);
        assert_eq!(out.matches("role=\"region\"").count(), tables, "{case}");
        assert_eq!(
            out.split("<div role=\"region\"")
                .skip(1)
                .filter(|region| region.split('>').next().unwrap().contains("tabindex=\"0\""))
                .count(),
            tables,
            "{case}"
        );
        assert_eq!(out.matches("</table></div>").count(), tables, "{case}");
    }
}
#[test]
fn actual_reduced_capture_retains_full_bounds_and_complete_exact_samples() {
    let (_, result) = controls::many(2001);
    let RegressionOutcome::Fitted(f) = &result.outcome else {
        panic!("fit unavailable")
    };
    let d = &f.residual_diagnostics;
    assert!(d.residual_reduced && d.qq_reduced);
    let out = html(&result);
    assert!(out.contains("Showing 2000 of 2001 fitted source rows"));
    assert!(out.contains("Showing 2000 of 2001 replicated observation ranks"));
    for e in d.residual_range.iter().chain(d.fitted_range.iter()) {
        let mut bytes = vec![];
        estimate(&mut bytes, e).unwrap();
        assert!(out.contains(&String::from_utf8(bytes).unwrap()));
    }
    assert_eq!(out.matches("<circle ").count(), 4000);
}
#[test]
fn actual_direct_captures_preserve_every_scalar_reason_and_identity() {
    for case in controls::cases() {
        let (_, result) = controls::run(case);
        let out = html(&result);
        let check = |value: &Estimate| {
            let mut bytes = Vec::new();
            let exact = if let Some(captured) = value.value {
                format!("title=\"Exact captured value: {captured}\"")
            } else {
                text(&mut bytes, value.reason.as_deref().unwrap_or("Unavailable")).unwrap();
                String::from_utf8(bytes).unwrap()
            };
            assert!(out.contains(&exact), "{case}: missing {exact}");
        };
        match &result.outcome {
            RegressionOutcome::Fitted(f) => {
                let s = &f.statistics;
                for e in [
                    &s.sse,
                    &s.sst,
                    &s.model_ss,
                    &s.residual_standard_error,
                    &s.r_squared,
                    &s.adjusted_r_squared,
                    &s.f,
                    &s.f_p,
                    &f.condition,
                    &f.outcome_scale,
                ] {
                    check(e);
                }
                for p in std::iter::once(&s.f_p).chain(f.coefficients.iter().map(|c| &c.p)) {
                    let mut bytes = Vec::new();
                    p_value(&mut bytes, p).unwrap();
                    assert!(
                        out.contains(&String::from_utf8(bytes).unwrap()),
                        "{case}: incorrect visible p formatting"
                    );
                }
                for c in &f.coefficients {
                    for e in [
                        &c.estimate,
                        &c.standard_error,
                        &c.t,
                        &c.p,
                        &c.interval[0],
                        &c.interval[1],
                    ] {
                        check(e);
                    }
                }
                for e in &f.term_scales {
                    check(e);
                }
                let d = &f.residual_diagnostics;
                for e in [
                    &d.residual_mean,
                    &d.residual_sample_sd,
                    &d.residual_range[0],
                    &d.residual_range[1],
                    &d.fitted_range[0],
                    &d.fitted_range[1],
                    &d.theoretical_range[0],
                    &d.theoretical_range[1],
                ] {
                    check(e);
                }
                for extent in [&d.leverage, &d.studentised, &d.cooks_distance] {
                    for e in &extent.range {
                        check(e);
                    }
                }
                for p in &d.residual_points {
                    for e in [
                        &p.fitted,
                        &p.residual,
                        &p.leverage,
                        &p.studentised,
                        &p.cooks_distance,
                    ] {
                        check(e);
                    }
                }
                for p in &d.qq_points {
                    check(&p.theoretical);
                    check(&p.residual);
                }
                assert!(
                    out.contains("chart-17-residual") && out.contains("chart-17-qq"),
                    "{case}"
                );
                if case == "tiny" {
                    assert!(s.sse.value.is_none());
                    assert!(!out.contains("Zero residual variance:"));
                }
                if case == "exact_zero" {
                    assert!(out.contains("Zero residual variance:"));
                }
                if case == "no_df" {
                    assert!(out.contains("There are no residual degrees of freedom."));
                }
                if case == "huge_n" {
                    assert_eq!(result.frequency, 1 << 53);
                }
            }
            RegressionOutcome::Unavailable(f) => {
                let mut bytes = Vec::new();
                text(&mut bytes, &f.reason).unwrap();
                assert!(out.contains(&String::from_utf8(bytes).unwrap()));
                assert!(!out.contains("<svg"));
            }
        }
        if let Some(dir) = std::env::var_os("TESSERA_TASK8_ARTIFACT_DIR") {
            let css = include_str!("../../report.rs")
                .split("<style>")
                .nth(1)
                .unwrap()
                .split("</style>")
                .next()
                .unwrap();
            let css = css.replace("\\\"", "\"");
            std::fs::write(std::path::Path::new(&dir).join(format!("{case}.html")), format!("<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>Direct regression: {case}</title><style>{css}</style><body><main>{out}</main></body></html>")).unwrap();
        }
    }
}
