//! Debounced checkpoints and explicit recovery choices share the analysis worker.
use super::DesktopState;
use crate::{
    error::{AppError, AppResult},
    jobs::{JobOperation, JobRunner},
    operation::JobToken,
    project::Project,
    recovery::{RecoveryEntry, RecoveryStore},
};
use eframe::egui;
use std::{
    sync::{Arc, Weak},
    time::{Duration, Instant},
};
const DEBOUNCE: Duration = Duration::from_secs(2);
const UI_LIMIT: usize = 512 * 1024;
#[derive(Clone, Copy)]
enum Scheduled {
    Discover,
    Checkpoint,
    Retire,
    RetireSource,
    Recover(usize),
    Discard(usize),
}
pub(crate) struct RecoveryManager {
    store: RecoveryStore,
    entries: Vec<RecoveryEntry>,
    prompt: bool,
    observed: Weak<Project>,
    attempted: Weak<Project>,
    changed: Instant,
    sequence: u64,
    checkpoint_owned: bool,
    scheduled: Option<(Scheduled, JobToken)>,
    failed: bool,
    document: u64,
    discovery: bool,
    recovered_source: Option<RecoveryEntry>,
    cleanup_decision: bool,
}
impl RecoveryManager {
    pub(crate) fn initialize() -> AppResult<Self> {
        Self::with_store(RecoveryStore::open_native()?)
    }
    fn with_store(store: RecoveryStore) -> AppResult<Self> {
        if store.owned_bytes() > UI_LIMIT {
            return Err(AppError::SizeLimit { limit: UI_LIMIT });
        }
        Ok(Self {
            prompt: true,
            store,
            entries: Vec::new(),
            observed: Weak::new(),
            attempted: Weak::new(),
            changed: Instant::now(),
            sequence: 0,
            checkpoint_owned: false,
            scheduled: None,
            failed: false,
            document: 0,
            discovery: true,
            recovered_source: None,
            cleanup_decision: false,
        })
    }
    pub(crate) fn blocks_ui(&self) -> bool {
        self.prompt || self.cleanup_decision
    }
    fn completed(&mut self, state: &mut DesktopState) {
        let Some((_, expected)) = self.scheduled.as_ref() else {
            return;
        };
        let Some((token, success)) = state.recovery_finished else {
            return;
        };
        if token != *expected {
            return;
        }
        state.recovery_finished = None;
        let (operation, _) = self.scheduled.take().expect("matched scheduled recovery");
        self.failed = !success;
        if !success && matches!(operation, Scheduled::Retire | Scheduled::RetireSource) {
            self.cleanup_decision = true;
        }
        if success {
            match operation {
                Scheduled::Discover => {
                    self.discovery = false;
                    if let Some(entries) = state.recovery_entries.take() {
                        let bytes = entries.iter().fold(
                            self.store.owned_bytes()
                                + entries.capacity() * std::mem::size_of::<RecoveryEntry>(),
                            |n, e| n.saturating_add(e.owned_bytes()),
                        );
                        if bytes > UI_LIMIT {
                            state.fail(AppError::SizeLimit { limit: UI_LIMIT });
                            self.failed = true;
                        } else {
                            self.entries = entries;
                            self.prompt &= !self.entries.is_empty();
                        }
                    }
                }
                Scheduled::Retire => {
                    self.checkpoint_owned = false;
                    self.cleanup_decision = false;
                }
                Scheduled::RetireSource => {
                    self.recovered_source = None;
                    self.cleanup_decision = false;
                }
                Scheduled::Recover(index) => {
                    self.recovered_source = Some(self.entries.remove(index));
                    self.prompt = false;
                }
                Scheduled::Discard(index) => {
                    self.entries.remove(index);
                    self.prompt = !self.entries.is_empty();
                }
                Scheduled::Checkpoint => {}
            }
        }
    }
    fn start(
        &mut self,
        state: &mut DesktopState,
        jobs: &mut JobRunner,
        operation: JobOperation,
        scheduled: Scheduled,
    ) -> bool {
        match state.start_operation(jobs, operation) {
            Ok(()) => {
                self.scheduled = Some((scheduled, state.token()));
                self.failed = false;
                true
            }
            Err(error) => {
                state.fail(error);
                self.failed = true;
                false
            }
        }
    }
    fn cleanup(&mut self, state: &mut DesktopState, jobs: &mut JobRunner) -> bool {
        if !self.checkpoint_owned && self.recovered_source.is_none() {
            return true;
        }
        if state.active {
            return false;
        }
        if self.failed {
            self.cleanup_decision = true;
            return false;
        }
        let (operation, scheduled) = if self.checkpoint_owned {
            (
                JobOperation::RetireCheckpoint {
                    store: self.store.clone(),
                },
                Scheduled::Retire,
            )
        } else {
            (
                JobOperation::DiscardRecovery {
                    store: self.store.clone(),
                    entry: self.recovered_source.as_ref().unwrap().clone(),
                },
                Scheduled::RetireSource,
            )
        };
        if !self.start(state, jobs, operation, scheduled) {
            self.cleanup_decision = true;
        }
        false
    }
    pub(crate) fn before_transition(
        &mut self,
        state: &mut DesktopState,
        jobs: &mut JobRunner,
    ) -> bool {
        self.completed(state);
        if state.pending_ready {
            return self.cleanup(state, jobs);
        }
        !self.cleanup_decision
    }
    fn keep_and_continue(&mut self, state: &mut DesktopState) {
        // Keep the old session immutable; subsequent documents use a fresh checkpoint.
        let parent = self
            .store
            .path()
            .parent()
            .and_then(std::path::Path::parent)
            .unwrap();
        match RecoveryStore::open_at(parent) {
            Ok(store) => {
                self.store = store;
                self.checkpoint_owned = false;
                self.recovered_source = None;
                self.cleanup_decision = false;
                self.failed = false;
            }
            Err(error) => state.fail(error),
        }
    }
    pub(crate) fn tick(&mut self, state: &mut DesktopState, jobs: &mut JobRunner) {
        self.tick_at(state, jobs, Instant::now());
    }
    fn tick_at(&mut self, state: &mut DesktopState, jobs: &mut JobRunner, now: Instant) {
        self.completed(state);
        if self.discovery && self.prompt && !state.active && !self.failed {
            self.start(
                state,
                jobs,
                JobOperation::DiscoverRecovery {
                    store: self.store.clone(),
                },
                Scheduled::Discover,
            );
            return;
        }
        if self.document != state.document_id() {
            self.document = state.document_id();
            self.observed = Weak::new();
            self.attempted = Weak::new();
        }
        let current = Arc::downgrade(state.project());
        if !Weak::ptr_eq(&self.observed, &current) {
            self.observed = current.clone();
            self.changed = now;
        }
        if state.active || state.shutdown || state.pending_ready || self.prompt || self.failed {
            return;
        }
        if !state.is_dirty() {
            self.cleanup(state, jobs);
            return;
        }
        if now.saturating_duration_since(self.changed) < DEBOUNCE
            || Weak::ptr_eq(&self.attempted, &current)
        {
            return;
        }
        let Some(sequence) = self.sequence.checked_add(1) else {
            state.fail(AppError::IdentityExhausted);
            self.failed = true;
            return;
        };
        if self.start(
            state,
            jobs,
            JobOperation::Checkpoint {
                project: state.project().clone(),
                store: self.store.clone(),
                sequence,
            },
            Scheduled::Checkpoint,
        ) {
            self.sequence = sequence;
            self.attempted = current;
            // An uncertain publication must also be retired before leaving the document.
            self.checkpoint_owned = true;
        }
    }
    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        state: &mut DesktopState,
        jobs: &mut JobRunner,
    ) {
        if self.cleanup_decision {
            egui::Modal::new(egui::Id::new("recovery-cleanup")).show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 64.0).clamp(260.0, 440.0));
                ui.heading("Recovery files could not be removed");
                egui::ScrollArea::vertical()
                    .id_salt("recovery-cleanup-details")
                    .max_height((ctx.content_rect().height() - 140.0).max(80.0))
                    .show(ui, |ui| {
                        if let Some(message) = state.error() {
                            ui.label(message);
                        }
                        if let Some(recovery) = state.recovery_receipt().and_then(|outcome| outcome.recovery.as_ref()) {
                            ui.label("Copy the recovery folder path to review the retained files before continuing.");
                            super::dialogs::path_detail(ui, "Recovery", &recovery.directory);
                            for (label, path) in [
                                ("Previous version", &recovery.displaced),
                                ("Saved revision", &recovery.frozen_revision),
                                ("Prepared file", &recovery.prepared),
                            ] {
                                if let Some(path) = path {
                                    super::dialogs::path_detail(ui, label, path);
                                }
                            }
                        }
                        ui.label("Keeping recovery files lets you continue without removing them. Retained copies may need manual recovery.");
                    });
                if ui
                    .add_enabled(!state.active, egui::Button::new("Retry"))
                    .clicked()
                {
                    self.failed = false;
                    self.cleanup_decision = false;
                }
                if ui
                    .add_enabled(
                        !state.active,
                        egui::Button::new("Keep recovery files and continue"),
                    )
                    .clicked()
                {
                    self.keep_and_continue(state);
                }
            });
            return;
        }
        if self.failed && !self.prompt {
            egui::Window::new("Recovery unavailable")
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label("Your project is unchanged.");
                    if ui
                        .add_enabled(!state.active, egui::Button::new("Retry"))
                        .clicked()
                    {
                        self.failed = false;
                        self.attempted = Weak::new();
                    }
                });
        }
        if !self.prompt {
            return;
        }
        egui::Popup::close_all(ctx);
        egui::Modal::new(egui::Id::new("recovery-choice")).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.heading("Recover unsaved work");
            let safe = state.document_id() == 1
                && state.pending().is_none()
                && state.dialog().is_none()
                && state.project().dataset().columns().is_empty()
                && !state.active
                && !state.is_dirty()
                && state.project().dataset().row_count() == 0
                && state.project().runs().is_empty();
            let mut action = None;
            egui::ScrollArea::vertical()
                .max_height(300.0)
                .show(ui, |ui| {
                    for index in 0..self.entries.len() {
                        ui.horizontal(|ui| {
                            ui.label(format!("Recovered project {}", index + 1));
                            if ui.add_enabled(safe, egui::Button::new("Recover")).clicked() {
                                action = Some((index, true));
                            }
                            if ui
                                .add_enabled(!state.active, egui::Button::new("Discard"))
                                .clicked()
                            {
                                action = Some((index, false));
                            }
                        });
                    }
                });
            if let Some((index, recover)) = action {
                let entry = self.entries[index].clone();
                let (operation, scheduled) = if recover {
                    (
                        JobOperation::Recover {
                            store: self.store.clone(),
                            entry,
                        },
                        Scheduled::Recover(index),
                    )
                } else {
                    (
                        JobOperation::DiscardRecovery {
                            store: self.store.clone(),
                            entry,
                        },
                        Scheduled::Discard(index),
                    )
                };
                self.start(state, jobs, operation, scheduled);
            }
            if self.discovery {
                ui.label("Looking for unsaved work…");
            }
            if self.failed {
                if let Some(message) = state.error() {
                    ui.label(message);
                }
                if ui.button("Retry").clicked() {
                    self.failed = false;
                }
            }
            if ui.button("Later").clicked() {
                if self.discovery && state.active {
                    jobs.cancel();
                }
                self.prompt = false;
            }
        });
    }
}
// Published recovery checkpoints require verified directory durability (Unix only).
#[cfg(all(test, unix))]
mod tests {
    mod lifecycle;
    use super::*;
    use crate::{data::Dataset, operation::JobControl};
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            Self(std::env::temp_dir().canonicalize().unwrap().join(format!(
                "tessera-ui-recovery-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )))
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn finish(state: &mut DesktopState, jobs: &mut JobRunner, manager: &mut RecoveryManager) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while state.active {
            state.poll(jobs);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        manager.completed(state);
        assert!(state.error().is_none(), "{:?}", state.error());
    }
    #[test]
    fn discovery_is_worker_owned_and_does_not_adopt_a_project() {
        let temp = Temp::new();
        let old = RecoveryStore::open_at(&temp.0).unwrap();
        old.checkpoint(
            &Project::new("Unsaved", Dataset::new(vec![]).unwrap()).unwrap(),
            1,
            &JobControl::default(),
        )
        .unwrap();
        drop(old);
        let mut manager =
            RecoveryManager::with_store(RecoveryStore::open_at(&temp.0).unwrap()).unwrap();
        assert!(manager.entries.is_empty());
        let mut state = DesktopState::new().unwrap();
        let before = state.project().clone();
        let mut jobs = JobRunner::default();
        manager.tick(&mut state, &mut jobs);
        assert!(state.active);
        finish(&mut state, &mut jobs, &mut manager);
        assert_eq!(manager.entries.len(), 1);
        assert!(manager.blocks_ui());
        assert!(Arc::ptr_eq(&before, state.project()));
        assert!(!state.is_dirty());
    }
    #[test]
    fn checkpoint_debounces_once_and_retirement_preserves_dirty_identity() {
        let temp = Temp::new();
        let store = RecoveryStore::open_at(&temp.0).unwrap();
        let mut manager = RecoveryManager::with_store(store.clone()).unwrap();
        manager.discovery = false;
        manager.prompt = false;
        let mut state = DesktopState::from_project(
            Project::new("Unsaved", Dataset::new(vec![]).unwrap()).unwrap(),
        )
        .unwrap();
        let before = state.project().clone();
        let mut jobs = JobRunner::default();
        let now = Instant::now();
        manager.tick_at(&mut state, &mut jobs, now);
        manager.tick_at(&mut state, &mut jobs, now + Duration::from_millis(1999));
        assert!(!state.active);
        manager.tick_at(&mut state, &mut jobs, now + DEBOUNCE);
        assert!(state.active);
        finish(&mut state, &mut jobs, &mut manager);
        assert!(store.path().exists());
        assert!(state.is_dirty());
        assert!(Arc::ptr_eq(&before, state.project()));
        manager.tick_at(&mut state, &mut jobs, now + DEBOUNCE * 2);
        assert!(!state.active);
        state.pending_ready = true;
        manager.before_transition(&mut state, &mut jobs);
        assert!(state.active);
        finish(&mut state, &mut jobs, &mut manager);
        assert!(!manager.checkpoint_owned);
        assert!(state.is_dirty());
        assert!(Arc::ptr_eq(&before, state.project()));
    }
    #[test]
    fn cancelling_discovery_and_later_keeps_existing_work() {
        let temp = Temp::new();
        let mut manager =
            RecoveryManager::with_store(RecoveryStore::open_at(&temp.0).unwrap()).unwrap();
        let mut state = DesktopState::new().unwrap();
        let mut jobs = JobRunner::default();
        manager.tick(&mut state, &mut jobs);
        manager.prompt = false;
        jobs.cancel();
        let deadline = Instant::now() + Duration::from_secs(10);
        while state.active {
            state.poll(&mut jobs);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        manager.completed(&mut state);
        assert!(!manager.blocks_ui());
        assert!(!state.is_dirty());
    }
    #[test]
    fn recovery_completion_matches_its_token_even_during_another_job() {
        let temp = Temp::new();
        let mut manager =
            RecoveryManager::with_store(RecoveryStore::open_at(&temp.0).unwrap()).unwrap();
        let mut state = DesktopState::new().unwrap();
        let expected = state.token();
        manager.checkpoint_owned = true;
        manager.scheduled = Some((Scheduled::Retire, expected));
        state.active = true;
        let other = JobToken {
            generation: expected.generation + 1,
            ..expected
        };
        state.recovery_finished = Some((other, true));
        manager.completed(&mut state);
        assert!(manager.checkpoint_owned);
        assert!(manager.scheduled.is_some());
        state.recovery_finished = Some((expected, false));
        manager.completed(&mut state);
        assert!(manager.checkpoint_owned);
        assert!(manager.failed);
        assert!(manager.scheduled.is_none());
        manager.scheduled = Some((Scheduled::Retire, expected));
        state.recovery_finished = Some((expected, true));
        manager.completed(&mut state);
        assert!(!manager.checkpoint_owned);
        assert!(!manager.failed);
        assert!(state.recovery_finished.is_none());
        assert!(state.active);
    }
    #[test]
    fn recovered_source_is_claimed_until_save_then_retired() {
        let temp = Temp::new();
        let old = RecoveryStore::open_at(&temp.0).unwrap();
        old.checkpoint(
            &Project::new("Unsaved", Dataset::new(vec![]).unwrap()).unwrap(),
            1,
            &JobControl::default(),
        )
        .unwrap();
        drop(old);
        let mut manager =
            RecoveryManager::with_store(RecoveryStore::open_at(&temp.0).unwrap()).unwrap();
        let mut state = DesktopState::new().unwrap();
        let mut jobs = JobRunner::default();
        manager.tick(&mut state, &mut jobs);
        finish(&mut state, &mut jobs, &mut manager);
        let source_path = manager.entries[0].path.clone();
        assert!(manager.start(
            &mut state,
            &mut jobs,
            JobOperation::Recover {
                store: manager.store.clone(),
                entry: manager.entries[0].clone()
            },
            Scheduled::Recover(0)
        ));
        finish(&mut state, &mut jobs, &mut manager);
        assert!(state.is_dirty());
        assert!(manager.recovered_source.is_some());
        assert!(source_path.exists());
        let other = RecoveryStore::open_at(&temp.0).unwrap();
        assert!(other.list().unwrap().is_empty());
        let path = temp.0.join("saved.tessera");
        state
            .start_operation(
                &mut jobs,
                JobOperation::Save {
                    project: state.project().clone(),
                    path,
                    expected: None,
                },
            )
            .unwrap();
        finish(&mut state, &mut jobs, &mut manager);
        assert!(!state.is_dirty());
        manager.tick(&mut state, &mut jobs);
        assert!(state.active);
        finish(&mut state, &mut jobs, &mut manager);
        assert!(manager.recovered_source.is_none());
        assert!(other.list().unwrap().is_empty());
    }
    #[test]
    fn failed_cleanup_blocks_new_and_close_until_explicit_keep() {
        for action in [
            super::super::PendingAction::New,
            super::super::PendingAction::Close,
        ] {
            let temp = Temp::new();
            let store = RecoveryStore::open_at(&temp.0).unwrap();
            store
                .checkpoint(
                    &Project::new("Checkpoint", Dataset::new(vec![]).unwrap()).unwrap(),
                    1,
                    &JobControl::default(),
                )
                .unwrap();
            let old_path = store.path().to_owned();
            let mut manager = RecoveryManager::with_store(store).unwrap();
            manager.prompt = false;
            manager.discovery = false;
            manager.checkpoint_owned = true;
            std::fs::write(&old_path, b"externally changed checkpoint").unwrap();
            let mut state = DesktopState::new().unwrap();
            let before = state.project().clone();
            state.act(super::super::UiAction::Request(action));
            let mut jobs = JobRunner::default();
            assert!(!manager.before_transition(&mut state, &mut jobs));
            let deadline = Instant::now() + Duration::from_secs(10);
            while state.active {
                state.poll(&mut jobs);
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
            manager.completed(&mut state);
            assert!(state.error().is_some());
            assert!(!manager.before_transition(&mut state, &mut jobs));
            assert!(manager.blocks_ui());
            assert!(Arc::ptr_eq(&before, state.project()));
            assert!(!state.shutdown);
            manager.keep_and_continue(&mut state);
            assert!(manager.before_transition(&mut state, &mut jobs));
            assert_ne!(manager.store.path(), old_path);
            state.process_pending(&mut jobs);
            assert!(old_path.exists());
            assert!(state.shutdown || !Arc::ptr_eq(&before, state.project()));
        }
    }
    #[test]
    fn explicit_discard_retires_recovered_source_before_new_document() {
        let temp = Temp::new();
        let old = RecoveryStore::open_at(&temp.0).unwrap();
        old.checkpoint(
            &Project::new("Unsaved", Dataset::new(vec![]).unwrap()).unwrap(),
            1,
            &JobControl::default(),
        )
        .unwrap();
        drop(old);
        let mut manager =
            RecoveryManager::with_store(RecoveryStore::open_at(&temp.0).unwrap()).unwrap();
        let mut state = DesktopState::new().unwrap();
        let mut jobs = JobRunner::default();
        manager.tick(&mut state, &mut jobs);
        finish(&mut state, &mut jobs, &mut manager);
        assert!(manager.start(
            &mut state,
            &mut jobs,
            JobOperation::Recover {
                store: manager.store.clone(),
                entry: manager.entries[0].clone()
            },
            Scheduled::Recover(0)
        ));
        finish(&mut state, &mut jobs, &mut manager);
        let document = state.document_id();
        state.act(super::super::UiAction::Request(
            super::super::PendingAction::New,
        ));
        state.act(super::super::UiAction::ResolveUnsaved(
            super::super::UnsavedChoice::Discard,
        ));
        assert!(!manager.before_transition(&mut state, &mut jobs));
        assert_eq!(state.document_id(), document);
        finish(&mut state, &mut jobs, &mut manager);
        assert!(manager.before_transition(&mut state, &mut jobs));
        state.process_pending(&mut jobs);
        assert_ne!(state.document_id(), document);
        assert!(RecoveryStore::open_at(&temp.0)
            .unwrap()
            .list()
            .unwrap()
            .is_empty());
    }
    #[test]
    fn failed_open_after_keep_does_not_bypass_later_checkpoint_retirement() {
        use super::super::{PendingAction, UiAction, UnsavedChoice};
        let temp = Temp::new();
        let mut manager =
            RecoveryManager::with_store(RecoveryStore::open_at(&temp.0).unwrap()).unwrap();
        manager.prompt = false;
        manager.discovery = false;
        manager.checkpoint_owned = true;
        manager.failed = true;
        let mut state = DesktopState::new().unwrap();
        manager.document = state.document_id();
        let mut jobs = JobRunner::default();
        let document = state.document_id();
        state.act(UiAction::Request(PendingAction::Open(
            temp.0.join("missing.tessera"),
        )));
        assert!(!manager.before_transition(&mut state, &mut jobs));
        manager.keep_and_continue(&mut state);
        assert!(manager.before_transition(&mut state, &mut jobs));
        state.process_pending(&mut jobs);
        let deadline = Instant::now() + Duration::from_secs(10);
        while state.active {
            state.poll(&mut jobs);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(state.error().is_some());
        assert_eq!(state.document_id(), document);
        state.act(UiAction::AddColumn {
            name: "Score".into(),
            kind: crate::data::ColumnKind::Number,
        });
        let now = Instant::now();
        manager.tick_at(&mut state, &mut jobs, now);
        manager.tick_at(&mut state, &mut jobs, now + DEBOUNCE);
        finish(&mut state, &mut jobs, &mut manager);
        assert!(manager.checkpoint_owned);
        assert!(manager.store.path().exists());
        state.act(UiAction::Request(PendingAction::Close));
        state.act(UiAction::ResolveUnsaved(UnsavedChoice::Discard));
        assert!(!manager.before_transition(&mut state, &mut jobs));
        assert!(state.active);
        assert!(!state.shutdown);
        finish(&mut state, &mut jobs, &mut manager);
        assert!(manager.before_transition(&mut state, &mut jobs));
        state.process_pending(&mut jobs);
        assert!(state.shutdown);
        assert!(RecoveryStore::open_at(&temp.0)
            .unwrap()
            .list()
            .unwrap()
            .is_empty());
    }
}
