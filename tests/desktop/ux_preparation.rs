use super::*;
use tessera::preparation::{Arithmetic, FilterPredicate, Operand, Step};

fn feedback(state: &mut DesktopState, jobs: &mut JobRunner) -> String {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1180., 780.))),
            ..Default::default()
        },
        |ui| render(ui, state, jobs),
    );
    output.textures_delta.clear();
    output
        .platform_output
        .accesskit_update
        .unwrap()
        .nodes
        .iter()
        .filter_map(|(_, node)| node.label().or(node.value()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn preparation_completion_identifies_created_variable_and_inspection_undo() {
    let mut state = state()
        .apply_action(UiAction::Navigate(Workspace::Prepare))
        .unwrap();
    let mut jobs = JobRunner::default();
    let project = Arc::clone(state.project());
    run(
        &mut state,
        &mut jobs,
        JobOperation::Prepare {
            project,
            step: Step::Compute {
                name: "Age_plus_one".into(),
                left: Operand::Column(ColumnId(1)),
                operator: Arithmetic::Add,
                right: Operand::Constant(1.0),
            },
        },
    );
    assert_eq!(
        state.project().dataset().cell(ColumnId(3), 0).unwrap(),
        Cell::Number(18.0)
    );
    let text = feedback(&mut state, &mut jobs);
    assert!(text.contains("Created variable “Age_plus_one”"), "{text}");
    assert!(
        text.contains("3 rows") && text.contains("Inspect") && text.contains("Undo"),
        "{text}"
    );
    assert!(state.can_undo());
    let ctx = egui::Context::default();
    let size = Vec2::new(1180., 780.);
    frame(&ctx, &mut state, &mut jobs, size, vec![]);
    assert!(state.control_rect("Inspect prepared data").is_some());
    click(&ctx, &mut state, &mut jobs, "Inspect prepared data", size);
    assert_eq!(state.workspace(), Workspace::Data);
    let state = state.apply_action(UiAction::Undo).unwrap();
    assert_eq!(state.project().dataset().column_count(), 2);
    assert_eq!(
        state.project().dataset().cell(ColumnId(1), 0).unwrap(),
        Cell::Number(17.0)
    );
}

#[test]
fn filter_completion_reports_kept_rows_and_reversible_outcome() {
    let mut state = state();
    let mut jobs = JobRunner::default();
    let project = Arc::clone(state.project());
    run(
        &mut state,
        &mut jobs,
        JobOperation::Prepare {
            project,
            step: Step::Filter {
                source: ColumnId(1),
                predicate: FilterPredicate::Compare {
                    comparison: tessera::analysis::NumericComparison::Greater,
                    value: Cell::Number(20.0),
                },
            },
        },
    );
    assert_eq!(state.project().dataset().row_count(), 2);
    let text = feedback(&mut state, &mut jobs);
    assert!(text.contains("Kept 2 of 3 rows"), "{text}");
    assert!(
        text.contains("Inspect") && text.contains("Undo") && text.contains("restore"),
        "{text}"
    );
    let state = state.apply_action(UiAction::Undo).unwrap();
    assert_eq!(state.project().dataset().row_count(), 3);
}
