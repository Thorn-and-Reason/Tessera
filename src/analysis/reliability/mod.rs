//! Typed reliability requests and one immutable complete cohort.
mod accounting;
mod cohort;
mod model;
mod scoring;
pub(crate) mod validation;
use crate::{
    analysis::{model::checked_sum, support},
    data::Budget,
    error::AppResult,
    operation::JobControl,
};
pub use cohort::capture_cohort;
pub use model::*;
pub use scoring::score_item;
/// Caller charges the separate copied owner alongside retained inputs.
pub fn capture_request(r: &ReliabilityRequest, c: &JobControl) -> AppResult<ReliabilityRequest> {
    c.check_cancelled()?;
    r.admit_controlled(c)?;
    Budget::check_datasets(
        &[],
        checked_sum([
            2 * std::mem::size_of::<ReliabilityRequest>(),
            r.heap_bytes_controlled(Some(c))?,
            r.heap_bytes_controlled(Some(c))?,
        ])?,
    )?;
    let copied = ReliabilityRequest {
        items: r.items.clone(),
        filter: support::clone_filter(&r.filter, c)?,
        frequency_weight: r.frequency_weight,
        reversals: r.reversals.clone(),
    };
    c.check_cancelled()?;
    Ok(copied)
}

mod compute;
pub(crate) use compute::compute;

#[cfg(test)]
mod numerical_tests;

mod arithmetic;

mod capture;
mod captured_validation;
pub use capture::reliability_with_budget;
