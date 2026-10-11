use super::*;
use crate::{
    analysis::{moments::BoundedSum, validation::invalid_analysis},
    data::{Cell, ColumnKind, Dataset},
    error::AppResult,
};
/// Arithmetic leaf over source rows already admitted by the complete cohort.
pub fn score_item(
    d: &Dataset,
    r: &ReliabilityRequest,
    row: usize,
    item_index: usize,
) -> AppResult<f64> {
    let id = *r.items.get(item_index).ok_or_else(invalid_analysis)?;
    let column = d.column(id)?;
    score_column(
        column,
        r.reversals.iter().find(|reverse| reverse.column == id),
        row,
    )
}
/// Shared bounded scoring leaf; caller resolves source/reversal identities once.
pub(super) fn score_column(
    column: &crate::data::Column,
    reverse: Option<&ReliabilityReverse>,
    row: usize,
) -> AppResult<f64> {
    if column.kind() != ColumnKind::Number {
        return Err(invalid_analysis());
    }
    let Cell::Number(value) = column.cell(row)? else {
        return Err(invalid_analysis());
    };
    if !value.is_finite() {
        return Err(invalid_analysis());
    }
    let Some(reverse) = reverse else {
        return Ok(value);
    };
    if !reverse.lower.is_finite()
        || !reverse.upper.is_finite()
        || reverse.lower >= reverse.upper
        || value < reverse.lower
        || value > reverse.upper
    {
        return Err(invalid_analysis());
    }
    let mut sum = BoundedSum::default();
    for x in [reverse.lower, reverse.upper, -value] {
        sum.push(x, 1)?;
    }
    let result = sum.scaled(1)?.value();
    if !result.is_finite() {
        return Err(invalid_analysis());
    }
    Ok(if result == 0. { 0. } else { result })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Column, ColumnId};
    #[test]
    fn malformed_text_item_is_rejected_before_cell_clone() {
        let d = Dataset::new(vec![Column::text(
            ColumnId(1),
            "text",
            vec![Some("ŵ".repeat(50000))],
        )
        .unwrap()])
        .unwrap();
        let r = ReliabilityRequest {
            items: vec![ColumnId(1), ColumnId(2)],
            filter: Default::default(),
            frequency_weight: None,
            reversals: vec![],
        };
        let (result, largest) = crate::data::measure_test_allocations(|| score_item(&d, &r, 0, 0));
        assert!(result.is_err());
        assert!(largest < 4096, "text cell was cloned: {largest}");
    }
}
