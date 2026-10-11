//! Native menu commands and termination use the existing application decisions.
use super::{DesktopState, PendingAction, UiAction};
use eframe::egui;
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(isize)]
pub(super) enum NativeCommand {
    New = 1,
    Open,
    Import,
    Save,
    SaveAs,
    ExportReport,
    Copy,
    Paste,
    SelectAll,
    Data,
    Variables,
    Analyses,
    Results,
    RunAnalysis,
    Undo,
    Redo,
}
impl NativeCommand {
    fn from_tag(tag: isize) -> Option<Self> {
        use NativeCommand::*;
        [
            New,
            Open,
            Import,
            Save,
            SaveAs,
            ExportReport,
            Copy,
            Paste,
            SelectAll,
            Data,
            Variables,
            Analyses,
            Results,
            RunAnalysis,
            Undo,
            Redo,
        ]
        .into_iter()
        .find(|command| *command as isize == tag)
    }
}

#[derive(Default)]
struct CommandQueue(Mutex<VecDeque<NativeCommand>>);
impl CommandQueue {
    fn push(&self, command: NativeCommand) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push_back(command);
    }
    fn take(&self) -> Option<NativeCommand> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop_front()
    }
}

#[derive(Default)]
struct QuitRequest {
    pending: AtomicBool,
}
impl QuitRequest {
    fn request(&self, ctx: &egui::Context) {
        self.pending.store(true, Ordering::Release);
        ctx.request_repaint();
    }
    fn process(&self, state: &mut DesktopState) {
        if self.pending.swap(false, Ordering::AcqRel)
            && !state.shutdown
            && !matches!(state.pending(), Some(PendingAction::Close))
        {
            state.act(UiAction::Request(PendingAction::Close));
        }
    }
}

#[cfg(any(target_os = "macos", test))]
fn check_clipboard_bytes(state: &DesktopState, bytes: usize) -> crate::error::AppResult<()> {
    if bytes > 512 * 1024 {
        return Err(crate::error::AppError::invalid(
            "clipboard",
            "Paste at most 512 KiB and 4,096 cells at a time.",
        ));
    }
    state.check_ui_copy(bytes)
}

#[cfg(target_os = "macos")]
pub(super) use native::{clipboard_text, install, process_request, take_command};

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use crate::error::{AppError, AppResult};
    use objc2::{
        ffi::class_addMethod,
        runtime::{AnyClass, AnyObject, Imp, Sel},
        sel, Encode, MainThreadMarker, MainThreadOnly,
    };
    use objc2_app_kit::{
        NSApplication, NSApplicationTerminateReply, NSEventModifierFlags, NSMenu, NSMenuItem,
        NSPasteboard, NSPasteboardTypeString,
    };
    use objc2_foundation::NSString;
    use std::{ffi::CString, sync::OnceLock};

    static REQUEST: QuitRequest = QuitRequest {
        pending: AtomicBool::new(false),
    };
    static CONTEXT: OnceLock<egui::Context> = OnceLock::new();
    static COMMANDS: CommandQueue = CommandQueue(Mutex::new(VecDeque::new()));

    pub(in super::super) fn install(ctx: &egui::Context) -> AppResult<()> {
        let main_thread = MainThreadMarker::new()
            .ok_or_else(|| failure("The termination guard requires the main thread."))?;
        let app = NSApplication::sharedApplication(main_thread);
        let delegate = app
            .delegate()
            .ok_or_else(|| failure("The native application delegate is missing."))?;
        let object: &AnyObject = AsRef::<AnyObject>::as_ref(&*delegate);
        let class = object.class();
        if class.name().to_bytes() != b"WinitApplicationDelegate" {
            return Err(failure("The native application delegate is not supported."));
        }
        let selector = sel!(applicationShouldTerminate:);
        if class.instance_method(selector).is_some() {
            return Err(failure("A native termination handler already exists."));
        }
        let encoding = CString::new(format!("{}@:@", NSApplicationTerminateReply::ENCODING))
            .map_err(|_| failure("The native termination signature is invalid."))?;
        CONTEXT
            .set(ctx.clone())
            .map_err(|_| failure("The native termination guard was already initialized."))?;
        type Callback =
            extern "C" fn(*mut AnyObject, Sel, *mut NSApplication) -> NSApplicationTerminateReply;
        // SAFETY: Objective-C IMP erases a method's signature. This callback has
        // precisely the encoded NSUInteger-returning (id, SEL, id) ABI above.
        // It borrows/dereferences neither object pointer and catches Rust panics.
        let implementation: Imp = unsafe { std::mem::transmute(should_terminate as Callback) };
        // SAFETY: This is the live runtime class of winit's existing delegate,
        // accessed on the main thread. The class is not replaced or subclassed.
        // Only an absent selector is added; no existing/inherited method is
        // overwritten. The runtime copies the encoding and retains the static IMP.
        let added = unsafe {
            class_addMethod(
                class as *const AnyClass as *mut AnyClass,
                selector,
                implementation,
                encoding.as_ptr(),
            )
        };
        if !added.as_bool() {
            return Err(failure(
                "The native termination guard could not be installed.",
            ));
        }
        install_menus(&app, object, class, main_thread)?;
        Ok(())
    }
    fn install_menus(
        app: &NSApplication,
        target: &AnyObject,
        class: &AnyClass,
        mtm: MainThreadMarker,
    ) -> AppResult<()> {
        let selector = sel!(tesseraMenuCommand:);
        if class.instance_method(selector).is_some() {
            return Err(failure("A native menu command handler already exists."));
        }
        type Callback = extern "C" fn(*mut AnyObject, Sel, *mut NSMenuItem);
        // SAFETY: The callback matches the void-returning (id, SEL, id) method ABI.
        let implementation: Imp = unsafe { std::mem::transmute(menu_command as Callback) };
        // SAFETY: Only an absent selector is added to the existing delegate class.
        let added = unsafe {
            class_addMethod(
                class as *const AnyClass as *mut AnyClass,
                selector,
                implementation,
                c"v@:@".as_ptr(),
            )
        };
        if !added.as_bool() {
            return Err(failure(
                "The native menu command handler could not be installed.",
            ));
        }
        let main = app
            .mainMenu()
            .ok_or_else(|| failure("The native application menu is missing."))?;
        use NativeCommand::*;
        type MenuEntry = (&'static str, NativeCommand, &'static str, bool);
        let groups: &[(&str, &[MenuEntry])] = &[
            (
                "File",
                &[
                    ("New Project", New, "n", false),
                    ("Open Project…", Open, "o", false),
                    ("Import Data…", Import, "i", false),
                    ("Save", Save, "s", false),
                    ("Save As…", SaveAs, "s", true),
                    ("Export Report…", ExportReport, "e", true),
                ],
            ),
            (
                "Edit",
                &[
                    ("Undo", Undo, "z", false),
                    ("Redo", Redo, "z", true),
                    ("Copy", Copy, "c", false),
                    ("Paste", Paste, "v", false),
                    ("Select All", SelectAll, "a", false),
                ],
            ),
            (
                "View",
                &[
                    ("Data", Data, "1", false),
                    ("Variables", Variables, "2", false),
                    ("Analyses", Analyses, "3", false),
                    ("Results", Results, "4", false),
                ],
            ),
            ("Analysis", &[("Run Analysis", RunAnalysis, "r", false)]),
        ];
        for (title, entries) in groups {
            let submenu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title));
            // Availability is enforced by the shared command handlers, including active jobs/dialogs.
            submenu.setAutoenablesItems(false);
            for (label, command, key, shift) in *entries {
                // SAFETY: This selector was installed above with the exact NSMenuItem callback ABI.
                let item = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        &NSString::from_str(label),
                        Some(selector),
                        &NSString::from_str(key),
                    )
                };
                item.setTag(*command as isize);
                item.setKeyEquivalentModifierMask(
                    NSEventModifierFlags::Command
                        | if *shift {
                            NSEventModifierFlags::Shift
                        } else {
                            NSEventModifierFlags::empty()
                        },
                );
                // SAFETY: The application owns its delegate for the lifetime of the menu.
                unsafe { item.setTarget(Some(target)) };
                submenu.addItem(&item);
            }
            // SAFETY: A submenu parent has no action selector.
            let parent = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(title),
                    None,
                    &NSString::from_str(""),
                )
            };
            parent.setSubmenu(Some(&submenu));
            main.addItem(&parent);
        }
        Ok(())
    }
    extern "C" fn menu_command(_receiver: *mut AnyObject, _selector: Sel, sender: *mut NSMenuItem) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // SAFETY: Cocoa supplies the sending menu item for the installed item action.
            if let Some(sender) = unsafe { sender.as_ref() } {
                if let Some(command) = NativeCommand::from_tag(sender.tag()) {
                    COMMANDS.push(command);
                    if let Some(ctx) = CONTEXT.get() {
                        ctx.request_repaint();
                    }
                }
            }
        }));
        if result.is_err() {
            use std::io::Write;
            let _ = writeln!(
                std::io::stderr(),
                "Tessera could not queue the native menu command."
            );
        }
    }
    pub(in super::super) fn clipboard_text(state: &DesktopState) -> AppResult<String> {
        // SAFETY: AppKit provides this immutable process-lifetime pasteboard type constant.
        let text = NSPasteboard::generalPasteboard()
            .stringForType(unsafe { NSPasteboardTypeString })
            .ok_or_else(|| {
                AppError::invalid("clipboard", "The clipboard does not contain text.")
            })?;
        check_clipboard_bytes(state, text.len())?;
        Ok(text.to_string())
    }
    pub(in super::super) fn take_command() -> Option<NativeCommand> {
        COMMANDS.take()
    }
    fn failure(message: &str) -> AppError {
        AppError::invalid("macOS startup", message)
    }
    extern "C" fn should_terminate(
        _receiver: *mut AnyObject,
        _selector: Sel,
        _sender: *mut NSApplication,
    ) -> NSApplicationTerminateReply {
        // Always veto immediate OS termination. The normal application close
        // path alone decides whether to save/discard/cancel and drains the worker.
        // Even a repaint failure must not unwind over Cocoa or permit data loss.
        let repaint = std::panic::catch_unwind(|| {
            if let Some(ctx) = CONTEXT.get() {
                REQUEST.request(ctx);
            }
        });
        if repaint.is_err() {
            use std::io::Write;
            let _ = writeln!(
                std::io::stderr(),
                "Tessera kept the application open because the quit repaint failed."
            );
        }
        NSApplicationTerminateReply::TerminateCancel
    }
    pub(in super::super) fn process_request(state: &mut DesktopState) {
        REQUEST.process(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        data::{Cell, Column, ColumnId, Dataset},
        desktop::{Dialog, UnsavedChoice},
        jobs::{JobOperation, JobRunner},
        operation::JobControl,
        project::Project,
    };
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };

    #[test]
    fn clipboard_utf8_bytes_are_admitted_before_owned_conversion() {
        let mut state = dirty_state(0);
        check_clipboard_bytes(&state, 512 * 1024).unwrap();
        assert!(check_clipboard_bytes(&state, 512 * 1024 + 1).is_err());
        let baseline = state.project_baseline().unwrap().1;
        state.active_recipe_bytes = crate::data::MAX_OWNED_BYTES - baseline - state.extra() - 1024;
        let (result, largest) =
            crate::data::measure_test_allocations(|| check_clipboard_bytes(&state, 2048));
        assert!(matches!(
            result,
            Err(crate::error::AppError::SizeLimit { .. })
        ));
        assert_eq!(largest, 0);
    }

    #[test]
    fn menu_commands_preserve_click_order_and_reject_unknown_tags() {
        let queue = CommandQueue::default();
        queue.push(NativeCommand::Save);
        queue.push(NativeCommand::Open);
        assert_eq!(queue.take(), Some(NativeCommand::Save));
        assert_eq!(queue.take(), Some(NativeCommand::Open));
        assert_eq!(queue.take(), None);
        assert_eq!(NativeCommand::from_tag(0), None);
        assert_eq!(NativeCommand::from_tag(99), None);
        assert_eq!(
            NativeCommand::from_tag(14),
            Some(NativeCommand::RunAnalysis)
        );
    }
    fn dirty_state(rows: usize) -> DesktopState {
        DesktopState::from_project(
            Project::new(
                "Close check",
                Dataset::new(vec![Column::numeric(
                    ColumnId(1),
                    "Score",
                    vec![Some(17.0); rows],
                )
                .unwrap()])
                .unwrap(),
            )
            .unwrap(),
        )
        .unwrap()
    }
    fn poll_until_idle(state: &mut DesktopState, jobs: &mut JobRunner) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while jobs.is_running() {
            state.poll(jobs);
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
    #[test]
    fn queued_native_quit_coalesces_and_cancel_preserves_the_document() {
        let mut state = dirty_state(0);
        let project = Arc::clone(state.project());
        let requests = QuitRequest::default();
        let ctx = egui::Context::default();
        requests.request(&ctx);
        requests.request(&ctx);
        requests.process(&mut state);
        assert!(matches!(state.pending(), Some(PendingAction::Close)));
        assert_eq!(state.dialog(), Some(Dialog::Unsaved));
        assert!(!state.shutdown);
        state.act(UiAction::ResolveUnsaved(UnsavedChoice::Cancel));
        requests.process(&mut state);
        assert!(state.pending().is_none());
        assert!(!state.shutdown);
        assert!(state.is_dirty());
        assert!(Arc::ptr_eq(&project, state.project()));
        requests.request(&ctx);
        requests.process(&mut state);
        assert_eq!(state.dialog(), Some(Dialog::Unsaved));
    }
    #[test]
    fn repeated_native_quit_waits_for_confirmed_save_before_shutdown() {
        let dir = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("tessera-native-quit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("saved.tessera");
        let _ = std::fs::remove_file(&path);
        let mut state = dirty_state(300_000);
        let mut jobs = JobRunner::default();
        state
            .start_operation(
                &mut jobs,
                JobOperation::Save {
                    project: Arc::clone(state.project()),
                    path: path.clone(),
                    expected: None,
                },
            )
            .unwrap();
        poll_until_idle(&mut state, &mut jobs);
        assert!(!state.is_dirty());
        state.act(UiAction::EditCell {
            column: ColumnId(1),
            row: 0,
            value: Cell::Number(88.0),
        });
        let requests = QuitRequest::default();
        let ctx = egui::Context::default();
        requests.request(&ctx);
        requests.process(&mut state);
        state.act(UiAction::ResolveUnsaved(UnsavedChoice::Save));
        assert!(state.take_save_requested());
        super::super::dialogs::save(&mut state, &mut jobs, false);
        assert!(jobs.is_running());
        for _ in 0..3 {
            requests.request(&ctx);
            requests.process(&mut state);
            state.process_pending(&mut jobs);
        }
        assert!(!state.shutdown);
        assert!(
            !state.cancelled,
            "repeated Quit must not cancel the protective save"
        );
        poll_until_idle(&mut state, &mut jobs);
        assert!(!state.is_dirty());
        assert!(state.error().is_none());
        state.process_pending(&mut jobs);
        assert!(state.shutdown);
        assert!(jobs.is_shutdown_complete());
        let (saved, _) = crate::files::read_project(&path, &JobControl::default()).unwrap();
        assert_eq!(
            saved.dataset().cell(ColumnId(1), 0).unwrap(),
            Cell::Number(88.0)
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn native_quit_cancels_active_work_then_waits_for_the_close_decision() {
        let mut state = dirty_state(300_000);
        let project = Arc::clone(state.project());
        let mut jobs = JobRunner::default();
        state
            .start_operation(
                &mut jobs,
                JobOperation::Analyze {
                    project: Arc::clone(state.project()),
                    request: crate::analysis::DescribeRequest {
                        columns: vec![ColumnId(1)],
                        statistics: vec![crate::analysis::Statistic::Mean],
                        missing_policy: crate::analysis::MissingPolicy::PerVariable,
                        filter: crate::analysis::RowFilter::default(),
                        frequency_weight: None,
                    }
                    .into(),
                },
            )
            .unwrap();
        let requests = QuitRequest::default();
        requests.request(&egui::Context::default());
        requests.process(&mut state);
        state.process_pending(&mut jobs);
        assert!(
            jobs.is_running(),
            "close must leave the active slot for later polling"
        );
        assert!(state.cancelled);
        assert!(!state.shutdown);
        assert!(Arc::ptr_eq(&project, state.project()));
        poll_until_idle(&mut state, &mut jobs);
        assert_eq!(state.dialog(), Some(Dialog::Unsaved));
        assert!(!state.shutdown);
        state.act(UiAction::ResolveUnsaved(UnsavedChoice::Discard));
        state.process_pending(&mut jobs);
        assert!(state.shutdown);
        assert!(jobs.is_shutdown_complete());
        requests.request(&egui::Context::default());
        requests.process(&mut state);
        assert!(state.pending().is_none());
    }
    #[test]
    fn conflicting_quit_save_preserves_the_pending_decision_and_external_file() {
        let dir = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "tessera-native-quit-conflict-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("saved.tessera");
        let _ = std::fs::remove_file(&path);
        let mut state = dirty_state(1);
        let mut jobs = JobRunner::default();
        state
            .start_operation(
                &mut jobs,
                JobOperation::Save {
                    project: Arc::clone(state.project()),
                    path: path.clone(),
                    expected: None,
                },
            )
            .unwrap();
        poll_until_idle(&mut state, &mut jobs);
        state.act(UiAction::EditCell {
            column: ColumnId(1),
            row: 0,
            value: Cell::Number(99.0),
        });
        std::fs::write(&path, b"external version").unwrap();
        let requests = QuitRequest::default();
        requests.request(&egui::Context::default());
        requests.process(&mut state);
        state.act(UiAction::ResolveUnsaved(UnsavedChoice::Save));
        assert!(state.take_save_requested());
        super::super::dialogs::save(&mut state, &mut jobs, false);
        poll_until_idle(&mut state, &mut jobs);
        state.process_pending(&mut jobs);
        assert!(!state.shutdown);
        assert!(state.is_dirty());
        assert!(matches!(state.pending(), Some(PendingAction::Close)));
        assert_eq!(std::fs::read(&path).unwrap(), b"external version");
        assert_eq!(
            state.project().dataset().cell(ColumnId(1), 0).unwrap(),
            Cell::Number(99.0)
        );
        state.act(UiAction::ResolveUnsaved(UnsavedChoice::Cancel));
        assert!(state.pending().is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
