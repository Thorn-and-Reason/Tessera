use tessera::{
    analysis::{analyze_with_budget, AnalysisRequest, HistogramBins, HistogramRequest, RowFilter},
    data::{Column, ColumnId, Dataset},
    figures::{render, FigureKey, FigureKind, FigureSettings},
    operation::JobControl,
    project::Project,
};
fn project() -> Project {
    let data = Dataset::new(vec![Column::numeric(
        ColumnId(1),
        "A long bilingual outcome ŵ <&>",
        vec![Some(1.), Some(2.), Some(3.), None],
    )
    .unwrap()])
    .unwrap();
    let result = analyze_with_budget(
        &data,
        &AnalysisRequest::Histogram(HistogramRequest {
            variable: ColumnId(1),
            bins: HistogramBins::Count(2),
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        &JobControl::default(),
        &[],
        4096,
    )
    .unwrap();
    Project::new("Figures", data)
        .unwrap()
        .with_run(result)
        .unwrap()
}
#[test]
fn standalone_svg_and_png_preserve_captured_data_and_labels() {
    let p = project();
    let before = p.runs()[0].analysis.clone();
    let key = FigureKey {
        run: 1,
        kind: FigureKind::Histogram,
        variant: 0,
    };
    let settings = FigureSettings {
        title: "Title <&>".into(),
        ..Default::default()
    };
    let svg = render::svg(&p.runs()[0], &key, &settings, false, &JobControl::default()).unwrap();
    assert!(svg.contains("xmlns=\"http://www.w3.org/2000/svg\""));
    assert!(svg.contains("Title &lt;&amp;&gt;"));
    assert!(svg.contains("Missing"));
    assert!(!svg.contains("NaN"));
    assert!(!svg.contains("<script"));
    let png = render::png(&svg, &settings, &JobControl::default()).unwrap();

    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    let (_, _, rgba) = render::raster(&svg, 200, 125, &JobControl::default()).unwrap();
    assert_eq!(&rgba[rgba.len() - 4..], &[255, 255, 255, 255]);
    let decoder = png::Decoder::new(std::io::Cursor::new(&png));
    let reader = decoder.read_info().unwrap();
    assert_eq!(reader.info().pixel_dims.unwrap().xppu, 11811);
    assert_eq!(p.runs()[0].analysis, before);
}
#[test]
fn cancelled_render_and_invalid_key_fail() {
    let p = project();
    let key = FigureKey {
        run: 9,
        kind: FigureKind::Histogram,
        variant: 0,
    };
    let c = JobControl::default();
    assert!(render::svg(&p.runs()[0], &key, &FigureSettings::default(), false, &c).is_err());
    c.cancel();
    assert!(render::svg(&p.runs()[0], &key, &FigureSettings::default(), false, &c).is_err());
}
#[test]
fn export_is_physical_sized_accessible_and_atomic() {
    use tessera::export::figures::{export_figure, FigureFormat};
    let p = project();
    let key = FigureKey {
        run: 1,
        kind: FigureKind::Histogram,
        variant: 0,
    };
    let c = JobControl::default();
    let svg = render::svg(&p.runs()[0], &key, &FigureSettings::default(), true, &c).unwrap();
    assert!(svg.contains("width=\"160mm\""));
    assert!(svg.contains("role=\"img\""));
    assert!(
        svg.contains("aria-labelledby=\"figure-1-Histogram-0-title figure-1-Histogram-0-desc\"")
    );
    assert!(svg.contains("<title id=\"figure-1-Histogram-0-title\">"));
    assert!(svg.contains("<desc id=\"figure-1-Histogram-0-desc\">"));
    assert!(svg.contains("Stale result"));
    assert!(!svg.contains("<text"));
    let path = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("tessera-figure-{}.svg", std::process::id()));
    std::fs::write(&path, "original").unwrap();
    let cancelled = JobControl::default();
    cancelled.cancel();
    assert!(export_figure(&path, &p, &key, FigureFormat::Svg, &cancelled, &[], 0).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
    std::fs::remove_file(&path).unwrap();
    export_figure(&path, &p, &key, FigureFormat::Svg, &c, &[], 0).unwrap();
    assert!(std::fs::read_to_string(&path).unwrap().contains("<svg"));
    let png_path = path.with_extension("png");
    export_figure(&png_path, &p, &key, FigureFormat::Png, &c, &[], 0).unwrap();
    assert_eq!(
        &std::fs::read(&png_path).unwrap()[..8],
        b"\x89PNG\r\n\x1a\n"
    );
    std::fs::remove_file(png_path).unwrap();
    let invalid = FigureKey { variant: 99, ..key };
    assert!(export_figure(&path, &p, &invalid, FigureFormat::Svg, &c, &[], 0).is_err());
    std::fs::remove_file(path).unwrap();
}
#[test]
fn weighted_bars_keep_explicit_denominators_and_all_labels() {
    use tessera::{analysis::FrequenciesRequest, figures::FrequencyMetric};
    let data = Dataset::new(vec![
        Column::text(
            ColumnId(1),
            "Response",
            vec![Some("First ŵ <&>".into()), Some("Second".into()), None],
        )
        .unwrap(),
        Column::numeric(ColumnId(2), "Weight", vec![Some(2.), Some(3.), Some(5.)]).unwrap(),
    ])
    .unwrap();
    let result = analyze_with_budget(
        &data,
        &AnalysisRequest::Frequencies(FrequenciesRequest {
            group: None,
            columns: vec![ColumnId(1)],
            filter: RowFilter::default(),
            frequency_weight: Some(ColumnId(2)),
        }),
        &JobControl::default(),
        &[],
        4096,
    )
    .unwrap();
    let p = Project::new("Bars", data)
        .unwrap()
        .with_run(result)
        .unwrap();
    let key = FigureKey {
        run: 1,
        kind: FigureKind::Frequency(1),
        variant: 0,
    };
    for (metric, description) in [
        (
            FrequencyMetric::ValidPercent,
            "Denominator: valid frequency",
        ),
        (
            FrequencyMetric::AllPercent,
            "Denominator: valid plus missing frequency",
        ),
    ] {
        let s = FigureSettings {
            metric,
            ..Default::default()
        };
        let svg = render::svg(&p.runs()[0], &key, &s, false, &JobControl::default()).unwrap();
        assert!(svg.contains(description));
        assert!(svg.contains("First ŵ &lt;&amp;&gt;"));
        assert!(svg.contains("Second"));
        assert!(svg.contains("frequency 5"));
    }
}
#[test]
fn constant_extreme_and_custom_clipped_histograms_are_finite() {
    for values in [
        vec![Some(3.), Some(3.)],
        vec![Some(-f64::MAX), Some(f64::MAX)],
    ] {
        let data = Dataset::new(vec![
            Column::numeric(ColumnId(1), "Measure", values).unwrap()
        ])
        .unwrap();
        let result = analyze_with_budget(
            &data,
            &AnalysisRequest::Histogram(HistogramRequest {
                variable: ColumnId(1),
                bins: HistogramBins::Count(2),
                filter: RowFilter::default(),
                frequency_weight: None,
            }),
            &JobControl::default(),
            &[],
            4096,
        )
        .unwrap();
        let p = Project::new("Extrema", data)
            .unwrap()
            .with_run(result)
            .unwrap();
        let key = FigureKey {
            run: 1,
            kind: FigureKind::Histogram,
            variant: 0,
        };
        let s = FigureSettings {
            width_mm: 300.,
            height_mm: 150.,
            ticks: 2,
            ..Default::default()
        };
        let svg = render::svg(&p.runs()[0], &key, &s, false, &JobControl::default()).unwrap();
        assert!(!svg.contains("NaN"));
        assert!(!svg.contains("inf"));
    }
    let p = project();
    let key = FigureKey {
        run: 1,
        kind: FigureKind::Histogram,
        variant: 0,
    };
    let s = FigureSettings {
        x_bounds: Some([1.5, 2.5]),
        ..Default::default()
    };
    let svg = render::svg(&p.runs()[0], &key, &s, false, &JobControl::default()).unwrap();
    assert!(svg.contains("viewport are clipped"));
}
#[test]
fn too_many_categories_fail_without_omission() {
    use tessera::analysis::FrequenciesRequest;
    let data = Dataset::new(vec![Column::text(
        ColumnId(1),
        "Response",
        (0..100)
            .map(|i| Some(format!("Distinct response {i}")))
            .collect(),
    )
    .unwrap()])
    .unwrap();
    let result = analyze_with_budget(
        &data,
        &AnalysisRequest::Frequencies(FrequenciesRequest {
            group: None,
            columns: vec![ColumnId(1)],
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        &JobControl::default(),
        &[],
        4096,
    )
    .unwrap();
    let p = Project::new("Density", data)
        .unwrap()
        .with_run(result)
        .unwrap();
    let key = FigureKey {
        run: 1,
        kind: FigureKind::Frequency(1),
        variant: 0,
    };
    let error = render::svg(
        &p.runs()[0],
        &key,
        &FigureSettings::default(),
        false,
        &JobControl::default(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("no categories were omitted"));
}
#[path = "common/regression.rs"]
mod regression_fixture;
#[test]
fn captured_figure_gallery_renders_every_existing_family() {
    use tessera::{
        analysis::{BoxplotRequest, DescribeRequest, MissingPolicy, ScatterRequest, Statistic},
        figures::{figure_keys, FigurePalette},
    };
    let c = JobControl::default();
    let data = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Outcome",
            vec![Some(1.), Some(2.), Some(9.), Some(3.)],
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "Predictor",
            vec![Some(2.), Some(4.), Some(3.), Some(1.)],
        )
        .unwrap(),
        Column::text(
            ColumnId(3),
            "Group",
            vec![
                Some("First".into()),
                Some("First".into()),
                Some("Second".into()),
                Some("Second".into()),
            ],
        )
        .unwrap(),
    ])
    .unwrap();
    let requests = [
        AnalysisRequest::Scatter(ScatterRequest {
            x: ColumnId(1),
            y: ColumnId(2),
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        AnalysisRequest::Boxplot(BoxplotRequest {
            outcome: ColumnId(1),
            group: Some(ColumnId(3)),
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        AnalysisRequest::Describe(DescribeRequest {
            columns: vec![ColumnId(1)],
            statistics: vec![Statistic::Minimum, Statistic::Maximum],
            missing_policy: MissingPolicy::PerVariable,
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
    ];
    for request in requests {
        let result = analyze_with_budget(&data, &request, &c, &[], 4096).unwrap();
        let p = Project::new("Gallery", data.clone())
            .unwrap()
            .with_run(result)
            .unwrap();
        for key in figure_keys(&p.runs()[0]) {
            for palette in [
                FigurePalette::Report,
                FigurePalette::Monochrome,
                FigurePalette::Accessible,
            ] {
                let s = FigureSettings {
                    palette,
                    ..Default::default()
                };
                let svg = render::svg(&p.runs()[0], &key, &s, false, &c).unwrap();
                assert!(svg.contains("width=\"160mm\""));
                assert!(svg.contains("height=\"100mm\""));
                let (_, _, rgba) = render::raster(&svg, 320, 200, &c).unwrap();
                assert_eq!(rgba.len(), 320 * 200 * 4);
                assert!(!svg.contains("NaN"));
            }
        }
    }
    let p = regression_fixture::captured();
    for key in figure_keys(&p.runs()[0]) {
        let svg = render::svg(&p.runs()[0], &key, &FigureSettings::default(), false, &c).unwrap();
        assert!(svg.contains("literal replicated frequency"));
        assert!(!svg.contains("NaN"));
    }
}
#[test]
fn transparent_and_invalid_raster_sizes_are_checked() {
    let p = project();
    let key = FigureKey {
        run: 1,
        kind: FigureKind::Histogram,
        variant: 0,
    };
    let c = JobControl::default();
    let s = FigureSettings {
        transparent: true,
        ..Default::default()
    };
    let svg = render::svg(&p.runs()[0], &key, &s, false, &c).unwrap();
    let (_, _, rgba) = render::raster(&svg, 200, 125, &c).unwrap();
    assert_eq!(rgba.last(), Some(&0));
    assert!(render::raster(&svg, 0, 1, &c).is_err());
    assert!(render::raster(&svg, 10000, 10000, &c).is_err());
    assert!(render::raster("invalid", 20, 20, &c).is_err());
}
#[test]
fn cropped_categorical_figures_keep_valid_visible_labels_and_full_capture() {
    use tessera::analysis::{BoxplotRequest, FrequenciesRequest};
    let c = JobControl::default();
    let data = Dataset::new(vec![
        Column::text(
            ColumnId(1),
            "Group",
            vec![
                Some("First".into()),
                Some("Second".into()),
                Some("Third".into()),
            ],
        )
        .unwrap(),
        Column::numeric(ColumnId(2), "Outcome", vec![Some(1.), Some(2.), Some(3.)]).unwrap(),
    ])
    .unwrap();
    let requests = [
        AnalysisRequest::Frequencies(FrequenciesRequest {
            group: None,
            columns: vec![ColumnId(1)],
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
        AnalysisRequest::Boxplot(BoxplotRequest {
            outcome: ColumnId(2),
            group: Some(ColumnId(1)),
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
    ];
    for request in requests {
        let result = analyze_with_budget(&data, &request, &c, &[], 4096).unwrap();
        let p = Project::new("Cropped categories", data.clone())
            .unwrap()
            .with_run(result)
            .unwrap();
        let before = p.runs()[0].analysis.clone();
        let kind = if matches!(request, AnalysisRequest::Frequencies(_)) {
            FigureKind::Frequency(1)
        } else {
            FigureKind::Boxplot
        };
        let key = FigureKey {
            run: 1,
            kind,
            variant: 0,
        };
        for horizontal in [false, true] {
            if horizontal && kind == FigureKind::Boxplot {
                continue;
            }
            let settings = FigureSettings {
                horizontal,
                x_bounds: (!horizontal).then_some([1., 2.]),
                y_bounds: horizontal.then_some([1., 2.]),
                ..Default::default()
            };
            let svg = render::svg(&p.runs()[0], &key, &settings, false, &c).unwrap();
            for full_label in ["First", "Second", "Third"] {
                assert!(svg.contains(full_label));
            }
            assert!(svg.contains("values outside the viewport are clipped"));
            let (_, _, rgba) = render::raster(&svg, 320, 200, &c).unwrap();
            assert_eq!(rgba.len(), 320 * 200 * 4);
            assert_eq!(p.runs()[0].analysis, before);
        }
    }
}
