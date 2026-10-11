#![cfg(feature = "desktop")]
use eframe::egui::{self, Event, Key, Modifiers, Pos2, Rect, Vec2};
use tessera::{
    data::{Column, ColumnId, Dataset},
    desktop::{render, DesktopState, UiAction, Workspace},
    jobs::JobRunner,
    project::Project,
};
fn frame(
    ctx: &egui::Context,
    state: &mut DesktopState,
    jobs: &mut JobRunner,
    events: Vec<Event>,
) -> Vec<egui::OutputCommand> {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200., 800.))),
            events,
            ..Default::default()
        },
        |ui| render(ui, state, jobs),
    );
    output.textures_delta.clear();
    output.platform_output.commands
}
#[test]
fn pointer_selection_and_keyboard_movement_preserve_data() {
    let dataset = Dataset::new(vec![
        Column::numeric(ColumnId(1), "Score", vec![Some(1.23456789), Some(2.)]).unwrap(),
        Column::text(ColumnId(2), "Name", vec![Some("Carys".into()), None]).unwrap(),
    ])
    .unwrap();
    let mut state = DesktopState::from_project(Project::new("Worksheet", dataset).unwrap())
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Data))
        .unwrap();
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    let mut jobs = JobRunner::default();
    for _ in 0..3 {
        frame(&ctx, &mut state, &mut jobs, vec![]);
    }
    let pos = state.control_rect("Cell 1:0").unwrap().center();
    for pressed in [true, false] {
        frame(
            &ctx,
            &mut state,
            &mut jobs,
            vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
    }
    assert!(
        state.control_rect("Cell editor").is_none(),
        "selection must not start editing"
    );
    let copied = frame(&ctx, &mut state, &mut jobs, vec![Event::Copy]);
    assert!(copied.iter().any(
        |command| matches!(command, egui::OutputCommand::CopyText(text) if text == "1.23456789")
    ));
    frame(
        &ctx,
        &mut state,
        &mut jobs,
        vec![Event::Key {
            key: Key::ArrowRight,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    frame(
        &ctx,
        &mut state,
        &mut jobs,
        vec![Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    frame(&ctx, &mut state, &mut jobs, vec![]);
    assert!(state.control_rect("Cell editor").is_some());
    assert_eq!(
        state.project().dataset().columns()[0].cell(0).unwrap(),
        tessera::data::Cell::Number(1.23456789)
    );
}

#[test]
fn oversized_cell_copy_is_refused_before_clipboard_output() {
    let dataset = Dataset::new(vec![Column::text(
        ColumnId(1),
        "Large",
        vec![Some("x".repeat(512 * 1024 + 1))],
    )
    .unwrap()])
    .unwrap();
    let mut state = DesktopState::from_project(Project::new("Large cell", dataset).unwrap())
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Data))
        .unwrap();
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    let mut jobs = JobRunner::default();
    for _ in 0..3 {
        frame(&ctx, &mut state, &mut jobs, vec![]);
    }
    let pos = state.control_rect("Cell 1:0").unwrap().center();
    for pressed in [true, false] {
        frame(
            &ctx,
            &mut state,
            &mut jobs,
            vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
    }
    let copied = frame(&ctx, &mut state, &mut jobs, vec![Event::Copy]);
    assert!(!copied
        .iter()
        .any(|command| matches!(command, egui::OutputCommand::CopyText(_))));
    assert!(state.error().is_some());
}
