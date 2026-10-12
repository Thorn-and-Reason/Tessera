//! Capacity-aware owned storage and precharged validation scratch.
use super::{model::RequestRef, validation, *};
use crate::{
    analysis::{
        model::{checked_product, checked_sum},
        Estimate, RowPredicate, RunColumnInfo,
    },
    data::{Budget, Cell, ColumnId, Dataset, MissingRules},
    error::AppResult,
};
use std::{collections::BTreeSet, mem::size_of};

impl RequestRef<'_> {
    fn heap_bytes(self) -> AppResult<usize> {
        let filter = self.filter();
        let mut bytes = checked_product(filter.predicates.capacity(), size_of::<RowPredicate>())?;
        for predicate in &filter.predicates {
            if let RowPredicate::Text { value, .. } = predicate {
                bytes = checked_sum([bytes, value.capacity()])?;
            }
        }
        if let Self::Frequencies(r) = self {
            bytes = checked_sum([
                bytes,
                checked_product(r.columns.capacity(), size_of::<ColumnId>())?,
            ])?;
        }
        Ok(bytes)
    }
    fn scratch_bytes(self) -> AppResult<usize> {
        let count = checked_sum([
            self.role_count(),
            self.filter().predicates.len(),
            usize::from(self.weight().is_some()),
        ])?;
        checked_product(count, 1024)
    }
    fn validate_current(
        self,
        dataset: &Dataset,
        retained: &[&Dataset],
        extra_bytes: usize,
    ) -> AppResult<BTreeSet<ColumnId>> {
        let extra = checked_sum([extra_bytes, self.scratch_bytes()?])?;
        Budget::dataset_bytes_iter(
            std::iter::once(dataset).chain(retained.iter().copied()),
            extra,
        )?;
        validation::request(self, |id| Ok(dataset.column(id)?.kind()))
    }
}
macro_rules! request_api {
    ($type:ty, $variant:ident) => {
        impl $type {
            /// Caller-owned live request storage belongs in `extra_bytes`, once.
            pub fn heap_bytes(&self) -> AppResult<usize> {
                RequestRef::$variant(self).heap_bytes()
            }
            pub fn validation_scratch_bytes(&self) -> AppResult<usize> {
                RequestRef::$variant(self).scratch_bytes()
            }
            pub fn validate_current(
                &self,
                dataset: &Dataset,
                retained: &[&Dataset],
                extra_bytes: usize,
            ) -> AppResult<BTreeSet<ColumnId>> {
                RequestRef::$variant(self).validate_current(dataset, retained, extra_bytes)
            }
        }
    };
}
request_api!(FrequenciesRequest, Frequencies);
request_api!(HistogramRequest, Histogram);
request_api!(ScatterRequest, Scatter);
request_api!(BoxplotRequest, Boxplot);

fn snapshot_bytes(columns: &Vec<RunColumnInfo>) -> AppResult<usize> {
    let mut bytes = checked_product(columns.capacity(), size_of::<RunColumnInfo>())?;
    for column in columns {
        bytes = checked_sum([bytes, column.heap_bytes()?])?;
    }
    Ok(bytes)
}
fn snapshot_scratch(columns: &[RunColumnInfo]) -> AppResult<usize> {
    let mut bytes = checked_product(columns.len(), 1024)?;
    for column in columns {
        let missing_entries = match &column.missing_rules {
            MissingRules::None => 0,
            MissingRules::Number { values, ranges } => checked_sum([values.len(), ranges.len()])?,
            MissingRules::Text(values) => values.len(),
        };
        bytes = checked_sum([
            bytes,
            checked_product(column.value_labels.len(), 1024)?,
            checked_product(missing_entries, 1024)?,
        ])?;
    }
    Ok(bytes)
}
fn reason(e: &Estimate) -> usize {
    e.reason.as_ref().map_or(0, String::capacity)
}
fn text(cell: &Cell) -> usize {
    if let Cell::Text(s) = cell {
        s.capacity()
    } else {
        0
    }
}
macro_rules! result_api {
    ($type:ty, $validator:ident, $scratch:expr) => {
        impl $type {
            pub fn validation_scratch_bytes(&self) -> AppResult<usize> {
                checked_sum([
                    self.request.validation_scratch_bytes()?,
                    snapshot_scratch(&self.columns)?,
                    ($scratch)(self)?,
                ])
            }
            pub fn validate(&self) -> AppResult<()> {
                Budget::check_datasets(
                    &[],
                    checked_sum([self.owned_bytes()?, self.validation_scratch_bytes()?])?,
                )?;
                validation::$validator(self)
            }
            fn base_bytes(&self) -> AppResult<usize> {
                checked_sum([
                    size_of::<Self>(),
                    self.request.heap_bytes()?,
                    snapshot_bytes(&self.columns)?,
                ])
            }
        }
    };
}
result_api!(
    ComputedFrequencies,
    frequencies,
    |r: &ComputedFrequencies| -> AppResult<usize> {
        if r.request.group.is_none() {
            return Ok(0);
        }
        checked_product(
            r.tables
                .iter()
                .map(|t| t.categories.len())
                .max()
                .unwrap_or(0),
            size_of::<(usize, u64)>(),
        )
    }
);
result_api!(
    ComputedHistogram,
    histogram,
    |_: &ComputedHistogram| -> AppResult<usize> { Ok(0) }
);
result_api!(ComputedScatter, scatter, |r: &ComputedScatter| {
    checked_product(r.points.len(), 256)
});
result_api!(ComputedBoxplot, boxplot, |r: &ComputedBoxplot| checked_sum(
    [
        checked_product(r.groups.len(), 1024)?,
        checked_product(r.points.len(), 1024)?
    ]
));

impl ComputedFrequencies {
    pub fn owned_bytes(&self) -> AppResult<usize> {
        let mut bytes = checked_sum([
            self.base_bytes()?,
            checked_product(self.tables.capacity(), size_of::<FrequencyTable>())?,
            checked_product(self.groups.capacity(), size_of::<FrequencyGroup>())?,
        ])?;
        for group in &self.groups {
            bytes = checked_sum([
                bytes,
                text(&group.value),
                checked_product(group.tables.capacity(), size_of::<FrequencyTable>())?,
            ])?;
        }
        for table in self
            .tables
            .iter()
            .chain(self.groups.iter().flat_map(|g| &g.tables))
        {
            bytes = checked_sum([
                bytes,
                reason(&table.missing_percent),
                checked_product(table.categories.capacity(), size_of::<FrequencyCategory>())?,
            ])?;
            for category in &table.categories {
                bytes = checked_sum([
                    bytes,
                    text(&category.value),
                    reason(&category.percent),
                    reason(&category.valid_percent),
                    category.cumulative_percent.as_ref().map_or(0, reason),
                ])?;
            }
        }
        Ok(bytes)
    }
}
impl ComputedHistogram {
    pub fn owned_bytes(&self) -> AppResult<usize> {
        checked_sum([
            self.base_bytes()?,
            checked_product(self.bins.capacity(), size_of::<HistogramBin>())?,
            reason(&self.range[0]),
            reason(&self.range[1]),
            self.bin_adjustment.as_ref().map_or(0, String::capacity),
        ])
    }
}
impl ComputedScatter {
    pub fn owned_bytes(&self) -> AppResult<usize> {
        let mut bytes = checked_sum([
            self.base_bytes()?,
            checked_product(self.points.capacity(), size_of::<ScatterPoint>())?,
        ])?;
        for range in &self.ranges {
            bytes = checked_sum([bytes, reason(&range[0]), reason(&range[1])])?;
        }
        Ok(bytes)
    }
}
impl ComputedBoxplot {
    pub fn owned_bytes(&self) -> AppResult<usize> {
        let mut bytes = checked_sum([
            self.base_bytes()?,
            checked_product(self.groups.capacity(), size_of::<BoxplotGroup>())?,
            checked_product(self.points.capacity(), size_of::<BoxplotPoint>())?,
            self.empty_reason.as_ref().map_or(0, String::capacity),
        ])?;
        for group in &self.groups {
            bytes = checked_sum([bytes, group.value.as_ref().map_or(0, text)])?;
            for estimate in [
                &group.mean,
                &group.minimum,
                &group.q1,
                &group.median,
                &group.q3,
                &group.maximum,
                &group.whiskers[0],
                &group.whiskers[1],
            ] {
                bytes = checked_sum([bytes, reason(estimate)])?;
            }
        }
        Ok(bytes)
    }
}
