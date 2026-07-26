//! Undo/redo for the grid.
//!
//! Snapshot-based: every mutating action records the active sheet's editable
//! state BEFORE it changes, and undo swaps the live state with the top of the
//! stack. Snapshots beat command-pattern deltas here because the mutations
//! are heterogeneous (typing, paste, row surgery, colour fills, table
//! formatting) and a missed inverse in any one of them silently corrupts
//! history; a snapshot cannot be wrong about how to go back.
//!
//! The cost is a clone of the sheet's maps per edit. The reference workbooks
//! hold a few thousand occupied cells, which clones in well under a
//! millisecond — bounded further by [`UNDO_DEPTH`].

use std::collections::HashMap;

use crate::spreadsheet::Spreadsheet;
use crate::types::CellStyle;

/// How many steps back the user can go. Deep enough that "I broke it a while
/// ago" is recoverable, bounded so a long session cannot hoard memory.
const UNDO_DEPTH: usize = 100;

/// Everything undo restores about one sheet, plus WHICH sheet it was: undo
/// after switching sheets first switches back, so the change the user sees
/// reverting is the change they made.
#[derive(Clone)]
pub struct SheetSnapshot {
    pub sheet_index: usize,
    pub cells: HashMap<(usize, usize), String>,
    pub formulas: HashMap<(usize, usize), String>,
    pub cell_styles: HashMap<(usize, usize), CellStyle>,
    pub col_widths: HashMap<usize, u16>,
    pub row_heights: HashMap<usize, u16>,
    pub num_rows: usize,
    pub num_cols: usize,
    pub cursor_row: usize,
    pub cursor_col: usize,
}

#[derive(Default)]
pub struct UndoStack {
    undo: Vec<SheetSnapshot>,
    redo: Vec<SheetSnapshot>,
}

impl UndoStack {
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

impl Spreadsheet {
    fn snapshot(&self) -> SheetSnapshot {
        SheetSnapshot {
            sheet_index: self.active_sheet,
            cells: self.cells.clone(),
            formulas: self.formulas.clone(),
            cell_styles: self.cell_styles.clone(),
            col_widths: self.col_widths.clone(),
            row_heights: self.row_heights.clone(),
            num_rows: self.num_rows,
            num_cols: self.num_cols,
            cursor_row: self.cursor_row,
            cursor_col: self.cursor_col,
        }
    }

    fn apply_snapshot(&mut self, snapshot: SheetSnapshot) {
        // A snapshot of another sheet first parks the current one and checks
        // the right one out, so the restore lands where the edit happened.
        if snapshot.sheet_index != self.active_sheet {
            self.activate_sheet(snapshot.sheet_index);
        }
        self.cells = snapshot.cells;
        self.formulas = snapshot.formulas;
        self.cell_styles = snapshot.cell_styles;
        self.col_widths = snapshot.col_widths;
        self.row_heights = snapshot.row_heights;
        self.num_rows = snapshot.num_rows;
        self.num_cols = snapshot.num_cols;
        self.cursor_row = snapshot.cursor_row.min(self.num_rows.saturating_sub(1));
        self.cursor_col = snapshot.cursor_col.min(self.num_cols.saturating_sub(1));
        self.clear_selection();
    }

    /// Record the state a mutating action is about to change. Every entry
    /// point that writes cells, styles or geometry calls this FIRST; a new
    /// edit invalidates the redo branch, exactly as every editor does.
    pub fn record_undo(&mut self) {
        let snapshot = self.snapshot();
        self.undo_stack.undo.push(snapshot);
        if self.undo_stack.undo.len() > UNDO_DEPTH {
            self.undo_stack.undo.remove(0);
        }
        self.undo_stack.redo.clear();
    }

    /// Step back once. Returns whether anything changed.
    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo_stack.undo.pop() else {
            return false;
        };
        let current = self.snapshot();
        self.undo_stack.redo.push(current);
        self.apply_snapshot(previous);
        self.mark_dirty();
        true
    }

    /// Step forward once after an undo. Returns whether anything changed.
    pub fn redo(&mut self) -> bool {
        let Some(next) = self.undo_stack.redo.pop() else {
            return false;
        };
        let current = self.snapshot();
        self.undo_stack.undo.push(current);
        self.apply_snapshot(next);
        self.mark_dirty();
        true
    }

    pub fn can_undo(&self) -> bool {
        self.undo_stack.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.undo_stack.can_redo()
    }
}

#[cfg(test)]
mod tests {
    use crate::spreadsheet::Spreadsheet;

    #[test]
    fn undo_restores_the_overwritten_cell_and_redo_reapplies_it() {
        let mut sheet = Spreadsheet::new();
        sheet.set_cell(0, 0, "eski".into());

        sheet.record_undo();
        sheet.set_cell(0, 0, "yeni".into());
        assert_eq!(sheet.get_cell(0, 0), "yeni");

        assert!(sheet.undo());
        assert_eq!(sheet.get_cell(0, 0), "eski");

        assert!(sheet.redo());
        assert_eq!(sheet.get_cell(0, 0), "yeni");
    }

    #[test]
    fn a_new_edit_clears_the_redo_branch() {
        let mut sheet = Spreadsheet::new();
        sheet.record_undo();
        sheet.set_cell(0, 0, "a".into());
        assert!(sheet.undo());
        assert!(sheet.can_redo());

        sheet.record_undo();
        sheet.set_cell(0, 0, "b".into());
        assert!(!sheet.can_redo(), "a new edit forks history");
        assert!(sheet.undo());
        assert_eq!(sheet.get_cell(0, 0), "");
    }

    #[test]
    fn undo_on_an_empty_stack_is_a_no_op() {
        let mut sheet = Spreadsheet::new();
        assert!(!sheet.undo());
        assert!(!sheet.redo());
    }

    #[test]
    fn undo_crossing_a_sheet_switch_returns_to_the_edited_sheet() {
        let mut sheet = Spreadsheet::new();
        sheet.replace_sheets(vec![
            crate::sheet::Sheet::new("Alpha"),
            crate::sheet::Sheet::new("Beta"),
        ]);
        sheet.record_undo();
        sheet.set_cell(0, 0, "alpha-degeri".into());

        assert!(sheet.activate_sheet(1));
        assert!(sheet.undo());
        assert_eq!(sheet.active_sheet_name(), "Alpha");
        assert_eq!(sheet.get_cell(0, 0), "");
    }
}
