//! Independent-process save/load and worker cancellation/shutdown audit.
use std::{
    fs::File,
    io::Read,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use tessera::{
    analysis::{AnalysisRequest, CorrelationMissingPolicy, PearsonRequest, RowFilter},
    data::{Budget, ColumnKind},
    error::{AppError, AppResult},
    files::{self, Publication},
    import::{commit_preview, preview, Delimiter, MAX_INPUT_BYTES},
    jobs::{JobEvent, JobOperation, JobRunner, JobTask},
    operation::{JobControl, JobToken},
    project::Project,
    project_io::decode,
};

fn elapsed(name: &str, start: Instant) {
    eprintln!(
        "stage_{name}_ms={:.6}",
        start.elapsed().as_secs_f64() * 1000.0
    );
}
fn load(path: &std::path::Path, control: &JobControl) -> AppResult<Project> {
    if path.extension().is_some_and(|e| e == "tessera") {
        return decode(File::open(path)?, control);
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(AppError::SizeLimit {
            limit: MAX_INPUT_BYTES,
        });
    }
    let input = preview(Arc::from(bytes), Delimiter::Comma, control)?;
    let choices = input.default_choices()?;
    Project::new(
        "Lifecycle performance fixture",
        commit_preview(&input, &choices, control)?,
    )
}
fn worker(project: Arc<Project>, shutdown: bool) -> AppResult<()> {
    let revision = project.dataset().revision();
    let history = project.runs().len();
    let columns: Vec<_> = project
        .dataset()
        .columns()
        .iter()
        .filter(|c| c.kind() == ColumnKind::Number)
        .map(|c| c.id())
        .collect();
    if columns.len() < 2 {
        return Err(AppError::invalid(
            "fixture",
            "At least two numeric variables required.",
        ));
    }
    let task = JobTask::new(JobOperation::Analyze {
        project: project.clone(),
        request: AnalysisRequest::Pearson(PearsonRequest {
            columns,
            missing_policy: CorrelationMissingPolicy::PairwiseComplete,
            filter: RowFilter::default(),
            frequency_weight: None,
        }),
    });
    let token = JobToken {
        document: 1,
        generation: 1,
        revision,
    };
    let mut runner = JobRunner::default();
    let start = Instant::now();
    runner.start(task, token)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut requested = None;
    let mut terminal = false;
    while runner.is_running() {
        let events = runner.poll();
        let completed = events
            .iter()
            .any(|e| matches!(e, JobEvent::Completed { .. }));
        let progress = events
            .iter()
            .any(|e| matches!(e, JobEvent::Progress { progress, .. } if progress.done > 0));
        if requested.is_none() && progress && !completed {
            elapsed("worker_to_progress", start);
            let now = Instant::now();
            if shutdown {
                runner.begin_shutdown();
            } else {
                runner.cancel();
            }
            requested = Some(now);
        }
        for event in events {
            if let JobEvent::Completed {
                cancelled, result, ..
            } = event
            {
                terminal = true;
                if let Some(when) = requested {
                    assert!(cancelled, "Requested cancellation was not acknowledged");
                    assert!(
                        matches!(result, Err(AppError::Cancelled)),
                        "Cancelled analysis published a result or failed unexpectedly"
                    );
                    elapsed(
                        if shutdown {
                            "shutdown_to_finished"
                        } else {
                            "cancel_to_finished"
                        },
                        when,
                    );
                    eprintln!("lifecycle_status=cancelled_after_progress");
                } else {
                    result?;
                    eprintln!("lifecycle_status=completed_before_observable_progress");
                    elapsed("worker_completed", start);
                }
            }
        }
        if Instant::now() >= deadline && runner.is_running() {
            runner.begin_shutdown();
            return Err(AppError::invalid(
                "timeout",
                "Worker did not finish within 30 seconds.",
            ));
        }
        if runner.is_running() {
            thread::sleep(Duration::from_millis(1));
        }
    }
    assert!(terminal, "Worker stopped without a terminal event");
    if shutdown && requested.is_some() {
        assert!(runner.is_shutdown_complete());
    }
    assert_eq!(project.dataset().revision(), revision);
    assert_eq!(project.runs().len(), history);
    Ok(())
}
fn save_load(project: &Project, control: &JobControl) -> AppResult<()> {
    let directory = std::env::temp_dir()
        .canonicalize()?
        .join(format!("tessera-lifecycle-perf-{}", std::process::id()));
    std::fs::create_dir(&directory)?;
    let path = directory.join("saved.tessera");
    let start = Instant::now();
    let outcome = files::save_project(&path, project, None, control)
        .inspect_err(|_| eprintln!("Save evidence retained at {}", directory.display()))?;
    elapsed("save_publication", start);
    if !matches!(outcome.publication, Publication::Committed)
        || outcome.committed_stamp.is_none()
        || outcome.warning.is_some()
        || outcome.recovery.is_some()
    {
        eprintln!(
            "Save evidence retained at {}: {:?}; warning={:?}; recovery={:?}",
            directory.display(),
            outcome.publication,
            outcome.warning,
            outcome.recovery
        );
        return Err(AppError::invalid(
            "publication",
            "Save publication not cleanly confirmed; evidence preserved.",
        ));
    }
    let start = Instant::now();
    let (loaded, stamp) = files::read_project_with_budget(
        &path,
        control,
        &[project.dataset()],
        project.owned_bytes()?,
    )
    .inspect_err(|_| eprintln!("Load evidence retained at {}", directory.display()))?;
    elapsed("load_after_save", start);
    assert_eq!(Some(&stamp), outcome.committed_stamp.as_ref());
    assert_eq!(loaded.dataset().revision(), project.dataset().revision());
    assert_eq!(loaded.dataset().row_count(), project.dataset().row_count());
    assert_eq!(loaded.runs().len(), project.runs().len());
    eprintln!(
        "retained_save_load_overlap_bytes={}",
        Budget::dataset_bytes(
            &[project.dataset(), loaded.dataset()],
            Project::retained_owned_bytes(&[project, &loaded])?
        )?
    );
    // Confirmed operation ended; the exclusively owned directory includes its lock file.
    std::fs::remove_dir_all(&directory)?;
    Ok(())
}
fn run() -> AppResult<()> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 3 {
        return Err(AppError::invalid(
            "arguments",
            "Use cancel, shutdown or save-load PATH.",
        ));
    }
    let control = JobControl::default();
    let start = Instant::now();
    let project = Arc::new(load(std::path::Path::new(&args[2]), &control)?);
    elapsed("fixture_load", start);
    eprintln!(
        "retained_project_bytes={}",
        Budget::dataset_bytes(&[project.dataset()], project.owned_bytes()?)?
    );
    match args[1].to_str() {
        Some("cancel") => worker(project, false),
        Some("shutdown") => worker(project, true),
        Some("save-load") => save_load(&project, &control),
        _ => Err(AppError::invalid(
            "operation",
            "Use cancel, shutdown or save-load.",
        )),
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
