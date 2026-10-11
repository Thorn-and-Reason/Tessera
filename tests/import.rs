use std::sync::{atomic::AtomicBool, mpsc, Arc};
use tessera::{
    data::{Budget, Cell, Column, ColumnId, ColumnKind, Dataset, MissingRules, MAX_OWNED_BYTES},
    error::AppError,
    import::{
        commit_preview, commit_preview_with_budget, preview, preview_with_budget, Delimiter,
        ImportPreview, ImportWarning, MAX_INPUT_BYTES,
    },
    operation::JobControl,
};

fn inspect(text: &str) -> ImportPreview {
    preview(
        Arc::from(text.as_bytes()),
        Delimiter::Comma,
        &JobControl::default(),
    )
    .unwrap()
}
fn import(text: &str) -> Dataset {
    let p = inspect(text);
    commit_preview(&p, &p.default_choices().unwrap(), &JobControl::default()).unwrap()
}

// Catches incorrect quote, BOM, UTF-8, CRLF, TSV and final-field decoding.
#[test]
fn quoted_utf8_roundtrip() {
    let source =
        "\u{feff}Person\tComment\tEmpty\r\n\"Siân\"\t\"line one\r\nline \"\"two\"\"\"\t\r\n";
    let p = preview(
        Arc::from(source.as_bytes()),
        Delimiter::Tab,
        &JobControl::default(),
    )
    .unwrap();
    assert_eq!(p.row_count(), 1);
    assert_eq!(p.columns()[0].header, "Person");
    assert_eq!(p.sample_rows()[0], ["Siân", "line one\r\nline \"two\"", ""]);
    let ds = commit_preview(&p, &p.default_choices().unwrap(), &JobControl::default()).unwrap();
    assert_eq!(ds.cell(ColumnId(1), 0).unwrap(), Cell::Text("Siân".into()));
    assert_eq!(
        ds.cell(ColumnId(2), 0).unwrap(),
        Cell::Text("line one\r\nline \"two\"".into())
    );
    assert_eq!(ds.cell(ColumnId(3), 0).unwrap(), Cell::Text(String::new()));
    assert_eq!(import("A,B\n1,").row_count(), 1);
    assert_eq!(import("A\r1\r2\r").row_count(), 2);
}

// Catches lossy identifier inference, including exactly representable integers outside ±2^53.
#[test]
fn large_identifiers_and_blank_columns_stay_text() {
    for token in [
        "9007199254740993",
        "9007199254740994",
        "-9007199254740994",
        "9.007199254740993e15",
        "1e23",
    ] {
        let p = inspect(&format!("id\n{token}\n"));
        assert_eq!(p.columns()[0].inferred_kind, ColumnKind::Text);
        assert!(p.columns()[0]
            .warnings
            .contains(&ImportWarning::PrecisionRisk));
        let ds = commit_preview(&p, &p.default_choices().unwrap(), &JobControl::default()).unwrap();
        assert_eq!(ds.cell(ColumnId(1), 0).unwrap(), Cell::Text(token.into()));
        let mut choices = p.default_choices().unwrap();
        choices[0].kind = ColumnKind::Number;
        assert!(commit_preview(&p, &choices, &JobControl::default()).is_err());
        choices[0].acknowledge_precision_loss = true;
        assert!(commit_preview(&p, &choices, &JobControl::default()).is_ok());
    }
    assert_eq!(
        inspect("empty\n\"\"\n").columns()[0].inferred_kind,
        ColumnKind::Text
    );
    assert_eq!(
        inspect("value\n1\n2\nlate text\n").columns()[0].inferred_kind,
        ColumnKind::Text
    );
    assert_eq!(
        inspect("N\n9007199254740992\n").columns()[0].inferred_kind,
        ColumnKind::Number
    );
}

// Catches loss of empty text, numeric nulls or selected missing definitions, and partial publication.
#[test]
fn overrides_and_missing_rules_are_transactional() {
    let p = inspect("Numeric,Text\n1,a\n,\n-99,NA\n");
    let original = p.source_bytes().to_vec();
    let mut choices = p.default_choices().unwrap();
    choices[0].missing_rules = MissingRules::Number {
        values: vec![-99.0],
        ranges: vec![],
    };
    choices[1].missing_rules = MissingRules::Text(vec!["NA".into()]);
    let ds = commit_preview(&p, &choices, &JobControl::default()).unwrap();
    assert_eq!(ds.cell(ColumnId(1), 1).unwrap(), Cell::Missing);
    assert_eq!(ds.cell(ColumnId(2), 1).unwrap(), Cell::Text(String::new()));
    assert!(ds.column(ColumnId(1)).unwrap().is_missing(2).unwrap());
    assert!(ds.column(ColumnId(2)).unwrap().is_missing(2).unwrap());
    choices[1].kind = ColumnKind::Number;
    assert!(commit_preview(&p, &choices, &JobControl::default()).is_err());
    assert_eq!(p.source_bytes(), original);
    assert_eq!(p.columns()[1].inferred_kind, ColumnKind::Text);
    assert!(commit_preview(&p, &choices[..1], &JobControl::default()).is_err());
    let blank = inspect("N\n\"\"\n");
    let mut choices = blank.default_choices().unwrap();
    choices[0].kind = ColumnKind::Number;
    assert_eq!(
        commit_preview(&blank, &choices, &JobControl::default())
            .unwrap()
            .cell(ColumnId(1), 0)
            .unwrap(),
        Cell::Missing
    );
}

// Catches acceptance of malformed boundaries, leaking raw values, and preallocation over the budget.
#[test]
fn location_and_allocation_limits() {
    for text in [
        "A,B\n1\n",
        "A\n\"private\"junk\n",
        "A\n\"private",
        "A\nprivate\"quote\n",
    ] {
        let error = preview(
            Arc::from(text.as_bytes()),
            Delimiter::Comma,
            &JobControl::default(),
        )
        .unwrap_err();
        assert!(
            matches!(&error, AppError::InvalidInput { location, .. } if location.contains("row 2") && location.contains("field")),
            "{error}"
        );
        assert!(!error.to_string().contains("private"));
    }
    let error = preview(
        Arc::from(&b"A\n\xff"[..]),
        Delimiter::Comma,
        &JobControl::default(),
    )
    .unwrap_err();
    assert!(matches!(error, AppError::InvalidInput { .. }));
    assert!(preview(
        Arc::from(&b""[..]),
        Delimiter::Comma,
        &JobControl::default()
    )
    .is_err());
    let source: Arc<[u8]> = Arc::from(vec![b'a'; MAX_INPUT_BYTES + 1]);
    assert!(matches!(
        preview(source, Delimiter::Comma, &JobControl::default()),
        Err(AppError::SizeLimit {
            limit: MAX_INPUT_BYTES
        })
    ));
    // A large valid field fits the serialized ceiling, but retaining its source
    // while copying the text output must fail before that copy is allocated.
    let mut source = vec![b'x'; MAX_INPUT_BYTES];
    source[..2].copy_from_slice(b"T\n");
    source[MAX_INPUT_BYTES - 1] = b'\n';
    let p = preview(Arc::from(source), Delimiter::Comma, &JobControl::default()).unwrap();
    assert_eq!(p.row_count(), 1);
    assert!(p.samples_truncated());
    assert!(matches!(
        commit_preview(&p, &p.default_choices().unwrap(), &JobControl::default()),
        Err(AppError::SizeLimit { .. })
    ));
    drop(p);
    let source: Arc<[u8]> =
        Arc::from(format!("T\n\"{}\"\n", "a\"\"".repeat(256 * 1024)).into_bytes());
    assert!(matches!(
        preview_with_budget(
            source,
            Delimiter::Comma,
            &JobControl::default(),
            &[],
            MAX_OWNED_BYTES - 1024
        ),
        Err(AppError::SizeLimit { .. })
    ));
}

// Catches silently renamed invalid/duplicate headers and metadata that bypasses validation.
#[test]
fn headers_require_explicit_resolution() {
    let p = inspect("A,A,\"bad\nname\", \n1,2,3,4\n");
    assert!(p.columns()[1]
        .warnings
        .contains(&ImportWarning::DuplicateHeader));
    assert!(p.columns()[2]
        .warnings
        .contains(&ImportWarning::InvalidHeader));
    assert!(p.columns()[3]
        .warnings
        .contains(&ImportWarning::InvalidHeader));
    assert!(commit_preview(&p, &p.default_choices().unwrap(), &JobControl::default()).is_err());
    let mut choices = p.default_choices().unwrap();
    choices[1].name = "B".into();
    choices[2].name = "C".into();
    choices[3].name = "  D  ".into();
    let ds = commit_preview(&p, &choices, &JobControl::default()).unwrap();
    assert_eq!(ds.columns()[3].name(), "  D  ");
    choices[0].missing_rules = MissingRules::Number {
        values: vec![f64::NAN],
        ranges: vec![],
    };
    assert!(commit_preview(&p, &choices, &JobControl::default()).is_err());
}

// Catches missing current-project accounting before new typed buffers are allocated.
#[test]
fn aggregate_preview_and_current_project_accounting() {
    let retained = Dataset::new(vec![Column::text(
        ColumnId(1),
        "Existing",
        vec![Some("x".repeat(1024 * 1024))],
    )
    .unwrap()])
    .unwrap();
    let p = inspect("N\n1\n2\n");
    let retained_bytes = Budget::dataset_bytes(&[&retained], 0).unwrap();
    let extra = MAX_OWNED_BYTES - retained_bytes - p.owned_bytes();
    assert!(matches!(
        commit_preview_with_budget(
            &p,
            &p.default_choices().unwrap(),
            &JobControl::default(),
            &[&retained],
            extra
        ),
        Err(AppError::SizeLimit { .. })
    ));
    assert!(matches!(
        preview_with_budget(
            Arc::from(&b"N\n1\n"[..]),
            Delimiter::Comma,
            &JobControl::default(),
            &[&retained],
            MAX_OWNED_BYTES - retained_bytes
        ),
        Err(AppError::SizeLimit { .. })
    ));
    assert_eq!(
        commit_preview_with_budget(
            &p,
            &p.default_choices().unwrap(),
            &JobControl::default(),
            &[&retained],
            0
        )
        .unwrap()
        .row_count(),
        2
    );
}

// Catches cloning choice headers before accounting for retained project/work storage.
#[test]
fn default_choices_check_retained_project_before_cloning_headers() {
    let retained = Dataset::new(vec![Column::text(
        ColumnId(1),
        "Existing",
        vec![Some("x".repeat(1024 * 1024))],
    )
    .unwrap()])
    .unwrap();
    let source: Arc<[u8]> = Arc::from(format!("{}\n", "H".repeat(1024 * 1024)).into_bytes());
    let original = preview(source.clone(), Delimiter::Comma, &JobControl::default()).unwrap();
    let retained_bytes = Budget::dataset_bytes(&[&retained], 0).unwrap();
    let extra = MAX_OWNED_BYTES - retained_bytes - original.owned_bytes() - 96 * 1024;
    drop(original);
    let p = preview_with_budget(
        source,
        Delimiter::Comma,
        &JobControl::default(),
        &[&retained],
        extra,
    )
    .unwrap();
    assert!(matches!(
        p.default_choices_with_budget(&[&retained], extra),
        Err(AppError::SizeLimit { .. })
    ));
    let choices = p.default_choices_with_budget(&[&retained], 0).unwrap();
    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0].name.len(), 1024 * 1024);
    assert_eq!(p.source_bytes()[0], b'H');
}

// Catches cancellation checks limited to record boundaries and accidental mutation of source.
#[test]
fn cancelled_import_preserves_source() {
    let p = inspect("N\n1\n2\n");
    let control = JobControl::default();
    control.cancel();
    assert!(matches!(
        commit_preview(&p, &p.default_choices().unwrap(), &control),
        Err(AppError::Cancelled)
    ));
    assert_eq!(p.source_bytes(), b"N\n1\n2\n");
    let source: Arc<[u8]> =
        Arc::from(format!("T\n\"{}\"", "a".repeat(8 * 1024 * 1024)).into_bytes());
    let (sender, receiver) = mpsc::channel();
    let control = JobControl::new(Arc::new(AtomicBool::new(false)), Some(sender));
    let worker_control = control.clone();
    let original = source.clone();
    let worker = std::thread::spawn(move || preview(source, Delimiter::Comma, &worker_control));
    receiver.recv().unwrap();
    while let Ok(progress) = receiver.recv() {
        if progress.done > 4 && progress.done < original.len() {
            control.cancel();
            break;
        }
    }
    assert!(matches!(worker.join().unwrap(), Err(AppError::Cancelled)));
    assert_eq!(original[0], b'T');
    assert_eq!(*original.last().unwrap(), b'"');
}
