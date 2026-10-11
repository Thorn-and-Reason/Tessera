//! Precharged borrowed label lookup; returned indexes retain captured metadata order.
use super::ordering::{compare_cells, sort_indices};
use crate::{
    analysis::{
        model::{checked_product, checked_sum},
        RunColumnInfo,
    },
    data::Cell,
    error::AppResult,
    operation::JobControl,
};
use std::{cmp::Ordering, mem::size_of};

pub(crate) struct LabelIndex<'a> {
    info: &'a RunColumnInfo,
    indices: Vec<usize>,
}
impl<'a> LabelIndex<'a> {
    /// Charge before `new`; covers both live index and temporary merge scratch.
    pub(crate) fn scratch_bytes(count: usize) -> AppResult<usize> {
        checked_product(
            count,
            checked_sum([size_of::<usize>(), size_of::<usize>()])?,
        )
    }
    pub(crate) fn new(info: &'a RunColumnInfo, control: &JobControl) -> AppResult<Self> {
        let quiet = control.without_progress();
        let control = &quiet;
        let mut indices = Vec::with_capacity(info.value_labels.len());
        for index in 0..info.value_labels.len() {
            if index.is_multiple_of(4096) {
                control.checkpoint(index, None)?;
            }
            indices.push(index);
        }
        let mut scratch = vec![0; indices.len()];
        sort_indices(&mut indices, &mut scratch, control, |a, b| {
            compare_cells(
                &info.value_labels[a].value,
                &info.value_labels[b].value,
                control,
            )
        })?;
        Ok(Self { info, indices })
    }
    pub(crate) fn find(&self, value: &Cell, control: &JobControl) -> AppResult<Option<usize>> {
        let (mut lo, mut hi) = (0, self.indices.len());
        while lo < hi {
            let middle = lo + (hi - lo) / 2;
            let original = self.indices[middle];
            match compare_cells(&self.info.value_labels[original].value, value, control)? {
                Ordering::Less => lo = middle + 1,
                Ordering::Greater => hi = middle,
                Ordering::Equal => return Ok(Some(original)),
            }
        }
        Ok(None)
    }
}
