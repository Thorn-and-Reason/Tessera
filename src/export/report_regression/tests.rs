use super::*;
mod actual;
mod fixtures;
use fixtures::{available, fitted, unavailable};

#[test]
fn table_regions_escape_long_captions_and_preserve_semantics() {
    let title = format!(
        "{}\"><img src=x onerror='bad'>&",
        "ŵ long caption ".repeat(500)
    );
    let mut bytes = vec![];
    table(&mut bytes, &title, &["Value <&>"]).unwrap();
    row(&mut bytes, "Exact scalar", &[&available(1e-200)]).unwrap();
    end_table(&mut bytes).unwrap();
    let out = String::from_utf8(bytes).unwrap();
    let mut escaped = vec![];
    text(&mut escaped, &title).unwrap();
    let escaped = String::from_utf8(escaped).unwrap();
    assert!(out.contains(&format!(
        "role=\"region\" aria-label=\"{escaped}\" tabindex=\"0\""
    )));
    assert!(out.contains(&format!("<table><caption>{escaped}</caption>")));
    assert!(out.contains("<th scope=\"col\">Value &lt;&amp;&gt;</th>"));
    assert!(out.contains("<th scope=\"row\">Exact scalar</th>"));
    assert!(out.ends_with("</tbody></table></div>"));
    assert!(!out.contains("<img"));
    if let Some(dir) = std::env::var_os("TESSERA_TASK8_ARTIFACT_DIR") {
        let css = include_str!("../report.rs")
            .split("<style>")
            .nth(1)
            .unwrap()
            .split("</style>")
            .next()
            .unwrap()
            .replace("\\\"", "\"");
        std::fs::write(std::path::Path::new(&dir).join("hostile-caption.html"), format!("<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>Long hostile caption control</title><style>{css}</style><body><main>{out}</main></body></html>")).unwrap();
    }
}

fn html(result: &ComputedRegression) -> String {
    let mut bytes = Vec::new();
    write(&mut bytes, RunId(17), result, &JobControl::default()).unwrap();
    String::from_utf8(bytes).unwrap()
}

#[test]
fn captured_values_and_distinct_accessible_plots_are_reported() {
    let result = fitted();
    let before = result.clone();
    let out = html(&result);
    assert_eq!(result, before);
    for expected in [
        "Captured outcome &lt;Y&gt;",
        "Captured predictor &amp; X",
        "chart-17-residual",
        "chart-17-qq",
        "Residual versus fitted values for Captured outcome &lt;Y&gt;",
        "Normal Q–Q plot of raw residuals for Captured outcome &lt;Y&gt;",
        "Theoretical normal quantile",
        "M: 4",
        "N: 10",
        "Expanded terms p: 2",
        "Residual df: 8",
        "title=\"Exact captured value: 1.25\"",
        "title=\"Exact captured value: 0.125\"",
        ">1.25</span>",
        ">0.12</span>",
        ">&lt;0.001</span>",
        "Source row (starting at 1)",
        "Cohort ordinal (starting at 1)",
        "Replicated rank (starting at 1)",
        "<tr><th scope=\"row\">1</th><td>3</td><td>2</td>",
        "Full-cohort diagnostic extents",
        "Outcome scale",
        "<summary>Exact retained residual diagnostics</summary>",
    ] {
        assert!(out.contains(expected), "missing {expected}");
    }
    assert_eq!(out.matches("<svg ").count(), 2);
    assert_eq!(out.matches("<title id=\"chart-17-residual\"").count(), 1);
    assert_eq!(out.matches("<title id=\"chart-17-qq\"").count(), 1);
    assert!(!out.contains("<Y>"));
}

#[test]
fn coding_references_and_all_captured_definitions_escape() {
    let result = fixtures::categorical();
    let out = html(&result);
    for expected in [
        "Categorical; reference:",
        "Treatment coding",
        "&quot; a&lt;&amp; &quot;",
        "&quot;&quot;",
        "&quot;B&quot;",
        "&quot; b &quot;",
        "Label &lt;script&gt;",
        "Captured filter",
        " equals ",
        " AND ",
        "Captured weight",
        "Definition &lt;img&gt;",
        "Missing definitions:",
        "Intercept",
        "versus reference",
        "Numerical version: 1",
        "Rank threshold:",
        "Condition warning threshold:",
    ] {
        assert!(out.contains(expected), "missing {expected}");
    }
    assert!(!out.contains("<script>"));
    assert!(!out.contains("<img>"));
}

#[test]
fn unavailable_fit_keeps_known_facts_and_exact_escaped_reason() {
    let result = fixtures::failed();
    let out = html(&result);
    assert!(out.contains("Unavailable fit: reference &lt;absent&gt; &amp; rejected"));
    assert!(out.contains("Expected terms: unavailable"));
    assert!(out.contains("Numerical rank: unavailable"));
    assert!(out.contains("Captured outcome &lt;Y&gt;"));
    assert!(out.contains("Complete-case excluded frequency: 5"));
    assert!(!out.contains("<svg"));
    let result = ComputedRegression {
        outcome: RegressionOutcome::Unavailable(RegressionFailure {
            kind: RegressionFailureKind::Unidentified,
            reason: "Dependent terms".into(),
            expected_terms: Some(2),
            rank: Some(1),
            implicated_terms: vec![1],
        }),
        ..result
    };
    let out = html(&result);
    assert!(out.contains("Expected terms: 2"));
    assert!(out.contains("Numerical rank: 1"));
    assert!(out.contains("Term 2: Captured predictor &amp; X"));
}

#[test]
fn independent_unavailable_scalars_do_not_hide_other_retained_values() {
    let mut result = fitted();
    let RegressionOutcome::Fitted(fit) = &mut result.outcome else {
        unreachable!()
    };
    fit.statistics.sse = unavailable("A nonzero estimate is below Float64 range.");
    fit.coefficients[1].standard_error = available(1e-200);
    fit.residual_diagnostics.residual_points[0].residual = unavailable("Residual < range");
    let out = html(&result);
    assert!(out.contains("A nonzero estimate is below Float64 range."));
    assert!(out.contains("Residual &lt; range"));
    assert!(out.contains("title=\"Exact captured value: 1.25\""));
    assert!(!out.contains("Zero residual variance:"));
    assert_eq!(out.matches("<circle ").count(), 5);
}

#[test]
fn exact_zero_and_no_df_have_specific_notices() {
    let mut result = fitted();
    let RegressionOutcome::Fitted(fit) = &mut result.outcome else {
        unreachable!()
    };
    fit.statistics.residual_standard_error = available(0.);
    let out = html(&result);
    assert!(out.contains("Zero residual variance:"));
    assert!(out.contains("do not establish error-free population effects"));
    let RegressionOutcome::Fitted(fit) = &mut result.outcome else {
        unreachable!()
    };
    fit.residual_df = 0;
    fit.statistics.residual_standard_error =
        unavailable("There are no residual degrees of freedom.");
    let out = html(&result);
    assert!(out.contains("There are no residual degrees of freedom."));
    assert!(!out.contains("Zero residual variance:"));
}

#[test]
fn reduced_plots_keep_full_cohort_bounds_and_exact_tables() {
    let mut result = fitted();
    let RegressionOutcome::Fitted(fit) = &mut result.outcome else {
        unreachable!()
    };
    fit.residual_diagnostics.residual_reduced = true;
    fit.residual_diagnostics.qq_reduced = true;
    fit.residual_diagnostics.fitted_range = [available(-1e300), available(1e300)];
    fit.residual_diagnostics.residual_range = [available(-100.), available(100.)];
    let out = html(&result);
    assert!(
        out.contains("Drawing values were reduced; summaries and bounds use the complete cohort.")
    );
    assert!(out.contains("Residual minimum"));
    assert!(out.contains("Exact retained residual diagnostics"));
    assert!(out.contains("Exact retained Q–Q ranks"));
    assert!(out.contains("undefined source rows"));
}

#[test]
fn unavailable_range_keeps_reasons_and_exact_rows() {
    let mut result = fitted();
    let RegressionOutcome::Fitted(fit) = &mut result.outcome else {
        unreachable!()
    };
    fit.residual_diagnostics.fitted_range[0] = unavailable("Fitted minimum < range");
    let out = html(&result);
    assert!(out.contains("Drawing unavailable because a full-cohort axis bound is unavailable."));
    assert!(out.contains("Fitted minimum &lt; range"));
    assert!(out.contains("Exact retained residual diagnostics"));
    assert_eq!(out.matches("<svg ").count(), 1);
}

#[test]
fn pre_cancelled_writer_produces_no_output() {
    let control = JobControl::default();
    control.cancel();
    let mut out = Vec::new();
    assert!(matches!(
        write(&mut out, RunId(17), &fitted(), &control),
        Err(crate::error::AppError::Cancelled)
    ));
    assert!(out.is_empty());
}

#[test]
fn writer_errors_propagate() {
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("failed output"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    assert!(matches!(
        write(&mut Broken, RunId(17), &fitted(), &JobControl::default()),
        Err(crate::error::AppError::Io(_))
    ));
}

#[test]
fn long_captured_text_cancels_without_resetting_outer_progress() {
    use crate::operation::CHECKPOINT_HOOK;
    let (sender, receiver) = std::sync::mpsc::channel();
    let control = JobControl::new(
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        Some(sender),
    );
    control.checkpoint(7, Some(11)).unwrap();
    let cancel = control.clone();
    let mut result = fitted();
    result.columns[0].name = "ŵ<&>".repeat(100_000);
    CHECKPOINT_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |p| {
            if p.done >= 8192 {
                cancel.cancel();
            }
        }))
    });
    let mut bytes = Vec::new();
    let output = write(&mut bytes, RunId(17), &result, &control);
    CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
    assert!(matches!(output, Err(crate::error::AppError::Cancelled)));
    assert!(bytes.len() < 20_000);
    assert_eq!(
        receiver.try_iter().collect::<Vec<_>>(),
        vec![crate::operation::Progress {
            done: 7,
            total: Some(11)
        }]
    );
}

#[test]
fn table_and_plot_writes_cancel_with_original_error_kind() {
    struct Cancels<'a> {
        control: &'a JobControl,
        needle: &'a str,
        bytes: Vec<u8>,
    }
    impl Write for Cancels<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.bytes.extend_from_slice(bytes);
            if String::from_utf8_lossy(&self.bytes).contains(self.needle) {
                self.control.cancel();
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    for needle in ["Original-unit coefficients", "<circle", "</section>"] {
        let control = JobControl::default();
        let mut writer = Cancels {
            control: &control,
            needle,
            bytes: vec![],
        };
        assert!(
            matches!(
                write(&mut writer, RunId(17), &fitted(), &control),
                Err(crate::error::AppError::Cancelled)
            ),
            "{needle}"
        );
    }
}

#[test]
fn partial_writes_are_completed_and_zero_writes_keep_write_zero() {
    struct Partial {
        bytes: Vec<u8>,
        limit: usize,
    }
    impl Write for Partial {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let count = self.limit.min(bytes.len());
            self.bytes.extend_from_slice(&bytes[..count]);
            Ok(count)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let result = fitted();
    let mut partial = Partial {
        bytes: vec![],
        limit: 3,
    };
    write(&mut partial, RunId(17), &result, &JobControl::default()).unwrap();
    assert_eq!(String::from_utf8(partial.bytes).unwrap(), html(&result));
    let mut zero = Partial {
        bytes: vec![],
        limit: 0,
    };
    assert!(
        matches!(write(&mut zero, RunId(17), &result, &JobControl::default()), Err(crate::error::AppError::Io(error)) if error.kind() == std::io::ErrorKind::WriteZero)
    );
}
