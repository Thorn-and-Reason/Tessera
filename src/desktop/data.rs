use super::{
    button, dialogs,
    state::{MAX_DRAFT_BYTES, TEXT_LIMIT},
    theme, variables, DesktopState, Dialog, UiAction,
};
use crate::{
    data::{Cell, ColumnKind},
    jobs::JobRunner,
};
use eframe::egui::{self, Align2, FontId, Id, Key, Pos2, Rect, Sense, Vec2};
const ROW: f32 = 34.0;
const COLUMN: f32 = 148.0;
const GUTTER: f32 = 44.0;
pub fn show(ui: &mut egui::Ui, state: &mut DesktopState, jobs: &mut JobRunner) {
    ui.horizontal_wrapped(|ui| {
        ui.heading("Data");
        if ui
            .add_enabled(
                !state.active && state.project().dataset().column_count() > 0,
                egui::Button::new("Add row"),
            )
            .clicked()
        {
            state.act(UiAction::AppendRow);
        }
        if ui
            .add_enabled(!state.active, egui::Button::new("Add variable"))
            .clicked()
        {
            state.dialog = Some(Dialog::AddVariable);
        }
        let response = super::toggle(ui, state, "Properties");
        if response.clicked() {
            if !theme::inspector_visible(ui.ctx()) {
                state.dialog = Some(Dialog::Properties);
                state.return_focus = Some(response.id);
            } else {
                state.properties_open = !state.properties_open;
            }
        }
    });
    ui.separator();
    if let Some(edit) = &mut state.cell_editor {
        ui.horizontal(|ui| {
            ui.label(format!("Row {}", edit.row + 1));
            ui.checkbox(&mut edit.missing, "Missing");
        });
        if state.dialog.is_none()
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape))
        {
            state.act(UiAction::CancelCell);
            ui.memory_mut(|m| {
                m.request_focus(Id::new("worksheet-focus"));
                m.set_focus_lock_filter(
                    Id::new("worksheet-focus"),
                    egui::EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        ..Default::default()
                    },
                );
            });
        } else if state.dialog.is_none()
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter))
        {
            state.act(UiAction::MoveCell {
                columns: 0,
                rows: 1,
                edit: true,
            });
        }
    }
    if state.properties_open
        && theme::inspector_visible(ui.ctx())
        && state.dialog != Some(Dialog::Properties)
        && state.project().dataset().column_count() > 0
    {
        egui::Panel::right("data-properties")
            .default_size(theme::inspector_width(ui.ctx()))
            .min_size(230.0)
            .max_size(420.0_f32.min(ui.ctx().content_rect().width() * 0.4))
            .resizable(true)
            .show(ui, |ui| variables::properties(ui, state, jobs));
    }
    exact_value(ui, state);
    grid(ui, state);
}
fn grid(ui: &mut egui::Ui, state: &mut DesktopState) {
    let project = std::sync::Arc::clone(state.project());
    let dataset = project.dataset();
    let count = dataset.column_count();
    let rows = dataset.row_count();
    if count == 0 || rows == 0 {
        ui.add_space(24.0);
        ui.heading("Start your worksheet");
        ui.label("Import CSV, TSV or Excel (.xlsx), or add variables and rows to enter your data.");
        ui.horizontal(|ui| {
            if button(ui, state, "Import data…").clicked() {
                dialogs::import(state);
            }
            if count == 0 {
                if button(ui, state, "Create first variable").clicked() {
                    state.dialog = Some(Dialog::AddVariable);
                }
            } else if button(ui, state, "Create first row").clicked() {
                state.act(UiAction::AppendRow);
            }
        });
        if count == 0 {
            return;
        }
    }
    ui.interact(
        ui.available_rect_before_wrap(),
        Id::new("worksheet-focus"),
        Sense::focusable_noninteractive(),
    );
    let focused = ui.memory(|m| m.has_focus(Id::new("worksheet-focus")));
    if focused {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                Id::new("worksheet-focus"),
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            )
        });
    }
    if state.dialog.is_none() && !state.active {
        if focused && state.cell_editor.is_none() {
            for (key, columns, rows, edit) in [
                (Key::ArrowLeft, -1, 0, false),
                (Key::ArrowRight, 1, 0, false),
                (Key::ArrowUp, 0, -1, false),
                (Key::ArrowDown, 0, 1, false),
                (Key::Enter, 0, 0, true),
            ] {
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key)) {
                    state.act(UiAction::MoveCell {
                        columns,
                        rows,
                        edit,
                    });
                    if !edit && state.dialog.is_none() {
                        ui.memory_mut(|m| {
                            m.move_focus(egui::FocusDirection::None);
                            m.request_focus(Id::new("worksheet-focus"));
                        });
                    }
                }
            }
            if ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy))) {
                copy_selected(ui, state);
            }
            let paste = ui.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Paste(text) => {
                        Some(if text.len() > super::state::MAX_DRAFT_BYTES {
                            Err(crate::error::AppError::invalid(
                                "clipboard",
                                "Paste at most 512 KiB and 4,096 cells at a time.",
                            ))
                        } else {
                            state.check_ui_copy(text.len()).map(|()| text.clone())
                        })
                    }
                    _ => None,
                })
            });
            if let Some(result) = paste {
                match result {
                    Ok(text) => state.act(UiAction::PasteCells(text)),
                    Err(error) => state.fail(error),
                }
            }
        }
        if focused || state.cell_editor.is_some() {
            let backwards = ui.input(|i| i.modifiers.shift);
            if ui.input_mut(|i| {
                i.consume_key(
                    if backwards {
                        egui::Modifiers::SHIFT
                    } else {
                        egui::Modifiers::NONE
                    },
                    Key::Tab,
                )
            }) {
                ui.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
                state.act(UiAction::MoveCell {
                    columns: if backwards { -1 } else { 1 },
                    rows: 0,
                    edit: true,
                });
            }
        }
    }
    let total_width = GUTTER
        + COLUMN * count as f32
        + state
            .column_widths
            .values()
            .map(|width| width - COLUMN)
            .sum::<f32>();
    egui::ScrollArea::both()
        .id_salt("worksheet")
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, viewport| {
            let origin = ui.cursor().min;
            let previous = ui
                .ctx()
                .data(|d| {
                    d.get_temp::<Option<(crate::data::ColumnId, usize)>>(Id::new(
                        "worksheet-last-selection",
                    ))
                })
                .flatten();
            if state.selected_cell != previous {
                ui.ctx().data_mut(|d| {
                    d.insert_temp(Id::new("worksheet-last-selection"), state.selected_cell)
                });
                if let Some((id, row)) = state.selected_cell {
                    let mut x = GUTTER;
                    for column in dataset.columns() {
                        let width = state
                            .column_widths
                            .get(&column.id())
                            .copied()
                            .unwrap_or(COLUMN);
                        if column.id() == id {
                            ui.scroll_to_rect(
                                Rect::from_min_size(
                                    origin + Vec2::new(x - GUTTER, row as f32 * ROW),
                                    Vec2::new(width + GUTTER, ROW * 2.0),
                                ),
                                None,
                            );
                            break;
                        }
                        x += width;
                    }
                }
            }

            ui.set_min_size(Vec2::new(total_width, ROW * (rows + 1) as f32));
            let mut first_col = 0;
            let mut x = GUTTER;
            while first_col < count {
                let width = state
                    .column_widths
                    .get(&dataset.columns()[first_col].id())
                    .copied()
                    .unwrap_or(COLUMN);
                if x + width > viewport.min.x + GUTTER {
                    break;
                }
                x += width;
                first_col += 1;
            }
            let mut visible = Vec::new();
            for index in first_col..count {
                if x > viewport.max.x {
                    break;
                }
                let width = state
                    .column_widths
                    .get(&dataset.columns()[index].id())
                    .copied()
                    .unwrap_or(COLUMN);
                visible.push((index, x, width));
                x += width;
            }
            let first_row = (viewport.min.y / ROW).floor() as usize;
            let last_row = ((viewport.max.y / ROW).ceil() as usize + 1).min(rows);
            let left = origin.x + viewport.min.x;
            let top = origin.y + viewport.min.y;
            let body_clip = ui.clip_rect().intersect(Rect::from_min_max(
                Pos2::new(left + GUTTER, top + ROW),
                ui.clip_rect().max,
            ));
            let mut cells = ui.new_child(egui::UiBuilder::new().max_rect(ui.max_rect()));
            cells.set_clip_rect(body_clip);
            for row in first_row..last_row {
                for &(col_index, x, width) in &visible {
                    let column = &dataset.columns()[col_index];
                    let rect = Rect::from_min_size(
                        origin + Vec2::new(x, (row + 1) as f32 * ROW),
                        Vec2::new(width, ROW),
                    );
                    if !rect.intersects(body_clip) {
                        continue;
                    }
                    let is_edit = state
                        .cell_editor
                        .as_ref()
                        .is_some_and(|e| e.column == column.id() && e.row == row);
                    if is_edit {
                        if let Some(edit) = &mut state.cell_editor {
                            let response = cells.put(
                                rect.shrink(2.0),
                                egui::TextEdit::singleline(&mut edit.text)
                                    .id(Id::new("cell-editor"))
                                    .char_limit(TEXT_LIMIT)
                                    .font(egui::TextStyle::Monospace),
                            );
                            if edit.focus {
                                response.request_focus();
                                edit.focus = false;
                            }
                            if response.has_focus() {
                                cells.memory_mut(|m| {
                                    m.set_focus_lock_filter(
                                        response.id,
                                        egui::EventFilter {
                                            tab: true,
                                            horizontal_arrows: true,
                                            vertical_arrows: true,
                                            ..Default::default()
                                        },
                                    )
                                });
                            }
                            if response.changed() {
                                edit.missing = false;
                            }
                            state.controls.insert("Cell editor".into(), response.rect);
                        }
                    } else {
                        if state.selected_cell == Some((column.id(), row)) {
                            cells.painter().rect_filled(rect, 0.0, theme::selection(ui));
                            cells.painter().rect_stroke(
                                rect.shrink(1.0),
                                0.0,
                                ui.visuals().selection.stroke,
                                egui::StrokeKind::Inside,
                            );
                        }
                        let text = match column.kind() {
                            ColumnKind::Text => column
                                .text_value(row)
                                .ok()
                                .flatten()
                                .map(|s| s.chars().take(120).collect())
                                .unwrap_or_else(|| "·".into()),
                            ColumnKind::Number => match column.cell(row) {
                                Ok(Cell::Number(v)) => format!(
                                    "{:.*}",
                                    usize::from(column.metadata().display_decimals),
                                    v
                                ),
                                _ => "·".into(),
                            },
                        };
                        let value_rect = rect.shrink2(Vec2::new(6.0, 2.0));
                        let numeric = column.kind() == ColumnKind::Number;
                        cells.painter().with_clip_rect(value_rect).text(
                            if numeric {
                                value_rect.right_center()
                            } else {
                                value_rect.left_center()
                            },
                            if numeric {
                                Align2::RIGHT_CENTER
                            } else {
                                Align2::LEFT_CENTER
                            },
                            text,
                            FontId::monospace(12.0),
                            theme::text(ui),
                        );
                        let response = cells.interact(
                            value_rect,
                            Id::new(("worksheet-cell", column.id(), row)),
                            Sense::click(),
                        );
                        state
                            .controls
                            .insert(format!("Cell {}:{}", column.id().0, row), response.rect);
                        let value = match column.kind() {
                            ColumnKind::Text => match column.text_value(row).ok().flatten() {
                                None => "Missing".into(),
                                Some("") => "Empty text".into(),
                                Some(text) => theme::caption_limit(text, 128),
                            },
                            ColumnKind::Number => match column.cell(row) {
                                Ok(Cell::Number(value)) => value.to_string(),
                                _ => "Missing".into(),
                            },
                        };
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                true,
                                theme::caption(format_args!(
                                    "{}, row {}: {}",
                                    theme::caption_limit(column.name(), 64),
                                    row + 1,
                                    theme::caption_limit(&value, 128)
                                )),
                            )
                        });
                        if response.clicked() {
                            cells.memory_mut(|m| {
                                m.request_focus(Id::new("worksheet-focus"));
                                m.set_focus_lock_filter(
                                    Id::new("worksheet-focus"),
                                    egui::EventFilter {
                                        tab: true,
                                        horizontal_arrows: true,
                                        vertical_arrows: true,
                                        ..Default::default()
                                    },
                                );
                            });
                            state.act(UiAction::SelectCell {
                                column: column.id(),
                                row,
                            });
                        }
                        if (response.double_clicked()
                            || (response.has_focus() && cells.input(|i| i.key_pressed(Key::Enter))))
                            && !state.active
                            && state.dialog.is_none()
                            && state.selected_cell == Some((column.id(), row))
                        {
                            state.act(UiAction::BeginCell {
                                column: column.id(),
                                row,
                            });
                        }
                    }
                    cells.painter().line_segment(
                        [rect.left_bottom(), rect.right_bottom()],
                        egui::Stroke::new(0.5, theme::line(ui)),
                    );
                    cells.painter().line_segment(
                        [rect.right_top(), rect.right_bottom()],
                        egui::Stroke::new(0.5, theme::line(ui)),
                    );
                }
            }
            let painter = ui.painter();
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(left, top),
                    Pos2::new(left + GUTTER, ui.clip_rect().max.y),
                ),
                0.0,
                theme::surface(ui),
            );
            for row in first_row..last_row {
                let y = origin.y + (row + 1) as f32 * ROW;
                if y + ROW > top + ROW {
                    painter
                        .with_clip_rect(Rect::from_min_max(
                            Pos2::new(left, top + ROW),
                            ui.clip_rect().max,
                        ))
                        .text(
                            Pos2::new(left + GUTTER / 2.0, y + ROW / 2.0),
                            Align2::CENTER_CENTER,
                            (row + 1).to_string(),
                            FontId::proportional(12.0),
                            theme::text(ui),
                        );
                }
            }
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(left, top),
                    Pos2::new(ui.clip_rect().max.x, top + ROW),
                ),
                0.0,
                theme::surface(ui),
            );
            let header_clip = Rect::from_min_max(
                Pos2::new(left + GUTTER, top),
                Pos2::new(ui.clip_rect().max.x, top + ROW),
            );
            let mut headers = ui.new_child(egui::UiBuilder::new().max_rect(header_clip));
            headers.set_clip_rect(header_clip);
            for &(index, x, width) in &visible {
                let column = &dataset.columns()[index];
                let rect = Rect::from_min_size(Pos2::new(origin.x + x, top), Vec2::new(width, ROW));
                let response = headers.put(
                    rect,
                    egui::Button::new(theme::caption(column.name()))
                        .selected(state.selected_column == Some(column.id())),
                );
                let handle = headers
                    .interact(
                        Rect::from_min_max(
                            rect.right_top() - Vec2::new(4.0, 0.0),
                            rect.right_bottom() + Vec2::new(4.0, 0.0),
                        ),
                        Id::new(("column-width", column.id())),
                        Sense::drag(),
                    )
                    .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
                if handle.dragged() {
                    let width = (rect.width() + handle.drag_delta().x).clamp(80.0, 480.0);
                    if let Err(error) = state.set_column_width(column.id(), width) {
                        state.fail(error);
                    }
                }
                if response.clicked() {
                    state.act(UiAction::SelectColumn(column.id()));
                }
            }
            ui.interact(
                Rect::from_min_size(Pos2::new(left, top), Vec2::new(GUTTER, ROW)),
                Id::new("row-number-header"),
                Sense::hover(),
            )
            .on_hover_text("Row");
        });
}

fn exact_value(ui: &mut egui::Ui, state: &DesktopState) {
    let Some((id, row)) = state.selected_cell else {
        return;
    };
    let Some(column) = state
        .project()
        .dataset()
        .columns()
        .iter()
        .find(|c| c.id() == id)
    else {
        return;
    };
    const INSPECTION_LIMIT: usize = MAX_DRAFT_BYTES;
    egui::CollapsingHeader::new(format!(
        "Inspect cell · {} · Row {}",
        theme::caption(column.name()),
        row + 1
    ))
    .id_salt("exact-cell-value")
    .show(ui, |ui| {
        let text = if column.kind() == ColumnKind::Text {
            match column.text_value(row) {
                Ok(Some(value)) if value.len() <= INSPECTION_LIMIT => {
                    if let Err(error) = state.check_ui_copy(value.len().saturating_mul(3)) {
                        ui.label(theme::caption(error));
                        return;
                    }
                    value.to_owned()
                }
                Ok(Some(_)) => {
                    ui.label("This value is too large to inspect or copy here (limit: 512 KB).");
                    return;
                }
                Ok(None) => {
                    ui.label("Missing value");
                    return;
                }
                Err(error) => {
                    ui.label(theme::caption(error));
                    return;
                }
            }
        } else {
            if let Err(error) = state.check_ui_copy(128) {
                ui.label(theme::caption(error));
                return;
            }
            match column.cell(row) {
                Ok(Cell::Number(value)) => value.to_string(),
                _ => {
                    ui.label("Missing value");
                    return;
                }
            }
        };
        if ui.button("Copy exact value").clicked() {
            ui.ctx().copy_text(text.clone());
        }
        if text.is_empty() {
            ui.label("Empty text (not missing)");
        }
        egui::ScrollArea::vertical()
            .max_height(140.0)
            .show(ui, |ui| {
                let display: String = text.chars().take(TEXT_LIMIT).collect();
                if text.chars().count() > TEXT_LIMIT {
                    ui.label("Preview shortened. Copy exact value to inspect the complete text.");
                }
                let mut value = display.as_str();
                ui.add(
                    egui::TextEdit::multiline(&mut value)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace),
                );
            });
    });
}

fn copy_selected(ui: &egui::Ui, state: &mut DesktopState) {
    let Some((id, row)) = state.selected_cell else {
        return;
    };
    let project = std::sync::Arc::clone(state.project());
    let Ok(column) = project.dataset().column(id) else {
        return;
    };
    let result = if column.kind() == ColumnKind::Text {
        match column.text_value(row) {
            Ok(Some(value)) if value.len() <= MAX_DRAFT_BYTES => {
                state.check_ui_copy(value.len()).map(|()| value.to_owned())
            }
            Ok(Some(_)) => Err(crate::error::AppError::SizeLimit {
                limit: MAX_DRAFT_BYTES,
            }),
            Ok(None) => Ok(String::new()),
            Err(error) => Err(error),
        }
    } else {
        state
            .check_ui_copy(128)
            .and_then(|()| column.cell(row))
            .map(|cell| match cell {
                Cell::Number(value) => value.to_string(),
                _ => String::new(),
            })
    };
    match result {
        Ok(value) => ui.ctx().copy_text(value),
        Err(error) => state.fail(error),
    }
}
