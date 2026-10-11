//! Bounded Gauss–Kronrod integration for the two Student density transforms.
use crate::{error::AppResult, operation::JobControl};
const NODES: [f64; 8] = [
    0.9914553711208126,
    0.9491079123427585,
    0.8648644233597691,
    0.7415311855993945,
    0.5860872354676911,
    0.4058451513773972,
    0.2077849550078985,
    0.0,
];
const WEIGHTS: [f64; 8] = [
    0.022935322010529225,
    0.06309209262997855,
    0.10479001032225018,
    0.14065325971552592,
    0.1690047266392679,
    0.19035057806478542,
    0.20443294007529889,
    0.20948214108472782,
];
const GAUSS: [f64; 4] = [
    0.1294849661688697,
    0.27970539148927667,
    0.3818300505051189,
    0.4179591836734694,
];
#[derive(Clone, Copy, Default)]
struct Interval {
    left: f64,
    right: f64,
    depth: usize,
}
pub(super) struct Integral {
    pub value: f64,
    pub error: f64,
}
pub(super) fn integrate(
    f: impl Fn(f64) -> f64,
    end: f64,
    control: &JobControl,
) -> AppResult<Option<Integral>> {
    integrate_bounded(f, end, control, 32768, 24)
}
// Fixed stack: depth-first subdivision stores at most depth+1 pending intervals.
fn integrate_bounded(
    f: impl Fn(f64) -> f64,
    end: f64,
    control: &JobControl,
    max_evaluations: usize,
    max_depth: usize,
) -> AppResult<Option<Integral>> {
    let mut stack = [Interval::default(); 25];
    stack[0] = Interval {
        left: 0.0,
        right: end,
        depth: 0,
    };
    let (mut pending, mut evaluations, mut total, mut error) = (1, 0, 0.0, 0.0);
    while pending > 0 {
        pending -= 1;
        let interval = stack[pending];
        if evaluations + 15 > max_evaluations {
            return Ok(None);
        }
        let (value, uncertainty) = panel(&f, interval, &mut evaluations, control)?;
        let tolerance = (2e-15 * (interval.right - interval.left) / end).max(2e-14 * value.abs());
        if !value.is_finite() || !uncertainty.is_finite() {
            return Ok(None);
        }
        if uncertainty <= tolerance {
            total += value;
            error += uncertainty;
            continue;
        }
        if interval.depth >= max_depth.min(24) {
            return Ok(None);
        }
        let middle = (interval.left + interval.right) * 0.5;
        stack[pending] = Interval {
            left: middle,
            right: interval.right,
            depth: interval.depth + 1,
        };
        stack[pending + 1] = Interval {
            left: interval.left,
            right: middle,
            depth: interval.depth + 1,
        };
        pending += 2;
    }
    control.check_cancelled()?;
    Ok(
        (error <= 2e-15_f64.max(2e-14 * total.abs())).then_some(Integral {
            value: total,
            error,
        }),
    )
}
fn panel(
    f: &impl Fn(f64) -> f64,
    interval: Interval,
    evaluations: &mut usize,
    control: &JobControl,
) -> AppResult<(f64, f64)> {
    let middle = (interval.left + interval.right) * 0.5;
    let half = (interval.right - interval.left) * 0.5;
    let mut eval = |x| {
        if (*evaluations).is_multiple_of(64) {
            control.check_cancelled()?;
        }
        *evaluations += 1;
        Ok::<f64, crate::error::AppError>(f(x))
    };
    let center = eval(middle)?;
    let (mut kronrod, mut gauss) = (WEIGHTS[7] * center, GAUSS[3] * center);
    let mut samples = [0.0; 14];
    for i in 0..7 {
        samples[2 * i] = eval(middle - half * NODES[i])?;
        samples[2 * i + 1] = eval(middle + half * NODES[i])?;
        let pair = samples[2 * i] + samples[2 * i + 1];
        kronrod += WEIGHTS[i] * pair;
        if i % 2 == 1 {
            gauss += GAUSS[i / 2] * pair;
        }
    }
    let mean = kronrod * 0.5;
    let mut deviation = WEIGHTS[7] * (center - mean).abs();
    for i in 0..7 {
        deviation +=
            WEIGHTS[i] * ((samples[2 * i] - mean).abs() + (samples[2 * i + 1] - mean).abs());
    }
    let raw = (kronrod - gauss).abs() * half;
    let asc = deviation * half;
    let error = if asc > 0.0 {
        asc * (200.0 * raw / asc).powf(1.5).min(1.0)
    } else {
        raw
    };
    Ok((
        kronrod * half,
        error.max(50.0 * f64::EPSILON * kronrod.abs() * half),
    ))
}

#[cfg(test)]
pub(super) fn integrate_with_test_limits(
    f: impl Fn(f64) -> f64,
    end: f64,
    control: &JobControl,
    max_evaluations: usize,
    max_depth: usize,
) -> AppResult<Option<Integral>> {
    integrate_bounded(f, end, control, max_evaluations, max_depth)
}
