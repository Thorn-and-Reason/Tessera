//! Scalar chart roles are located once on run selection, without copying captured text.
use super::*;
use crate::analysis::{CompletedAnalysis, RunColumnInfo};
use std::sync::Weak;
#[derive(Debug, Clone)]
pub(super) struct ChartRoles {
    owner: Weak<AnalysisRun>,
    indices: [Option<usize>; 2],
}
impl DesktopState {
    pub(super) fn prepare_chart_roles(&mut self) {
        let Some(run) = self.selected_run() else {
            self.chart_roles = None;
            return;
        };
        let ids = match &run.analysis {
            CompletedAnalysis::Histogram(v) => [Some(v.request.variable), None],
            CompletedAnalysis::Scatter(v) => [Some(v.request.x), Some(v.request.y)],
            CompletedAnalysis::Boxplot(v) => [Some(v.request.outcome), v.request.group],
            _ => {
                self.chart_roles = None;
                return;
            }
        };
        // Only two scalar indexes are retained. This explicit selection pass never allocates result-sized storage.
        let mut indices = [None, None];
        for (i, c) in run.analysis.columns().iter().enumerate() {
            for role in 0..2 {
                if ids[role] == Some(c.id) {
                    indices[role] = Some(i);
                }
            }
            if (0..2).all(|role| ids[role].is_none() || indices[role].is_some()) {
                break;
            }
        }
        self.chart_roles = Some(ChartRoles {
            owner: Arc::downgrade(run),
            indices,
        });
    }
    pub(crate) fn chart_column<'a>(
        &self,
        run: &'a AnalysisRun,
        id: ColumnId,
    ) -> Option<&'a RunColumnInfo> {
        let roles = self.chart_roles.as_ref()?;
        if !roles
            .owner
            .upgrade()
            .is_some_and(|owner| std::ptr::eq(owner.as_ref(), run))
        {
            return None;
        }
        roles
            .indices
            .iter()
            .flatten()
            .filter_map(|i| run.analysis.columns().get(*i))
            .find(|c| c.id == id)
    }
    pub(crate) fn chart_name<'a>(&self, run: &'a AnalysisRun, id: ColumnId) -> &'a str {
        self.chart_column(run, id)
            .map_or("Unavailable variable", |c| c.name.as_str())
    }
}
