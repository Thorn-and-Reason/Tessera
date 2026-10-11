use super::{
    button, start,
    state::{MetadataEditor, MAX_DRAFT_BYTES, TEXT_LIMIT},
    DesktopState, Dialog, UiAction,
};
use crate::{
    data::{Cell, ColumnKind, Measurement, MissingRules, ValueLabel},
    error::AppError,
    jobs::{JobOperation, JobRunner},
};
use eframe::egui;
use std::sync::Arc;
pub fn show(ui: &mut egui::Ui, state: &mut DesktopState, jobs: &mut JobRunner) {
    let wide = super::theme::inspector_visible(ui.ctx());
    ui.horizontal(|ui| {
        ui.heading("Variables");
        if ui
            .add_enabled(!state.active, egui::Button::new("Add variable"))
            .clicked()
        {
            state.dialog = Some(Dialog::AddVariable);
        }
        let response = super::toggle(ui, state, "Properties");
        if response.clicked() {
            if wide {
                state.properties_open = !state.properties_open;
            } else {
                state.dialog = Some(Dialog::Properties);
                state.return_focus = Some(response.id);
            }
        }
    });
    if wide
        && state.properties_open
        && !state.project().dataset().columns().is_empty()
        && state.dialog != Some(Dialog::Properties)
    {
        egui::Panel::right("variable-properties")
            .default_size(super::theme::inspector_width(ui.ctx()))
            .size_range(240.0..=420.0)
            .resizable(true)
            .show(ui, |ui| properties(ui, state, jobs));
    }
    let search_label = ui.label("Find a variable");
    ui.add(
        egui::TextEdit::singleline(&mut state.variable_search)
            .hint_text("Search names and descriptions")
            .char_limit(256),
    )
    .labelled_by(search_label.id);
    let project = Arc::clone(state.project());
    let columns = project.dataset().columns();
    if columns.is_empty() {
        ui.heading("Define your variables");
        ui.label("Add a variable or import CSV/TSV data to describe your dataset.");
        ui.horizontal(|ui| {
            if button(ui, state, "Import data…").clicked() {
                super::dialogs::import(state);
            }
            if button(ui, state, "Create first variable").clicked() {
                state.dialog = Some(Dialog::AddVariable);
            }
        });
        return;
    }
    let query = match super::theme::NameQuery::new(state.variable_search.trim()) {
        Ok(query) => query,
        Err(error) => {
            state.fail(error);
            return;
        }
    };
    let matches = |column: &&crate::data::Column| {
        query.matches(column.name()) || query.matches(&column.metadata().description)
    };
    let count = columns.iter().filter(matches).count();
    if count == 0 {
        ui.label("No variables match your search.");
        return;
    }
    egui::ScrollArea::both().show(ui, |ui| {
        let width = ui.available_width().max(540.0);
        let widths = [width * 0.27, 75.0, 110.0, (width * 0.73 - 185.0).max(170.0)];
        ui.horizontal(|ui| {
            for (heading, width) in ["Name", "Type", "Measurement", "Description"]
                .into_iter()
                .zip(widths)
            {
                ui.add_sized(
                    [width, 24.0],
                    egui::Label::new(egui::RichText::new(heading).strong()),
                );
            }
        });
        egui::ScrollArea::vertical().show_rows(ui, 40.0, count, |ui, range| {
            for column in columns
                .iter()
                .filter(matches)
                .skip(range.start)
                .take(range.len())
            {
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(width + 24.0, 36.0), egui::Sense::click());
                if state.selected_column == Some(column.id()) {
                    ui.painter()
                        .rect_filled(rect, 5.0, super::theme::selection(ui));
                } else if response.hovered() {
                    ui.painter()
                        .rect_filled(rect, 5.0, super::theme::surface(ui));
                }
                if response.has_focus() {
                    ui.painter().rect_stroke(
                        rect,
                        5.0,
                        ui.visuals().selection.stroke,
                        egui::StrokeKind::Inside,
                    );
                }
                let texts = [
                    super::theme::caption(column.name()),
                    kind_label(column.kind()).into(),
                    measurement_label(column.metadata().measurement).into(),
                    super::theme::caption(&column.metadata().description),
                ];
                let mut x = rect.left() + 8.0;
                for (text, w) in texts.iter().zip(widths) {
                    let area = egui::Rect::from_min_size(
                        egui::pos2(x, rect.top()),
                        egui::vec2(w, rect.height()),
                    );
                    ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
                        ui.add(egui::Label::new(text).truncate().selectable(false));
                    });
                    x += w + ui.spacing().item_spacing.x;
                }
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::SelectableLabel,
                        true,
                        state.selected_column == Some(column.id()),
                        format!("Variable {}, {}, {}", texts[0], texts[1], texts[2]),
                    )
                });
                if response.clicked() {
                    state.act(UiAction::SelectColumn(column.id()));
                }
                if response.double_clicked() && state.dialog != Some(Dialog::PropertyChanges) {
                    if wide {
                        state.properties_open = true;
                    } else {
                        state.dialog = Some(Dialog::Properties);
                        state.return_focus = Some(response.id);
                    }
                }
            }
        });
    });
}
fn measurement_label(measurement: Measurement) -> &'static str {
    match measurement {
        Measurement::Unknown => "Not set",
        Measurement::Nominal => "Nominal",
        Measurement::Ordinal => "Ordinal",
        Measurement::Scale => "Scale",
    }
}
pub fn kind_label(kind: ColumnKind) -> &'static str {
    match kind {
        ColumnKind::Number => "Number",
        ColumnKind::Text => "Text",
    }
}
pub fn kind_picker(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    kind: &mut ColumnKind,
) {
    ui.vertical(|ui| {
        let label = ui.label("Type");
        let response = egui::ComboBox::from_id_salt(format!("{id:?}"))
            .selected_text(kind_label(*kind))
            .show_ui(ui, |ui| {
                ui.selectable_value(kind, ColumnKind::Number, "Number");
                ui.selectable_value(kind, ColumnKind::Text, "Text");
            })
            .response;
        response.labelled_by(label.id);
    });
}
pub fn properties(ui: &mut egui::Ui, state: &mut DesktopState, jobs: &mut JobRunner) {
    ui.heading("Properties");
    let Some(id) = state.selected_column else {
        ui.label("Select a variable to edit its name, description and value settings.");
        return;
    };
    if state
        .metadata_editor
        .as_ref()
        .is_none_or(|e| e.column != id)
    {
        match state.project().dataset().column(id).and_then(|column| {
            let bytes = column.metadata().heap_bytes()?;
            if bytes > MAX_DRAFT_BYTES {
                return Err(AppError::SizeLimit {
                    limit: MAX_DRAFT_BYTES,
                });
            }
            state.check_ui_copy(bytes)?;
            Ok(MetadataEditor {
                column: id,
                metadata: column.metadata().clone(),
                kind: column.kind(),
            })
        }) {
            Ok(editor) => state.metadata_editor = Some(editor),
            Err(error) => {
                state.fail(error);
                return;
            }
        }
    }
    let project = Arc::clone(state.project());
    if let Ok(column) = project.dataset().column(id) {
        ui.label(super::theme::caption(column.name()));
    }
    if state.metadata_has_changes() {
        ui.colored_label(ui.visuals().warn_fg_color, "Unapplied changes");
    }
    let mut draft = state.metadata_editor.take();

    let mut convert = None;
    egui::ScrollArea::vertical()
        .id_salt("properties-fields")
        .max_height((ui.available_height() - 110.0).max(80.0))
        .show(ui, |ui| {
            ui.add_enabled_ui(!state.active, |ui| {
                let Some(editor) = &mut draft else {
                    return;
                };
                let name_label = ui.label("Name");
                ui.add(
                    egui::TextEdit::singleline(&mut editor.metadata.name)
                        .id(egui::Id::new(("property-name", id)))
                        .char_limit(TEXT_LIMIT),
                )
                .labelled_by(name_label.id);
                let description_label = ui.label("Description");
                ui.add(
                    egui::TextEdit::multiline(&mut editor.metadata.description)
                        .id(egui::Id::new(("property-description", id)))
                        .desired_rows(2)
                        .char_limit(TEXT_LIMIT),
                )
                .labelled_by(description_label.id);
                kind_picker(ui, "variable-type", &mut editor.kind);
                let actual_kind = project
                    .dataset()
                    .column(id)
                    .map_or(editor.kind, |c| c.kind());
                if editor.kind != actual_kind && ui.button("Convert type").clicked() {
                    convert = Some(editor.kind);
                }
                let measurement_label_response = ui.label("Measurement");
                egui::ComboBox::from_id_salt("variable-measurement")
                    .selected_text(measurement_label(editor.metadata.measurement))
                    .show_ui(ui, |ui| {
                        for value in [
                            Measurement::Unknown,
                            Measurement::Nominal,
                            Measurement::Ordinal,
                            Measurement::Scale,
                        ] {
                            ui.selectable_value(
                                &mut editor.metadata.measurement,
                                value,
                                measurement_label(value),
                            );
                        }
                    })
                    .response
                    .labelled_by(measurement_label_response.id);
                let label = ui.label("Display decimals");
                ui.add(egui::DragValue::new(&mut editor.metadata.display_decimals).range(0..=16))
                    .labelled_by(label.id);
                ui.collapsing("Missing values", |ui| {
                    missing_rules(ui, id, actual_kind, &mut editor.metadata.missing_rules)
                });
                ui.collapsing("Value labels", |ui| {
                    labels(ui, id, actual_kind, &mut editor.metadata.value_labels)
                });
            });
        });
    let apply = ui
        .add_enabled(!state.active, super::theme::primary(ui, "Apply properties"))
        .clicked();
    if let Some(editor) = &draft {
        if editor
            .metadata
            .heap_bytes()
            .map_or(true, |bytes| bytes > MAX_DRAFT_BYTES)
        {
            state.metadata_editor = draft;
            state.fail(AppError::SizeLimit {
                limit: MAX_DRAFT_BYTES,
            });
            return;
        }
    }
    state.metadata_editor = draft;
    if apply {
        if let Some(editor) = &state.metadata_editor {
            match editor
                .metadata
                .heap_bytes()
                .and_then(|bytes| state.check_ui_copy(bytes))
            {
                Ok(()) => state.act(UiAction::Metadata {
                    column: id,
                    metadata: editor.metadata.clone(),
                }),
                Err(error) => state.fail(error),
            }
        }
    }
    if let Some(kind) = convert {
        if state.metadata_editor.as_ref().is_some_and(|e| {
            project
                .dataset()
                .column(id)
                .is_ok_and(|c| c.metadata() != &e.metadata)
        }) {
            state.fail(AppError::invalid(
                "type",
                "Apply property changes before converting the variable.",
            ));
            return;
        }
        start(
            state,
            jobs,
            JobOperation::ConvertColumn {
                project: Arc::clone(state.project()),
                id,
                kind,
            },
        );
    }
}
fn missing_rules(
    ui: &mut egui::Ui,
    column: crate::data::ColumnId,
    kind: ColumnKind,
    rules: &mut MissingRules,
) {
    let mut enabled = !matches!(rules, MissingRules::None);
    if ui.checkbox(&mut enabled, "Define missing values").changed() {
        *rules = if enabled {
            match kind {
                ColumnKind::Number => MissingRules::Number {
                    values: vec![],
                    ranges: vec![],
                },
                ColumnKind::Text => MissingRules::Text(vec![]),
            }
        } else {
            MissingRules::None
        };
    }
    match rules {
        MissingRules::None => {}
        MissingRules::Number { values, ranges } => {
            let mut remove = None;
            for (index, value) in values.iter_mut().enumerate() {
                ui.push_id(("missing-number", index), |ui| {
                    let field_label = ui.label(format!("Missing number {}", index + 1));
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(value)
                                .custom_formatter(|value, _| value.to_string()),
                        )
                        .labelled_by(field_label.id);
                        if remove_button(ui, format!("Remove missing number {}", index + 1)) {
                            remove = Some(index);
                        }
                    })
                });
            }
            if let Some(index) = remove {
                values.remove(index);
            }
            if values.len() < 256 && ui.button("Add missing number").clicked() {
                values.push(0.0);
            }
            let mut remove = None;
            for (index, (lo, hi)) in ranges.iter_mut().enumerate() {
                ui.push_id(("missing-range", index), |ui| {
                    let low_label = ui.label(format!("Range {} lower bound", index + 1));
                    let high_label = ui.label(format!("Range {} upper bound", index + 1));
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(lo).custom_formatter(|value, _| value.to_string()),
                        )
                        .labelled_by(low_label.id);
                        ui.label("to");
                        ui.add(
                            egui::DragValue::new(hi).custom_formatter(|value, _| value.to_string()),
                        )
                        .labelled_by(high_label.id);
                        if remove_button(ui, format!("Remove missing range {}", index + 1)) {
                            remove = Some(index);
                        }
                    })
                });
            }
            if let Some(index) = remove {
                ranges.remove(index);
            }
            if ranges.len() < 256 && ui.button("Add missing range").clicked() {
                ranges.push((0.0, 0.0));
            }
        }
        MissingRules::Text(values) => {
            let mut remove = None;
            for (index, value) in values.iter_mut().enumerate() {
                ui.push_id(("missing-text", index), |ui| {
                    let field_label = ui.label(format!("Missing text {}", index + 1));
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(value)
                                .id(egui::Id::new(("property-missing-text", column, index)))
                                .char_limit(256)
                                .desired_width(130.0),
                        )
                        .labelled_by(field_label.id);
                        if remove_button(ui, format!("Remove missing text {}", index + 1)) {
                            remove = Some(index);
                        }
                    })
                });
            }
            if let Some(index) = remove {
                values.remove(index);
            }
            if values.len() < 256 && ui.button("Add missing text").clicked() {
                values.push(String::new());
            }
        }
    }
}
fn labels(
    ui: &mut egui::Ui,
    column: crate::data::ColumnId,
    kind: ColumnKind,
    labels: &mut Vec<ValueLabel>,
) {
    let mut remove = None;
    for (index, label) in labels.iter_mut().enumerate() {
        ui.push_id(("value-label", index), |ui| {
            let value_label = ui.label(format!("Value {}", index + 1));
            ui.horizontal(|ui| {
                match &mut label.value {
                    Cell::Number(value) => {
                        ui.add(
                            egui::DragValue::new(value)
                                .custom_formatter(|value, _| value.to_string()),
                        )
                        .labelled_by(value_label.id);
                    }
                    Cell::Text(value) => {
                        ui.add(
                            egui::TextEdit::singleline(value)
                                .id(egui::Id::new(("property-label-value", column, index)))
                                .char_limit(256)
                                .desired_width(70.0),
                        )
                        .labelled_by(value_label.id);
                    }
                    Cell::Missing => {}
                }
                if remove_button(ui, format!("Remove value label {}", index + 1)) {
                    remove = Some(index);
                }
            });
            let label_label = ui.label(format!("Label {}", index + 1));
            ui.add(
                egui::TextEdit::singleline(&mut label.label)
                    .id(egui::Id::new(("property-label-text", column, index)))
                    .hint_text("Label")
                    .char_limit(256),
            )
            .labelled_by(label_label.id);
        });
    }
    if let Some(index) = remove {
        labels.remove(index);
    }
    if labels.len() < 256 && ui.button("Add value label").clicked() {
        labels.push(ValueLabel {
            value: match kind {
                ColumnKind::Number => Cell::Number(0.0),
                ColumnKind::Text => Cell::Text(String::new()),
            },
            label: String::new(),
        });
    }
}

fn remove_button(ui: &mut egui::Ui, label: String) -> bool {
    let response = ui.small_button("Remove");
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &label)
    });
    response.clicked()
}

#[cfg(test)]
mod tests {
    use super::super::theme::NameQuery;
    #[test]
    fn apply_button_commits_metadata_and_retains_the_selected_conversion_type() {
        use crate::data::{Column, ColumnId, ColumnKind, Dataset};
        use eframe::egui::{self, Event, Modifiers, PointerButton, Pos2};
        let project = crate::project::Project::new(
            "Draft",
            Dataset::new(vec![
                Column::numeric(ColumnId(1), "Score", vec![Some(1.0)]).unwrap()
            ])
            .unwrap(),
        )
        .unwrap();
        let mut state = super::DesktopState::from_project(project)
            .unwrap()
            .apply_action(super::UiAction::SelectColumn(ColumnId(1)))
            .unwrap();
        let mut metadata = state
            .project()
            .dataset()
            .column(ColumnId(1))
            .unwrap()
            .metadata()
            .clone();
        metadata.name = "Edited score".into();
        state.metadata_editor = Some(super::MetadataEditor {
            column: ColumnId(1),
            metadata,
            kind: ColumnKind::Text,
        });
        let mut jobs = crate::jobs::JobRunner::default();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        {
            let mut frame = |events| {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        events,
                        screen_rect: Some(egui::Rect::from_min_size(
                            Pos2::ZERO,
                            egui::vec2(560.0, 780.0),
                        )),
                        ..Default::default()
                    },
                    |ui| super::properties(ui, &mut state, &mut jobs),
                );
                output.textures_delta.clear();
                output
            };
            let output = frame(vec![]);
            let bounds = output
                .platform_output
                .accesskit_update
                .unwrap()
                .nodes
                .into_iter()
                .find(|(_, node)| {
                    node.role() == egui::accesskit::Role::Button
                        && node.label() == Some("Apply properties")
                })
                .expect("Apply properties must be reachable")
                .1
                .bounds()
                .unwrap();
            let pos = Pos2::new(
                ((bounds.x0 + bounds.x1) / 2.0) as f32,
                ((bounds.y0 + bounds.y1) / 2.0) as f32,
            );
            for pressed in [true, false] {
                frame(vec![
                    Event::PointerMoved(pos),
                    Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    },
                ]);
            }
        }
        assert_eq!(
            state
                .project()
                .dataset()
                .column(ColumnId(1))
                .unwrap()
                .name(),
            "Edited score"
        );
        assert_eq!(
            state
                .project()
                .dataset()
                .column(ColumnId(1))
                .unwrap()
                .kind(),
            ColumnKind::Number
        );
        let editor = state
            .metadata_editor
            .as_ref()
            .expect("Apply button discarded the selected conversion type");
        assert_eq!(editor.kind, ColumnKind::Text);
        assert_eq!(editor.metadata.name, "Edited score");
        assert!(state.metadata_has_changes());
        assert!(!jobs.is_running());
    }
    #[test]
    fn warm_properties_frame_preserves_the_admitted_editor_buffers() {
        use crate::data::{Column, ColumnId, Dataset};
        let project = crate::project::Project::new(
            "Draft",
            Dataset::new(vec![Column::numeric(ColumnId(1), "Score", vec![]).unwrap()]).unwrap(),
        )
        .unwrap();
        let mut state = super::DesktopState::from_project(project)
            .unwrap()
            .apply_action(super::UiAction::SelectColumn(ColumnId(1)))
            .unwrap();
        let mut jobs = crate::jobs::JobRunner::default();
        let ctx = eframe::egui::Context::default();
        let mut frame = |state: &mut super::DesktopState| {
            let mut output = ctx.run_ui(Default::default(), |ui| {
                super::properties(ui, state, &mut jobs)
            });
            output.textures_delta.clear();
        };
        frame(&mut state);
        let pointer = state
            .metadata_editor
            .as_ref()
            .unwrap()
            .metadata
            .name
            .as_ptr();
        frame(&mut state);
        assert_eq!(
            state
                .metadata_editor
                .as_ref()
                .unwrap()
                .metadata
                .name
                .as_ptr(),
            pointer,
            "an unchanged frame must reuse the admitted property buffer"
        );
    }
    #[test]
    fn property_typing_keeps_focus_when_unapplied_status_appears() {
        use crate::data::{Column, ColumnId, Dataset};
        use eframe::egui::{self, Event, Modifiers, PointerButton, Pos2};
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let project = crate::project::Project::new(
            "Draft",
            Dataset::new(vec![Column::numeric(ColumnId(1), "Score", vec![]).unwrap()]).unwrap(),
        )
        .unwrap();
        let mut state = super::DesktopState::from_project(project)
            .unwrap()
            .apply_action(super::UiAction::SelectColumn(ColumnId(1)))
            .unwrap();
        let mut jobs = crate::jobs::JobRunner::default();
        {
            let mut frame = |events| {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        events,
                        screen_rect: Some(egui::Rect::from_min_size(
                            Pos2::ZERO,
                            egui::vec2(560.0, 780.0),
                        )),
                        ..Default::default()
                    },
                    |ui| super::properties(ui, &mut state, &mut jobs),
                );
                output.textures_delta.clear();
                output
            };
            let output = frame(vec![]);
            let bounds = output
                .platform_output
                .accesskit_update
                .unwrap()
                .nodes
                .into_iter()
                .find(|(_, node)| {
                    node.role() == egui::accesskit::Role::TextInput && node.value() == Some("Score")
                })
                .unwrap()
                .1
                .bounds()
                .unwrap();
            let position = Pos2::new(
                (bounds.x1 - 5.0) as f32,
                ((bounds.y0 + bounds.y1) / 2.0) as f32,
            );
            for pressed in [true, false] {
                frame(vec![
                    Event::PointerMoved(position),
                    Event::PointerButton {
                        pos: position,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    },
                ]);
            }
            frame(vec![Event::Text("X".into())]);
            frame(vec![Event::Text("Y".into())]);
        }
        let name = &state.metadata_editor.as_ref().unwrap().metadata.name;
        assert!(
            name.contains("XY"),
            "typing lost focus after status insertion: {name}"
        );
    }
    #[test]
    fn type_label_precedes_control_vertically() {
        let ctx = eframe::egui::Context::default();
        ctx.enable_accesskit();
        let mut kind = crate::data::ColumnKind::Number;
        let mut output = ctx.run_ui(Default::default(), |ui| {
            super::kind_picker(ui, "type-layout", &mut kind)
        });
        output.textures_delta.clear();
        let update = output.platform_output.accesskit_update.unwrap();
        let label = update
            .nodes
            .iter()
            .find(|(_, node)| node.label().or_else(|| node.value()) == Some("Type"))
            .unwrap()
            .1
            .bounds()
            .unwrap();
        let control = update
            .nodes
            .iter()
            .find(|(_, node)| node.label().or_else(|| node.value()) == Some("Number"))
            .unwrap()
            .1
            .bounds()
            .unwrap();
        assert!(
            label.y1 <= control.y0,
            "label {label:?}, control {control:?}"
        );
    }
    #[test]
    fn search_finds_unicode_and_text_beyond_display_caption() {
        assert!(NameQuery::new("sgôr ŵ").unwrap().matches("SGÔR ŵ"));
        assert!(NameQuery::new("needle")
            .unwrap()
            .matches(&format!("{} needle", "x".repeat(10000))));
        assert!(!NameQuery::new("different").unwrap().matches("Score"));
    }
}
