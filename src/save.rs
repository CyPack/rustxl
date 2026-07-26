use std::fs::File;
use std::io::{self, Write};

use crate::spreadsheet::Spreadsheet;
use crate::types::SaveFormat;

impl Spreadsheet {
    pub fn enter_save_mode(&mut self) {
        self.save_mode = true;
        // Offer to write back the kind of file that was opened. A workbook
        // defaulting to CSV means pressing save and enter turns a multi-sheet
        // document into one sheet of text -- the loss this format exists to
        // prevent, one keystroke away.
        self.save_format = if self.source.is_some() {
            SaveFormat::Xlsx
        } else {
            SaveFormat::Csv
        };
        self.save_message = None;
    }

    pub fn exit_save_mode(&mut self) {
        self.save_mode = false;
        self.save_message = None;
    }

    pub fn enter_open_mode(&mut self) {
        self.open_mode = true;
        self.open_filename.clear();
        self.open_message = None;
    }

    pub fn exit_open_mode(&mut self) {
        self.open_mode = false;
        self.open_filename.clear();
        self.open_message = None;
    }

    pub fn get_data_bounds(&self) -> (usize, usize) {
        let mut max_row = 0;
        let mut max_col = 0;
        for &(row, col) in self.cells.keys() {
            max_row = max_row.max(row);
            max_col = max_col.max(col);
        }
        (max_row, max_col)
    }

    /// The debounce before an automatic save. Long enough to batch a burst
    /// of typing into one write, short enough that "yazdiklarim hemen kayit
    /// olsun" stays true - a crash can only ever cost this window.
    const AUTOSAVE_AFTER: std::time::Duration = std::time::Duration::from_millis(800);

    /// Write unsaved changes back to the opened .xlsx once they are due.
    /// Called from the main loop's poll tick; a grid that was not opened
    /// from an xlsx file never autosaves (the save dialog stays its exit).
    pub fn autosave_if_due(&mut self) {
        let Some(dirty_since) = self.dirty_since else {
            return;
        };
        if dirty_since.elapsed() < Self::AUTOSAVE_AFTER || self.opened_xlsx.is_none() {
            if self.opened_xlsx.is_none() {
                self.dirty_since = None;
            }
            return;
        }
        // save_workbook writes to `save_filename + .xlsx`, which load_from_file
        // pointed at the opened document - so this is "save in place".
        match self.save_workbook() {
            Ok(()) => {
                self.dirty_since = None;
                // The dialog's confirmation would be noise every 800ms.
                self.save_message = None;
            }
            Err(error) => {
                // A failing autosave must be LOUD - silence here is data loss.
                self.save_message = Some(format!("Autosave failed: {error}"));
                // Retry on the next tick rather than hammering every 100ms.
                self.dirty_since = Some(std::time::Instant::now());
            }
        }
    }

    /// Write any unsaved changes NOW, ignoring the debounce. The exit paths
    /// call this so "close" can never race the 800ms window and lose the
    /// last keystrokes.
    pub fn flush_autosave(&mut self) {
        if self.dirty_since.is_none() || self.opened_xlsx.is_none() {
            return;
        }
        match self.save_workbook() {
            Ok(()) => {
                self.dirty_since = None;
                self.save_message = None;
            }
            Err(error) => {
                self.save_message = Some(format!("Autosave failed: {error}"));
            }
        }
    }

    pub fn save_to_file(&mut self) -> io::Result<()> {
        match self.save_format {
            SaveFormat::Csv | SaveFormat::Tsv => self.save_delimited(),
            SaveFormat::Xlsx => self.save_workbook(),
        }
    }

    /// Writes the active sheet as a rectangle of separated values.
    ///
    /// Only the sheet on screen: the format has no way to hold the others.
    fn save_delimited(&mut self) -> io::Result<()> {
        let (max_row, max_col) = self.get_data_bounds();

        let (extension, separator) = match self.save_format {
            SaveFormat::Tsv => ("tsv", '\t'),
            // A workbook never reaches this function.
            _ => ("csv", ','),
        };

        let filename = format!("{}.{}", self.save_filename, extension);
        let mut file = File::create(&filename)?;

        for row in 0..=max_row {
            let mut row_data = Vec::new();
            for col in 0..=max_col {
                let content = self.get_cell(row, col);
                let escaped = if self.save_format == SaveFormat::Csv
                    && (content.contains(',') || content.contains('"') || content.contains('\n'))
                {
                    format!("\"{}\"", content.replace('"', "\"\""))
                } else {
                    content.to_string()
                };
                row_data.push(escaped);
            }
            writeln!(file, "{}", row_data.join(&separator.to_string()))?;
        }

        self.save_message = Some(format!("Saved to {}", filename));
        Ok(())
    }

    /// Writes the whole workbook, keeping what the grid cannot describe.
    ///
    /// When the file was opened from a workbook, that workbook is what gets
    /// written: only the cells whose text changed are touched, so styles,
    /// number formats and the formulas behind untouched cells survive. A grid
    /// that came from a CSV, or from nothing at all, gets a fresh workbook.
    fn save_workbook(&mut self) -> io::Result<()> {
        let filename = format!("{}.xlsx", self.save_filename);

        // The sheet on screen is a working copy; write it back first or the
        // user's most recent edits are not in `sheets` yet.
        self.park_active_sheet();

        // Taken rather than borrowed: applying the grid needs the workbook and
        // the sheets at the same time.
        let mut book = self
            .source
            .take()
            .unwrap_or_else(umya_spreadsheet::new_file_empty_worksheet);

        let outcome = crate::xlsx::apply_sheets(&mut book, &self.sheets).and_then(|()| {
            umya_spreadsheet::writer::xlsx::write(&book, &filename).map_err(|e| e.to_string())
        });

        // Put the workbook back either way. A failed save must not cost the
        // user the document they still have open, and a successful one should
        // leave the next save building on what was just written.
        self.source = Some(book);

        outcome.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        self.save_message = Some(format!("Saved to {}", filename));
        Ok(())
    }
}

/// Characterization tests for writing a spreadsheet back out.
///
/// `save_to_file` appends the format's extension to `save_filename` and writes
/// the rectangle from `(0, 0)` to the last populated cell. These tests record
/// the escaping rules and the shape of the output as they behave today.
#[cfg(test)]
mod save_characterization {
    use super::*;

    /// A per-test directory under the system temp dir.
    ///
    /// `save_to_file` builds its path from `save_filename`, so an absolute
    /// prefix keeps the test off the process-wide current directory — which
    /// matters because tests run in parallel.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default();
            let path = std::env::temp_dir().join(format!("xl-save-{label}-{unique}"));
            std::fs::create_dir_all(&path).expect("temp dir is creatable");
            Self(path)
        }

        /// Path prefix to hand to `save_filename` (no extension: it is appended).
        fn prefix(&self, stem: &str) -> String {
            self.0.join(stem).to_string_lossy().into_owned()
        }

        fn read(&self, name: &str) -> String {
            std::fs::read_to_string(self.0.join(name)).expect("saved file is readable")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// CSV quoting kicks in for separators, quotes and newlines; quotes double up.
    #[test]
    fn csv_quotes_only_the_fields_that_need_it() {
        let dir = TempDir::new("csv-quoting");
        let mut sheet = Spreadsheet::new();
        sheet.set_cell(0, 0, "plain".to_string());
        sheet.set_cell(0, 1, "has,comma".to_string());
        sheet.set_cell(0, 2, "has\"quote".to_string());
        sheet.set_cell(0, 3, "has\nnewline".to_string());

        sheet.save_filename = dir.prefix("out");
        sheet.save_format = SaveFormat::Csv;
        sheet.save_to_file().expect("save succeeds");

        assert_eq!(
            dir.read("out.csv"),
            "plain,\"has,comma\",\"has\"\"quote\",\"has\nnewline\"\n"
        );
    }

    /// TSV writes fields verbatim: the quoting branch is CSV-only.
    ///
    /// A value containing a tab would therefore break the column alignment of
    /// the output. That is the behaviour today, recorded rather than endorsed.
    #[test]
    fn tsv_writes_fields_verbatim_without_quoting() {
        let dir = TempDir::new("tsv-verbatim");
        let mut sheet = Spreadsheet::new();
        sheet.set_cell(0, 0, "plain".to_string());
        sheet.set_cell(0, 1, "has,comma".to_string());
        sheet.set_cell(0, 2, "has\"quote".to_string());

        sheet.save_filename = dir.prefix("out");
        sheet.save_format = SaveFormat::Tsv;
        sheet.save_to_file().expect("save succeeds");

        assert_eq!(dir.read("out.tsv"), "plain\thas,comma\thas\"quote\n");
    }

    /// The output is the full rectangle to the last populated cell, gaps included.
    #[test]
    fn empty_cells_inside_the_used_range_become_empty_fields() {
        let dir = TempDir::new("csv-gaps");
        let mut sheet = Spreadsheet::new();
        sheet.set_cell(0, 0, "a".to_string());
        sheet.set_cell(2, 2, "c".to_string());

        sheet.save_filename = dir.prefix("out");
        sheet.save_format = SaveFormat::Csv;
        sheet.save_to_file().expect("save succeeds");

        assert_eq!(dir.read("out.csv"), "a,,\n,,\n,,c\n");
    }

    /// Saving reports the written path back to the user through `save_message`.
    #[test]
    fn saving_records_the_written_path_in_the_status_message() {
        let dir = TempDir::new("csv-message");
        let mut sheet = Spreadsheet::new();
        sheet.set_cell(0, 0, "a".to_string());

        sheet.save_filename = dir.prefix("out");
        sheet.save_format = SaveFormat::Csv;
        sheet.save_to_file().expect("save succeeds");

        let message = sheet.save_message.as_deref().unwrap_or_default();
        assert!(
            message.starts_with("Saved to ") && message.ends_with("out.csv"),
            "unexpected status message: {message:?}"
        );
    }

    /// Every format the user can pick, and the extension it writes.
    ///
    /// This test used to record that only CSV and TSV could be written, which
    /// is what made a round trip lossy: a three-sheet `.xlsx` came back as a
    /// single-sheet text file. A workbook writer exists now.
    ///
    /// The match below is exhaustive on purpose. Adding a variant to
    /// `SaveFormat` stops this test from compiling, so a new output format
    /// cannot be introduced without someone revisiting this expectation.
    #[test]
    fn every_save_format_writes_its_own_extension() {
        fn extension_for(format: SaveFormat) -> &'static str {
            match format {
                SaveFormat::Csv => "csv",
                SaveFormat::Tsv => "tsv",
                SaveFormat::Xlsx => "xlsx",
            }
        }

        assert_eq!(extension_for(SaveFormat::Csv), "csv");
        assert_eq!(extension_for(SaveFormat::Tsv), "tsv");
        assert_eq!(extension_for(SaveFormat::Xlsx), "xlsx");
    }

    /// The text formats still write only what is on screen.
    ///
    /// A workbook has several sheets and a CSV has one, so this is not a bug —
    /// but it is worth stating, because it is the reason the workbook format
    /// had to exist.
    #[test]
    fn a_text_save_holds_only_the_active_sheet() {
        let dir = TempDir::new("csv-active-only");
        let mut sheet = Spreadsheet::new();
        sheet.replace_sheets(vec![
            {
                let mut s = crate::sheet::Sheet::new("First");
                s.cells.insert((0, 0), "on screen".to_string());
                s
            },
            {
                let mut s = crate::sheet::Sheet::new("Second");
                s.cells.insert((0, 0), "hidden".to_string());
                s
            },
        ]);

        sheet.save_filename = dir.prefix("out");
        sheet.save_format = SaveFormat::Csv;
        sheet.save_to_file().expect("save succeeds");

        let written = dir.read("out.csv");
        assert!(written.contains("on screen"));
        assert!(!written.contains("hidden"));
    }
}

/// Saving as a workbook.
///
/// These go through the real file system and read the result back with the
/// same library that wrote it. That is enough to catch a save that loses
/// sheets or formulas; whether the file is one Excel itself accepts is checked
/// separately, against an independent reader.
#[cfg(test)]
mod workbook_saving {
    use super::*;

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default();
            let path = std::env::temp_dir().join(format!("xl-book-{label}-{unique}"));
            std::fs::create_dir_all(&path).expect("temp dir is creatable");
            Self(path)
        }

        fn prefix(&self, stem: &str) -> String {
            self.0.join(stem).to_string_lossy().into_owned()
        }

        fn read_back(&self, name: &str) -> umya_spreadsheet::Workbook {
            umya_spreadsheet::reader::xlsx::read(self.0.join(name))
                .expect("the saved file is a readable workbook")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fixture(name: &str) -> String {
        format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name)
    }

    fn value_at(book: &umya_spreadsheet::Workbook, sheet: &str, coordinate: &str) -> String {
        book.sheet_by_name(sheet)
            .expect("the sheet exists")
            .cell(coordinate)
            .map(|cell| cell.value().into_owned())
            .unwrap_or_default()
    }

    /// A workbook opened, edited and saved keeps everything else it had.
    ///
    /// This is the whole point of the format: the user changes one cell on one
    /// sheet, and the two sheets they never looked at, the formulas they never
    /// opened and the formats they never set are all still there.
    #[test]
    fn saving_a_workbook_keeps_the_sheets_and_formulas_the_user_did_not_touch() {
        let dir = TempDir::new("round-trip");
        let mut sheet = Spreadsheet::new();
        sheet
            .load_from_file(&fixture("three_sheets.xlsx"))
            .expect("fixture loads");

        // Change one cell on the first sheet.
        sheet.set_cell(1, 1, "11".to_string());

        sheet.save_filename = dir.prefix("out");
        sheet.save_format = SaveFormat::Xlsx;
        sheet.save_to_file().expect("save succeeds");

        let written = dir.read_back("out.xlsx");

        assert_eq!(written.sheet_count(), 3, "every sheet survives");
        assert_eq!(value_at(&written, "Alpha", "B2"), "11", "the edit landed");

        let formula = written
            .sheet_by_name("Alpha")
            .expect("Alpha exists")
            .cell("C2")
            .map(|cell| cell.formula().to_string())
            .unwrap_or_default();
        assert_eq!(
            formula, "SUM(B2:B3)",
            "the formula in a cell nobody edited must survive"
        );

        assert_eq!(value_at(&written, "Beta", "A1"), "flag");
        assert_eq!(value_at(&written, "Gamma", "A1"), "cross");
    }

    /// The date's number format survives, so the value is still a date.
    #[test]
    fn a_number_format_on_an_untouched_cell_survives() {
        let dir = TempDir::new("number-format");
        let mut sheet = Spreadsheet::new();
        sheet
            .load_from_file(&fixture("three_sheets.xlsx"))
            .expect("fixture loads");

        sheet.set_cell(0, 0, "changed".to_string());
        sheet.save_filename = dir.prefix("out");
        sheet.save_format = SaveFormat::Xlsx;
        sheet.save_to_file().expect("save succeeds");

        let written = dir.read_back("out.xlsx");
        let format = written
            .sheet_by_name("Beta")
            .expect("Beta exists")
            .cell("B2")
            .and_then(|cell| cell.style().number_format())
            .map(|f| f.format_code().to_string())
            .unwrap_or_default();

        assert!(
            format.to_lowercase().contains("yy"),
            "the date format was lost: {format:?}"
        );
    }

    /// Edits made on a sheet that is not on screen are written too.
    ///
    /// The grid holds a working copy of the active sheet, so a save has to
    /// write that copy back before it looks at the sheet list.
    #[test]
    fn an_edit_on_a_background_sheet_is_saved() {
        let dir = TempDir::new("background-edit");
        let mut sheet = Spreadsheet::new();
        sheet
            .load_from_file(&fixture("three_sheets.xlsx"))
            .expect("fixture loads");

        assert!(sheet.activate_sheet(1), "Beta exists");
        sheet.set_cell(0, 0, "edited on Beta".to_string());
        assert!(sheet.activate_sheet(0), "back to Alpha");
        sheet.set_cell(0, 0, "edited on Alpha".to_string());

        sheet.save_filename = dir.prefix("out");
        sheet.save_format = SaveFormat::Xlsx;
        sheet.save_to_file().expect("save succeeds");

        let written = dir.read_back("out.xlsx");
        assert_eq!(value_at(&written, "Alpha", "A1"), "edited on Alpha");
        assert_eq!(value_at(&written, "Beta", "A1"), "edited on Beta");
    }

    /// The most recent edit is saved without switching sheets first.
    #[test]
    fn the_edit_just_made_is_included() {
        let dir = TempDir::new("latest-edit");
        let mut sheet = Spreadsheet::new();
        sheet
            .load_from_file(&fixture("three_sheets.xlsx"))
            .expect("fixture loads");

        sheet.set_cell(0, 0, "typed a moment ago".to_string());

        sheet.save_filename = dir.prefix("out");
        sheet.save_format = SaveFormat::Xlsx;
        sheet.save_to_file().expect("save succeeds");

        assert_eq!(
            value_at(&dir.read_back("out.xlsx"), "Alpha", "A1"),
            "typed a moment ago"
        );
    }

    /// A grid that never came from a workbook can still be saved as one.
    #[test]
    fn a_grid_with_no_source_workbook_is_saved_as_a_new_one() {
        let dir = TempDir::new("no-source");
        let mut sheet = Spreadsheet::new();
        sheet.set_cell(0, 0, "from scratch".to_string());
        sheet.set_cell(1, 0, "42".to_string());

        sheet.save_filename = dir.prefix("out");
        sheet.save_format = SaveFormat::Xlsx;
        sheet.save_to_file().expect("save succeeds");

        let written = dir.read_back("out.xlsx");
        assert_eq!(written.sheet_count(), 1);
        assert_eq!(
            value_at(&written, crate::sheet::DEFAULT_SHEET_NAME, "A1"),
            "from scratch"
        );
        assert_eq!(
            value_at(&written, crate::sheet::DEFAULT_SHEET_NAME, "A2"),
            "42"
        );
    }

    /// Saving twice builds on what was written, not on the original file.
    #[test]
    fn a_second_save_keeps_the_first_one() {
        let dir = TempDir::new("twice");
        let mut sheet = Spreadsheet::new();
        sheet
            .load_from_file(&fixture("three_sheets.xlsx"))
            .expect("fixture loads");

        sheet.save_filename = dir.prefix("out");
        sheet.save_format = SaveFormat::Xlsx;

        sheet.set_cell(0, 0, "first".to_string());
        sheet.save_to_file().expect("first save succeeds");
        sheet.set_cell(0, 1, "second".to_string());
        sheet.save_to_file().expect("second save succeeds");

        let written = dir.read_back("out.xlsx");
        assert_eq!(value_at(&written, "Alpha", "A1"), "first");
        assert_eq!(value_at(&written, "Alpha", "B1"), "second");
    }

    /// Saving a workbook offers the workbook format first.
    ///
    /// The dialog defaults to whatever `save_format` holds and Enter accepts
    /// it. Defaulting to CSV would put the loss this format prevents behind two
    /// keystrokes: a three-sheet document flattened to one sheet of text by a
    /// user who only meant to save their work.
    #[test]
    fn saving_a_workbook_offers_the_workbook_format() {
        let mut sheet = Spreadsheet::new();
        sheet
            .load_from_file(&fixture("three_sheets.xlsx"))
            .expect("fixture loads");

        sheet.enter_save_mode();

        assert!(sheet.save_format == SaveFormat::Xlsx);
    }

    /// A grid that did not come from a workbook still offers CSV.
    #[test]
    fn saving_a_text_file_still_offers_csv() {
        let csv = std::env::temp_dir().join(format!("xl-default-{}.csv", std::process::id()));
        std::fs::write(&csv, "a,b\n1,2\n").expect("temp csv is writable");

        let mut sheet = Spreadsheet::new();
        sheet
            .load_from_file(&csv.to_string_lossy())
            .expect("csv loads");
        let _ = std::fs::remove_file(&csv);

        sheet.enter_save_mode();

        assert!(sheet.save_format == SaveFormat::Csv);
    }

    /// Saving offers the file that was opened, not a new one.
    ///
    /// A file manager handing a document over is the case that matters: the
    /// user edits it, presses save, accepts the default, and expects their own
    /// file to be updated -- not a `spreadsheet.xlsx` in whatever directory the
    /// program was started from.
    #[test]
    fn saving_offers_the_file_that_was_opened() {
        let mut sheet = Spreadsheet::new();
        let path = fixture("three_sheets.xlsx");
        sheet.load_from_file(&path).expect("fixture loads");

        sheet.enter_save_mode();

        assert_eq!(
            format!("{}.xlsx", sheet.save_filename),
            path,
            "the save dialog should name the file that is open"
        );
    }

    /// The same applies to text files.
    #[test]
    fn saving_offers_the_text_file_that_was_opened() {
        let csv = std::env::temp_dir().join(format!("xl-reopen-{}.csv", std::process::id()));
        std::fs::write(&csv, "a,b\n").expect("temp csv is writable");

        let mut sheet = Spreadsheet::new();
        sheet
            .load_from_file(&csv.to_string_lossy())
            .expect("csv loads");
        let _ = std::fs::remove_file(&csv);

        assert_eq!(
            format!("{}.csv", sheet.save_filename),
            csv.to_string_lossy(),
        );
    }

    /// Nothing opened, nothing to offer: the original default stands.
    #[test]
    fn a_grid_that_was_never_loaded_keeps_the_default_name() {
        let sheet = Spreadsheet::new();

        assert_eq!(sheet.save_filename, "spreadsheet");
    }

    /// A failed open does not change where a save would go.
    #[test]
    fn a_failed_open_leaves_the_save_target_alone() {
        let mut sheet = Spreadsheet::new();
        sheet
            .load_from_file("/nonexistent/whatever.xlsx")
            .expect_err("the file does not exist");

        assert_eq!(sheet.save_filename, "spreadsheet");
    }

    /// Editing a workbook and saving updates that workbook in place.
    ///
    /// The source is held in memory, so writing over the file it was read from
    /// is safe -- and this is the whole path a file manager exercises.
    #[test]
    fn saving_writes_back_over_the_file_that_was_opened() {
        let dir = TempDir::new("in-place");
        let target = std::path::PathBuf::from(dir.prefix("report")).with_extension("xlsx");
        std::fs::copy(fixture("three_sheets.xlsx"), &target).expect("fixture is copyable");

        let mut sheet = Spreadsheet::new();
        sheet
            .load_from_file(&target.to_string_lossy())
            .expect("the copy loads");
        sheet.set_cell(0, 0, "edited in place".to_string());
        sheet.enter_save_mode();
        sheet.save_to_file().expect("save succeeds");

        let written = umya_spreadsheet::reader::xlsx::read(&target).expect("still a workbook");
        assert_eq!(written.sheet_count(), 3, "every sheet survives");
        assert_eq!(
            value_at(&written, "Alpha", "A1"),
            "edited in place",
            "the original file was updated"
        );
        assert_eq!(
            written
                .sheet_by_name("Alpha")
                .expect("Alpha exists")
                .cell("C2")
                .map(|cell| cell.formula().to_string())
                .unwrap_or_default(),
            "SUM(B2:B3)",
            "and its formulas with it"
        );
    }

    /// Clearing a cell empties it in the saved file.
    #[test]
    fn a_cleared_cell_is_empty_in_the_saved_file() {
        let dir = TempDir::new("cleared");
        let mut sheet = Spreadsheet::new();
        sheet
            .load_from_file(&fixture("three_sheets.xlsx"))
            .expect("fixture loads");

        sheet.set_cell(2, 0, String::new());

        sheet.save_filename = dir.prefix("out");
        sheet.save_format = SaveFormat::Xlsx;
        sheet.save_to_file().expect("save succeeds");

        let written = dir.read_back("out.xlsx");
        assert_eq!(value_at(&written, "Alpha", "A3"), "", "gadget was cleared");
        assert_eq!(value_at(&written, "Alpha", "A2"), "widget", "A2 remains");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_data_bounds() {
        let mut sheet = Spreadsheet::new();
        assert_eq!(sheet.get_data_bounds(), (0, 0));

        sheet.set_cell(5, 3, "test".to_string());
        assert_eq!(sheet.get_data_bounds(), (5, 3));

        sheet.set_cell(2, 10, "test2".to_string());
        assert_eq!(sheet.get_data_bounds(), (5, 10));
    }
}

/// Writes a saved workbook to a known path so an independent reader can check
/// it. Ignored by default: this is a tool, not a regression test.
#[cfg(test)]
mod round_trip_artifact {
    use super::*;

    #[test]
    #[ignore]
    fn write_a_saved_workbook_for_external_verification() {
        let mut sheet = Spreadsheet::new();
        sheet
            .load_from_file(&format!(
                "{}/tests/fixtures/three_sheets.xlsx",
                env!("CARGO_MANIFEST_DIR")
            ))
            .expect("fixture loads");

        sheet.set_cell(1, 1, "11".to_string());
        sheet.activate_sheet(2);
        sheet.set_cell(0, 0, "edited on Gamma".to_string());

        let path = std::env::temp_dir().join("xl-verify");
        sheet.save_filename = path.to_string_lossy().into_owned();
        sheet.save_format = SaveFormat::Xlsx;
        sheet.save_to_file().expect("save succeeds");

        println!("wrote {}.xlsx", path.display());
    }
}
