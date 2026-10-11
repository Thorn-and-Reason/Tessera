use super::*;
use crate::{
    data::{Column, ColumnId, Dataset},
    project::Project,
};
fn state() -> DesktopState {
    DesktopState::from_project(
        Project::new(
            "Text transaction",
            Dataset::new(vec![
                Column::numeric(ColumnId(1), "Y", vec![Some(1.)]).unwrap()
            ])
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap()
}
#[test]
fn untouched_identity_capacity_and_ledger_remain_exact() {
    let state = state();
    let mut value = " ŵ ".to_owned();
    let cap = value.capacity();
    let mut heap = cap;
    let edit = EditBuffer::new(&mut value, &state, &mut heap);
    assert_eq!(edit.as_str(), " ŵ ");
    assert!(!edit.finish().unwrap());
    assert_eq!((value.capacity(), heap), (cap, cap));
}
#[test]
fn unicode_growth_is_exactly_charged_and_returns_character_count() {
    let state = state();
    let mut value = "A".to_owned();
    let mut heap = value.capacity();
    let mut edit = EditBuffer::new(&mut value, &state, &mut heap);
    assert_eq!(edit.insert_text("ŵ", CharIndex(1)), 1);
    assert!(edit.finish().unwrap());
    assert_eq!(value, "Aŵ");
    assert_eq!(heap, value.capacity());
    assert_eq!(heap, 3);
}
#[test]
fn refused_selected_replacement_restores_identity_and_latches_before_allocation() {
    let state = state();
    let mut value = "Saved ŵ".to_owned();
    let before = value.clone();
    let cap = value.capacity();
    let mut heap = MAX_DRAFT_BYTES;
    let text = "X".repeat(512);
    let mut edit = EditBuffer::new(&mut value, &state, &mut heap);
    edit.delete_char_range(CharIndex(0)..CharIndex(7));
    let (inserted, largest) =
        crate::data::measure_test_allocations(|| edit.insert_text(&text, CharIndex(0)));
    assert_eq!((inserted, largest), (0, 0));
    assert_eq!(edit.as_str(), before);
    edit.replace_with("Partial");
    assert_eq!(edit.as_str(), before);
    assert!(edit.finish().is_err());
    assert_eq!(value, before);
    assert_eq!(value.capacity(), cap);
    assert_eq!(heap, MAX_DRAFT_BYTES);
}
#[test]
fn undo_replacement_cannot_bypass_character_limit_or_retain_partial_deletion() {
    let state = state();
    let mut value = " ŵ ".to_owned();
    let mut heap = value.capacity();
    let mut edit = EditBuffer::new(&mut value, &state, &mut heap);
    edit.delete_char_range(CharIndex(0)..CharIndex(3));
    edit.replace_with(&"ŵ".repeat(513));
    assert_eq!(edit.as_str(), " ŵ ");
    assert!(edit.finish().is_err());
    assert_eq!(value, " ŵ ");
}
#[test]
fn cut_take_copy_is_admitted_before_allocation_or_mutation() {
    let mut state = state();
    state.active_recipe_bytes = crate::data::MAX_OWNED_BYTES;
    let mut value = " ŵ ".to_owned();
    let cap = value.capacity();
    let mut heap = cap;
    let mut edit = EditBuffer::new(&mut value, &state, &mut heap);
    let (taken, largest) = crate::data::measure_test_allocations(|| edit.take());
    assert_eq!((taken, largest), (String::new(), 0));
    assert!(edit.finish().is_err());
    assert_eq!(value, " ŵ ");
    assert_eq!((value.capacity(), heap), (cap, cap));
}
