//! Question-led routing opens checked forms without running an analysis.
use super::draft::{AnalysisDraft, Procedure};
use super::{button, DesktopState, UiAction};
use eframe::egui;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Goal {
    #[default]
    Summary,
    Compare,
    Relationship,
    Reference,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Outcome {
    #[default]
    Numeric,
    Category,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Observations {
    #[default]
    Unclear,
    Independent,
    Paired,
    Repeated,
    Clustered,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Structure {
    #[default]
    Unclear,
    Two,
    Several,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
struct Answers {
    goal: Goal,
    outcome: Outcome,
    observations: Observations,
    structure: Structure,
}
fn recommendation(a: Answers) -> Result<(Procedure, &'static str), &'static str> {
    if a.goal == Goal::Summary {
        return Ok(match a.outcome {
            Outcome::Numeric => (
                Procedure::Describe,
                "Summarise averages, spread and ranges of numeric measurements.",
            ),
            Outcome::Category => (
                Procedure::Frequencies,
                "Count exact responses and report their percentages.",
            ),
        });
    }
    if a.observations == Observations::Clustered {
        return Err("People within schools, teams or households may be related. This guide cannot choose a model for clustered data.");
    }
    if a.observations == Observations::Unclear {
        return Err("Confirm whether observations are separate, matched or repeated before choosing an analysis.");
    }
    if a.outcome == Outcome::Category {
        return Err("This guide does not offer categorical inference. Use response counts for a descriptive report or seek advice on a suitable method.");
    }
    match (a.goal, a.observations, a.structure) {
        (Goal::Reference, Observations::Independent, _) => Ok((Procedure::OneSampleT, "Compare a numeric mean with a reference value you specify.")),
        (Goal::Relationship, Observations::Independent, _) => Ok((Procedure::Pearson, "Measure a linear relationship between two numeric variables; inspect a scatterplot and check assumptions.")),
        (Goal::Compare, Observations::Independent, Structure::Two) => Ok((Procedure::IndependentT, "Compare means for two separate groups. Check the variance assumptions and method in the form.")),
        (Goal::Compare, Observations::Independent, Structure::Several) => Ok((Procedure::Anova, "Compare means across three or more separate groups. Choose the outcome and group variable in the form.")),
        (Goal::Compare, Observations::Paired | Observations::Repeated, Structure::Two) => Ok((Procedure::PairedT, "Compare two measurements from the same people or matched pairs. Confirm which columns form each pair.")),
        (Goal::Compare, Observations::Repeated, Structure::Several) => Ok((Procedure::Anova, "Compare three or more measurements of the same people. Map subject IDs and measurement columns explicitly.")),
        _ => Err("This combination needs more information or a different model. Confirm the number of groups or measurements and their design."),
    }
}

fn id() -> egui::Id {
    egui::Id::new("analysis-question-guide")
}

pub(super) fn entry(ui: &mut egui::Ui, state: &mut DesktopState) {
    if button(ui, state, "Help choose an analysis").clicked() {
        ui.ctx()
            .data_mut(|data| data.insert_temp(id(), Some(Answers::default())));
    }
}

fn choices<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    question: &str,
    value: &mut T,
    options: &[(T, &str)],
) {
    ui.strong(question);
    ui.horizontal_wrapped(|ui| {
        for (choice, label) in options {
            let response = ui.radio_value(value, *choice, *label);
            super::procedure_forms::record(state, label, &response);
        }
    });
}

fn recommended_draft(draft: &AnalysisDraft, a: Answers, procedure: Procedure) -> AnalysisDraft {
    if procedure == Procedure::Anova {
        AnalysisDraft::Anova(super::draft::anova::AnovaDraft {
            design: if a.observations == Observations::Repeated {
                super::draft::anova::AnovaDesignDraft::Repeated {
                    subject: None,
                    factors: vec![],
                    measurements: vec![],
                }
            } else {
                super::draft::anova::AnovaDesignDraft::OneWay {
                    outcome: None,
                    factor: None,
                }
            },
            ..Default::default()
        })
    } else {
        draft.for_procedure(procedure)
    }
}

fn footer(ui: &mut egui::Ui, state: &mut DesktopState, answers: Answers) -> bool {
    let result = recommendation(answers);
    let mut leave = false;
    egui::Panel::bottom("analysis-question-footer").show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            let response = ui.add_enabled(
                result.is_ok(),
                egui::Button::new("Open recommended analysis"),
            );
            super::procedure_forms::record(state, "Open recommended analysis", &response);
            if response.clicked() {
                if let Ok((procedure, _)) = result {
                    match state.checked_editable_draft() {
                        Ok(draft) => {
                            state.act(UiAction::SetDraft(recommended_draft(
                                &draft, answers, procedure,
                            )));
                            leave = state.error().is_none();
                        }
                        Err(error) => state.fail(error),
                    }
                }
            }
            if button(ui, state, "Choose manually").clicked() {
                leave = true;
            }
            if button(ui, state, "Open linear regression form").clicked() {
                open_procedure(state, Procedure::Regression);
                leave = state.error().is_none();
            }
            if button(ui, state, "Open scale reliability form").clicked() {
                open_procedure(state, Procedure::Reliability);
                leave = state.error().is_none();
            }
        });
    });
    leave
}

fn open_procedure(state: &mut DesktopState, procedure: Procedure) {
    match state.checked_editable_draft() {
        Ok(draft) => state.act(UiAction::SetDraft(draft.for_procedure(procedure))),
        Err(error) => state.fail(error),
    }
}

fn questionnaire(ui: &mut egui::Ui, state: &mut DesktopState, answers: &mut Answers) {
    ui.heading("Choose an analysis");
    ui.label("Answer for the observations you will analyse. The suggested form still needs variable choices and assumption checks.");
    choices(
        ui,
        state,
        "What do you want to find out?",
        &mut answers.goal,
        &[
            (Goal::Summary, "Describe responses"),
            (Goal::Compare, "Compare measurements"),
            (Goal::Relationship, "Explore a relationship"),
            (Goal::Reference, "Compare with a reference"),
        ],
    );
    choices(
        ui,
        state,
        if answers.goal == Goal::Relationship {
            "What kind of variables?"
        } else {
            "What kind of outcome?"
        },
        &mut answers.outcome,
        &[
            (
                Outcome::Numeric,
                if answers.goal == Goal::Relationship {
                    "Both variables are numeric"
                } else {
                    "Numeric measurements"
                },
            ),
            (Outcome::Category, "Categories or response labels"),
        ],
    );
    if answers.goal != Goal::Summary {
        choices(
            ui,
            state,
            "How are the observations related?",
            &mut answers.observations,
            &[
                (Observations::Unclear, "I'm unsure"),
                (Observations::Independent, "Separate people or observations"),
                (Observations::Paired, "Matched pairs"),
                (Observations::Repeated, "Same people measured repeatedly"),
                (
                    Observations::Clustered,
                    "People within schools, teams or households",
                ),
            ],
        );
        if answers.goal == Goal::Compare {
            choices(
                ui,
                state,
                "How many groups or measurements?",
                &mut answers.structure,
                &[
                    (Structure::Unclear, "Number not confirmed"),
                    (Structure::Two, "Two groups or measurements"),
                    (Structure::Several, "Three or more groups or measurements"),
                ],
            );
        }
    }
    ui.separator();
    match recommendation(*answers) {
        Ok((procedure, reason)) => {
            ui.strong(procedure.label());
            ui.label(reason);
        }
        Err(reason) => {
            ui.label(reason);
        }
    }
    ui.small("A suggestion does not establish causation or statistical significance. No analysis runs until you complete and run the form.");
}

pub(super) fn show(ui: &mut egui::Ui, state: &mut DesktopState) -> bool {
    let Some(mut answers) = ui
        .ctx()
        .data(|data| data.get_temp::<Option<Answers>>(id()))
        .flatten()
    else {
        return false;
    };
    let leave = footer(ui, state, answers);
    egui::ScrollArea::vertical()
        .id_salt("analysis-question-scroll")
        .show(ui, |ui| {
            questionnaire(ui, state, &mut answers);
        });
    ui.ctx()
        .data_mut(|data| data.insert_temp(id(), if leave { None } else { Some(answers) }));
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    fn answers(goal: Goal, observations: Observations, structure: Structure) -> Answers {
        Answers {
            goal,
            observations,
            structure,
            outcome: Outcome::Numeric,
        }
    }
    #[test]
    fn routes_only_supported_explicit_designs() {
        for (goal, observations, structure, expected) in [
            (
                Goal::Summary,
                Observations::Unclear,
                Structure::Unclear,
                Procedure::Describe,
            ),
            (
                Goal::Reference,
                Observations::Independent,
                Structure::Unclear,
                Procedure::OneSampleT,
            ),
            (
                Goal::Relationship,
                Observations::Independent,
                Structure::Unclear,
                Procedure::Pearson,
            ),
            (
                Goal::Compare,
                Observations::Independent,
                Structure::Two,
                Procedure::IndependentT,
            ),
            (
                Goal::Compare,
                Observations::Independent,
                Structure::Several,
                Procedure::Anova,
            ),
            (
                Goal::Compare,
                Observations::Paired,
                Structure::Two,
                Procedure::PairedT,
            ),
            (
                Goal::Compare,
                Observations::Repeated,
                Structure::Several,
                Procedure::Anova,
            ),
        ] {
            assert_eq!(
                recommendation(answers(goal, observations, structure))
                    .unwrap()
                    .0,
                expected
            );
        }
    }
    #[test]
    fn rejects_unclear_clustered_and_categorical_inference() {
        for observations in [Observations::Unclear, Observations::Clustered] {
            assert!(recommendation(answers(Goal::Compare, observations, Structure::Two)).is_err());
        }
        assert!(recommendation(Answers {
            outcome: Outcome::Category,
            ..answers(Goal::Compare, Observations::Independent, Structure::Two)
        })
        .is_err());
        assert!(recommendation(answers(
            Goal::Compare,
            Observations::Independent,
            Structure::Unclear
        ))
        .is_err());
        assert!(recommendation(answers(
            Goal::Compare,
            Observations::Paired,
            Structure::Several
        ))
        .is_err());
        assert!(recommendation(answers(
            Goal::Reference,
            Observations::Repeated,
            Structure::Two
        ))
        .is_err());
    }
    #[test]
    fn summaries_allow_categories_but_inference_requires_design_confirmation() {
        assert_eq!(
            recommendation(Answers {
                outcome: Outcome::Category,
                ..Answers::default()
            })
            .unwrap()
            .0,
            Procedure::Frequencies
        );
        for goal in [Goal::Compare, Goal::Relationship, Goal::Reference] {
            for outcome in [Outcome::Numeric, Outcome::Category] {
                for observations in [Observations::Unclear, Observations::Clustered] {
                    for structure in [Structure::Unclear, Structure::Two, Structure::Several] {
                        assert!(recommendation(Answers {
                            goal,
                            outcome,
                            observations,
                            structure
                        })
                        .is_err());
                    }
                }
            }
        }
    }
    #[test]
    fn repeated_recommendation_seeds_empty_repeated_design_without_changing_original() {
        let original = AnalysisDraft::Anova(Default::default());
        let a = answers(Goal::Compare, Observations::Repeated, Structure::Several);
        let next = recommended_draft(&original, a, recommendation(a).unwrap().0);
        assert!(matches!(
            next,
            AnalysisDraft::Anova(super::super::draft::anova::AnovaDraft {
                design: super::super::draft::anova::AnovaDesignDraft::Repeated {
                    subject: None,
                    ..
                },
                ..
            })
        ));
        assert!(matches!(
            original,
            AnalysisDraft::Anova(super::super::draft::anova::AnovaDraft {
                design: super::super::draft::anova::AnovaDesignDraft::OneWay {
                    outcome: None,
                    factor: None
                },
                ..
            })
        ));
    }
}
