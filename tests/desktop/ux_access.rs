use super::*;

fn draw(
    ctx: &egui::Context,
    state: &mut DesktopState,
    jobs: &mut JobRunner,
    size: Vec2,
    events: Vec<Event>,
) -> egui::FullOutput {
    let mut input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
        events,
        ..Default::default()
    };
    input
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .native_pixels_per_point = Some(2.);
    let mut output = ctx.run_ui(input, |ui| render(ui, state, jobs));
    output.textures_delta.clear();
    output
}

fn configured_context() -> egui::Context {
    let ctx = egui::Context::default();
    tessera::desktop::configure_context(&ctx);
    ctx.enable_accesskit();
    ctx.all_styles_mut(|style| style.animation_time = 0.);
    ctx
}

fn painted_control(
    output: &egui::FullOutput,
    state: &DesktopState,
    size: Vec2,
    label: &str,
) -> bool {
    state.control_rect(label).is_some_and(|rect| {
        if !Rect::from_min_size(Pos2::ZERO, size).contains_rect(rect) {
            return false;
        }
        output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == label)
                && shape.clip_rect.contains_rect(rect)
        })
    })
}

fn key(ctx: &egui::Context, state: &mut DesktopState, jobs: &mut JobRunner, size: Vec2, key: Key) {
    for pressed in [true, false] {
        draw(
            ctx,
            state,
            jobs,
            size,
            vec![Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
        );
    }
}

fn name(tree: &egui::accesskit::TreeUpdate, node: &egui::accesskit::Node) -> String {
    node.label().map(str::to_owned).unwrap_or_else(|| {
        node.labelled_by()
            .iter()
            .filter_map(|id| tree.nodes.iter().find(|(node_id, _)| node_id == id))
            .filter_map(|(_, label)| label.label().or_else(|| label.value()))
            .collect::<Vec<_>>()
            .join(" ")
    })
}

fn focus(
    ctx: &egui::Context,
    state: &mut DesktopState,
    jobs: &mut JobRunner,
    size: Vec2,
    target: egui::accesskit::NodeId,
) {
    draw(
        ctx,
        state,
        jobs,
        size,
        vec![Event::AccessKitActionRequest(
            egui::accesskit::ActionRequest {
                action: egui::accesskit::Action::Focus,
                target_tree: egui::accesskit::TreeId::ROOT,
                target_node: target,
                data: None,
            },
        )],
    );
}

#[test]
fn data_compact_toolbar_wraps_whole_controls() {
    for zoom in [1., 1.5] {
        let ctx = configured_context();
        ctx.set_zoom_factor(zoom);
        let size = Vec2::new(640. / zoom, 440. / zoom);
        let mut state = state();
        let mut jobs = JobRunner::default();
        for _ in 0..4 {
            draw(&ctx, &mut state, &mut jobs, size, vec![]);
        }
        let output = draw(&ctx, &mut state, &mut jobs, size, vec![]);
        let label = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Analyse data" => Some(text),
                _ => None,
            })
            .expect("Data's analysis control is painted");
        assert_eq!(label.galley.rows.len(), 1, "words fragment at zoom {zoom}");
        let painted: String = label
            .galley
            .rows
            .iter()
            .flat_map(|row| row.glyphs.iter())
            .map(|glyph| glyph.chr)
            .collect();
        assert_eq!(
            painted, "Analyse data",
            "the full action label is painted at zoom {zoom}"
        );
        for label in [
            "Import data…",
            "Add variable",
            "Analyse data",
            "Variable details",
            "Prepare data",
        ] {
            assert!(
                painted_control(&output, &state, size, label),
                "{label} stays fully within the compact viewport at zoom {zoom}: {:?}; text clips {:?}",
                state.control_rect(label),
                output.shapes.iter().filter_map(|shape| {
                    matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == label)
                        .then_some(shape.clip_rect)
                }).collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn data_compact_empty_actions_scroll_into_view() {
    for zoom in [1., 1.5] {
        let ctx = configured_context();
        ctx.set_zoom_factor(zoom);
        let size = Vec2::new(640. / zoom, 440. / zoom);
        let mut state = DesktopState::new().unwrap();
        let mut jobs = JobRunner::default();
        for _ in 0..4 {
            draw(&ctx, &mut state, &mut jobs, Vec2::new(1180., 780.), vec![]);
        }
        for _ in 0..4 {
            draw(&ctx, &mut state, &mut jobs, size, vec![]);
        }
        let mut import_visible = false;
        let mut create_visible = false;
        let mut description_visible = false;
        for _ in 0..60 {
            let output = draw(&ctx, &mut state, &mut jobs, size, vec![]);
            import_visible |= painted_control(&output, &state, size, "Import data…");
            create_visible |= painted_control(&output, &state, size, "Create first variable");
            description_visible |= output.shapes.iter().any(|shape| {
                if let egui::Shape::Text(text) = &shape.shape {
                    if text.galley.text().starts_with("Import an Excel workbook") {
                        let rect = text.galley.rect.translate(text.pos.to_vec2());
                        return Rect::from_min_size(Pos2::ZERO, size).contains_rect(rect)
                            && shape.clip_rect.contains_rect(rect);
                    }
                }
                false
            });
            if import_visible
                && create_visible
                && description_visible
                && painted_control(&output, &state, size, "Create first variable")
            {
                break;
            }
            draw(
                &ctx,
                &mut state,
                &mut jobs,
                size,
                vec![
                    Event::PointerMoved(Pos2::new(size.x * 0.5, size.y * 0.65)),
                    Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        phase: egui::TouchPhase::Move,
                        delta: Vec2::new(0., -30.),
                        modifiers: Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(import_visible, "import is unreachable at zoom {zoom}");
        assert!(create_visible, "manual entry is unreachable at zoom {zoom}");
        assert!(
            description_visible,
            "first-use explanation is unreachable at zoom {zoom}"
        );
        click(&ctx, &mut state, &mut jobs, "Create first variable", size);
        assert_eq!(state.dialog(), Some(Dialog::AddVariable));
        assert_eq!(state.project().dataset().column_count(), 0);
    }
}

#[test]
fn data_compact_first_row_is_reachable_after_defining_variable() {
    for zoom in [1., 1.5] {
        let ctx = configured_context();
        ctx.set_zoom_factor(zoom);
        let size = Vec2::new(640. / zoom, 440. / zoom);
        let mut state = DesktopState::new()
            .unwrap()
            .apply_action(UiAction::AddColumn {
                name: "First variable".into(),
                kind: tessera::data::ColumnKind::Number,
            })
            .unwrap();
        let mut jobs = JobRunner::default();
        for _ in 0..4 {
            draw(&ctx, &mut state, &mut jobs, size, vec![]);
        }
        let output = draw(&ctx, &mut state, &mut jobs, size, vec![]);
        assert!(painted_control(&output, &state, size, "Create first row"));
        click(&ctx, &mut state, &mut jobs, "Create first row", size);
        assert_eq!(state.project().dataset().row_count(), 1);
        assert!(state.control_rect("Add variable").is_some());
    }
}

#[test]
fn empty_data_omits_unavailable_actions() {
    let ctx = configured_context();
    let mut state = DesktopState::new().unwrap();
    let mut jobs = JobRunner::default();
    let output = draw(&ctx, &mut state, &mut jobs, Vec2::new(760., 520.), vec![]);
    let tree = output.platform_output.accesskit_update.unwrap();
    for irrelevant in [
        "Add row",
        "Variable details",
        "Prepare data",
        "Analyse data",
        "Instant survey report",
    ] {
        assert!(
            !tree
                .nodes
                .iter()
                .any(|(_, node)| node.label() == Some(irrelevant)),
            "empty Data hides {irrelevant}"
        );
    }
}

#[test]
fn worksheet_keyboard_exposes_current_cell_and_selection() {
    let ctx = configured_context();
    let size = Vec2::new(760., 520.);
    let mut state = state()
        .apply_action(UiAction::EditCell {
            column: ColumnId(1),
            row: 0,
            value: Cell::Number(17.125000000000004),
        })
        .unwrap();
    let mut jobs = JobRunner::default();
    draw(&ctx, &mut state, &mut jobs, size, vec![]);
    click(&ctx, &mut state, &mut jobs, "Cell 1:0", size);
    for (next_key, expected) in [
        (None, "Sgôr ŵ, row 1: 17.125000000000004"),
        (Some(Key::ArrowDown), "Sgôr ŵ, row 2: 25"),
        (Some(Key::ArrowRight), "Enw, row 2: Missing"),
        (Some(Key::ArrowUp), "Enw, row 1: Empty text"),
    ] {
        if let Some(next_key) = next_key {
            key(&ctx, &mut state, &mut jobs, size, next_key);
        }
        let output = draw(&ctx, &mut state, &mut jobs, size, vec![]);
        let tree = output.platform_output.accesskit_update.unwrap();
        let focused = tree
            .nodes
            .iter()
            .find(|(id, _)| *id == tree.focus)
            .map(|(_, node)| node)
            .expect("keyboard focus is reachable");
        let focused_name = name(&tree, focused);
        assert!(focused_name.contains("Worksheet"), "{focused_name:?}");
        assert!(
            focused_name.contains(expected),
            "{focused_name:?} misses {expected}"
        );
        let selected = tree
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some(expected))
            .map(|(_, node)| node)
            .expect("current cell is in the tree");
        assert_eq!(selected.toggled(), Some(egui::accesskit::Toggled::True));
    }
    key(&ctx, &mut state, &mut jobs, size, Key::Enter);
    assert!(state.control_rect("Cell editor").is_some());
    key(&ctx, &mut state, &mut jobs, size, Key::Escape);
    key(&ctx, &mut state, &mut jobs, size, Key::Escape);
    assert_ne!(
        ctx.memory(|memory| memory.focused()),
        Some(egui::Id::new("worksheet-focus"))
    );
    assert_eq!(
        state.project().dataset().cell(ColumnId(1), 0).unwrap(),
        Cell::Number(17.125000000000004)
    );
}

#[test]
fn worksheet_keyboard_hint_is_visible_on_focus() {
    let ctx = configured_context();
    let size = Vec2::new(760., 520.);
    let mut state = state();
    let mut jobs = JobRunner::default();
    draw(&ctx, &mut state, &mut jobs, size, vec![]);
    click(&ctx, &mut state, &mut jobs, "Cell 1:0", size);
    let output = draw(&ctx, &mut state, &mut jobs, size, vec![]);
    assert!(output.shapes.iter().any(|shape| {
        matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains("Arrow keys") && text.galley.text().contains("Enter") && text.galley.text().contains("Escape"))
    }), "worksheet keyboard help is visible");
}

#[test]
fn add_variable_dismissal_returns_to_stable_opener() {
    for workspace in [Workspace::Data, Workspace::Variables] {
        let ctx = configured_context();
        let size = Vec2::new(760., 520.);
        let mut state = state().apply_action(UiAction::Navigate(workspace)).unwrap();
        let mut jobs = JobRunner::default();
        let output = draw(&ctx, &mut state, &mut jobs, size, vec![]);
        let tree = output.platform_output.accesskit_update.unwrap();
        let target = tree
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Add variable"))
            .map(|(id, _)| *id)
            .unwrap();
        focus(&ctx, &mut state, &mut jobs, size, target);
        let opener = ctx.memory(|memory| memory.focused()).unwrap();
        key(&ctx, &mut state, &mut jobs, size, Key::Enter);
        assert_eq!(state.dialog(), Some(Dialog::AddVariable));
        let output = draw(&ctx, &mut state, &mut jobs, size, vec![]);
        let tree = output.platform_output.accesskit_update.unwrap();
        let target = tree
            .nodes
            .iter()
            .find(|(_, node)| {
                node.role() == egui::accesskit::Role::TextInput
                    && node.value() == Some("")
                    && name(&tree, node) == "Name"
            })
            .map(|(id, _)| *id)
            .unwrap();
        focus(&ctx, &mut state, &mut jobs, size, target);
        assert_ne!(ctx.memory(|memory| memory.focused()), Some(opener));
        key(&ctx, &mut state, &mut jobs, size, Key::Escape);
        draw(&ctx, &mut state, &mut jobs, size, vec![]);
        assert!(state.dialog().is_none());
        assert_eq!(ctx.memory(|memory| memory.focused()), Some(opener));
    }
}

#[test]
fn add_variable_completion_returns_to_stable_opener() {
    for workspace in [Workspace::Data, Workspace::Variables] {
        let ctx = configured_context();
        let size = Vec2::new(760., 520.);
        let mut state = state().apply_action(UiAction::Navigate(workspace)).unwrap();
        let mut jobs = JobRunner::default();
        let output = draw(&ctx, &mut state, &mut jobs, size, vec![]);
        let tree = output.platform_output.accesskit_update.unwrap();
        let target = tree
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Add variable"))
            .map(|(id, _)| *id)
            .unwrap();
        focus(&ctx, &mut state, &mut jobs, size, target);
        let opener = ctx.memory(|memory| memory.focused()).unwrap();
        key(&ctx, &mut state, &mut jobs, size, Key::Enter);
        let output = draw(&ctx, &mut state, &mut jobs, size, vec![]);
        let tree = output.platform_output.accesskit_update.unwrap();
        let (target, field_bottom) = tree
            .nodes
            .iter()
            .find(|(_, node)| {
                node.role() == egui::accesskit::Role::TextInput
                    && node.value() == Some("")
                    && name(&tree, node) == "Name"
            })
            .map(|(id, node)| (*id, node.bounds().unwrap().y1))
            .unwrap();
        focus(&ctx, &mut state, &mut jobs, size, target);
        let output = draw(
            &ctx,
            &mut state,
            &mut jobs,
            size,
            vec![Event::Text("New variable".into())],
        );
        let tree = output.platform_output.accesskit_update.unwrap();
        let target = tree
            .nodes
            .iter()
            .find(|(_, node)| {
                node.label() == Some("Add variable")
                    && node.bounds().is_some_and(|bounds| bounds.y0 > field_bottom)
            })
            .map(|(id, _)| *id)
            .unwrap();
        draw(
            &ctx,
            &mut state,
            &mut jobs,
            size,
            vec![Event::AccessKitActionRequest(
                egui::accesskit::ActionRequest {
                    action: egui::accesskit::Action::Click,
                    target_tree: egui::accesskit::TreeId::ROOT,
                    target_node: target,
                    data: None,
                },
            )],
        );
        draw(&ctx, &mut state, &mut jobs, size, vec![]);
        assert_eq!(state.project().dataset().column_count(), 3);
        assert!(state.dialog().is_none());
        assert_eq!(ctx.memory(|memory| memory.focused()), Some(opener));
    }
}

#[test]
fn property_decision_dismissal_returns_to_variable_opener() {
    let ctx = configured_context();
    let size = Vec2::new(760., 520.);
    let mut state = state()
        .apply_action(UiAction::Navigate(Workspace::Variables))
        .unwrap();
    let mut jobs = JobRunner::default();
    draw(&ctx, &mut state, &mut jobs, size, vec![]);
    ctx.memory_mut(|memory| {
        memory.request_focus(egui::Id::new(("property-description", ColumnId(1))))
    });
    draw(
        &ctx,
        &mut state,
        &mut jobs,
        size,
        vec![Event::Text("Reviewed description".into())],
    );
    assert!(state.metadata_has_changes());
    let opener = state.control_id("Variable 2").unwrap();
    ctx.memory_mut(|memory| memory.request_focus(opener));
    key(&ctx, &mut state, &mut jobs, size, Key::Enter);
    assert_eq!(state.dialog(), Some(Dialog::PropertyChanges));
    key(&ctx, &mut state, &mut jobs, size, Key::Tab);
    assert_ne!(ctx.memory(|memory| memory.focused()), Some(opener));
    key(&ctx, &mut state, &mut jobs, size, Key::Escape);
    draw(&ctx, &mut state, &mut jobs, size, vec![]);
    assert!(state.dialog().is_none());
    assert_eq!(ctx.memory(|memory| memory.focused()), Some(opener));
    assert!(state.metadata_has_changes());
}
