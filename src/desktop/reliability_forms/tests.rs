use super::*;
use crate::{
    analysis::*,
    data::{Column, Dataset},
    project::Project,
};
use egui::{Event, Key, Modifiers, PointerButton};
struct Form {
    ctx: egui::Context,
    state: DesktopState,
    draft: ReliabilityDraft,
    size: egui::Vec2,
}
impl Form {
    fn new() -> Self {
        let data = Dataset::new(
            (1..=4)
                .map(|i| {
                    Column::numeric(ColumnId(i), &format!("Item {i}"), vec![Some(0.), Some(1.)])
                        .unwrap()
                })
                .collect(),
        )
        .unwrap();
        Self {
            ctx: egui::Context::default(),
            state: DesktopState::from_project(Project::new("Controls", data).unwrap()).unwrap(),
            draft: ReliabilityDraft {
                items: vec![ColumnId(2), ColumnId(1)],
                ..Default::default()
            },
            size: egui::vec2(288., 780.),
        }
    }
    fn frame(&mut self, events: Vec<Event>) {
        self.state.controls.clear();
        self.state.control_ids.clear();
        let mut o = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default()
                    .show(ui, |ui| settings(ui, &mut self.state, &mut self.draft));
            },
        );
        o.textures_delta.clear();
    }
    fn click(&mut self, label: &str) {
        let pos = self
            .state
            .control_rect(label)
            .unwrap_or_else(|| panic!("missing {label}"))
            .center();
        for pressed in [true, false] {
            self.frame(vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ]);
        }
        self.frame(vec![]);
    }
    fn key(&mut self, key: Key, modifiers: Modifiers) {
        for pressed in [true, false] {
            self.frame(vec![Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            }]);
        }
    }
    fn text(&mut self, label: &str, text: &str) {
        self.click(label);
        self.key(Key::A, Modifiers::COMMAND);
        self.frame(vec![Event::Paste(text.into())]);
        self.frame(vec![]);
    }
}
#[test]
fn reliability_actual_controls_preserve_order_explicit_bounds_and_removal() {
    let mut f = Form::new();
    f.frame(vec![]);
    f.click("Move item 2 down");
    assert_eq!(f.draft.items, vec![ColumnId(1), ColumnId(2)]);
    f.click("Reverse item 1");
    assert!(f.draft.build_request(f.state.project().dataset()).is_err());
    f.text("Lower bound: 1", "0");
    f.text("Upper bound: 1", "5");
    let r = f.draft.build_request(f.state.project().dataset()).unwrap();
    assert_eq!(
        r.reversals,
        vec![ReliabilityReverse {
            column: ColumnId(1),
            lower: 0.,
            upper: 5.
        }]
    );
    f.text("Lower bound: 1", "NaN");
    assert!(f.draft.build_request(f.state.project().dataset()).is_err());
    f.text("Lower bound: 1", "0");
    f.click("Remove item 1");
    assert_eq!(f.draft.items, vec![ColumnId(2)]);
    assert!(f.draft.reversals.is_empty());
    f.click("Add item");
    assert!(f.state.control_rect("Add item: 1").is_some());
    f.click("Add item: 3");
    assert_eq!(f.draft.items, vec![ColumnId(2), ColumnId(3)]);
    assert!(f.draft.build_request(f.state.project().dataset()).is_ok());
}
#[test]
fn reliability_keyboard_activates_controls_and_narrow_form_preserves_identity() {
    let mut f = Form::new();
    f.frame(vec![]);
    let id = f.state.control_ids["Move item 2 down"];
    f.ctx.memory_mut(|m| m.request_focus(id));
    f.key(Key::Enter, Modifiers::NONE);
    assert_eq!(f.draft.items, vec![ColumnId(1), ColumnId(2)]);
    let remove = f.state.control_ids["Remove item 1"];
    f.ctx.memory_mut(|m| m.request_focus(remove));
    f.key(Key::Tab, Modifiers::NONE);
    let expected = f.state.control_ids["Reverse item 1"];
    assert_eq!(f.ctx.memory(|m| m.focused()), Some(expected));
    f.key(Key::Space, Modifiers::NONE);
    assert_eq!(f.draft.reversals.len(), 1);
    let before = f.draft.clone();
    f.size = egui::vec2(240., 520.);
    f.frame(vec![]);
    assert_eq!(f.draft, before);
    assert!(f.state.error().is_none());
}
#[test]
fn reliability_growth_refuses_before_allocation() {
    let mut s = DesktopState::new().unwrap();
    let d = ReliabilityDraft::default();
    s.active_recipe_bytes = crate::data::MAX_OWNED_BYTES;
    assert!(!admit(&mut s, &d, 64, 0));
    assert!(s.error().is_some());
}

#[test]
fn reliability_search_reaches_final_numeric_identity_beyond_catalog_cap() {
    let mut f = Form::new();
    let mut columns = (1..=300)
        .map(|i| {
            Column::numeric(
                ColumnId(i),
                &format!("Column {i}"),
                vec![Some(0.), Some(1.)],
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    columns
        .push(Column::numeric(ColumnId(301), "Final numeric ŵ", vec![Some(0.), Some(1.)]).unwrap());
    columns.push(
        Column::text(
            ColumnId(302),
            "Final text",
            vec![Some("x".into()), Some("y".into())],
        )
        .unwrap(),
    );
    f.state = DesktopState::from_project(
        Project::new("Wide search", Dataset::new(columns).unwrap()).unwrap(),
    )
    .unwrap();
    f.frame(vec![]);
    f.click("Add item");
    assert!(f.state.control_rect("Add item: 301").is_none());
    f.text("Search numeric items", "Final");
    assert!(f.state.control_rect("Add item: 301").is_some());
    assert!(f.state.control_rect("Add item: 302").is_none());
    f.click("Add item: 301");
    assert_eq!(f.draft.items, vec![ColumnId(2), ColumnId(1), ColumnId(301)]);
    assert!(f.draft.build_request(f.state.project().dataset()).is_ok());
}

#[test]
fn reliability_pages_reach_final_item_identity_after_256() {
    let mut f = Form::new();
    let columns = (1..=300)
        .map(|i| {
            Column::numeric(
                ColumnId(i),
                &format!("{}-{i}", "Same name".repeat(64)),
                vec![Some(0.), Some(1.)],
            )
            .unwrap()
        })
        .collect();
    f.state = DesktopState::from_project(
        Project::new("Wide item pages", Dataset::new(columns).unwrap()).unwrap(),
    )
    .unwrap();
    f.frame(vec![]);
    f.click("Add item");
    for _ in 0..100 {
        if f.state.control_rect("Add item: 300").is_some() {
            break;
        }
        let id = f.state.control_ids["Next available items page"];
        f.ctx.memory_mut(|m| m.request_focus(id));
        f.key(Key::Enter, Modifiers::NONE);
    }
    assert!(f.state.control_rect("Add item: 300").is_some());
    f.click("Add item: 300");
    assert_eq!(f.draft.items, vec![ColumnId(2), ColumnId(1), ColumnId(300)]);
}

#[test]
fn reliability_accessibility_preserves_checked_unchecked_and_disabled_reversal_state() {
    for enabled in [true, false] {
        let mut f = Form::new();
        f.ctx.enable_accesskit();
        f.draft.reversals.push(ReverseDraft {
            column: ColumnId(2),
            lower: "0".into(),
            upper: "1".into(),
        });
        let before = f.draft.clone();
        let mut out = f.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, f.size)),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.add_enabled_ui(enabled, |ui| settings(ui, &mut f.state, &mut f.draft));
                });
            },
        );
        out.textures_delta.clear();
        let update = out
            .platform_output
            .accesskit_update
            .expect("real accessibility update");
        for (label, checked) in [("Reverse item 2", true), ("Reverse item 1", false)] {
            let node = update
                .nodes
                .iter()
                .find_map(|(_, n)| (n.label() == Some(label)).then_some(n))
                .unwrap_or_else(|| panic!("missing {label}"));
            assert_eq!(node.role(), egui::accesskit::Role::CheckBox);
            assert_eq!(
                node.toggled(),
                Some(if checked {
                    egui::accesskit::Toggled::True
                } else {
                    egui::accesskit::Toggled::False
                }),
                "{label} must expose its real checked state"
            );
            assert_eq!(
                node.is_disabled(),
                !enabled,
                "{label} must expose its real enabled state"
            );
        }
        assert_eq!(f.draft, before);
    }
}

#[test]
fn bulk_items_preserve_order_and_refuse_over_64() {
    let mut f = Form::new();
    f.frame(vec![]);
    f.click("Add item");
    f.click("Select all items");
    assert_eq!(
        f.draft.items,
        [ColumnId(2), ColumnId(1), ColumnId(3), ColumnId(4)]
    );
    let data = Dataset::new(
        (1..=65)
            .map(|id| Column::numeric(ColumnId(id), &format!("Item {id}"), vec![]).unwrap())
            .collect(),
    )
    .unwrap();
    f.state = DesktopState::from_project(Project::new("Wide", data).unwrap()).unwrap();
    f.state.picker = true;
    f.frame(vec![]);
    f.click("Select all items");
    assert_eq!(f.draft.items.len(), 4);
}

#[test]
fn bulk_items_cover_all_pages_filter_numeric_columns_and_refuse_memory_growth() {
    let mut f = Form::new();
    let mut columns = (1..=64)
        .map(|id| Column::numeric(ColumnId(id), &format!("Item {id}"), vec![]).unwrap())
        .collect::<Vec<_>>();
    columns.push(Column::text(ColumnId(65), "Item text", vec![]).unwrap());
    f.state =
        DesktopState::from_project(Project::new("Wide", Dataset::new(columns).unwrap()).unwrap())
            .unwrap();
    f.state.picker = true;
    f.frame(vec![]);
    f.click("Select all items");
    assert_eq!(f.draft.items.len(), 64);
    assert_eq!(f.draft.items[63], ColumnId(64));
    let mut f = Form::new();
    f.state.picker = true;
    f.state.search = "Item 4".into();
    f.frame(vec![]);
    f.click("Select all items");
    assert_eq!(f.draft.items, [ColumnId(2), ColumnId(1), ColumnId(4)]);
    let mut f = Form::new();
    f.state.picker = true;
    f.frame(vec![]);
    f.state.active_recipe_bytes = crate::data::MAX_OWNED_BYTES;
    f.click("Select all items");
    assert_eq!(f.draft.items, [ColumnId(2), ColumnId(1)]);
    assert!(f.state.error().is_some());
}
