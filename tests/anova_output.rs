mod common;
#[path = "common/anova.rs"]
mod fixture;
use tessera::{
    data::MAX_OWNED_BYTES,
    export::{export_report, export_report_with_budget},
    operation::{JobControl, RunId},
};
#[test]
fn all_design_reports_escape_identities_and_keep_full_families() {
    let temp = fixture::Temp::new();
    for design in 0..3 {
        let p = fixture::captured(design);
        let file = temp.0.join(format!("anova-{design}.html"));
        export_report(&file, &p, &[RunId(1)], &JobControl::default()).unwrap();
        let html = std::fs::read_to_string(file).unwrap();
        for s in [
            "ANOVA",
            "Captured cells",
            "Fixed family size m: 1",
            "Raw two-sided p",
            "Holm-adjusted p",
            "Pointwise lower bound",
            "first requested level minus second",
        ] {
            assert!(html.contains(s), "missing {s}");
        }
        for private in [
            "private-person-",
            "private-missing-ID",
            "private-ID-label",
            "private-filter-ID",
        ] {
            assert!(!html.contains(private), "leaked {private}");
        }
        assert!(!html.contains("Outcome <script>"));
        if design < 2 {
            assert!(html.contains("Outcome &lt;script&gt;"));
        } else {
            assert!(html.contains("Greenhouse-Geisser"));
        }
    }
}
#[test]
fn report_refusal_and_cancellation_preserve_destination() {
    let temp = fixture::Temp::new();
    let p = fixture::captured(0);
    let file = temp.0.join("report.html");
    std::fs::write(&file, "original").unwrap();
    assert!(export_report_with_budget(
        &file,
        &p,
        &[RunId(1)],
        &JobControl::default(),
        &[],
        MAX_OWNED_BYTES
    )
    .is_err());
    let c = JobControl::default();
    c.cancel();
    assert!(export_report(&file, &p, &[RunId(1)], &c).is_err());
    assert_eq!(std::fs::read_to_string(file).unwrap(), "original");
}
