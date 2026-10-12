mod excel;
use super::*;
use crate::analysis::DescribeRequest;
use crate::{
    analysis::{MissingPolicy, RowFilter, Statistic},
    data::Column,
    operation::CHECKPOINT_HOOK,
};
use std::{
    sync::mpsc::{sync_channel, SyncSender},
    time::{Duration, Instant},
};
fn token() -> JobToken {
    JobToken {
        document: 1,
        revision: RevisionId(1),
        generation: 1,
    }
}
fn project() -> Arc<Project> {
    Arc::new(
        Project::new(
            "Worker",
            Dataset::new(vec![Column::numeric(
                ColumnId(1),
                "N",
                vec![Some(2.0); 10_000],
            )
            .unwrap()])
            .unwrap(),
        )
        .unwrap(),
    )
}
fn complete(runner: &mut JobRunner) -> JobEvent {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        for event in runner.poll() {
            if matches!(event, JobEvent::Completed { .. }) {
                return event;
            }
        }
        assert!(Instant::now() < deadline, "worker did not complete");
        thread::yield_now();
    }
}
fn receive<T>(receiver: &Receiver<T>) -> T {
    receiver.recv_timeout(Duration::from_secs(10)).unwrap()
}
fn hold_checkpoint(entered: SyncSender<()>, resume: Receiver<()>) {
    CHECKPOINT_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |progress| {
            if progress.done > 0 {
                entered.send(()).unwrap();
                receive(&resume);
            }
        }))
    });
}
#[test]
fn single_operation_and_progress() {
    let (entered, reached) = sync_channel(0);
    let (resume, release) = sync_channel(0);
    let mut runner = JobRunner::default();
    runner
        .start_work(token(), move |control| {
            for done in 0..100_000 {
                control.checkpoint(done, Some(100_000))?;
            }
            entered.send(()).unwrap();
            receive(&release);
            Err(AppError::Cancelled)
        })
        .unwrap();
    receive(&reached);
    let active = runner.active.as_ref().unwrap();
    assert_eq!(active.latest.lock().unwrap().unwrap().done, 99_999);
    assert!(runner
        .start(
            JobTask::new(JobOperation::Open {
                path: "unused".into()
            }),
            token()
        )
        .is_err());
    let events = runner.poll();
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        JobEvent::Progress {
            progress: Progress { done: 99_999, .. },
            ..
        }
    ));
    assert!(runner.poll().is_empty());
    runner.cancel();
    assert!(matches!(
        runner.poll().as_slice(),
        [JobEvent::CancellationRequested { .. }]
    ));
    assert!(runner.is_running());
    assert!(runner
        .start(
            JobTask::new(JobOperation::Open {
                path: "unused".into()
            }),
            token()
        )
        .is_err());
    resume.send(()).unwrap();
    assert!(matches!(
        complete(&mut runner),
        JobEvent::Completed {
            result: Err(AppError::Cancelled),
            ..
        }
    ));
}
#[test]
fn cancellation_before_domain_work_never_loads_the_path() {
    let (entered, reached) = sync_channel(0);
    let (resume, release) = sync_channel(0);
    let mut runner = JobRunner::default();
    runner
        .start_work(token(), move |control| {
            entered.send(()).unwrap();
            receive(&release);
            JobTask::new(JobOperation::PreviewPath {
                path: "absent-before-cancel.csv".into(),
                delimiter: Delimiter::Comma,
            })
            .execute(control)
        })
        .unwrap();
    receive(&reached);
    runner.cancel();
    resume.send(()).unwrap();
    assert!(matches!(
        complete(&mut runner),
        JobEvent::Completed {
            result: Err(AppError::Cancelled),
            ..
        }
    ));
}
#[test]
fn cancellation_during_real_parsing_calculation_and_conversion() {
    for operation in ["parsing", "calculation", "conversion"] {
        let (entered, reached) = sync_channel(0);
        let (resume, release) = sync_channel(0);
        let mut runner = JobRunner::default();
        runner
            .start_work(token(), move |control| {
                hold_checkpoint(entered, release);
                if operation == "parsing" {
                    let bytes: Arc<[u8]> =
                        Arc::from(format!("N\n{}", "2\n".repeat(10_000)).into_bytes());
                    import::preview(bytes, Delimiter::Comma, control)
                        .map(|p| JobOutput::Preview(Arc::new(p)))
                } else if operation == "calculation" {
                    let request = DescribeRequest {
                        columns: vec![ColumnId(1)],
                        statistics: vec![Statistic::Mean],
                        missing_policy: MissingPolicy::PerVariable,
                        filter: RowFilter::default(),
                        frequency_weight: None,
                    };
                    JobTask::new(JobOperation::Analyze {
                        project: project(),
                        request: request.into(),
                    })
                    .execute(control)
                } else {
                    JobTask::new(JobOperation::ConvertColumn {
                        project: project(),
                        id: ColumnId(1),
                        kind: ColumnKind::Text,
                    })
                    .execute(control)
                }
            })
            .unwrap();
        receive(&reached);
        runner.cancel();
        assert!(runner
            .poll()
            .iter()
            .any(|e| matches!(e, JobEvent::CancellationRequested { .. })));
        assert!(runner.is_running());
        resume.send(()).unwrap();
        assert!(matches!(
            complete(&mut runner),
            JobEvent::Completed {
                cancelled: true,
                result: Err(AppError::Cancelled),
                ..
            }
        ));
    }
}
#[test]
fn worker_panic_is_visible_and_slot_can_be_reused() {
    let mut runner = JobRunner::default();
    runner
        .start_work(token(), |_| panic!("test worker failure"))
        .unwrap();
    assert!(matches!(
        complete(&mut runner),
        JobEvent::Completed {
            result: Err(AppError::InvalidInput { .. }),
            ..
        }
    ));
    runner
        .start_work(token(), |_| Err(AppError::Cancelled))
        .unwrap();
    assert!(matches!(complete(&mut runner), JobEvent::Completed { .. }));
}
#[test]
fn completed_file_survives_cancel_new_document_and_responsive_shutdown() {
    for publication in [
        Publication::Committed,
        Publication::Conflict,
        Publication::Uncertain,
    ] {
        let (entered, reached) = sync_channel(0);
        let (resume, release) = sync_channel(0);
        let directory = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "tessera-worker-save-{}-{publication:?}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("document.tessera");
        let destination = path.clone();
        let mut runner = JobRunner::default();
        runner
            .start_work(token(), move |control| {
                let JobOutput::Saved(mut outcome) = JobTask::new(JobOperation::Save {
                    project: project(),
                    path,
                    expected: None,
                })
                .execute(control)?
                else {
                    unreachable!()
                };
                // The real file is already published. Reconciliation statuses must not erase that fact.
                outcome.publication = publication;
                entered.send(()).unwrap();
                receive(&release);
                Ok(JobOutput::Saved(outcome))
            })
            .unwrap();
        receive(&reached);
        assert!(destination.exists());
        runner.begin_shutdown();
        assert!(!runner.is_shutdown_complete());
        assert!(runner
            .poll()
            .iter()
            .any(|e| matches!(e, JobEvent::CancellationRequested { .. })));
        assert!(!runner.is_shutdown_complete());
        resume.send(()).unwrap();
        let event = complete(&mut runner);
        assert!(event.published_file().is_some());
        assert!(!event.may_publish(2, RevisionId(1), 1));
        assert!(!event.confirms_current_save(1, RevisionId(1), 1));
        assert!(runner.is_shutdown_complete());
        assert!(runner
            .start_work(token(), |_| Err(AppError::Cancelled))
            .is_err());
        assert!(files::read_project(&destination, &JobControl::default()).is_ok());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
#[test]
fn shutdown_before_publication_preserves_an_existing_destination() {
    let (entered, reached) = sync_channel(0);
    let (resume, release) = sync_channel(0);
    let directory = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("tessera-worker-cancel-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("document.tessera");
    std::fs::write(&path, b"existing complete document").unwrap();
    let destination = path.clone();
    let mut runner = JobRunner::default();
    runner
        .start_work(token(), move |control| {
            let expected = files::stamp(&path)?;
            entered.send(()).unwrap();
            receive(&release);
            JobTask::new(JobOperation::Save {
                project: project(),
                path,
                expected: Some(expected),
            })
            .execute(control)
        })
        .unwrap();
    receive(&reached);
    runner.begin_shutdown();
    runner.poll();
    resume.send(()).unwrap();
    assert!(matches!(
        complete(&mut runner),
        JobEvent::Completed {
            result: Err(AppError::Cancelled),
            ..
        }
    ));
    assert_eq!(
        std::fs::read(destination).unwrap(),
        b"existing complete document"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn cancel_after_success_before_delivery_discards_workspace_result() {
    let (entered, reached) = sync_channel(0);
    let (resume, release) = sync_channel(0);
    let mut runner = JobRunner::default();
    runner
        .start_work(token(), move |control| {
            let preview = import::preview(Arc::from(&b"N\n2\n"[..]), Delimiter::Comma, control)?;
            entered.send(()).unwrap();
            receive(&release);
            Ok(JobOutput::Preview(Arc::new(preview)))
        })
        .unwrap();
    receive(&reached);
    runner.cancel();
    resume.send(()).unwrap();
    assert!(matches!(
        complete(&mut runner),
        JobEvent::Completed {
            cancelled: true,
            result: Err(AppError::Cancelled),
            ..
        }
    ));
}

#[test]
fn changing_import_size_is_detected_during_worker_read() {
    for grow in [true, false] {
        let directory = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "tessera-worker-input-{}-{grow}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("source.csv");
        std::fs::write(&path, b"N\n1\n").unwrap();
        let source = path.clone();
        let (entered, reached) = sync_channel(0);
        let (resume, release) = sync_channel(0);
        let mut runner = JobRunner::default();
        runner
            .start_work(token(), move |control| {
                let held = std::cell::Cell::new(false);
                CHECKPOINT_HOOK.with(|hook| {
                    *hook.borrow_mut() = Some(Box::new(move |progress| {
                        if progress.total == Some(4) && !held.replace(true) {
                            entered.send(()).unwrap();
                            receive(&release);
                        }
                    }))
                });
                JobTask::new(JobOperation::PreviewPath {
                    path,
                    delimiter: Delimiter::Comma,
                })
                .execute(control)
            })
            .unwrap();
        receive(&reached);
        std::fs::write(&source, if grow { &b"N\n1\n2\n"[..] } else { &b"N\n"[..] }).unwrap();
        resume.send(()).unwrap();
        assert!(matches!(
            complete(&mut runner),
            JobEvent::Completed {
                result: Err(AppError::Conflict(_)),
                ..
            }
        ));
        std::fs::remove_dir_all(directory).unwrap();
    }
}

fn near_limit_task(distinct_datasets: bool) -> JobTask {
    let base = Project::new(
        "Budget",
        Dataset::new(vec![
            Column::numeric(ColumnId(1), "N", vec![Some(2.0)]).unwrap()
        ])
        .unwrap(),
    )
    .unwrap();
    let request = DescribeRequest {
        columns: vec![ColumnId(1)],
        statistics: vec![Statistic::Mean],
        missing_policy: MissingPolicy::PerVariable,
        filter: RowFilter::default(),
        frequency_weight: None,
    };
    let analysis = analysis::describe(base.dataset(), &request, &JobControl::default()).unwrap();
    let projects: Vec<_> = (0..2048)
        .map(|_| {
            let project = if distinct_datasets {
                Project::new("Budget", base.dataset().clone()).unwrap()
            } else {
                base.clone()
            };
            Arc::new(project.with_run(analysis.clone()).unwrap())
        })
        .collect();
    let mut text = String::with_capacity(32 * 1024 * 1024);
    text.push('x');
    let large = Arc::new(
        Dataset::new(vec![Column::text(
            ColumnId(1),
            "Retained",
            vec![Some(text)],
        )
        .unwrap()])
        .unwrap(),
    );
    let project_refs: Vec<_> = projects.iter().map(Arc::as_ref).collect();
    let datasets: Vec<_> = projects
        .iter()
        .map(|p| p.dataset())
        .chain(std::iter::once(large.as_ref()))
        .collect();
    let metadata = Project::retained_owned_bytes(&project_refs).unwrap();
    let owned = Budget::dataset_bytes(&datasets, metadata).unwrap();
    let mut task = JobTask::new(JobOperation::Open {
        path: "not-opened.tessera".into(),
    });
    task.retained.projects = projects;
    task.retained.datasets = vec![large];
    let input_refs = (task.retained.projects.capacity() + task.retained.datasets.capacity())
        * size_of::<usize>();
    task.retained.extra_bytes = MAX_OWNED_BYTES - owned - input_refs - 1024;
    task
}
#[test]
fn complete_live_baseline_refuses_before_job_reference_growth() {
    let task = near_limit_task(false);
    let (result, largest) =
        crate::data::measure_test_allocations(|| task.execute(&JobControl::default()));
    assert!(matches!(result, Err(AppError::SizeLimit { .. })));
    assert!(
        largest <= 128,
        "job bookkeeping allocated {largest} bytes before refusing the complete live baseline"
    );
}
#[test]
fn complete_live_baseline_refuses_before_worker_launch() {
    let task = near_limit_task(false);
    let mut runner = JobRunner::default();
    let (result, largest) = crate::data::measure_test_allocations(|| runner.start(task, token()));
    assert!(
        matches!(result, Err(AppError::SizeLimit { .. })),
        "over-budget work launched a worker"
    );
    assert!(!runner.is_running());
    assert!(
        largest <= 128,
        "worker launch allocated {largest} bytes before refusing the complete live baseline"
    );
}

#[test]
fn complete_live_baseline_includes_dataset_storage_after_many_identities() {
    let task = near_limit_task(true);
    let (result, largest) =
        crate::data::measure_test_allocations(|| task.execute(&JobControl::default()));
    assert!(matches!(result, Err(AppError::SizeLimit { .. })));
    assert!(
        largest <= 128,
        "dataset bookkeeping allocated {largest} bytes before counting the later retained dataset"
    );
}

#[test]
fn every_analysis_variant_dispatches_and_lookup_storage_is_counted_once() {
    let control = JobControl::default();
    let p = Arc::new(
        crate::project_io::decode(
            &include_bytes!("../../examples/mixed-analysis.tessera")[..],
            &control,
        )
        .unwrap(),
    );
    for run in p.runs() {
        let request = run.analysis.request().clone_owned();
        let request_bytes = request.heap_bytes().unwrap();
        // A historical Mac capture is not a bitwise oracle for another OS's
        // transcendental library. Compare the worker with the direct engine on
        // this platform; independent numerical fixtures test mathematical fidelity.
        let expected =
            analysis::analyze_with_budget(p.dataset(), &request, &control, &[], request_bytes)
                .unwrap();
        let task = JobTask::new(JobOperation::Analyze {
            project: Arc::clone(&p),
            request,
        });
        assert_eq!(task.operation.pending_bytes().unwrap(), request_bytes);
        let JobOutput::Analyzed(result) = task.execute(&control).unwrap() else {
            panic!("expected analysis")
        };
        assert_eq!(result, expected);
        assert_eq!(result.request(), run.analysis.request());
    }
    let request = analysis::GroupLookupRequest {
        column: ColumnId(3),
        role_epoch: 7,
        query: "absent search".into(),
        selected: [
            Some(crate::data::Cell::Text("<A>".into())),
            Some(crate::data::Cell::Text("B&".into())),
        ],
    };
    let bytes = request.heap_bytes().unwrap();
    let task = JobTask::new(JobOperation::GroupLookup {
        project: Arc::clone(&p),
        request,
    });
    assert_eq!(task.operation.pending_bytes().unwrap(), bytes);
    let JobOutput::GroupValues(result) = task.execute(&control).unwrap() else {
        panic!("expected groups")
    };
    assert_eq!(result.column, ColumnId(3));
    assert_eq!(result.role_epoch, 7);
    assert_eq!(result.query, "absent search");
    assert_eq!(result.selected_present, [true, true]);
    let mut retained = JobTask::new(JobOperation::Analyze {
        project: Arc::clone(&p),
        request: p.runs()[0].analysis.request().clone_owned(),
    });
    let before = retained.preflight().unwrap();
    retained.retained.group_values = vec![Arc::clone(&result)];
    let once = retained.preflight().unwrap();
    assert_eq!(
        once - before,
        result.owned_bytes().unwrap() + 3 * size_of::<usize>()
    );
    retained.retained.group_values = vec![Arc::clone(&result), Arc::clone(&result)];
    assert_eq!(retained.preflight().unwrap() - once, size_of::<usize>());
    retained.retained.extra_bytes = MAX_OWNED_BYTES - once + result.owned_bytes().unwrap() / 2;
    assert!(matches!(
        retained.preflight(),
        Err(AppError::SizeLimit { .. })
    ));
}

#[test]
fn exploration_jobs_and_search_charge_retained_results_and_terminal_storage() {
    let control = JobControl::default();
    let mut project = crate::project_io::decode(
        &include_bytes!("../../examples/exploration.tessera")[..],
        &control,
    )
    .unwrap();
    for index in 5..9 {
        let request = project.runs()[index].analysis.request().clone_owned();
        let task = JobTask::new(JobOperation::Analyze {
            project: Arc::new(project.clone()),
            request,
        });
        let JobOutput::Analyzed(result) = task.execute(&control).unwrap() else {
            panic!()
        };
        let mut expected = project.runs()[index].analysis.clone();
        if let (
            crate::analysis::CompletedAnalysis::Boxplot(actual),
            crate::analysis::CompletedAnalysis::Boxplot(legacy),
        ) = (&result, &mut expected)
        {
            assert_eq!(
                actual
                    .groups
                    .iter()
                    .map(|g| g.mean.value)
                    .collect::<Vec<_>>(),
                vec![Some(1.5), Some(4.5)]
            );
            for (old, new) in legacy.groups.iter_mut().zip(&actual.groups) {
                assert!(old.mean.value.is_none());
                old.mean = new.mean.clone();
            }
        }
        assert_eq!(result, expected);
        project = project.with_run(result).unwrap();
    }
    let request = crate::analysis::FrequencySearchRequest {
        run: RunId(6),
        column: ColumnId(1),
        query: String::new(),
        offset: 0,
    };
    let task = JobTask::new(JobOperation::FrequencySearch {
        project: Arc::new(project.clone()),
        request,
    });
    let JobOutput::FrequencyRows(result) = task.execute(&control).unwrap() else {
        panic!()
    };
    assert!(result.rows.matched > 0);
    let retained = Arc::new(result.rows);
    let mut task = JobTask::new(JobOperation::Analyze {
        project: Arc::new(project.clone()),
        request: project.runs()[6].analysis.request().clone_owned(),
    });
    let baseline = task.preflight().unwrap();
    task.retained.frequency_rows = vec![retained.clone(), retained.clone()];
    let extra = task.preflight().unwrap();
    assert_eq!(
        extra - baseline,
        2 * size_of::<usize>() + retained.owned_bytes().unwrap() + 2 * size_of::<usize>()
    );
    task.retained.extra_bytes = MAX_OWNED_BYTES - JOB_WORK_BYTES;
    assert!(matches!(
        task.execute(&control),
        Err(AppError::SizeLimit { .. })
    ));
    assert_eq!(project.runs().len(), 13);
}

#[test]
fn reliability_worker_precharge_forwards_supplied_control_before_request_capacity_refusal() {
    use crate::{
        analysis::{ReliabilityRequest, RowFilter, RowPredicate},
        operation::CHECKPOINT_HOOK,
    };
    let dataset = Dataset::new(vec![
        Column::numeric(ColumnId(1), "First", vec![Some(1.), Some(2.)]).unwrap(),
        Column::numeric(ColumnId(2), "Second", vec![Some(2.), Some(4.)]).unwrap(),
    ])
    .unwrap();
    let request = AnalysisRequest::Reliability(ReliabilityRequest {
        items: vec![ColumnId(1), ColumnId(2)],
        filter: RowFilter {
            predicates: vec![RowPredicate::Text {
                column: ColumnId(1),
                value: String::with_capacity(crate::data::MAX_OWNED_BYTES),
                equal: true,
            }],
        },
        frequency_weight: None,
        reversals: vec![],
    });
    let task = JobTask::new(JobOperation::Analyze {
        project: Arc::new(Project::new("Worker", dataset).unwrap()),
        request,
    });
    let control = JobControl::default();
    let cancel = control.clone();
    let calls = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let observed = std::rc::Rc::clone(&calls);
    CHECKPOINT_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |_| {
            observed.set(observed.get() + 1);
            if observed.get() > 1 {
                cancel.cancel();
            }
        }))
    });
    let result = task.execute_preflighted(&control);
    CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
    assert!(matches!(result, Err(AppError::Cancelled)), "{result:?}");
    assert!(calls.get() > 1);
}

mod survey;
