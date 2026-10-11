use super::*;
pub(super) type CenteredRow = (Vec<moments::Scaled>, Vec<moments::Scaled>, moments::Scaled);
pub(super) struct Source<'a> {
    pub cohort: &'a ReliabilityCohort,
    pub columns: Vec<(&'a Column, Option<&'a ReliabilityReverse>)>,
    pub row: Vec<f64>,
    pub n: u64,
}
pub(super) struct Sums {
    pub items: Vec<BoundedSum>,
    pub total: BoundedSum,
}
pub(super) struct Scales {
    pub item: Vec<Option<i32>>,
    pub deleted: Vec<Option<i32>>,
    pub total: Option<i32>,
}
pub(super) fn item_sums(
    source: &mut Source<'_>,
    counter: &mut Counter<'_>,
) -> AppResult<(Vec<BoundedSum>, Vec<Estimate>)> {
    let k = source.row.len();
    let mut sums: Vec<_> = (0..k).map(|_| BoundedSum::default()).collect();
    let mut moments: Vec<_> = (0..k).map(|_| Moments::default()).collect();
    counter.checkpoint()?;
    for e in &source.cohort.eligible {
        counter.source()?;
        if !e.complete {
            continue;
        }
        fill_row(&source.columns, e.row, &mut source.row, counter)?;
        for i in 0..k {
            sums[i].push(source.row[i], e.weight)?;
            moments[i].add(source.row[i], e.weight);
            counter.tick()?;
        }
    }
    let mut mean = Vec::with_capacity(k);
    for m in &moments {
        counter.finalize()?;
        mean.push(publish_mean(m));
    }
    Ok((sums, mean))
}
pub(super) fn aggregate(items: Vec<BoundedSum>, counter: &mut Counter<'_>) -> AppResult<Sums> {
    let mut total = BoundedSum::default();
    for sum in &items {
        total.add_sum(sum, 1)?;
        counter.tick()?;
    }
    Ok(Sums { items, total })
}
pub(super) fn find_scales(
    source: &mut Source<'_>,
    sums: &Sums,
    counter: &mut Counter<'_>,
) -> AppResult<Scales> {
    let k = source.row.len();
    let mut scales = Scales {
        item: vec![None; k],
        deleted: vec![None; k],
        total: None,
    };
    counter.checkpoint()?;
    for e in &source.cohort.eligible {
        counter.source()?;
        if !e.complete {
            continue;
        }
        fill_row(&source.columns, e.row, &mut source.row, counter)?;
        let (d, rest, t) = centered(&sums.items, &sums.total, &source.row, source.n, counter)?;
        maxima(&mut scales.total, t);
        for i in 0..k {
            maxima(&mut scales.item[i], d[i]);
            maxima(&mut scales.deleted[i], rest[i]);
            counter.tick()?;
        }
    }
    Ok(scales)
}
pub(super) fn two_point(
    source: &mut Source<'_>,
    sums: &Sums,
    counter: &mut Counter<'_>,
) -> AppResult<Option<CenteredRow>> {
    if source.cohort.complete_rows != 2 {
        return Ok(None);
    }
    let mut first = None;
    for e in &source.cohort.eligible {
        counter.source()?;
        if e.complete {
            first = Some(e);
            break;
        }
    }
    let first = first.ok_or_else(invalid_analysis)?;
    fill_row(&source.columns, first.row, &mut source.row, counter)?;
    centered(&sums.items, &sums.total, &source.row, source.n, counter).map(Some)
}
