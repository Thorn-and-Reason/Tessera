use super::*;

pub(super) fn show(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    run_id: RunId,
    r: &ComputedRegression,
    d: &RegressionDiagnostics,
) {
    ui.push_id((run_id, "residual"), |ui| {
        let title = theme::caption(format_args!(
            "Residual versus fitted values · {}",
            name(r, r.request.outcome)
        ));
        notice(
            ui,
            d.residual_points.len(),
            r.fitted_rows as u64,
            d.residual_reduced,
        );
        let rect = canvas(ui, state, &title, &d.fitted_range, &d.residual_range);
        let draw = (|| {
            let (xs, ys) = scales(&d.fitted_range, &d.residual_range)?;
            chart_drawing::axes(ui, rect, &xs, Some(&ys), "Fitted value", "Raw residual")?;
            for p in &d.residual_points {
                if let (Some(x), Some(y)) = (p.fitted.value, p.residual.value) {
                    let point = Pos2::new(
                        chart_drawing::x(rect, &xs, x)?,
                        chart_drawing::y(rect, &ys, y)?,
                    );
                    mark(ui, point);
                    chart_drawing::hover(ui, point, || residual_context(p));
                }
            }
            Ok(())
        })();
        chart_drawing::unavailable(ui, draw);
        ui.strong("Exact retained residual diagnostics");
        rows(ui, "residual-rows", d.residual_points.len(), |ui, i| {
            context(ui, residual_context(&d.residual_points[i]));
        });
    });
    ui.push_id((run_id, "qq"), |ui| {
        let title = theme::caption(format_args!(
            "Normal Q–Q plot of raw residuals · {}",
            name(r, r.request.outcome)
        ));
        notice(ui, d.qq_points.len(), r.frequency, d.qq_reduced);
        let rect = canvas(ui, state, &title, &d.theoretical_range, &d.residual_range);
        let draw = (|| {
            let (xs, ys) = scales(&d.theoretical_range, &d.residual_range)?;
            chart_drawing::axes(
                ui,
                rect,
                &xs,
                Some(&ys),
                "Theoretical normal quantile",
                "Raw residual",
            )?;
            for p in &d.qq_points {
                if let (Some(x), Some(y)) = (p.theoretical.value, p.residual.value) {
                    let point = Pos2::new(
                        chart_drawing::x(rect, &xs, x)?,
                        chart_drawing::y(rect, &ys, y)?,
                    );
                    mark(ui, point);
                    chart_drawing::hover(ui, point, || qq_context(p));
                }
            }
            Ok(())
        })();
        chart_drawing::unavailable(ui, draw);
        ui.strong("Exact retained Q–Q ranks");
        rows(ui, "qq-rows", d.qq_points.len(), |ui, i| {
            context(ui, qq_context(&d.qq_points[i]));
        });
    });
}
fn notice(ui: &mut egui::Ui, shown: usize, total: u64, reduced: bool) {
    ui.label(format!(
        "Showing {shown} of {total} retained observations{}",
        if reduced {
            " · Drawing reduced; summaries and bounds use the complete cohort"
        } else {
            ""
        }
    ));
    ui.small("Markers omit unavailable coordinates; independent values and reasons remain in the exact table.");
}
fn scales(
    x: &[Estimate; 2],
    y: &[Estimate; 2],
) -> crate::error::AppResult<(ChartScale, ChartScale)> {
    let bounds = |range: &[Estimate; 2]| {
        range[0].value.zip(range[1].value).ok_or_else(|| {
            crate::error::AppError::invalid("drawing", "Full-cohort axis bound is unavailable.")
        })
    };
    let (xl, xh) = bounds(x)?;
    let (yl, yh) = bounds(y)?;
    Ok((ChartScale::new(xl, xh)?, ChartScale::new(yl, yh)?))
}
fn canvas(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    title: &str,
    x: &[Estimate; 2],
    y: &[Estimate; 2],
) -> egui::Rect {
    let precise = |range: &[Estimate; 2]| {
        range[0]
            .value
            .zip(range[1].value)
            .and_then(|(lo, hi)| ChartScale::new(lo, hi).ok())
            .is_some_and(|s| s.precise_ticks())
    };
    let xp = precise(x);
    chart_drawing::canvas(
        ui,
        state,
        title,
        if precise(y) {
            184.
        } else if xp {
            84.
        } else {
            68.
        },
        if xp { 84. } else { 30. },
    )
}
fn mark(ui: &egui::Ui, point: Pos2) {
    ui.painter().circle_filled(point, 3., theme::accent(ui));
    ui.painter()
        .circle_stroke(point, 3., Stroke::new(1., theme::text(ui)));
}
fn residual_context(p: &RegressionResidualPoint) -> String {
    format!("Cohort ordinal: {} · Source row: {} · Frequency: {} · Fitted value: {} · Raw residual: {} · Leverage: {} · Internally studentised residual: {} · Cook’s distance: {}", p.cohort_ordinal + 1, p.row + 1, p.frequency, estimate(&p.fitted), estimate(&p.residual), estimate(&p.leverage), estimate(&p.studentised), estimate(&p.cooks_distance))
}
fn qq_context(p: &RegressionQqPoint) -> String {
    format!("Replicated rank: {} · Source row: {} · Source-row frequency: {} · Theoretical normal quantile: {} · Raw residual: {}", p.rank + 1, p.row + 1, p.frequency, estimate(&p.theoretical), estimate(&p.residual))
}
