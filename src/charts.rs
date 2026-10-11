//! Finite full-cohort axis transformations shared by SVG and native drawings.
use crate::{analysis::validation::invalid_analysis, error::AppResult};
pub struct ChartScale {
    lo: f64,
    hi: f64,
    span: f64,
    halved: bool,
    precise_ticks: bool,
}
impl ChartScale {
    pub fn new(lo: f64, hi: f64) -> AppResult<Self> {
        if !lo.is_finite() || !hi.is_finite() || lo > hi {
            return Err(invalid_analysis());
        }
        let raw_span = hi - lo;
        let halved = !raw_span.is_finite();
        let span = if halved {
            hi * 0.5 - lo * 0.5
        } else {
            raw_span
        };
        Ok(Self {
            lo,
            hi,
            span,
            halved,
            precise_ticks: lo != hi && compact_axis(lo) == compact_axis(hi),
        })
    }
    /// Tick values remain in data units; positions use the same transform as marks.
    pub fn ticks(&self) -> impl Iterator<Item = AppResult<(f64, f64)>> + '_ {
        let count = if self.lo == self.hi { 1 } else { 5 };
        (0..count).map(move |i| {
            let value = crate::analysis::exploration::numeric::interpolate(self.lo, self.hi, i, 4)?;
            Ok((value, self.position(value)?))
        })
    }
    /// Axis labels are bounded; fine spreads use two precise endpoint labels.
    pub fn precise_ticks(&self) -> bool {
        self.precise_ticks
    }
    pub fn axis_label(&self, value: f64) -> AxisCaption {
        AxisCaption {
            value,
            precise: self.precise_ticks,
        }
    }
    pub fn axis_ticks(&self) -> impl Iterator<Item = AppResult<(f64, f64)>> + '_ {
        let mut previous = None;
        self.ticks().enumerate().filter_map(move |(i, tick)| {
            if self.precise_ticks && i != 0 && i != 4 {
                return None;
            }
            match tick {
                Ok((value, position)) if previous != Some(value) => {
                    previous = Some(value);
                    Some(Ok((value, position)))
                }
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            }
        })
    }
    pub fn position(&self, value: f64) -> AppResult<f64> {
        if !value.is_finite() || value < self.lo || value > self.hi {
            return Err(invalid_analysis());
        }
        if self.lo == self.hi {
            return Ok(0.5);
        }
        let offset = if self.halved {
            value * 0.5 - self.lo * 0.5
        } else {
            value - self.lo
        };
        Ok(offset / self.span)
    }
}

fn compact_axis(value: f64) -> String {
    if value != 0. && (value.abs() < 0.005 || value.abs() >= 1e9) {
        format!("{value:.2e}")
    } else {
        format!("{value:.2}")
    }
}
pub struct AxisCaption {
    value: f64,
    precise: bool,
}
impl std::fmt::Display for AxisCaption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.precise {
            write!(f, "{:.17e}", self.value)
        } else if self.value != 0. && (self.value.abs() < 0.005 || self.value.abs() >= 1e9) {
            write!(f, "{:.2e}", self.value)
        } else {
            write!(f, "{:.2}", self.value)
        }
    }
}

/// Exact typed category identity, including quotes and control whitespace.
/// Formatting can stop early in a bounded native caption or stream into escaped HTML.
pub struct CategoryCaption<'a>(pub &'a crate::data::Cell);
impl std::fmt::Display for CategoryCaption<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use std::fmt::Write;
        match self.0 {
            crate::data::Cell::Number(v) => write!(f, "{v}"),
            crate::data::Cell::Missing => f.write_str("Missing"),
            crate::data::Cell::Text(s) => {
                f.write_char('"')?;
                for ch in s.chars() {
                    for escaped in ch.escape_debug() {
                        f.write_char(escaped)?;
                    }
                }
                f.write_char('"')
            }
        }
    }
}
