use crate::{error::AppResult, operation::JobControl};
use std::{path::PathBuf, time::Duration};

pub(super) fn config(control: &JobControl) -> AppResult<tessera_workbook::supervisor::Config> {
    let executable = std::env::current_exe()?;
    let worker = executable.with_file_name(if cfg!(windows) {
        "tessera-workbook-worker.exe"
    } else {
        "tessera-workbook-worker"
    });
    Ok(tessera_workbook::supervisor::Config {
        worker,
        source: PathBuf::new(),
        range: [0; 4],
        worksheet_index: 0,
        quota: 16 * 1024 * 1024,
        retained_bytes: 0,
        timeout: Duration::from_secs(60),
        cancel: control.cancellation_flag(),
        probe: None,
    })
}
