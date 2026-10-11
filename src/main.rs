#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
fn main() {
    if let Err(error) = tessera::desktop::launch() {
        rfd::MessageDialog::new()
            .set_title("Tessera")
            .set_description(error.to_string())
            .set_level(rfd::MessageLevel::Error)
            .show();
    }
}
