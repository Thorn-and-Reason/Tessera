//! Project actions and responsive workflow navigation.
use super::{
    button, dialogs, experience, procedure_forms, theme, DesktopState, Dialog, PendingAction,
    UiAction, Workspace,
};
use crate::jobs::JobRunner;
use eframe::egui::{self, Key, Modifiers};

pub(super) fn show(ui: &mut egui::Ui, state: &mut DesktopState, jobs: &mut JobRunner) {
    egui::Panel::top("project-bar")
        .exact_size(44.0)
        .frame(
            egui::Frame::new()
                .fill(theme::surface(ui))
                .inner_margin(egui::Margin::symmetric(12, 4)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_min_height(34.);
                ui.spacing_mut().item_spacing.x = 6.;
                ui.spacing_mut().button_padding = egui::vec2(8., 4.);
                if ui.available_width() >= 700. { theme::wordmark(ui, "Tessera"); ui.separator(); }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if button(ui, state, "Import").clicked() { dialogs::import(state); }
                    if ui
                        .add_enabled(
                            !state.active,
                            theme::secondary(ui, "Save")
                                .min_size(egui::vec2(64., 28.)),
                        )
                        .clicked()
                    {
                        dialogs::save(state, jobs, false);
                    }
                    if button(ui, state, "Open").clicked() {
                        dialogs::open(state);
                    }
                    let help = ui.menu_button("Help", |ui| {
                        let mut opened = false;
                        if button(ui, state, "User guide").clicked() {
                            state.dialog = Some(Dialog::UserGuide);
                            opened = true;
                            ui.close();
                        }
                        let response = button(ui, state, "About Tessera");
                        if response.clicked() {
                            state.dialog = Some(Dialog::About);
                            opened = true;
                            ui.close();
                        }
                        opened
                    });
                    if help.inner == Some(true) {
                        state.return_focus = Some(help.response.id);
                    }
                    procedure_forms::record(state, "Help", &help.response);
                    if state.dialog.is_none() && state.return_focus == Some(help.response.id) {
                        ui.ctx().memory_mut(|memory| memory.request_focus(help.response.id));
                        state.return_focus = None;
                    }
                    let view = ui.menu_button("View", |ui| {
                        ui.label("Analysis detail");
                        experience::mode_choices(ui, state);
                        ui.separator();
                        ui.strong("Appearance");
                        let mut preference = ui.ctx().options(|o| o.theme_preference);
                        preference.radio_buttons(ui);
                        ui.ctx().set_theme(preference);
                        ui.separator();
                        let label = ui.label("Text and interface size");
                        let mut zoom = f64::from(ui.ctx().zoom_factor());
                        let size = ui.add(egui::Slider::new(&mut zoom, 0.85..=1.5).step_by(0.05).suffix("×")).labelled_by(label.id);
                        size.widget_info(|| egui::WidgetInfo::slider(size.enabled(), zoom as f64, "Text and interface size"));
                        procedure_forms::record(state, "Text and interface size", &size);
                        if size.changed() {
                            ui.ctx().set_zoom_factor(zoom as f32);
                        }
                        if ui.button("Reset size").clicked() { ui.ctx().set_zoom_factor(1.); }
                        ui.separator();
                        ui.label(format!("Run analysis: {}", ui.ctx().format_shortcut(&egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::Enter))));
                        ui.small("In Analyse. In Data, use arrows to move, Enter to edit, Tab to advance, and Escape to cancel.");
                        ui.small("Paste TSV into existing cells: up to 4,096 cells / 512 KiB. Blank numbers become missing; blank text stays empty.");
                    });
                    procedure_forms::record(state, "View", &view.response);
                    ui.menu_button("Edit", |ui| {
                        if ui.add_enabled(state.can_undo(), egui::Button::new("Undo").shortcut_text(ui.ctx().format_shortcut(&egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::Z)))).clicked() { state.act(UiAction::Undo); ui.close(); }
                        if ui.add_enabled(state.can_redo(), egui::Button::new("Redo").shortcut_text(ui.ctx().format_shortcut(&egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)))).clicked() { state.act(UiAction::Redo); ui.close(); }
                        ui.separator();
                        if ui.add_enabled(!state.active, egui::Button::new("Prepare data…")).clicked() { state.act(UiAction::Navigate(Workspace::Prepare)); ui.close(); }
                    });
                    ui.menu_button("File", |ui| {
                        ui.small("For a different destination, use a new filename. Exports never replace existing files.");
                        if ui.add(egui::Button::new("New project").shortcut_text(ui.ctx().format_shortcut(&egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::N)))).clicked() {
                            state.act(UiAction::Request(PendingAction::New));
                            ui.close();
                        }
                        if ui.add(egui::Button::new("Open…").shortcut_text(ui.ctx().format_shortcut(&egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::O)))).clicked() {
                            dialogs::open(state);
                            ui.close();
                        }
                        if button(ui, state, "Import data…").clicked() {
                            dialogs::import(state);
                            ui.close();
                        }
                        ui.separator();
                        if ui
                            .add_enabled(!state.active, egui::Button::new("Save").shortcut_text(ui.ctx().format_shortcut(&egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::S))))
                            .clicked()
                        {
                            dialogs::save(state, jobs, false);
                            ui.close();
                        }
                        if ui
                            .add_enabled(!state.active, egui::Button::new("Save as…"))
                            .clicked()
                        {
                            dialogs::save(state, jobs, true);
                            ui.close();
                        }
                        ui.separator();
                        if ui
                            .add_enabled(!state.active, egui::Button::new("Export CSV…"))
                            .clicked()
                        {
                            dialogs::export_data(state, jobs, crate::import::Delimiter::Comma);
                            ui.close();
                        }
                        if ui
                            .add_enabled(!state.active, egui::Button::new("Export TSV…"))
                            .clicked()
                        {
                            dialogs::export_data(state, jobs, crate::import::Delimiter::Tab);
                            ui.close();
                        }
                        if ui
                            .add_enabled(
                                !state.active && !state.project().runs().is_empty(),
                                egui::Button::new("Export all results…"),
                            )
                            .clicked()
                        {
                            dialogs::export_report(state, jobs);
                            ui.close();
                        }
                    });
                    let response = ui.add_sized([ui.available_width().max(0.), 28.], egui::Label::new(egui::RichText::new(theme::caption(state.project().title())).small()).truncate().sense(egui::Sense::drag()));
                    if response.drag_started() { ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag); }
                    let identity = state.path.as_ref().map_or_else(
                        || state.save_state_label().to_owned(),
                        |path| format!("{}\n{}", state.save_state_label(), path.display()),
                    );
                    response.on_hover_text(identity);
                });
            });
        });
    egui::Panel::bottom("status-bar")
        .min_size(28.0)
        .frame(
            egui::Frame::new()
                .fill(theme::surface(ui))
                .inner_margin(egui::Margin::symmetric(12, 4)),
        )
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "{} rows · {} variables",
                        state.project().dataset().row_count(),
                        state.project().dataset().column_count()
                    ))
                    .small()
                    .color(theme::muted(ui)),
                );
                let save_state = state.save_state_label();
                let save_label =
                    ui.add(egui::Button::new(egui::RichText::new(save_state).small()).frame(false));
                save_label.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, save_state)
                });
                if state.active {
                    ui.spinner();
                    ui.label(if state.cancelled {
                        "Cancelling…"
                    } else {
                        state.operation_name
                    });
                    if let Some(p) = state.progress {
                        if let Some(total) = p.total {
                            ui.label(format!("{}%", p.done.saturating_mul(100) / total.max(1)));
                        }
                    }
                    let cancel = ui.add_enabled(
                        !state.cancelled,
                        egui::Button::new("Cancel operation").wrap_mode(egui::TextWrapMode::Extend),
                    );
                    procedure_forms::record(state, "Cancel operation", &cancel);
                    if cancel.clicked() {
                        state.request_cancel(jobs);
                    }
                } else {
                    ui.add(egui::Label::new(theme::caption(&state.status)).truncate());
                }
                if state.has_file_receipt() {
                    let details = ui.small_button("File details");
                    procedure_forms::record(state, "File details", &details);
                    if details.clicked() {
                        state.return_focus = Some(details.id);
                        state.show_file_receipt();
                    }
                }
                if state.has_recovery_receipt() {
                    let details = ui.small_button("Recovery details");
                    procedure_forms::record(state, "Recovery details", &details);
                    if details.clicked() {
                        state.return_focus = Some(details.id);
                        state.show_recovery_receipt();
                    }
                }
            });
            if let Some(error) = state.error() {
                ui.label(
                    egui::RichText::new(format!("Needs attention: {}", theme::caption(error)))
                        .color(ui.visuals().error_fg_color),
                );
                let details = ui.small_button("Error details");
                procedure_forms::record(state, "Error details", &details);
                if details.clicked() {
                    state.return_focus = Some(details.id);
                    state.show_error_details();
                }
            }
        });
    navigation(ui, state);
}

fn navigation(ui: &mut egui::Ui, state: &mut DesktopState) {
    if ui.ctx().content_rect().width() >= 1000. && ui.ctx().content_rect().height() >= 640. {
        egui::Panel::left("navigation")
            .exact_size(160.)
            .resizable(false)
            .frame(egui::Frame::new().fill(theme::surface(ui)).inner_margin(12))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.;
                ui.small("Workspace");
                ui.add_space(6.);
                for view in [
                    Workspace::Data,
                    Workspace::Prepare,
                    Workspace::Analyses,
                    Workspace::Results,
                    Workspace::Variables,
                ] {
                    let response =
                        theme::list_row(ui, state.workspace() == view, display_label(view), None);
                    procedure_forms::record(state, view.label(), &response);
                    if view == Workspace::Analyses {
                        procedure_forms::record(state, "Analyse", &response);
                    }
                    if response.clicked() {
                        state.act(UiAction::Navigate(view));
                    }
                }
            });
    } else {
        egui::Panel::top("workflow-navigation")
            .frame(
                egui::Frame::new()
                    .fill(theme::surface(ui))
                    .inner_margin(egui::Margin::symmetric(8, 4)),
            )
            .show(ui, |ui| {
                ui.spacing_mut().button_padding = egui::vec2(8., 5.);
                ui.spacing_mut().item_spacing.x = 4.;
                ui.horizontal_wrapped(|ui| {
                    for view in [
                        Workspace::Data,
                        Workspace::Prepare,
                        Workspace::Analyses,
                        Workspace::Results,
                        Workspace::Variables,
                    ] {
                        navigation_button(ui, state, view, display_label(view), [0., 30.]);
                    }
                });
            });
    }
}

fn display_label(view: Workspace) -> &'static str {
    match view {
        Workspace::Analyses => "Analyse",
        _ => view.label(),
    }
}

fn navigation_button(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    view: Workspace,
    label: &str,
    size: [f32; 2],
) {
    let selected = state.workspace() == view;
    let response = ui.add_sized(
        size,
        egui::Button::new(label)
            .selected(selected)
            .stroke(egui::Stroke::NONE)
            .frame_when_inactive(selected),
    );
    procedure_forms::record(state, view.label(), &response);
    if view == Workspace::Analyses {
        procedure_forms::record(state, "Analyse", &response);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.shrink(1.),
            6.,
            egui::Stroke::new(2., theme::text(ui)),
            egui::StrokeKind::Inside,
        );
    }
    if response.clicked() {
        state.act(UiAction::Navigate(view));
    }
}
