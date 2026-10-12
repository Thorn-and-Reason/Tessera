use super::{column::Chunks, Column, Dataset};
use crate::{
    error::{AppError, AppResult},
    operation::JobControl,
};
use std::{mem::size_of, sync::Arc};
pub const MAX_OWNED_BYTES: usize = 256 * 1024 * 1024;

pub struct Budget;
#[derive(Default)]
pub(crate) struct AllocationCounter {
    pub(crate) bytes: usize,
}
impl AllocationCounter {
    pub(crate) fn add(&mut self, bytes: usize) -> AppResult<()> {
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .filter(|n| *n <= MAX_OWNED_BYTES)
            .ok_or(AppError::SizeLimit {
                limit: MAX_OWNED_BYTES,
            })?;
        Ok(())
    }
    pub(crate) fn array<T>(&mut self, count: usize) -> AppResult<()> {
        self.add(
            count
                .checked_mul(size_of::<T>())
                .ok_or(AppError::SizeLimit {
                    limit: MAX_OWNED_BYTES,
                })?,
        )
    }
}
#[derive(Default)]
pub(crate) struct AllocationSet {
    pointers: Vec<*const ()>,
    baseline: usize,
}
impl AllocationSet {
    pub(crate) fn guard(&self, counter: &AllocationCounter) -> AppResult<()> {
        let mut peak = AllocationCounter {
            bytes: self.baseline.max(counter.bytes),
        };
        peak.array::<*const ()>(self.pointers.capacity())
    }
    pub(crate) fn count(
        &mut self,
        pointer: *const (),
        counter: &mut AllocationCounter,
        charge: impl FnOnce(&mut AllocationCounter) -> AppResult<()>,
    ) -> AppResult<bool> {
        let index = match self.pointers.binary_search(&pointer) {
            Ok(_) => return Ok(false),
            Err(index) => index,
        };
        charge(counter)?;
        if self.pointers.len() == self.pointers.capacity() {
            let capacity = self
                .pointers
                .capacity()
                .checked_mul(2)
                .map(|capacity| capacity.max(1))
                .ok_or(AppError::SizeLimit {
                    limit: MAX_OWNED_BYTES,
                })?;
            let mut peak = AllocationCounter {
                bytes: self.baseline.max(counter.bytes),
            };
            peak.array::<*const ()>(self.pointers.capacity())?;
            peak.array::<*const ()>(capacity)?;
            let mut next = Vec::with_capacity(capacity);
            next.extend_from_slice(&self.pointers);
            self.pointers = next;
        } else {
            self.guard(counter)?;
        }
        // ponytail: sorted pointer insertion shifts O(n) entries; batch/index accounting if very wide datasets become a measured bottleneck.
        self.pointers.insert(index, pointer);
        Ok(true)
    }
}

impl Budget {
    pub(crate) fn check_column(column: &Column, extra_bytes: usize) -> AppResult<()> {
        Self::column_bytes(column, extra_bytes).map(|_| ())
    }
    pub(crate) fn column_bytes(column: &Column, extra_bytes: usize) -> AppResult<usize> {
        count_allocations(
            std::iter::once(OwnedAllocation::Column(column)).chain(column_allocations(column)),
            extra_bytes,
            None,
        )
    }
    pub(crate) fn context_bytes(
        dataset: &Dataset,
        retained: &[&Dataset],
        column: Option<&Column>,
        extra_bytes: usize,
    ) -> AppResult<usize> {
        Self::count_datasets(
            std::iter::once(dataset).chain(retained.iter().copied()),
            column,
            extra_bytes,
        )
    }
    pub(crate) fn context_bytes_controlled(
        dataset: &Dataset,
        retained: &[&Dataset],
        column: Option<&Column>,
        extra_bytes: usize,
        control: &JobControl,
    ) -> AppResult<usize> {
        let quiet = control.without_progress();
        Self::count_datasets_controlled(
            std::iter::once(dataset).chain(retained.iter().copied()),
            column,
            extra_bytes,
            Some(&quiet),
        )
    }
    pub(crate) fn replacement_bytes(
        dataset: &Dataset,
        next: &Dataset,
        retained: &[&Dataset],
        extra_bytes: usize,
    ) -> AppResult<usize> {
        Self::count_datasets(
            [dataset, next].into_iter().chain(retained.iter().copied()),
            None,
            extra_bytes,
        )
    }
    fn count_datasets<'a>(
        datasets: impl Iterator<Item = &'a Dataset> + Clone,
        column: Option<&'a Column>,
        extra_bytes: usize,
    ) -> AppResult<usize> {
        Self::count_datasets_controlled(datasets, column, extra_bytes, None)
    }
    fn count_datasets_controlled<'a>(
        datasets: impl Iterator<Item = &'a Dataset> + Clone,
        column: Option<&'a Column>,
        extra_bytes: usize,
        control: Option<&JobControl>,
    ) -> AppResult<usize> {
        let allocations =
            datasets
                .flat_map(dataset_allocations)
                .chain(column.into_iter().flat_map(|column| {
                    std::iter::once(OwnedAllocation::Column(column))
                        .chain(column_allocations(column))
                }));
        count_allocations(allocations, extra_bytes, control)
    }
    pub fn check_datasets(datasets: &[&Dataset], extra_bytes: usize) -> AppResult<()> {
        Self::dataset_bytes(datasets, extra_bytes).map(|_| ())
    }
    /// Use borrowing iterators whose Clone does not allocate, such as slice/option chains.
    pub(crate) fn dataset_bytes_iter<'a>(
        datasets: impl Iterator<Item = &'a Dataset> + Clone,
        extra_bytes: usize,
    ) -> AppResult<usize> {
        Self::count_datasets(datasets, None, extra_bytes)
    }
    pub(crate) fn dataset_bytes_iter_controlled<'a>(
        datasets: impl Iterator<Item = &'a Dataset> + Clone,
        extra_bytes: usize,
        control: Option<&JobControl>,
    ) -> AppResult<usize> {
        Self::count_datasets_controlled(datasets, None, extra_bytes, control)
    }
    pub fn dataset_bytes(datasets: &[&Dataset], extra_bytes: usize) -> AppResult<usize> {
        Self::dataset_bytes_iter(datasets.iter().copied(), extra_bytes)
    }
}
#[derive(Clone, Copy)]
enum OwnedAllocation<'a> {
    Dataset(&'a Dataset),
    Column(&'a Column),
    Metadata(&'a super::ColumnMetadata),
    Number(&'a [Option<f64>]),
    Text(&'a [Option<String>]),
}
impl OwnedAllocation<'_> {
    fn pointer(self) -> *const () {
        match self {
            Self::Dataset(value) => (value as *const Dataset).cast(),
            Self::Column(value) => (value as *const Column).cast(),
            Self::Metadata(value) => (value as *const super::ColumnMetadata).cast(),
            Self::Number(value) => value.as_ptr().cast(),
            Self::Text(value) => value.as_ptr().cast(),
        }
    }
    fn charge(
        self,
        counter: &mut AllocationCounter,
        control: Option<&JobControl>,
    ) -> AppResult<()> {
        match self {
            Self::Dataset(dataset) => {
                counter.add(size_of::<Dataset>())?;
                counter.array::<Column>(dataset.columns.capacity())?;
                for (index, column) in dataset.columns.iter().enumerate() {
                    ownership_checkpoint(control, index)?;
                    charge_references(column, counter)?;
                }
            }
            Self::Column(column) => {
                counter.add(size_of::<Column>())?;
                charge_references(column, counter)?;
            }
            Self::Metadata(metadata) => {
                counter.add(size_of::<super::ColumnMetadata>() + 2 * size_of::<usize>())?;
                counter.add(metadata.heap_bytes_controlled(control)?)?;
            }
            Self::Number(chunk) => {
                counter.add(2 * size_of::<usize>())?;
                counter.array::<Option<f64>>(chunk.len())?;
            }
            Self::Text(chunk) => {
                counter.add(2 * size_of::<usize>())?;
                counter.array::<Option<String>>(chunk.len())?;
                for (index, value) in chunk.iter().enumerate() {
                    if index.is_multiple_of(256) {
                        ownership_checkpoint(control, index)?;
                    }
                    if let Some(value) = value {
                        counter.add(value.capacity())?;
                    }
                }
            }
        }
        Ok(())
    }
}
fn charge_references(column: &Column, counter: &mut AllocationCounter) -> AppResult<()> {
    match &column.chunks {
        Chunks::Number(chunks) => counter.array::<Arc<[Option<f64>]>>(chunks.capacity()),
        Chunks::Text(chunks) => counter.array::<Arc<[Option<String>]>>(chunks.capacity()),
    }
}
fn column_allocations(column: &Column) -> impl Iterator<Item = OwnedAllocation<'_>> + Clone {
    let chunks = match &column.chunks {
        Chunks::Number(values) => values.len(),
        Chunks::Text(values) => values.len(),
    };
    std::iter::once(OwnedAllocation::Metadata(column.metadata())).chain((0..chunks).map(
        move |index| match &column.chunks {
            Chunks::Number(values) => OwnedAllocation::Number(&values[index]),
            Chunks::Text(values) => OwnedAllocation::Text(&values[index]),
        },
    ))
}
fn dataset_allocations(dataset: &Dataset) -> impl Iterator<Item = OwnedAllocation<'_>> + Clone {
    std::iter::once(OwnedAllocation::Dataset(dataset))
        .chain(dataset.columns.iter().flat_map(column_allocations))
}
fn scratch_fits(baseline: usize, entries: usize) -> AppResult<()> {
    let mut peak = AllocationCounter { bytes: baseline };
    let capacity = if entries == 0 {
        0
    } else {
        entries
            .checked_next_power_of_two()
            .ok_or(AppError::SizeLimit {
                limit: MAX_OWNED_BYTES,
            })?
    };
    let previous = if capacity > 1 { capacity / 2 } else { 0 };
    peak.array::<*const ()>(capacity)?;
    peak.array::<*const ()>(previous)
}
fn complete_baseline<'a>(
    allocations: impl Iterator<Item = OwnedAllocation<'a>> + Clone,
    extra_bytes: usize,
    control: Option<&JobControl>,
) -> AppResult<usize> {
    ownership_checkpoint(control, 0)?;
    let mut upper = AllocationCounter::default();
    upper.add(extra_bytes)?;
    let mut entries = 0;
    let bound = allocations.clone().try_for_each(|allocation| {
        ownership_checkpoint(control, entries)?;
        entries += 1;
        allocation.charge(&mut upper, control)
    });
    if matches!(bound, Err(AppError::Cancelled)) {
        return Err(AppError::Cancelled);
    }
    if bound.is_ok() && scratch_fits(upper.bytes, entries).is_ok() {
        return Ok(upper.bytes);
    }
    let mut exact = AllocationCounter::default();
    exact.add(extra_bytes)?;
    let mut unique = 0;
    // ponytail: sharing-heavy/tight-budget fallback scans prior identities in O(n²); use a fully reserved index if profiling shows a bottleneck.
    for (index, allocation) in allocations.clone().enumerate() {
        ownership_checkpoint(control, index)?;
        let mut shared = false;
        for (prior_index, prior) in allocations.clone().take(index).enumerate() {
            ownership_checkpoint(control, prior_index)?;
            if prior.pointer() == allocation.pointer() {
                shared = true;
                break;
            }
        }
        if shared {
            continue;
        }
        allocation.charge(&mut exact, control)?;
        unique += 1;
    }
    scratch_fits(exact.bytes, unique)?;
    Ok(exact.bytes)
}
fn count_allocations<'a>(
    allocations: impl Iterator<Item = OwnedAllocation<'a>> + Clone,
    extra_bytes: usize,
    control: Option<&JobControl>,
) -> AppResult<usize> {
    let baseline = complete_baseline(allocations.clone(), extra_bytes, control)?;
    let mut seen = AllocationSet {
        pointers: Vec::new(),
        baseline,
    };
    let mut counter = AllocationCounter::default();
    counter.add(extra_bytes)?;
    for (index, allocation) in allocations.enumerate() {
        ownership_checkpoint(control, index)?;
        seen.count(allocation.pointer(), &mut counter, |counter| {
            allocation.charge(counter, control)
        })?;
    }
    Ok(counter.bytes)
}

fn ownership_checkpoint(control: Option<&JobControl>, done: usize) -> AppResult<()> {
    if let Some(control) = control {
        control.checkpoint(done, None)?;
    }
    Ok(())
}

#[cfg(test)]
mod controlled_tests {
    use super::*;
    use crate::{data::ColumnId, operation::JobControl};
    use std::{cell::Cell, rc::Rc};
    #[test]
    fn retained_text_ownership_can_cancel_mid_scan() {
        let source = Dataset::new(vec![
            Column::numeric(ColumnId(1), "Choice", vec![Some(1.)]).unwrap()
        ])
        .unwrap();
        let retained = Dataset::new(vec![Column::text(
            ColumnId(1),
            "Choice",
            vec![Some("text".into()); 8192],
        )
        .unwrap()])
        .unwrap();
        let control = JobControl::default();
        let cancellation = control.clone();
        let checkpoints = Rc::new(Cell::new(0));
        let count = Rc::clone(&checkpoints);
        crate::operation::CHECKPOINT_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |progress| {
                let next = count.get() + 1;
                count.set(next);
                if progress.done == 2048 {
                    cancellation.cancel();
                }
            }))
        });
        let result = Budget::context_bytes_controlled(&source, &[&retained], None, 0, &control);
        crate::operation::CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
        assert!(matches!(result, Err(AppError::Cancelled)));
        assert!(checkpoints.get() > 7);
        assert_eq!(retained.row_count(), 8192);
        assert_eq!(source.revision(), crate::data::RevisionId(1));
    }
    #[test]
    fn controlled_accounting_preserves_shared_ownership_totals() {
        let column = Column::text(ColumnId(1), "Choice", vec![Some("answer".into()); 32]).unwrap();
        let mut metadata = column.metadata().clone();
        metadata.missing_rules = crate::data::MissingRules::Text(vec!["Missing".into()]);
        let source = Dataset::new(vec![column.with_metadata(metadata).unwrap()]).unwrap();
        let shared = source.clone();
        let control = JobControl::default();
        assert_eq!(
            Budget::context_bytes_controlled(&source, &[&shared], None, 17, &control).unwrap(),
            Budget::context_bytes(&source, &[&shared], None, 17).unwrap()
        );
        let mut metadata_counter = AllocationCounter::default();
        let metadata = source.columns()[0].metadata();
        metadata_counter
            .add(metadata.heap_bytes().unwrap())
            .unwrap();
        assert_eq!(
            metadata_counter.bytes,
            metadata.heap_bytes_controlled(Some(&control)).unwrap()
        );
    }
    #[test]
    fn sharing_heavy_identity_fallback_can_cancel() {
        let values = [Some(1.)];
        let allocations = [OwnedAllocation::Number(&values); 4];
        let control = JobControl::default();
        let cancellation = control.clone();
        let armed = Rc::new(Cell::new(false));
        let observed = Rc::clone(&armed);
        crate::operation::CHECKPOINT_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |progress| {
                if progress.done == 1 {
                    observed.set(true);
                } else if progress.done == 0 && observed.get() {
                    cancellation.cancel();
                }
            }))
        });
        let result = complete_baseline(
            allocations.into_iter(),
            MAX_OWNED_BYTES - 48,
            Some(&control),
        );
        crate::operation::CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
        assert!(armed.get());
        assert!(matches!(result, Err(AppError::Cancelled)));
    }
}

#[cfg(test)]
mod project_iterator_tests {
    use super::*;
    use crate::{data::ColumnId, operation::CHECKPOINT_HOOK};
    #[test]
    fn controlled_project_iterator_matches_totals_and_cancels() {
        let dataset = Dataset::new(vec![Column::text(
            ColumnId(1),
            "Synthetic",
            vec![Some("choice".into()); 8192],
        )
        .unwrap()])
        .unwrap();
        let shared = dataset.clone();
        let control = JobControl::default();
        assert_eq!(
            Budget::dataset_bytes_iter_controlled(
                [&dataset, &shared].into_iter(),
                17,
                Some(&control)
            )
            .unwrap(),
            Budget::dataset_bytes(&[&dataset, &shared], 17).unwrap()
        );
        let cancel = control.clone();
        CHECKPOINT_HOOK.with(|h| {
            *h.borrow_mut() = Some(Box::new(move |progress| {
                if progress.done == 2048 {
                    cancel.cancel();
                }
            }))
        });
        let result = Budget::dataset_bytes_iter_controlled(
            [&dataset, &shared].into_iter(),
            17,
            Some(&control),
        );
        CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
        assert!(matches!(result, Err(AppError::Cancelled)));
    }
}
