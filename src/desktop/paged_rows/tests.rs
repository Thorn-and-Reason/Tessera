use super::*;
#[test]
fn sparse_page_renders_only_viewport_intersection_without_phantom_overscan_rows() {
    let ctx = egui::Context::default();
    let mut seen = vec![];
    let mut out = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1180., 780.),
            )),
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                show_rows(
                    ui,
                    "one-row-viewport",
                    "test rows",
                    100.,
                    64,
                    (100., 100.),
                    |ui, range| {
                        for row in range {
                            seen.push(row);
                            ui.allocate_space(egui::vec2(10., 100.));
                        }
                    },
                );
            });
        },
    );
    out.textures_delta.clear();
    assert_eq!(
        seen,
        vec![0],
        "one complete row viewport must not lay out a hidden extra row"
    );
}

fn page_frame(ctx: &egui::Context, rows: usize, offset: Option<f32>) -> (Vec<usize>, Vec<usize>) {
    let mut seen = vec![];
    let mut out = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1180., 780.),
            )),
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                if let Some(offset) = offset {
                    let id = ui.make_persistent_id((&"geometry", "keyboard-page-offset"));
                    ui.ctx().data_mut(|data| data.insert_temp(id, offset));
                }
                show_rows(
                    ui,
                    "geometry",
                    "geometry",
                    100.,
                    rows,
                    (200., 100.),
                    |ui, range| {
                        for row in range {
                            seen.push(row);
                            ui.scope(|ui| {
                                ui.set_min_height(100.);
                                ui.label(format!("Row {row}"));
                            });
                        }
                    },
                );
            });
        },
    );
    out.textures_delta.clear();
    let visible = out
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::epaint::Shape::Text(t) if s.clip_rect.contains(t.pos + t.galley.size() * 0.5) => {
                t.galley
                    .text()
                    .strip_prefix("Row ")
                    .and_then(|row| row.parse().ok())
            }
            _ => None,
        })
        .collect();
    (seen, visible)
}
#[test]
fn list_shrink_backfills_then_settles_without_stale_blank_page() {
    let ctx = egui::Context::default();
    page_frame(&ctx, 64, Some(6500.));
    let (seen, _) = page_frame(&ctx, 2, None);
    assert_eq!(
        seen,
        vec![0, 1],
        "shrink must backfill bounded range before scroll clamp"
    );
    page_frame(&ctx, 2, None);
    assert_eq!(page_frame(&ctx, 2, None).1, vec![0, 1]);
}
#[test]
fn partial_first_last_rows_and_bottom_page_are_retained() {
    let ctx = egui::Context::default();
    assert_eq!(page_frame(&ctx, 64, Some(50.)).0, vec![0, 1, 2]);
    page_frame(&ctx, 64, Some(100000.));
    page_frame(&ctx, 64, None);
    let (seen, visible) = page_frame(&ctx, 64, None);
    assert_eq!(seen, vec![62, 63]);
    assert!(visible.contains(&63));
}

#[test]
fn empty_list_does_not_reserve_space_or_render_paging_controls() {
    let ctx = egui::Context::default();
    let mut rendered = false;
    let mut consumed = 0.;
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            let before = ui.cursor().min.y;
            let controls = show_rows(
                ui,
                "empty-list",
                "empty list",
                28.,
                0,
                (168., 56.),
                |_, _| rendered = true,
            );
            consumed = ui.cursor().min.y - before;
            assert!(controls.0.is_empty());
        });
    });
    out.textures_delta.clear();
    assert!(!rendered, "empty lists should not enter the row renderer");
    assert_eq!(
        consumed, 0.,
        "empty selected lists must not leave a blank panel"
    );
}
