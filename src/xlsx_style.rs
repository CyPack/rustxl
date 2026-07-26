//! Mapping workbook formatting into the grid's own style model.
//!
//! A Hasrapport or Lasschema is unreadable as bare text: the colour blocks ARE
//! the document — a red cell is a fault, a green one a completed connection,
//! and the row and column sizing is what groups them. Loading only the values
//! showed a grey grid that answered none of the questions the file exists for.
//!
//! The mapping is deliberately lossy in one direction only: everything the
//! grid can draw (fill colour, font colour, bold, alignment, column width,
//! row height) is carried over; everything it cannot (borders, number
//! formats, conditional formatting) stays in the source workbook and
//! round-trips untouched through the save path, which never rebuilds styles.

use ratatui::style::Color;
use umya_spreadsheet::{PatternValues, Style};

use crate::constants::{MAX_COL_WIDTH, MAX_ROW_HEIGHT, MIN_COL_WIDTH, MIN_ROW_HEIGHT};
use crate::types::{CellStyle, DataType, TextAlignment, VerticalAlignment};

/// One Excel row of default height (15pt) is one terminal row.
const POINTS_PER_TERMINAL_ROW: f64 = 15.0;

/// The grid style a workbook style paints, or `None` when the cell carries
/// nothing the grid can draw — so unstyled cells stay off the style map and
/// the renderer keeps its fast default path.
pub fn cell_style_from(style: &Style) -> Option<CellStyle> {
    let bg = solid_fill_color(style);
    let font = style.get_font();
    let fg = font.and_then(|font| color_from_argb(font.get_color().argb()));
    let bold = font.is_some_and(|font| font.get_bold());
    let (alignment, vertical_alignment) = alignments_from(style);
    let (border_left, border_right, border_top, border_bottom) = borders_from(style);
    let data_type = date_data_type(style);

    if bg.is_none()
        && fg.is_none()
        && !bold
        && alignment.is_none()
        && vertical_alignment.is_none()
        && data_type.is_none()
        && !(border_left || border_right || border_top || border_bottom)
    {
        return None;
    }
    Some(CellStyle {
        fg,
        bg,
        bold,
        alignment,
        vertical_alignment,
        data_type,
        border_left,
        border_right,
        border_top,
        border_bottom,
        // The file's border COLOURS collapse onto the default strong line;
        // this channel belongs to the user's own line-colouring tool.
        border_color: None,
    })
}

/// Which sides the workbook draws a border on. Any style other than "none"
/// counts — the grid has one line weight, so thin, medium and double all
/// collapse onto it; what matters is WHERE the file drew table lines.
fn borders_from(style: &Style) -> (bool, bool, bool, bool) {
    let Some(borders) = style.get_borders() else {
        return (false, false, false, false);
    };
    let drawn = |border: &umya_spreadsheet::Border| {
        border.get_border_style() != umya_spreadsheet::Border::BORDER_NONE
    };
    (
        drawn(borders.get_left()),
        drawn(borders.get_right()),
        drawn(borders.get_top()),
        drawn(borders.get_bottom()),
    )
}

/// The visible background of a solid pattern fill.
///
/// Only `solid` counts: in a solid fill the FOREGROUND colour is the one the
/// cell shows (the OOXML naming is the trap here), while `gray125` and the
/// other hatches are texture, and painting their colour as a full block would
/// invent a highlight the file does not have.
fn solid_fill_color(style: &Style) -> Option<Color> {
    let pattern = style.get_fill()?.get_pattern_fill()?;
    if *pattern.get_pattern_type() != PatternValues::Solid {
        return None;
    }
    color_from_argb(pattern.get_foreground_color()?.argb())
}

/// An all-zero ARGB means "no colour recorded", not transparent black: umya
/// resolves rgb and indexed colours to real bytes and leaves theme-indexed
/// colours (which these files do not use) at the default. Excel writes true
/// black as FF000000, so alpha keeps black distinguishable from unset.
fn color_from_argb(argb: umya_spreadsheet::ARGB8) -> Option<Color> {
    if argb.a == 0 && argb.r == 0 && argb.g == 0 && argb.b == 0 {
        return None;
    }
    Some(Color::Rgb(argb.r, argb.g, argb.b))
}

fn alignments_from(style: &Style) -> (Option<TextAlignment>, Option<VerticalAlignment>) {
    use umya_spreadsheet::{HorizontalAlignmentValues, VerticalAlignmentValues};
    let Some(alignment) = style.get_alignment() else {
        return (None, None);
    };
    let horizontal = match alignment.get_horizontal() {
        HorizontalAlignmentValues::Left => Some(TextAlignment::Left),
        HorizontalAlignmentValues::Center | HorizontalAlignmentValues::CenterContinuous => {
            Some(TextAlignment::Center)
        }
        HorizontalAlignmentValues::Right => Some(TextAlignment::Right),
        // General lets the renderer keep its numbers-right, text-left rule,
        // and the justify/fill/distributed family has no grid equivalent.
        _ => None,
    };
    let vertical = match alignment.get_vertical() {
        VerticalAlignmentValues::Top => Some(VerticalAlignment::Top),
        VerticalAlignmentValues::Center => Some(VerticalAlignment::Center),
        VerticalAlignmentValues::Bottom => Some(VerticalAlignment::Bottom),
        _ => None,
    };
    (horizontal, vertical)
}

/// A number format that renders its value as a calendar date.
///
/// Without this a date column shows the raw Excel serial — `46223` where the
/// planner wrote a work date — because a workbook stores dates as day counts
/// and the FORMAT is the only thing that says so. Detection reads the format
/// code: quoted literals and `[colour]` sections say nothing about the value,
/// so they are stripped, and what remains is a date format when it spells
/// year or day tokens. (`m` alone is ambiguous with minutes and `0.00` has
/// no letters at all, so neither can trigger it.)
fn date_data_type(style: &Style) -> Option<DataType> {
    let code = style.get_numbering_format()?.get_format_code();
    let mut cleaned = String::with_capacity(code.len());
    let mut chars = code.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                for inner in chars.by_ref() {
                    if inner == '"' {
                        break;
                    }
                }
            }
            '[' => {
                for inner in chars.by_ref() {
                    if inner == ']' {
                        break;
                    }
                }
            }
            '\\' => {
                let _ = chars.next();
            }
            _ => cleaned.push(ch.to_ascii_lowercase()),
        }
    }
    (cleaned.contains('y') || cleaned.contains('d')).then_some(DataType::Date)
}

/// One Excel date serial as `yyyy-mm-dd`, the format the reference files
/// themselves use. Day 0 is 1899-12-30 in the 1900 system, which quietly
/// absorbs Excel's fictional 1900-02-29 for every date after March 1900 —
/// the only range these documents live in.
pub fn format_excel_date_serial(serial: f64) -> Option<String> {
    if !serial.is_finite() || !(1.0..=2_958_465.0).contains(&serial) {
        return None;
    }
    // Whole days since 1970-01-01 (Excel serial 25569), then civil-from-days
    // (Howard Hinnant's algorithm) — no date crate needed for y/m/d alone.
    let days = (serial.trunc() as i64) - 25_569;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

/// Excel column width in character units, as terminal cells.
///
/// The unit is already "how many digits fit", which is exactly what a
/// terminal column is, so the mapping is a round and a clamp. The clamp keeps
/// a decorative 100-wide column from pushing the whole sheet off screen.
pub fn col_width_cells(excel_width: f64) -> u16 {
    if !excel_width.is_finite() || excel_width <= 0.0 {
        return MIN_COL_WIDTH;
    }
    (excel_width.round() as u16).clamp(MIN_COL_WIDTH, MAX_COL_WIDTH)
}

/// Excel row height in points, as terminal rows.
///
/// 15pt is the default single row. Anything taller earns extra rows in
/// proportion, so a title row set to 30pt draws two rows tall — which is the
/// visual grouping the file's author built.
pub fn row_height_cells(points: f64) -> u16 {
    if !points.is_finite() || points <= 0.0 {
        return MIN_ROW_HEIGHT;
    }
    ((points / POINTS_PER_TERMINAL_ROW).round() as u16).clamp(MIN_ROW_HEIGHT, MAX_ROW_HEIGHT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style_with_solid_fill(argb: &str) -> Style {
        let mut style = Style::default();
        style.set_background_color(argb.to_string());
        style
    }

    #[test]
    fn a_solid_fill_becomes_the_cell_background() {
        let style = style_with_solid_fill("FFFF0000");
        let cell = cell_style_from(&style).expect("a red fill is a style");
        assert_eq!(cell.bg, Some(Color::Rgb(255, 0, 0)));
    }

    #[test]
    fn a_cell_with_no_formatting_maps_to_none() {
        assert!(cell_style_from(&Style::default()).is_none());
    }

    #[test]
    fn font_color_and_bold_are_carried() {
        let mut style = Style::default();
        let font = style.get_font_mut();
        font.get_color_mut().set_argb_str("FF0070C0");
        font.set_bold(true);
        let cell = cell_style_from(&style).expect("a coloured bold font is a style");
        assert_eq!(cell.fg, Some(Color::Rgb(0, 0x70, 0xC0)));
        assert!(cell.bold);
    }

    #[test]
    fn black_text_is_black_not_unset() {
        let mut style = Style::default();
        style
            .get_font_mut()
            .get_color_mut()
            .set_argb_str("FF000000");
        let cell = cell_style_from(&style).expect("explicit black is a style");
        assert_eq!(cell.fg, Some(Color::Rgb(0, 0, 0)));
    }

    #[test]
    fn center_alignment_is_carried() {
        use umya_spreadsheet::{HorizontalAlignmentValues, VerticalAlignmentValues};
        let mut style = Style::default();
        let alignment = style.get_alignment_mut();
        alignment.set_horizontal(HorizontalAlignmentValues::Center);
        alignment.set_vertical(VerticalAlignmentValues::Center);
        let cell = cell_style_from(&style).expect("alignment is a style");
        assert_eq!(cell.alignment, Some(TextAlignment::Center));
        assert_eq!(cell.vertical_alignment, Some(VerticalAlignment::Center));
    }

    #[test]
    fn workbook_borders_are_carried_per_side() {
        use umya_spreadsheet::BorderStyleValues;
        let mut style = Style::default();
        let borders = style.get_borders_mut();
        borders.get_bottom_mut().set_style(BorderStyleValues::Thin);
        borders.get_right_mut().set_style(BorderStyleValues::Medium);
        let cell = cell_style_from(&style).expect("a bordered cell is a style");
        assert!(cell.border_bottom);
        assert!(cell.border_right);
        assert!(!cell.border_left);
        assert!(!cell.border_top);
    }

    #[test]
    fn date_formats_are_detected_and_number_formats_are_not() {
        let date = |code: &str| {
            let mut style = Style::default();
            style.get_numbering_format_mut().set_format_code(code);
            date_data_type(&style)
        };
        assert_eq!(
            date("yyyy\\-mm\\-dd"),
            Some(DataType::Date),
            "the reference file's own code"
        );
        assert_eq!(date("dd/mm/yyyy"), Some(DataType::Date));
        assert_eq!(date("d-mmm-yy"), Some(DataType::Date));
        assert_eq!(date("0.00"), None, "a plain number");
        assert_eq!(date("General"), None);
        assert_eq!(date("@"), None, "text format");
        assert_eq!(date("h:mm"), None, "time alone stays a number for now");
        assert_eq!(
            date("\"day\" 0.0"),
            None,
            "a quoted literal must not smuggle a d into detection"
        );
        assert_eq!(date("[Red]0.00"), None, "colour sections say nothing");
    }

    #[test]
    fn excel_date_serials_convert_to_civil_dates() {
        assert_eq!(
            format_excel_date_serial(25_569.0).as_deref(),
            Some("1970-01-01")
        );
        assert_eq!(
            format_excel_date_serial(45_658.0).as_deref(),
            Some("2025-01-01")
        );
        assert_eq!(
            format_excel_date_serial(46_223.0).as_deref(),
            Some("2026-07-20"),
            "the serial from the live screenshot"
        );
        assert_eq!(format_excel_date_serial(0.0), None, "out of range");
        assert_eq!(format_excel_date_serial(f64::NAN), None);
    }

    #[test]
    fn column_widths_round_and_clamp() {
        assert_eq!(col_width_cells(8.43), 8, "the Excel default width");
        assert_eq!(col_width_cells(2.0), MIN_COL_WIDTH);
        assert_eq!(col_width_cells(100.0), MAX_COL_WIDTH);
        assert_eq!(col_width_cells(f64::NAN), MIN_COL_WIDTH);
    }

    #[test]
    fn row_heights_scale_by_the_default_point_size() {
        assert_eq!(row_height_cells(15.0), 1, "the Excel default height");
        assert_eq!(row_height_cells(30.0), 2);
        assert_eq!(row_height_cells(300.0), MAX_ROW_HEIGHT);
        assert_eq!(row_height_cells(0.0), MIN_ROW_HEIGHT);
    }
}
