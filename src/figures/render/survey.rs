//! Full captured cohorts supply item distributions and group means.
use super::{block, escape, invalid, measure, parse, wrap};
use crate::{
    analysis::{CompletedAnalysis, FrequencyTable},
    charts::{CategoryCaption, ChartScale},
    data::{Cell, ColumnId},
    error::AppResult,
    figures::{FigureKey, FigureKind, FigurePalette, FigureSettings},
    operation::JobControl,
    project::AnalysisRun,
};
use std::fmt::Write;
mod ink;
struct Row {
    label: String,
    heading: Option<String>,
    values: Vec<u64>,
    mean: Option<f64>,
    rows: usize,
    n: u64,
    missing_rows: usize,
    missing_n: u64,
}
fn variable(run: &AnalysisRun, id: ColumnId) -> String {
    run.analysis
        .columns()
        .iter()
        .find(|c| c.id == id)
        .map_or_else(
            || format!("Variable {}", id.0),
            |c| {
                if c.description.is_empty() {
                    c.name.clone()
                } else {
                    c.description.clone()
                }
            },
        )
}
fn group(run: &AnalysisRun, id: ColumnId, value: &Cell, index: Option<usize>) -> String {
    index
        .and_then(|i| {
            run.analysis
                .columns()
                .iter()
                .find(|c| c.id == id)
                .and_then(|c| c.value_labels.get(i))
        })
        .map_or_else(
            || match value {
                Cell::Text(text) => text.clone(),
                Cell::Missing => "Group not recorded".into(),
                _ => CategoryCaption(value).to_string(),
            },
            |l| l.label.clone(),
        )
}
fn item(label: String, table: &FrequencyTable, points: u8, c: &JobControl) -> AppResult<Row> {
    let mut values = vec![0u64; points as usize];
    for (i, category) in table.categories.iter().enumerate() {
        c.checkpoint(i, Some(table.categories.len()))?;
        let Cell::Number(value) = category.value else {
            return Err(invalid(
                "Likert charts need confirmed numeric response codes. Text categories cannot be treated as agreement.",
            ));
        };
        if !value.is_finite() || value.fract() != 0. || value < 1. || value > points as f64 {
            return Err(invalid(
                "A captured response falls outside the confirmed ordered scale. Check the coding and rerun frequencies; no responses were omitted.",
            ));
        }
        values[value as usize - 1] = values[value as usize - 1]
            .checked_add(category.frequency)
            .ok_or_else(|| invalid("The captured response counts exceed the supported range."))?;
    }
    if values.iter().try_fold(0u64, |a, b| a.checked_add(*b)) != Some(table.valid_n) {
        return Err(invalid(
            "The captured category counts do not match the valid frequency.",
        ));
    }
    Ok(Row {
        label,
        heading: None,
        values,
        mean: None,
        rows: table.valid_rows,
        n: table.valid_n,
        missing_rows: table.missing_rows,
        missing_n: table.missing_n,
    })
}
fn scale_labels(points: u8) -> Vec<&'static str> {
    match points {
        3 => vec!["Disagree", "Neutral", "Agree"],
        7 => vec![
            "Strongly disagree",
            "Disagree",
            "Somewhat disagree",
            "Neutral",
            "Somewhat agree",
            "Agree",
            "Strongly agree",
        ],
        _ => vec![
            "Strongly disagree",
            "Disagree",
            "Neutral",
            "Agree",
            "Strongly agree",
        ],
    }
}
fn colours(points: usize, palette: FigurePalette) -> Vec<&'static str> {
    let all = match palette {
        FigurePalette::Monochrome => [
            "#252525", "#525252", "#737373", "#E0E0E0", "#969696", "#636363", "#303030",
        ],
        _ => [
            "#8C3E18", "#A85422", "#E0A66E", "#DFE4E6", "#8BBAC9", "#397A96", "#174F6A",
        ],
    };
    match points {
        3 => vec![all[0], all[3], all[6]],
        5 => vec![all[0], all[1], all[3], all[5], all[6]],
        _ => all.to_vec(),
    }
}
fn variable_bytes(run: &AnalysisRun, id: ColumnId) -> usize {
    run.analysis
        .columns()
        .iter()
        .find(|c| c.id == id)
        .map_or(32, |c| {
            if c.description.is_empty() {
                c.name.len()
            } else {
                c.description.len()
            }
        })
}
fn group_bytes(
    run: &AnalysisRun,
    id: Option<ColumnId>,
    value: Option<&Cell>,
    index: Option<usize>,
) -> usize {
    let label = id
        .and_then(|id| run.analysis.columns().iter().find(|c| c.id == id))
        .and_then(|col| index.and_then(|i| col.value_labels.get(i)));
    if let Some(label) = label {
        return label.label.len();
    }
    match value {
        Some(Cell::Text(text)) => text.len(),
        Some(Cell::Number(value)) => value.to_string().len(),
        _ => 32,
    }
}
fn preflight(
    run: &AnalysisRun,
    key: &FigureKey,
    s: &FigureSettings,
    c: &JobControl,
) -> AppResult<()> {
    let mut bytes = [&s.title, &s.subtitle, &s.caption, &s.x_label, &s.y_label]
        .iter()
        .map(|v| v.len())
        .sum::<usize>();
    let count = match (&run.analysis, key.kind) {
        (CompletedAnalysis::Frequencies(v), FigureKind::Likert) => {
            let count = v
                .tables
                .len()
                .checked_mul(v.groups.len().max(1))
                .unwrap_or(usize::MAX);
            if count > 1024 {
                return Err(invalid(
                    "The figure exceeds the 1024-row drawing budget; no rows were omitted.",
                ));
            }
            for (i, t) in v.tables.iter().enumerate() {
                c.checkpoint(i, Some(v.tables.len()))?;
                bytes = bytes.saturating_add(
                    variable_bytes(run, t.column).saturating_mul(v.groups.len().max(1)),
                );
            }
            for (i, g) in v.groups.iter().enumerate() {
                c.checkpoint(i, Some(v.groups.len()))?;
                bytes = bytes.saturating_add(
                    group_bytes(run, v.request.group, Some(&g.value), g.value_label_index)
                        .saturating_add(4)
                        .saturating_mul(v.tables.len()),
                );
            }
            count
        }
        (CompletedAnalysis::Boxplot(v), FigureKind::GroupMeans) => {
            if v.groups.len() > 1024 {
                return Err(invalid(
                    "The figure exceeds the 1024-row drawing budget; no groups were omitted.",
                ));
            }
            for (i, g) in v.groups.iter().enumerate() {
                c.checkpoint(i, Some(v.groups.len()))?;
                bytes = bytes.saturating_add(group_bytes(
                    run,
                    v.request.group,
                    g.value.as_ref(),
                    g.value_label_index,
                ));
            }
            v.groups.len()
        }
        _ => 0,
    };
    if count > 1024 || bytes > 16384 {
        return Err(invalid(
            "The complete labels exceed the figure's text budget. Select fewer items or shorten labels; no rows were omitted.",
        ));
    }
    Ok(())
}
pub(super) fn svg(
    run: &AnalysisRun,
    key: &FigureKey,
    s: &FigureSettings,
    stale: bool,
    c: &JobControl,
) -> AppResult<String> {
    preflight(run, key, s, c)?;
    let likert = matches!(key.kind, FigureKind::Likert);
    for (i, column) in run.analysis.columns().iter().enumerate() {
        c.checkpoint(i, None)?;
        if column.name.len() > 4096
            || column.description.len() > 4096
            || column.value_labels.iter().any(|l| l.label.len() > 4096)
        {
            return Err(invalid(
                "The captured labels are too long for a report figure; shorten them and rerun the analysis.",
            ));
        }
    }
    let mut rows = Vec::new();
    let mut bounds = [0., 1.];
    let default_title;
    let default_axis;
    let weight;
    let diagnostics;
    match (&run.analysis, key.kind) {
        (CompletedAnalysis::Frequencies(v), FigureKind::Likert) => {
            if v.tables.len().saturating_mul(v.groups.len().max(1)) > 1024
                || v.groups
                    .iter()
                    .any(|g| matches!(&g.value, Cell::Text(text) if text.len() > 4096))
            {
                return Err(invalid(
                    "The complete item and group labels exceed the drawing budget. Select fewer items or groups; no rows were omitted.",
                ));
            }
            let points = s.likert_points.ok_or_else(|| invalid("Set up the ordered response scale first: confirm three, five or seven numeric codes starting at 1, with the middle code neutral."))?;
            default_title = "Ordered response distributions".to_owned();
            default_axis =
                "Percent of valid answers: disagreement (left), agreement (right)".to_owned();
            weight = v.request.frequency_weight;
            diagnostics = &v.diagnostics;
            for (i, table) in v.tables.iter().enumerate() {
                c.checkpoint(i, Some(v.tables.len()))?;
                if v.groups.is_empty() {
                    rows.push(item(variable(run, table.column), table, points, c)?);
                } else {
                    let id = v
                        .request
                        .group
                        .ok_or_else(|| invalid("The captured groups have no grouping variable."))?;
                    for (group_index, g) in v.groups.iter().enumerate() {
                        c.check_cancelled()?;
                        let t = g
                            .tables
                            .iter()
                            .find(|t| t.column == table.column)
                            .ok_or_else(|| {
                                invalid(
                                    "A captured group is missing an item table; rerun frequencies.",
                                )
                            })?;
                        let mut row =
                            item(group(run, id, &g.value, g.value_label_index), t, points, c)?;
                        if group_index == 0 {
                            row.heading = Some(variable(run, table.column));
                        }
                        rows.push(row);
                    }
                }
            }
        }
        (CompletedAnalysis::Boxplot(v), FigureKind::GroupMeans) => {
            if v.groups.len() > 1024
                || v.groups
                    .iter()
                    .any(|g| matches!(&g.value, Some(Cell::Text(text)) if text.len() > 4096))
            {
                return Err(invalid(
                    "The complete group labels exceed the drawing budget; no groups were omitted.",
                ));
            }
            default_title = format!("{} · group means", variable(run, v.request.outcome));
            default_axis = format!("Mean {}", variable(run, v.request.outcome));
            weight = v.request.frequency_weight;
            diagnostics = &v.diagnostics;
            let lo = v
                .groups
                .iter()
                .filter_map(|g| g.minimum.value)
                .reduce(f64::min);
            let hi = v
                .groups
                .iter()
                .filter_map(|g| g.maximum.value)
                .reduce(f64::max);
            if let (Some(lo), Some(hi)) = (lo, hi) {
                bounds = [lo, hi];
            }
            for (i, g) in v.groups.iter().enumerate() {
                c.checkpoint(i, Some(v.groups.len()))?;
                if g.n > 0 && g.mean.value.is_none() {
                    return Err(invalid(
                        "This saved result has no captured group mean. Rerun the grouped summary before drawing means.",
                    ));
                }
                rows.push(Row {
                    label: match (v.request.group, &g.value) {
                        (Some(id), Some(value)) => group(run, id, value, g.value_label_index),
                        _ => "All eligible outcomes".into(),
                    },
                    heading: None,
                    values: vec![],
                    mean: g.mean.value,
                    rows: g.rows,
                    n: g.n,
                    missing_rows: 0,
                    missing_n: 0,
                });
            }
        }
        _ => {
            return Err(invalid(
                "The figure does not belong to this captured analysis.",
            ));
        }
    }
    if rows.is_empty() {
        return Err(invalid("This capture has no item or group rows to draw."));
    }
    if rows.len() > 1024 || rows.iter().map(|r| r.label.len()).sum::<usize>() > 16384 {
        return Err(invalid(
            "The complete labels exceed the figure's text budget. Select fewer items or shorten labels; no rows were omitted.",
        ));
    }
    let w = s.width_mm * 72. / 25.4;
    let h = s.height_mm * 72. / 25.4;
    let fs = s.font_size;
    let small = fs * 0.8;
    let pad = fs;
    let title = if s.title.is_empty() {
        default_title
    } else {
        s.title.clone()
    };
    let axis = if s.x_label.is_empty() {
        default_axis
    } else {
        s.x_label.clone()
    };
    let mut note = if likert {
        "Each row uses its own valid frequency; missing answers excluded. n = source rows."
            .to_owned()
    } else {
        "Each point is the captured group mean; n = source rows. Numeric scores assume equally spaced values; these summaries are not performance rankings.".to_owned()
    };
    if let CompletedAnalysis::Boxplot(v) = &run.analysis {
        write!(
            note,
            " Missing group: {} rows (f={}); missing outcome: {} rows (f={}).",
            v.missing_group_rows, v.missing_group_n, v.missing_outcome_rows, v.missing_outcome_n
        )
        .unwrap();
    }
    if let Some(id) = weight {
        write!(
            note,
            " Frequency weights: {} (literal replication).",
            variable(run, id)
        )
        .unwrap();
    }
    write!(
        note,
        " Excluded rows: {} filtered, {} missing weight, {} zero weight.",
        diagnostics.filtered_rows, diagnostics.missing_weight_rows, diagnostics.zero_weight_rows
    )
    .unwrap();
    if stale {
        note.push_str(" Stale result: earlier data revision.");
    }
    if !s.caption.is_empty() {
        note = format!("{} {}", s.caption, note);
    }
    let titles = wrap(&title, w - pad * 2., fs * 1.25, c)?;
    let subtitles = wrap(&s.subtitle, w - pad * 2., small, c)?;
    let notes = wrap(&note, w - pad * 2., small, c)?;
    let axis_lines = wrap(&axis, w * 0.6, small, c)?;
    let left = w * 0.37;
    let mean_bounds = s.x_bounds.unwrap_or(bounds);
    let precise_means = !likert && ChartScale::new(mean_bounds[0], mean_bounds[1])?.precise_ticks();
    let mean_label_space = rows
        .iter()
        .filter_map(|r| r.mean)
        .map(|v| measure(&super::tick_label(v, s.decimals, precise_means), small) + small * 2.)
        .fold(pad * 3.5, f64::max);
    let right = if likert {
        w * 0.77
    } else {
        w - mean_label_space
    };
    let mut labels = Vec::new();
    let mut exact = String::new();
    let mut headings = Vec::new();
    let mut context = "";
    let category_labels = scale_labels(s.likert_points.unwrap_or(5));
    for (i, row) in rows.iter().enumerate() {
        c.checkpoint(i, Some(rows.len()))?;
        let mut suffix = format!("n={}", row.rows);
        if weight.is_some() {
            write!(suffix, "; f={}", row.n).unwrap();
        }
        if likert {
            write!(suffix, "; missing={}", row.missing_rows).unwrap();
        }
        let heading = row.heading.as_deref().unwrap_or("");
        if !heading.is_empty() {
            context = heading;
        }
        headings.push(wrap(heading, right - pad, small, c)?);
        labels.push(wrap(
            &format!("{}\n{}", row.label, suffix),
            left - pad * 1.8,
            small,
            c,
        )?);
        write!(
            exact,
            "{}: valid rows {}; valid frequency {}. ",
            if context.is_empty() {
                row.label.clone()
            } else {
                format!("{context} · {}", row.label)
            },
            row.rows,
            row.n
        )
        .unwrap();
        if likert {
            write!(
                exact,
                "missing rows {}; missing frequency {}. ",
                row.missing_rows, row.missing_n
            )
            .unwrap();
        }
        for (label, count) in category_labels.iter().zip(&row.values) {
            let percent = if row.n == 0 {
                0.
            } else {
                *count as f64 / row.n as f64 * 100.
            };
            write!(exact, "{label}: {count} ({percent:.8}%); ").unwrap();
        }
        if let Some(mean) = row.mean {
            write!(exact, "Mean: {mean}. ").unwrap();
        }
        if row.n == 0 {
            exact.push_str("No recorded answers. ");
        }
    }
    let colours = colours(category_labels.len(), s.palette);
    let top = pad + (titles.len() as f64 * fs * 1.25 + subtitles.len() as f64 * small) * 1.3 + pad;
    let bottom = h
        - pad
        - notes.len() as f64 * small * 1.3
        - axis_lines.len() as f64 * small * 1.3
        - small * 3.;
    let heading_height = |lines: &[String]| {
        if lines.is_empty() {
            0.
        } else {
            lines.len() as f64 * small * 1.3 + small * 1.4
        }
    };
    let row_height = (bottom - top - headings.iter().map(|l| heading_height(l)).sum::<f64>())
        / rows.len() as f64;
    if right - left < fs * 8.
        || row_height < fs * 2.
        || labels
            .iter()
            .any(|l| l.len() as f64 * small * 1.3 + small > row_height)
    {
        return Err(invalid(
            "The complete item labels do not fit. Increase figure height or width, shorten labels or reduce font size; no rows were omitted.",
        ));
    }
    if likert && (s.x_bounds.is_some() || s.y_bounds.is_some()) {
        return Err(invalid(
            "Combined Likert charts use the shared −100% to 100% axis. Clear custom limits so every response remains visible.",
        ));
    }
    if !likert && s.y_bounds.is_some() {
        return Err(invalid(
            "Group mean rows require the complete category axis. Clear Y axis limits.",
        ));
    }
    let bounds = if likert {
        [-100., 100.]
    } else {
        s.x_bounds.unwrap_or(bounds)
    };
    if !likert
        && rows
            .iter()
            .filter_map(|r| r.mean)
            .any(|v| v < bounds[0] || v > bounds[1])
    {
        return Err(invalid(
            "The axis limits hide a group mean. Expand the limits so every captured mean remains visible.",
        ));
    }
    let scale = ChartScale::new(bounds[0], bounds[1])?;
    let inset = if likert { 0. } else { s.marker_size + 1. };
    if right - left - inset * 2. < fs * 8. {
        return Err(invalid(
            "Increase figure width or reduce marker size to fit the full means and their boundary markers.",
        ));
    }
    let x = |v: f64| -> AppResult<f64> {
        Ok(left
            + inset
            + scale.position(if likert { v.clamp(-100., 100.) } else { v })?
                * (right - left - inset * 2.))
    };
    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">"
    );
    if !s.transparent {
        write!(out, "<rect width=\"{w}\" height=\"{h}\" fill=\"white\"/>").unwrap();
    }
    out.push_str("<g font-family=\"Noto Sans\" fill=\"#202B33\">");
    block(&mut out, &titles, pad, pad + fs * 1.2, fs * 1.25, "start");
    let sub_y = pad + fs * 1.2 + titles.len() as f64 * fs * 1.25 * 1.3;
    block(&mut out, &subtitles, pad, sub_y, small, "start");
    if likert {
        let legend_x = w * 0.81;
        let mut y = top + small;
        for (i, (label, colour)) in category_labels.iter().zip(&colours).enumerate() {
            let lines = wrap(
                &format!("{} · {label}", i + 1),
                w - legend_x - small * 1.5 - pad,
                small,
                c,
            )?;
            if y + lines.len() as f64 * small * 1.3 > bottom {
                return Err(invalid(
                    "The full scale legend does not fit. Increase figure height or reduce the font size.",
                ));
            }
            write!(out, "<rect x=\"{legend_x}\" y=\"{}\" width=\"{small}\" height=\"{small}\" fill=\"{colour}\"/>", y - small * 0.85).unwrap();
            block(&mut out, &lines, legend_x + small * 1.5, y, small, "start");
            y += lines.len() as f64 * small * 1.3 + small * 0.7;
        }
    }
    let ticks = if likert {
        vec![-100., -50., 0., 50., 100.]
    } else {
        super::tick_values(bounds, if scale.precise_ticks() { 2 } else { s.ticks })?
    };
    let tick_width = ticks
        .iter()
        .map(|v| {
            measure(
                &super::tick_label(*v, s.decimals, scale.precise_ticks()),
                small,
            )
        })
        .fold(0., f64::max);
    if !likert
        && ticks.len() > 1
        && tick_width > (right - left - inset * 2.) / (ticks.len() - 1) as f64
    {
        return Err(invalid(
            "The tick labels overlap. Increase figure width, reduce ticks or reduce font size.",
        ));
    }
    for tick in ticks {
        let px = x(tick)?;
        if s.grid {
            write!(
                out,
                "<path d=\"M{px} {top} V{bottom}\" stroke=\"#DDE5E9\" stroke-width=\"0.6\"/>"
            )
            .unwrap();
        }
        let label = if likert {
            format!("{:.0}%", tick.abs())
        } else {
            super::tick_label(tick, s.decimals, scale.precise_ticks())
        };
        block(
            &mut out,
            &[label],
            px,
            bottom + small * 1.6,
            small,
            "middle",
        );
    }
    if likert {
        write!(
            out,
            "<path d=\"M{} {top} V{bottom}\" stroke=\"#687780\" stroke-width=\"0.8\"/>",
            x(0.)?
        )
        .unwrap();
    }
    let mut cursor = top;
    for (i, row) in rows.iter().enumerate() {
        c.checkpoint(i, Some(rows.len()))?;
        if !headings[i].is_empty() {
            block(&mut out, &headings[i], pad, cursor + small, small, "start");
        }
        cursor += heading_height(&headings[i]);
        let y = cursor + row_height * 0.5;
        cursor += row_height;
        block(
            &mut out,
            &labels[i],
            left - small,
            y - labels[i].len().saturating_sub(1) as f64 * small * 0.65 + small * 0.35,
            small,
            "end",
        );
        if row.n == 0 {
            block(
                &mut out,
                &["No recorded answers".into()],
                (left + right) / 2.,
                y + small * 0.35,
                small,
                "middle",
            );
            continue;
        }
        if likert {
            let mid = row.values.len() / 2;
            let percent = |count: u64| count as f64 / row.n as f64 * 100.;
            let negative: f64 = row.values[..mid].iter().map(|v| percent(*v)).sum();
            let mut start = -negative - percent(row.values[mid]) / 2.;
            let thickness = row_height * 0.66;
            for (j, count) in row.values.iter().enumerate() {
                let size = percent(*count);
                let end = start + size;
                let px = x(start)?;
                let width = x(end)? - px;
                write!(out, "<rect x=\"{px}\" y=\"{}\" width=\"{width}\" height=\"{thickness}\" fill=\"{}\"/>", y - thickness / 2., colours[j]).unwrap();
                let percent_label = format!("{size:.0}%");
                let value_fs = if s.likert_counts { small * 0.72 } else { small };
                let lines = if s.likert_counts {
                    vec![count.to_string(), percent_label]
                } else {
                    vec![percent_label]
                };
                if s.value_labels
                    && width
                        > lines
                            .iter()
                            .map(|l| measure(l, value_fs))
                            .fold(0., f64::max)
                            + value_fs
                    && thickness > value_fs * lines.len() as f64 * 1.3
                {
                    write!(out, "<g fill=\"{}\">", ink::label(colours[j])).unwrap();
                    block(
                        &mut out,
                        &lines,
                        px + width / 2.,
                        y + value_fs * 0.35
                            - lines.len().saturating_sub(1) as f64 * value_fs * 0.65,
                        value_fs,
                        "middle",
                    );
                    out.push_str("</g>");
                }
                start = end;
            }
        } else if let Some(mean) = row.mean {
            write!(
                out,
                "<circle cx=\"{}\" cy=\"{y}\" r=\"{}\" fill=\"#315A73\" fill-opacity=\"{}\"/>",
                x(mean)?,
                s.marker_size,
                s.opacity
            )
            .unwrap();
            if s.value_labels {
                block(
                    &mut out,
                    &[super::tick_label(mean, s.decimals, scale.precise_ticks())],
                    right + small,
                    y + small * 0.35,
                    small,
                    "start",
                );
            }
        }
    }
    block(
        &mut out,
        &axis_lines,
        (left + right) / 2.,
        bottom + small * 3.,
        small,
        "middle",
    );
    block(
        &mut out,
        &notes,
        pad,
        h - pad - notes.len().saturating_sub(1) as f64 * small * 1.3,
        small,
        "start",
    );
    out.push_str("</g></svg>");
    c.check_cancelled()?;
    let tree = parse(&out)?;
    let id = format!("survey-{}-{}", key.run, key.variant);
    let mut svg = tree.to_string(&resvg::usvg::WriteOptions {
        id_prefix: Some(format!("{id}-")),
        ..Default::default()
    });
    for (attr, value) in [("width", s.width_mm), ("height", s.height_mm)] {
        let search = format!("{attr}=\"");
        if let Some(start) = svg.find(&search) {
            let start = start + search.len();
            if let Some(end) = svg[start..].find('"') {
                svg.replace_range(start..start + end, &format!("{value}mm"));
            }
        }
    }
    if let Some(end) = svg.find('>') {
        svg.insert_str(
            end + 1,
            &format!(
                "<title id=\"{id}-title\">{}</title><desc id=\"{id}-desc\">{} {} {}</desc>",
                escape(&title),
                escape(&s.subtitle),
                escape(&note),
                escape(&exact)
            ),
        );
        svg.insert_str(
            4,
            &format!(
                " role=\"img\" aria-labelledby=\"{id}-title {id}-desc\" viewBox=\"0 0 {w} {h}\""
            ),
        );
    }
    c.check_cancelled()?;
    Ok(svg)
}
