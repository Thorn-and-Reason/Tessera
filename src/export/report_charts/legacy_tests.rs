use crate::{
    analysis::*,
    data::{Column, ColumnId, Dataset},
    operation::{JobControl, RunId},
    project::AnalysisRun,
};
use sha2::{Digest, Sha256};

// Normalize only the approved visual delta; legacy hashes still guard coordinates and captures.
fn legacy_style(svg: &str) -> String {
    let svg = svg
        .replace(
            "stroke=\"#F2EBDC\" class=\"chart-bar\"",
            "stroke=\"#FFFFFF\" class=\"chart-bar\"",
        )
        .replace("#263449", "#023047")
        .replace("#C59B43", "#219EBC")
        .replace("#86631F", "#219EBC")
        .replace("#F2EBDC", "#8ECAE6");
    let mut result = String::with_capacity(svg.len());
    let mut median_edge = None;
    for tag in svg.split_inclusive('>') {
        if tag.starts_with("<path class=\"chart-grid\"") {
            continue;
        }
        if tag.contains("class=\"chart-median\"") {
            let path = tag
                .split(" d=\"")
                .nth(1)
                .unwrap()
                .split('"')
                .next()
                .unwrap();
            assert_eq!(
                median_edge.take(),
                Some(path),
                "median overlay must retain the exact edge geometry"
            );
            continue;
        }
        if tag.contains("class=\"chart-median-edge\"") {
            median_edge = Some(
                tag.split(" d=\"")
                    .nth(1)
                    .unwrap()
                    .split('"')
                    .next()
                    .unwrap(),
            );
        }
        result.push_str(
            &tag.replace(
                "stroke=\"#219EBC\" stroke-width=\"8\"",
                "stroke=\"#023047\" stroke-width=\"8\"",
            )
            .replace(
                "stroke=\"#FFFFFF\" class=\"chart-bar\"",
                "stroke=\"#023047\"",
            )
            .replace("fill=\"#219EBC\" class=\"chart-point\"", "fill=\"#023047\"")
            .replace(
                "fill=\"#8ECAE6\" stroke=\"#023047\" class=\"chart-box\"",
                "fill=\"#EAF5FB\" stroke=\"#023047\"",
            )
            .replace(
                "stroke-width=\"5\" class=\"chart-median-edge\"",
                "stroke-width=\"2\"",
            ),
        );
    }
    assert!(
        median_edge.is_none(),
        "median edge must have its approved overlay"
    );
    result
}

#[test]
fn legacy_svg_outputs_preserve_frozen_baseline() {
    let mut hashes = vec![];
    for values in [
        vec![Some(0.), Some(1.)],
        vec![Some(7.), Some(7.)],
        vec![Some(-f64::MAX), Some(f64::MAX)],
        vec![Some(f64::from_bits(f64::MAX.to_bits() - 1)), Some(f64::MAX)],
        vec![None, None],
    ] {
        let data = Dataset::new(vec![
            Column::numeric(ColumnId(1), "Captured X <&>", values.clone()).unwrap(),
            Column::numeric(ColumnId(2), "Captured Y <&>", values).unwrap(),
        ])
        .unwrap();
        for request in [
            AnalysisRequest::Histogram(HistogramRequest {
                variable: ColumnId(1),
                bins: HistogramBins::Count(2),
                filter: RowFilter::default(),
                frequency_weight: None,
            }),
            AnalysisRequest::Scatter(ScatterRequest {
                x: ColumnId(1),
                y: ColumnId(2),
                filter: RowFilter::default(),
                frequency_weight: None,
            }),
            AnalysisRequest::Boxplot(BoxplotRequest {
                outcome: ColumnId(1),
                group: None,
                filter: RowFilter::default(),
                frequency_weight: None,
            }),
        ] {
            let analysis =
                analyze_with_budget(&data, &request, &JobControl::default(), &[], 4096).unwrap();
            let run = AnalysisRun {
                id: RunId(51),
                analysis,
            };
            let mut bytes = vec![];
            match &run.analysis {
                CompletedAnalysis::Histogram(v) => crate::export::report_exploration::histogram(
                    &mut bytes,
                    &run,
                    v,
                    &JobControl::default(),
                )
                .unwrap(),
                CompletedAnalysis::Scatter(v) => crate::export::report_exploration::scatter(
                    &mut bytes,
                    &run,
                    v,
                    &JobControl::default(),
                )
                .unwrap(),
                CompletedAnalysis::Boxplot(v) => crate::export::report_exploration::boxplot(
                    &mut bytes,
                    &run,
                    v,
                    &JobControl::default(),
                )
                .unwrap(),
                _ => unreachable!(),
            }
            let svg = String::from_utf8(bytes).unwrap();
            if svg.contains("<svg ") {
                match &run.analysis {
                    CompletedAnalysis::Histogram(_) => assert!(
                        svg.contains("class=\"chart-bar\"")
                            || svg.contains("stroke=\"#86631F\" stroke-width=\"8\"")
                    ),
                    CompletedAnalysis::Scatter(_) => assert!(svg.contains("class=\"chart-point\"")),
                    CompletedAnalysis::Boxplot(_) => {
                        assert!(svg.contains("class=\"chart-box\""));
                        assert!(svg.contains("class=\"chart-median-edge\""));
                        assert!(svg.contains("class=\"chart-median\""));
                    }
                    _ => unreachable!(),
                }
            }

            hashes.push(format!(
                "{:x}",
                Sha256::digest(legacy_style(&svg).as_bytes())
            ));
        }
    }
    println!(
        "LEGACY_SVG_HASHES {}",
        serde_json::to_string(&hashes).unwrap()
    );
    if std::env::var_os("TESSERA_TASK8_CAPTURE_BASELINE").is_none() {
        let expected: Vec<String> =
            serde_json::from_str(include_str!("legacy-hashes.json")).unwrap();
        assert_eq!(hashes, expected);
    }
}

#[test]
fn approved_grid_style_keeps_exact_tick_positions() {
    let xs = crate::charts::ChartScale::new(0., 10.).unwrap();
    let ys = crate::charts::ChartScale::new(0., 10.).unwrap();
    let mut bytes = Vec::new();
    super::axes(&mut bytes, "X", "Y", &xs, &ys).unwrap();
    let svg = String::from_utf8(bytes).unwrap();
    for tick in xs.axis_ticks() {
        let (_, p) = tick.unwrap();
        if p > 0. && p < 1. {
            assert!(svg.contains(&format!(
                "<path class=\"chart-grid\" d=\"M{} 20 V280\"/>",
                super::left(&ys) + (super::right(&xs) - super::left(&ys)) * p
            )));
        }
    }
    for tick in ys.axis_ticks() {
        let (_, p) = tick.unwrap();
        if p > 0. && p < 1. {
            assert!(svg.contains(&format!(
                "<path class=\"chart-grid\" d=\"M{} {} H{}\"/>",
                super::left(&ys),
                280. - 260. * p,
                super::right(&xs)
            )));
        }
    }
    assert!(!legacy_style(&svg).contains("chart-grid"));
}
