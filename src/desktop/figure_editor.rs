//! Report figures use the exported SVG for their preview as well as PNG output.
use super::{button, procedure_forms::record, start, DesktopState};
use crate::{
    error::AppResult,
    export::figures::FigureFormat,
    figures::{
        self, CategoryOrder, FigureKey, FigureKind, FigurePalette, FigureSettings, FrequencyMetric,
    },
    jobs::{JobOperation, JobRunner},
    operation::JobControl,
};
use eframe::egui::{self, ColorImage, TextureHandle};
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct FigureEditor {
    key: FigureKey,
    settings: FigureSettings,
    previous: Option<FigureSettings>,
    previous_applied: bool,
    previous_saved: bool,
    preset_name: String,
    format: FigureFormat,
    targets: Vec<FigureKey>,
    texture: Option<TextureHandle>,
    rendered: Option<(FigureSettings, bool)>,
    error: String,
}
impl std::fmt::Debug for FigureEditor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FigureEditor")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}
impl FigureEditor {
    fn new(key: FigureKey, settings: FigureSettings) -> Self {
        Self {
            targets: vec![key],
            key,
            settings,
            previous: None,
            previous_applied: false,
            previous_saved: false,
            preset_name: String::new(),
            format: FigureFormat::Svg,
            texture: None,
            rendered: None,
            error: String::new(),
        }
    }
    pub(crate) fn owned_bytes(&self) -> usize {
        self.settings.heap_bytes()
            + self.previous.as_ref().map_or(0, FigureSettings::heap_bytes)
            + self.rendered.as_ref().map_or(0, |(s, _)| s.heap_bytes())
            + self.preset_name.capacity()
            + self.error.capacity()
            + self.targets.capacity() * std::mem::size_of::<FigureKey>()
            + usize::from(self.texture.is_some()) * 2_000_000
    }
}
fn label(key: &FigureKey) -> String {
    let kind = match key.kind {
        FigureKind::Histogram => "Histogram".into(),
        FigureKind::Scatter => "Scatterplot".into(),
        FigureKind::Boxplot => "Boxplot".into(),
        FigureKind::Residual => "Residual plot".into(),
        FigureKind::Qq => "Normal Q–Q plot".into(),
        FigureKind::Distribution(c) => format!("Distribution · variable {c}"),
        FigureKind::Frequency(c) => format!("Bar chart · variable {c}"),
    };
    format!(
        "Run {} · {kind}{}",
        key.run,
        if key.variant == 0 {
            String::new()
        } else {
            format!(" · version {}", u64::from(key.variant) + 1)
        }
    )
}
fn keys(state: &DesktopState, selected_run: bool) -> Vec<FigureKey> {
    let mut keys = Vec::new();
    for run in state.project().runs() {
        if selected_run && !state.selected_run().is_some_and(|r| r.id == run.id) {
            continue;
        }
        keys.extend(figures::figure_keys(run));
        keys.extend(
            state
                .project()
                .figures()
                .iter()
                .filter(|f| f.key.run == run.id.0 && f.key.variant != 0)
                .map(|f| f.key),
        );
    }
    keys
}
pub(super) fn toolbar(ui: &mut egui::Ui, state: &mut DesktopState, jobs: &mut JobRunner) {
    let available = keys(state, true);
    if available.is_empty() {
        return;
    }
    let mut selected = state
        .selected_figure
        .filter(|k| available.contains(k))
        .unwrap_or_else(|| available[0]);
    ui.horizontal_wrapped(|ui| {
        ui.strong("Report figures");
        let combo = egui::ComboBox::from_id_salt("report-figure")
            .selected_text(label(&selected))
            .show_ui(ui, |ui| {
                for key in &available {
                    ui.selectable_value(&mut selected, *key, label(key));
                }
            });
        record(state, "Report figure", &combo.response);
        if button(ui, state, "Edit figure").clicked() && state.figure_editor.is_none() {
            state.figure_editor = Some(FigureEditor::new(
                selected,
                state.project().figure_settings(&selected),
            ));
        }
        let export = ui.add_enabled(!state.active, egui::Button::new("Export figure…"));
        record(state, "Export figure…", &export);
        if export.clicked() {
            export_one(state, jobs, &selected, FigureFormat::Svg);
        }
        if ui
            .add_enabled(!state.active, egui::Button::new("Duplicate figure"))
            .clicked()
        {
            if let Err(e) = state.duplicate_report_figure(&selected) {
                state.fail(e);
            } else if let Some(k) = state.selected_figure {
                selected = k;
            }
        }
    });
    state.selected_figure = Some(selected);
    editor(ui, state, jobs);
    ui.separator();
}
fn export_one(
    state: &mut DesktopState,
    jobs: &mut JobRunner,
    key: &FigureKey,
    format: FigureFormat,
) {
    let extension = if format == FigureFormat::Svg {
        "svg"
    } else {
        "png"
    };
    state.modal_file_dialog = true;
    if let Some(path) = rfd::FileDialog::new()
        .add_filter("Figure", &[extension])
        .set_file_name(format!("figure-{}.{}", key.run, extension))
        .save_file()
    {
        start(
            state,
            jobs,
            JobOperation::ExportFigure {
                project: Arc::clone(state.project()),
                path,
                key: *key,
                format,
            },
        );
    }
}
fn editor(ui: &mut egui::Ui, state: &mut DesktopState, jobs: &mut JobRunner) {
    let Some(mut edit) = state.figure_editor.take() else {
        return;
    };
    state.figure_work_bytes = edit.owned_bytes();
    let mut open = true;
    let mut cancel = false;
    let run = state
        .project()
        .runs()
        .iter()
        .find(|r| r.id.0 == edit.key.run)
        .cloned();
    let Some(run) = run else {
        state.figure_work_bytes = 0;
        return;
    };
    let stale = run.analysis.revision() != state.project().dataset().revision();
    let available = ui.ctx().content_rect().shrink(8.0);
    egui::Window::new("Edit report figure").id(egui::Id::new("figure-editor")).open(&mut open).default_size([1020.,650.]).max_size(available.size()).constrain_to(available).resizable(true).show(ui.ctx(),|ui|{
        ui.label(label(&edit.key));
        ui.small("Apply saves presentation in the project. Captured data and calculations stay attached to the original run.");
        ui.add_enabled_ui(!state.active,|ui|{
            egui::ScrollArea::vertical().id_salt("figure-properties-scroll").max_height((available.height()-320.).clamp(80.,500.)).show(ui,|ui|{
                ui.columns(2,|columns|{
                    properties(&mut columns[0],state,jobs,&mut edit);
                    preview(&mut columns[1],state,&run,stale,&mut edit);
                });
            });
            ui.separator();
            ui.horizontal_wrapped(|ui|{
                if button(ui,state,"Apply figure").clicked() {
                    if let Err(e)=apply_edit(state,&mut edit){edit.error=e.to_string();}
                }
                if button(ui,state,"Reset style").clicked() {edit.previous=Some(edit.settings.clone());edit.previous_applied=false;edit.settings=edit.settings.copy_style_from(&FigureSettings::default());}
                let undo=ui.add_enabled(edit.previous.is_some(),egui::Button::new("Undo figure edit"));
                record(state,"Undo figure edit",&undo);
                if undo.clicked() {
                    if let Err(e)=undo_edit(state,&mut edit){edit.error=e.to_string();}
                }
                if button(ui,state,"Cancel figure edit").clicked(){cancel=true;}
                egui::ComboBox::from_id_salt("figure-format").selected_text(if edit.format==FigureFormat::Svg{"SVG"}else{"PNG"}).show_ui(ui,|ui|{
                    ui.selectable_value(&mut edit.format,FigureFormat::Svg,"SVG · vector");ui.selectable_value(&mut edit.format,FigureFormat::Png,"PNG · image");
                });
                if button(ui,state,"Apply and export…").clicked() {
                    match apply_edit(state,&mut edit) {Ok(())=>export_one(state,jobs,&edit.key,edit.format),Err(e)=>edit.error=e.to_string()}
                }
                if button(ui,state,"Copy figure image").clicked() {
                    let control=JobControl::default();
                    let copied=(||{
                        state.check_ui_copy(figures::SVG_WORK_BYTES)?;
                        let svg=figures::render::svg(&run,&edit.key,&edit.settings,stale,&control)?;
                        let (w,h)=edit.settings.pixel_size()?;
                        state.check_ui_copy((w as usize).saturating_mul(h as usize).saturating_mul(12)+svg.len())?;
                        let (w,h,rgba)=figures::render::raster(&svg,w,h,&control)?;
                        ui.ctx().copy_image(ColorImage::from_rgba_unmultiplied([w as usize,h as usize],&rgba));
                        Ok::<_,crate::error::AppError>(())
                    })();
                    match copied {Ok(())=>state.status="Figure image copied at the selected resolution.".into(),Err(e)=>edit.error=e.to_string()}
                }
            });
        });
        if !edit.error.is_empty(){ui.colored_label(ui.visuals().error_fg_color,&edit.error);}
    });
    if open && !cancel {
        state.figure_editor = Some(edit);
    }
    state.figure_work_bytes = 0;
}
fn apply_edit(state: &mut DesktopState, edit: &mut FigureEditor) -> AppResult<()> {
    let old = state.project().figure_settings(&edit.key);
    let saved = state.project().figures().iter().any(|f| f.key == edit.key);
    state.apply_figure_settings(&edit.key, edit.settings.clone())?;
    edit.previous = Some(old);
    edit.previous_applied = true;
    edit.previous_saved = saved;
    edit.error.clear();
    Ok(())
}
fn undo_edit(state: &mut DesktopState, edit: &mut FigureEditor) -> AppResult<()> {
    if let Some(previous) = &edit.previous {
        if edit.previous_applied {
            if edit.previous_saved {
                state.apply_figure_settings(&edit.key, previous.clone())?;
            } else {
                state.remove_figure_settings(&edit.key)?;
            }
        }
        edit.settings = previous.clone();
        edit.previous = None;
        edit.previous_applied = false;
        edit.error.clear();
    }
    Ok(())
}
fn text_field(ui: &mut egui::Ui, state: &mut DesktopState, label: &str, text: &mut String) {
    ui.label(label);
    let response = ui.add(
        egui::TextEdit::multiline(text)
            .desired_rows(1)
            .char_limit(1024)
            .hint_text("Automatic when blank"),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, label));
    record(state, label, &response);
}
fn properties(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    jobs: &mut JobRunner,
    edit: &mut FigureEditor,
) {
    let s = &mut edit.settings;
    egui::CollapsingHeader::new("Text")
        .default_open(true)
        .show(ui, |ui| {
            for (label, text) in [
                ("Figure title", &mut s.title),
                ("Subtitle", &mut s.subtitle),
                ("X axis title", &mut s.x_label),
                ("Y axis title", &mut s.y_label),
                ("Caption / source", &mut s.caption),
            ] {
                text_field(ui, state, label, text);
            }
        });
    egui::CollapsingHeader::new("Appearance")
        .default_open(true)
        .show(ui, |ui| {
            egui::ComboBox::from_id_salt("figure-palette")
                .selected_text(format!("{:?}", s.palette))
                .show_ui(ui, |ui| {
                    for (v, name) in [
                        (FigurePalette::Report, "Report"),
                        (FigurePalette::Monochrome, "Monochrome"),
                        (FigurePalette::Accessible, "Colour-accessible"),
                    ] {
                        ui.selectable_value(&mut s.palette, v, name);
                    }
                });
            number(ui, state, "Text size (pt)", &mut s.font_size, 6., 36.);
            number(ui, state, "Marker size", &mut s.marker_size, 0.5, 20.);
            number(ui, state, "Opacity", &mut s.opacity, 0., 1.);
            for (label, v) in [
                ("Gridlines", &mut s.grid),
                ("Transparent background", &mut s.transparent),
            ] {
                let r = ui.checkbox(v, label);
                record(state, label, &r);
            }
        });
    egui::CollapsingHeader::new("Size")
        .default_open(true)
        .show(ui, |ui| {
            number(ui, state, "Width (mm)", &mut s.width_mm, 40., 500.);
            number(ui, state, "Height (mm)", &mut s.height_mm, 40., 500.);
            ui.horizontal(|ui| {
                ui.label("PNG resolution (dpi)");
                let r = ui.add(egui::DragValue::new(&mut s.dpi).range(72..=600));
                record(state, "PNG resolution (dpi)", &r);
            });
            if let Ok((w, h)) = s.pixel_size() {
                ui.small(format!("PNG: {w} × {h} pixels"));
            }
        });
    egui::CollapsingHeader::new("Axes").show(ui, |ui| {
        bounds(ui, state, "X axis limits", &mut s.x_bounds);
        bounds(ui, state, "Y axis limits", &mut s.y_bounds);
        ui.horizontal(|ui| {
            ui.label("Ticks");
            let r = ui.add(egui::DragValue::new(&mut s.ticks).range(2..=12));
            record(state, "Ticks", &r);
        });
        ui.horizontal(|ui| {
            ui.label("Decimal places");
            let r = ui.add(egui::DragValue::new(&mut s.decimals).range(0..=8));
            record(state, "Decimal places", &r);
        });
    });
    if matches!(edit.key.kind, FigureKind::Frequency(_)) {
        egui::CollapsingHeader::new("Bar chart")
            .default_open(true)
            .show(ui, |ui| {
                let r = ui.checkbox(&mut s.horizontal, "Horizontal bars");
                record(state, "Horizontal bars", &r);
                egui::ComboBox::from_id_salt("figure-metric")
                    .selected_text(format!("{:?}", s.metric))
                    .show_ui(ui, |ui| {
                        for (v, n) in [
                            (FrequencyMetric::Count, "Counts"),
                            (FrequencyMetric::ValidPercent, "Percent of valid responses"),
                            (FrequencyMetric::AllPercent, "Percent of all responses"),
                        ] {
                            ui.selectable_value(&mut s.metric, v, n);
                        }
                    });
                egui::ComboBox::from_id_salt("figure-order")
                    .selected_text(format!("{:?}", s.order))
                    .show_ui(ui, |ui| {
                        for (v, n) in [
                            (CategoryOrder::Original, "Original category order"),
                            (CategoryOrder::Value, "Category value"),
                            (CategoryOrder::Frequency, "Frequency descending"),
                        ] {
                            ui.selectable_value(&mut s.order, v, n);
                        }
                    });
                let r = ui.checkbox(&mut s.value_labels, "Show values");
                record(state, "Show values", &r);
            });
    }
    // Styles preserve titles, axes and the figure's statistical display choices.
    egui::CollapsingHeader::new("Reuse styles / batch export").show(ui, |ui| {
        let presets = state.project().presets().to_vec();
        egui::ComboBox::from_id_salt("figure-preset")
            .selected_text("Choose a saved style")
            .show_ui(ui, |ui| {
                for preset in presets {
                    if ui.selectable_label(false, &preset.name).clicked() {
                        edit.settings = edit.settings.copy_style_from(&preset.settings);
                        ui.close();
                    }
                }
            });
        text_field(ui, state, "Style name", &mut edit.preset_name);
        if button(ui, state, "Save figure style").clicked() {
            if let Err(e) = state.save_figure_preset(&edit.preset_name, edit.settings.clone()) {
                edit.error = e.to_string();
            }
        }
        ui.small("Select figures for style application or export:");
        let all = keys(state, false);
        egui::ScrollArea::vertical()
            .id_salt("batch-figures")
            .max_height(150.)
            .show_rows(ui, 24., all.len(), |ui, range| {
                for k in &all[range] {
                    let mut selected = edit.targets.contains(k);
                    if ui.checkbox(&mut selected, label(k)).changed() {
                        if selected && edit.targets.len() < 256 {
                            edit.targets.push(*k);
                        } else if !selected {
                            edit.targets.retain(|key| key != k);
                        } else {
                            edit.error = "Select at most 256 figures in one batch.".into();
                        }
                    }
                }
            });
        if button(ui, state, "Apply style to selected figures").clicked() {
            let result = apply_style(state, &edit.targets, &edit.settings);
            if let Err(e) = result {
                edit.error = e.to_string();
            }
        }
        if ui
            .add_enabled(
                !edit.targets.is_empty(),
                egui::Button::new("Export selected figures…"),
            )
            .clicked()
        {
            state.modal_file_dialog = true;
            if let Some(directory) = rfd::FileDialog::new().pick_folder() {
                start(
                    state,
                    jobs,
                    JobOperation::ExportFigures {
                        project: Arc::clone(state.project()),
                        directory,
                        keys: edit.targets.clone(),
                        format: edit.format,
                    },
                );
            }
        }
    });
}
fn number(ui: &mut egui::Ui, state: &mut DesktopState, label: &str, v: &mut f64, lo: f64, hi: f64) {
    ui.horizontal(|ui| {
        ui.label(label);
        let r = ui.add(egui::DragValue::new(v).range(lo..=hi).speed(0.1));
        record(state, label, &r);
    });
}
fn bounds(ui: &mut egui::Ui, state: &mut DesktopState, label: &str, b: &mut Option<[f64; 2]>) {
    let mut custom = b.is_some();
    let r = ui.checkbox(&mut custom, label);
    record(state, label, &r);
    if custom && b.is_none() {
        *b = Some([0., 1.]);
    } else if !custom {
        *b = None;
    }
    if let Some([lo, hi]) = b {
        ui.horizontal(|ui| {
            ui.label("Minimum");
            ui.add(egui::DragValue::new(lo).speed(0.1));
            ui.label("Maximum");
            ui.add(egui::DragValue::new(hi).speed(0.1));
        });
    }
}
fn apply_style(
    state: &mut DesktopState,
    keys: &[FigureKey],
    settings: &FigureSettings,
) -> AppResult<()> {
    if state.active {
        return Err(crate::error::AppError::Conflict(
            "Wait for the current operation.".into(),
        ));
    }
    let original = Arc::clone(state.project());
    let mut updated = original.as_ref().clone();
    for key in keys {
        let applied = updated.figure_settings(key).copy_style_from(settings);
        updated = updated.with_figure_with_budget(
            key,
            applied,
            &[],
            state.extra() + original.owned_bytes()?,
        )?;
    }
    // Apply atomically: no selected figure changes if validation fails midway.
    state.apply_figure_project(updated)?;
    Ok(())
}
fn preview(
    ui: &mut egui::Ui,
    state: &DesktopState,
    run: &crate::project::AnalysisRun,
    stale: bool,
    edit: &mut FigureEditor,
) {
    ui.strong("Report preview");
    ui.small("The preview uses the same SVG as the exported figure.");
    if edit.rendered.as_ref() != Some(&(edit.settings.clone(), stale)) {
        let control = JobControl::default();
        let result = (|| {
            state.check_ui_copy(figures::SVG_WORK_BYTES)?;
            let svg = figures::render::svg(run, &edit.key, &edit.settings, stale, &control)?;
            let ratio = edit.settings.width_mm / edit.settings.height_mm;
            let w = (600_f64.min(600. * ratio)).round().max(1.) as u32;
            let h = (w as f64 / ratio).round().max(1.) as u32;
            state.check_ui_copy(
                (w as usize) * (h as usize) * 12 + svg.len() + figures::SVG_WORK_BYTES,
            )?;
            let (w, h, rgba) = figures::render::raster(&svg, w, h, &control)?;
            Ok::<_, crate::error::AppError>(ColorImage::from_rgba_unmultiplied(
                [w as usize, h as usize],
                &rgba,
            ))
        })();
        match result {
            Ok(image) => {
                edit.texture = Some(ui.ctx().load_texture(
                    "report-figure-preview",
                    image,
                    egui::TextureOptions::LINEAR,
                ));
                edit.error.clear();
            }
            Err(e) => {
                edit.texture = None;
                edit.error = e.to_string();
            }
        }
        edit.rendered = Some((edit.settings.clone(), stale));
    }
    if let Some(texture) = &edit.texture {
        ui.add(
            egui::Image::from_texture(texture).fit_to_exact_size(egui::vec2(
                ui.available_width(),
                ui.available_width() * edit.settings.height_mm as f32
                    / edit.settings.width_mm as f32,
            )),
        );
    }
    if stale {
        ui.small("Data changed since this run. This figure uses its original captured results.");
    }
}
