use tessera::survey::{SurveyFinding, SurveyRole};

#[test]
fn reporting_actions_describe_the_task_in_plain_language() {
    assert_eq!(SurveyRole::Unresolved.label(), "Choose an action");
    assert_eq!(SurveyRole::Exclude.label(), "Leave out");
    assert_eq!(SurveyRole::Categories.label(), "Count responses");
    assert_eq!(
        SurveyRole::NumericMeasure.label(),
        "Summarise a measurement"
    );
}

#[test]
fn review_explanations_describe_the_decision_without_claiming_certainty() {
    assert!(SurveyFinding::CandidateMissingToken
        .explanation()
        .contains("choose"));
    assert!(SurveyFinding::LabelVariants
        .explanation()
        .contains("review"));
    assert!(SurveyFinding::MultipleAnswers
        .explanation()
        .contains("complete cell"));
    assert!(SurveyFinding::PersonalDataCandidate
        .explanation()
        .contains("may"));
}
