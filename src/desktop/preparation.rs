use super::{state::TEXT_LIMIT, theme, DesktopState};
use crate::{
    analysis::NumericComparison,
    data::{Cell, ColumnId, ColumnKind},
    error::{AppError, AppResult},
    jobs::{JobOperation, JobRunner},
    preparation::{Arithmetic, FilterPredicate, Operand, RecodeRule, Step, Unmatched},
};
use eframe::egui;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub(crate) struct PreparationForm {
    operation: usize,
    source: Option<ColumnId>,
    right: Option<ColumnId>,
    arithmetic: Arithmetic,
    name: String,
    value: String,
    replacement: String,
    kind: ColumnKind,
    descending: bool,
    preserve: bool,
    missing: bool,
    predicate: usize,
}
impl Default for PreparationForm {
    fn default() -> Self {
        Self {
            operation: 0,
            source: None,
            right: None,
            arithmetic: Arithmetic::Add,
            name: String::new(),
            value: "1".into(),
            replacement: String::new(),
            kind: ColumnKind::Number,
            descending: false,
            preserve: true,
            missing: false,
            predicate: 0,
        }
    }
}
fn invalid(message: &'static str) -> AppError {
    AppError::invalid("Preparation", message)
}
fn number(value: &str) -> AppResult<f64> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
        .ok_or_else(|| invalid("Enter a finite number."))
}
fn cell(text: &str, kind: ColumnKind) -> AppResult<Cell> {
    match kind {
        ColumnKind::Number => number(text).map(Cell::Number),
        ColumnKind::Text => Ok(Cell::Text(text.into())),
    }
}
impl PreparationForm {
    pub(crate) fn owned_bytes(&self) -> usize {
        self.name.capacity() + self.value.capacity() + self.replacement.capacity()
    }
    fn step(&self, state: &DesktopState) -> AppResult<Step> {
        let source = self.source.ok_or_else(|| invalid("Choose a variable."))?;
        let source_kind = state.project().dataset().column(source)?.kind();
        match self.operation {
            0 => Ok(Step::Compute {
                name: self.name.clone(),
                left: Operand::Column(source),
                operator: self.arithmetic,
                right: self
                    .right
                    .map(Operand::Column)
                    .map(Ok)
                    .unwrap_or_else(|| number(&self.value).map(Operand::Constant))?,
            }),
            1 => Ok(Step::Recode {
                source,
                name: self.name.clone(),
                kind: self.kind,
                rules: vec![RecodeRule {
                    from: cell(&self.value, source_kind)?,
                    to: if self.missing {
                        Cell::Missing
                    } else {
                        cell(&self.replacement, self.kind)?
                    },
                }],
                unmatched: if self.preserve {
                    Unmatched::Preserve
                } else {
                    Unmatched::Missing
                },
            }),
            2 => Ok(Step::Sort {
                source,
                descending: self.descending,
            }),
            3 => Ok(Step::Filter {
                source,
                predicate: match self.predicate {
                    6 => FilterPredicate::Missing,
                    7 => FilterPredicate::Present,
                    p => FilterPredicate::Compare {
                        comparison: comparisons()[p].0,
                        value: cell(&self.value, source_kind)?,
                    },
                },
            }),
            _ => Err(invalid("Choose a preparation operation.")),
        }
    }
}
fn comparisons() -> [(NumericComparison, &'static str); 6] {
    [
        (NumericComparison::Equal, "Equals"),
        (NumericComparison::NotEqual, "Does not equal"),
        (NumericComparison::Less, "Less than"),
        (NumericComparison::LessOrEqual, "At most"),
        (NumericComparison::Greater, "Greater than"),
        (NumericComparison::GreaterOrEqual, "At least"),
    ]
}
fn text(ui: &mut egui::Ui, label: &str, value: &mut String) {
    let labelled = ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .id_salt(label)
            .char_limit(TEXT_LIMIT),
    )
    .labelled_by(labelled.id);
}
fn column(
    ui: &mut egui::Ui,
    label: &str,
    selected: &mut Option<ColumnId>,
    state: &DesktopState,
    numeric: bool,
    constant: bool,
) {
    if selected.is_some_and(|id| state.project().dataset().column(id).is_err()) {
        *selected = None;
    }
    let name = selected
        .and_then(|id| state.project().dataset().column(id).ok())
        .map_or(
            if constant {
                "Constant"
            } else {
                "Choose variable"
            },
            |c| c.name(),
        );
    let labelled = ui.label(label);
    egui::ComboBox::from_id_salt(label)
        .selected_text(theme::caption(name))
        .show_ui(ui, |ui| {
            if constant {
                theme::selectable_value(ui, selected, None, "Constant");
            }
            for c in state
                .project()
                .dataset()
                .columns()
                .iter()
                .filter(|c| !numeric || c.kind() == ColumnKind::Number)
            {
                theme::selectable_value(ui, selected, Some(c.id()), theme::caption(c.name()));
            }
        })
        .response
        .labelled_by(labelled.id);
}
pub(crate) fn show(ui: &mut egui::Ui, state: &mut DesktopState, jobs: &mut JobRunner) {
    let mut form = state.preparation.clone();
    let request = egui::ScrollArea::vertical()
        .id_salt("preparation-form")
        .max_height((ui.available_height() - 44.0).max(20.0))
        .auto_shrink([false, false])
        .show(ui, |ui| {
    ui.set_max_width(600.0);
    ui.heading("Prepare data");
    ui.label(format!(
        "{} rows · {} variables",
        state.project().dataset().row_count(),
        state.project().dataset().column_count()
    ));
    let editable = !state.active && !state.metadata_has_changes() && state.cell_editor.is_none();
    ui.add_enabled_ui(editable, |ui| {
    ui.horizontal_wrapped(|ui| {
        for (i, label) in ["Compute", "Recode", "Sort", "Filter"]
            .into_iter()
            .enumerate()
        {
            theme::selectable_value(ui, &mut form.operation, i, label);
        }
    });
    ui.separator();
    ui.label(match form.operation {
        0 => "Compute adds a new numeric variable using an existing variable and a number or another variable.",
        1 => "Recode adds a new variable with a value replaced. The source variable stays unchanged.",
        2 => "Sort changes the order of whole rows, keeping each row's values together.",
        _ => "Filter keeps only rows that match your condition. Removed rows can be restored with Undo.",
    });
    column(
        ui,
        "Variable",
        &mut form.source,
        state,
        form.operation == 0,
        false,
    );
    match form.operation {
        0 => {
            text(ui, "New variable", &mut form.name);
            egui::ComboBox::from_id_salt("arithmetic")
                .selected_text(match form.arithmetic {
                    Arithmetic::Add => "+",
                    Arithmetic::Subtract => "−",
                    Arithmetic::Multiply => "×",
                    Arithmetic::Divide => "÷",
                })
                .show_ui(ui, |ui| {
                    for (op, label) in [
                        (Arithmetic::Add, "+"),
                        (Arithmetic::Subtract, "−"),
                        (Arithmetic::Multiply, "×"),
                        (Arithmetic::Divide, "÷"),
                    ] {
                        theme::selectable_value(ui, &mut form.arithmetic, op, label);
                    }
                });
            column(ui, "Operand", &mut form.right, state, true, true);
            if form.right.is_none() {
                text(ui, "Constant", &mut form.value);
            }
        }
        1 => {
            text(ui, "New variable", &mut form.name);
            super::variables::kind_picker(ui, "recode-kind", &mut form.kind);
            text(ui, "Match value", &mut form.value);
            ui.checkbox(&mut form.missing, "Replace with missing");
            if !form.missing {
                text(ui, "Replacement", &mut form.replacement);
            }
            ui.checkbox(&mut form.preserve, "Keep other values");
        }
        2 => {
            ui.checkbox(&mut form.descending, "Descending");
        }
        3 => {
            egui::ComboBox::from_id_salt("filter-predicate")
                .selected_text(if form.predicate < 6 {
                    comparisons()[form.predicate].1
                } else if form.predicate == 6 {
                    "Missing"
                } else {
                    "Present"
                })
                .show_ui(ui, |ui| {
                    for (i, (_, label)) in comparisons().into_iter().enumerate() {
                        theme::selectable_value(ui, &mut form.predicate, i, label);
                    }
                    theme::selectable_value(ui, &mut form.predicate, 6, "Missing");
                    theme::selectable_value(ui, &mut form.predicate, 7, "Present");
                });
            if form.predicate < 6 {
                text(ui, "Value", &mut form.value);
            }
            ui.small("Keeps matching rows. Undo restores removed rows.");
        }
        _ => {}
    }
    });
    ui.separator();
    let current_request = form.step(state).and_then(|step| {
        step.validate_shape()?;
        Ok(step)
    });
    if state.active {
        ui.label("Wait for the current operation to finish before preparing data.");
    } else if state.cell_editor.is_some() {
        ui.label("Finish or cancel the cell edit in Data before applying a preparation step.");
    } else if state.metadata_has_changes() {
        ui.label("Apply or discard variable property changes before preparing data.");
    } else if let Err(error) = &current_request {
        ui.label(theme::caption(error));
    } else {
        ui.strong("Ready to apply to the current dataset");
        ui.small("Compute and Recode add a variable. Sort and Filter change the rows. Check the result in Data; use Undo if needed.");
    }
    if let Some(error) = state.error() {
        ui.colored_label(ui.visuals().error_fg_color, theme::caption(error));
    }
    if !state.project().steps().is_empty() {
        ui.collapsing("Preparation history", |ui| {
        for record in state.project().steps().iter().rev().take(12) {
            ui.label(format!(
                "{} · revision {}",
                record.step.label(),
                record.result_revision.0
            ));
        }
        });
    }
    current_request
    }).inner;
    let ready = !state.active
        && !state.metadata_has_changes()
        && state.cell_editor.is_none()
        && request.is_ok();
    let response = ui
        .horizontal_wrapped(|ui| {
            let response = ui.add_enabled(ready, theme::primary(ui, "Apply preparation"));
            if ui
                .add_enabled(
                    !state.active
                        && !state.metadata_has_changes()
                        && state.cell_editor.is_none()
                        && state.can_undo(),
                    egui::Button::new("Undo last step"),
                )
                .clicked()
            {
                state.act(super::UiAction::Undo);
            }
            response
        })
        .inner;
    super::procedure_forms::record(state, "Apply preparation", &response);
    state.preparation = form;
    if response.clicked() {
        match request {
            Ok(step) => {
                let project = Arc::clone(state.project());
                super::start(state, jobs, JobOperation::Prepare { project, step });
                if state.active {
                    state.dialog = None;
                }
            }
            Err(error) => state.fail(error),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn apply_stays_visible_in_a_short_preparation_workspace() {
        let mut state = DesktopState::new().unwrap();
        let mut jobs = JobRunner::default();
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(620.0, 120.0),
                )),
                ..Default::default()
            },
            |ui| show(ui, &mut state, &mut jobs),
        );
        output.textures_delta.clear();
        let apply = state.control_rect("Apply preparation").unwrap();
        assert!(
            apply.bottom() <= 120.0,
            "Apply must stay visible: {apply:?}"
        );
    }

    #[test]
    fn unnamed_compute_cannot_be_applied() {
        use crate::data::{Column, Dataset};
        let project = crate::project::Project::new(
            "Draft",
            Dataset::new(vec![
                Column::numeric(ColumnId(1), "Score", vec![Some(1.0)]).unwrap()
            ])
            .unwrap(),
        )
        .unwrap();
        let mut state = DesktopState::from_project(project).unwrap();
        state.preparation.source = Some(ColumnId(1));
        let mut jobs = JobRunner::default();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut output = ctx.run_ui(Default::default(), |ui| show(ui, &mut state, &mut jobs));
        output.textures_delta.clear();
        let update = output.platform_output.accesskit_update.unwrap();
        let apply = update
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Apply preparation"))
            .unwrap();
        assert!(
            apply.1.is_disabled(),
            "a blank output name must not look ready to apply"
        );
        assert!(!jobs.is_running());
    }

    #[test]
    fn finite_numeric_controls_and_exact_text_are_distinct() {
        assert!(number("NaN").is_err());
        assert!(number("inf").is_err());
        assert_eq!(
            cell("00123", ColumnKind::Text).unwrap(),
            Cell::Text("00123".into())
        );
        assert_eq!(
            cell("", ColumnKind::Text).unwrap(),
            Cell::Text(String::new())
        );
        assert!(cell("", ColumnKind::Number).is_err());
    }
}
