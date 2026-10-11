//! One measured SVG drawing supplies the report, export and desktop preview.
use super::data::{self, Data, Mark};
use super::{FigureKey, FigureKind, FigurePalette, FigureSettings};
use crate::{
    charts::ChartScale,
    error::{AppError, AppResult},
    operation::JobControl,
    project::AnalysisRun,
};
use resvg::{tiny_skia, usvg};
use std::fmt::Write;
const FONT: &[u8] = include_bytes!("../../assets/fonts/NotoSans-Regular.ttf");
fn invalid(message: &str) -> AppError {
    AppError::invalid("figure", message)
}
fn escape(s: &str) -> String {
    s.chars()
        .map(|ch| match ch {
            '&' => "&amp;".into(),
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '"' => "&quot;".into(),
            '\'' => "&apos;".into(),
            _ => ch.to_string(),
        })
        .collect()
}
fn measure(s: &str, size: f64) -> f64 {
    let face = ttf_parser::Face::parse(FONT, 0).expect("bundled font checked");
    s.chars()
        .map(|c| {
            face.glyph_index(c)
                .and_then(|id| face.glyph_hor_advance(id))
                .unwrap_or(face.units_per_em()) as f64
        })
        .sum::<f64>()
        * size
        / face.units_per_em() as f64
}
fn wrap(s: &str, width: f64, size: f64, c: &JobControl) -> AppResult<Vec<String>> {
    if width < size * 2. {
        return Err(invalid(
            "Increase the figure width or reduce the font size to fit its labels.",
        ));
    }
    let mut lines = Vec::new();
    let mut line = String::new();
    for (i, word) in s.split_inclusive(char::is_whitespace).enumerate() {
        c.checkpoint(i, None)?;
        if word.trim_end_matches(['\r', '\n']).is_empty() && line.is_empty() {
            continue;
        }
        let ends = word.ends_with('\n');
        let word = word.trim_end_matches(['\r', '\n']);
        if measure(&(line.clone() + word), size) > width && !line.is_empty() {
            lines.push(line.trim_end().into());
            line = String::new();
        }
        for (j, ch) in word.chars().enumerate() {
            if j.is_multiple_of(64) {
                c.check_cancelled()?;
            }
            if lines.len() > 1000 {
                return Err(invalid("This label is too long for a report figure; shorten it or increase the figure size."));
            }
            let mut next = line.clone();
            next.push(ch);
            if measure(&next, size) > width && !line.is_empty() {
                lines.push(line.trim_end().into());
                line = String::new();
            }
            line.push(ch);
        }
        if ends {
            lines.push(line.trim_end().into());
            line = String::new();
        }
        if lines.len() > 1000 {
            return Err(invalid("This label is too long for a report figure; shorten it or increase the figure size."));
        }
    }
    if !line.is_empty() {
        lines.push(line.trim_end().into());
    }
    Ok(lines)
}
fn block(out: &mut String, lines: &[String], x: f64, y: f64, size: f64, anchor: &str) {
    for (i, line) in lines.iter().enumerate() {
        write!(
            out,
            "<text x=\"{x}\" y=\"{}\" font-size=\"{size}\" text-anchor=\"{anchor}\">{}</text>",
            y + i as f64 * size * 1.3,
            escape(line)
        )
        .unwrap();
    }
}
fn parse(svg: &str) -> AppResult<usvg::Tree> {
    if svg.len() > 32 * 1024 * 1024 {
        return Err(invalid(
            "The figure exceeds the supported SVG geometry budget.",
        ));
    }
    let mut options = usvg::Options {
        font_family: "Noto Sans".into(),
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    options.fontdb_mut().load_font_data(FONT.to_vec());
    usvg::Tree::from_str(svg, &options).map_err(|_| invalid("The figure could not be rendered."))
}
fn tick_values(bounds: [f64; 2], count: u8) -> AppResult<Vec<f64>> {
    if bounds[0] == bounds[1] {
        return Ok(vec![bounds[0]]);
    }
    (0..count)
        .map(|i| {
            crate::analysis::exploration::numeric::interpolate(
                bounds[0],
                bounds[1],
                i as u64,
                count as u64 - 1,
            )
        })
        .collect()
}
fn tick_label(v: f64, decimals: u8, precise: bool) -> String {
    if precise {
        format!("{v:.17e}")
    } else if v != 0. && (v.abs() < 10f64.powi(-(decimals as i32)) || v.abs() >= 1e7) {
        format!("{:.*e}", decimals as usize, v)
    } else {
        format!("{:.*}", decimals as usize, v)
    }
}
struct Layout {
    left: f64,
    right: f64,
    top: f64,
    bottom: f64,
    xs: ChartScale,
    ys: ChartScale,
    xb: [f64; 2],
    yb: [f64; 2],
}
impl Layout {
    fn x(&self, x: f64) -> AppResult<f64> {
        Ok(self.left + (self.right - self.left) * self.xs.position(x)?)
    }
    fn y(&self, y: f64) -> AppResult<f64> {
        Ok(self.bottom - (self.bottom - self.top) * self.ys.position(y)?)
    }
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.xb[0] && x <= self.xb[1] && y >= self.yb[0] && y <= self.yb[1]
    }
}
pub fn svg(
    run: &AnalysisRun,
    key: &FigureKey,
    s: &FigureSettings,
    stale: bool,
    c: &JobControl,
) -> AppResult<String> {
    c.check_cancelled()?;
    s.validate()?;
    if key.run != run.id.0 {
        return Err(invalid("Choose a figure from the retained analysis run."));
    }
    let mut d = data::captured(run, &key.kind, s, c)?;
    if stale {
        d.note
            .push_str(" Stale result: captured from an earlier data revision.");
        d.visible_note
            .push_str(" Stale result: earlier data revision.");
    }
    if s.x_bounds.is_some() || s.y_bounds.is_some() {
        d.note
            .push_str(" Display limits are customised; values outside the viewport are clipped.");
        d.visible_note
            .push_str(" Custom display limits; values outside the viewport are clipped.");
    }
    if !s.caption.is_empty() {
        d.note = format!("{} {}", s.caption, d.note);
        d.visible_note = format!("{} {}", s.caption, d.visible_note);
    }
    let title = if s.title.is_empty() {
        d.title.clone()
    } else {
        s.title.clone()
    };
    let xname = if s.x_label.is_empty() {
        d.x.clone()
    } else {
        s.x_label.clone()
    };
    let yname = if s.y_label.is_empty() {
        d.y.clone()
    } else {
        s.y_label.clone()
    };
    let horizontal = d.categorical && matches!(key.kind, FigureKind::Frequency(_)) && s.horizontal;
    if horizontal {
        d.bounds.swap(0, 1);
    }
    let xb = s.x_bounds.unwrap_or(d.bounds[0]);
    let yb = s.y_bounds.unwrap_or(d.bounds[1]);
    if matches!(key.kind, FigureKind::Histogram | FigureKind::Frequency(_))
        && (if horizontal { xb[0] } else { yb[0] }) != 0.
    {
        return Err(invalid(
            "Frequency bars require a zero baseline. Set the frequency axis minimum to zero.",
        ));
    }
    let xs = ChartScale::new(xb[0], xb[1])?;
    let ys = ChartScale::new(yb[0], yb[1])?;
    let width = s.width_mm * 72. / 25.4;
    let height = s.height_mm * 72. / 25.4;
    let fs = s.font_size;
    let small = fs * 0.8;
    let pad = fs;
    let xticks = tick_values(xb, s.ticks)?;
    let yticks = tick_values(yb, s.ticks)?;
    let xprecise = xs.precise_ticks()
        || xticks
            .windows(2)
            .any(|v| tick_label(v[0], s.decimals, false) == tick_label(v[1], s.decimals, false));
    let yprecise = ys.precise_ticks()
        || yticks
            .windows(2)
            .any(|v| tick_label(v[0], s.decimals, false) == tick_label(v[1], s.decimals, false));
    let xtick_labels: Vec<_> = xticks
        .iter()
        .map(|v| tick_label(*v, s.decimals, xprecise))
        .collect();
    let ytick_labels: Vec<_> = yticks
        .iter()
        .map(|v| tick_label(*v, s.decimals, yprecise))
        .collect();
    let titles = wrap(&title, width - 2. * pad, fs * 1.25, c)?;
    let subtitles = wrap(&s.subtitle, width - 2. * pad, small, c)?;
    let notes = wrap(&d.visible_note, width - 2. * pad, small, c)?;
    let value_space = if matches!(key.kind, FigureKind::Frequency(_)) && s.value_labels {
        small * 1.3
    } else {
        0.
    };
    let top = value_space
        + pad
        + (titles.len() as f64 * fs * 1.25 + subtitles.len() as f64 * small) * 1.3
        + pad * 0.4;
    let mut left = pad * 2.8
        + ytick_labels
            .iter()
            .map(|s| measure(s, small))
            .fold(0., f64::max);
    let mut categories = Vec::new();
    if horizontal {
        left = width * 0.36;
        for label in &d.categories {
            categories.push(wrap(label, left - pad * 2., small, c)?);
        }
    }
    let label_space = if horizontal && s.value_labels {
        d.marks
            .iter()
            .filter_map(|m| match m {
                Mark::Bar(_, v) => Some(measure(&tick_label(*v, s.decimals, false), small) + small),
                _ => None,
            })
            .fold(0., f64::max)
    } else {
        0.
    };
    let right = width
        - pad
        - label_space
        - measure(xtick_labels.last().map_or("", String::as_str), small) * 0.5;
    if d.categorical && !horizontal {
        let cell = (right - left) / d.categories.len().max(1) as f64;
        for label in &d.categories {
            categories.push(wrap(label, cell - fs * 0.3, small, c)?);
        }
    }
    let maxcat = categories.iter().map(Vec::len).max().unwrap_or(0) as f64;
    let category_height = if horizontal { 0. } else { maxcat * small * 1.3 };
    let xlines = wrap(
        if horizontal { &yname } else { &xname },
        right - left,
        fs,
        c,
    )?;
    let ylines = wrap(
        if horizontal { &xname } else { &yname },
        height * 0.55,
        fs,
        c,
    )?;
    let bottom = height
        - pad
        - (notes.len() as f64 * small * 1.3)
        - pad * 0.6
        - (xlines.len() as f64 * fs * 1.3)
        - category_height
        - small * 1.7;
    if right - left < fs * 5. || bottom - top < fs * 4. {
        return Err(invalid("The labels leave too little room for the plot. Increase figure dimensions or reduce font size and caption length."));
    }
    if horizontal
        && categories.iter().any(|lines| {
            lines.len() as f64 * small * 1.3 + small * 0.6
                > (bottom - top) / categories.len().max(1) as f64
        })
    {
        return Err(invalid("The category labels overlap. Increase figure height, shorten labels, or reduce the font size."));
    }
    if ylines.len() > 2 {
        return Err(invalid("The vertical axis title needs more space. Shorten it, reduce font size, or increase figure height."));
    }
    if !d.categorical || horizontal {
        let needed = xtick_labels
            .iter()
            .map(|s| measure(s, small))
            .fold(0., f64::max);
        if xticks.len() > 1 && needed > (right - left) / (xticks.len() - 1) as f64 {
            return Err(invalid("Horizontal tick labels overlap. Increase figure width, reduce tick count, or reduce font size."));
        }
    }
    let l = Layout {
        left,
        right,
        top,
        bottom,
        xs,
        ys,
        xb,
        yb,
    };
    let colour = match s.palette {
        FigurePalette::Report => "#315A73",
        FigurePalette::Monochrome => "#505050",
        FigurePalette::Accessible => "#0072B2",
    };
    let clip =
        format!("figure-{}-{:?}-{}", key.run, key.kind, key.variant).replace(['(', ')'], "_");
    let mut out=format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\" role=\"img\"><title>{}</title><desc>{} {} {} {}</desc>",escape(&title),escape(&s.subtitle),escape(&xname),escape(&yname),escape(&d.note));
    if !s.transparent {
        write!(
            out,
            "<rect width=\"{width}\" height=\"{height}\" fill=\"white\"/>"
        )
        .unwrap();
    }
    out.push_str("<g font-family=\"Noto Sans\" fill=\"#202B33\">");
    block(&mut out, &titles, pad, pad + fs * 1.1, fs * 1.25, "start");
    block(
        &mut out,
        &subtitles,
        pad,
        pad + fs * 1.1 + titles.len() as f64 * fs * 1.25 * 1.3,
        small,
        "start",
    );
    axes(
        &mut out,
        &l,
        &d,
        horizontal,
        &categories,
        &xticks,
        &yticks,
        &xtick_labels,
        &ytick_labels,
        s,
    )?;
    block(
        &mut out,
        &xlines,
        (left + right) * 0.5,
        bottom + small * 1.7 + category_height + fs,
        fs,
        "middle",
    );
    write!(
        out,
        "<g transform=\"translate({} {}) rotate(-90)\">",
        pad,
        (top + bottom) * 0.5
    )
    .unwrap();
    block(&mut out, &ylines, 0., 0., fs, "middle");
    out.push_str("</g>");
    block(
        &mut out,
        &notes,
        pad,
        height - pad - (notes.len().saturating_sub(1)) as f64 * small * 1.3,
        small,
        "start",
    );
    write!(out,"<defs><clipPath id=\"{clip}\"><rect x=\"{left}\" y=\"{top}\" width=\"{}\" height=\"{}\"/></clipPath></defs><g clip-path=\"url(#{clip})\" fill=\"{colour}\" stroke=\"{colour}\" fill-opacity=\"{}\">",right-left,bottom-top,s.opacity).unwrap();
    marks(&mut out, &l, &d, horizontal, s, c)?;
    out.push_str("</g>");
    if s.value_labels {
        for mark in &d.marks {
            if let Mark::Bar(index, value) = mark {
                let pos = category_position(*index, d.categories.len(), horizontal);
                let label = tick_label(*value, s.decimals, false);
                if horizontal {
                    if pos >= l.yb[0] && pos <= l.yb[1] {
                        block(
                            &mut out,
                            &[label],
                            l.x(value.min(l.xb[1]))? + small * 0.5,
                            l.y(pos)? + small * 0.35,
                            small,
                            "start",
                        );
                    }
                } else if pos >= l.xb[0] && pos <= l.xb[1] {
                    block(
                        &mut out,
                        &[label],
                        l.x(pos)?,
                        l.y(value.min(l.yb[1]))? - small * 0.4,
                        small,
                        "middle",
                    );
                }
            }
        }
    }
    out.push_str("</g></svg>");
    c.check_cancelled()?;
    // Outline text with the bundled font so destination applications need no fonts or network.
    let tree = parse(&out)?;
    let mut outlined = tree.to_string(&usvg::WriteOptions {
        id_prefix: Some(format!("{clip}-")),
        ..Default::default()
    });
    let metadata = format!(
        "<title>{}</title><desc>{} {} {} {} {}</desc>",
        escape(&title),
        escape(&s.subtitle),
        escape(&xname),
        escape(&yname),
        escape(&d.note),
        escape(&d.categories.join("; "))
    );
    for (name, value) in [("width", s.width_mm), ("height", s.height_mm)] {
        let attr = format!("{name}=\"");
        if let Some(start) = outlined.find(&attr) {
            let start = start + attr.len();
            if let Some(end) = outlined[start..].find('"') {
                outlined.replace_range(start..start + end, &format!("{value}mm"));
            }
        }
    }
    if let Some(end) = outlined.find('>') {
        outlined.insert_str(end + 1, &metadata);
        outlined.insert_str(
            4,
            &format!(" role=\"img\" viewBox=\"0 0 {width} {height}\""),
        );
    }
    c.check_cancelled()?;
    Ok(outlined)
}
fn category_position(index: usize, count: usize, horizontal: bool) -> f64 {
    if horizontal {
        count as f64 - index as f64 - 0.5
    } else {
        index as f64 + 0.5
    }
}
fn axes(
    out: &mut String,
    l: &Layout,
    d: &Data,
    horizontal: bool,
    categories: &[Vec<String>],
    xticks: &[f64],
    yticks: &[f64],
    xlabels: &[String],
    ylabels: &[String],
    s: &FigureSettings,
) -> AppResult<()> {
    let small = s.font_size * 0.8;
    write!(
        out,
        "<path d=\"M{} {} V{} H{}\" fill=\"none\" stroke=\"#66727A\" stroke-width=\"0.6\"/>",
        l.left, l.top, l.bottom, l.right
    )
    .unwrap();
    if !d.categorical || horizontal {
        for (value, label) in xticks.iter().zip(xlabels) {
            let x = l.x(*value)?;
            if s.grid {
                write!(
                    out,
                    "<path d=\"M{x} {} V{}\" stroke=\"#E3E8EB\" stroke-width=\"0.5\"/>",
                    l.top, l.bottom
                )
                .unwrap();
            }
            block(
                out,
                std::slice::from_ref(label),
                x,
                l.bottom + small * 1.4,
                small,
                "middle",
            );
        }
    }
    if !horizontal {
        for (value, label) in yticks.iter().zip(ylabels) {
            let y = l.y(*value)?;
            if s.grid {
                write!(
                    out,
                    "<path d=\"M{} {y} H{}\" stroke=\"#E3E8EB\" stroke-width=\"0.5\"/>",
                    l.left, l.right
                )
                .unwrap();
            }
            block(
                out,
                std::slice::from_ref(label),
                l.left - small * 0.6,
                y + small * 0.35,
                small,
                "end",
            );
        }
    }
    for (i, lines) in categories.iter().enumerate() {
        let pos = category_position(i, d.categories.len(), horizontal);
        let bounds = if horizontal { l.yb } else { l.xb };
        if pos < bounds[0] || pos > bounds[1] {
            continue;
        }
        if horizontal {
            let y = l.y(pos)?;
            block(
                out,
                lines,
                l.left - small * 0.7,
                y - (lines.len().saturating_sub(1)) as f64 * small * 0.65 + small * 0.35,
                small,
                "end",
            );
        } else {
            block(
                out,
                lines,
                l.x(pos)?,
                l.bottom + small * 1.4,
                small,
                "middle",
            );
        }
    }
    Ok(())
}
fn marks(
    out: &mut String,
    l: &Layout,
    d: &Data,
    horizontal: bool,
    s: &FigureSettings,
    c: &JobControl,
) -> AppResult<()> {
    for (i, mark) in d.marks.iter().enumerate() {
        c.checkpoint(i, Some(d.marks.len()))?;
        match mark {
            Mark::Point(x, y) => {
                if l.contains(*x, *y) {
                    write!(
                        out,
                        "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" stroke=\"none\"/>",
                        l.x(*x)?,
                        l.y(*y)?,
                        s.marker_size
                    )
                    .unwrap();
                }
            }
            Mark::Bin(a, b, v) => {
                if *b < l.xb[0] || *a > l.xb[1] {
                    continue;
                }
                let left = l.x(a.max(l.xb[0]))?;
                let right = l.x(b.min(l.xb[1]))?;
                let y = l.y(v.min(l.yb[1]).max(l.yb[0]))?;
                if a == b {
                    write!(
                        out,
                        "<path d=\"M{left} {} V{y}\" stroke-width=\"{}\"/>",
                        l.bottom,
                        s.marker_size * 2.
                    )
                    .unwrap();
                } else {
                    write!(out,"<rect x=\"{left}\" y=\"{y}\" width=\"{}\" height=\"{}\" stroke=\"white\" stroke-width=\"0.5\"/>",right-left,l.bottom-y).unwrap();
                }
            }
            Mark::Bar(index, value) => {
                let pos = category_position(*index, d.categories.len(), horizontal);
                let thickness = if horizontal {
                    (l.bottom - l.top) / d.categories.len() as f64 * 0.68
                } else {
                    (l.right - l.left) / d.categories.len() as f64 * 0.68
                };
                let (x, y, w, h) = if horizontal {
                    if pos < l.yb[0] || pos > l.yb[1] {
                        continue;
                    }
                    (
                        l.left,
                        l.y(pos)? - thickness * 0.5,
                        l.x(value.min(l.xb[1]))? - l.left,
                        thickness,
                    )
                } else {
                    if pos < l.xb[0] || pos > l.xb[1] {
                        continue;
                    }
                    let y = l.y(value.min(l.yb[1]))?;
                    (l.x(pos)? - thickness * 0.5, y, thickness, l.bottom - y)
                };
                write!(
                    out,
                    "<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" stroke=\"none\"/>"
                )
                .unwrap();
            }
            Mark::Box(index, q1, median, q3, whiskers) => {
                let pos = category_position(*index, d.categories.len(), horizontal);
                if pos < l.xb[0] || pos > l.xb[1] {
                    continue;
                }
                let x = l.x(pos)?;
                let width = ((l.right - l.left) / d.categories.len() as f64 * 0.5).min(30.);
                let clamp = |v: f64| v.max(l.yb[0]).min(l.yb[1]);
                let top = l.y(clamp(*q3))?;
                let bottom = l.y(clamp(*q1))?;
                write!(out,"<rect x=\"{}\" y=\"{top}\" width=\"{width}\" height=\"{}\" fill-opacity=\"0.3\" stroke-width=\"1\"/>",x-width*0.5,bottom-top).unwrap();
                let mid = l.y(clamp(*median))?;
                write!(
                    out,
                    "<path d=\"M{} {mid} H{}\" fill=\"none\" stroke-width=\"2\"/>",
                    x - width * 0.5,
                    x + width * 0.5
                )
                .unwrap();
                for value in whiskers.iter().flatten() {
                    let y = l.y(clamp(*value))?;
                    write!(out,"<path d=\"M{x} {mid} V{y} M{} {y} H{}\" fill=\"none\" stroke-width=\"0.8\"/>",x-width*0.3,x+width*0.3).unwrap();
                }
            }
        }
    }
    Ok(())
}
fn pixmap(svg: &str, width: u32, height: u32, c: &JobControl) -> AppResult<tiny_skia::Pixmap> {
    c.check_cancelled()?;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 40_000_000 {
        return Err(invalid(
            "Choose an image size of at most 40 million pixels.",
        ));
    }
    let tree = parse(svg)?;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| invalid("There is not enough memory to render this figure."))?;
    let transform = tiny_skia::Transform::from_scale(
        width as f32 / tree.size().width(),
        height as f32 / tree.size().height(),
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    c.check_cancelled()?;
    Ok(pixmap)
}
pub fn png(svg: &str, s: &FigureSettings, c: &JobControl) -> AppResult<Vec<u8>> {
    let (w, h) = s.pixel_size()?;
    let (_, _, rgba) = raster(svg, w, h, c)?;
    let mut bytes = Vec::new();
    {
        let mut encoder = ::png::Encoder::new(&mut bytes, w, h);
        encoder.set_color(::png::ColorType::Rgba);
        encoder.set_depth(::png::BitDepth::Eight);
        let ppm = (s.dpi as f64 / 0.0254).round() as u32;
        encoder.set_pixel_dims(Some(::png::PixelDimensions {
            xppu: ppm,
            yppu: ppm,
            unit: ::png::Unit::Meter,
        }));
        let mut writer = encoder
            .write_header()
            .map_err(|_| invalid("Could not encode the PNG figure."))?;
        writer
            .write_image_data(&rgba)
            .map_err(|_| invalid("Could not encode the PNG figure."))?;
    }
    c.check_cancelled()?;
    Ok(bytes)
}
pub fn raster(
    svg: &str,
    width: u32,
    height: u32,
    c: &JobControl,
) -> AppResult<(u32, u32, Vec<u8>)> {
    let p = pixmap(svg, width, height, c)?;
    let mut bytes = Vec::with_capacity(p.data().len());
    for (i, pixel) in p.pixels().iter().enumerate() {
        if i.is_multiple_of(4096) {
            c.checkpoint(i, Some(p.pixels().len()))?;
        }
        let colour = pixel.demultiply();
        bytes.extend_from_slice(&[colour.red(), colour.green(), colour.blue(), colour.alpha()]);
    }
    Ok((width, height, bytes))
}
