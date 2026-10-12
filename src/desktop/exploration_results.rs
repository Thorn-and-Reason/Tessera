//! Captured drawings and virtual exact tables; metadata is never read from today's data.
use super::{
    button,
    chart_drawing::{axes, canvas, canvas_height, hover, paint_text, unavailable, x, y},
    procedure_forms::record,
    theme, DesktopState,
};
use crate::{
    analysis::*,
    charts::{CategoryCaption, ChartScale},
    data::Cell,
    project::AnalysisRun,
};
use eframe::egui::{self, Align2, Pos2, Rect, Sense, Stroke};

fn rows(ui: &mut egui::Ui, id: &str, len: usize, mut row: impl FnMut(&mut egui::Ui, usize)) {
    egui::ScrollArea::both()
        .id_salt(id)
        .max_height(270.)
        .show_rows(ui, 28., len, |ui, range| {
            for i in range {
                ui.push_id(i, |ui| {
                    if i % 2 == 1 {
                        ui.painter().rect_filled(
                            Rect::from_min_size(
                                ui.next_widget_position(),
                                egui::vec2(ui.available_width(), 28.),
                            ),
                            0.,
                            ui.visuals().faint_bg_color,
                        );
                    }
                    ui.horizontal(|ui| row(ui, i));
                });
            }
        });
}
fn detailed_rows(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    id: &str,
    len: usize,
    row: impl FnMut(&mut egui::Ui, usize),
) {
    let panel = egui::CollapsingHeader::new("Detailed values")
        .id_salt(id)
        .show(ui, |ui| rows(ui, id, len, row));
    record(state, "Detailed values", &panel.header_response);
}
fn caption(info: Option<&RunColumnInfo>, value: &Cell, label: Option<usize>) -> String {
    let raw = theme::caption(CategoryCaption(value));
    match label.and_then(|i| info.and_then(|c| c.value_labels.get(i))) {
        Some(l) => theme::caption(format_args!("{raw} · {}", l.label)),
        None => raw,
    }
}
fn context(ui: &mut egui::Ui, text: String) {
    read_only(ui, &text, &text);
}
fn read_only(ui: &mut egui::Ui, shown: &str, exact: &str) {
    let response = ui.add(
        egui::Label::new(shown)
            .sense(Sense::focusable_noninteractive())
            .truncate(),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, exact));
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.expand(2.),
            2.,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Outside,
        );
    }
    response.on_hover_text(exact);
}
fn estimate(e: &Estimate) -> String {
    e.value.map_or_else(
        || theme::caption(e.reason.as_deref().unwrap_or("Unavailable")),
        |v| v.to_string(),
    )
}
fn values(ui: &mut egui::Ui, prefix: &str, values: &[(&str, &Estimate)]) {
    egui::Grid::new(("exact-values", prefix))
        .striped(true)
        .show(ui, |ui| {
            for (label, e) in values {
                ui.label(*label);
                let exact = estimate(e);
                read_only(ui, &exact, &format!("{prefix} · {label}: {exact}"));
                ui.end_row();
            }
        });
}

pub(super) fn frequencies(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    _run: &AnalysisRun,
    v: &ComputedFrequencies,
) {
    ui.collapsing("Method", |ui| {
        ui.label(crate::export::report_exploration::FREQUENCY_METHOD);
    });
    ui.label("Complete category values: Export report…");
    let current = state
        .frequency_table_index()
        .min(v.tables.len().saturating_sub(1));
    let Some(cache) = state.frequency_metadata().cloned() else {
        if let Some(t) = v.tables.get(current) {
            if let Err(e) = state.set_frequency_search(t.column, String::new(), 0) {
                state.fail(e);
            }
        }
        ui.label(if state.error().is_some() && !state.active {
            "Captured table view unavailable · history and report remain available"
        } else {
            "Preparing captured table names…"
        });
        return;
    };
    let mut selected = current;
    let response = egui::ComboBox::from_id_salt("frequency-table")
        .selected_text(v.tables.get(current).map_or_else(
            || "No tables".into(),
            |_| theme::caption(&v.columns[cache.table_columns[current]].name),
        ))
        .show_ui(ui, |ui| {
            egui::ScrollArea::vertical().max_height(180.).show_rows(
                ui,
                28.,
                v.tables.len(),
                |ui, range| {
                    for i in range {
                        let response = theme::selectable_value(
                            ui,
                            &mut selected,
                            i,
                            theme::caption(&v.columns[cache.table_columns[i]].name),
                        );
                        record(state, &format!("Frequency table: {i}"), &response);
                    }
                },
            );
        })
        .response;
    record(state, "Frequency table", &response);
    let Some(t) = v.tables.get(selected) else {
        return;
    };
    let info = v.columns.get(cache.table_columns[selected]);
    let variable =
        theme::caption_limit(info.map_or("Unavailable variable", |c| c.name.as_str()), 64);
    ui.strong(&variable);
    ui.label(format!(
        "Valid rows: {} · valid frequency: {} · missing rows: {} · missing frequency: {}",
        t.valid_rows, t.valid_n, t.missing_rows, t.missing_n
    ));
    values(ui, &variable, &[("Missing percent", &t.missing_percent)]);
    let old = state.frequency_request();
    let mut query = old
        .filter(|r| r.column == t.column && selected == current)
        .map_or_else(String::new, |r| r.query.clone());
    let response = ui.add(
        egui::TextEdit::singleline(&mut query)
            .hint_text("Search categories")
            .char_limit(super::state::TEXT_LIMIT),
    );
    record(state, "Search categories", &response);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Search categories")
    });
    let offset = if response.changed() || selected != current {
        0
    } else {
        state.frequency_request().map_or(0, |r| r.offset)
    };
    if let Err(e) = state.set_frequency_search(t.column, query, offset) {
        state.fail(e);
    }
    let page = state.frequency_rows().cloned();
    if let Some(page) = page {
        ui.horizontal(|ui| {
            ui.label(format!(
                "{} matching categories · rows {}–{}",
                page.matched,
                if page.indices.is_empty() {
                    0
                } else {
                    page.request.offset + 1
                },
                page.request.offset + page.indices.len()
            ));
            if page.request.offset > 0 && button(ui, state, "Previous categories").clicked() {
                if let Err(e) = state.set_frequency_search(
                    t.column,
                    page.request.query.clone(),
                    page.request.offset.saturating_sub(256),
                ) {
                    state.fail(e);
                }
            }
            if let Some(next) = page.next_offset {
                if button(ui, state, "Next categories").clicked() {
                    if let Err(e) =
                        state.set_frequency_search(t.column, page.request.query.clone(), next)
                    {
                        state.fail(e);
                    }
                }
            }
        });
        rows(ui, "frequency-categories", page.indices.len(), |ui, i| {
            let r = &t.categories[page.indices[i]];
            let prefix = format!(
                "{variable} · Category {} · {}",
                page.indices[i] + 1,
                caption(info, &r.value, r.value_label_index)
            );
            context(
                ui,
                format!(
                    "{prefix} · Rows: {} · Frequency: {} · Percent: {} · Valid percent: {}{}",
                    r.rows,
                    r.frequency,
                    estimate(&r.percent),
                    estimate(&r.valid_percent),
                    r.cumulative_frequency.map_or_else(String::new, |n| format!(
                        " · Cumulative frequency: {n} · Cumulative valid percent: {}",
                        r.cumulative_percent
                            .as_ref()
                            .map_or_else(String::new, estimate)
                    ))
                ),
            );
        });
    } else {
        ui.label("Preparing category rows…");
    }
}
pub(super) fn histogram(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    run: &AnalysisRun,
    v: &ComputedHistogram,
) {
    let variable = theme::caption_limit(state.chart_name(run, v.request.variable), 64);
    ui.collapsing("Method", |ui| {
        ui.label(crate::export::report_exploration::HISTOGRAM_METHOD);
    });
    ui.label(format!("Requested bins: {} · actual bins: {} · valid rows: {} · frequency: {} · missing rows: {} · missing frequency: {}",crate::export::report_exploration::bin_setting(v.request.bins),v.bins.len(),v.valid_rows,v.n,v.missing_rows,v.missing_n));
    let precise = v.range[0]
        .value
        .zip(v.range[1].value)
        .and_then(|(lo, hi)| ChartScale::new(lo, hi).ok())
        .is_some_and(|s| s.precise_ticks());
    let rect = canvas(
        ui,
        state,
        &format!("Histogram · {variable}"),
        if precise { 84. } else { 68. },
        if precise { 84. } else { 30. },
    );
    let draw = (|| {
        let (Some(lo), Some(hi)) = (v.range[0].value, v.range[1].value) else {
            paint_text(
                ui,
                rect.center(),
                Align2::CENTER_CENTER,
                "No eligible observations",
            );
            return Ok(());
        };
        let xs = ChartScale::new(lo, hi)?;
        let ys = ChartScale::new(
            0.,
            v.bins.iter().map(|b| b.frequency).max().unwrap_or(0) as f64,
        )?;
        axes(ui, rect, &xs, Some(&ys), &variable, "Frequency")?;
        for (i, b) in v.bins.iter().enumerate() {
            let left = x(rect, &xs, b.left)?;
            let right = x(rect, &xs, b.right)?;
            let bar = Rect::from_min_max(
                Pos2::new(
                    if left == right { left - 4. } else { left },
                    y(rect, &ys, b.frequency as f64)?,
                ),
                Pos2::new(
                    if left == right { right + 4. } else { right },
                    rect.bottom(),
                ),
            );
            ui.painter().rect_filled(bar, 0., theme::accent(ui));
            ui.painter().rect_stroke(
                bar,
                0.,
                Stroke::new(1., theme::surface(ui)),
                egui::StrokeKind::Inside,
            );
            hover(ui, bar.center(), || {
                format!(
                    "{variable} · Bin {} · [{}, {}{} · Rows: {} · Frequency: {}",
                    i + 1,
                    b.left,
                    b.right,
                    if i + 1 == v.bins.len() { "]" } else { ")" },
                    b.rows,
                    b.frequency
                )
            });
        }
        Ok(())
    })();
    unavailable(ui, draw);
    values(
        ui,
        &variable,
        &[("Minimum", &v.range[0]), ("Maximum", &v.range[1])],
    );
    if let Some(s) = &v.bin_adjustment {
        ui.label(theme::caption(s));
    }
    detailed_rows(ui, state, "histogram-bins", v.bins.len(), |ui, i| {
        let b = &v.bins[i];
        context(
            ui,
            format!(
                "{variable} · Bin {} · [{}, {}{} · Rows: {} · Frequency: {}",
                i + 1,
                b.left,
                b.right,
                if i + 1 == v.bins.len() { "]" } else { ")" },
                b.rows,
                b.frequency
            ),
        );
    });
}
pub(super) fn scatter(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    run: &AnalysisRun,
    v: &ComputedScatter,
) {
    let xn = theme::caption_limit(state.chart_name(run, v.request.x), 64);
    let yn = theme::caption_limit(state.chart_name(run, v.request.y), 64);
    ui.collapsing("Method", |ui| {
        ui.label(crate::export::report_exploration::SCATTER_METHOD);
    });
    ui.label(format!("Showing {} of {} rows · frequency: {} · missing-pair rows: {} · missing-pair frequency: {}{}",v.points.len(),v.valid_rows,v.n,v.missing_rows,v.missing_n,if v.reduced{" · Drawing reduced"}else{""}));
    let precise = v.ranges[0][0]
        .value
        .zip(v.ranges[0][1].value)
        .and_then(|(lo, hi)| ChartScale::new(lo, hi).ok())
        .is_some_and(|s| s.precise_ticks());
    let left = v.ranges[1][0]
        .value
        .zip(v.ranges[1][1].value)
        .and_then(|(lo, hi)| ChartScale::new(lo, hi).ok())
        .map_or(if precise { 84. } else { 68. }, |s| {
            if s.precise_ticks() {
                184.
            } else if precise {
                84.
            } else {
                68.
            }
        });
    let rect = canvas(
        ui,
        state,
        &format!("Scatterplot · {xn} / {yn}"),
        left,
        if precise { 84. } else { 30. },
    );
    let draw = (|| {
        let (Some(xlo), Some(xhi), Some(ylo), Some(yhi)) = (
            v.ranges[0][0].value,
            v.ranges[0][1].value,
            v.ranges[1][0].value,
            v.ranges[1][1].value,
        ) else {
            paint_text(
                ui,
                rect.center(),
                Align2::CENTER_CENTER,
                "No eligible observations",
            );
            return Ok(());
        };
        let xs = ChartScale::new(xlo, xhi)?;
        let ys = ChartScale::new(ylo, yhi)?;
        axes(ui, rect, &xs, Some(&ys), &xn, &yn)?;
        for p in &v.points {
            let point = Pos2::new(x(rect, &xs, p.x)?, y(rect, &ys, p.y)?);
            ui.painter().circle_filled(point, 3., theme::accent(ui));
            ui.painter()
                .circle_stroke(point, 3., Stroke::new(1., theme::text(ui)));
            hover(ui, point, || {
                format!(
                    "{xn} / {yn} · Source row: {} · X: {} · Y: {} · Frequency: {}",
                    p.row + 1,
                    p.x,
                    p.y,
                    p.frequency
                )
            });
        }
        Ok(())
    })();
    unavailable(ui, draw);
    values(
        ui,
        &format!("{xn} / {yn}"),
        &[
            ("X minimum", &v.ranges[0][0]),
            ("X maximum", &v.ranges[0][1]),
            ("Y minimum", &v.ranges[1][0]),
            ("Y maximum", &v.ranges[1][1]),
        ],
    );
    detailed_rows(ui, state, "scatter-points", v.points.len(), |ui, i| {
        let p = &v.points[i];
        context(
            ui,
            format!(
                "{xn} / {yn} · Source row: {} · X: {} · Y: {} · Frequency: {}",
                p.row + 1,
                p.x,
                p.y,
                p.frequency
            ),
        );
    });
}
pub(super) fn boxplot(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    run: &AnalysisRun,
    v: &ComputedBoxplot,
) {
    let outcome = theme::caption_limit(state.chart_name(run, v.request.outcome), 64);
    ui.collapsing("Method", |ui| {
        ui.label(crate::export::report_exploration::BOXPLOT_METHOD);
    });
    ui.label(format!("Missing group rows: {} · missing group frequency: {} · missing outcome rows: {} · missing outcome frequency: {}",v.missing_group_rows,v.missing_group_n,v.missing_outcome_rows,v.missing_outcome_n));
    ui.label(format!(
        "Showing {} retained outlier rows{}",
        v.points.len(),
        if v.reduced { " · Drawing reduced" } else { "" }
    ));
    if let Some(reason) = &v.empty_reason {
        ui.label(theme::caption(reason));
    }
    let info = v.request.group.and_then(|id| state.chart_column(run, id));
    let bounds = v
        .groups
        .iter()
        .filter_map(|g| g.minimum.value)
        .reduce(f64::min)
        .zip(
            v.groups
                .iter()
                .filter_map(|g| g.maximum.value)
                .reduce(f64::max),
        );
    let precise = bounds
        .and_then(|(lo, hi)| ChartScale::new(lo, hi).ok())
        .is_some_and(|s| s.precise_ticks());
    ui.small("All groups use the same outcome scale.");
    egui::ScrollArea::vertical().id_salt("boxplot-panels").max_height(464.).show_rows(ui,232.,v.groups.len(),|ui,range| {
        for i in range {let g=&v.groups[i];ui.push_id(i,|ui| {
            let group=g.value.as_ref().map_or_else(||"All eligible outcomes".into(),|value|caption(info,value,g.value_label_index));
            let prefix=format!("{outcome} · Group {} · {} · {group}",
                        i+1,info.map_or("Group",|c|c.name.as_str()).chars().take(64).collect::<String>());
            let rect=canvas_height(ui,state,&format!("Boxplot · {prefix}"),if precise{84.}else{68.},if precise{84.}else{30.}, 160.);
            let start=v.points.partition_point(|p|p.group_index<i);let end=v.points.partition_point(|p|p.group_index<=i);
            let draw=(|| {
                let Some((lo,hi))=bounds else {paint_text(ui,rect.center(),Align2::CENTER_CENTER,"No eligible observations");return Ok(());};
                let scale=ChartScale::new(lo,hi)?;axes(ui,rect,&scale,None,&outcome,"")?;
                if let(Some(q1),Some(q3),Some(median))=(g.q1.value,g.q3.value,g.median.value) {
                    let mid=rect.center().y;let left=x(rect,&scale,q1)?;let right=x(rect,&scale,q3)?;
                    let boxrect=Rect::from_min_max(Pos2::new(left,mid-18.),Pos2::new(right,mid+18.));
                    ui.painter().rect_filled(boxrect,0.,super::output_style::blue(ui));ui.painter().rect_stroke(boxrect,0.,Stroke::new(1.5,theme::text(ui)),egui::StrokeKind::Middle);
                    let median=x(rect,&scale,median)?;
                    ui.painter().line_segment([Pos2::new(median,mid-18.),Pos2::new(median,mid+18.)],Stroke::new(5.,theme::text(ui)));
                    ui.painter().line_segment([Pos2::new(median,mid-18.),Pos2::new(median,mid+18.)],Stroke::new(3.,theme::GOLD));
                    // Observed whiskers may lie inside interpolated quartiles. Separate lanes retain both exact positions.
                    for(k,(whisker,quartile))in g.whiskers.iter().zip([q1,q3]).enumerate() {if let Some(value)=whisker.value {
                        let wx=x(rect,&scale,value)?;let qx=x(rect,&scale,quartile)?;let lane=mid+if k==0{-30.}else{30.};
                        ui.painter().line_segment([Pos2::new(qx,lane),Pos2::new(wx,lane)],Stroke::new(1.5,theme::text(ui)));
                        ui.painter().line_segment([Pos2::new(qx,mid),Pos2::new(qx,lane)],Stroke::new(1.,theme::text(ui)));
                        ui.painter().line_segment([Pos2::new(wx,lane-7.),Pos2::new(wx,lane+7.)],Stroke::new(2.,theme::text(ui)));
                    }}
                }
                for p in &v.points[start..end] {let point=Pos2::new(x(rect,&scale,p.value)?,rect.center().y);ui.painter().circle_stroke(point,4.,Stroke::new(1.5,theme::text(ui)));hover(ui,point,||format!("{prefix} · Source row: {} · Value: {} · Frequency: {}",p.row+1,p.value,p.frequency));}Ok(())
            })();unavailable(ui,draw);
            context(ui,format!("{prefix} · Rows: {} · Frequency: {} · Outlier rows: {} · Outlier frequency: {}",g.rows,g.n,g.outlier_rows,g.outlier_n));

        });}
    });
    for (index, metric) in ["Lower observed whisker", "Upper observed whisker"]
        .into_iter()
        .enumerate()
    {
        if let Some(reason) = v
            .groups
            .iter()
            .find_map(|g| g.whiskers[index].reason.as_deref())
        {
            ui.label(theme::caption(format_args!(
                "{metric}: {reason}. See Detailed values for affected groups."
            )));
        }
    }
    let panel = egui::CollapsingHeader::new("Detailed values").id_salt("boxplot-details").show(ui, |ui| {
        ui.strong("Group summaries");
        egui::ScrollArea::vertical().id_salt("boxplot-summary-values").max_height(340.).show_rows(ui, 220., v.groups.len(), |ui, range| {
            for i in range { let g = &v.groups[i]; ui.push_id(i, |ui| {
                let group = g.value.as_ref().map_or_else(|| "All eligible outcomes".into(), |value| caption(info, value, g.value_label_index));
                let prefix = format!("{outcome} · Group {} · {} · {group}", i+1, info.map_or("Group", |c| c.name.as_str()).chars().take(64).collect::<String>());
                ui.add(egui::Label::new(egui::RichText::new(theme::caption(&prefix)).strong()).truncate());
                values(ui, &prefix, &[("Minimum", &g.minimum), ("Q1", &g.q1), ("Median", &g.median), ("Q3", &g.q3), ("Maximum", &g.maximum), ("Lower observed whisker", &g.whiskers[0]), ("Upper observed whisker", &g.whiskers[1])]);
            }); }
        });
        ui.strong("Retained outlier observations");
        rows(ui, "boxplot-outliers", v.points.len(), |ui, i| {
            let p = &v.points[i];
            let g = &v.groups[p.group_index];
            let group = g.value.as_ref().map_or_else(|| "All eligible outcomes".into(), |value| caption(info, value, g.value_label_index));
            context(ui, format!("{outcome} · Group {}: {group} · Source row: {} · Value: {} · Frequency: {}", p.group_index+1, p.row+1, p.value, p.frequency));
        });
    });
    record(state, "Detailed values", &panel.header_response);
}

pub(super) fn settings(ui: &mut egui::Ui, state: &DesktopState, run: &AnalysisRun) {
    let request = run.analysis.request();
    ui.label(request.frequency_weight().map_or_else(
        || "Unweighted observations".into(),
        |id| {
            format!(
                "Frequency weight: variable {} · repeated observations",
                id.0
            )
        },
    ));
    let predicates = &request.filter().predicates;
    rows(ui, "captured-filters", predicates.len(), |ui, i| {
        ui.label(theme::caption(format_args!("{:?}", predicates[i])));
    });
    let d = run.analysis.diagnostics();
    ui.label(format!(
        "Excluded rows: {} filtered; {} missing weights; {} zero weights; {} incomplete cases",
        d.filtered_rows, d.missing_weight_rows, d.zero_weight_rows, d.complete_case_rows
    ));
    let columns = run.analysis.columns();
    egui::ScrollArea::vertical()
        .id_salt("captured-definitions")
        .max_height(360.)
        .show_rows(ui, 190., columns.len(), |ui, range| {
            for i in range {
                let c = &columns[i];
                ui.push_id(i, |ui| {
                    ui.label(theme::caption(format_args!(
                        "Variable {} · {} · {}",
                        c.id.0, c.name, c.description
                    )));
                    ui.label(theme::caption(format_args!(
                        "Missing definitions: {:?}",
                        c.missing_rules
                    )));
                    egui::ScrollArea::both()
                        .id_salt("captured-value-labels")
                        .max_height(100.)
                        .show_rows(ui, 24., c.value_labels.len(), |ui, range| {
                            for j in range {
                                let label = &c.value_labels[j];
                                ui.label(theme::caption(format_args!(
                                    "{}: {}",
                                    CategoryCaption(&label.value),
                                    label.label
                                )));
                            }
                        });
                });
            }
        });
    rows(
        ui,
        "captured-provenance",
        state.project().provenance().len(),
        |ui, i| {
            ui.label(theme::caption(&state.project().provenance()[i]));
        },
    );
}
