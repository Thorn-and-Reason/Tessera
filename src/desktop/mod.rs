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
pub(super) mod guidance;
mod help;
pub use experience::AnalysisMode;
mod figure_editor;
mod import_review;
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
mod shell;
mod state;
mod survey;
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
        .with_fullsize_content_view(false)
        .with_titlebar_shown(true)
        .with_title_shown(true)
        .with_titlebar_buttons_shown(true);
    let options = eframe::NativeOptions {
        viewport,
        persist_window: true,
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
                Command::Prepare => self.state.act(UiAction::Navigate(Workspace::Prepare)),
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
    let restore_focus = state.dialog.is_none() && !state.active && !state.modal_file_dialog;
    if !blocked {
        shortcuts(ui.ctx(), state, jobs);
    }
    if blocked {
        ui.disable();
    }
    shell::show(ui, state, jobs);
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(ui.visuals().panel_fill)
                .inner_margin(16),
        )
        .show(ui, |ui| match state.workspace() {
            Workspace::Data => data::show(ui, state, jobs),
            Workspace::Prepare => preparation::show(ui, state, jobs),
            Workspace::Variables => variables::show(ui, state, jobs),
            Workspace::Analyses => analyses::show(ui, state, jobs),
            Workspace::Results => results::show(ui, state, jobs),
        });
    if state.take_survey_import_picker() {
        dialogs::import(state);
    }
    dialogs::show(ui.ctx(), state, jobs);
    if restore_focus && state.dialog.is_none() && !state.active && !state.modal_file_dialog {
        if let Some(opener) = state.return_focus.take() {
            let destination = if state.control_ids.values().any(|id| *id == opener) {
                Some(opener)
            } else {
                state.control_id(state.workspace().label())
            };
            if let Some(destination) = destination {
                ui.memory_mut(|memory| memory.request_focus(destination));
            }
        }
    }
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
    let text = egui::RichText::new(&displayed);
    let text = if kind == theme::Kind::Primary {
        text.strong()
    } else {
        text
    };
    let galley = egui::WidgetText::from(text).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    let mut size = galley.size() + 2. * ui.spacing().button_padding;
    size.y = size.y.max(ui.spacing().interact_size.y);
    let shortcut = match label {
        "New project" => Some(egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::N)),
        "Open" | "Open…" => Some(egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::O)),
        "Save" => Some(egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::S)),
        _ => None,
    };
    let response = ui
        // Reserve the whole control before a wrapped row chooses its position.
        .allocate_ui(size, |ui| {
            ui.push_id(egui::Id::new(label), |ui| {
                let command = kind.build(ui, &displayed);
                let response = ui.add(command);
                if let Some(shortcut) = shortcut {
                    response.on_hover_text(ui.ctx().format_shortcut(&shortcut))
                } else {
                    response
                }
            })
            .inner
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
    if state.survey_review().is_some() {
        if !state.survey_ready() {
            return;
        }
        if let Err(error) = state.submit_survey(jobs) {
            state.fail(error);
        }
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
