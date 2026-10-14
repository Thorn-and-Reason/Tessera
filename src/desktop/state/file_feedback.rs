//! Separate user file receipts, recovery warnings and bounded diagnostic details.
use super::*;
use std::{fmt, path::Path};

#[derive(Debug, Clone, Copy)]
pub(super) enum FileOutcomeKind {
    Project,
    Export,
    Recipe,
}
const ERROR_DETAIL_BYTES: usize = 32 * 1024;

pub(super) fn bounded_error(error: impl fmt::Display) -> String {
    struct Bounded(String);
    impl fmt::Write for Bounded {
        fn write_str(&mut self, text: &str) -> fmt::Result {
            let room = ERROR_DETAIL_BYTES - 3 - self.0.len();
            if text.len() <= room {
                self.0.push_str(text);
                return Ok(());
            }
            let mut end = room;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            self.0.push_str(&text[..end]);
            self.0.push('…');
            Err(fmt::Error)
        }
    }
    use fmt::Write;
    let mut result = Bounded(String::new());
    let _ = write!(result, "{error}");
    result.0
}

impl DesktopState {
    pub(crate) fn bounded_file_warning(&self, warning: &str) -> String {
        bounded_error(warning)
    }
    pub fn file_receipt(&self) -> Option<&SaveOutcome> {
        self.file_outcome.as_ref()
    }
    pub fn recovery_receipt(&self) -> Option<&SaveOutcome> {
        self.recovery_outcome.as_ref()
    }
    pub(crate) fn has_file_receipt(&self) -> bool {
        self.file_outcome.is_some()
    }
    pub(crate) fn has_recovery_receipt(&self) -> bool {
        self.recovery_outcome.is_some()
    }
    pub(crate) fn show_file_receipt(&mut self) {
        self.show_recovery_outcome = false;
        self.dialog = Some(Dialog::FileOutcome);
    }
    pub(crate) fn show_recovery_receipt(&mut self) {
        self.show_recovery_outcome = true;
        self.dialog = Some(Dialog::FileOutcome);
    }
    pub(crate) fn shown_file_receipt(&self) -> Option<&SaveOutcome> {
        if self.show_recovery_outcome {
            self.recovery_receipt()
        } else {
            self.file_receipt()
        }
    }
    pub(crate) fn file_receipt_label(&self) -> &'static str {
        if self.show_recovery_outcome {
            "Recovery update"
        } else {
            match self.file_outcome_kind {
                FileOutcomeKind::Project => "Project save",
                FileOutcomeKind::Export => "File export",
                FileOutcomeKind::Recipe => "Survey recipe save",
            }
        }
    }
    pub(crate) fn file_receipt_can_save_as(&self) -> bool {
        !self.show_recovery_outcome && matches!(self.file_outcome_kind, FileOutcomeKind::Project)
    }
    pub fn error_details(&self) -> Option<&str> {
        self.error()
    }
    pub(crate) fn show_error_details(&mut self) {
        self.dialog = Some(Dialog::ErrorDetails);
    }
    pub(crate) fn save_state_label(&self) -> &'static str {
        if self.path.is_none() {
            "Unsaved project"
        } else if self.is_dirty() {
            "Unsaved changes"
        } else {
            "Project saved"
        }
    }
    pub(crate) fn expected_save_stamp(&self, path: &Path) -> Option<FileStamp> {
        (self.path.as_deref() == Some(path))
            .then(|| self.stamp.clone())
            .flatten()
    }
    fn import_is_active(&self) -> bool {
        self.active
            && matches!(
                self.operation_name,
                "Inspect Excel" | "Excel preview" | "Excel import" | "Import preview" | "Import"
            )
    }
    pub(crate) fn request_cancel(&mut self, jobs: &mut JobRunner) {
        if !self.active || self.cancelled {
            return;
        }
        if self.import_is_active() {
            self.abandoned_import = Some(self.token());
            self.cancel_survey_import();
        }
        jobs.cancel();
        self.cancelled = true;
        self.status = "Cancelling…".into();
    }
    pub(crate) fn dismiss_import(&mut self, jobs: &mut JobRunner) {
        self.request_cancel(jobs);
        self.cancel_survey_import();
        self.dialog = None;
        // Worker-retained previews remain owned until completion.
        self.preview = None;
        self.excel = Default::default();
        self.choices = Arc::new(vec![]);
    }
}

pub(super) fn receipt_bytes(outcome: &SaveOutcome) -> usize {
    let mut bytes = outcome.destination.capacity();
    bytes = bytes.saturating_add(outcome.warning.as_ref().map_or(0, String::capacity));
    if let Some(paths) = &outcome.recovery {
        bytes = bytes.saturating_add(paths.directory.capacity());
        for path in [&paths.frozen_revision, &paths.displaced, &paths.prepared]
            .into_iter()
            .flatten()
        {
            bytes = bytes.saturating_add(path.capacity());
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oversized_error_details_stay_bounded_and_valid_utf8() {
        let mut state = DesktopState::new().unwrap();
        state.fail(AppError::invalid("export", "ŵ".repeat(100_000)));
        let details = state.error_details().unwrap();
        assert!(details.len() <= 32 * 1024);
        assert!(details.starts_with("export:"));
        assert!(details.ends_with('…'));
        assert!(details.is_char_boundary(details.len()));
    }
    #[test]
    fn same_project_save_as_keeps_stamp_guard_against_external_changes() {
        let directory = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "tessera-save-as-stamp-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("project.tessera");
        let mut state = DesktopState::new().unwrap();
        let outcome =
            crate::files::save_project(&path, state.project(), None, &Default::default()).unwrap();
        state
            .handle_event(JobEvent::Completed {
                token: state.token(),
                cancelled: false,
                result: Ok(JobOutput::Saved(outcome)),
            })
            .unwrap();
        let expected = state.expected_save_stamp(&path).unwrap();
        assert!(state
            .expected_save_stamp(&directory.join("different.tessera"))
            .is_none());
        let outcome = crate::files::save_project(
            &path,
            state.project(),
            Some(&expected),
            &Default::default(),
        )
        .unwrap();
        assert_eq!(outcome.publication, crate::files::Publication::Committed);
        let current = outcome.committed_stamp.unwrap();
        state.stamp = Some(current);
        std::fs::write(&path, "External writer's changed project").unwrap();
        let error = crate::files::save_project(
            &path,
            state.project(),
            state.expected_save_stamp(&path).as_ref(),
            &Default::default(),
        )
        .unwrap_err();
        assert!(matches!(error, AppError::Conflict(_)));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "External writer's changed project"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
