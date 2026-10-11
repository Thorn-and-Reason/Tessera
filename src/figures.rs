//! Presentation settings are independent of captured statistical results.
use crate::{
    analysis::{CompletedAnalysis, RegressionOutcome},
    error::{AppError, AppResult},
    project::AnalysisRun,
};
use serde::{Deserialize, Serialize};
pub(crate) const SVG_WORK_BYTES: usize = 32 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FigureKind {
    Histogram,
    Scatter,
    Boxplot,
    Residual,
    Qq,
    Distribution(u64),
    Frequency(u64),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FigureKey {
    pub run: u64,
    pub kind: FigureKind,
    pub variant: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FigurePalette {
    #[default]
    Report,
    Monochrome,
    Accessible,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FrequencyMetric {
    #[default]
    Count,
    ValidPercent,
    AllPercent,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CategoryOrder {
    #[default]
    Original,
    Value,
    Frequency,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FigureSettings {
    pub title: String,
    pub subtitle: String,
    pub x_label: String,
    pub y_label: String,
    pub caption: String,
    pub palette: FigurePalette,
    pub width_mm: f64,
    pub height_mm: f64,
    pub dpi: u32,
    pub font_size: f64,
    pub grid: bool,
    pub marker_size: f64,
    pub opacity: f64,
    pub transparent: bool,
    pub x_bounds: Option<[f64; 2]>,
    pub y_bounds: Option<[f64; 2]>,
    pub ticks: u8,
    pub decimals: u8,
    pub horizontal: bool,
    pub metric: FrequencyMetric,
    pub order: CategoryOrder,
    pub value_labels: bool,
}
impl Default for FigureSettings {
    fn default() -> Self {
        Self {
            title: String::new(),
            subtitle: String::new(),
            x_label: String::new(),
            y_label: String::new(),
            caption: String::new(),
            palette: FigurePalette::Report,
            width_mm: 160.,
            height_mm: 100.,
            dpi: 300,
            font_size: 12.,
            grid: true,
            marker_size: 3.,
            opacity: 0.8,
            transparent: false,
            x_bounds: None,
            y_bounds: None,
            ticks: 5,
            decimals: 2,
            horizontal: true,
            metric: FrequencyMetric::Count,
            order: CategoryOrder::Original,
            value_labels: true,
        }
    }
}
impl FigureSettings {
    pub fn validate(&self) -> AppResult<()> {
        if [
            &self.title,
            &self.subtitle,
            &self.x_label,
            &self.y_label,
            &self.caption,
        ]
        .iter()
        .any(|s| s.len() > 4096 || s.chars().any(|c| c.is_control() && c != '\n'))
        {
            return Err(AppError::invalid(
                "figure text",
                "Use at most 4096 bytes of printable text per field.",
            ));
        }
        for (value, low, high) in [
            (self.width_mm, 40., 500.),
            (self.height_mm, 40., 500.),
            (self.font_size, 6., 36.),
            (self.marker_size, 0.5, 20.),
            (self.opacity, 0., 1.),
        ] {
            if !value.is_finite() || !(low..=high).contains(&value) {
                return Err(AppError::invalid(
                    "figure settings",
                    "A figure size or appearance value is outside its supported range.",
                ));
            }
        }
        if !(72..=600).contains(&self.dpi) || !(2..=12).contains(&self.ticks) || self.decimals > 8 {
            return Err(AppError::invalid(
                "figure settings",
                "Choose 72–600 dpi, 2–12 ticks and at most 8 decimal places.",
            ));
        }
        for bounds in [self.x_bounds, self.y_bounds].into_iter().flatten() {
            if !bounds.iter().all(|n| n.is_finite()) || bounds[0] >= bounds[1] {
                return Err(AppError::invalid(
                    "axis limits",
                    "Axis limits must be finite and increase.",
                ));
            }
        }
        self.checked_pixels().map(|_| ())
    }
    fn checked_pixels(&self) -> AppResult<(u32, u32)> {
        let w = (self.width_mm * self.dpi as f64 / 25.4).round();
        let h = (self.height_mm * self.dpi as f64 / 25.4).round();
        if !w.is_finite() || !h.is_finite() || w < 1. || h < 1. || w * h > 40_000_000. {
            return Err(AppError::invalid(
                "figure size",
                "The figure must contain at most 40 million pixels.",
            ));
        }
        Ok((w as u32, h as u32))
    }
    pub fn pixel_size(&self) -> AppResult<(u32, u32)> {
        self.validate()?;
        self.checked_pixels()
    }
    pub fn copy_style_from(&self, other: &Self) -> Self {
        Self {
            title: self.title.clone(),
            subtitle: self.subtitle.clone(),
            caption: self.caption.clone(),
            x_label: self.x_label.clone(),
            y_label: self.y_label.clone(),
            x_bounds: self.x_bounds,
            y_bounds: self.y_bounds,
            metric: self.metric,
            order: self.order,
            horizontal: self.horizontal,
            ..other.clone()
        }
    }
    pub(crate) fn heap_bytes(&self) -> usize {
        [
            &self.title,
            &self.subtitle,
            &self.x_label,
            &self.y_label,
            &self.caption,
        ]
        .iter()
        .map(|s| s.capacity())
        .sum()
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FigureRecord {
    pub key: FigureKey,
    pub settings: FigureSettings,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FigurePreset {
    pub name: String,
    pub settings: FigureSettings,
}
pub fn figure_keys(run: &AnalysisRun) -> Vec<FigureKey> {
    let kinds = match &run.analysis {
        CompletedAnalysis::Histogram(_) => vec![FigureKind::Histogram],
        CompletedAnalysis::Scatter(_) => vec![FigureKind::Scatter],
        CompletedAnalysis::Boxplot(_) => vec![FigureKind::Boxplot],
        CompletedAnalysis::Describe(a) => a
            .plots
            .iter()
            .map(|p| FigureKind::Distribution(p.column.0))
            .collect(),
        CompletedAnalysis::Frequencies(a) => a
            .tables
            .iter()
            .map(|p| FigureKind::Frequency(p.column.0))
            .collect(),
        CompletedAnalysis::Regression(a) => match &a.outcome {
            RegressionOutcome::Fitted(f) => {
                let mut k = vec![];
                if !f.residual_diagnostics.residual_points.is_empty() {
                    k.push(FigureKind::Residual);
                }
                if !f.residual_diagnostics.qq_points.is_empty() {
                    k.push(FigureKind::Qq);
                }
                k
            }
            _ => vec![],
        },
        _ => vec![],
    };
    kinds
        .into_iter()
        .map(|kind| FigureKey {
            run: run.id.0,
            kind,
            variant: 0,
        })
        .collect()
}

mod data;
pub mod render;
