//! Guards the vendored umya-spreadsheet patch.
//!
//! Upstream 3.0.1's styles reader pushed a default (all-none) Borders for
//! every <border> without parsing its children, so borders loaded from ANY
//! file came back "none" and the grid could not tell a form's table lines
//! from empty space. The vendor/umya-spreadsheet copy carries the one-line
//! fix; when a new upstream release makes this test pass without the
//! [patch.crates-io] entry, the vendored copy can be dropped.

use umya_spreadsheet::BorderStyleValues;

#[test]
fn borders_survive_a_write_read_roundtrip() {
    let mut book = umya_spreadsheet::new_file();
    book.sheet_by_name_mut("Sheet1")
        .expect("fresh workbook has Sheet1")
        .get_style_mut("A1")
        .get_borders_mut()
        .get_bottom_mut()
        .set_style(BorderStyleValues::Thin);

    let path = std::env::temp_dir().join(format!(
        "rustxl-border-roundtrip-{}.xlsx",
        std::process::id()
    ));
    umya_spreadsheet::writer::xlsx::write(&book, &path).expect("write");
    let back = umya_spreadsheet::reader::xlsx::read(&path).expect("read");
    let _ = std::fs::remove_file(&path);

    let bottom = back
        .sheet_by_name("Sheet1")
        .expect("sheet survives")
        .get_style("A1")
        .get_borders()
        .map(|borders| borders.get_bottom().get_border_style().to_string());
    assert_eq!(
        bottom.as_deref(),
        Some("thin"),
        "the reader must bring the border style back, not \"none\""
    );
}
