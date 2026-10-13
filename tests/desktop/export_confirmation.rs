use super::*;

#[test]
fn formula_export_modal_requires_an_explicit_choice() {
    let directory = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "tessera-export-modal-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("raw.csv");
    let mut state = DesktopState::from_project(
        Project::new(
            "Export",
            Dataset::new(vec![Column::text(
                ColumnId(1),
                "text",
                vec![Some("=1+1".into())],
            )
            .unwrap()])
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let mut jobs = JobRunner::default();
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    let size = Vec2::new(1000., 760.);
    for accept in [false, true] {
        let project = Arc::clone(state.project());
        run(
            &mut state,
            &mut jobs,
            JobOperation::ExportData {
                project,
                path: path.clone(),
                delimiter: tessera::import::Delimiter::Comma,
                acknowledge_formula_risk: false,
            },
        );
        assert_eq!(state.dialog(), Some(Dialog::ExportDataConfirmation));
        assert!(!path.exists());
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
        frame(&ctx, &mut state, &mut jobs, size, vec![]);
        click(
            &ctx,
            &mut state,
            &mut jobs,
            if accept {
                "Acknowledge risk and export"
            } else {
                "Cancel export"
            },
            size,
        );
        let until = Instant::now() + Duration::from_secs(10);
        while jobs.is_running() {
            state.poll(&mut jobs);
            assert!(Instant::now() < until);
            std::thread::yield_now();
        }
        assert!(state.error().is_none(), "{:?}", state.error());
        assert_ne!(state.dialog(), Some(Dialog::ExportDataConfirmation));
        assert_eq!(path.exists(), accept);
    }
    assert_eq!(std::fs::read_to_string(path).unwrap(), "text\r\n=1+1\r\n");
    std::fs::remove_dir_all(directory).unwrap();
}
