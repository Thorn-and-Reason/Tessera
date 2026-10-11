#[path = "common/regression.rs"]
mod fixture;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use tessera::{
    data::{Cell, ColumnId, MAX_OWNED_BYTES},
    error::AppError,
    export::{export_report, export_report_with_budget},
    files::{read_project, save_project},
    operation::{JobControl, RunId},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "tessera-regression-output-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn real_file_save_fresh_open_and_frozen_report_do_not_recompute() {
    let temp = Temp::new();
    let p = fixture::captured();
    let path = temp.0.join("regression.tessera");
    save_project(&path, &p, None, &JobControl::default()).unwrap();
    let (reopened, _) = read_project(&path, &JobControl::default()).unwrap();
    assert_eq!(p.runs(), reopened.runs());
    let before = reopened.runs()[0].analysis.clone();
    let changed = reopened
        .with_dataset(
            reopened
                .dataset()
                .edit_cell(ColumnId(1), 0, Cell::Number(-900.))
                .unwrap(),
        )
        .unwrap();
    let report = temp.0.join("report.html");
    export_report(&report, &changed, &[RunId(1)], &JobControl::default()).unwrap();
    let html = fs::read_to_string(report).unwrap();
    for expected in [
        "Linear regression",
        "Historical results: the current data revision differs.",
        "Outcome &lt;Y&gt;",
        "Predictor &amp; X",
        "Treatment coding",
        "Fitted source rows M: 6",
    ] {
        assert!(html.contains(expected), "missing {expected}");
    }
    assert_eq!(html.matches("<svg ").count(), 2);
    for key in tessera::figures::figure_keys(&changed.runs()[0]) {
        let svg = tessera::figures::render::svg(
            &changed.runs()[0],
            &key,
            &changed.figure_settings(&key),
            true,
            &JobControl::default(),
        )
        .unwrap();
        assert!(html.contains(&svg));
    }
    assert_eq!(html.matches("id=\"run-1\"").count(), 1);
    let tessera::analysis::CompletedAnalysis::Regression(r) = &before else {
        panic!()
    };
    let tessera::analysis::RegressionOutcome::Fitted(f) = &r.outcome else {
        panic!()
    };
    for c in &f.coefficients {
        assert!(html.contains(&format!("<td>{}</td>", c.estimate.value.unwrap())));
    }
    assert_eq!(changed.runs()[0].analysis, before);
    assert!(!html.contains("-900"));
}
#[test]
fn report_and_save_refusal_or_cancellation_preserve_existing_destination() {
    let temp = Temp::new();
    let p = fixture::captured();
    let path = temp.0.join("preserve.html");
    fs::write(&path, "original destination").unwrap();
    let result = export_report_with_budget(
        &path,
        &p,
        &[RunId(1)],
        &JobControl::default(),
        &[],
        MAX_OWNED_BYTES,
    );
    assert!(matches!(result, Err(AppError::SizeLimit { .. })));
    let control = JobControl::default();
    control.cancel();
    assert!(matches!(
        export_report(&path, &p, &[RunId(1)], &control),
        Err(AppError::Cancelled)
    ));
    assert!(matches!(
        save_project(&path, &p, None, &control),
        Err(AppError::Cancelled)
    ));
    assert_eq!(fs::read_to_string(path).unwrap(), "original destination");
    assert_eq!(p.runs().len(), 1);
}
