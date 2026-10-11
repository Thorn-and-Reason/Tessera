//! Frozen 613df22b numerical kernel, with typed endpoint refusal and existing exact owners.
//! Private normalized fractions keep formed arithmetic bounded (|fraction| <= 4).
//! Float64 residuals, N<=2^53 and k<=64 put endpoint exponents within +/- 16384;
//! exponent sums/differences therefore fit i32 without saturating arithmetic.
use super::ReliabilityEnergyState;
use crate::analysis;
use crate::{
    analysis::{moments::Moments, Estimate},
    error::{AppError, AppResult},
};
fn invariant() -> AppError {
    AppError::invalid(
        "Reliability arithmetic",
        "Endpoint normalization invariant violated.",
    )
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Extended {
    pub(super) f: f64,
    pub(super) e: i32,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Interval {
    pub(super) lo: Extended,
    pub(super) hi: Extended,
}
pub(super) fn norm(f: f64, e: i32) -> Extended {
    if f == 0. {
        return Extended { f: 0., e: 0 };
    }
    let bits = f.to_bits();
    let field = ((bits >> 52) & 2047) as i32;
    if field == 0 {
        return norm(f * 4503599627370496., e - 52);
    }
    let fraction = f64::from_bits((bits & !(2047u64 << 52)) | (1023u64 << 52));
    Extended {
        f: fraction,
        e: e + field - 1023,
    }
}
fn edge(x: f64, up: bool) -> f64 {
    if up {
        x.next_up()
    } else {
        x.next_down()
    }
}
fn aligned(a: Extended, e: i32, up: bool) -> f64 {
    if a.f == 0. {
        return 0.;
    }
    let d = a.e - e;
    if d == 0 {
        return a.f;
    }
    if d < -1074 {
        return match (a.f > 0., up) {
            (true, true) => f64::from_bits(1),
            (true, false) => 0.,
            (false, true) => 0.,
            (false, false) => -f64::from_bits(1),
        };
    }
    let v = a.f * 2f64.powi(d);
    // powi may underflow for -1074 on some libraries; use exact bit power.
    let p = if d < -1022 {
        f64::from_bits(1u64 << (d + 1074))
    } else {
        2f64.powi(d)
    };
    let v = if d < -1022 { a.f * p } else { v };
    edge(v, up)
}
fn add(a: Extended, b: Extended) -> Interval {
    if a.f == 0. {
        return Interval { lo: b, hi: b };
    }
    if b.f == 0. {
        return Interval { lo: a, hi: a };
    }
    if a.e == b.e && a.f == -b.f {
        return Interval {
            lo: norm(0., 0),
            hi: norm(0., 0),
        };
    }
    let e = a.e.max(b.e);
    let lo = aligned(a, e, false) + aligned(b, e, false);
    let hi = aligned(a, e, true) + aligned(b, e, true);
    Interval {
        lo: norm(edge(lo, false), e),
        hi: norm(edge(hi, true), e),
    }
}
fn mul(a: Extended, b: Extended) -> Interval {
    if a.f == 0. || b.f == 0. {
        return Interval {
            lo: norm(0., 0),
            hi: norm(0., 0),
        };
    }
    let e = a.e + b.e;
    let f = a.f * b.f;
    Interval {
        lo: norm(edge(f, false), e),
        hi: norm(edge(f, true), e),
    }
}
fn div(a: Extended, b: Extended) -> Interval {
    if a.f == 0. {
        return Interval {
            lo: norm(0., 0),
            hi: norm(0., 0),
        };
    }
    let e = a.e - b.e;
    let f = a.f / b.f;
    Interval {
        lo: norm(edge(f, false), e),
        hi: norm(edge(f, true), e),
    }
}
fn sqrt(a: Extended) -> Interval {
    if a.f == 0. {
        return Interval { lo: a, hi: a };
    }
    let e = a.e.div_euclid(2);
    let f = (a.f * if a.e.rem_euclid(2) == 1 { 2. } else { 1. }).sqrt();
    Interval {
        lo: norm(edge(f, false), e),
        hi: norm(edge(f, true), e),
    }
}
pub(super) fn abs_cmp(a: Extended, b: Extended) -> std::cmp::Ordering {
    if a.f == 0. {
        return if b.f == 0. {
            std::cmp::Ordering::Equal
        } else {
            std::cmp::Ordering::Less
        };
    }
    if b.f == 0. {
        return std::cmp::Ordering::Greater;
    }
    a.e.cmp(&b.e).then(a.f.abs().total_cmp(&b.f.abs()))
}
fn scalar(a: Extended) -> Option<f64> {
    if a.f == 0. {
        return Some(0.);
    }
    if a.e > 1023 || a.e < -1075 {
        return None;
    }
    let v = if a.e < -1022 {
        (a.f * 2f64.powi(a.e + 1022)) * f64::MIN_POSITIVE
    } else {
        a.f * 2f64.powi(a.e)
    };
    (v.is_finite() && v != 0.).then_some(v)
}
fn publication(a: Interval) -> Option<f64> {
    let center = add(a.lo, a.hi).lo;
    let center = norm(center.f, center.e - 1);
    let value = scalar(center)?;
    let v = norm(value, 0);
    let minus = Extended { f: -v.f, e: v.e };
    let left = add(a.lo, minus);
    let right = add(a.hi, minus);
    let mut error = norm(0., 0);
    for candidate in [left.lo, left.hi, right.lo, right.hi] {
        if abs_cmp(candidate, error).is_gt() {
            error = norm(candidate.f.abs(), candidate.e);
        }
    }
    let magnitude = if abs_cmp(v, norm(1., 0)).is_gt() {
        norm(v.f.abs(), v.e)
    } else {
        norm(1., 0)
    };
    let target = Extended {
        f: magnitude.f,
        e: magnitude.e - 40,
    };
    (!abs_cmp(error, target).is_gt()).then_some(value)
}
fn cmp(a: Extended, b: Extended) -> std::cmp::Ordering {
    if a.f >= 0. && b.f < 0. {
        return std::cmp::Ordering::Greater;
    }
    if a.f < 0. && b.f >= 0. {
        return std::cmp::Ordering::Less;
    }
    let c = abs_cmp(a, b);
    if a.f < 0. {
        c.reverse()
    } else {
        c
    }
}
fn lower(a: Extended, b: Extended) -> Extended {
    if cmp(a, b).is_le() {
        a
    } else {
        b
    }
}
fn upper(a: Extended, b: Extended) -> Extended {
    if cmp(a, b).is_ge() {
        a
    } else {
        b
    }
}
pub(super) fn iadd(a: Interval, b: Interval) -> Interval {
    Interval {
        lo: add(a.lo, b.lo).lo,
        hi: add(a.hi, b.hi).hi,
    }
}
pub(super) fn ineg(a: Interval) -> Interval {
    Interval {
        lo: Extended {
            f: -a.hi.f,
            e: a.hi.e,
        },
        hi: Extended {
            f: -a.lo.f,
            e: a.lo.e,
        },
    }
}
pub(super) fn imul(a: Interval, b: Interval) -> Interval {
    let v = [
        mul(a.lo, b.lo),
        mul(a.lo, b.hi),
        mul(a.hi, b.lo),
        mul(a.hi, b.hi),
    ];
    Interval {
        lo: lower(lower(v[0].lo, v[1].lo), lower(v[2].lo, v[3].lo)),
        hi: upper(upper(v[0].hi, v[1].hi), upper(v[2].hi, v[3].hi)),
    }
}
pub(super) fn idiv(a: Interval, b: Interval) -> Interval {
    imul(
        a,
        Interval {
            lo: div(norm(1., 0), b.hi).lo,
            hi: div(norm(1., 0), b.lo).hi,
        },
    )
}
pub(super) fn isqrt(a: Interval) -> Interval {
    Interval {
        lo: sqrt(a.lo).lo,
        hi: sqrt(a.hi).hi,
    }
}
pub(super) fn exact(x: f64) -> Interval {
    Interval {
        lo: norm(x, 0),
        hi: norm(x, 0),
    }
}
#[derive(Default)]
pub(super) struct Pair {
    lo: analysis::moments::BoundedSum,
    hi: analysis::moments::BoundedSum,
}
fn endpoint(a: Extended, up: bool) -> AppResult<f64> {
    if !a.f.is_finite() || a.e > 1023 {
        return Err(invariant());
    }
    // Outward Float64 conversion for normalized formed terms. No silent underflow.
    if a.f == 0. {
        return Ok(0.);
    }
    if a.e < -1075 {
        return Ok(if a.f > 0. {
            if up {
                f64::from_bits(1)
            } else {
                0.
            }
        } else if up {
            0.
        } else {
            -f64::from_bits(1)
        });
    }
    let v = if a.e < -1022 {
        (a.f * 2f64.powi(a.e + 1022)) * f64::MIN_POSITIVE
    } else {
        a.f * 2f64.powi(a.e)
    };
    let result = edge(v, up);
    if result.is_finite() {
        Ok(result)
    } else {
        Err(invariant())
    }
}
impl Pair {
    pub(super) fn push_weighted(&mut self, a: Interval, w: u64) -> AppResult<()> {
        self.lo.push(endpoint(a.lo, false)?, w)?;
        self.hi.push(endpoint(a.hi, true)?, w)?;
        Ok(())
    }
    pub(super) fn value(&self, shift: i32) -> AppResult<Interval> {
        let l = self.lo.scaled(1)?;
        let h = self.hi.scaled(1)?;
        Ok(Interval {
            lo: if l.fraction == 0. {
                norm(0., 0)
            } else {
                norm(l.fraction.next_down(), l.exponent + shift)
            },
            hi: if h.fraction == 0. {
                norm(0., 0)
            } else {
                norm(h.fraction.next_up(), h.exponent + shift)
            },
        })
    }
}
pub(super) fn isquare(a: Interval) -> Interval {
    if a.lo.f <= 0. && a.hi.f >= 0. {
        Interval {
            lo: norm(0., 0),
            hi: upper(mul(a.lo, a.lo).hi, mul(a.hi, a.hi).hi),
        }
    } else {
        Interval {
            lo: lower(mul(a.lo, a.lo).lo, mul(a.hi, a.hi).lo),
            hi: upper(mul(a.lo, a.lo).hi, mul(a.hi, a.hi).hi),
        }
    }
}
pub(super) fn interval_of(c: analysis::moments::Scaled, e: i32) -> Interval {
    if c.fraction == 0. {
        return exact(0.);
    }
    let center = norm(c.fraction, c.exponent - e);
    let radius = mul(norm(center.f.abs(), center.e), norm(3. * 2f64.powi(-53), 0)).hi;
    Interval {
        lo: add(
            center,
            Extended {
                f: -radius.f,
                e: radius.e,
            },
        )
        .lo,
        hi: add(center, radius).hi,
    }
}
fn estimate(a: Interval, range: Option<(f64, f64)>) -> Estimate {
    let Some(v) = publication(a) else {
        return Estimate::unavailable("numerical or scalar publication unresolved");
    };
    if range.is_some_and(|(lo, hi)| v < lo || v > hi) {
        return Estimate::unavailable("range violation; no clipping");
    }
    Estimate::available(v)
}
pub(super) fn energy_state(saw: bool, a: Interval) -> ReliabilityEnergyState {
    if !saw {
        ReliabilityEnergyState::ExactZero
    } else if a.lo.f > 0. {
        ReliabilityEnergyState::Positive
    } else {
        ReliabilityEnergyState::Unresolved
    }
}
pub(super) fn publish_alpha(
    a: Interval,
    b: Interval,
    k: usize,
    state: ReliabilityEnergyState,
) -> Estimate {
    if k < 2 {
        return Estimate::unavailable("k<2");
    }
    if state != ReliabilityEnergyState::Positive {
        return Estimate::unavailable(if state == ReliabilityEnergyState::ExactZero {
            "ExactZero denominator"
        } else if state == ReliabilityEnergyState::Unavailable {
            "Unavailable denominator"
        } else {
            "Unresolved denominator"
        });
    }
    estimate(
        imul(
            idiv(exact(k as f64), exact((k - 1) as f64)),
            iadd(exact(1.), ineg(idiv(a, b))),
        ),
        Some((f64::NEG_INFINITY, 1.)),
    )
}

pub(super) fn publish_mean(moments: &Moments) -> Estimate {
    moments
        .mean()
        .map(Estimate::available)
        .unwrap_or_else(|| Estimate::unavailable("N=0"))
}
pub(super) fn publish_sd(scatter: Interval, n: u64, state: ReliabilityEnergyState) -> Estimate {
    if n < 2 || state == ReliabilityEnergyState::Unavailable {
        return Estimate::unavailable("N<2");
    }
    if state == ReliabilityEnergyState::ExactZero {
        return Estimate::available(0.);
    }
    if state != ReliabilityEnergyState::Positive {
        return Estimate::unavailable("Unresolved item energy");
    }
    estimate(isqrt(idiv(scatter, exact((n - 1) as f64))), None)
}
pub(super) fn publish_correlation(
    covariance: Interval,
    item: Interval,
    remaining: Interval,
    item_state: ReliabilityEnergyState,
    remaining_state: ReliabilityEnergyState,
    exact_two_point_sign: Option<bool>,
) -> Estimate {
    if item_state != ReliabilityEnergyState::Positive
        || remaining_state != ReliabilityEnergyState::Positive
    {
        return Estimate::unavailable("zero/unresolved variance");
    }
    if let Some(sign) = exact_two_point_sign {
        return Estimate::available(if sign { 1. } else { -1. });
    }
    estimate(
        idiv(covariance, isqrt(imul(item, remaining))),
        Some((-1., 1.)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoint_invariant_refuses_typed_without_panic() {
        let invalid = Interval {
            lo: Extended { f: 1., e: 1024 },
            hi: Extended { f: 1., e: 1024 },
        };
        let mut sum = Pair::default();
        assert!(matches!(
            sum.push_weighted(invalid, 1),
            Err(AppError::InvalidInput { .. })
        ));
        let invalid = Interval {
            lo: Extended {
                f: f64::INFINITY,
                e: 0,
            },
            hi: Extended { f: 1., e: 0 },
        };
        assert!(sum.push_weighted(invalid, 1).is_err());
    }
}
