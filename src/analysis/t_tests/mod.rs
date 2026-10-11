mod independent;
mod one_sample;
mod paired;
use super::{
    moments::{ContrastMoments, ContrastNorm, Scaled},
    selection::Eligible,
    *,
};
pub use independent::independent_t_with_budget;
pub use one_sample::one_sample_t_with_budget;
pub use paired::paired_t_with_budget;

fn prepare(
    dataset: &Dataset,
    request: AnalysisRequestRef<'_>,
    control: &JobControl,
    retained: &[&Dataset],
    extra: usize,
) -> AppResult<(Vec<Eligible>, RunDiagnostics, Vec<RunColumnInfo>)> {
    control.checkpoint(0, None)?;
    let validation = request.validation_scratch_bytes()?;
    Budget::context_bytes(
        dataset,
        retained,
        None,
        model::checked_sum([extra, validation])?,
    )?;
    let ids = support::validate_current(dataset, request)?;
    let live = model::checked_sum([
        extra,
        validation,
        request.heap_bytes()?,
        support::snapshot_bytes(dataset, &ids)?,
        model::checked_product(dataset.row_count(), size_of::<Eligible>())?,
        inference::INFERENCE_WORK_BYTES,
        16_384,
    ])?;
    // Fixed allowance covers all three accumulators, stack scratch, result and reasons.
    Budget::context_bytes(dataset, retained, None, live)?;
    let (rows, diagnostics) = selection::select_rows(
        dataset,
        request.filter(),
        request.frequency_weight(),
        &[],
        control,
        dataset.row_count(),
    )?;
    Ok((
        rows,
        diagnostics,
        support::capture_columns(dataset, &ids, control)?,
    ))
}
fn number(dataset: &Dataset, id: ColumnId, row: usize) -> AppResult<Option<f64>> {
    let column = dataset.column(id)?;
    if column.is_missing(row)? {
        return Ok(None);
    }
    Ok(match column.cell(row)? {
        Cell::Number(value) => Some(value),
        _ => None,
    })
}
fn tick(control: &JobControl, index: usize) -> AppResult<()> {
    if index.is_multiple_of(4096) {
        control.check_cancelled()?;
    }
    Ok(())
}
fn missing() -> Estimate {
    Estimate::unavailable("At least two observations per sample are required.")
}
fn summary(sample: &ContrastMoments, sd: Option<Scaled>) -> SampleSummary {
    SampleSummary {
        n: sample.n,
        mean: if sample.n > 0 {
            sample.mean().estimate()
        } else {
            Estimate::unavailable("No valid observations.")
        },
        sample_sd: sd.map_or_else(missing, Scaled::estimate),
    }
}
fn statistics(
    difference: Option<Scaled>,
    se: Option<Scaled>,
    df: Option<f64>,
    effect_sd: Option<Scaled>,
    confidence: ConfidenceLevel,
    control: &JobControl,
) -> AppResult<TStatistics> {
    let mut result = TStatistics {
        difference: difference.map_or_else(missing, Scaled::estimate),
        standard_error: se.map_or_else(missing, Scaled::estimate),
        t: missing(),
        df: df.map_or_else(missing, Estimate::available),
        p: missing(),
        interval: [missing(), missing()],
        effect_size: missing(),
    };
    if se.is_some_and(|value| value.fraction == 0.) {
        result.t = Estimate::unavailable("The standard error is zero.");
        result.p = result.t.clone();
        if df.is_none() {
            result.df = Estimate::unavailable("Both groups have zero sample variance.");
            result.interval = [result.df.clone(), result.df.clone()];
        }
    }
    if let (Some(difference), Some(sd)) = (difference, effect_sd) {
        if sd.fraction != 0. {
            result.effect_size = difference.ratio(sd).estimate();
        } else {
            result.effect_size = Estimate::unavailable("The sample spread is zero.");
        }
    }
    if let (Some(difference), Some(se), Some(df)) = (difference, se, df) {
        if se.fraction != 0. {
            let t = difference.ratio(se);
            result.t = t.estimate();
            result.p = if t.value().is_finite() {
                inference::student_t_two_sided_p(t.value(), df, control)?
            } else {
                inference::student_t_two_sided_p_from_log(t.log_abs(), df, control)?
            };
        }
        let critical = inference::student_t_critical(df, confidence, control)?;
        if let Some(critical) = critical.value {
            let margin = se.times(critical);
            result.interval = [
                difference.plus(margin.times(-1.)).estimate(),
                difference.plus(margin).estimate(),
            ];
        } else {
            result.interval = [critical.clone(), critical];
        }
    }
    Ok(result)
}
