use super::*;
use crate::export::figures::FigureFormat;
use crate::figures::{FigureKey, FigureKind};
use std::path::Path;

pub(super) fn pending_bytes(directory: &PathBuf, keys: &Vec<FigureKey>) -> AppResult<usize> {
    if keys.is_empty() || keys.len() > 4096 {
        return Err(AppError::invalid(
            "figures",
            "Select between 1 and 4096 figures.",
        ));
    }
    let per_result = add(
        size_of::<SaveOutcome>() + 4096,
        directory
            .capacity()
            .checked_mul(8)
            .ok_or(AppError::SizeLimit {
                limit: MAX_OWNED_BYTES,
            })?,
    )?;
    let results = keys
        .len()
        .checked_mul(per_result)
        .ok_or(AppError::SizeLimit {
            limit: MAX_OWNED_BYTES,
        })?;
    add(
        add(
            directory.as_os_str().len(),
            keys.capacity()
                .checked_mul(size_of::<FigureKey>())
                .ok_or(AppError::SizeLimit {
                    limit: MAX_OWNED_BYTES,
                })?,
        )?,
        results,
    )
}
fn filename(key: &FigureKey, format: FigureFormat) -> String {
    let kind = match key.kind {
        FigureKind::Histogram => "histogram".into(),
        FigureKind::Scatter => "scatter".into(),
        FigureKind::Boxplot => "boxplot".into(),
        FigureKind::GroupMeans => "group-means".into(),
        FigureKind::Likert => "likert".into(),
        FigureKind::Residual => "residual".into(),
        FigureKind::Qq => "qq".into(),
        FigureKind::Distribution(column) => format!("distribution-{column}"),
        FigureKind::Frequency(column) => format!("frequency-{column}"),
    };
    let extension = match format {
        FigureFormat::Svg => "svg",
        FigureFormat::Png => "png",
    };
    format!("run-{}-{kind}-{}.{}", key.run, key.variant, extension)
}
pub(super) fn export_batch(
    project: &Project,
    directory: &Path,
    keys: &[FigureKey],
    format: FigureFormat,
    control: &JobControl,
    retained: &[&Dataset],
    extra: usize,
) -> AppResult<Vec<SaveOutcome>> {
    if !directory.is_dir() {
        return Err(AppError::invalid(
            "figure directory",
            "Choose an existing directory.",
        ));
    }
    for (index, key) in keys.iter().enumerate() {
        control.checkpoint(index, Some(keys.len()))?;
        if keys[..index].contains(key) {
            return Err(AppError::invalid(
                "figures",
                "Each figure may only be selected once.",
            ));
        }
        let valid = project
            .runs()
            .iter()
            .find(|run| run.id.0 == key.run)
            .is_some_and(|run| {
                crate::figures::figure_keys(run)
                    .iter()
                    .any(|base| base.kind == key.kind)
            });
        let variant_exists =
            key.variant == 0 || project.figures().iter().any(|record| record.key == *key);
        if !valid || !variant_exists {
            return Err(AppError::invalid(
                "figure",
                "The figure must belong to a saved result.",
            ));
        }
        project.figure_settings(key).validate()?;
    }
    let mut outcomes = Vec::with_capacity(keys.len());
    for key in keys {
        let result = export::figures::export_figure(
            &directory.join(filename(key, format)),
            project,
            key,
            format,
            control,
            retained,
            extra,
        );
        match result {
            Ok(outcome) => {
                let stop =
                    outcome.publication != Publication::Committed || outcome.warning.is_some();
                outcomes.push(outcome);
                if stop {
                    break;
                }
            }
            Err(error) if outcomes.is_empty() => return Err(error),
            Err(error) => {
                return Err(AppError::invalid(
                    "figure export",
                    format!(
                        "{error}. Export stopped after {} figures. Completed files remain in {}.",
                        outcomes
                            .iter()
                            .filter(|o| o.committed_stamp.is_some())
                            .count(),
                        directory.display()
                    ),
                ));
            }
        }
    }
    Ok(outcomes)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(unix)]
    fn partial_export_reports_the_cause_before_a_long_directory() {
        let base = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "tessera-batch-cause-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let directory = base.join("long-folder/".repeat(50));
        std::fs::create_dir_all(&directory).unwrap();
        let data = Dataset::new(vec![crate::data::Column::numeric(
            crate::data::ColumnId(1),
            "Score",
            vec![Some(1.), Some(2.), Some(3.)],
        )
        .unwrap()])
        .unwrap();
        let request =
            crate::analysis::AnalysisRequest::Histogram(crate::analysis::HistogramRequest {
                variable: crate::data::ColumnId(1),
                bins: crate::analysis::HistogramBins::Count(2),
                filter: Default::default(),
                frequency_weight: None,
            });
        let analysis = crate::analysis::analyze_with_budget(
            &data,
            &request,
            &JobControl::default(),
            &[],
            4096,
        )
        .unwrap();
        let project = Project::new("Figures", data)
            .unwrap()
            .with_run(analysis.clone())
            .unwrap()
            .with_run(analysis)
            .unwrap();
        let keys = [
            FigureKey {
                run: 1,
                kind: FigureKind::Histogram,
                variant: 0,
            },
            FigureKey {
                run: 2,
                kind: FigureKind::Histogram,
                variant: 0,
            },
        ];
        std::fs::create_dir(directory.join("run-2-histogram-0.svg")).unwrap();
        let error = export_batch(
            &project,
            &directory,
            &keys,
            FigureFormat::Svg,
            &JobControl::default(),
            &[],
            0,
        )
        .unwrap_err()
        .to_string();
        let summary = error.chars().take(256).collect::<String>();
        assert!(summary.contains("Choose a regular file"), "{summary}");
        assert!(error.contains("after 1 figures"));
        assert!(error.contains(&directory.display().to_string()));
        assert!(directory.join("run-1-histogram-0.svg").is_file());
        std::fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn stable_names_include_column_and_variant() {
        assert_eq!(
            filename(
                &FigureKey {
                    run: 7,
                    kind: FigureKind::Frequency(12),
                    variant: 3
                },
                FigureFormat::Svg
            ),
            "run-7-frequency-12-3.svg"
        );
    }
}
