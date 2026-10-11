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
                ui.selectable_value(selected, None, "Constant");
            }
            for c in state
                .project()
                .dataset()
                .columns()
                .iter()
                .filter(|c| !numeric || c.kind() == ColumnKind::Number)
            {
                ui.selectable_value(selected, Some(c.id()), theme::caption(c.name()));
            }
        })
        .response
        .labelled_by(labelled.id);
}
pub(crate) fn show(ui: &mut egui::Ui, state: &mut DesktopState, jobs: &mut JobRunner) {
    let mut form = state.preparation.clone();
    ui.horizontal_wrapped(|ui| {
        for (i, label) in ["Compute", "Recode", "Sort", "Filter"]
            .into_iter()
            .enumerate()
        {
            ui.selectable_value(&mut form.operation, i, label);
        }
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
                        ui.selectable_value(&mut form.arithmetic, op, label);
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
                        ui.selectable_value(&mut form.predicate, i, label);
                    }
                    ui.selectable_value(&mut form.predicate, 6, "Missing");
                    ui.selectable_value(&mut form.predicate, 7, "Present");
                });
            if form.predicate < 6 {
                text(ui, "Value", &mut form.value);
            }
            ui.small("Keeps matching rows. Undo restores removed rows.");
        }
        _ => {}
    }
    let request = form.step(state);
    let ready = !state.active
        && !state.metadata_has_changes()
        && state.cell_editor.is_none()
        && request.is_ok();
    let response = ui.add_enabled(ready, egui::Button::new("Apply"));
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
    if let Some(error) = state.error() {
        ui.colored_label(ui.visuals().error_fg_color, theme::caption(error));
    }
    if !state.project().steps().is_empty() {
        ui.separator();
        ui.strong("Preparation history");
        for record in state.project().steps().iter().rev().take(12) {
            ui.label(format!(
                "{} · revision {}",
                record.step.label(),
                record.result_revision.0
            ));
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
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
