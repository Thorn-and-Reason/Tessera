//! Original-unit transformations retain exponents until each scalar is published.
use super::*;
use crate::analysis::{inference::arithmetic::TailFloat as D, moments::Scaled, Estimate};

#[derive(Clone, Copy)]
pub(in crate::analysis::regression) struct Scale {
    pub anchor: f64,
    pub delta: f64,
    pub mean: f64,
    pub sd: f64,
}
impl Scale {
    pub fn at(self, value: f64) -> f64 {
        ((value - self.anchor) / self.delta - self.mean) / self.sd
    }
    pub fn unit(self) -> Scaled {
        multiply(Scaled::new(self.delta), Scaled::new(self.sd))
    }
    pub fn centre(self) -> Scaled {
        Scaled::new(self.anchor).plus(multiply(Scaled::new(self.delta), Scaled::new(self.mean)))
    }
}
pub(in crate::analysis::regression) fn multiply(a: Scaled, b: Scaled) -> Scaled {
    let part = Scaled::new(a.fraction * b.fraction);
    Scaled {
        fraction: part.fraction,
        exponent: a.exponent + b.exponent + part.exponent,
    }
}
pub(super) fn norm(
    values: impl Iterator<Item = Scaled>,
    ck: &mut Check<'_>,
) -> Result<Scaled, Abort> {
    let mut total = Scaled::default();
    for value in values {
        ck.tick()?;
        if total.fraction == 0. {
            total = Scaled {
                fraction: value.fraction.abs(),
                ..value
            };
            continue;
        }
        if value.fraction == 0. {
            continue;
        }
        let exponent = total.exponent.max(value.exponent);
        let part = Scaled::new(
            (total.fraction * 2f64.powi(total.exponent - exponent))
                .hypot(value.fraction * 2f64.powi(value.exponent - exponent)),
        );
        total = Scaled {
            fraction: part.fraction,
            exponent: exponent + part.exponent,
        };
    }
    Ok(total)
}
pub(super) fn sum(
    values: impl Iterator<Item = Scaled> + Clone,
    ck: &mut Check<'_>,
) -> Result<Scaled, Abort> {
    let mut exponent = None;
    for value in values.clone() {
        ck.tick()?;
        if value.fraction != 0. {
            exponent = Some(exponent.unwrap_or(value.exponent).max(value.exponent));
        }
    }
    let exponent = exponent.unwrap_or(0);
    let mut total = D(0., 0.);
    for value in values {
        ck.tick()?;
        total = total.add(D(value.fraction * 2f64.powi(value.exponent - exponent), 0.));
    }
    let part = Scaled::new(total.0 + total.1);
    Ok(Scaled {
        fraction: part.fraction,
        exponent: exponent + part.exponent,
    })
}
pub(in crate::analysis::regression) fn estimate(value: Scaled) -> Estimate {
    let v = value.value();
    if !v.is_finite() {
        Estimate::unavailable("The estimate is outside finite Float64 range.")
    } else if v == 0. && value.fraction != 0. {
        Estimate::unavailable("A nonzero estimate is below Float64 range.")
    } else {
        Estimate::available(v)
    }
}
pub(in crate::analysis::regression) fn log(value: Scaled) -> D {
    D(value.fraction.abs(), 0.)
        .ln()
        .add(D::LN_2.mul(D(value.exponent as f64, 0.)))
}
pub(super) fn scale(
    input: &Input<'_>,
    term: Option<&RegressionTerm>,
    ck: &mut Check<'_>,
) -> Result<Scale, Abort> {
    let mut first = None;
    for record in input.eligible {
        ck.tick()?;
        if record.complete {
            first = Some(record);
            break;
        }
    }
    let first = first.ok_or(Abort::Numerical(
        "No complete positive-frequency observations remain.",
    ))?;
    let anchor = input.value(first.row, term, ck)?;
    let mut delta = 0f64;
    for r in input.eligible {
        ck.tick()?;
        if !r.complete {
            continue;
        }
        let d = input.value(r.row, term, ck)? - anchor;
        if !d.is_finite() {
            return Err(Abort::Numerical(
                "Finite input differences exceed the supported Float64 centring range.",
            ));
        }
        delta = delta.max(d.abs());
    }
    if delta == 0. {
        return Ok(Scale {
            anchor,
            delta: 1.,
            mean: 0.,
            sd: 1.,
        });
    }
    let mut mean = Sum::default();
    for r in input.eligible {
        ck.tick()?;
        if !r.complete {
            continue;
        }
        mean.add(
            (input.value(r.row, term, ck)? - anchor) / delta * (r.weight as f64 / input.n as f64),
        );
    }
    let mean = mean.value();
    let mut sd = 0f64;
    for r in input.eligible {
        ck.tick()?;
        if !r.complete {
            continue;
        }
        sd = sd.hypot(
            ((input.value(r.row, term, ck)? - anchor) / delta - mean)
                * (r.weight as f64 / input.n as f64).sqrt(),
        );
    }
    if !sd.is_finite() || sd == 0. {
        return Err(Abort::Numerical(
            "The scaled column spread is not positive and finite.",
        ));
    }
    Ok(Scale {
        anchor,
        delta,
        mean,
        sd,
    })
}
