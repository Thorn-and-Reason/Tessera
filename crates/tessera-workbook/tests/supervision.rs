use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
    time::{Duration, Instant},
};
use tessera_workbook::{
    protocol::Value,
    supervisor::{inspect, run, run_snapshot, Config, WorkbookSnapshot},
};
fn worker() -> PathBuf {
    env!("CARGO_BIN_EXE_tessera-workbook-worker").into()
}
fn config(name: &str) -> Config {
    Config {
        worker: worker(),
        source: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(name),
        worksheet_index: 0,
        range: [0, 0, 1, 1],
        quota: 2 * 1024 * 1024,
        retained_bytes: 0,
        timeout: Duration::from_secs(3),
        cancel: Arc::new(AtomicBool::new(false)),
        probe: None,
    }
}
#[test]
fn valid_cached_formula_roundtrip_and_sparse_selection() {
    let cells = run(config("tiny.xlsx")).unwrap();
    assert_eq!(cells.len(), 4);
    assert_eq!(cells[2].value, Value::Text("00123".into()));
    assert_eq!(cells[3].flags, 33);
    let cells = run(config("sparse-distant.xlsx")).unwrap();
    assert_eq!(cells.len(), 1);
}
#[test]
fn hostile_allocations_and_zip_expansion_fail_without_partial_return() {
    for name in ["unique-count-bomb.xlsx", "zip-expansion.xlsx"] {
        assert!(run(config(name)).is_err(), "{name}");
    }
}
#[test]
fn timeout_cancellation_flood_and_partial_protocol_are_rejected() {
    for mode in ["sleep", "flood", "partial", "bad-coordinate"] {
        let mut c = config("tiny.xlsx");
        c.probe = Some(mode.into());
        c.timeout = Duration::from_millis(150);
        let start = Instant::now();
        assert!(run(c).is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    let mut c = config("tiny.xlsx");
    c.probe = Some("sleep".into());
    let flag = Arc::clone(&c.cancel);
    let cancel = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    let start = Instant::now();
    assert!(run(c).is_err());
    cancel.join().unwrap();
    assert!(start.elapsed() < Duration::from_secs(2));
}
#[test]
fn aggregate_admission_rejects_before_spawn() {
    let mut c = config("tiny.xlsx");
    c.retained_bytes = 256 * 1024 * 1024;
    c.worker = "missing-worker".into();
    assert_eq!(run(c).unwrap_err(), "aggregate ownership budget exceeded");
}
#[test]
fn date_epochs_fictitious_day_duration_missing_cache_and_precision_are_explicit() {
    for (name, epoch, flags) in [("dates-1900.xlsx", false, 16), ("dates-1904.xlsx", true, 8)] {
        let cells = run(config(name)).unwrap();
        assert_eq!(
            cells[0].value,
            Value::ExcelDate {
                serial: 60.0,
                epoch_1904: epoch,
                duration: false
            }
        );
        assert_eq!(cells[0].flags, flags);
        assert_eq!(
            cells[1].value,
            Value::ExcelDate {
                serial: 1.5,
                epoch_1904: epoch,
                duration: true
            }
        );
        assert_eq!(cells[2].value, Value::Missing);
        assert_eq!(cells[2].flags, 3);
        assert_eq!(cells[3].flags, 32);
        // Calamine has already rounded the XML integer; the flag states raw lexeme unavailable.
        assert_eq!(cells[3].value, Value::Number(9007199254740992.0));
    }
}
#[test]
fn inspect_select_and_source_replacement_use_one_retained_snapshot() {
    let original = std::env::temp_dir().join(format!(
        "excel-original-{}-{}.xlsx",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::copy(config("multi-sheet.xlsx").source, &original).unwrap();
    let mut c = config("multi-sheet.xlsx");
    c.source = original.clone();
    let snapshot = WorkbookSnapshot::prepare(&c.source, c.retained_bytes, &c.cancel).unwrap();
    let identity = snapshot.source_identity();
    let owned = snapshot.owned_bytes();
    let info = inspect(&snapshot, &c).unwrap();
    assert_eq!(info.sheets.len(), 2);
    assert_eq!(info.sheets[1].name, "Ail ddalen");
    assert_eq!(info.source_identity, identity);
    assert!(owned >= std::fs::metadata(&original).unwrap().len() as usize);
    std::fs::write(&original, b"the original has been replaced").unwrap();
    c.worksheet_index = 1;
    let cells = run_snapshot(&snapshot, &c).unwrap();
    assert_eq!(cells[0].value, Value::Number(777.0));
    assert_eq!(cells[0].flags, 32);
    assert_eq!(snapshot.source_identity(), identity);
    c.worksheet_index = 2;
    assert!(run_snapshot(&snapshot, &c).is_err());
    std::fs::remove_file(original).unwrap();
}
#[test]
fn oversized_workbook_metadata_is_rejected() {
    for name in ["too-many-sheets.xlsx", "oversized-sheet-name.xlsx"] {
        let c = config(name);
        let snapshot = WorkbookSnapshot::prepare(&c.source, c.retained_bytes, &c.cancel).unwrap();
        assert!(inspect(&snapshot, &c).is_err());
    }
}
