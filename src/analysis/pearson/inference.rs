use crate::analysis::{
    inference::{arithmetic::TailFloat as D, student_t_two_sided_p},
    moments::{BoundedSum, Scaled},
    Estimate,
};
use crate::{error::AppResult, operation::JobControl};

pub(super) const UNRESOLVED: &str = "Pearson inference exceeds its numerical precision or range.";

pub(super) struct Inference {
    ranges: [Range; 2],
    endpoint: [D; 2],
    means: [D; 4],
    xx: D,
    xy: D,
    rr: D,
    xr: D,
    valid: bool,
    affine: bool,
}

impl Inference {
    pub(super) fn new(bounds: [[f64; 2]; 2], anchor: [f64; 2], endpoint: [f64; 2]) -> Self {
        let ranges = [
            Range::new(bounds[0], anchor[0]),
            Range::new(bounds[1], anchor[1]),
        ];
        let differences = [
            ranges[0].difference(endpoint[0]),
            ranges[1].difference(endpoint[1]),
        ];
        Self {
            ranges,
            endpoint: differences.map(|d| d.unwrap_or(D(0., 0.))),
            means: [D(0., 0.); 4],
            xx: D(0., 0.),
            xy: D(0., 0.),
            rr: D(0., 0.),
            xr: D(0., 0.),
            valid: true,
            affine: differences.iter().all(Option::is_some),
        }
    }
    fn coordinates(&mut self, a: f64, b: f64) -> [D; 2] {
        match (self.ranges[0].at(a), self.ranges[1].at(b)) {
            (Some(x), Some(y)) => [x, y],
            _ => {
                self.valid = false;
                [D(0., 0.); 2]
            }
        }
    }
    pub(super) fn add_means(&mut self, a: f64, b: f64, weight: u64) {
        let [x, y] = self.coordinates(a, b);
        for (sum, value) in self.means.iter_mut().zip([x, y, y.sub(x), y.add(x)]) {
            *sum = sum.add(value.mul(D(weight as f64, 0.)));
        }
    }
    pub(super) fn center(&mut self, n: u64) {
        for mean in &mut self.means {
            *mean = mean.div(D(n.max(1) as f64, 0.));
        }
    }
    pub(super) fn add_cross(&mut self, a: f64, b: f64, weight: u64) {
        let [x, y] = self.coordinates(a, b);
        let (x, y) = (x.sub(self.means[0]), y.sub(self.means[1]));
        let w = D(weight as f64, 0.);
        self.xx = self.xx.add(x.mul(x).mul(w));
        self.xy = self.xy.add(x.mul(y).mul(w));
    }
    pub(super) fn add_residual(&mut self, a: f64, b: f64, weight: u64) -> AppResult<()> {
        if self.affine {
            let affine = match (self.ranges[0].difference(a), self.ranges[1].difference(b)) {
                (Some(x), Some(y)) => {
                    exact_determinant_zero(x, self.endpoint[1], y, self.endpoint[0])?
                }
                _ => None,
            };
            self.affine = affine == Some(true);
            self.valid &= affine.is_some();
        }
        let [x, y] = self.coordinates(a, b);
        let residual = if self.xy.0 >= 0. {
            y.sub(x).sub(self.means[2])
        } else {
            y.add(x).sub(self.means[3])
        };
        let energy = residual.mul(residual);
        if residual.0 != 0. && energy.0 == 0. {
            self.valid = false;
        }
        let w = D(weight as f64, 0.);
        self.rr = self.rr.add(energy.mul(w));
        self.xr = self.xr.add(x.sub(self.means[0]).mul(residual).mul(w));
        Ok(())
    }
    pub(super) fn probability(&self, n: u64, control: &JobControl) -> AppResult<Estimate> {
        if !self.valid {
            return Ok(Estimate::unavailable(UNRESOLVED));
        }
        if self.is_perfect() {
            return Ok(Estimate::available(0.));
        }
        // y = ±x + residual leaves the Gram determinant unchanged, without 1-r² cancellation.
        let determinant = self.xx.mul(self.rr).sub(self.xr.mul(self.xr));
        let determinant = determinant.0 + determinant.1;
        if !determinant.is_finite() || determinant <= 0. {
            return Ok(Estimate::unavailable(UNRESOLVED));
        }
        let t = (self.xy.0 + self.xy.1).abs() / determinant.sqrt() * ((n - 2) as f64).sqrt();
        student_t_two_sided_p(t, (n - 2) as f64, control)
    }
    pub(super) fn is_perfect(&self) -> bool {
        self.valid && self.affine && self.xx.0 > 0.
    }
}

struct Range {
    anchor: D,
    span: D,
    exponent: i32,
}
impl Range {
    fn new([minimum, maximum]: [f64; 2], anchor: f64) -> Self {
        let exponent = Scaled::new(minimum.abs().max(maximum.abs())).exponent;
        let minimum = D(rescale(minimum, exponent), 0.);
        let span = D(rescale(maximum, exponent), 0.).sub(minimum);
        Self {
            anchor: D(rescale(anchor, exponent), 0.),
            span,
            exponent,
        }
    }
    fn difference(&self, value: f64) -> Option<D> {
        let scaled = rescale(value, self.exponent);
        let restored = if self.exponent < -1022 {
            scaled * 2f64.powi(self.exponent + 1022) * f64::MIN_POSITIVE
        } else {
            scaled * 2f64.powi(self.exponent)
        };
        if restored != value {
            return None;
        }
        Some(D(scaled, 0.).sub(self.anchor))
    }
    fn at(&self, value: f64) -> Option<D> {
        let difference = self.difference(value)?;
        if self.span.0 == 0. {
            return Some(D(0., 0.));
        }
        let result = difference.div(self.span);
        (result.0.is_finite() && result.1.is_finite()).then_some(result)
    }
}

fn exact_determinant_zero(a: D, b: D, c: D, d: D) -> AppResult<Option<bool>> {
    let mut sum = BoundedSum::default();
    for (left, right, sign) in [(a, b, 1.), (c, d, -1.)] {
        for x in [left.0, left.1] {
            for y in [right.0, right.1] {
                if x == 0. || y == 0. {
                    continue;
                }
                // FMA is error-free only when every exact product bit fits Float64's exponent range.
                if lowest_bit(x) + lowest_bit(y) < -1074 {
                    return Ok(None);
                }
                let product = x * y;
                sum.push(sign * product, 1)?;
                sum.push(sign * x.mul_add(y, -product), 1)?;
            }
        }
    }
    Ok(Some(sum.scaled(1)?.fraction == 0.))
}

fn lowest_bit(value: f64) -> i32 {
    let bits = value.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let significand = (bits & ((1 << 52) - 1)) | if exponent == 0 { 0 } else { 1 << 52 };
    let unit = if exponent == 0 {
        -1074
    } else {
        exponent - 1075
    };
    unit + significand.trailing_zeros() as i32
}
fn rescale(value: f64, exponent: i32) -> f64 {
    if exponent < -1022 {
        value * 2f64.powi(1022) / 2f64.powi(exponent + 1022)
    } else {
        value / 2f64.powi(exponent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_affine_check_retains_low_components_and_refuses_product_underflow() {
        let one = D(1., 0.);
        assert_eq!(
            exact_determinant_zero(one, one, one, one).unwrap(),
            Some(true)
        );
        assert_eq!(
            exact_determinant_zero(D(1., 1e-300), one, one, one).unwrap(),
            Some(false)
        );
        let tiny = D(f64::from_bits(1), 0.);
        assert_eq!(
            exact_determinant_zero(tiny, tiny, tiny, tiny).unwrap(),
            None
        );
    }
}
