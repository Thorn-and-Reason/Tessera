#![cfg(feature = "desktop")]
use eframe::egui::{self, Event, Key, Modifiers, PointerButton};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering::*},
};
use tessera::{
    analysis::*,
    data::{Cell, Column, ColumnId, Dataset, ValueLabel},
    desktop::{self, draft::AnalysisDraft, DesktopState, UiAction, Workspace},
    jobs::JobRunner,
    operation::{JobControl, RunId},
    project::Project,
};
struct Tracked;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static TRACK: AtomicBool = AtomicBool::new(false);
unsafe impl GlobalAlloc for Tracked {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            let n = LIVE.fetch_add(l.size(), SeqCst) + l.size();
            if TRACK.load(SeqCst) {
                PEAK.fetch_max(n, SeqCst);
            }
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), SeqCst);
        unsafe { System.dealloc(p, l) }
    }
}
#[global_allocator]
static ALLOC: Tracked = Tracked;
fn project(wide: usize, long: bool) -> Project {
    let mut cols: Vec<Column> = (1..=wide)
        .map(|i| {
            let c = Column::numeric(
                ColumnId(i as u64),
                &format!(
                    "Item {i} {}",
                    if long {
                        "ŵ".repeat(2048)
                    } else {
                        String::new()
                    }
                ),
                vec![Some(0.), Some(1.), Some(2.), Some(3.)],
            )
            .unwrap();
            if !long {
                return c;
            }
            let mut m = c.metadata().clone();
            m.description = "Description ŵ ".repeat(1024);
            m.value_labels = (0..4)
                .map(|j| ValueLabel {
                    value: Cell::Number(j as f64),
                    label: format!("Label {j} {}", "ŵ".repeat(4096)),
                })
                .collect();
            c.with_metadata(m).unwrap()
        })
        .collect();
    let filter_value = if long { "ŵ".repeat(128) } else { "x".into() };
    cols.push(
        Column::text(
            ColumnId(wide as u64 + 1),
            "Text filter",
            vec![Some(filter_value.clone()); 4],
        )
        .unwrap(),
    );
    let data = Dataset::new(cols).unwrap();
    let r = ReliabilityRequest {
        items: (1..=64.min(wide)).map(|i| ColumnId(i as u64)).collect(),
        reversals: vec![ReliabilityReverse {
            column: ColumnId(1),
            lower: 0.,
            upper: 3.,
        }],
        filter: RowFilter {
            predicates: if long {
                (0..256)
                    .map(|_| RowPredicate::Text {
                        column: ColumnId(wide as u64 + 1),
                        value: filter_value.clone(),
                        equal: true,
                    })
                    .collect()
            } else {
                vec![]
            },
        },
        frequency_weight: None,
    };
    let run = reliability_with_budget(
        &data,
        &r,
        &JobControl::default(),
        &[],
        std::mem::size_of::<ReliabilityRequest>() + r.heap_bytes().unwrap(),
    )
    .unwrap();
    Project::new("Reliability layout", data)
        .unwrap()
        .with_run(CompletedAnalysis::Reliability(run))
        .unwrap()
}
fn state(p: Project) -> DesktopState {
    let mut s = DesktopState::from_project(p)
        .unwrap()
        .apply_action(UiAction::SelectRun(RunId(1)))
        .unwrap()
        .apply_action(UiAction::Navigate(Workspace::Analyses))
        .unwrap();
    let r = s.project().runs()[0].analysis.request().clone_owned();
    s = s
        .apply_action(UiAction::SetDraft(AnalysisDraft::from_request(r)))
        .unwrap();
    s
}
struct Output(egui::FullOutput);
impl std::ops::Deref for Output {
    type Target = egui::FullOutput;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        self.0.textures_delta.clear();
    }
}
fn frame(
    ctx: &egui::Context,
    s: &mut DesktopState,
    j: &mut JobRunner,
    size: egui::Vec2,
    events: Vec<Event>,
) -> Output {
    Output(ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ui| desktop::render(ui, s, j),
    ))
}
fn text_pos(out: &egui::FullOutput, label: &str) -> Option<egui::Pos2> {
    fn scan(s: &egui::Shape, label: &str) -> Option<egui::Pos2> {
        match s {
            egui::Shape::Text(t) if t.galley.text() == label => Some(t.pos + t.galley.size() * 0.5),
            egui::Shape::Vec(v) => v.iter().find_map(|s| scan(s, label)),
            _ => None,
        }
    }
    out.shapes.iter().find_map(|s| scan(&s.shape, label))
}
fn keep(previous: &mut Output, next: Output) {
    *previous = next;
}
fn click(
    ctx: &egui::Context,
    s: &mut DesktopState,
    j: &mut JobRunner,
    previous: &mut Output,
    pos: egui::Pos2,
    size: egui::Vec2,
) {
    for pressed in [true, false] {
        keep(
            previous,
            frame(
                ctx,
                s,
                j,
                size,
                vec![
                    Event::PointerMoved(pos),
                    Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    },
                ],
            ),
        );
    }
}
fn enter(
    ctx: &egui::Context,
    s: &mut DesktopState,
    j: &mut JobRunner,
    previous: &mut Output,
    label: &str,
    size: egui::Vec2,
) -> bool {
    let Some(id) = s.control_id(label) else {
        return false;
    };
    ctx.memory_mut(|m| m.request_focus(id));
    for pressed in [true, false] {
        keep(
            previous,
            frame(
                ctx,
                s,
                j,
                size,
                vec![Event::Key {
                    key: Key::Enter,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }],
            ),
        );
    }
    ctx.memory(|m| m.focused()) == Some(id)
}
#[test]
fn reliability_full_root_frame_aggregate_peak_preserves_saved_identity() {
    for wide in [64, 512] {
        let ctx = egui::Context::default();
        desktop::configure_context(&ctx);
        ctx.enable_accesskit();
        let mut jobs = JobRunner::default();
        let tall = egui::vec2(1180., 2200.);
        let normal = egui::vec2(1180., 780.);
        let narrow = egui::vec2(760., 520.);
        let mut warm = state(project(2, false));
        let mut previous = frame(&ctx, &mut warm, &mut jobs, tall, vec![]);
        for _ in 0..5 {
            keep(
                &mut previous,
                frame(&ctx, &mut warm, &mut jobs, tall, vec![]),
            );
        }
        for label in ["More settings", "Run settings and provenance"] {
            let pos = text_pos(&previous, label).unwrap_or_else(|| panic!("warm missing {label}"));
            click(&ctx, &mut warm, &mut jobs, &mut previous, pos, tall);
            for _ in 0..3 {
                keep(
                    &mut previous,
                    frame(&ctx, &mut warm, &mut jobs, tall, vec![]),
                );
            }
        }
        drop(previous);
        let mut s = state(project(wide, true));
        let captured = s.project().runs()[0].analysis.clone();
        let draft = s.draft().clone();
        let AnalysisDraft::Reliability(d) = &draft else {
            panic!()
        };
        assert_eq!(d.filter.predicates.len(), 256);
        s = s.apply_action(UiAction::SetDraft(draft)).unwrap();
        let before = s.draft().clone();
        let bytes = before.heap_bytes().unwrap();
        let base = LIVE.load(SeqCst);
        PEAK.store(base, SeqCst);
        TRACK.store(true, SeqCst);
        let mut previous = frame(&ctx, &mut s, &mut jobs, normal, vec![]);
        keep(&mut previous, frame(&ctx, &mut s, &mut jobs, tall, vec![]));
        for label in ["None", "Add condition"] {
            if label == "Add condition" {
                let mut fewer = s.draft().clone();
                let AnalysisDraft::Reliability(d) = &mut fewer else {
                    panic!()
                };
                d.filter.predicates.pop();
                s = s.apply_action(UiAction::SetDraft(fewer)).unwrap();
                keep(&mut previous, frame(&ctx, &mut s, &mut jobs, tall, vec![]));
            }
            let pos = text_pos(&previous, label).unwrap_or_else(|| panic!("missing popup {label}"));
            click(&ctx, &mut s, &mut jobs, &mut previous, pos, tall);
            for _ in 0..3 {
                keep(&mut previous, frame(&ctx, &mut s, &mut jobs, tall, vec![]));
            }
            keep(
                &mut previous,
                frame(
                    &ctx,
                    &mut s,
                    &mut jobs,
                    tall,
                    vec![Event::Key {
                        key: Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Modifiers::NONE,
                    }],
                ),
            );
        }
        s = s.apply_action(UiAction::SetDraft(before.clone())).unwrap();
        keep(&mut previous, frame(&ctx, &mut s, &mut jobs, tall, vec![]));
        // Bound editing and egui undo owners contribute to the same measured interval.
        let lower = s
            .control_id("Lower bound: 1")
            .expect("first explicit bound");
        ctx.memory_mut(|m| m.request_focus(lower));
        keep(
            &mut previous,
            frame(
                &ctx,
                &mut s,
                &mut jobs,
                tall,
                vec![
                    Event::Key {
                        key: Key::A,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Modifiers::COMMAND,
                    },
                    Event::Paste("0".repeat(512)),
                ],
            ),
        );
        let edited = s.draft().clone();
        let AnalysisDraft::Reliability(d) = &edited else {
            panic!()
        };
        assert_eq!(d.reversals[0].lower.len(), 512);
        assert!(edited.heap_bytes().unwrap() >= bytes);
        keep(&mut previous, frame(&ctx, &mut s, &mut jobs, tall, vec![]));
        keep(
            &mut previous,
            frame(
                &ctx,
                &mut s,
                &mut jobs,
                tall,
                vec![Event::Key {
                    key: Key::Z,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::COMMAND,
                }],
            ),
        );
        // Restore the exact saved draft through its public checked owner after exercising growth/undo.
        s = s.apply_action(UiAction::SetDraft(before.clone())).unwrap();
        keep(
            &mut previous,
            frame(&ctx, &mut s, &mut jobs, normal, vec![]),
        );
        let mut traversals = 0;
        for label in [
            "Next selected items page",
            "Next item results page",
            "Next filter conditions page",
            "Next captured items page",
            "Next captured definitions page",
            "Next captured filter conditions page",
        ] {
            for _ in 0..70 {
                if !enter(&ctx, &mut s, &mut jobs, &mut previous, label, normal) {
                    break;
                }
                traversals += 1;
            }
        }
        // Keep the prior complete output live while rendering its successor, including textures.
        for size in [narrow, normal] {
            keep(&mut previous, frame(&ctx, &mut s, &mut jobs, size, vec![]));
        }
        assert!(
            s.control_rect("Procedure").is_some(),
            "wide settings inspector is visible"
        );
        let pos = s.control_rect("Settings").unwrap().center();
        click(&ctx, &mut s, &mut jobs, &mut previous, pos, normal);
        keep(
            &mut previous,
            frame(&ctx, &mut s, &mut jobs, normal, vec![]),
        );
        assert!(s.dialog().is_none(), "wide Settings toggles the inspector");
        assert!(
            s.control_rect("Procedure").is_none(),
            "wide Settings hides its editor"
        );
        // Narrow Settings opens the same editor in a modal; resize while retaining both outputs.
        for _ in 0..2 {
            keep(
                &mut previous,
                frame(&ctx, &mut s, &mut jobs, narrow, vec![]),
            );
        }
        let pos = s.control_rect("Settings").unwrap().center();
        click(&ctx, &mut s, &mut jobs, &mut previous, pos, narrow);
        assert_eq!(s.dialog(), Some(desktop::Dialog::Settings));
        for _ in 0..4 {
            keep(
                &mut previous,
                frame(&ctx, &mut s, &mut jobs, normal, vec![]),
            );
        }
        keep(
            &mut previous,
            frame(
                &ctx,
                &mut s,
                &mut jobs,
                narrow,
                vec![Event::Key {
                    key: Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }],
            ),
        );
        s = s
            .apply_action(UiAction::Navigate(Workspace::Results))
            .unwrap();
        for _ in 0..4 {
            keep(
                &mut previous,
                frame(&ctx, &mut s, &mut jobs, normal, vec![]),
            );
        }
        let peak = PEAK.load(SeqCst).saturating_sub(base);
        TRACK.store(false, SeqCst);
        drop(previous);
        println!("reliability root-frame wide={wide} selected=64 captured_long_metadata=true filters=256 pager_enter_cycles={traversals} old_new_full_output=true peak={peak} reserve={}",desktop::UI_WORK_BYTES);
        assert!(traversals > 0);
        assert!(
            peak <= desktop::UI_WORK_BYTES,
            "full-frame UI reserve exceeded"
        );
        assert_eq!(s.draft(), &before);
        assert_eq!(s.draft().heap_bytes().unwrap(), bytes);
        assert_eq!(s.project().runs()[0].analysis, captured);
        assert!(s.error().is_none(), "unexpected UI error {:?}", s.error());
        assert!(!jobs.is_running());
    }
}
