mod about;
mod analyses;
mod anova_forms;
mod chart_drawing;
mod data;
mod dialogs;
pub mod draft;
mod experience;
mod exploration_forms;
mod exploration_results;
pub use experience::AnalysisMode;
mod figure_editor;
#[cfg(any(target_os = "macos", test))]
mod macos;
mod output_style;
mod preparation;
mod procedure_forms;
mod recovery;
mod regression_forms;
mod regression_results;
mod requests;
mod result_tables;
mod results;
mod state;
mod theme;
mod variables;
#[cfg(target_os = "windows")]
mod windows;
use crate::{
    error::{AppError, AppResult},
    jobs::{JobOperation, JobRunner},
};
use eframe::egui::{self, Key, Modifiers};
pub use state::{
    DesktopState, Dialog, PendingAction, PropertyChoice, UiAction, UnsavedChoice, Workspace,
    UI_WORK_BYTES,
};
use std::{
    io::Write,
    time::{Duration, Instant},
};
pub use theme::install as configure_context;

pub fn launch() -> AppResult<()> {
    let state = DesktopState::new()?;
    let icon = image::load_from_memory(include_bytes!("../../assets/tessera-icon.png"))
        .map_err(|error| crate::error::AppError::invalid("Application icon", error.to_string()))?
        .into_rgba8();
    let icon = egui::IconData {
        width: icon.width(),
        height: icon.height(),
        rgba: icon.into_raw(),
    };
    let viewport = egui::ViewportBuilder::default()
        .with_app_id("com.thornandreason.tessera")
        .with_icon(icon)
        .with_inner_size([1180.0, 780.0])
        .with_min_inner_size([640.0, 440.0]);
    #[cfg(target_os = "macos")]
    let viewport = viewport
        .with_fullsize_content_view(true)
        .with_titlebar_shown(false)
        .with_title_shown(false)
        .with_titlebar_buttons_shown(true);
    let options = eframe::NativeOptions {
        viewport,
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        "Tessera",
        options,
        Box::new(move |cc| {
            #[cfg(target_os = "macos")]
            macos::install(&cc.egui_ctx)?;
            theme::install(&cc.egui_ctx);
            let logo = theme::mark(&cc.egui_ctx)
                .map_err(Box::<dyn std::error::Error + Send + Sync>::from)?;
            let perf = std::env::var_os("TESSERA_PERF_LOG")
                .map(|path| {
                    std::fs::OpenOptions::new()
                        .create_new(true)
                        .write(true)
                        .open(path)
                })
                .transpose()?;
            let mut state =
                state.apply_action(UiAction::SetAnalysisMode(experience::load_mode(cc.storage)))?;
            let recovery = match recovery::RecoveryManager::initialize() {
                Ok(manager) => Some(manager),
                Err(error) => {
                    state.fail(error);
                    None
                }
            };
            Ok(Box::new(NativeApp {
                state,
                recovery,
                jobs: JobRunner::default(),
                logo,
                perf,
                input_time: None,
            }))
        }),
    )
    .map_err(|e| AppError::invalid("application", e.to_string()))
}
struct NativeApp {
    state: DesktopState,
    jobs: JobRunner,
    logo: egui::TextureHandle,
    perf: Option<std::fs::File>,
    input_time: Option<(Instant, bool, &'static str, bool)>,
    recovery: Option<recovery::RecoveryManager>,
}
impl eframe::App for NativeApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        experience::save_mode(storage, self.state.analysis_mode());
    }
    fn persist_egui_memory(&self) -> bool {
        false
    }
    fn raw_input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        #[cfg(target_os = "macos")]
        while let Some(command) = macos::take_command() {
            use macos::NativeCommand as Command;
            match command {
                Command::Copy => input.events.push(egui::Event::Copy),
                Command::Paste => match macos::clipboard_text(&self.state) {
                    Ok(text) => input.events.push(egui::Event::Paste(text)),
                    Err(error) => self.state.fail(error),
                },
                Command::SelectAll => input.events.push(egui::Event::Key {
                    key: Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::COMMAND,
                }),
                Command::Undo | Command::Redo => input.events.push(egui::Event::Key {
                    key: Key::Z,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: if matches!(command, Command::Redo) {
                        Modifiers::COMMAND | Modifiers::SHIFT
                    } else {
                        Modifiers::COMMAND
                    },
                }),
                _ if self.state.dialog.is_some()
                    || self.recovery.as_ref().is_some_and(|m| m.blocks_ui()) => {}
                Command::New => self.state.act(UiAction::Request(PendingAction::New)),
                Command::Open => dialogs::open(&mut self.state),
                Command::Import => dialogs::import(&mut self.state),
                Command::Save if !self.state.active => {
                    dialogs::save(&mut self.state, &mut self.jobs, false)
                }
                Command::SaveAs if !self.state.active => {
                    dialogs::save(&mut self.state, &mut self.jobs, true)
                }
                Command::ExportReport if !self.state.active => {
                    dialogs::export_report(&mut self.state, &mut self.jobs)
                }
                Command::Data => self.state.act(UiAction::Navigate(Workspace::Data)),
                Command::Variables => self.state.act(UiAction::Navigate(Workspace::Variables)),
                Command::Analyses => self.state.act(UiAction::Navigate(Workspace::Analyses)),
                Command::Results => self.state.act(UiAction::Navigate(Workspace::Results)),
                Command::RunAnalysis => run_analysis(&mut self.state, &mut self.jobs),
                _ => {}
            }
        }
        if !input.events.is_empty() {
            self.input_time = Some((
                Instant::now(),
                self.jobs.is_running(),
                self.state.operation_name,
                self.state.cancelled,
            ));
        }
    }
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.state.poll(&mut self.jobs);
        #[cfg(target_os = "macos")]
        macos::process_request(&mut self.state);
        if ctx.input(|i| i.viewport().close_requested()) && !self.state.shutdown {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.state.act(UiAction::Request(PendingAction::Close));
        }
        let may_transition = self.recovery.as_mut().map_or(true, |manager| {
            manager.before_transition(&mut self.state, &mut self.jobs)
        });
        if may_transition {
            self.state.process_pending(&mut self.jobs);
        }
        if let Some(manager) = &mut self.recovery {
            manager.tick(&mut self.state, &mut self.jobs);
            if self.state.is_dirty() {
                ctx.request_repaint_after(Duration::from_millis(250));
            }
        }
        if self.state.shutdown && self.jobs.is_shutdown_complete() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.jobs.is_running() || self.state.shutdown {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        #[cfg(target_os = "windows")]
        windows::sync(_frame, ui.ctx());
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::new("tessera-logo"), self.logo.id()));
        render_with_recovery(ui, &mut self.state, &mut self.jobs, self.recovery.as_mut());
        if let (Some((start, arrival_active, operation, cancelled_at_arrival)), Some(log)) =
            (self.input_time.take(), self.perf.as_mut())
        {
            // Local opt-in timings contain no paths, imported values or identifiers.
            if let Err(error) = writeln!(
                log,
                "{:.3},{},{},{},{},{},{}",
                start.elapsed().as_secs_f64() * 1000.0,
                arrival_active,
                self.jobs.is_running(),
                operation,
                cancelled_at_arrival,
                self.state.cancelled,
                self.state.modal_file_dialog
            ) {
                self.state.fail(error.into());
                self.perf = None;
            }
        }
    }
}
/// The same root UI is used by the native shell and headless input tests.
pub fn render(ui: &mut egui::Ui, state: &mut DesktopState, jobs: &mut JobRunner) {
    render_with_recovery(ui, state, jobs, None);
}
fn render_with_recovery(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    jobs: &mut JobRunner,
    mut recovery: Option<&mut recovery::RecoveryManager>,
) {
    let blocked = recovery.as_ref().is_some_and(|m| m.blocks_ui());
    state.modal_file_dialog = blocked;
    state.controls.clear();
    state.control_ids.clear();
    state.poll(jobs);
    if !blocked {
        shortcuts(ui.ctx(), state, jobs);
    }
    if blocked {
        ui.disable();
    }
    egui::Panel::top("project-bar")
        .exact_size(54.0)
        .frame(
            egui::Frame::new()
                .fill(theme::surface(ui))
                .inner_margin(egui::Margin::symmetric(12, 0)),
        )
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                #[cfg(target_os = "macos")]
                ui.add_space(78. / ui.ctx().zoom_factor());
                if let Some(logo) = ui
                    .ctx()
                    .data(|d| d.get_temp::<egui::TextureId>(egui::Id::new("tessera-logo")))
                {
                    ui.image((logo, egui::vec2(24.0, 24.0)));
                }
                theme::wordmark(ui, "Tessera");
                ui.separator();
                experience::mode_switch(ui, state);
                ui.separator();
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(theme::caption(format_args!(
                            "{}{}",
                            state.project().title(),
                            if state.is_dirty() { " •" } else { "" }
                        )))
                        .strong(),
                    )
                    .truncate(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                        let response = button(ui, state, "About Tessera");
                        if response.clicked() {
                            state.dialog = Some(Dialog::About);
                            state.return_focus = Some(response.id);
                            ui.close();
                        }
                    });
                    procedure_forms::record(state, "Help", &help.response);
                    ui.menu_button("View", |ui| {
                        ui.strong("Appearance");
                        let mut preference = ui.ctx().options(|o| o.theme_preference);
                        preference.radio_buttons(ui);
                        ui.ctx().set_theme(preference);
                        ui.separator();
                        ui.label("Text and interface size");
                        let mut zoom = ui.ctx().zoom_factor();
                        if ui.add(egui::Slider::new(&mut zoom, 0.85..=1.5).step_by(0.05).suffix("×")).changed() {
                            ui.ctx().set_zoom_factor(zoom);
                        }
                        if ui.button("Reset size").clicked() { ui.ctx().set_zoom_factor(1.); }
                        ui.separator();
                        ui.label(format!("Run analysis: {}", ui.ctx().format_shortcut(&egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::Enter))));
                        ui.small("In Analyses. In Data, use arrows to move, Enter to edit, Tab to advance, and Escape to cancel.");
                        ui.small("Paste TSV into existing cells: up to 4,096 cells / 512 KiB. Blank numbers become missing; blank text stays empty.");
                    });
                    ui.menu_button("Edit", |ui| {
                        if ui.add_enabled(state.can_undo(), egui::Button::new("Undo").shortcut_text(ui.ctx().format_shortcut(&egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::Z)))).clicked() { state.act(UiAction::Undo); ui.close(); }
                        if ui.add_enabled(state.can_redo(), egui::Button::new("Redo").shortcut_text(ui.ctx().format_shortcut(&egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)))).clicked() { state.act(UiAction::Redo); ui.close(); }
                        ui.separator();
                        if ui.add_enabled(!state.active && state.cell_editor.is_none() && !state.metadata_has_changes() && !state.project().dataset().columns().is_empty(),egui::Button::new("Prepare data…")).clicked() { state.dialog = Some(Dialog::Prepare); ui.close(); }
                    });
                    ui.menu_button("File", |ui| {
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
                                egui::Button::new("Export report…"),
                            )
                            .clicked()
                        {
                            dialogs::export_report(state, jobs);
                            ui.close();
                        }
                    });
                    #[cfg(target_os = "macos")]
                    {
                        let drag = ui.allocate_response(egui::vec2(ui.available_width().max(0.), ui.available_height()), egui::Sense::drag());
                        if drag.drag_started() { ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag); }
                    }
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
                if state.active {
                    ui.spinner();
                    ui.label(state.operation_name);
                    if let Some(p) = state.progress {
                        if let Some(total) = p.total {
                            ui.label(format!("{}%", p.done.saturating_mul(100) / total.max(1)));
                        }
                    }
                    if button(ui, state, "Cancel operation").clicked() {
                        jobs.cancel();
                        state.cancelled = true;
                    }
                } else {
                    ui.add(egui::Label::new(theme::caption(&state.status)).truncate());
                }
                if state.file_outcome.is_some() && ui.small_button("File details").clicked() {
                    state.dialog = Some(Dialog::FileOutcome);
                }
            });
            if let Some(error) = state.error() {
                ui.label(
                    egui::RichText::new(format!("Needs attention: {}", theme::caption(error)))
                        .color(ui.visuals().error_fg_color),
                );
            }
        });
    egui::Panel::left("navigation")
        .exact_size(164.0)
        .resizable(false)
        .frame(
            egui::Frame::new()
                .fill(theme::surface(ui))
                .inner_margin(egui::Margin::same(12)),
        )
        .show(ui, |ui| {
            ui.add_space(4.);
            ui.label(
                egui::RichText::new("WORKSPACE")
                    .small()
                    .strong()
                    .color(theme::muted(ui)),
            );
            ui.add_space(8.);
            for view in [
                Workspace::Data,
                Workspace::Variables,
                Workspace::Analyses,
                Workspace::Results,
            ] {
                let selected = state.workspace() == view;
                let response = ui.add_sized(
                    [ui.available_width(), 40.0],
                    egui::Button::new(egui::RichText::new(view.label()).color(if selected {
                        theme::text(ui)
                    } else {
                        theme::muted(ui)
                    }))
                    .right_text("")
                    .selected(selected)
                    .frame_when_inactive(selected)
                    .fill(if selected {
                        theme::selection(ui)
                    } else {
                        egui::Color32::TRANSPARENT
                    }),
                );
                state.controls.insert(view.label().into(), response.rect);
                if response.has_focus() {
                    ui.painter().rect_stroke(
                        response.rect.shrink(1.0),
                        6.0,
                        egui::Stroke::new(2.0, theme::text(ui)),
                        egui::StrokeKind::Inside,
                    );
                }
                if selected {
                    ui.painter().rect_filled(
                        egui::Rect::from_min_size(
                            response.rect.left_top() + egui::vec2(0., 7.),
                            egui::vec2(3., response.rect.height() - 14.),
                        ),
                        1.5,
                        theme::accent(ui),
                    );
                }
                if response.clicked() {
                    state.act(UiAction::Navigate(view));
                }
            }
            ui.separator();
            if button(ui, state, "New project").clicked() {
                state.act(UiAction::Request(PendingAction::New));
            }
            if button(ui, state, "Import…").clicked() {
                dialogs::import(state);
            }
        });
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(ui.visuals().panel_fill)
                .inner_margin(18),
        )
        .show(ui, |ui| match state.workspace() {
            Workspace::Data => data::show(ui, state, jobs),
            Workspace::Variables => variables::show(ui, state, jobs),
            Workspace::Analyses => analyses::show(ui, state, jobs),
            Workspace::Results => results::show(ui, state, jobs),
        });
    dialogs::show(ui.ctx(), state, jobs);
    if let Some(save_as) = state.take_property_save_requested() {
        dialogs::save(state, jobs, save_as);
    }
    if state.take_save_requested() {
        dialogs::save(state, jobs, false);
    }
    let may_transition = recovery
        .as_mut()
        .map_or(true, |manager| manager.before_transition(state, jobs));
    if may_transition {
        state.process_pending(jobs);
    }
    if let Some(manager) = recovery {
        manager.show(ui.ctx(), state, jobs);
    }
    if jobs.is_running() {
        ui.ctx().request_repaint_after(Duration::from_millis(16));
    }
}
pub(crate) fn button(ui: &mut egui::Ui, state: &mut DesktopState, label: &str) -> egui::Response {
    button_styled(ui, state, label, theme::Kind::Secondary)
}
/// The decisive action in a dialog footer.
pub(crate) fn command(ui: &mut egui::Ui, state: &mut DesktopState, label: &str) -> egui::Response {
    button_styled(ui, state, label, theme::Kind::Primary)
}
/// Inspector and panel toggles recede from the workspace's main action.
pub(crate) fn toggle(ui: &mut egui::Ui, state: &mut DesktopState, label: &str) -> egui::Response {
    button_styled(ui, state, label, theme::Kind::Quiet)
}
fn button_styled(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    label: &str,
    kind: theme::Kind,
) -> egui::Response {
    let displayed = theme::caption(label);
    let shortcut = match label {
        "New project" => Some(egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::N)),
        "Open" | "Open…" => Some(egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::O)),
        "Save" => Some(egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::S)),
        _ => None,
    };
    let response = ui
        .push_id(egui::Id::new(label), |ui| {
            let command = kind.build(ui, &displayed);
            let response = ui.add(command);
            if let Some(shortcut) = shortcut {
                response.on_hover_text(ui.ctx().format_shortcut(&shortcut))
            } else {
                response
            }
        })
        .inner;
    state.controls.insert(displayed.clone(), response.rect);
    state.control_ids.insert(displayed, response.id);
    response
}
fn shortcuts(ctx: &egui::Context, state: &mut DesktopState, jobs: &mut JobRunner) {
    if state.dialog.is_some() {
        return;
    }
    let consumed = |key, modifiers| ctx.input_mut(|i| i.consume_key(modifiers, key));
    if !ctx.egui_wants_keyboard_input() {
        if consumed(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT) {
            state.act(UiAction::Redo);
        } else if consumed(Key::Z, Modifiers::COMMAND) {
            state.act(UiAction::Undo);
        }
    }
    if consumed(Key::O, Modifiers::COMMAND) {
        dialogs::open(state);
    }
    if consumed(Key::S, Modifiers::COMMAND) && !state.active {
        dialogs::save(state, jobs, false);
    }
    if consumed(Key::N, Modifiers::COMMAND) {
        state.act(UiAction::Request(PendingAction::New));
    }
    if consumed(Key::Enter, Modifiers::COMMAND) {
        run_analysis(state, jobs);
    }
}
fn run_analysis(state: &mut DesktopState, jobs: &mut JobRunner) {
    if state.active || state.dialog.is_some() || state.workspace() != Workspace::Analyses {
        return;
    }
    let project = Arc::clone(state.project());
    match state.checked_draft_request() {
        Ok(request) => start(state, jobs, JobOperation::Analyze { project, request }),
        Err(error) => state.fail(error),
    }
}
use std::sync::Arc;
pub(crate) fn start(state: &mut DesktopState, jobs: &mut JobRunner, operation: JobOperation) {
    if let Err(error) = state.start_operation(jobs, operation) {
        state.fail(error);
    }
}

mod anova_results;

mod paged_rows;

mod reliability_forms;
mod reliability_results;
