use super::*;
fn group(
    dataset: &Dataset,
    request: &IndependentTRequest,
    row: usize,
    exclusions: &mut GroupExclusions,
) -> AppResult<Option<(usize, f64)>> {
    let column = dataset.column(request.group)?;
    if column.is_missing(row)? {
        exclusions.missing_group_rows += 1;
        return Ok(None);
    }
    let mut selected = None;
    for (index, value) in request.groups.iter().enumerate() {
        let matches = match value {
            Cell::Number(value) => number(dataset, request.group, row)? == Some(*value),
            Cell::Text(value) => column.text_value(row)? == Some(value.as_str()),
            Cell::Missing => false,
        };
        if matches {
            selected = Some(index);
            break;
        }
    }
    let Some(index) = selected else {
        exclusions.other_group_rows += 1;
        return Ok(None);
    };
    let Some(value) = number(dataset, request.outcome, row)? else {
        exclusions.missing_outcome_rows += 1;
        return Ok(None);
    };
    Ok(Some((index, value)))
}
pub fn independent_t_with_budget(
    dataset: &Dataset,
    request: &IndependentTRequest,
    control: &JobControl,
    retained: &[&Dataset],
    extra: usize,
) -> AppResult<ComputedIndependentT> {
    let (rows, diagnostics, columns) = prepare(
        dataset,
        AnalysisRequestRef::IndependentT(request),
        control,
        retained,
        extra,
    )?;
    let mut samples: [ContrastMoments; 2] = std::array::from_fn(|_| ContrastMoments::default());
    let mut exclusions = GroupExclusions::default();
    for (index, row) in rows.iter().enumerate() {
        tick(control, index)?;
        if let Some((i, value)) = group(dataset, request, row.row, &mut exclusions)? {
            samples[i].add(value, 0., row.weight);
        }
    }
    let mut norms: [ContrastNorm; 2] = std::array::from_fn(|_| ContrastNorm::default());
    for (index, row) in rows.iter().enumerate() {
        tick(control, index)?;
        if let Some((i, value)) = group(dataset, request, row.row, &mut GroupExclusions::default())?
        {
            norms[i].add(samples[i].deviation(value, 0.), row.weight);
        }
    }
    let sds: [Option<Scaled>; 2] =
        std::array::from_fn(|i| (samples[i].n > 1).then(|| norms[i].sd(samples[i].n)));
    let (se, df, pooled) = inference_parts(&samples, sds, request.method);
    let statistics = statistics(
        samples
            .iter()
            .all(|s| s.n > 0)
            .then(|| samples[0].contrast(&samples[1])),
        se,
        df,
        pooled,
        request.confidence,
        control,
    )?;
    control.check_cancelled()?;
    Ok(ComputedIndependentT {
        revision: dataset.revision(),
        request: request.clone(),
        columns,
        diagnostics,
        groups: std::array::from_fn(|i| summary(&samples[i], sds[i])),
        exclusions,
        statistics,
    })
}
fn inference_parts(
    samples: &[ContrastMoments; 2],
    sds: [Option<Scaled>; 2],
    method: IndependentMethod,
) -> (Option<Scaled>, Option<f64>, Option<Scaled>) {
    let [Some(a), Some(b)] = sds else {
        return (None, None, None);
    };
    let (n, m) = (samples[0].n as f64, samples[1].n as f64);
    let df = n + m - 2.;
    let scale = if a.exponent >= b.exponent && a.fraction != 0. || b.fraction == 0. {
        a
    } else {
        b
    };
    if scale.fraction == 0. {
        return (
            Some(scale),
            (method == IndependentMethod::Pooled).then_some(df),
            Some(scale),
        );
    }
    let (a, b) = (a.ratio(scale).value(), b.ratio(scale).value());
    let pooled = scale.times(((a * a * (n - 1.) + b * b * (m - 1.)) / df).sqrt());
    if method == IndependentMethod::Pooled {
        return (
            Some(pooled.times((1. / n + 1. / m).sqrt())),
            Some(df),
            Some(pooled),
        );
    }
    let (v, w) = (a * a / n, b * b / m);
    let sum = v + w;
    let df = 1. / ((v / sum).powi(2) / (n - 1.) + (w / sum).powi(2) / (m - 1.));
    (Some(scale.times(sum.sqrt())), Some(df), Some(pooled))
}
