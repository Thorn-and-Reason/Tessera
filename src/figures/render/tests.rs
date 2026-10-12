use super::*;
fn geometry(marks: Vec<Mark>) -> (Layout, Data) {
    (
        Layout {
            left: 0.,
            right: 100.,
            top: 0.,
            bottom: 100.,
            xs: ChartScale::new(0., 1.).unwrap(),
            ys: ChartScale::new(0., 10.).unwrap(),
            xb: [0., 1.],
            yb: [0., 10.],
        },
        Data {
            title: String::new(),
            x: String::new(),
            y: String::new(),
            note: String::new(),
            visible_note: String::new(),
            bounds: [[0., 1.], [0., 10.]],
            marks,
            categories: vec!["Group".into()],
            categorical: true,
        },
    )
}
#[test]
fn whiskers_start_at_their_quartiles_and_inside_whiskers_have_separate_lanes() {
    let (l, d) = geometry(vec![Mark::Box(0, 2., 5., 8., [Some(1.), Some(9.)])]);
    let mut out = String::new();
    marks(
        &mut out,
        &l,
        &d,
        false,
        &FigureSettings::default(),
        &JobControl::default(),
    )
    .unwrap();
    assert!(
        out.contains("M50 80 V90"),
        "lower whisker must start at Q1: {out}"
    );
    assert!(
        out.contains("M50 20 V10"),
        "upper whisker must start at Q3: {out}"
    );
    let (l, d) = geometry(vec![Mark::Box(0, 2., 5., 8., [Some(3.), Some(7.)])]);
    out.clear();
    marks(
        &mut out,
        &l,
        &d,
        false,
        &FigureSettings::default(),
        &JobControl::default(),
    )
    .unwrap();
    assert!(out.contains("M42.5 80 V70"), "inside lower whisker: {out}");
    assert!(out.contains("M57.5 20 V30"), "inside upper whisker: {out}");
}
#[test]
fn point_edges_remain_visible_when_the_fill_is_translucent() {
    let (l, d) = geometry(vec![Mark::Point(0.5, 5.)]);
    let mut out = String::new();
    marks(
        &mut out,
        &l,
        &d,
        false,
        &FigureSettings::default(),
        &JobControl::default(),
    )
    .unwrap();
    assert!(out.contains("stroke-width=\"0.8\""));
    assert!(!out.contains("stroke=\"none\""));
}

#[test]
fn count_ticks_are_whole_distinct_values_inside_the_display_limits() {
    assert_eq!(count_ticks([0., 3.], 5).unwrap(), vec![0., 1., 2., 3.]);
    assert_eq!(count_ticks([0., 0.], 5).unwrap(), vec![0.]);
    let ticks = count_ticks([0.4, 4.3], 5).unwrap();
    assert!(ticks
        .iter()
        .all(|value| value.fract() == 0. && *value >= 0.4 && *value <= 4.3));
    assert!(ticks.windows(2).all(|v| v[0] < v[1]));
}
