//! The header toolbar: clickable undo/redo, row surgery, and colour tools.
//!
//! The grid is mouse-first inside herdr, so the operations people reach for
//! most — step back, step forward, add or remove a row, colour a selection —
//! must exist as visible buttons, not only as key chords. The renderer draws
//! the strip and records each button's rectangle here; the mouse handler
//! resolves a click back through the same table, so the two can never
//! disagree about where a button is.

use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::constants::COLOR_PALETTE;
use crate::spreadsheet::Spreadsheet;

/// What one toolbar button does. `PaletteColor` carries the palette index so
/// the swatch row reuses the same dispatch as the fixed buttons.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ToolbarAction {
    Undo,
    Redo,
    InsertRowAbove,
    InsertRowBelow,
    DeleteRow,
    CopyRow,
    Paste,
    TextColor,
    FillColor,
    BorderColor,
    PaletteColor(usize),
    PaletteClear,
}

/// Which property an open palette paints.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PaletteTarget {
    Text,
    Fill,
    Border,
}

/// Button rectangles as drawn in the last frame — the mouse's map.
#[derive(Default, Clone)]
pub struct ToolbarGeometry {
    pub buttons: Vec<(Rect, ToolbarAction)>,
}

impl ToolbarGeometry {
    pub fn action_at(&self, column: u16, row: u16) -> Option<ToolbarAction> {
        self.buttons
            .iter()
            .find(|(rect, _)| {
                column >= rect.x
                    && column < rect.x.saturating_add(rect.width)
                    && row >= rect.y
                    && row < rect.y.saturating_add(rect.height)
            })
            .map(|(_, action)| *action)
    }
}

/// How long a pressed button stays lit. Long enough to be seen at the far end
/// of a click, short enough that it is gone before the eye moves on; the main
/// loop redraws on its own tick, so the flash clears itself without an event.
pub const PRESS_FLASH: std::time::Duration = std::time::Duration::from_millis(180);

impl Spreadsheet {
    /// Light one button up because the mouse just pressed it.
    pub fn flash_toolbar(&mut self, action: ToolbarAction) {
        self.toolbar_pressed = Some((action, std::time::Instant::now()));
    }

    /// Whether this button should be drawn pressed right now.
    pub fn toolbar_is_pressed(&self, action: ToolbarAction) -> bool {
        self.toolbar_is_pressed_at(action, std::time::Instant::now())
    }

    /// The clock-free half, so the fade is testable without sleeping.
    pub fn toolbar_is_pressed_at(&self, action: ToolbarAction, now: std::time::Instant) -> bool {
        self.toolbar_pressed
            .is_some_and(|(pressed, at)| pressed == action && now.duration_since(at) < PRESS_FLASH)
    }

    /// Perform one toolbar action. Returns whether anything changed.
    pub fn apply_toolbar_action(&mut self, action: ToolbarAction) -> bool {
        match action {
            ToolbarAction::Undo => self.undo(),
            ToolbarAction::Redo => self.redo(),
            ToolbarAction::InsertRowAbove => {
                self.insert_row_above_cursor();
                true
            }
            ToolbarAction::InsertRowBelow => {
                self.insert_row_below_cursor();
                true
            }
            ToolbarAction::DeleteRow => {
                self.delete_cursor_row();
                true
            }
            ToolbarAction::CopyRow => {
                self.copy_cursor_row();
                true
            }
            // Copy without paste is half a clipboard. `paste` already decides
            // between the internal styled copy and the system clipboard, so
            // the button is the same door the keyboard uses.
            ToolbarAction::Paste => {
                self.paste();
                true
            }
            ToolbarAction::TextColor => {
                self.toolbar_palette = match self.toolbar_palette {
                    Some(PaletteTarget::Text) => None,
                    _ => Some(PaletteTarget::Text),
                };
                true
            }
            ToolbarAction::FillColor => {
                self.toolbar_palette = match self.toolbar_palette {
                    Some(PaletteTarget::Fill) => None,
                    _ => Some(PaletteTarget::Fill),
                };
                true
            }
            ToolbarAction::BorderColor => {
                self.toolbar_palette = match self.toolbar_palette {
                    Some(PaletteTarget::Border) => None,
                    _ => Some(PaletteTarget::Border),
                };
                true
            }
            ToolbarAction::PaletteColor(index) => {
                let Some(target) = self.toolbar_palette else {
                    return false;
                };
                let Some((color, _)) = COLOR_PALETTE.get(index).copied() else {
                    return false;
                };
                self.apply_color_to_selection(target, Some(color));
                true
            }
            ToolbarAction::PaletteClear => {
                let Some(target) = self.toolbar_palette else {
                    return false;
                };
                self.apply_color_to_selection(target, None);
                true
            }
        }
    }

    /// Every cell the colour tools act on: the drag selection, a selected
    /// row/column band, or just the cursor cell — the same precedence the
    /// copy path uses, so what colours is what would copy.
    fn selection_cells(&self) -> Vec<(usize, usize)> {
        let (min_row, min_col, max_row, max_col) =
            if let Some(((r1, c1), (r2, c2))) = self.get_selection_range() {
                (r1, c1, r2, c2)
            } else if let Some((min_row, max_row)) = self.selected_rows {
                (min_row, 0, max_row, self.num_cols.saturating_sub(1))
            } else if let Some((min_col, max_col)) = self.selected_cols {
                (0, min_col, self.num_rows.saturating_sub(1), max_col)
            } else {
                (
                    self.cursor_row,
                    self.cursor_col,
                    self.cursor_row,
                    self.cursor_col,
                )
            };
        let mut cells = Vec::new();
        for row in min_row..=max_row {
            for col in min_col..=max_col {
                cells.push((row, col));
            }
        }
        cells
    }

    fn apply_color_to_selection(&mut self, target: PaletteTarget, color: Option<Color>) {
        self.record_undo();
        self.mark_dirty();
        for (row, col) in self.selection_cells() {
            match target {
                PaletteTarget::Text => self.set_cell_fg(row, col, color),
                PaletteTarget::Fill => self.set_cell_bg(row, col, color),
                PaletteTarget::Border => self.set_cell_border_color(row, col, color),
            }
        }
    }

    /// Copy one row's FORMATTING onto another: cell colours, borders,
    /// alignment, and the row height — never the values. A row inserted into
    /// a coloured band must belong to the band, exactly as Excel inherits
    /// the neighbour's formatting.
    fn inherit_row_style(&mut self, from_row: usize, to_row: usize) {
        for col in 0..self.num_cols {
            match self.cell_styles.get(&(from_row, col)).copied() {
                Some(style) => {
                    self.cell_styles.insert((to_row, col), style);
                }
                None => {
                    self.cell_styles.remove(&(to_row, col));
                }
            }
        }
        match self.row_heights.get(&from_row).copied() {
            Some(height) => {
                self.row_heights.insert(to_row, height);
            }
            None => {
                self.row_heights.remove(&to_row);
            }
        }
    }

    /// Insert an empty row above the cursor; the cursor stays on its data,
    /// which is now one row further down.
    pub fn insert_row_above_cursor(&mut self) {
        self.record_undo();
        self.mark_dirty();
        if self.cursor_row == 0 {
            // insert_row_after only knows "after", so row 0 gets the row
            // inserted after it and then swaps contents outward — cheaper to
            // insert after row 0 and shift row 0 down by moving its cells.
            self.insert_row_after(0);
            let cols: Vec<usize> = (0..self.num_cols).collect();
            for col in cols {
                if let Some(value) = self.cells.remove(&(0, col)) {
                    self.cells.insert((1, col), value);
                }
                if let Some(style) = self.cell_styles.remove(&(0, col)) {
                    self.cell_styles.insert((1, col), style);
                }
                if let Some(formula) = self.formulas.remove(&(0, col)) {
                    self.formulas.insert((1, col), formula);
                }
            }
            self.cursor_row = 1;
            self.inherit_row_style(1, 0);
            return;
        } else {
            self.insert_row_after(self.cursor_row - 1);
            self.cursor_row += 1;
        }
        // The freshly inserted row sits right above the cursor's row now.
        let source = self.cursor_row;
        self.inherit_row_style(source, source - 1);
    }

    /// Insert an empty row below the cursor.
    pub fn insert_row_below_cursor(&mut self) {
        self.record_undo();
        self.mark_dirty();
        self.insert_row_after(self.cursor_row);
        self.inherit_row_style(self.cursor_row, self.cursor_row + 1);
    }

    /// Delete the cursor's row.
    pub fn delete_cursor_row(&mut self) {
        self.record_undo();
        self.mark_dirty();
        self.delete_row(self.cursor_row);
        self.cursor_row = self.cursor_row.min(self.num_rows.saturating_sub(1));
    }

    /// Copy the cursor's whole row to both clipboards, without disturbing
    /// the current selection mode.
    pub fn copy_cursor_row(&mut self) {
        let row = self.cursor_row;
        let previous = self.selected_rows;
        self.selected_rows = Some((row, row));
        self.copy_selection();
        self.selected_rows = previous;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_above_pushes_the_cursor_row_down() {
        let mut sheet = Spreadsheet::new();
        sheet.set_cell(0, 0, "bas".into());
        sheet.set_cell(1, 0, "orta".into());
        sheet.cursor_row = 1;

        sheet.insert_row_above_cursor();

        assert_eq!(sheet.get_cell(1, 0), "", "the new row is empty");
        assert_eq!(sheet.get_cell(2, 0), "orta", "the data moved down");
        assert_eq!(sheet.cursor_row, 2, "the cursor follows its data");

        assert!(sheet.undo());
        assert_eq!(sheet.get_cell(1, 0), "orta", "undo restores the layout");
    }

    #[test]
    fn insert_above_the_first_row_works_too() {
        let mut sheet = Spreadsheet::new();
        sheet.set_cell(0, 0, "ilk".into());
        sheet.cursor_row = 0;

        sheet.insert_row_above_cursor();

        assert_eq!(sheet.get_cell(0, 0), "");
        assert_eq!(sheet.get_cell(1, 0), "ilk");
    }

    #[test]
    fn delete_row_removes_the_cursor_row_and_undo_brings_it_back() {
        let mut sheet = Spreadsheet::new();
        sheet.set_cell(0, 0, "kalan".into());
        sheet.set_cell(1, 0, "giden".into());
        sheet.cursor_row = 1;

        sheet.delete_cursor_row();
        assert_eq!(sheet.get_cell(1, 0), "");

        assert!(sheet.undo());
        assert_eq!(sheet.get_cell(1, 0), "giden");
    }

    #[test]
    fn palette_colours_the_selection_and_undo_reverts_it() {
        let mut sheet = Spreadsheet::new();
        sheet.select_cell(2, 3);
        sheet.toolbar_palette = Some(PaletteTarget::Fill);

        assert!(sheet.apply_toolbar_action(ToolbarAction::PaletteColor(2)));
        assert_eq!(
            sheet.get_cell_style(2, 3).bg,
            Some(COLOR_PALETTE[2].0),
            "the swatch paints the cell"
        );

        assert!(sheet.undo());
        assert_eq!(sheet.get_cell_style(2, 3).bg, None);
    }

    #[test]
    fn border_palette_colours_the_lines_not_the_text() {
        let mut sheet = Spreadsheet::new();
        sheet.select_cell(1, 1);
        sheet.toolbar_palette = Some(PaletteTarget::Border);

        assert!(sheet.apply_toolbar_action(ToolbarAction::PaletteColor(2)));
        let style = sheet.get_cell_style(1, 1);
        assert_eq!(style.border_color, Some(COLOR_PALETTE[2].0));
        assert_eq!(style.fg, None, "the line colour must not touch the ink");
        assert_eq!(style.bg, None);

        assert!(sheet.undo());
        assert_eq!(sheet.get_cell_style(1, 1).border_color, None);
    }

    #[test]
    fn inserted_rows_inherit_the_clicked_rows_colours_not_its_values() {
        use ratatui::style::Color;
        let mut sheet = Spreadsheet::new();
        sheet.set_cell(1, 0, "veri".into());
        sheet.set_cell_bg(1, 0, Some(Color::Green));
        sheet.set_row_height(1, 2);
        sheet.cursor_row = 1;

        sheet.insert_row_below_cursor();
        assert_eq!(sheet.get_cell(2, 0), "", "values never copy");
        assert_eq!(
            sheet.get_cell_style(2, 0).bg,
            Some(Color::Green),
            "the band's colour does"
        );
        assert_eq!(sheet.get_row_height(2), 2, "and so does the row height");

        sheet.insert_row_above_cursor();
        assert_eq!(sheet.cursor_row, 2, "cursor follows its data");
        assert_eq!(
            sheet.get_cell_style(1, 0).bg,
            Some(Color::Green),
            "the row inserted above wears the band too"
        );
        assert_eq!(sheet.get_cell(1, 0), "");
    }

    #[test]
    fn geometry_resolves_a_click_to_its_button() {
        let geometry = ToolbarGeometry {
            buttons: vec![
                (Rect::new(0, 4, 6, 1), ToolbarAction::Undo),
                (Rect::new(6, 4, 6, 1), ToolbarAction::Redo),
            ],
        };
        assert_eq!(geometry.action_at(1, 4), Some(ToolbarAction::Undo));
        assert_eq!(geometry.action_at(7, 4), Some(ToolbarAction::Redo));
        assert_eq!(geometry.action_at(7, 5), None, "wrong row misses");
        assert_eq!(geometry.action_at(40, 4), None);
    }
}
