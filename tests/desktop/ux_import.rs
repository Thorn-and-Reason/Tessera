use super::*;

#[test]
fn csv_import_review_explains_headers_delimiter_and_selected_conversion() {
    let directory = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "tessera-import-review-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("identifiers.csv");
    std::fs::write(&path, "Identifier,Amount,Comment\n00123,,\n00007,3.50,\n").unwrap();

    let mut state = state();
    let mut jobs = JobRunner::default();
    run(
        &mut state,
        &mut jobs,
        JobOperation::PreviewPath {
            path,
            delimiter: tessera::import::Delimiter::Comma,
        },
    );
    assert_eq!(state.dialog(), Some(Dialog::Import));

    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let labels = super::review::access_labels(&ctx, &mut state, &mut jobs);
    let text = labels.join("\n");
    for expected in [
        "First row is used as variable names. Review each variable before importing.",
        "Changing delimiter previews the file again and resets reviewed choices.",
        "Selected conversion preview",
        "Source",
        "Stored",
        "Keep as Text to preserve leading zeros and exact characters.",
        "Source samples",
        "Identifier",
    ] {
        assert!(text.contains(expected), "missing {expected}; got {text}");
    }

    std::fs::remove_dir_all(directory).unwrap();
}
