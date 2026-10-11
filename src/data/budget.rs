use super::{column::Chunks, Column, Dataset};
use crate::error::{AppError, AppResult};
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
        let allocations =
            datasets
                .flat_map(dataset_allocations)
                .chain(column.into_iter().flat_map(|column| {
                    std::iter::once(OwnedAllocation::Column(column))
                        .chain(column_allocations(column))
                }));
        count_allocations(allocations, extra_bytes)
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
    fn charge(self, counter: &mut AllocationCounter) -> AppResult<()> {
        match self {
            Self::Dataset(dataset) => {
                counter.add(size_of::<Dataset>())?;
                counter.array::<Column>(dataset.columns.capacity())?;
                for column in &dataset.columns {
                    charge_references(column, counter)?;
                }
            }
            Self::Column(column) => {
                counter.add(size_of::<Column>())?;
                charge_references(column, counter)?;
            }
            Self::Metadata(metadata) => {
                counter.add(size_of::<super::ColumnMetadata>() + 2 * size_of::<usize>())?;
                counter.add(metadata.heap_bytes()?)?;
            }
            Self::Number(chunk) => {
                counter.add(2 * size_of::<usize>())?;
                counter.array::<Option<f64>>(chunk.len())?;
            }
            Self::Text(chunk) => {
                counter.add(2 * size_of::<usize>())?;
                counter.array::<Option<String>>(chunk.len())?;
                for value in chunk.iter().flatten() {
                    counter.add(value.capacity())?;
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
) -> AppResult<usize> {
    let mut upper = AllocationCounter::default();
    upper.add(extra_bytes)?;
    let mut entries = 0;
    let bound = allocations.clone().try_for_each(|allocation| {
        entries += 1;
        allocation.charge(&mut upper)
    });
    if bound.is_ok() && scratch_fits(upper.bytes, entries).is_ok() {
        return Ok(upper.bytes);
    }
    let mut exact = AllocationCounter::default();
    exact.add(extra_bytes)?;
    let mut unique = 0;
    // ponytail: sharing-heavy/tight-budget fallback scans prior identities in O(n²); use a fully reserved index if profiling shows a bottleneck.
    for (index, allocation) in allocations.clone().enumerate() {
        if allocations
            .clone()
            .take(index)
            .any(|prior| prior.pointer() == allocation.pointer())
        {
            continue;
        }
        allocation.charge(&mut exact)?;
        unique += 1;
    }
    scratch_fits(exact.bytes, unique)?;
    Ok(exact.bytes)
}
fn count_allocations<'a>(
    allocations: impl Iterator<Item = OwnedAllocation<'a>> + Clone,
    extra_bytes: usize,
) -> AppResult<usize> {
    let baseline = complete_baseline(allocations.clone(), extra_bytes)?;
    let mut seen = AllocationSet {
        pointers: Vec::new(),
        baseline,
    };
    let mut counter = AllocationCounter::default();
    counter.add(extra_bytes)?;
    for allocation in allocations {
        seen.count(allocation.pointer(), &mut counter, |counter| {
            allocation.charge(counter)
        })?;
    }
    Ok(counter.bytes)
}
