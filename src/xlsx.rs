//! Writing the grid back into the workbook it was read from.
//!
//! A grid is a rectangle of text. A workbook is not: it carries column widths,
//! number formats, conditional formatting, charts, merged ranges, and a formula
//! behind every calculated cell. Rebuilding a file from the grid would throw
//! all of that away, and the user would have no way to tell until they opened
//! the result somewhere else.
//!
//! So saving does not rebuild. It takes the workbook the file was opened from
//! and changes only the cells whose text differs from what the grid holds.
//! Everything else is left exactly as it was found.

use umya_spreadsheet::Workbook;

use crate::sheet::Sheet;

/// Writes `sheets` into `book`, changing only the cells that differ.
///
/// Cells whose text still matches the workbook are skipped, which is what keeps
/// a formula alive: `=SUM(B2:B3)` shows as `30` in the grid, the workbook still
/// says `30`, so the cell is never touched and the formula survives.
///
/// A cell the grid no longer has is cleared rather than removed, so any format
/// on it stays. Cells the workbook holds that were empty to begin with are left
/// alone entirely — they never reached the grid, so their absence there says
/// nothing about what the user wanted.
pub fn apply_sheets(book: &mut Workbook, sheets: &[Sheet]) -> Result<(), String> {
    for sheet in sheets {
        if book.sheet_by_name(&sheet.name).is_err() {
            book.new_sheet(&sheet.name).map_err(|e| e.to_string())?;
        }
        let worksheet = book
            .sheet_by_name_mut(&sheet.name)
            .map_err(|e| e.to_string())?;

        for (&(row, col), value) in &sheet.cells {
            // Workbook coordinates are `(column, row)` and start at 1.
            let coordinate = (col as u32 + 1, row as u32 + 1);

            let unchanged = worksheet
                .cell(coordinate)
                .is_some_and(|cell| cell.value().as_ref() == value.as_str());
            if unchanged {
                continue;
            }

            let cell = worksheet.cell_mut(coordinate);
            match value.strip_prefix('=') {
                // The user typed a formula; store it as one so the workbook
                // recalculates it rather than treating it as a label.
                Some(formula) => {
                    cell.set_formula(formula);
                }
                // `set_value` types the text the way a spreadsheet would —
                // `31` becomes a number, `TRUE` a boolean — and drops whatever
                // formula the cell used to hold.
                None => {
                    cell.set_value(value);
                }
            }
        }

        let emptied = cells_the_grid_no_longer_has(worksheet, sheet);
        for coordinate in emptied {
            worksheet.cell_mut(coordinate).set_blank();
        }
    }

    Ok(())
}

/// Coordinates that hold a value in the workbook but not in the grid.
///
/// Collected before anything is written because reading the worksheet and
/// modifying it cannot overlap.
fn cells_the_grid_no_longer_has(
    worksheet: &umya_spreadsheet::Worksheet,
    sheet: &Sheet,
) -> Vec<(u32, u32)> {
    worksheet
        .cells()
        .iter()
        .filter(|cell| !cell.value().is_empty())
        .filter_map(|cell| {
            let coordinate = cell.coordinate();
            let (col, row) = (coordinate.col_num(), coordinate.row_num());
            // A zero would mean a malformed file rather than cell A1.
            let grid = (
                (row as usize).checked_sub(1)?,
                (col as usize).checked_sub(1)?,
            );
            (!sheet.cells.contains_key(&grid)).then_some((col, row))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A workbook with one sheet holding `A1 = "kept"` and `B1 = 7`.
    fn book_with_a_sheet() -> Workbook {
        let mut book = umya_spreadsheet::new_file_empty_worksheet();
        let sheet = book.new_sheet("Data").expect("a fresh workbook accepts it");
        sheet.cell_mut("A1").set_value("kept");
        sheet.cell_mut("B1").set_value("7");
        book
    }

    fn grid(name: &str, cells: &[((usize, usize), &str)]) -> Sheet {
        let mut sheet = Sheet::new(name);
        for &((row, col), value) in cells {
            sheet.cells.insert((row, col), value.to_string());
        }
        sheet
    }

    fn value_at(book: &Workbook, sheet: &str, coordinate: &str) -> String {
        book.sheet_by_name(sheet)
            .expect("the sheet exists")
            .cell(coordinate)
            .map(|cell| cell.value().into_owned())
            .unwrap_or_default()
    }

    #[test]
    fn a_changed_cell_is_written() {
        let mut book = book_with_a_sheet();
        let sheets = [grid("Data", &[((0, 0), "kept"), ((0, 1), "8")])];

        apply_sheets(&mut book, &sheets).expect("apply succeeds");

        assert_eq!(value_at(&book, "Data", "B1"), "8");
    }

    /// Numbers stay numbers, so they still add up in a spreadsheet.
    #[test]
    fn a_number_is_written_as_a_number_not_as_text() {
        let mut book = book_with_a_sheet();
        let sheets = [grid("Data", &[((0, 1), "8")])];

        apply_sheets(&mut book, &sheets).expect("apply succeeds");

        let data_type = book
            .sheet_by_name("Data")
            .expect("the sheet exists")
            .cell("B1")
            .map(|cell| cell.data_type().to_string())
            .unwrap_or_default();
        assert_eq!(data_type, "n", "B1 should be numeric");
    }

    /// A cell whose text has not changed is not written to at all.
    ///
    /// This is what protects a formula. The grid shows the result, the workbook
    /// still agrees with it, and the cell is skipped — so `=SUM(...)` is still
    /// there afterwards.
    #[test]
    fn an_untouched_formula_cell_keeps_its_formula() {
        let mut book = umya_spreadsheet::new_file_empty_worksheet();
        {
            let sheet = book.new_sheet("Data").expect("fresh workbook");
            sheet.cell_mut("A1").set_value("10");
            let total = sheet.cell_mut("B1");
            total.set_formula("SUM(A1:A1)");
            total.set_formula_result_number(10);
        }

        // The grid holds the cached result, exactly as the reader loaded it.
        let sheets = [grid("Data", &[((0, 0), "10"), ((0, 1), "10")])];
        apply_sheets(&mut book, &sheets).expect("apply succeeds");

        let formula = book
            .sheet_by_name("Data")
            .expect("the sheet exists")
            .cell("B1")
            .map(|cell| cell.formula().to_string())
            .unwrap_or_default();
        assert_eq!(formula, "SUM(A1:A1)");
    }

    /// Typing a formula into the grid stores it as a formula.
    #[test]
    fn a_typed_formula_is_stored_as_a_formula() {
        let mut book = book_with_a_sheet();
        let sheets = [grid("Data", &[((0, 1), "=SUM(A1:A1)")])];

        apply_sheets(&mut book, &sheets).expect("apply succeeds");

        let cell_is = |f: fn(&umya_spreadsheet::Cell) -> String| {
            book.sheet_by_name("Data")
                .expect("the sheet exists")
                .cell("B1")
                .map(f)
                .unwrap_or_default()
        };
        assert_eq!(cell_is(|c| c.formula().to_string()), "SUM(A1:A1)");
    }

    /// Overwriting a formula cell with a value drops the formula.
    #[test]
    fn writing_a_value_over_a_formula_removes_the_formula() {
        let mut book = umya_spreadsheet::new_file_empty_worksheet();
        {
            let sheet = book.new_sheet("Data").expect("fresh workbook");
            let total = sheet.cell_mut("A1");
            total.set_formula("SUM(B1:B9)");
            total.set_formula_result_number(10);
        }

        let sheets = [grid("Data", &[((0, 0), "99")])];
        apply_sheets(&mut book, &sheets).expect("apply succeeds");

        let sheet = book.sheet_by_name("Data").expect("the sheet exists");
        let cell = sheet.cell("A1").expect("A1 exists");
        assert_eq!(cell.value().as_ref(), "99");
        assert!(!cell.is_formula(), "the formula must not survive the value");
    }

    /// A cell the user cleared is emptied in the workbook too.
    #[test]
    fn a_cell_removed_from_the_grid_is_cleared() {
        let mut book = book_with_a_sheet();
        let sheets = [grid("Data", &[((0, 0), "kept")])];

        apply_sheets(&mut book, &sheets).expect("apply succeeds");

        assert_eq!(value_at(&book, "Data", "A1"), "kept");
        assert_eq!(value_at(&book, "Data", "B1"), "", "B1 was cleared");
    }

    /// Cells that were already empty are left alone.
    ///
    /// A workbook can hold a cell that carries only formatting. It never
    /// reaches the grid, so its absence there is not the user asking for it to
    /// be removed — and removing it would throw away the format.
    #[test]
    fn an_already_empty_cell_is_not_disturbed() {
        let mut book = umya_spreadsheet::new_file_empty_worksheet();
        {
            let sheet = book.new_sheet("Data").expect("fresh workbook");
            sheet.cell_mut("A1").set_value("kept");
            // Touching a cell creates it; this one has no value.
            sheet.cell_mut("C3");
        }

        let sheets = [grid("Data", &[((0, 0), "kept")])];
        apply_sheets(&mut book, &sheets).expect("apply succeeds");

        assert!(
            book.sheet_by_name("Data")
                .expect("the sheet exists")
                .cell("C3")
                .is_some(),
            "an empty cell should survive a save"
        );
    }

    /// Sheets the grid does not mention are not touched.
    #[test]
    fn other_sheets_are_left_alone() {
        let mut book = book_with_a_sheet();
        book.new_sheet("Untouched")
            .expect("fresh name")
            .cell_mut("A1")
            .set_value("still here");

        let sheets = [grid("Data", &[((0, 0), "kept"), ((0, 1), "7")])];
        apply_sheets(&mut book, &sheets).expect("apply succeeds");

        assert_eq!(value_at(&book, "Untouched", "A1"), "still here");
    }

    /// A sheet the workbook does not have yet is created.
    #[test]
    fn a_sheet_that_is_not_in_the_workbook_is_added() {
        let mut book = book_with_a_sheet();
        let sheets = [
            grid("Data", &[((0, 0), "kept"), ((0, 1), "7")]),
            grid("New", &[((0, 0), "fresh")]),
        ];

        apply_sheets(&mut book, &sheets).expect("apply succeeds");

        assert_eq!(value_at(&book, "New", "A1"), "fresh");
    }

    /// Row and column indices are not transposed on the way out.
    ///
    /// Grid coordinates are `(row, column)` and workbook coordinates are
    /// `(column, row)`. Swapping them writes to the wrong cell in a way that a
    /// square test case would never notice.
    #[test]
    fn grid_coordinates_map_to_the_right_workbook_cell() {
        let mut book = book_with_a_sheet();
        // Row 2, column 0 -> A3.
        let sheets = [grid(
            "Data",
            &[((0, 0), "kept"), ((0, 1), "7"), ((2, 0), "down")],
        )];

        apply_sheets(&mut book, &sheets).expect("apply succeeds");

        assert_eq!(value_at(&book, "Data", "A3"), "down");
        assert_eq!(value_at(&book, "Data", "C1"), "", "not transposed");
    }
}
