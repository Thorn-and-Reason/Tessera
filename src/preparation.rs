//! Closed, passive preparation requests; execution always produces a complete new dataset.
use crate::{
    analysis::NumericComparison,
    data::{AllocationCounter, Budget, Cell, ColumnId, ColumnKind, ColumnMetadata, Dataset},
    error::{AppError, AppResult},
    operation::JobControl,
    project::Project,
    project_io::wire,
};
use serde::{Deserialize, Serialize};
use std::mem::size_of;
mod transform;
pub const MAX_STEPS: usize = 256;
pub const MAX_RECODE_RULES: usize = 256;
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Arithmetic {
    Add,
    Subtract,
    Multiply,
    Divide,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Operand {
    Column(#[serde(with = "wire::column_id")] ColumnId),
    Constant(f64),
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unmatched {
    Preserve,
    Missing,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecodeRule {
    #[serde(with = "wire::cell")]
    pub from: Cell,
    #[serde(with = "wire::cell")]
    pub to: Cell,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum FilterPredicate {
    Compare {
        #[serde(with = "wire::Comparison")]
        comparison: NumericComparison,
        #[serde(with = "wire::cell")]
        value: Cell,
    },
    Missing,
    Present,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    Compute {
        name: String,
        left: Operand,
        operator: Arithmetic,
        right: Operand,
    },
    Recode {
        #[serde(with = "wire::column_id")]
        source: ColumnId,
        name: String,
        #[serde(with = "wire::Kind")]
        kind: ColumnKind,
        rules: Vec<RecodeRule>,
        unmatched: Unmatched,
    },
    Sort {
        #[serde(with = "wire::column_id")]
        source: ColumnId,
        descending: bool,
    },
    Filter {
        #[serde(with = "wire::column_id")]
        source: ColumnId,
        predicate: FilterPredicate,
    },
    /// Descriptive boundary: direct edits are retained as data, not executable recipes.
    DatasetEdit,
}
impl Step {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Compute { .. } => "Compute variable",
            Self::Recode { .. } => "Recode variable",
            Self::Sort { .. } => "Sort rows",
            Self::Filter { .. } => "Keep rows",
            Self::DatasetEdit => "Edit data",
        }
    }
    pub fn owned_bytes(&self) -> AppResult<usize> {
        let mut n = AllocationCounter::default();
        n.add(size_of::<Self>())?;
        match self {
            Self::Compute { name, .. } => n.add(name.capacity())?,
            Self::Recode { name, rules, .. } => {
                n.add(name.capacity())?;
                n.array::<RecodeRule>(rules.capacity())?;
                for r in rules {
                    n.add(cell_heap(&r.from) + cell_heap(&r.to))?;
                }
            }
            Self::Filter {
                predicate: FilterPredicate::Compare { value, .. },
                ..
            } => n.add(cell_heap(value))?,
            _ => {}
        }
        Ok(n.bytes)
    }
    pub(crate) fn validate_shape(&self) -> AppResult<()> {
        let name = match self {
            Self::Compute { name, .. } | Self::Recode { name, .. } => Some(name),
            _ => None,
        };
        if let Some(n) = name {
            ColumnMetadata::validate_name(n)?;
            if n.len() > 1024 {
                return Err(invalid("Variable names are limited to 1024 bytes."));
            }
        }
        match self {
            Self::Compute { left, right, .. } => validate_operands(left, right)?,
            Self::Recode {
                source,
                rules,
                kind,
                ..
            } => validate_recode(*source, rules, *kind)?,
            Self::Sort { source, .. } | Self::Filter { source, .. } if source.0 == 0 => {
                return Err(invalid("Source identity must be positive."))
            }
            Self::Filter {
                predicate: FilterPredicate::Compare { value, .. },
                ..
            } => {
                valid_cell(value)?;
                if *value == Cell::Missing {
                    return Err(invalid(
                        "Use the missing test instead of comparing a missing value.",
                    ));
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(crate) fn source_ids(&self) -> impl Iterator<Item = ColumnId> + '_ {
        let (a, b) = match self {
            Self::Compute { left, right, .. } => (
                match left {
                    Operand::Column(c) => Some(*c),
                    _ => None,
                },
                match right {
                    Operand::Column(c) => Some(*c),
                    _ => None,
                },
            ),
            Self::Recode { source, .. }
            | Self::Sort { source, .. }
            | Self::Filter { source, .. } => (Some(*source), None),
            Self::DatasetEdit => (None, None),
        };
        a.into_iter().chain(b)
    }
    fn validate_current(&self, d: &Dataset) -> AppResult<()> {
        self.validate_shape()?;
        for id in self.source_ids() {
            d.column(id)?;
        }
        match self {
            Self::Compute { .. } => {
                for id in self.source_ids() {
                    if d.column(id)?.kind() != ColumnKind::Number {
                        return Err(invalid("Compute requires numeric source variables."));
                    }
                }
            }
            Self::Recode {
                source,
                kind,
                rules,
                unmatched,
                ..
            } => {
                let c = d.column(*source)?;
                if rules.iter().any(|r| !cell_kind(&r.from, c.kind()))
                    || (*unmatched == Unmatched::Preserve && *kind != c.kind())
                {
                    return Err(invalid("Recode pairs and preserved unmatched values must match their variable types."));
                }
            }
            Self::Filter {
                source,
                predicate: FilterPredicate::Compare { value, .. },
            } if !cell_kind(value, d.column(*source)?.kind()) => {
                return Err(invalid(
                    "Comparison values must match the source variable type.",
                ))
            }
            Self::DatasetEdit => {
                return Err(invalid(
                    "Direct-edit records cannot be executed as a preparation recipe.",
                ))
            }
            _ => {}
        }
        Ok(())
    }
}
fn validate_operands(left: &Operand, right: &Operand) -> AppResult<()> {
    for o in [left, right] {
        match o {
            Operand::Column(id) if id.0 == 0 => {
                return Err(invalid("Source identity must be positive."))
            }
            Operand::Constant(v) if !v.is_finite() => {
                return Err(invalid("Constants must be finite."))
            }
            _ => {}
        }
    }
    Ok(())
}
fn validate_recode(source: ColumnId, rules: &[RecodeRule], kind: ColumnKind) -> AppResult<()> {
    if source.0 == 0 || rules.is_empty() || rules.len() > MAX_RECODE_RULES {
        return Err(invalid(
            "Choose a source and between 1 and 256 recode pairs.",
        ));
    }
    for (i, r) in rules.iter().enumerate() {
        valid_cell(&r.from)?;
        valid_cell(&r.to)?;
        if r.from == Cell::Missing
            || !cell_kind(&r.to, kind)
            || rules[..i].iter().any(|p| p.from == r.from)
        {
            return Err(invalid("Recode sources must be unique present values; destinations must match the output type."));
        }
    }
    Ok(())
}
fn invalid(message: &str) -> AppError {
    AppError::invalid("preparation", message)
}
fn cell_heap(c: &Cell) -> usize {
    match c {
        Cell::Text(t) => t.capacity(),
        _ => 0,
    }
}
fn valid_cell(c: &Cell) -> AppResult<()> {
    match c {
        Cell::Number(n) if !n.is_finite() => Err(invalid("Recipe numbers must be finite.")),
        Cell::Text(s) if s.len() > 4096 => {
            Err(invalid("Recipe text values are limited to 4096 bytes."))
        }
        _ => Ok(()),
    }
}
fn cell_kind(c: &Cell, k: ColumnKind) -> bool {
    matches!(
        (c, k),
        (Cell::Missing, _)
            | (Cell::Number(_), ColumnKind::Number)
            | (Cell::Text(_), ColumnKind::Text)
    )
}
/// Caller extra excludes this project's owned metadata; retained contains all other datasets.
pub fn prepare_with_budget(
    project: &Project,
    step: &Step,
    control: &JobControl,
    retained: &[&Dataset],
    extra: usize,
) -> AppResult<Project> {
    control.check_cancelled()?;
    step.validate_current(project.dataset())?;
    project.admit_preparation(step)?;
    let output = transform::apply(project, step, control, retained, extra)?;
    control.check_cancelled()?;
    project.with_prepared_dataset(output, step, control, retained, extra)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Column;
    #[test]
    fn cancellation_during_row_processing_returns_no_project() {
        let p = Project::new(
            "Cancel",
            Dataset::new(vec![
                Column::numeric(ColumnId(1), "x", vec![Some(1.); 100]).unwrap()
            ])
            .unwrap(),
        )
        .unwrap();
        let c = JobControl::default();
        let cc = c.clone();
        crate::operation::CHECKPOINT_HOOK.with(|h| {
            *h.borrow_mut() = Some(Box::new(move |progress| {
                if progress.done == 10 {
                    cc.cancel()
                }
            }))
        });
        let result = prepare_with_budget(
            &p,
            &Step::Compute {
                name: "y".into(),
                left: Operand::Column(ColumnId(1)),
                operator: Arithmetic::Add,
                right: Operand::Constant(1.),
            },
            &c,
            &[],
            0,
        );
        crate::operation::CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
        assert!(matches!(result, Err(AppError::Cancelled)));
        assert_eq!(p.dataset().column_count(), 1);
    }
}
