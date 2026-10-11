use super::*;
pub fn paired_t_with_budget(
    dataset: &Dataset,
    request: &PairedTRequest,
    control: &JobControl,
    retained: &[&Dataset],
    extra: usize,
) -> AppResult<ComputedPairedT> {
    let (rows, mut diagnostics, columns) = prepare(
        dataset,
        AnalysisRequestRef::PairedT(request),
        control,
        retained,
        extra,
    )?;
    let mut samples: [ContrastMoments; 3] = std::array::from_fn(|_| ContrastMoments::default());
    for (index, row) in rows.iter().enumerate() {
        tick(control, index)?;
        if let (Some(a), Some(b)) = (
            number(dataset, request.first, row.row)?,
            number(dataset, request.second, row.row)?,
        ) {
            samples[0].add(a, 0., row.weight);
            samples[1].add(b, 0., row.weight);
            samples[2].add(a, b, row.weight);
        } else {
            diagnostics.complete_case_rows += 1;
        }
    }
    let mut norms: [ContrastNorm; 3] = std::array::from_fn(|_| ContrastNorm::default());
    for (index, row) in rows.iter().enumerate() {
        tick(control, index)?;
        if let (Some(a), Some(b)) = (
            number(dataset, request.first, row.row)?,
            number(dataset, request.second, row.row)?,
        ) {
            for (i, (a, b)) in [(a, 0.), (b, 0.), (a, b)].into_iter().enumerate() {
                norms[i].add(samples[i].deviation(a, b), row.weight);
            }
        }
    }
    let n = samples[0].n;
    let sds: [Option<Scaled>; 3] = std::array::from_fn(|i| (n > 1).then(|| norms[i].sd(n)));
    let statistics = statistics(
        (n > 0).then(|| samples[2].mean()),
        sds[2].map(|sd| sd.times(1. / (n as f64).sqrt())),
        (n > 1).then(|| (n - 1) as f64),
        sds[2],
        request.confidence,
        control,
    )?;
    control.check_cancelled()?;
    Ok(ComputedPairedT {
        revision: dataset.revision(),
        request: request.clone(),
        columns,
        diagnostics,
        first: summary(&samples[0], sds[0]),
        second: summary(&samples[1], sds[1]),
        difference_sd: sds[2].map_or_else(missing, Scaled::estimate),
        statistics,
    })
}
