use super::{start, state::TEXT_LIMIT, theme, variables, DesktopState};
use crate::{
    data::{ColumnKind, MissingRules},
    import::{Delimiter, ImportConversionNote, ImportWarning},
    jobs::{JobOperation, JobRunner},
};
use eframe::egui;

fn column_review_height(ui: &egui::Ui) -> f32 {
    14.0 * (ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y)
        + ui.spacing().interact_size.y
        + 8.0
}

fn line(ui: &mut egui::Ui, text: impl std::fmt::Display) {
    let text = theme::caption(text);
    ui.add(egui::Label::new(&text).truncate())
        .on_hover_text(text);
}

pub(crate) fn show(ui: &mut egui::Ui, state: &mut DesktopState, jobs: &mut JobRunner) {
    let Some(preview) = state.preview.as_ref().cloned() else {
        return;
    };
    ui.label(format!(
        "{} data rows · {} variables",
        preview.row_count(),
        preview.columns().len()
    ));
    ui.label(theme::caption(
        "First row is used as variable names. Review each variable before importing.",
    ));
    ui.label(theme::caption(
        "Changing delimiter previews the file again and resets reviewed choices.",
    ));

    let previous = state.import_delimiter;
    ui.add_enabled_ui(!state.active, |ui| {
        egui::ComboBox::new("delimiter", "Delimiter")
            .selected_text(if previous == Delimiter::Comma {
                "Comma"
            } else {
                "Tab"
            })
            .show_ui(ui, |ui| {
                theme::selectable_value(ui, &mut state.import_delimiter, Delimiter::Comma, "Comma");
                theme::selectable_value(ui, &mut state.import_delimiter, Delimiter::Tab, "Tab");
            });
    });
    if previous != state.import_delimiter {
        if let Some(path) = state.import_path.clone() {
            start(
                state,
                jobs,
                JobOperation::PreviewPath {
                    path,
                    delimiter: state.import_delimiter,
                },
            );
        }
        return;
    }

    let row_height = column_review_height(ui);
    ui.add_enabled_ui(!state.active, |ui| {
        // Only visible preview choices are rendered; source samples are already bounded.
        egui::ScrollArea::vertical()
            .id_salt("import-columns")
            .max_height(260.0)
            .show_rows(ui, row_height, state.choices.len(), |ui, range| {
                for index in range {
                    ui.push_id(index, |ui| {
                        ui.set_min_height(row_height);
                        let original = &state.choices[index];
                        if original.name.chars().take(TEXT_LIMIT + 1).count() > TEXT_LIMIT {
                            line(ui, "This header exceeds the editor's text limit. Shorten it in the source file and preview again.");
                            return;
                        }
                        let admitted = crate::import::choice_bytes(std::slice::from_ref(original))
                            .and_then(|bytes| state.check_ui_copy(bytes + original.name.len() + crate::import::REVIEW_WORK_BYTES));
                        if let Err(error) = admitted {
                            line(ui, error);
                            return;
                        }
                        let mut choice = original.clone();
                        let old_kind = choice.kind;
                        ui.horizontal(|ui| {
                            let name_label = ui.label(format!("Variable {} name", index + 1));
                            ui.add(
                                egui::TextEdit::singleline(&mut choice.name)
                                    .char_limit(TEXT_LIMIT)
                                    .desired_width(170.0),
                            )
                            .labelled_by(name_label.id);
                            variables::kind_picker(ui, "kind", &mut choice.kind);
                        });
                        if old_kind != choice.kind {
                            choice.missing_rules = MissingRules::None;
                        }
                        if preview.columns()[index]
                            .warnings
                            .contains(&ImportWarning::PrecisionRisk)
                        {
                            ui.checkbox(&mut choice.acknowledge_precision_loss, "Allow numeric precision loss").on_hover_text("Keep this identifier as Text to preserve its exact digits. Converting to Number may round those digits.");
                        }
                        if choice.kind == ColumnKind::Text {
                            let mut blank_missing = matches!(
                                &choice.missing_rules,
                                MissingRules::Text(values) if values.iter().any(String::is_empty)
                            );
                            if ui.checkbox(&mut blank_missing, "Empty text is missing").changed() {
                                choice.missing_rules = if blank_missing {
                                    MissingRules::Text(vec![String::new()])
                                } else {
                                    MissingRules::None
                                };
                            }
                        }
                        if preview.columns()[index].warnings.iter().any(|warning| {
                            matches!(
                                warning,
                                ImportWarning::InvalidHeader | ImportWarning::DuplicateHeader
                            )
                        }) {
                            line(ui, "Enter a unique, nonblank name.");
                        }
                        match preview.review_choice(index, &choice) {
                            Ok(review) => conversion_examples(ui, &review),
                            Err(error) => {
                                ui.add(egui::Label::new(egui::RichText::new(theme::caption(error)).color(ui.visuals().error_fg_color)).truncate());
                            }
                        }
                        let original = &state.choices[index];
                        if choice.name != original.name
                            || choice.kind != original.kind
                            || choice.missing_rules != original.missing_rules
                            || choice.acknowledge_precision_loss
                                != original.acknowledge_precision_loss
                        {
                            if let Err(error) = state.edit_import_choice(index, choice) {
                                state.fail(error);
                            }
                        }
                        ui.separator();
                    });
                }
            });
    });
    sample_rows(ui, &preview, &state.choices);
}

fn conversion_examples(ui: &mut egui::Ui, review: &crate::import::ImportColumnReview) {
    if !review.notes.is_empty() {
        for note in &review.notes {
            line(ui, note_text(*note));
        }
    }
    if review.examples.is_empty() {
        return;
    }
    line(ui, "Selected conversion preview");
    egui::Grid::new(("import-conversion", &review.name))
        .striped(true)
        .show(ui, |ui| {
            ui.label(theme::caption("Source"));
            ui.label(theme::caption("Stored"));
            ui.end_row();
            for example in review.examples.iter().take(3) {
                line(ui, display_empty(&example.source));
                let stored = display_empty(&example.stored);
                line(
                    ui,
                    if example.marked_missing {
                        format!("{stored} · marked missing")
                    } else {
                        stored
                    },
                );
                ui.end_row();
            }
        });
}

fn note_text(note: ImportConversionNote) -> &'static str {
    match note {
        ImportConversionNote::BlankNumberImportsAsMissing => {
            "Blank numeric cells import as Missing."
        }
        ImportConversionNote::EmptyTextMarkedMissing => "Empty text can be marked missing.",
        ImportConversionNote::KeepAsTextToPreserveExactCharacters => {
            "Keep as Text to preserve leading zeros and exact characters."
        }
        ImportConversionNote::NumericPrecisionMayRound => {
            "Converting large identifiers to Number can round their digits."
        }
        ImportConversionNote::ShortenedOrOmittedSamples => {
            "Some source samples are shortened or omitted. Their conversion is not shown."
        }
    }
}

fn display_empty(value: &str) -> String {
    if value.is_empty() {
        "Empty text".into()
    } else {
        value.into()
    }
}

fn sample_rows(
    ui: &mut egui::Ui,
    preview: &crate::import::ImportPreview,
    choices: &[crate::import::ImportColumnChoice],
) {
    const SAMPLE_COLUMNS: usize = 12;
    let retained_columns = preview
        .sample_rows()
        .iter()
        .map(Vec::len)
        .max()
        .unwrap_or(0);
    let shown_columns = retained_columns.min(choices.len()).min(SAMPLE_COLUMNS);
    egui::CollapsingHeader::new("Source samples")
        .default_open(true)
        .show(ui, |ui| {
            egui::ScrollArea::horizontal().show(ui, |ui| {
                egui::Grid::new("import-sample")
                    .striped(true)
                    .show(ui, |ui| {
                        for choice in choices.iter().take(shown_columns) {
                            ui.strong(theme::caption(&choice.name));
                        }
                        ui.end_row();
                        for row in preview.sample_rows() {
                            for value in row.iter().take(shown_columns) {
                                ui.label(theme::caption(value));
                            }
                            ui.end_row();
                        }
                    });
            });
            if shown_columns == 0 && preview.row_count() > 0 {
                ui.small("Source rows exceed the sample preview budget. Review each variable's conversion above.");
            } else if shown_columns < choices.len() {
                ui.small(format!("Showing the first {shown_columns} sampled columns of {}. Review each variable's conversion above.", choices.len()));
            }
            if preview.samples_truncated() {
                ui.small("Source samples are shortened or omitted to keep the preview small. Conversion examples use complete samples only.");
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn preview_state(source: &str) -> DesktopState {
        let preview = Arc::new(
            crate::import::preview(
                Arc::from(source.as_bytes()),
                Delimiter::Comma,
                &crate::operation::JobControl::default(),
            )
            .unwrap(),
        );
        let mut state = DesktopState::new().unwrap();
        state.choices = Arc::new(preview.default_choices().unwrap());
        state.preview = Some(preview);
        state
    }

    #[test]
    fn source_table_only_renders_a_bounded_retained_column_subset() {
        let headers = (0..64)
            .map(|index| format!("C{index}"))
            .collect::<Vec<_>>()
            .join(",");
        let values = std::iter::repeat_n("x", 64).collect::<Vec<_>>().join(",");
        let state = preview_state(&format!("{headers}\n{values}\n"));
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            sample_rows(ui, state.preview.as_ref().unwrap(), &state.choices);
        });
        output.textures_delta.clear();
        let nodes = output.platform_output.accesskit_update.unwrap().nodes;
        let has = |name: &str| {
            nodes
                .iter()
                .any(|(_, node)| node.label().or(node.value()) == Some(name))
        };
        assert!(has("C0"));
        assert!(has("C11"));
        assert!(!has("C12"));
        assert!(!has("C63"));
        assert!(nodes.iter().any(|(_, node)| node
            .label()
            .or(node.value())
            .is_some_and(|text| text.contains("Showing the first 12"))));

        let headers = (0..3000)
            .map(|index| format!("C{index}"))
            .collect::<Vec<_>>()
            .join(",");
        let values = std::iter::repeat_n("x", 3000).collect::<Vec<_>>().join(",");
        let state = preview_state(&format!("{headers}\n{values}\n"));
        assert!(state.preview.as_ref().unwrap().sample_rows().is_empty());
        let mut output = ctx.run_ui(Default::default(), |ui| {
            sample_rows(ui, state.preview.as_ref().unwrap(), &state.choices);
        });
        output.textures_delta.clear();
        let nodes = output.platform_output.accesskit_update.unwrap().nodes;
        assert!(!nodes
            .iter()
            .any(|(_, node)| node.label().or(node.value()) == Some("C0")));
        assert!(nodes.iter().any(|(_, node)| node
            .label()
            .or(node.value())
            .is_some_and(|text| text.contains("exceed the sample preview budget"))));
    }

    #[test]
    fn oversized_source_header_is_not_copied_into_the_text_editor() {
        let header = "x".repeat(TEXT_LIMIT + 1);
        let mut state = preview_state(&format!("{header}\n1\n"));
        let mut jobs = JobRunner::default();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut output = ctx.run_ui(Default::default(), |ui| show(ui, &mut state, &mut jobs));
        output.textures_delta.clear();
        let nodes = output.platform_output.accesskit_update.unwrap().nodes;
        assert!(!nodes
            .iter()
            .any(|(_, node)| node.role() == egui::accesskit::Role::TextInput
                && node.value() == Some(header.as_str())));
        assert!(nodes.iter().any(|(_, node)| node
            .label()
            .or(node.value())
            .is_some_and(|text| text.contains("exceeds the editor's text limit"))));
        assert_eq!(state.choices[0].name, header);
    }

    #[test]
    fn conversion_columns_use_the_declared_virtual_row_stride() {
        let mut state = preview_state("Identifier,Other\n001,001\n002,002\n003,003\n");
        let mut jobs = JobRunner::default();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut stride = 0.0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 900.0),
                )),
                ..Default::default()
            },
            |ui| {
                stride = column_review_height(ui) + ui.spacing().item_spacing.y;
                show(ui, &mut state, &mut jobs);
            },
        );
        output.textures_delta.clear();
        let nodes = output.platform_output.accesskit_update.unwrap().nodes;
        let top = |name: &str| {
            nodes
                .iter()
                .find(|(_, node)| node.label().or(node.value()) == Some(name))
                .unwrap()
                .1
                .bounds()
                .unwrap()
                .y0
        };
        let actual = top("Variable 2 name") - top("Variable 1 name");
        assert!(
            (actual - f64::from(stride)).abs() < 1.0,
            "actual {actual}, declared stride {stride}"
        );
    }
}
