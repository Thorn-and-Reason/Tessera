use super::*;

#[test]
fn finite_all_outlier_summary_can_have_two_unavailable_observed_whiskers() {
    let a = f64::from_bits(1);
    let mut valid = boxplot();
    valid.groups[0] = BoxplotGroup {
        value: None,
        value_label_index: None,
        rows: 2,
        n: 2,
        mean: Estimate {
            value: None,
            reason: Some("Mean was not captured.".into()),
        },
        minimum: e(-a),
        q1: e(-0.0),
        median: e(0.0),
        q3: e(0.0),
        maximum: e(a),
        whiskers: [unavailable(), unavailable()],
        outlier_rows: 2,
        outlier_n: 2,
    };
    valid.points = [-a, a]
        .into_iter()
        .enumerate()
        .map(|(row, value)| BoxplotPoint {
            group_index: 0,
            row,
            value,
            frequency: 1,
        })
        .collect();
    assert!(valid.validate().is_ok());
}

#[test]
fn histogram_extrema_require_nonempty_endpoint_bins() {
    let valid = histogram();
    assert!(valid.validate().is_ok());
    for (first, last) in [(0, 2), (2, 0)] {
        let mut invalid = valid.clone();
        invalid.bins[0].rows = first;
        invalid.bins[0].frequency = first as u64;
        invalid.bins[1].rows = last;
        invalid.bins[1].frequency = last as u64;
        assert!(
            invalid.validate().is_err(),
            "accepted endpoint counts {first}/{last}"
        );
    }
    let mut interior = valid;
    interior.request.bins = HistogramBins::Count(3);
    interior.range = [e(0.0), e(3.0)];
    interior.bins = vec![
        HistogramBin {
            left: 0.0,
            right: 1.0,
            rows: 1,
            frequency: 1,
        },
        HistogramBin {
            left: 1.0,
            right: 2.0,
            rows: 0,
            frequency: 0,
        },
        HistogramBin {
            left: 2.0,
            right: 3.0,
            rows: 1,
            frequency: 1,
        },
    ];
    assert!(interior.validate().is_ok());
}

// Five conceptual observations: [lo,lo,lo,hi,outlier], then optional reflection.
fn upper_outlier(lo: f64, hi: f64, outlier: f64, reflected: bool) -> ComputedBoxplot {
    let mut result = boxplot();
    result.source_rows = 5;
    result.groups[0] = BoxplotGroup {
        value: None,
        value_label_index: None,
        rows: 5,
        n: 5,
        mean: Estimate {
            value: None,
            reason: Some("Mean was not captured.".into()),
        },
        minimum: e(lo),
        q1: e(lo),
        median: e(lo),
        q3: e(hi),
        maximum: e(outlier),
        whiskers: [e(lo), e(hi)],
        outlier_rows: 1,
        outlier_n: 1,
    };
    result.points = vec![BoxplotPoint {
        group_index: 0,
        row: 4,
        value: outlier,
        frequency: 1,
    }];
    if reflected {
        let g = &mut result.groups[0];
        g.minimum = e(-outlier);
        g.q1 = e(-hi);
        g.median = e(-lo);
        g.q3 = e(-lo);
        g.maximum = e(-lo);
        g.whiskers = [e(-hi), e(-lo)];
        result.points[0].row = 0;
        result.points[0].value = -outlier;
    }
    result
}
fn suppress_outlier(mut result: ComputedBoxplot, reflected: bool) -> ComputedBoxplot {
    let g = &mut result.groups[0];
    g.whiskers[usize::from(!reflected)] = if reflected {
        g.minimum.clone()
    } else {
        g.maximum.clone()
    };
    g.outlier_rows = 0;
    g.outlier_n = 0;
    result.points.clear();
    result
}
#[test]
fn subnormal_box_fences_keep_literal_replication_rule() {
    let a = f64::from_bits(1);
    for reflected in [false, true] {
        let correct = upper_outlier(0.0, a, 3.0 * a, reflected);
        assert!(
            correct.validate().is_ok(),
            "refused exact subnormal outlier; reflected={reflected}"
        );
        assert!(
            suppress_outlier(correct, reflected).validate().is_err(),
            "accepted rounded subnormal fence; reflected={reflected}"
        );
    }
}
#[test]
fn normal_half_ulp_fences_do_not_round_an_outlier_into_an_inlier() {
    let one = 1.0_f64.to_bits();
    let lo = f64::from_bits(one + 1);
    let hi = f64::from_bits(one + 2);
    // The exact upper fence is 1 + 3.5 ULP; ordinary addition rounds it to 1 + 4 ULP.
    let outside = f64::from_bits(one + 4);
    for reflected in [false, true] {
        let correct = upper_outlier(lo, hi, outside, reflected);
        assert!(
            correct.validate().is_ok(),
            "refused normal half-ULP outlier; reflected={reflected}"
        );
        assert!(
            suppress_outlier(correct, reflected).validate().is_err(),
            "accepted rounded normal fence; reflected={reflected}"
        );
    }
}
#[test]
fn exact_fence_boundary_is_inclusive_at_normal_and_subnormal_scales() {
    for (lo, hi, boundary) in [(0.0, 2.0, 5.0), (0.0, f64::from_bits(2), f64::from_bits(5))] {
        for reflected in [false, true] {
            let wrong = upper_outlier(lo, hi, boundary, reflected);
            assert!(
                wrong.validate().is_err(),
                "classified exact fence boundary as outlier"
            );
            assert!(
                suppress_outlier(wrong, reflected).validate().is_ok(),
                "refused inclusive boundary"
            );
        }
    }
}
#[test]
fn extreme_finite_fences_do_not_overflow_and_zero_iqr_remains_exact() {
    let mut extreme = boxplot();
    let g = &mut extreme.groups[0];
    g.minimum = e(-f64::MAX);
    g.q1 = e(-f64::MAX / 2.0);
    g.median = e(0.0);
    g.q3 = e(f64::MAX / 2.0);
    g.maximum = e(f64::MAX);
    g.whiskers = [g.minimum.clone(), g.maximum.clone()];
    assert!(extreme.validate().is_ok());
    for reflected in [false, true] {
        assert!(upper_outlier(0.0, 0.0, f64::from_bits(1), reflected)
            .validate()
            .is_ok());
    }
}
