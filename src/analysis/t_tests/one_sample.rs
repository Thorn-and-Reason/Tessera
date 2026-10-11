use super::*;
pub fn one_sample_t_with_budget(
    dataset: &Dataset,
    request: &OneSampleTRequest,
    control: &JobControl,
    retained: &[&Dataset],
    extra: usize,
) -> AppResult<ComputedOneSampleT> {
    let (rows, diagnostics, columns) = prepare(
        dataset,
        AnalysisRequestRef::OneSampleT(request),
        control,
        retained,
        extra,
    )?;
    let mut sample = ContrastMoments::default();
    let mut missing_rows = 0;
    for (index, row) in rows.iter().enumerate() {
        tick(control, index)?;
        if let Some(value) = number(dataset, request.variable, row.row)? {
            sample.add(value, 0., row.weight);
        } else {
            missing_rows += 1;
        }
    }
    let mut norm = ContrastNorm::default();
    for (index, row) in rows.iter().enumerate() {
        tick(control, index)?;
        if let Some(value) = number(dataset, request.variable, row.row)? {
            norm.add(sample.deviation(value, 0.), row.weight);
        }
    }
    let sd = (sample.n > 1).then(|| norm.sd(sample.n));
    let statistics = statistics(
        (sample.n > 0).then(|| sample.difference(request.reference_mean)),
        sd.map(|sd| sd.times(1. / (sample.n as f64).sqrt())),
        (sample.n > 1).then(|| (sample.n - 1) as f64),
        sd,
        request.confidence,
        control,
    )?;
    control.check_cancelled()?;
    Ok(ComputedOneSampleT {
        revision: dataset.revision(),
        request: request.clone(),
        columns,
        diagnostics,
        sample: summary(&sample, sd),
        missing_rows,
        statistics,
    })
}
