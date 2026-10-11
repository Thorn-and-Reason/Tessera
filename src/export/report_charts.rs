//! Concrete SVG primitives. Callers supply captured captions and full-cohort bounds.
use super::report::text;
use crate::{charts::ChartScale, error::AppResult, operation::RunId};
use std::io::Write;

pub(super) fn right(xs: &ChartScale) -> f64 {
    if xs.precise_ticks() {
        636.
    } else {
        660.
    }
}
pub(super) fn left(ys: &ChartScale) -> f64 {
    if ys.precise_ticks() {
        184.
    } else {
        80.
    }
}
pub(super) fn svg_start(
    w: &mut dyn Write,
    run_id: RunId,
    plot_id: &str,
    title: &str,
    desc: &str,
    xs: &ChartScale,
    ys: &ChartScale,
) -> AppResult<()> {
    svg_description_start(w, run_id, plot_id, title, desc)?;
    svg_plot_area(w, xs, ys)
}
pub(super) fn svg_description_start(
    w: &mut dyn Write,
    run_id: RunId,
    plot_id: &str,
    title: &str,
    desc: &str,
) -> AppResult<()> {
    if plot_id.is_empty() {
        write!(w, "<figure><svg viewBox=\"0 0 720 360\" role=\"img\" aria-labelledby=\"chart-{}\"><title id=\"chart-{}\">", run_id.0, run_id.0)?;
        text(w, title)?;
        w.write_all(b"</title><desc>")?;
    } else {
        write!(
            w,
            "<figure><svg viewBox=\"0 0 720 360\" role=\"img\" aria-labelledby=\"chart-{}-",
            run_id.0
        )?;
        text(w, plot_id)?;
        write!(w, "\" aria-describedby=\"chart-desc-{}-", run_id.0)?;
        text(w, plot_id)?;
        write!(w, "\"><title id=\"chart-{}-", run_id.0)?;
        text(w, plot_id)?;
        w.write_all(b"\">")?;
        text(w, title)?;
        write!(w, "</title><desc id=\"chart-desc-{}-", run_id.0)?;
        text(w, plot_id)?;
        w.write_all(b"\">")?;
    }
    text(w, desc)?;
    Ok(())
}
pub(super) fn svg_plot_area(w: &mut dyn Write, xs: &ChartScale, ys: &ChartScale) -> AppResult<()> {
    write!(
        w,
        "</desc><path d=\"M{} 20 V280 H{}\" fill=\"none\" stroke=\"#263449\"/>",
        left(ys),
        right(xs)
    )?;
    Ok(())
}
pub(super) fn axes(
    w: &mut dyn Write,
    xname: &str,
    yname: &str,
    xs: &ChartScale,
    ys: &ChartScale,
) -> AppResult<()> {
    let left = left(ys);
    let right = right(xs);
    for tick in xs.axis_ticks() {
        let (value, p) = tick?;
        if p > 0. && p < 1. {
            write!(
                w,
                "<path class=\"chart-grid\" d=\"M{} 20 V280\"/>",
                left + (right - left) * p
            )?;
        }
        write!(
            w,
            "<text x=\"{}\" y=\"302\" text-anchor=\"middle\" font-size=\"11\">{}</text>",
            left + (right - left) * p,
            xs.axis_label(value)
        )?;
    }
    for tick in ys.axis_ticks() {
        let (value, p) = tick?;
        if p > 0. && p < 1. {
            write!(
                w,
                "<path class=\"chart-grid\" d=\"M{left} {} H{right}\"/>",
                280. - 260. * p
            )?;
        }
        write!(
            w,
            "<text x=\"{}\" y=\"{}\" text-anchor=\"end\" font-size=\"11\">{}</text>",
            left - 6.,
            284. - 260. * p,
            ys.axis_label(value)
        )?;
    }
    w.write_all(b"<text x=\"370\" y=\"342\" text-anchor=\"middle\">")?;
    text(w, xname)?;
    w.write_all(
        b"</text><text transform=\"translate(18 150) rotate(-90)\" text-anchor=\"middle\">",
    )?;
    text(w, yname)?;
    w.write_all(b"</text>")?;
    Ok(())
}

#[cfg(test)]
mod legacy_tests;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captions_escape_and_scale_geometry_handles_extreme_and_constant_ranges() {
        for (lo, hi) in [
            (-f64::MAX, f64::MAX),
            (0., 0.),
            (f64::from_bits(f64::MAX.to_bits() - 1), f64::MAX),
        ] {
            let xs = ChartScale::new(lo, hi).unwrap();
            let ys = ChartScale::new(lo, hi).unwrap();
            let mut out = Vec::new();
            svg_start(
                &mut out,
                RunId(8),
                "residual",
                "Title <&>",
                "Description <&>",
                &xs,
                &ys,
            )
            .unwrap();
            axes(&mut out, "X <&>", "Y <&>", &xs, &ys).unwrap();
            let out = String::from_utf8(out).unwrap();
            for escaped in [
                "Title &lt;&amp;&gt;",
                "Description &lt;&amp;&gt;",
                "X &lt;&amp;&gt;",
                "Y &lt;&amp;&gt;",
            ] {
                assert!(out.contains(escaped));
            }
            assert!(!out.contains("NaN"));
            assert!(!out.contains("inf"));
            if xs.precise_ticks() {
                assert!(out.contains("M184 20 V280 H636"));
            }
            assert_eq!(xs.position(lo).unwrap(), if lo == hi { 0.5 } else { 0. });
            assert_eq!(xs.position(hi).unwrap(), if lo == hi { 0.5 } else { 1. });
        }
    }
}
