mod common;
use std::sync::Arc;
use tessera::{analysis::*, data::*, operation::JobControl, project::Project};
fn request() -> DescribeRequest {
    DescribeRequest {
        columns: vec![ColumnId(6)],
        statistics: vec![Statistic::Mean],
        missing_policy: MissingPolicy::PerVariable,
        filter: RowFilter::default(),
        frequency_weight: None,
    }
}
// Catches dropping history on edit, rewriting historical metadata, mutable requests and ID reuse.
#[test]
fn run_history_survives_edits() {
    let original = Project::new("Study", common::sample_dataset()).unwrap();
    let first = original
        .with_run(describe(original.dataset(), &request(), &JobControl::default()).unwrap())
        .unwrap();
    let score = first.dataset().column(ColumnId(6)).unwrap();
    let renamed = score
        .with_metadata(ColumnMetadata {
            name: "Renamed".into(),
            description: "Updated".into(),
            ..score.metadata().clone()
        })
        .unwrap();
    let edited = first
        .with_dataset(first.dataset().replace_column(renamed).unwrap())
        .unwrap();
    let second = edited
        .with_run(describe(edited.dataset(), &request(), &JobControl::default()).unwrap())
        .unwrap();
    assert_eq!(original.runs().len(), 0);
    assert_eq!(first.runs().len(), 1);
    assert_eq!(second.runs().len(), 2);
    assert!(Arc::ptr_eq(&first.runs()[0], &second.runs()[0]));
    assert_eq!(second.runs()[0].id.0, 1);
    assert_eq!(second.runs()[1].id.0, 2);
    assert_eq!(second.runs()[0].analysis.revision(), RevisionId(1));
    assert_eq!(second.runs()[0].analysis.columns()[0].name, "Total score");
    assert_eq!(
        second.runs()[0].analysis.request(),
        tessera::analysis::AnalysisRequestRef::Describe(&request())
    );
    assert_eq!(second.runs()[1].analysis.columns()[0].name, "Renamed");
}
#[test]
fn rejects_stale_run_and_invalid_title() {
    assert!(Project::new(" \n", common::sample_dataset()).is_err());
    let project = Project::new("Study", common::sample_dataset()).unwrap();
    let stale = describe(project.dataset(), &request(), &JobControl::default()).unwrap();
    let edited = project
        .with_dataset(
            project
                .dataset()
                .edit_cell(ColumnId(6), 0, Cell::Number(18.0))
                .unwrap(),
        )
        .unwrap();
    assert!(edited.with_run(stale).is_err());
}
#[test]
fn restored_history_checks_identity_and_result_integrity() {
    let data = common::sample_dataset();
    let analysis = describe(&data, &request(), &JobControl::default()).unwrap();
    let run = Arc::new(tessera::project::AnalysisRun {
        id: tessera::operation::RunId(4),
        analysis: analysis.clone().into(),
    });
    let project = Project::from_parts(
        "Study",
        data.clone(),
        vec![run.clone()],
        tessera::operation::RunId(5),
    )
    .unwrap();
    assert_eq!(project.next_run_id().0, 5);
    assert_eq!(project.runs()[0].as_ref(), run.as_ref());
    assert!(Project::from_parts(
        "Study",
        data.clone(),
        vec![run.clone()],
        tessera::operation::RunId(4)
    )
    .is_err());
    assert!(Project::from_parts(
        "Study",
        data.clone(),
        vec![run.clone(), run],
        tessera::operation::RunId(5)
    )
    .is_err());
    let mut invalid = analysis;
    invalid.rows[0]
        .estimates
        .get_mut(&Statistic::Mean)
        .unwrap()
        .value = Some(f64::NAN);
    assert!(Project::new("Study", data)
        .unwrap()
        .with_run(invalid)
        .is_err());
}
#[test]
fn replacement_dataset_must_advance_revision() {
    let project = Project::new("Study", common::sample_dataset()).unwrap();
    assert!(project.with_dataset(common::sample_dataset()).is_err());
    assert!(project.with_dataset(project.dataset().clone()).is_err());
}
#[test]
fn snapshots_include_filter_weight_rules_labels_and_prior_types() {
    let base = common::sample_dataset();
    let group = base.column(ColumnId(5)).unwrap();
    let group = group
        .with_metadata(ColumnMetadata {
            missing_rules: MissingRules::Text(vec!["Unknown".into()]),
            value_labels: vec![ValueLabel {
                value: Cell::Text("Control".into()),
                label: "Control group".into(),
            }],
            ..group.metadata().clone()
        })
        .unwrap();
    let data = base
        .replace_column(group)
        .unwrap()
        .append_column(Column::numeric(ColumnId(7), "Weight", vec![Some(1.0); 6]).unwrap())
        .unwrap();
    let mut req = request();
    req.frequency_weight = Some(ColumnId(7));
    req.filter.predicates.push(RowPredicate::Text {
        column: ColumnId(5),
        value: "Unknown".into(),
        equal: false,
    });
    let project = Project::new("Study", data).unwrap();
    let first = project
        .with_run(describe(project.dataset(), &req, &JobControl::default()).unwrap())
        .unwrap();
    assert_eq!(
        first.runs()[0]
            .analysis
            .columns()
            .iter()
            .map(|c| c.id)
            .collect::<Vec<_>>(),
        vec![ColumnId(5), ColumnId(6), ColumnId(7)]
    );
    let group = &first.runs()[0].analysis.columns()[0];
    assert_eq!(group.kind, ColumnKind::Text);
    assert_eq!(group.value_labels[0].label, "Control group");
    let edited = first
        .with_dataset(
            first
                .dataset()
                .replace_column(
                    Column::numeric(ColumnId(5), "New group", vec![Some(0.0); 6]).unwrap(),
                )
                .unwrap(),
        )
        .unwrap();
    assert_eq!(
        edited.runs()[0].analysis.columns()[0].kind,
        ColumnKind::Text
    );
    assert_eq!(
        edited.runs()[0].analysis.request(),
        tessera::analysis::AnalysisRequestRef::Describe(&req)
    );
    let restored = Project::from_parts(
        "Study",
        edited.dataset().clone(),
        edited.runs().to_vec(),
        edited.next_run_id(),
    )
    .unwrap();
    assert_eq!(
        restored.runs()[0].analysis.columns()[0].missing_rules,
        MissingRules::Text(vec!["Unknown".into()])
    );
}

#[test]
fn provenance_is_inert_and_survives_edits_runs_and_restoration() {
    let entries = vec![
        "Ffynhonnell Gymraeg α\n".into(),
        "/private/tmp/not-a-file.csv".into(),
        "https://example.invalid/data".into(),
        "<script>executeNothing()</script>".into(),
    ];
    let original = Project::new("Study", common::sample_dataset()).unwrap();
    let annotated = original.with_provenance(entries.clone()).unwrap();
    assert!(original.provenance().is_empty());
    assert_eq!(annotated.provenance(), entries);
    assert!(annotated.owned_bytes().unwrap() > original.owned_bytes().unwrap());
    let first = annotated
        .with_run(describe(annotated.dataset(), &request(), &JobControl::default()).unwrap())
        .unwrap();
    let edited = first
        .with_dataset(
            first
                .dataset()
                .edit_cell(ColumnId(6), 0, Cell::Number(18.0))
                .unwrap(),
        )
        .unwrap();
    let second = edited
        .with_run(describe(edited.dataset(), &request(), &JobControl::default()).unwrap())
        .unwrap();
    assert_eq!(first.provenance(), entries);
    assert_eq!(edited.provenance(), entries);
    assert_eq!(second.provenance(), entries);
    assert!(Arc::ptr_eq(&first.runs()[0], &second.runs()[0]));
    let restored = Project::from_parts(
        second.title(),
        second.dataset().clone(),
        second.runs().to_vec(),
        second.next_run_id(),
    )
    .unwrap();
    assert!(restored.provenance().is_empty());
    let restored = restored.with_provenance(entries.clone()).unwrap();
    assert_eq!(restored.provenance(), entries);
    let cleared = restored.with_provenance(vec![]).unwrap();
    assert!(cleared.provenance().is_empty());
    assert_eq!(restored.provenance(), entries);
    assert_eq!(
        annotated.history_bytes().unwrap(),
        original.history_bytes().unwrap()
    );
}
#[test]
fn provenance_budget_includes_project_metadata_before_copying() {
    let project = Project::new("Study", common::sample_dataset()).unwrap();
    let dataset_bytes = Budget::dataset_bytes(&[project.dataset()], 0).unwrap();
    // Leave room for the budget checker’s transient pointer bookkeeping.
    let bookkeeping_headroom = 256;
    let mut text = String::with_capacity(
        MAX_OWNED_BYTES - dataset_bytes - std::mem::size_of::<String>() - bookkeeping_headroom,
    );
    text.push('x');
    let entries = vec![text];
    let entry_bytes = entries.capacity() * std::mem::size_of::<String>() + entries[0].capacity();
    assert!(Budget::check_datasets(&[project.dataset()], entry_bytes).is_ok());
    assert!(matches!(
        project.with_provenance(entries),
        Err(tessera::error::AppError::SizeLimit { .. })
    ));
    assert!(project.provenance().is_empty());
}

#[test]
fn retained_projects_deduplicate_aliases_and_shared_history_only() {
    let base = Project::new("Study", common::sample_dataset())
        .unwrap()
        .with_provenance(vec!["Source α".into()])
        .unwrap();
    let first = base
        .with_run(describe(base.dataset(), &request(), &JobControl::default()).unwrap())
        .unwrap();
    assert_eq!(Project::retained_owned_bytes(&[]).unwrap(), 0);
    assert_eq!(
        Project::retained_owned_bytes(&[&first, &first]).unwrap(),
        first.owned_bytes().unwrap()
    );
    let copy = first.clone();
    let copy_storage = std::mem::size_of::<Project>()
        + copy.title().len()
        + std::mem::size_of_val(copy.provenance())
        + copy.provenance().iter().map(String::len).sum::<usize>()
        + std::mem::size_of_val(copy.runs());
    assert_eq!(
        Project::retained_owned_bytes(&[&first, &copy, &first]).unwrap(),
        first.owned_bytes().unwrap() + copy_storage
    );
    let distinct_run = Arc::new(first.runs()[0].as_ref().clone());
    assert_eq!(first.runs()[0].as_ref(), distinct_run.as_ref());
    assert!(!Arc::ptr_eq(&first.runs()[0], &distinct_run));
    let distinct = Project::from_parts(
        first.title(),
        first.dataset().clone(),
        vec![distinct_run],
        first.next_run_id(),
    )
    .unwrap()
    .with_provenance(first.provenance().to_vec())
    .unwrap();
    assert_eq!(
        Project::retained_owned_bytes(&[&first, &distinct]).unwrap(),
        first.owned_bytes().unwrap() + distinct.owned_bytes().unwrap()
    );
}

struct RetainedAllocationTracker;
std::thread_local! {
    static MEASURING_RETAINED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static MAX_RETAINED_ALLOCATION: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[global_allocator]
static RETAINED_ALLOCATION_TRACKER: RetainedAllocationTracker = RetainedAllocationTracker;
// Tests observe allocation sizes on their own thread; System owns the memory.
unsafe impl std::alloc::GlobalAlloc for RetainedAllocationTracker {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        record_retained_allocation(layout.size());
        unsafe { std::alloc::GlobalAlloc::alloc(&std::alloc::System, layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::GlobalAlloc::dealloc(&std::alloc::System, pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: std::alloc::Layout, size: usize) -> *mut u8 {
        record_retained_allocation(size);
        unsafe { std::alloc::GlobalAlloc::realloc(&std::alloc::System, pointer, layout, size) }
    }
}
fn record_retained_allocation(bytes: usize) {
    let _ = MEASURING_RETAINED.try_with(|active| {
        if active.get() {
            let _ =
                MAX_RETAINED_ALLOCATION.try_with(|largest| largest.set(largest.get().max(bytes)));
        }
    });
}
#[test]
fn retained_metadata_refuses_before_pointer_table_growth() {
    let mut large = String::with_capacity(MAX_OWNED_BYTES - 4096);
    large.push('x');
    let first = Project::new("Study", common::sample_dataset())
        .unwrap()
        .with_provenance(vec![large])
        .unwrap();
    let mut extra = String::with_capacity(4096);
    extra.push('y');
    let second = Project::new("Study", common::sample_dataset())
        .unwrap()
        .with_provenance(vec![extra])
        .unwrap();
    assert!(first.owned_bytes().unwrap() + second.owned_bytes().unwrap() > MAX_OWNED_BYTES);
    let (accepted, accepted_growth) = measured_retention(&[&first]);
    assert_eq!(accepted.unwrap(), first.owned_bytes().unwrap());
    let (result, rejected_growth) = measured_retention(&[&first, &second]);
    assert!(matches!(
        result,
        Err(tessera::error::AppError::SizeLimit { .. })
    ));
    assert!(rejected_growth<=accepted_growth,"Rejected metadata grew pointer bookkeeping from {accepted_growth} to {rejected_growth} bytes.");
}

fn measured_retention(projects: &[&Project]) -> (tessera::error::AppResult<usize>, usize) {
    MAX_RETAINED_ALLOCATION.with(|largest| largest.set(0));
    MEASURING_RETAINED.with(|active| active.set(true));
    let result = Project::retained_owned_bytes(projects);
    MEASURING_RETAINED.with(|active| active.set(false));
    (result, MAX_RETAINED_ALLOCATION.with(std::cell::Cell::get))
}

#[test]
fn retained_accounting_allocates_nothing_with_near_ceiling_live_datasets() {
    let base = Project::new("Study", common::sample_dataset()).unwrap();
    let base = base
        .with_run(describe(base.dataset(), &request(), &JobControl::default()).unwrap())
        .unwrap();
    let count = 1024;
    let copied =
        std::mem::size_of::<Project>() + base.title().len() + std::mem::size_of_val(base.runs());
    let expected = base.owned_bytes().unwrap() + (count - 1) * copied;
    let projects = (0..count).map(|_| base.clone()).collect::<Vec<_>>();
    drop(base);
    let refs = projects.iter().collect::<Vec<_>>();
    let extra = expected + std::mem::size_of_val(refs.as_slice());
    let bare = Dataset::new(vec![
        Column::numeric(ColumnId(1), "Large", vec![None]).unwrap()
    ])
    .unwrap();
    let bare_bytes = Budget::dataset_bytes(&[&bare, projects[0].dataset()], 0).unwrap();
    let headroom = 512;
    let mut description = String::with_capacity(MAX_OWNED_BYTES - bare_bytes - extra - headroom);
    description.push('x');
    let column = Column::numeric(ColumnId(1), "Large", vec![None]).unwrap();
    let large = Dataset::new(vec![column
        .with_metadata(ColumnMetadata {
            description,
            ..column.metadata().clone()
        })
        .unwrap()])
    .unwrap();
    drop(bare);
    assert!(Budget::check_datasets(&[&large, projects[0].dataset()], extra).is_ok());
    let (result, allocation) = measured_retention(&refs);
    assert_eq!(result.unwrap(), expected);
    assert_eq!(allocation,0,"Retained accounting allocated {allocation} bytes while dataset/project storage left only {headroom} bytes.");
    let (iter_result, iter_allocation) = measured_retention_iter(refs.iter().copied());
    assert_eq!(iter_result.unwrap(), expected);
    assert_eq!(iter_allocation, 0);
}
fn measured_retention_iter<'a>(
    projects: impl Iterator<Item = &'a Project> + Clone,
) -> (tessera::error::AppResult<usize>, usize) {
    MAX_RETAINED_ALLOCATION.with(|largest| largest.set(0));
    MEASURING_RETAINED.with(|active| active.set(true));
    let result = Project::retained_owned_bytes_iter(projects);
    MEASURING_RETAINED.with(|active| active.set(false));
    (result, MAX_RETAINED_ALLOCATION.with(std::cell::Cell::get))
}

#[test]
fn shared_run_lookup_finds_later_sorted_identity_in_either_snapshot_order() {
    let base = Project::new("Study", common::sample_dataset()).unwrap();
    let first = base
        .with_run(describe(base.dataset(), &request(), &JobControl::default()).unwrap())
        .unwrap();
    let two = first
        .with_run(describe(first.dataset(), &request(), &JobControl::default()).unwrap())
        .unwrap();
    let only_later = Project::from_parts(
        two.title(),
        two.dataset().clone(),
        vec![two.runs()[1].clone()],
        two.next_run_id(),
    )
    .unwrap();
    let additional = std::mem::size_of::<Project>()
        + only_later.title().len()
        + std::mem::size_of_val(only_later.runs())
        + 2 * std::mem::size_of::<usize>();
    let expected = two.owned_bytes().unwrap() + additional;
    assert_eq!(
        Project::retained_owned_bytes(&[&two, &only_later]).unwrap(),
        expected
    );
    assert_eq!(
        Project::retained_owned_bytes_iter(
            std::iter::once(&only_later).chain(std::iter::once(&two))
        )
        .unwrap(),
        expected
    );
}

fn measured_project_update(
    action: impl FnOnce() -> tessera::error::AppResult<Project>,
) -> (tessera::error::AppResult<Project>, usize) {
    MAX_RETAINED_ALLOCATION.with(|largest| largest.set(0));
    MEASURING_RETAINED.with(|active| active.set(true));
    let result = action();
    MEASURING_RETAINED.with(|active| active.set(false));
    (result, MAX_RETAINED_ALLOCATION.with(std::cell::Cell::get))
}
#[test]
fn project_updates_include_retained_datasets_and_reject_before_allocating() {
    let initial = Project::new("Study", common::sample_dataset())
        .unwrap()
        .with_provenance(vec!["Original".into()])
        .unwrap();
    let project = initial
        .with_run(describe(initial.dataset(), &request(), &JobControl::default()).unwrap())
        .unwrap();
    let next = project
        .dataset()
        .edit_cell(ColumnId(6), 0, Cell::Number(18.0))
        .unwrap();
    let other = Dataset::new(vec![Column::text(
        ColumnId(1),
        "Retained",
        vec![Some("x".repeat(32768))],
    )
    .unwrap()])
    .unwrap();
    let current =
        Budget::dataset_bytes(&[project.dataset(), &next], project.owned_bytes().unwrap()).unwrap();
    let extra = MAX_OWNED_BYTES - current - 16384;
    let accepted = project
        .with_dataset_with_budget(next.clone(), &[], extra)
        .unwrap();
    assert_eq!(accepted.dataset().revision(), next.revision());
    assert_eq!(accepted.provenance(), project.provenance());
    assert!(Arc::ptr_eq(&accepted.runs()[0], &project.runs()[0]));
    let denied_next = next.clone();
    let (denied, allocation) =
        measured_project_update(|| project.with_dataset_with_budget(denied_next, &[&other], extra));
    assert!(matches!(
        denied,
        Err(tessera::error::AppError::SizeLimit { .. })
    ));
    assert_eq!(allocation, 0);
    let analysis = describe(project.dataset(), &request(), &JobControl::default()).unwrap();
    let accepted = project
        .with_run_with_budget(analysis.clone(), &[], extra)
        .unwrap();
    assert_eq!(accepted.runs().len(), 2);
    assert_eq!(accepted.runs()[1].id, project.next_run_id());
    assert_eq!(accepted.provenance(), project.provenance());
    let (denied, allocation) =
        measured_project_update(|| project.with_run_with_budget(analysis, &[&other], extra));
    assert!(matches!(
        denied,
        Err(tessera::error::AppError::SizeLimit { .. })
    ));
    assert_eq!(allocation, 0);
    let accepted = project
        .with_provenance_with_budget(vec!["Changed".into()], &[], extra)
        .unwrap();
    assert_eq!(accepted.provenance(), ["Changed"]);
    assert!(Arc::ptr_eq(&accepted.runs()[0], &project.runs()[0]));
    let text = vec!["Changed".into()];
    let (denied, allocation) =
        measured_project_update(|| project.with_provenance_with_budget(text, &[&other], extra));
    assert!(matches!(
        denied,
        Err(tessera::error::AppError::SizeLimit { .. })
    ));
    assert_eq!(allocation, 0);
    assert_eq!(project.dataset().revision(), RevisionId(1));
    assert_eq!(project.runs().len(), 1);
    assert_eq!(project.provenance(), ["Original"]);
}
#[test]
fn run_validation_scratch_is_reserved_before_building_indexes() {
    let project = Project::new("Study", common::sample_dataset()).unwrap();
    let analysis = describe(project.dataset(), &request(), &JobControl::default()).unwrap();
    let sample = project.with_run(analysis.clone()).unwrap();
    let ownership = project.owned_bytes().unwrap()
        + sample.history_bytes().unwrap()
        + std::mem::size_of::<Project>()
        + project.title().len();
    let baseline = Budget::dataset_bytes(&[project.dataset()], ownership).unwrap();
    let extra = MAX_OWNED_BYTES - baseline - 256;
    assert!(Budget::check_datasets(&[project.dataset()], ownership + extra).is_ok());
    let (result, allocation) =
        measured_project_update(|| project.with_run_with_budget(analysis, &[], extra));
    assert!(matches!(
        result,
        Err(tessera::error::AppError::SizeLimit { .. })
    ));
    assert_eq!(
        allocation, 0,
        "Validation allocated {allocation} bytes before its scratch fit the live baseline."
    );
}
