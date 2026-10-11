//! Borrowed direct-writer cancellation; publication limits remain with files::BoundedWriter.
use crate::{
    error::{AppError, AppResult},
    operation::JobControl,
};
use std::io::{self, Write};

pub(super) fn write(
    inner: &mut dyn Write,
    control: &JobControl,
    emit: impl FnOnce(&mut dyn Write, &JobControl) -> AppResult<()>,
) -> AppResult<()> {
    let quiet = control.without_progress();
    quiet.checkpoint(0, None)?;
    let mut writer = CancelWriter {
        inner,
        control: &quiet,
        bytes: 0,
        next_checkpoint: 4096,
        error: None,
    };
    let result = emit(&mut writer, &quiet);
    quiet.check_cancelled()?;
    match writer.error {
        Some(error) => Err(error),
        None => result,
    }
}
struct CancelWriter<'a> {
    inner: &'a mut dyn Write,
    control: &'a JobControl,
    bytes: usize,
    next_checkpoint: usize,
    error: Option<AppError>,
}
impl Write for CancelWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let check = if self.bytes >= self.next_checkpoint {
            self.next_checkpoint = self.bytes.saturating_add(4096);
            self.control.checkpoint(self.bytes, None)
        } else {
            self.control.check_cancelled()
        };
        if let Err(error) = check {
            self.error = Some(error);
            return Err(io::Error::other("Output cancelled."));
        }
        let count = self.inner.write(&bytes[..bytes.len().min(4096)])?;
        self.bytes = self
            .bytes
            .checked_add(count)
            .ok_or_else(|| io::Error::other("Output progress exceeds addressable range."))?;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
