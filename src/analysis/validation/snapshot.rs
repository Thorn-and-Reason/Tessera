//! One metadata invariant owner; controlled callers precharge label sorting scratch.
use super::*;
use crate::{
    analysis::exploration::ordering::{compare_cells, sort_indices},
    operation::JobControl,
};
use std::cmp::Ordering;
pub(crate) fn snapshot_scratch_bytes(info: &RunColumnInfo) -> AppResult<usize> {
    crate::analysis::model::checked_product(
        info.value_labels.len(),
        2 * std::mem::size_of::<usize>(),
    )
}
pub(crate) fn validate_snapshot(info: &RunColumnInfo) -> AppResult<()> {
    core(info, None)
}
pub(crate) fn validate_snapshot_controlled(info: &RunColumnInfo, c: &JobControl) -> AppResult<()> {
    c.check_cancelled()?;
    let quiet = c.without_progress();
    core(info, Some(&quiet))
}
fn checkpoint(c: Option<&JobControl>, i: usize) -> AppResult<()> {
    if let Some(c) = c {
        c.checkpoint(i, None)?;
    }
    Ok(())
}
fn text(s: &str, c: Option<&JobControl>, name: bool) -> AppResult<()> {
    let mut nonblank = false;
    let mut next = 0;
    for (i, ch) in s.char_indices() {
        if i >= next {
            checkpoint(c, i)?;
            next = i.saturating_add(4096);
        }
        nonblank |= !ch.is_whitespace();
        if name && ch.is_control() {
            return Err(invalid_analysis());
        }
    }
    checkpoint(c, s.len())?;
    if name && !nonblank {
        return Err(invalid_analysis());
    }
    Ok(())
}
fn rules(info: &RunColumnInfo, c: Option<&JobControl>) -> AppResult<()> {
    match (&info.missing_rules, info.kind) {
        (MissingRules::None, _) => {}
        (MissingRules::Number { values, ranges }, ColumnKind::Number) => {
            for (i, v) in values.iter().enumerate() {
                if i.is_multiple_of(64) {
                    checkpoint(c, i)?;
                }
                if !v.is_finite() {
                    return Err(invalid_analysis());
                }
            }
            for (i, (lo, hi)) in ranges.iter().enumerate() {
                if i.is_multiple_of(64) {
                    checkpoint(c, i)?;
                }
                if !lo.is_finite() || !hi.is_finite() || lo > hi {
                    return Err(invalid_analysis());
                }
            }
        }
        (MissingRules::Text(values), ColumnKind::Text) => {
            for v in values {
                text(v, c, false)?;
            }
        }
        _ => return Err(invalid_analysis()),
    }
    Ok(())
}
fn core(info: &RunColumnInfo, c: Option<&JobControl>) -> AppResult<()> {
    checkpoint(c, 0)?;
    if info.id.0 == 0 {
        return Err(invalid_analysis());
    }
    text(&info.name, c, true)?;
    text(&info.description, c, false)?;
    rules(info, c)?;
    for (i, label) in info.value_labels.iter().enumerate() {
        if i.is_multiple_of(64) {
            checkpoint(c, i)?;
        }
        match (&label.value, info.kind) {
            (Cell::Number(v), ColumnKind::Number) if v.is_finite() => {}
            (Cell::Text(v), ColumnKind::Text) => text(v, c, false)?,
            _ => return Err(invalid_analysis()),
        }
        text(&label.label, c, false)?;
    }
    unique(info, c)?;
    checkpoint(c, 0)
}
fn unique(info: &RunColumnInfo, c: Option<&JobControl>) -> AppResult<()> {
    if let Some(c) = c {
        c.check_cancelled()?;
        let mut indices = Vec::with_capacity(info.value_labels.len());
        for i in 0..info.value_labels.len() {
            if i.is_multiple_of(64) {
                checkpoint(Some(c), i)?;
            }
            indices.push(i);
        }
        c.check_cancelled()?;
        let mut scratch = vec![0; indices.len()];
        sort_indices(&mut indices, &mut scratch, c, |a, b| {
            compare_cells(&info.value_labels[a].value, &info.value_labels[b].value, c)
        })?;
        for pair in indices.windows(2) {
            if compare_cells(
                &info.value_labels[pair[0]].value,
                &info.value_labels[pair[1]].value,
                c,
            )? == Ordering::Equal
            {
                return Err(invalid_analysis());
            }
        }
    } else {
        let mut numbers = BTreeSet::new();
        let mut texts = BTreeSet::new();
        for label in &info.value_labels {
            let unique = match &label.value {
                Cell::Number(v) => numbers.insert(if *v == 0. { 0 } else { v.to_bits() }),
                Cell::Text(v) => texts.insert(v),
                _ => return Err(invalid_analysis()),
            };
            if !unique {
                return Err(invalid_analysis());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        data::{ColumnKind, ValueLabel},
        error::AppError,
        operation::CHECKPOINT_HOOK,
    };
    fn info() -> RunColumnInfo {
        RunColumnInfo {
            id: ColumnId(1),
            name: "Item".into(),
            kind: ColumnKind::Text,
            description: "ŵ".repeat(9000),
            missing_rules: MissingRules::Text(vec!["x".into()]),
            value_labels: vec![ValueLabel {
                value: Cell::Text("a".into()),
                label: "A".into(),
            }],
        }
    }
    #[test]
    fn supplied_cancelled_snapshot_control_allocates_nothing() {
        let info = info();
        let c = JobControl::default();
        c.cancel();
        let (result, largest) =
            crate::data::measure_test_allocations(|| validate_snapshot_controlled(&info, &c));
        assert!(matches!(result, Err(AppError::Cancelled)));
        assert_eq!(largest, 0);
    }
    #[test]
    fn controlled_snapshot_cancels_inside_long_text_without_cloning() {
        let info = info();
        let c = JobControl::default();
        let cancel = c.clone();
        CHECKPOINT_HOOK.with(|h| {
            *h.borrow_mut() = Some(Box::new(move |p| {
                if p.done >= 4096 {
                    cancel.cancel();
                }
            }))
        });
        let (result, largest) =
            crate::data::measure_test_allocations(|| validate_snapshot_controlled(&info, &c));
        CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
        assert!(matches!(result, Err(AppError::Cancelled)));
        assert_eq!(largest, 0);
    }
    #[test]
    fn both_snapshot_routes_preserve_metadata_invariants() {
        let mut info = info();
        validate_snapshot(&info).unwrap();
        validate_snapshot_controlled(&info, &JobControl::default()).unwrap();
        info.value_labels.push(info.value_labels[0].clone());
        assert!(validate_snapshot(&info).is_err());
        assert!(validate_snapshot_controlled(&info, &JobControl::default()).is_err());
        info.value_labels.clear();
        info.kind = ColumnKind::Number;
        info.missing_rules = MissingRules::Number {
            values: vec![],
            ranges: vec![(2., 1.)],
        };
        assert!(validate_snapshot(&info).is_err());
        assert!(validate_snapshot_controlled(&info, &JobControl::default()).is_err());
    }
}
