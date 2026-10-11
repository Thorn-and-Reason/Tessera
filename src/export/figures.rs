//! Standalone figure publication keeps the previous destination on failure or cancellation.
use crate::{
    data::{Dataset, MAX_OWNED_BYTES},
    error::{AppError, AppResult},
    figures::{render, FigureKey},
    files::{self, SaveOutcome},
    operation::JobControl,
    project::Project,
};
use std::path::Path;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FigureFormat {
    Svg,
    Png,
}
pub fn export_figure(
    path: &Path,
    project: &Project,
    key: &FigureKey,
    format: FigureFormat,
    control: &JobControl,
    retained: &[&Dataset],
    extra: usize,
) -> AppResult<SaveOutcome> {
    control.check_cancelled()?;
    let run = project
        .runs()
        .iter()
        .find(|r| r.id.0 == key.run)
        .ok_or_else(|| AppError::invalid("figure", "Choose a retained analysis figure."))?;
    if key.variant != 0 && !project.figures().iter().any(|record| record.key == *key) {
        return Err(AppError::invalid(
            "figure",
            "Choose a saved figure version.",
        ));
    }
    let settings = project.figure_settings(key);
    settings.validate()?;
    let (w, h) = settings.pixel_size()?;
    // Raster allocation, PNG encoder scratch and SVG/text geometry must coexist with retained work.
    let scratch = if format == FigureFormat::Png {
        (w as usize)
            .checked_mul(h as usize)
            .and_then(|n| n.checked_mul(16))
            .and_then(|n| n.checked_add(crate::figures::SVG_WORK_BYTES))
            .ok_or(AppError::SizeLimit {
                limit: MAX_OWNED_BYTES,
            })?
    } else {
        32 * 1024 * 1024
    };
    let peak = extra
        .checked_add(project.owned_bytes_controlled(control)?)
        .and_then(|n| n.checked_add(scratch))
        .ok_or(AppError::SizeLimit {
            limit: MAX_OWNED_BYTES,
        })?;
    super::check_budget(project.dataset(), retained, peak)?;
    run.analysis.validate_controlled(control)?;
    let svg = render::svg(
        run,
        key,
        &settings,
        run.analysis.revision() != project.dataset().revision(),
        control,
    )?;
    let bytes = match format {
        FigureFormat::Svg => svg.into_bytes(),
        FigureFormat::Png => render::png(&svg, &settings, control)?,
    };
    files::publish(
        path,
        Some(project.dataset().revision()),
        None,
        control,
        |writer| {
            control.check_cancelled()?;
            writer.write_all(&bytes)?;
            control.check_cancelled()
        },
    )
}
