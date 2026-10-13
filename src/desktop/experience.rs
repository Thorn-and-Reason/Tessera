//! A presentation preference; never part of the project or statistical recipe.
use super::{
    draft::{AnalysisDraft, Procedure},
    theme, DesktopState, UiAction,
};
use crate::analysis::*;
use eframe::egui;

const MODE_KEY: &str = "tessera.analysis_mode";
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnalysisMode {
    #[default]
    Guided,
    Advanced,
}
impl AnalysisMode {
    pub fn from_preference(value: Option<&str>) -> Self {
        if value == Some("advanced") {
            Self::Advanced
        } else {
            Self::Guided
        }
    }
    pub fn preference_value(self) -> &'static str {
        match self {
            Self::Guided => "guided",
            Self::Advanced => "advanced",
        }
    }
}
pub(crate) fn load_mode(storage: Option<&dyn eframe::Storage>) -> AnalysisMode {
    let value = storage.and_then(|s| s.get_string(MODE_KEY));
    AnalysisMode::from_preference(value.as_deref())
}
pub(crate) fn save_mode(storage: &mut dyn eframe::Storage, mode: AnalysisMode) {
    storage.set_string(MODE_KEY, mode.preference_value().into());
}
pub(crate) fn mode_switch(ui: &mut egui::Ui, state: &mut DesktopState) {
    let response = egui::ComboBox::from_id_salt("analysis-mode")
        .width(100.)
        .selected_text(match state.analysis_mode() {
            AnalysisMode::Guided => "Guided",
            AnalysisMode::Advanced => "Advanced",
        })
        .show_ui(ui, |ui| mode_choices(ui, state))
        .response;
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Analysis mode")
    });
    super::procedure_forms::record(state, "Analysis mode", &response);
}
pub(crate) fn mode_choices(ui: &mut egui::Ui, state: &mut DesktopState) {
    let mut mode = state.analysis_mode();
    for (choice, label) in [
        (AnalysisMode::Guided, "Guided"),
        (AnalysisMode::Advanced, "Advanced"),
    ] {
        let response = theme::selectable_value(ui, &mut mode, choice, label);
        if response.clicked() {
            ui.close();
        }
        super::procedure_forms::record(state, label, &response);
    }
    if mode != state.analysis_mode() {
        state.act(UiAction::SetAnalysisMode(mode));
    }
}
pub(crate) fn question(family: &str) -> &'static str {
    match family {
        "Summaries" => "Describe my data",
        "Relationships" => "Explore relationships",
        _ => "Compare values or groups",
    }
}
pub(crate) fn task_label(procedure: Procedure) -> &'static str {
    match procedure {
        Procedure::Describe => "Summarise numeric values",
        Procedure::Frequencies => "Count values and categories",
        Procedure::Histogram => "View a distribution",
        Procedure::Boxplot => "Inspect spread and unusual values",
        Procedure::Scatter => "Plot a relationship",
        Procedure::Pearson => "Measure a linear relationship",
        Procedure::OneSampleT => "Compare with a reference value",
        Procedure::PairedT => "Compare paired measurements",
        Procedure::IndependentT => "Compare two groups",
        Procedure::Regression => "Explain a numeric outcome",
        Procedure::Anova => "Compare several means",
        Procedure::Reliability => "Check scale consistency",
    }
}
pub(crate) fn description(procedure: Procedure) -> &'static str {
    match procedure {
        Procedure::Describe => "Summarise numeric values with averages, variation and ranges.",
        Procedure::Frequencies => "Count how often each value or category appears.",
        Procedure::Histogram => "See how numeric values are distributed across intervals.",
        Procedure::Boxplot => {
            "Inspect the middle, spread and outlying values, optionally by group."
        }
        Procedure::Scatter => "Plot two numeric variables to explore their relationship.",
        Procedure::Pearson => {
            "Measure the strength and direction of a linear relationship between numeric variables."
        }
        Procedure::OneSampleT => "Compare a numeric mean with a reference value you choose.",
        Procedure::PairedT => {
            "Compare two measurements from the same people or matched observations."
        }
        Procedure::IndependentT => "Compare a numeric mean between two separate groups.",
        Procedure::Regression => "Model a numeric outcome using numeric or categorical predictors.",
        Procedure::Anova => {
            "Compare means across groups or explicitly mapped repeated measurements."
        }
        Procedure::Reliability => {
            "Assess the internal consistency of a scale made from numeric items."
        }
    }
}
pub(crate) fn procedure_matches(procedure: Procedure, query: &theme::NameQuery) -> bool {
    query.matches(task_label(procedure))
        || query.matches(procedure.label())
        || query.matches(question(procedure.family()))
        || query.matches(description(procedure))
}
pub(crate) fn detailed_settings(
    ui: &mut egui::Ui,
    mode: AnalysisMode,
    id: egui::Id,
    content: impl FnOnce(&mut egui::Ui),
) -> Option<egui::Response> {
    if mode == AnalysisMode::Advanced {
        content(ui);
        None
    } else {
        Some(
            egui::CollapsingHeader::new("More settings")
                .id_salt(id)
                .show(ui, content)
                .header_response,
        )
    }
}
fn confidence_label(level: ConfidenceLevel) -> &'static str {
    match level {
        ConfidenceLevel::P90 => "90%",
        ConfidenceLevel::P95 => "95%",
        ConfidenceLevel::P99 => "99%",
    }
}
pub(crate) fn settings_summary(draft: &AnalysisDraft) -> String {
    let method = match draft {
        AnalysisDraft::Describe(r) => return describe_summary(r),
        AnalysisDraft::Pearson(r) => format!("Missing values: {}", match r.missing_policy {
            CorrelationMissingPolicy::PairwiseComplete => "pairwise complete", CorrelationMissingPolicy::CompleteCases => "complete cases",
        }),
        AnalysisDraft::OneSampleT { confidence, .. } | AnalysisDraft::PairedT { confidence, .. } => format!("{} confidence interval · two-sided inference", confidence_label(*confidence)),
        AnalysisDraft::IndependentT { confidence, method, .. } => format!("{} confidence interval · {} · two-sided inference", confidence_label(*confidence), match method { IndependentMethod::Welch => "Welch", IndependentMethod::Pooled => "Pooled equal variance" }),
        AnalysisDraft::Histogram { bins, .. } => match bins { HistogramBins::Automatic => "Automatic bins (Sturges)".into(), HistogramBins::Count(n) => format!("{n} bins") },
        AnalysisDraft::Regression(r) => format!("{} confidence interval · intercept included · complete cases", confidence_label(r.confidence)),
        AnalysisDraft::Anova(r) => format!("{} confidence interval · complete cases · {} planned comparisons · {} all-pairs families", confidence_label(r.confidence), r.planned_pairs.len(), r.all_pairs.len()),
        AnalysisDraft::Reliability(r) => format!("{} items · {} reverse-scored items · complete cases", r.items.len(), r.reversals.len()),
        _ => "Complete eligible observations".into(),
    };
    let (filter, weight) = draft.common();
    common_summary(method, filter, weight)
}
pub(crate) fn describe_summary(r: &DescribeRequest) -> String {
    common_summary(
        format!(
            "{} statistics · Missing values: {}",
            r.statistics.len(),
            match r.missing_policy {
                MissingPolicy::PerVariable => "per variable",
                MissingPolicy::CompleteCase => "complete cases",
            }
        ),
        &r.filter,
        r.frequency_weight,
    )
}
fn common_summary(
    method: String,
    filter: &RowFilter,
    weight: Option<crate::data::ColumnId>,
) -> String {
    let weighting = weight.map_or_else(
        || "Unweighted observations".into(),
        |id| {
            format!(
                "Frequency weight: variable {} · repeated observations",
                id.0
            )
        },
    );
    format!(
        "{method}\n{weighting} · {} filter conditions",
        filter.predicates.len()
    )
}
pub(crate) fn summary(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    text: String,
    weight: Option<crate::data::ColumnId>,
) {
    ui.small(text);
    if let Some(id) = weight {
        let name = state.project().dataset().column(id).map_or_else(
            |_| "Unavailable variable".into(),
            |c| theme::caption(c.name()),
        );
        ui.small(format!("Weight variable: {name}. Frequency weights represent repeated observations, not survey weights."));
    }
    if weight.is_some()
        && state.analysis_mode() == AnalysisMode::Guided
        && super::button(ui, state, "Open advanced settings").clicked()
    {
        state.act(UiAction::SetAnalysisMode(AnalysisMode::Advanced));
    }
}
pub(crate) fn reading_aid(request: AnalysisRequestRef<'_>) -> &'static str {
    match request {
        AnalysisRequestRef::Describe(_) => "The mean is the arithmetic average. Sample standard deviation describes spread. Valid N counts included observations; missing and excluded counts explain what was left out.",
        AnalysisRequestRef::Frequencies(_) => "Counts show how often each exact value appears. Check valid and missing counts when interpreting percentages. Frequency weights count repeated observations.",
        AnalysisRequestRef::Histogram(_) => "Each bar counts values in an interval. Bin settings change the picture of the distribution. Check the sample and missing counts alongside the chart.",
        AnalysisRequestRef::Scatter(_) => "Each point shows a pair of values. A pattern can suggest a relationship; it does not establish causation. Drawing limits are reported beside the chart.",
        AnalysisRequestRef::Boxplot(_) => "The box shows the middle half of values and the line marks the median. Points beyond the whiskers follow the stated outlier rule; they are not automatically errors.",
        AnalysisRequestRef::Pearson(_) => "Pearson r measures a linear relationship, from −1 to +1. Its sign shows direction; a value near zero does not rule out a nonlinear relationship. Association does not establish causation. Check pair-specific sample counts and the missing-data method.",
        AnalysisRequestRef::OneSampleT(_) => "The difference is the sample mean minus the reference mean. The confidence interval describes uncertainty under the method's assumptions. Inference is two-sided; a large p-value does not prove no difference.",
        AnalysisRequestRef::PairedT(_) => "The difference is First minus Second for complete pairs from the same people or matched observations. The confidence interval describes uncertainty under the method's assumptions; a large p-value does not prove no difference.",
        AnalysisRequestRef::IndependentT(_) => "The difference is First group minus Second group. These must be separate groups. Welch allows unequal variances; pooled inference assumes equal variances. A large p-value does not prove no difference.",
        AnalysisRequestRef::Regression(_) => "Each coefficient describes an outcome change holding the other predictors fixed. Categorical coefficients compare with the saved reference level. Check uncertainty, residual diagnostics and unavailable-fit reasons. A fitted relationship does not establish causation.",
        AnalysisRequestRef::Anova(_) => "ANOVA tests differences in means for the saved design. Check the fitted effects, complete-case sample and repeated-measures corrections. Planned and all-pairs comparisons use their stated multiplicity rules. A large p-value does not prove equal means.",
        AnalysisRequestRef::Reliability(_) => "Alpha describes internal consistency, not validity or unidimensionality. Check scoring, complete cases and item-deletion results.",
    }
}
pub(crate) fn guidance(ui: &mut egui::Ui, mode: AnalysisMode, request: AnalysisRequestRef<'_>) {
    if mode == AnalysisMode::Guided {
        ui.label(reading_aid(request));
    } else {
        ui.collapsing("Reading these results", |ui| {
            ui.label(reading_aid(request));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::Storage;
    #[derive(Default)]
    struct MemoryStorage(std::collections::BTreeMap<String, String>);
    impl eframe::Storage for MemoryStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.into(), value);
        }
        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }
        fn flush(&mut self) {}
    }
    #[test]
    fn preferences_roundtrip_and_unknown_values_default_to_guided() {
        let mut storage = MemoryStorage::default();
        assert_eq!(load_mode(None), AnalysisMode::Guided);
        assert_eq!(load_mode(Some(&storage)), AnalysisMode::Guided);
        for mode in [AnalysisMode::Advanced, AnalysisMode::Guided] {
            save_mode(&mut storage, mode);
            assert_eq!(load_mode(Some(&storage)), mode);
        }
        storage.set_string(MODE_KEY, "corrupt".into());
        assert_eq!(load_mode(Some(&storage)), AnalysisMode::Guided);
    }
    #[test]
    fn guided_inventory_covers_all_twelve_procedures_without_mutating_drafts() {
        let state = DesktopState::new().unwrap();
        for procedure in Procedure::ALL {
            let draft = state.draft().for_procedure(procedure);
            let before = draft.clone();
            assert!(!description(procedure).is_empty());
            assert!(procedure_matches(
                procedure,
                &theme::NameQuery::new(task_label(procedure)).unwrap()
            ));
            assert!(procedure_matches(
                procedure,
                &theme::NameQuery::new(description(procedure)).unwrap()
            ));
            assert!(!settings_summary(&draft).is_empty());
            assert_eq!(draft, before);
        }
    }
}
