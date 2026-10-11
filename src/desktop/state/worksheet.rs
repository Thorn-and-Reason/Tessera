use super::*;

impl DesktopState {
    pub fn selected_cell(&self) -> Option<(ColumnId, usize)> {
        self.selected_cell
    }
    pub(super) fn move_cell(&self, columns: i32, rows: i32, edit: bool) -> AppResult<Self> {
        let data = self.project.dataset();
        if data.row_count() == 0 || data.column_count() == 0 {
            return Ok(self.clone());
        }
        let (id, row) = self
            .selected_cell
            .or_else(|| self.selected_column.map(|c| (c, 0)))
            .unwrap_or((data.columns()[0].id(), 0));
        let index = data
            .columns()
            .iter()
            .position(|c| c.id() == id)
            .ok_or_else(|| AppError::invalid("cell", "Select an available variable."))?;
        let mut next = if self.cell_editor.is_some() {
            self.apply_action(UiAction::CommitCell)?
        } else {
            self.clone()
        };
        let (target_col, target_row) = if edit && columns != 0 && rows == 0 {
            let flat = (row * data.column_count() + index) as i64;
            let target = (flat + i64::from(columns))
                .clamp(0, (data.row_count() * data.column_count() - 1) as i64)
                as usize;
            (target % data.column_count(), target / data.column_count())
        } else {
            (
                (index as i64 + i64::from(columns)).clamp(0, data.column_count() as i64 - 1)
                    as usize,
                (row as i64 + i64::from(rows)).clamp(0, data.row_count() as i64 - 1) as usize,
            )
        };
        let target = data.columns()[target_col].id();
        next = next.apply_action(UiAction::SelectCell {
            column: target,
            row: target_row,
        })?;
        if edit && next.dialog != Some(Dialog::PropertyChanges) {
            next = next.apply_action(UiAction::BeginCell {
                column: target,
                row: target_row,
            })?;
        }
        Ok(next)
    }
    pub(super) fn paste_cells(&self, text: &str) -> AppResult<Self> {
        self.idle()?;
        if self.cell_editor.is_some() {
            return Err(AppError::invalid(
                "paste",
                "Finish editing the current cell before pasting a range.",
            ));
        }
        let (column, row) = self
            .selected_cell
            .ok_or_else(|| AppError::invalid("paste", "Select the first destination cell."))?;
        let data = self.project.dataset().paste_tsv_with_budget(
            column,
            row,
            text,
            &[],
            self.project_baseline()?.0 + self.extra(),
        )?;
        let mut next = self.clone();
        next.publish_dataset(data)?;
        next.status =
            "Pasted cells. Empty numeric fields are missing; empty text fields remain empty."
                .into();
        Ok(next)
    }
    pub(crate) fn set_column_width(&mut self, column: ColumnId, width: f32) -> AppResult<()> {
        self.project.dataset().column(column)?;
        if !width.is_finite() {
            return Err(AppError::invalid("width", "Enter a finite column width."));
        }
        if self.column_widths.len() >= 256 && !self.column_widths.contains_key(&column) {
            return Err(AppError::invalid(
                "width",
                "Up to 256 column widths can be customised per open project.",
            ));
        }
        self.column_widths.insert(column, width.clamp(80., 480.));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Column;
    fn state() -> DesktopState {
        DesktopState::from_project(
            Project::new(
                "Cells",
                Dataset::new(vec![
                    Column::numeric(ColumnId(1), "A", vec![Some(1.); 2]).unwrap(),
                    Column::numeric(ColumnId(2), "B", vec![Some(2.); 2]).unwrap(),
                ])
                .unwrap(),
            )
            .unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn invalid_edit_cannot_move_or_close_and_valid_tab_wraps_atomically() {
        let mut s = state()
            .apply_action(UiAction::BeginCell {
                column: ColumnId(2),
                row: 0,
            })
            .unwrap();
        s.set_cell_text("bad".into()).unwrap();
        assert!(s
            .apply_action(UiAction::MoveCell {
                columns: 1,
                rows: 0,
                edit: true
            })
            .is_err());
        assert!(s
            .apply_action(UiAction::Request(PendingAction::Close))
            .is_err());
        assert_eq!(s.selected_cell(), Some((ColumnId(2), 0)));
        assert_eq!(
            s.project.dataset().cell(ColumnId(2), 0).unwrap(),
            Cell::Number(2.)
        );
        s.set_cell_text("7".into()).unwrap();
        let after = s
            .apply_action(UiAction::MoveCell {
                columns: 1,
                rows: 0,
                edit: true,
            })
            .unwrap();
        assert_eq!(after.selected_cell(), Some((ColumnId(1), 1)));
        assert_eq!(
            after.project.dataset().cell(ColumnId(2), 0).unwrap(),
            Cell::Number(7.)
        );
        assert_eq!(after.cell_editor.as_ref().unwrap().column, ColumnId(1));
    }
    fn text_state(size: usize) -> DesktopState {
        DesktopState::from_project(
            Project::new(
                "Text",
                Dataset::new(vec![Column::text(
                    ColumnId(1),
                    "Text",
                    vec![Some("x".repeat(size))],
                )
                .unwrap()])
                .unwrap(),
            )
            .unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn selecting_large_text_does_not_copy_the_cell_value() {
        let state = text_state(MAX_DRAFT_BYTES + 1);
        let (result, largest) = crate::data::measure_test_allocations(|| {
            state.apply_action(UiAction::SelectCell {
                column: ColumnId(1),
                row: 0,
            })
        });
        assert!(result.is_ok());
        assert!(largest < 1024, "selection copied {largest} bytes");
    }
    #[test]
    fn cell_editor_refuses_text_copy_before_exhausting_dynamic_headroom() {
        let mut state = text_state(128 * 1024);
        let baseline = state.project_baseline().unwrap().1;
        state.active_recipe_bytes = crate::data::MAX_OWNED_BYTES - baseline - state.extra() - 8192;
        let (result, largest) = crate::data::measure_test_allocations(|| {
            state.apply_action(UiAction::BeginCell {
                column: ColumnId(1),
                row: 0,
            })
        });
        assert!(
            matches!(result, Err(AppError::SizeLimit { .. })),
            "text editor copy should be refused before allocation"
        );
        assert!(largest < 1024, "copied {largest} bytes before admission");
        assert!(state.cell_editor.is_none());
    }
    #[test]
    fn arrows_clamp_and_width_limit_rejects_nonfinite() {
        let s = state()
            .apply_action(UiAction::SelectCell {
                column: ColumnId(1),
                row: 0,
            })
            .unwrap();
        let mut s = s
            .apply_action(UiAction::MoveCell {
                columns: -1,
                rows: -1,
                edit: false,
            })
            .unwrap();
        assert_eq!(s.selected_cell(), Some((ColumnId(1), 0)));
        assert!(s.set_column_width(ColumnId(1), f32::NAN).is_err());
        s.set_column_width(ColumnId(1), 999.).unwrap();
        assert_eq!(s.column_widths[&ColumnId(1)], 480.);
    }
}
