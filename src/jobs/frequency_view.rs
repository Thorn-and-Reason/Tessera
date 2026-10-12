//! Worker-prepared metadata positions for immutable frequency tables; no captured text copies.
use super::*;
use crate::analysis::{exploration::ordering::sort_indices, ComputedFrequencies};
use crate::project::AnalysisRun;
use std::sync::Weak;
// The final cache and both sort vectors fit inside the existing UI reservation.
const VIEW_BYTES: usize = 4 * 1024 * 1024;
#[derive(Debug)]
pub struct FrequencyMetadata {
    owner: Weak<AnalysisRun>,
    pub table_columns: Vec<usize>,
}
impl FrequencyMetadata {
    pub fn matches(&self, run: &Arc<AnalysisRun>) -> bool {
        self.owner
            .upgrade()
            .is_some_and(|owner| Arc::ptr_eq(&owner, run))
    }
    pub fn owned_bytes(&self) -> AppResult<usize> {
        add(
            size_of::<Self>() + 2 * size_of::<usize>(),
            self.table_columns.capacity() * size_of::<usize>(),
        )
    }
    /// Live run blocks are already charged through retained projects. An expired Weak still owns its block.
    pub fn orphaned_owner_bytes(&self) -> usize {
        if self.owner.strong_count() == 0 {
            size_of::<AnalysisRun>() + 2 * size_of::<usize>()
        } else {
            0
        }
    }
}
#[derive(Debug)]
pub struct FrequencyRowsOutput {
    pub rows: FrequencySearchResult,
    pub metadata: Option<Arc<FrequencyMetadata>>,
}
impl From<FrequencySearchResult> for FrequencyRowsOutput {
    fn from(rows: FrequencySearchResult) -> Self {
        Self {
            rows,
            metadata: None,
        }
    }
}
pub(super) fn prepare(
    run: &Arc<AnalysisRun>,
    control: &JobControl,
    datasets: &[&Dataset],
    extra: usize,
) -> AppResult<Arc<FrequencyMetadata>> {
    let CompletedAnalysis::Frequencies(v) = &run.analysis else {
        return Err(AppError::invalid(
            "frequency search",
            "Select a frequency run.",
        ));
    };
    prepare_tables(run, v, control, datasets, extra)
}
fn prepare_tables(
    run: &Arc<AnalysisRun>,
    v: &ComputedFrequencies,
    control: &JobControl,
    datasets: &[&Dataset],
    extra: usize,
) -> AppResult<Arc<FrequencyMetadata>> {
    let n = v.tables.len();
    let bytes = n
        .checked_mul(3 * size_of::<usize>())
        .and_then(|b| b.checked_add(size_of::<FrequencyMetadata>() + 2 * size_of::<usize>()))
        .ok_or(AppError::SizeLimit { limit: VIEW_BYTES })?;
    // Leave 1 MiB for the draft and bounded query/presentation controls.
    if bytes > VIEW_BYTES - 1024 * 1024 {
        return Err(AppError::SizeLimit { limit: VIEW_BYTES });
    }
    Budget::check_datasets(datasets, add(extra, bytes)?)?;
    control.check_cancelled()?;
    let mut order = Vec::with_capacity(n);
    for i in 0..n {
        if i.is_multiple_of(4096) {
            control.checkpoint(i, None)?;
        }
        order.push(i);
    }
    let mut scratch = vec![0; n];
    sort_indices(&mut order, &mut scratch, control, |a, b| {
        Ok(v.tables[a].column.cmp(&v.tables[b].column))
    })?;
    let mut table_columns = vec![usize::MAX; n];
    for (index, c) in v.columns.iter().enumerate() {
        if index.is_multiple_of(4096) {
            control.checkpoint(index, Some(v.columns.len()))?;
        }
        if let Ok(found) = order.binary_search_by_key(&c.id, |i| v.tables[*i].column) {
            table_columns[order[found]] = index;
        }
    }
    for (i, column) in table_columns.iter().enumerate() {
        if i.is_multiple_of(4096) {
            control.check_cancelled()?;
        }
        if *column == usize::MAX {
            return Err(AppError::invalid(
                "frequency search",
                "Captured variable metadata is unavailable.",
            ));
        }
    }
    control.check_cancelled()?;
    Ok(Arc::new(FrequencyMetadata {
        owner: Arc::downgrade(run),
        table_columns,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{analyze_with_budget, AnalysisRequest, FrequenciesRequest, RowFilter};
    use crate::data::{Column, ColumnId};
    fn project() -> Arc<Project> {
        let data = Dataset::new(vec![
            Column::text(ColumnId(1), "First", vec![Some("A".into())]).unwrap(),
            Column::numeric(ColumnId(2), "Second", vec![Some(2.)]).unwrap(),
        ])
        .unwrap();
        let mut result = analyze_with_budget(
            &data,
            &AnalysisRequest::Frequencies(FrequenciesRequest {
                group: None,
                columns: vec![ColumnId(1), ColumnId(2)],
                filter: RowFilter::default(),
                frequency_weight: None,
            }),
            &JobControl::default(),
            &[],
            4096,
        )
        .unwrap();
        let CompletedAnalysis::Frequencies(v) = &mut result else {
            panic!()
        };
        v.columns.reverse();
        Arc::new(
            Project::new("Metadata", data)
                .unwrap()
                .with_run(result)
                .unwrap(),
        )
    }
    #[test]
    fn metadata_positions_preserve_arbitrary_order_and_reuse_only_same_immutable_run() {
        let p = project();
        let run = &p.runs()[0];
        let cache = prepare(run, &JobControl::default(), &[p.dataset()], 4096).unwrap();
        assert_eq!(cache.table_columns, vec![1, 0]);
        assert!(cache.matches(run));
        let other = project();
        assert!(!cache.matches(&other.runs()[0]));
        let task = JobTask {
            operation: JobOperation::FrequencySearch {
                project: Arc::clone(&p),
                request: FrequencySearchRequest {
                    run: run.id,
                    column: ColumnId(1),
                    query: "A".into(),
                    offset: 0,
                },
            },
            retained: RetainedWork {
                frequency_metadata: vec![Arc::clone(&cache), Arc::clone(&cache)],
                prepare_frequency_metadata: true,
                ..Default::default()
            },
        };
        let JobOperation::FrequencySearch { request, .. } = &task.operation else {
            panic!()
        };
        let baseline = JobTask::new(JobOperation::FrequencySearch {
            project: Arc::clone(&p),
            request: request.clone(),
        })
        .preflight()
        .unwrap();
        assert_eq!(
            task.preflight().unwrap() - baseline,
            2 * size_of::<usize>() + cache.owned_bytes().unwrap(),
            "shared cache storage is charged once plus two retained reference slots"
        );
        let JobOutput::FrequencyRows(output) = task.execute(&JobControl::default()).unwrap() else {
            panic!()
        };
        assert!(Arc::ptr_eq(output.metadata.as_ref().unwrap(), &cache));
        assert_eq!(output.rows.indices, vec![0]);
        drop(output);
        drop(task);
        drop(p);
        assert!(cache.owner.upgrade().is_none());
        assert!(cache.orphaned_owner_bytes() > 0);
    }
    #[test]
    fn cancelled_and_beyond_reserve_preparation_never_adopts_a_partial_index() {
        let p = project();
        let control = JobControl::default();
        control.cancel();
        assert!(matches!(
            prepare(&p.runs()[0], &control, &[p.dataset()], 4096),
            Err(AppError::Cancelled)
        ));
        let mut run = p.runs()[0].as_ref().clone();
        let CompletedAnalysis::Frequencies(v) = &mut run.analysis else {
            panic!()
        };
        let table = v.tables[0].clone();
        v.tables = vec![table; 160_000];
        let run = Arc::new(run);
        let control = JobControl::default();
        let (result, largest) =
            crate::data::measure_test_allocations(|| prepare(&run, &control, &[p.dataset()], 4096));
        assert!(matches!(
            result,
            Err(AppError::SizeLimit { limit: VIEW_BYTES })
        ));
        assert_eq!(largest, 0);
    }
}

#[cfg(test)]
mod headroom_tests {
    use super::*;
    use crate::{
        analysis::{analyze_with_budget, AnalysisRequest, FrequenciesRequest, RowFilter},
        data::{Column, ColumnId},
    };
    fn task(aliases: bool, headroom: usize) -> JobTask {
        let data = Dataset::new(
            (1..=128)
                .map(|id| {
                    Column::numeric(ColumnId(id), &format!("C{id}"), vec![Some(id as f64)]).unwrap()
                })
                .collect(),
        )
        .unwrap();
        let result = analyze_with_budget(
            &data,
            &AnalysisRequest::Frequencies(FrequenciesRequest {
                group: None,
                columns: (1..=128).map(ColumnId).collect(),
                filter: RowFilter::default(),
                frequency_weight: None,
            }),
            &JobControl::default(),
            &[],
            4096,
        )
        .unwrap();
        let current = Dataset::new(vec![Column::text(
            ColumnId(1),
            "Current",
            vec![Some("x".repeat(2 * 1024 * 1024))],
        )
        .unwrap()])
        .unwrap()
        .append_row_with_budget(&[], 0)
        .unwrap();
        let p = Arc::new(
            Project::new("Headroom", data)
                .unwrap()
                .with_run(result)
                .unwrap()
                .with_dataset(current)
                .unwrap(),
        );
        let mut task = JobTask {
            operation: JobOperation::FrequencySearch {
                project: Arc::clone(&p),
                request: FrequencySearchRequest {
                    run: RunId(1),
                    column: ColumnId(1),
                    query: String::new(),
                    offset: 0,
                },
            },
            retained: RetainedWork {
                projects: if aliases {
                    vec![Arc::clone(&p), Arc::clone(&p)]
                } else {
                    vec![]
                },
                prepare_frequency_metadata: true,
                ..Default::default()
            },
        };
        let extra = task.preflight().unwrap();
        let complete = Budget::dataset_bytes_iter(task.datasets(), extra).unwrap();
        task.retained.extra_bytes = MAX_OWNED_BYTES - complete - headroom;
        task.preflight().unwrap();
        task
    }
    #[test]
    fn review_fix_public_metadata_opt_in_precharges_all_live_datasets_before_vectors() {
        for aliases in [false, true] {
            let task = task(aliases, 512);
            let control = JobControl::default();
            let (result, largest) =
                crate::data::measure_test_allocations(|| task.execute(&control));
            assert!(matches!(
                result,
                Err(AppError::SizeLimit {
                    limit: MAX_OWNED_BYTES
                })
            ));
            assert!(
                largest < 128 * size_of::<usize>(),
                "metadata vectors allocated before combined refusal: {largest}"
            );
            assert!(task.retained.frequency_metadata.is_empty());
        }
    }
    #[test]
    fn review_fix_alias_headroom_and_cancellation_preserve_cache_adoption_boundary() {
        for aliases in [false, true] {
            let task = task(aliases, 1024 * 1024);
            let output = task.execute(&JobControl::default()).unwrap();
            let JobOutput::FrequencyRows(output) = output else {
                panic!()
            };
            assert_eq!(output.metadata.unwrap().table_columns.len(), 128);
            assert_eq!(output.rows.indices, vec![0]);
            let control = JobControl::default();
            control.cancel();
            let (result, largest) =
                crate::data::measure_test_allocations(|| task.execute(&control));
            assert!(matches!(result, Err(AppError::Cancelled)));
            assert!(largest < 128 * size_of::<usize>());
            assert!(task.retained.frequency_metadata.is_empty());
        }
    }
}
