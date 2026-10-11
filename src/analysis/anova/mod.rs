//! Captured ANOVA contracts and bounded cohort admission. Hidden from the chooser.
mod accounting;
pub(crate) mod arithmetic;
pub(crate) mod cohort;
pub(crate) mod independent;
mod model;
pub(crate) mod solver;
pub(crate) mod validation;
pub use model::*;
pub const MAX_GRID: usize = 64;
pub const MAX_REQUEST_BYTES: usize = 512 * 1024;

use crate::{
    analysis::{model::checked_sum, support},
    data::{Budget, Cell},
    error::AppResult,
    operation::JobControl,
};
/// Caller precharges a second request owner before this controlled copy.
pub(crate) fn capture_request(r: &AnovaRequest, control: &JobControl) -> AppResult<AnovaRequest> {
    control.check_cancelled()?;
    r.admit()?;
    Budget::check_datasets(
        &[],
        checked_sum([std::mem::size_of::<AnovaRequest>(), r.heap_bytes()?])?,
    )?;
    let design = match &r.design {
        AnovaDesign::OneWay { outcome, factor } => AnovaDesign::OneWay {
            outcome: *outcome,
            factor: *factor,
        },
        AnovaDesign::Factorial { outcome, factors } => AnovaDesign::Factorial {
            outcome: *outcome,
            factors: factors.clone(),
        },
        AnovaDesign::Repeated {
            subject,
            factors,
            measurements,
        } => {
            let mut copied = Vec::with_capacity(factors.len());
            for f in factors {
                control.checkpoint(0, None)?;
                let mut levels = Vec::with_capacity(f.levels.len());
                for s in &f.levels {
                    levels.push(support::clone_text(s, control)?);
                }
                copied.push(AnovaWithinFactor {
                    name: support::clone_text(&f.name, control)?,
                    levels,
                });
            }
            AnovaDesign::Repeated {
                subject: *subject,
                factors: copied,
                measurements: measurements.clone(),
            }
        }
    };
    let mut planned_pairs = Vec::with_capacity(r.planned_pairs.len());
    for p in &r.planned_pairs {
        control.checkpoint(0, None)?;
        planned_pairs.push(AnovaPairRequest {
            factor: p.factor,
            first: support::clone_cell(&p.first, control)?,
            second: support::clone_cell(&p.second, control)?,
            scope: clone_scope(&p.scope, control)?,
        });
    }
    let mut all_pairs = Vec::with_capacity(r.all_pairs.len());
    for f in &r.all_pairs {
        control.checkpoint(0, None)?;
        all_pairs.push(AnovaFamilyRequest {
            factor: f.factor,
            scope: clone_scope(&f.scope, control)?,
        });
    }
    Ok(AnovaRequest {
        design,
        confidence: r.confidence,
        filter: support::clone_filter(&r.filter, control)?,
        frequency_weight: r.frequency_weight,
        planned_pairs,
        all_pairs,
    })
}
fn clone_scope(scope: &AnovaScope, control: &JobControl) -> AppResult<AnovaScope> {
    Ok(match scope {
        AnovaScope::Marginal => AnovaScope::Marginal,
        AnovaScope::Fixed(values) => {
            let mut copied = Vec::with_capacity(values.len());
            for v in values {
                control.checkpoint(0, None)?;
                copied.push(match v {
                    Cell::Text(s) => Cell::Text(support::clone_text(s, control)?),
                    v => v.clone(),
                });
            }
            AnovaScope::Fixed(copied)
        }
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        analysis::{ConfidenceLevel, RowFilter},
        data::ColumnId,
        error::AppError,
        operation::CHECKPOINT_HOOK,
    };
    #[test]
    fn captured_request_preserves_exact_text_and_cancels() {
        let r = AnovaRequest {
            design: AnovaDesign::OneWay {
                outcome: ColumnId(1),
                factor: ColumnId(2),
            },
            confidence: ConfidenceLevel::P95,
            filter: RowFilter::default(),
            frequency_weight: None,
            planned_pairs: vec![AnovaPairRequest {
                factor: 0,
                first: Cell::Text(" ŵ ".repeat(20000)),
                second: Cell::Text("é".into()),
                scope: AnovaScope::Marginal,
            }],
            all_pairs: vec![],
        };
        let copy = capture_request(&r, &JobControl::default()).unwrap();
        assert_eq!(copy, r);
        let c = JobControl::default();
        let cancel = c.clone();
        CHECKPOINT_HOOK.with(|h| {
            *h.borrow_mut() = Some(Box::new(move |p| {
                if p.done >= 8192 {
                    cancel.cancel()
                }
            }))
        });
        let result = capture_request(&r, &c);
        CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
        assert!(matches!(result, Err(AppError::Cancelled)));
    }
}

#[cfg(test)]
mod scalar_inference;
pub(crate) mod tails;

pub(crate) mod covariance;

pub(crate) mod repeated;

pub(crate) mod contrasts;

pub(crate) mod compute;
pub use compute::anova_with_budget;
