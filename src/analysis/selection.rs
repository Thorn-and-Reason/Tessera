use super::{DescribeRequest, MissingPolicy, RunDiagnostics};
use crate::{
    data::{Cell, ColumnId, Dataset},
    error::{AppError, AppResult},
    operation::JobControl,
};
use std::collections::BTreeSet;

pub const MAX_FREQUENCY: u64 = 1 << 53;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumericComparison {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}
#[derive(Debug, Clone, PartialEq)]
pub enum RowPredicate {
    Number {
        column: ColumnId,
        comparison: NumericComparison,
        value: f64,
    },
    Text {
        column: ColumnId,
        value: String,
        equal: bool,
    },
    Missing {
        column: ColumnId,
        missing: bool,
    },
}
impl RowPredicate {
    pub fn column(&self) -> ColumnId {
        match self {
            Self::Number { column, .. }
            | Self::Text { column, .. }
            | Self::Missing { column, .. } => *column,
        }
    }
    fn matches(&self, dataset: &Dataset, row: usize, control: &JobControl) -> AppResult<bool> {
        let column = dataset.column(self.column())?;
        let missing = column.is_missing_controlled(row, control)?;
        if let Self::Missing {
            missing: wanted, ..
        } = self
        {
            return Ok(missing == *wanted);
        }
        if missing {
            return Ok(false);
        }
        if let Self::Text { value, equal, .. } = self {
            return match column.text_value(row)? {
                Some(actual) => Ok((super::exploration::ordering::compare_text(
                    actual, value, control,
                )? == std::cmp::Ordering::Equal)
                    == *equal),
                None => Ok(false),
            };
        }
        Ok(match (self, column.cell(row)?) {
            (
                Self::Number {
                    comparison, value, ..
                },
                Cell::Number(actual),
            ) => match comparison {
                NumericComparison::Equal => actual == *value,
                NumericComparison::NotEqual => actual != *value,
                NumericComparison::Less => actual < *value,
                NumericComparison::LessOrEqual => actual <= *value,
                NumericComparison::Greater => actual > *value,
                NumericComparison::GreaterOrEqual => actual >= *value,
            },
            _ => false,
        })
    }
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RowFilter {
    pub predicates: Vec<RowPredicate>,
}
impl RowFilter {
    pub(crate) fn matches(
        &self,
        dataset: &Dataset,
        row: usize,
        control: &JobControl,
    ) -> AppResult<bool> {
        let quiet = control.without_progress();
        for (index, predicate) in self.predicates.iter().enumerate() {
            if index.is_multiple_of(4096) {
                quiet.checkpoint(index, None)?;
            }
            if !predicate.matches(dataset, row, &quiet)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
pub(crate) fn validation_bytes(request: &DescribeRequest) -> AppResult<usize> {
    super::AnalysisRequestRef::Describe(request).validation_scratch_bytes()
}
pub(crate) fn validate(
    dataset: &Dataset,
    request: &DescribeRequest,
) -> AppResult<BTreeSet<ColumnId>> {
    super::support::validate_current(dataset, super::AnalysisRequestRef::Describe(request))
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct Eligible {
    pub row: usize,
    pub weight: u64,
    pub complete: bool,
}
pub(crate) fn select(
    dataset: &Dataset,
    request: &DescribeRequest,
    control: &JobControl,
    total_work: usize,
) -> AppResult<(Vec<Eligible>, RunDiagnostics)> {
    select_rows(
        dataset,
        &request.filter,
        request.frequency_weight,
        if request.missing_policy == MissingPolicy::CompleteCase {
            &request.columns
        } else {
            &[]
        },
        control,
        total_work,
    )
}
pub(crate) fn select_rows(
    dataset: &Dataset,
    filter: &RowFilter,
    frequency_weight: Option<ColumnId>,
    complete_columns: &[ColumnId],
    control: &JobControl,
    total_work: usize,
) -> AppResult<(Vec<Eligible>, RunDiagnostics)> {
    control.check_cancelled()?;
    let mut eligible = Vec::with_capacity(dataset.row_count());
    let mut diagnostic = RunDiagnostics::default();
    let mut total = 0_u64;
    let mut operations = 0usize;
    let quiet = control.without_progress();
    for row in 0..dataset.row_count() {
        if row % 4096 == 0 {
            control.checkpoint(row, Some(total_work))?;
        }
        if !filter.matches(dataset, row, &quiet)? {
            diagnostic.filtered_rows += 1;
            continue;
        }
        let Some(weight) = row_weight(dataset, frequency_weight, row, &mut diagnostic, &quiet)?
        else {
            continue;
        };
        total = total
            .checked_add(weight)
            .filter(|n| *n <= MAX_FREQUENCY)
            .ok_or_else(|| AppError::invalid("weights", "Total frequency exceeds 2^53."))?;
        let mut complete = true;
        for id in complete_columns {
            operations = operations.saturating_add(1);
            if operations.is_multiple_of(4096) {
                quiet.checkpoint(operations, None)?;
            }
            let missing = dataset.column(*id)?.is_missing_controlled(row, &quiet)?;
            complete &= !missing;
        }
        if !complete {
            diagnostic.complete_case_rows += 1;
        }
        eligible.push(Eligible {
            row,
            weight,
            complete,
        });
    }
    control.checkpoint(dataset.row_count(), Some(total_work))?;
    Ok((eligible, diagnostic))
}

fn row_weight(
    dataset: &Dataset,
    frequency_weight: Option<ColumnId>,
    row: usize,
    diagnostic: &mut RunDiagnostics,
    control: &JobControl,
) -> AppResult<Option<u64>> {
    let weight = match frequency_weight {
        None => 1,
        Some(id) if dataset.column(id)?.is_missing_controlled(row, control)? => {
            diagnostic.missing_weight_rows += 1;
            return Ok(None);
        }
        Some(id) => match dataset.cell(id, row)? {
            Cell::Number(value)
                if value >= 0.0 && value.fract() == 0.0 && value <= MAX_FREQUENCY as f64 =>
            {
                value as u64
            }
            _ => {
                return Err(AppError::invalid(
                    format!("weight, row {}", row + 1),
                    "Use a nonnegative integer frequency weight no greater than 2^53.",
                ))
            }
        },
    };
    if weight == 0 {
        diagnostic.zero_weight_rows += 1;
        Ok(None)
    } else {
        Ok(Some(weight))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Column;

    #[test]
    fn base_cohort_checks_weights_before_unselected_groups_and_missing_outcomes() {
        let dataset = Dataset::new(vec![
            Column::numeric(ColumnId(1), "Outcome", vec![None, Some(2.0)]).unwrap(),
            Column::text(
                ColumnId(2),
                "Group",
                vec![Some("Other".into()), Some("A".into())],
            )
            .unwrap(),
            Column::numeric(ColumnId(3), "Weight", vec![Some(-1.0), Some(2.0)]).unwrap(),
        ])
        .unwrap();
        assert!(select_rows(
            &dataset,
            &RowFilter::default(),
            Some(ColumnId(3)),
            &[],
            &JobControl::default(),
            2
        )
        .is_err());
        let filter = RowFilter {
            predicates: vec![RowPredicate::Text {
                column: ColumnId(2),
                value: "A".into(),
                equal: true,
            }],
        };
        let (rows, diagnostics) = select_rows(
            &dataset,
            &filter,
            Some(ColumnId(3)),
            &[],
            &JobControl::default(),
            2,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].row, 1);
        assert_eq!(rows[0].weight, 2);
        assert!(rows[0].complete);
        assert_eq!(diagnostics.filtered_rows, 1);
        let (base, diagnostics) = select_rows(
            &dataset,
            &RowFilter::default(),
            None,
            &[],
            &JobControl::default(),
            2,
        )
        .unwrap();
        assert_eq!(base.len(), 2);
        assert!(base.iter().all(|row| row.complete));
        assert_eq!(diagnostics.complete_case_rows, 0);
        let (complete, diagnostics) = select_rows(
            &dataset,
            &RowFilter::default(),
            None,
            &[ColumnId(1)],
            &JobControl::default(),
            2,
        )
        .unwrap();
        assert!(!complete[0].complete);
        assert!(complete[1].complete);
        assert_eq!(diagnostics.complete_case_rows, 1);
    }
}

#[cfg(test)]
mod regression_cancellation_tests {
    use super::*;
    use crate::{data::Column, operation::CHECKPOINT_HOOK};
    fn cancels(filter: RowFilter, threshold: usize, text: String) {
        let data = Dataset::new(vec![
            Column::text(ColumnId(1), "filter", vec![Some(text)]).unwrap()
        ])
        .unwrap();
        let control = JobControl::default();
        let cancel = control.clone();
        CHECKPOINT_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |p| {
                if p.total.is_none() && p.done >= threshold {
                    cancel.cancel()
                }
            }))
        });
        let result = select_rows(&data, &filter, None, &[], &control, 1);
        CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
        assert!(
            matches!(result, Err(AppError::Cancelled)),
            "threshold {threshold}"
        );
    }
    #[test]
    fn cancellation_inside_long_filter_text() {
        let text = "ŵ".repeat(20000) + "a";
        cancels(
            RowFilter {
                predicates: vec![RowPredicate::Text {
                    column: ColumnId(1),
                    value: text.clone(),
                    equal: true,
                }],
            },
            8192,
            text,
        );
    }
    #[test]
    fn cancellation_inside_wide_predicate_scan() {
        cancels(
            RowFilter {
                predicates: vec![
                    RowPredicate::Missing {
                        column: ColumnId(1),
                        missing: false
                    };
                    20000
                ],
            },
            4096,
            "a".into(),
        );
    }
}

#[cfg(test)]
mod controlled_missing_tests {
    use super::*;
    use crate::{
        data::{Column, ColumnMetadata, MissingRules},
        operation::CHECKPOINT_HOOK,
    };
    use std::sync::{atomic::AtomicBool, Arc};
    fn wide(id: u64) -> Column {
        let col = Column::numeric(ColumnId(id), "wide", vec![Some(10000.)]).unwrap();
        col.with_metadata(ColumnMetadata {
            missing_rules: MissingRules::Number {
                values: (0..200).map(|i| i as f64).collect(),
                ranges: vec![],
            },
            ..col.metadata().clone()
        })
        .unwrap()
    }
    fn run(d: Dataset, filter: RowFilter, weight: Option<ColumnId>, complete: &[ColumnId]) {
        let control = JobControl::new(Arc::new(AtomicBool::new(false)), None);
        let cancel = control.clone();
        CHECKPOINT_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |p| {
                if p.total.is_none() && p.done >= 64 {
                    cancel.cancel()
                }
            }))
        });
        let result = select_rows(&d, &filter, weight, complete, &control, 1);
        CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
        assert!(matches!(result, Err(AppError::Cancelled)), "{result:?}");
    }
    #[test]
    fn supplied_control_reaches_filter_and_weight_missing_lookup() {
        let d = Dataset::new(vec![wide(1)]).unwrap();
        run(
            d.clone(),
            RowFilter {
                predicates: vec![RowPredicate::Missing {
                    column: ColumnId(1),
                    missing: false,
                }],
            },
            None,
            &[],
        );
        run(d, RowFilter::default(), Some(ColumnId(1)), &[]);
    }
    #[test]
    fn supplied_control_reaches_later_items_after_first_missing_item() {
        let d = Dataset::new(vec![
            Column::numeric(ColumnId(1), "missing", vec![None]).unwrap(),
            wide(2),
        ])
        .unwrap();
        run(d, RowFilter::default(), None, &[ColumnId(1), ColumnId(2)]);
    }
    #[test]
    fn supplied_control_reaches_long_missing_text_equality() {
        let text = "ŵ".repeat(10000);
        let col = Column::text(ColumnId(1), "text", vec![Some(text.clone())]).unwrap();
        let col = col
            .with_metadata(ColumnMetadata {
                missing_rules: MissingRules::Text(vec![text]),
                ..col.metadata().clone()
            })
            .unwrap();
        run(
            Dataset::new(vec![col]).unwrap(),
            RowFilter {
                predicates: vec![RowPredicate::Missing {
                    column: ColumnId(1),
                    missing: true,
                }],
            },
            None,
            &[],
        );
    }
}
