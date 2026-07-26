use ratatui::style::Color;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum TextAlignment {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum VerticalAlignment {
    #[default]
    Top,
    Center,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum DataType {
    #[default]
    Text,
    Number,
    Currency,
    Percentage,
    Date,
    Time,
}

#[derive(Clone, Copy, Default)]
pub struct CellStyle {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub alignment: Option<TextAlignment>,
    pub vertical_alignment: Option<VerticalAlignment>,
    pub data_type: Option<DataType>,
    /// Borders the workbook draws around this cell. A form's table outline is
    /// authored as borders, not colours, and reading only the fills left every
    /// table in a Hasrapport looking like loose text. These say which sides
    /// the FILE asked for; the renderer paints them strong, and everything
    /// else gets the faint default gridline a spreadsheet shows anyway.
    pub border_left: bool,
    pub border_right: bool,
    pub border_top: bool,
    pub border_bottom: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub enum VisualSubMode {
    Main,
    TextColor,
    BackgroundColor,
    ColumnWidth,
    RowHeight,
    TextAlignment,
    VerticalAlignment,
    FontSize,
    DataType,
}

#[derive(Clone, Copy, PartialEq)]
pub enum SaveFormat {
    Csv,
    Tsv,
    /// A workbook. Unlike the text formats this keeps every sheet, and — when
    /// the file was opened from one — everything the grid cannot describe.
    Xlsx,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowColumnSelectMode {
    None,
    RowSelect,
    ColumnSelect,
}
