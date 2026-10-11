//! A widget edit is admitted as a transaction; browsing borrows the saved identity.
use super::*;
use egui::{text::CharIndex, TextBuffer};

pub(super) struct EditBuffer<'a> {
    original: &'a mut String,
    pending: Option<String>,
    state: &'a DesktopState,
    heap: &'a mut usize,
    error: Option<AppError>,
}
impl<'a> EditBuffer<'a> {
    pub(super) fn new(
        original: &'a mut String,
        state: &'a DesktopState,
        heap: &'a mut usize,
    ) -> Self {
        Self {
            original,
            pending: None,
            state,
            heap,
            error: None,
        }
    }
    fn refuse(&mut self, error: AppError) {
        self.error = Some(error);
        self.pending = None;
    }
    fn prepare(&mut self) -> bool {
        if self.error.is_some() {
            return false;
        }
        if self.pending.is_none() {
            if let Err(error) = admit(self.state, *self.heap, 0, self.original.len()) {
                self.refuse(error);
                return false;
            }
            self.pending = Some(self.original.clone());
        }
        true
    }
    fn grow(&mut self, required: usize) -> bool {
        let pending = self.pending.as_ref().expect("prepared edit");
        if required <= pending.capacity() {
            return true;
        }
        let overlap = self
            .original
            .capacity()
            .checked_add(pending.capacity())
            .unwrap_or(usize::MAX);
        if let Err(error) = admit(
            self.state,
            *self.heap,
            required.saturating_sub(self.original.capacity()),
            overlap,
        ) {
            self.refuse(error);
            return false;
        }
        let mut replacement = String::with_capacity(required);
        replacement.push_str(pending);
        self.pending = Some(replacement);
        true
    }
    pub(super) fn finish(self) -> AppResult<bool> {
        if let Some(error) = self.error {
            return Err(error);
        }
        let Some(pending) = self.pending else {
            return Ok(false);
        };
        if pending == *self.original {
            return Ok(false);
        }
        let next = self
            .heap
            .checked_sub(self.original.capacity())
            .and_then(|bytes| bytes.checked_add(pending.capacity()))
            .filter(|bytes| *bytes <= MAX_DRAFT_BYTES)
            .ok_or(AppError::SizeLimit {
                limit: MAX_DRAFT_BYTES,
            })?;
        *self.original = pending;
        *self.heap = next;
        Ok(true)
    }
}
impl TextBuffer for EditBuffer<'_> {
    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<EditBuffer<'static>>()
    }
    fn is_mutable(&self) -> bool {
        true
    }
    fn as_str(&self) -> &str {
        self.pending.as_deref().unwrap_or(self.original)
    }
    fn insert_text(&mut self, text: &str, index: CharIndex) -> usize {
        if self.error.is_some() || text.is_empty() {
            return 0;
        }
        if self
            .as_str()
            .chars()
            .count()
            .saturating_add(text.chars().count())
            > 512
        {
            self.refuse(AppError::invalid(
                "Declared label",
                "Use at most 512 characters.",
            ));
            return 0;
        }
        if !self.prepare() {
            return 0;
        }
        let required = self
            .as_str()
            .len()
            .checked_add(text.len())
            .unwrap_or(usize::MAX);
        if !self.grow(required) {
            return 0;
        }
        self.pending
            .as_mut()
            .expect("prepared edit")
            .insert_text(text, index)
    }
    fn replace_with(&mut self, text: &str) {
        if self.error.is_some() {
            return;
        }
        if text.chars().count() > 512 {
            self.refuse(AppError::invalid(
                "Declared label",
                "Use at most 512 characters.",
            ));
            return;
        }
        if self.prepare() {
            self.pending.as_mut().expect("prepared edit").clear();
            self.insert_text(text, CharIndex(0));
        }
    }
    fn take(&mut self) -> String {
        if !self.prepare() {
            return String::new();
        }
        let pending = self.pending.as_ref().expect("prepared edit");
        let overlap = pending
            .capacity()
            .checked_add(pending.len())
            .unwrap_or(usize::MAX);
        if let Err(error) = admit(self.state, *self.heap, 0, overlap) {
            self.refuse(error);
            return String::new();
        }
        let copied = pending.clone();
        self.pending.as_mut().expect("prepared edit").clear();
        copied
    }
    fn delete_char_range(&mut self, range: std::ops::Range<CharIndex>) {
        if self.prepare() {
            self.pending
                .as_mut()
                .expect("prepared edit")
                .delete_char_range(range);
        }
    }
}

#[cfg(test)]
mod tests;
