use super::*;

fn tree(
    ctx: &egui::Context,
    state: &mut DesktopState,
    jobs: &mut JobRunner,
    events: Vec<Event>,
) -> egui::accesskit::TreeUpdate {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(760., 520.))),
            events,
            ..Default::default()
        },
        |ui| render(ui, state, jobs),
    );
    output.textures_delta.clear();
    output.platform_output.accesskit_update.unwrap()
}

fn activate(
    ctx: &egui::Context,
    state: &mut DesktopState,
    jobs: &mut JobRunner,
    update: &egui::accesskit::TreeUpdate,
    label: &str,
) -> egui::accesskit::TreeUpdate {
    let id = update
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some(label))
        .unwrap()
        .0;
    tree(
        ctx,
        state,
        jobs,
        vec![Event::AccessKitActionRequest(
            egui::accesskit::ActionRequest {
                action: egui::accesskit::Action::Click,
                target_tree: egui::accesskit::TreeId::ROOT,
                target_node: id,
                data: None,
            },
        )],
    )
}

#[test]
fn interface_size_slider_exposes_its_purpose() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut state = state();
    let mut jobs = JobRunner::default();
    let root = tree(&ctx, &mut state, &mut jobs, vec![]);
    let menu = activate(&ctx, &mut state, &mut jobs, &root, "View");
    let slider = menu
        .nodes
        .iter()
        .find(|(_, node)| node.role() == egui::accesskit::Role::Slider)
        .unwrap();
    assert_eq!(slider.1.label(), Some("Text and interface size"));
    assert_eq!(slider.1.numeric_value(), Some(1.));
    assert!((slider.1.min_numeric_value().unwrap() - 0.85).abs() < 1e-7);
    assert_eq!(slider.1.max_numeric_value(), Some(1.5));
}

#[test]
fn new_project_exposes_unsaved_file_state_without_hover() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut state = DesktopState::new().unwrap();
    let root = tree(&ctx, &mut state, &mut JobRunner::default(), vec![]);
    assert!(root
        .nodes
        .iter()
        .any(|(_, node)| node.label().or(node.value()) == Some("Unsaved project")));
    assert!(!root
        .nodes
        .iter()
        .any(|(_, node)| node.label().or(node.value()) == Some("Project saved")));
}

#[test]
fn help_opens_an_offline_guide_and_restores_stable_menu_focus() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut state = state();
    let mut jobs = JobRunner::default();
    let root = tree(&ctx, &mut state, &mut jobs, vec![]);
    let menu = activate(&ctx, &mut state, &mut jobs, &root, "Help");
    assert!(menu
        .nodes
        .iter()
        .any(|(_, node)| node.label() == Some("User guide")));
    let guide = activate(&ctx, &mut state, &mut jobs, &menu, "User guide");
    assert!(guide.nodes.iter().any(|(_, node)| node
        .label()
        .is_some_and(|label| label.contains("Import and inspect"))));
    let help = state.control_id("Help").unwrap();
    tree(
        &ctx,
        &mut state,
        &mut jobs,
        vec![Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    tree(&ctx, &mut state, &mut jobs, vec![]);
    assert!(state.dialog().is_none());
    assert_eq!(ctx.memory(|memory| memory.focused()), Some(help));
}
