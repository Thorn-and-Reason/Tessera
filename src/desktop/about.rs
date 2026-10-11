//! Read-only release notices, available without modifying the document.
use super::{procedure_forms::record, DesktopState};
use eframe::egui;

const LICENSE: &str = include_str!("../../LICENSE");
pub(super) fn show(ui: &mut egui::Ui, state: &mut DesktopState) {
    ui.label(format!("Tessera {}", env!("CARGO_PKG_VERSION")));
    ui.label("A Thorn & Reason application.");
    ui.label("Copyright © 2026 Thorn & Reason. Original application code.");
    ui.label("Development candidate — public release approval pending.");
    ui.separator();
    ui.label(
        "Original application and workbook-worker source: GNU GPL version 3 only (GPL-3.0-only).",
    );
    ui.label("You may copy, modify and redistribute this software under the GNU GPL version 3. A matching source archive accompanies this local candidate.");
    ui.label("No warranty is provided, to the extent permitted by applicable law. This does not remove rights that applicable law requires.");
    ui.label("Dependencies and fonts retain their own copyright and licence notices. The distribution includes THIRD-PARTY-NOTICES.txt; Mac app resources contain those notices too.");
    let response = egui::CollapsingHeader::new("Full GPLv3 licence")
        .id_salt("application-licence")
        .show(ui, |ui| {
            let row_height =
                ui.text_style_height(&egui::TextStyle::Monospace) + ui.spacing().item_spacing.y;
            egui::ScrollArea::both()
                .id_salt("licence-text")
                .max_height(300.)
                .show_rows(ui, row_height, LICENSE.lines().count(), |ui, range| {
                    ui.set_min_width(740.);
                    for line in LICENSE.lines().skip(range.start).take(range.len()) {
                        ui.monospace(line);
                    }
                });
        });
    record(state, "Full GPLv3 licence", &response.header_response);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn licence_is_complete_and_notice_rendering_is_read_only() {
        assert!(LICENSE.contains("END OF TERMS AND CONDITIONS"));
        let mut state = DesktopState::new().unwrap();
        let project = state.project().clone();
        let token = state.token();
        let dirty = state.is_dirty();
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| show(ui, &mut state));
        output.textures_delta.clear();
        assert!(std::sync::Arc::ptr_eq(&project, state.project()));
        assert_eq!(token, state.token());
        assert_eq!(dirty, state.is_dirty());
        assert!(state.control_rect("Full GPLv3 licence").is_some());
    }
}
