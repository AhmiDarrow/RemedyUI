//! [`set_max_color_matrix_entry`] moves the bound entries are kept within. A binary of its
//! own, so no other test sees the bound changed.

use remedy_ui_core::color_matrix::{
    ColorMatrix, MAX_COLOR_MATRIX_ENTRY, max_color_matrix_entry, set_max_color_matrix_entry,
};

#[test]
fn the_app_sets_the_largest_entry() {
    let huge = ColorMatrix([1000.0; 20]);
    assert_eq!(max_color_matrix_entry(), MAX_COLOR_MATRIX_ENTRY);
    assert!(huge.limited().0.iter().all(|v| *v == MAX_COLOR_MATRIX_ENTRY));
    set_max_color_matrix_entry(4096.0);
    assert_eq!(huge.limited(), huge);
    set_max_color_matrix_entry(f32::NAN);
    assert!(huge.limited().0.iter().all(|v| *v == 0.0), "not a number counts as 0");
    set_max_color_matrix_entry(MAX_COLOR_MATRIX_ENTRY);
}
