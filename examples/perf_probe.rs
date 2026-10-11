//! Development-only independent-process benchmark; stdout is discarded by measure.py.
//! CSV roles: first numeric outcome, second numeric paired/predictor; first two text factors.
//! Repeated design requires a column named Subject with unique nonmissing IDs and first 3 numerics.
use std::{fs::File, io::Read, sync::Arc, time::Instant};
use tessera::{
    analysis::*,
    data::{Budget, Cell, ColumnId, ColumnKind, Dataset},
    error::{AppError, AppResult},
    files::Publication,
    import::{commit_preview, preview, Delimiter, MAX_INPUT_BYTES},
    operation::JobControl,
    project::Project,
    project_io::{decode, encode, encode_with_budget},
};

fn stage<T>(name: &str, work: impl FnOnce() -> AppResult<T>) -> AppResult<T> {
    let start = Instant::now();
    let result = work();
    eprintln!(
        "stage_{name}_ms={:.6}",
        start.elapsed().as_secs_f64() * 1000.0
    );
    result
}
fn required(ids: &[ColumnId], index: usize) -> AppResult<ColumnId> {
    ids.get(index)
        .copied()
        .ok_or_else(|| AppError::invalid("fixture", "Missing required fixture variable."))
}
fn request(op: &str, data: &Dataset) -> AppResult<AnalysisRequest> {
    let numbers: Vec<_> = data
        .columns()
        .iter()
        .filter(|c| c.kind() == ColumnKind::Number)
        .map(|c| c.id())
        .collect();
    let texts: Vec<_> = data
        .columns()
        .iter()
        .filter(|c| c.kind() == ColumnKind::Text)
        .map(|c| c.id())
        .collect();
    let first = required(&numbers, 0)?;
    let filter = RowFilter::default();
    let confidence = ConfidenceLevel::P95;
    Ok(match op {
        "describe" => AnalysisRequest::Describe(DescribeRequest {
            columns: numbers,
            statistics: vec![
                Statistic::Mean,
                Statistic::SampleStdDev,
                Statistic::Variance,
                Statistic::Minimum,
                Statistic::Maximum,
                Statistic::Sum,
                Statistic::Range,
                Statistic::StdErrorMean,
            ],
            missing_policy: MissingPolicy::PerVariable,
            filter,
            frequency_weight: None,
        }),
        "pearson" => AnalysisRequest::Pearson(PearsonRequest {
            columns: numbers,
            missing_policy: CorrelationMissingPolicy::PairwiseComplete,
            filter,
            frequency_weight: None,
        }),
        "frequencies" => AnalysisRequest::Frequencies(FrequenciesRequest {
            columns: data.columns().iter().map(|c| c.id()).collect(),
            filter,
            frequency_weight: None,
        }),
        "histogram" => AnalysisRequest::Histogram(HistogramRequest {
            variable: first,
            bins: HistogramBins::Automatic,
            filter,
            frequency_weight: None,
        }),
        "scatter" => AnalysisRequest::Scatter(ScatterRequest {
            x: first,
            y: required(&numbers, 1)?,
            filter,
            frequency_weight: None,
        }),
        "boxplot" => AnalysisRequest::Boxplot(BoxplotRequest {
            outcome: first,
            group: texts.first().copied(),
            filter,
            frequency_weight: None,
        }),
        "one-sample-t" => AnalysisRequest::OneSampleT(OneSampleTRequest {
            variable: first,
            reference_mean: 0.0,
            confidence,
            filter,
            frequency_weight: None,
        }),
        "paired-t" => AnalysisRequest::PairedT(PairedTRequest {
            first,
            second: required(&numbers, 1)?,
            confidence,
            filter,
            frequency_weight: None,
        }),
        "independent-t" => {
            let group = required(&texts, 0)?;
            let column = data.column(group)?;
            let mut values = Vec::new();
            for row in 0..data.row_count() {
                if !column.is_missing(row)? {
                    let value = column.cell(row)?;
                    if !values.contains(&value) {
                        values.push(value);
                    }
                    if values.len() == 2 {
                        break;
                    }
                }
            }
            if values.len() != 2 {
                return Err(AppError::invalid(
                    "fixture",
                    "Two nonmissing groups are required.",
                ));
            }
            AnalysisRequest::IndependentT(IndependentTRequest {
                outcome: first,
                group,
                groups: [values.remove(0), values.remove(0)],
                method: IndependentMethod::Welch,
                confidence,
                filter,
                frequency_weight: None,
            })
        }
        "regression" => AnalysisRequest::Regression(RegressionRequest {
            outcome: first,
            predictors: numbers
                .iter()
                .skip(1)
                .map(|column| RegressionPredictor {
                    column: *column,
                    role: PredictorRole::Numeric,
                })
                .collect(),
            confidence,
            filter,
            frequency_weight: None,
        }),
        "reliability" => AnalysisRequest::Reliability(ReliabilityRequest {
            items: numbers,
            filter,
            frequency_weight: None,
            reversals: vec![],
        }),
        "anova-one-way" | "anova-factorial" | "anova-repeated" => {
            let design = match op {
                "anova-one-way" => AnovaDesign::OneWay {
                    outcome: first,
                    factor: required(&texts, 0)?,
                },
                "anova-factorial" => AnovaDesign::Factorial {
                    outcome: first,
                    factors: vec![required(&texts, 0)?, required(&texts, 1)?],
                },
                _ => AnovaDesign::Repeated {
                    subject: data
                        .columns()
                        .iter()
                        .find(|c| c.name() == "Subject")
                        .map(|c| c.id())
                        .ok_or_else(|| {
                            AppError::invalid("fixture", "Repeated measures requires Subject.")
                        })?,
                    factors: vec![AnovaWithinFactor {
                        name: "Time".into(),
                        levels: vec!["1".into(), "2".into(), "3".into()],
                    }],
                    measurements: vec![first, required(&numbers, 1)?, required(&numbers, 2)?],
                },
            };
            AnalysisRequest::Anova(AnovaRequest {
                design,
                confidence,
                filter,
                frequency_weight: None,
                planned_pairs: vec![],
                all_pairs: vec![],
            })
        }
        _ => return Err(AppError::invalid("operation", "Unknown probe operation.")),
    })
}
fn run() -> AppResult<()> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 3 {
        return Err(AppError::invalid("arguments", "Use OPERATION PATH."));
    }
    let op = args[1]
        .to_str()
        .ok_or_else(|| AppError::invalid("operation", "Operation must be UTF-8."))?;
    let control = JobControl::default();
    let project = if op == "load"
        || std::path::Path::new(&args[2])
            .extension()
            .is_some_and(|e| e == "tessera")
    {
        stage("load", || decode(File::open(&args[2])?, &control))?
    } else {
        let bytes = stage("read", || {
            let mut bytes = Vec::new();
            File::open(&args[2])?
                .take(MAX_INPUT_BYTES as u64 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > MAX_INPUT_BYTES {
                return Err(AppError::SizeLimit {
                    limit: MAX_INPUT_BYTES,
                });
            }
            Ok(bytes)
        })?;
        let input = stage("preview", || {
            preview(Arc::from(bytes), Delimiter::Comma, &control)
        })?;
        let choices = input.default_choices()?;
        let dataset = stage("commit", || commit_preview(&input, &choices, &control))?;
        Project::new("Development reference probe", dataset)?
    };
    eprintln!(
        "retained_project_bytes={}",
        Budget::dataset_bytes(&[project.dataset()], project.owned_bytes()?)?
    );
    match op {
        "inspect" | "load" => println!(
            "{} rows × {} variables",
            project.dataset().row_count(),
            project.dataset().column_count()
        ),
        "save" => stage("encode", || {
            encode(&project, std::io::stdout().lock(), &control)
        })?,
        "edit" => {
            let column = project
                .dataset()
                .columns()
                .first()
                .ok_or_else(|| AppError::invalid("fixture", "A column is required."))?;
            let value = match column.kind() {
                ColumnKind::Number => Cell::Number(123.0),
                ColumnKind::Text => Cell::Text("benchmark edit".into()),
            };
            let edited = stage("edit", || {
                project.dataset().edit_cell_with_budget(
                    column.id(),
                    0,
                    value,
                    &[],
                    project.owned_bytes()?,
                )
            })?;
            eprintln!(
                "retained_overlap_bytes={}",
                Budget::dataset_bytes(&[project.dataset(), &edited], project.owned_bytes()?)?
            );
        }
        "groups" => {
            let column = project
                .dataset()
                .columns()
                .iter()
                .find(|c| c.kind() == ColumnKind::Text)
                .ok_or_else(|| AppError::invalid("fixture", "A text group is required."))?;
            let result = stage("groups", || {
                group_values_with_budget(
                    project.dataset(),
                    &GroupLookupRequest {
                        column: column.id(),
                        role_epoch: 0,
                        query: String::new(),
                        selected: [None, None],
                    },
                    &control,
                    &[],
                    project.owned_bytes()?,
                )
            })?;
            println!("{} choices; more={}", result.choices.len(), result.more);
        }
        "export" => {
            // Exclusive ownership prevents cleanup from touching another probe's output.
            let directory = std::env::temp_dir()
                .canonicalize()?
                .join(format!("tessera-perf-{}", std::process::id()));
            std::fs::create_dir(&directory)?;
            let path = directory.join("report.html");
            let ids: Vec<_> = project.runs().iter().map(|r| r.id).collect();
            let outcome = stage("export", || {
                tessera::export::export_report(&path, &project, &ids, &control)
            })
            .inspect_err(|_| eprintln!("Export evidence retained at {}", directory.display()))?;
            if !matches!(outcome.publication, Publication::Committed)
                || outcome.committed_stamp.is_none()
                || outcome.warning.is_some()
                || outcome.recovery.is_some()
            {
                eprintln!(
                    "Export evidence retained at {}: {:?}; warning={:?}; recovery={:?}",
                    directory.display(),
                    outcome.publication,
                    outcome.warning,
                    outcome.recovery
                );
                return Err(AppError::invalid(
                    "publication",
                    "Export publication was not cleanly confirmed; recovery evidence preserved.",
                ));
            }
            // Confirmed operation ended; the exclusively owned directory includes its lock file.
            std::fs::remove_dir_all(&directory)?;
        }
        _ => {
            let request = request(op, project.dataset())?;
            let result = stage("analysis", || {
                analyze_with_budget(
                    project.dataset(),
                    &request,
                    &control,
                    &[],
                    project.owned_bytes()?
                        + std::mem::size_of::<AnalysisRequest>()
                        + request.heap_bytes()?,
                )
            })?;
            eprintln!("retained_analysis_bytes={}", result.owned_bytes()?);
            let request_owner = std::mem::size_of::<AnalysisRequest>() + request.heap_bytes()?;
            let completed = stage("history", || {
                project.with_run_with_budget_controlled(result, &[], request_owner, &control)
            })?;
            let overlap = Project::retained_owned_bytes(&[&project, &completed])? + request_owner;
            eprintln!(
                "retained_history_project_bytes={}",
                Budget::dataset_bytes(&[project.dataset(), completed.dataset()], overlap)?
            );
            stage("encode", || {
                encode_with_budget(
                    &completed,
                    std::io::stdout().lock(),
                    &control,
                    &[project.dataset()],
                    overlap
                        .checked_sub(completed.owned_bytes()?)
                        .ok_or_else(|| {
                            AppError::invalid("accounting", "Invalid project overlap accounting.")
                        })?,
                )
            })?;
        }
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
