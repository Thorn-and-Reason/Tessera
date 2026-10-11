//! Headless developer frame probe: ui_perf_probe <fixture.csv|fixture.tessera>.
//! Measures egui CPU layout and state transitions, excluding native startup, GPU,
//! compositor presentation, and real input latency. Wheel stages measure wheel
//! input frames; short or empty panes may not move. Run in isolated processes.
#[cfg(feature = "desktop")]
mod probe {
    use eframe::egui::{self, Event, Pos2, Rect, Vec2};
    use std::{fs::File, io::Read, path::Path, sync::Arc, time::Instant};
    use tessera::{
        data::Budget,
        desktop::{configure_context, render, DesktopState, UiAction, Workspace},
        error::{AppError, AppResult},
        import::{commit_preview, preview, Delimiter, MAX_INPUT_BYTES},
        jobs::JobRunner,
        operation::JobControl,
        project::Project,
        project_io::decode,
    };

    fn load(path: &Path) -> AppResult<Project> {
        let control = JobControl::default();
        if path
            .extension()
            .is_some_and(|extension| extension == "tessera")
        {
            return decode(File::open(path)?, &control);
        }
        let mut bytes = Vec::new();
        File::open(path)?
            .take(MAX_INPUT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_INPUT_BYTES {
            return Err(AppError::SizeLimit {
                limit: MAX_INPUT_BYTES,
            });
        }
        let input = preview(Arc::from(bytes), Delimiter::Comma, &control)?;
        let choices = input.default_choices()?;
        Project::new(
            "UI performance fixture",
            commit_preview(&input, &choices, &control)?,
        )
    }

    fn frame(ctx: &egui::Context, state: &mut DesktopState, jobs: &mut JobRunner, wheel: f32) {
        let events = if wheel != 0.0 {
            vec![
                Event::PointerMoved(Pos2::new(600.0, 500.0)),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: Vec2::new(0.0, wheel),
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        } else {
            vec![]
        };
        frame_events(ctx, state, jobs, events);
    }

    fn frame_events(
        ctx: &egui::Context,
        state: &mut DesktopState,
        jobs: &mut JobRunner,
        events: Vec<Event>,
    ) -> egui::FullOutput {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0))),
                events,
                ..Default::default()
            },
            |ui| render(ui, state, jobs),
        );
        output.textures_delta.clear();
        output
    }

    fn wheel_delta(scroll: bool, index: usize) -> f32 {
        if !scroll {
            0.0
        } else if (index / 5) % 2 == 0 {
            -160.0
        } else {
            160.0
        }
    }

    fn metadata_search(state: &mut DesktopState, jobs: &mut JobRunner) -> AppResult<()> {
        *state = state.apply_action(UiAction::Navigate(Workspace::Variables))?;
        let ctx = egui::Context::default();
        configure_context(&ctx);
        ctx.enable_accesskit();
        let output = frame_events(&ctx, state, jobs, vec![]);
        let update = output.platform_output.accesskit_update.unwrap();
        let label = update
            .nodes
            .iter()
            .find(|(_, node)| node.label().or_else(|| node.value()) == Some("Find a variable"))
            .expect("variable search label")
            .1
            .bounds()
            .expect("variable search label bounds");
        // Egui's label text node can differ from its Response ID used by labelled_by.
        let inputs: Vec<_> = update
            .nodes
            .iter()
            .filter(|(_, node)| {
                node.role() == egui::accesskit::Role::TextInput
                    && node.bounds().is_some_and(|bounds| {
                        bounds.x0 <= label.x0
                            && bounds.x1 > label.x0
                            && bounds.y0 >= label.y1
                            && bounds.y0 - label.y1 < 32.0
                    })
            })
            .collect();
        assert_eq!(
            inputs.len(),
            1,
            "unique input directly below variable search label"
        );
        let (id, node) = inputs[0];
        let id = *id;
        let bounds = node.bounds().expect("variable search bounds");
        let position = Pos2::new(
            ((bounds.x0 + bounds.x1) / 2.0) as f32,
            ((bounds.y0 + bounds.y1) / 2.0) as f32,
        );
        for pressed in [true, false] {
            frame_events(
                &ctx,
                state,
                jobs,
                vec![
                    Event::PointerMoved(position),
                    Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(ctx.memory(|memory| memory.focused()).is_some());
        let query = "tessera-ui-perf-search";
        let start = Instant::now();
        for index in 0..100 {
            let text = if index % 2 == 0 { query } else { "" };
            let output = frame_events(
                &ctx,
                state,
                jobs,
                vec![
                    Event::Key {
                        key: egui::Key::A,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers {
                            command: true,
                            mac_cmd: true,
                            ..Default::default()
                        },
                    },
                    Event::Key {
                        key: egui::Key::Backspace,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                    Event::Text(text.into()),
                ],
            );
            let update = output.platform_output.accesskit_update.unwrap();
            assert_eq!(update.focus, id, "search field lost focus");
            assert!(
                update.nodes.iter().any(|(node_id, node)| {
                    *node_id == id && node.value().unwrap_or("") == text
                }),
                "query did not reach the search field"
            );
            assert!(state.error().is_none(), "{:?}", state.error());
        }
        eprintln!(
            "stage_ui_metadata_search_ms={:.6}",
            start.elapsed().as_secs_f64() * 10.0
        );
        Ok(())
    }

    pub fn run() -> AppResult<()> {
        let path = std::env::args_os().nth(1).ok_or_else(|| {
            AppError::invalid(
                "fixture",
                "Usage: ui_perf_probe <fixture.csv|fixture.tessera>",
            )
        })?;
        let start = Instant::now();
        let mut state = DesktopState::from_project(load(Path::new(&path))?)?;
        eprintln!(
            "stage_ui_load_ms={:.6}",
            start.elapsed().as_secs_f64() * 1000.0
        );
        let project = Arc::clone(state.project());
        let ctx = egui::Context::default();
        configure_context(&ctx);
        ctx.enable_accesskit();
        let mut jobs = JobRunner::default();
        let start = Instant::now();
        frame(&ctx, &mut state, &mut jobs, 0.0);
        eprintln!(
            "stage_ui_first_frame_ms={:.6}",
            start.elapsed().as_secs_f64() * 1000.0
        );
        eprintln!("ui_saved_runs={}", project.runs().len());
        for (name, workspace) in [
            ("data", Workspace::Data),
            ("variables", Workspace::Variables),
            ("results", Workspace::Results),
        ] {
            state = state.apply_action(UiAction::Navigate(workspace))?;
            for scroll in [false, true] {
                for index in 0..10 {
                    frame(&ctx, &mut state, &mut jobs, wheel_delta(scroll, index));
                }
                let start = Instant::now();
                for index in 0..100 {
                    frame(&ctx, &mut state, &mut jobs, wheel_delta(scroll, index));
                }
                let mode = if scroll { "wheel_frame" } else { "idle" };
                eprintln!(
                    "stage_ui_{name}_{mode}_ms={:.6}",
                    start.elapsed().as_secs_f64() * 10.0
                );
                assert_eq!(state.workspace(), workspace);
                assert!(state.error().is_none(), "{:?}", state.error());
                assert!(state.control_rect("Data").is_some());
            }
        }
        metadata_search(&mut state, &mut jobs)?;
        let ids: Vec<_> = project.runs().iter().map(|run| run.id).collect();
        if !ids.is_empty() {
            state = state.apply_action(UiAction::Navigate(Workspace::Results))?;
            let start = Instant::now();
            for index in 0..100 {
                let id = ids[index % ids.len()];
                state = state.apply_action(UiAction::SelectRun(id))?;
                frame(&ctx, &mut state, &mut jobs, 0.0);
                assert_eq!(state.selected_run().unwrap().id, id);
            }
            eprintln!(
                "stage_ui_history_ms={:.6}",
                start.elapsed().as_secs_f64() * 10.0
            );
        }
        assert!(Arc::ptr_eq(&project, state.project()));
        assert!(!jobs.is_running());
        // Public accounting excludes UI buffers; process peak RSS must be measured externally.
        eprintln!(
            "retained_project_bytes={}",
            Budget::dataset_bytes(&[project.dataset()], project.owned_bytes()?)?
        );
        Ok(())
    }
}

#[cfg(feature = "desktop")]
fn main() {
    if let Err(error) = probe::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(not(feature = "desktop"))]
fn main() {
    eprintln!("ui_perf_probe requires --features desktop");
    std::process::exit(1);
}
