//! Numerical export: the numbers behind a figure, written where a spreadsheet,
//! a scripting language or the next application can read them.
//!
//! Nothing here writes a rounded-for-display value: precision is a parameter,
//! and missing samples are written as `nan` rather than as zero.

use crate::text::format_number;
use ndarray::Array2;
use std::io::{BufWriter, Write};
use std::path::Path;
use ta_core::{Axis, CoreError, Result, SignalUnit, Spectrum, Trace};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delimiter {
    Comma,
    Tab,
    Space,
}

impl Delimiter {
    pub const fn as_char(self) -> char {
        match self {
            Delimiter::Comma => ',',
            Delimiter::Tab => '\t',
            Delimiter::Space => ' ',
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExportOptions {
    pub delimiter: Delimiter,
    /// Whether to write a header naming each column.
    pub header: bool,
    /// Digits after the decimal point.
    pub precision: usize,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            delimiter: Delimiter::Comma,
            header: true,
            precision: 6,
        }
    }
}

/// One `x`/`y` pair of columns.
#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    pub x_label: String,
    pub y_label: String,
    pub x: Vec<f64>,
    pub y: Vec<f64>,
}

impl Series {
    pub fn new(
        x_label: impl Into<String>,
        y_label: impl Into<String>,
        x: Vec<f64>,
        y: Vec<f64>,
    ) -> Self {
        Self {
            x_label: x_label.into(),
            y_label: y_label.into(),
            x,
            y,
        }
    }

    pub fn len(&self) -> usize {
        self.x.len().min(self.y.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl From<&Trace> for Series {
    fn from(trace: &Trace) -> Self {
        Series {
            // The label already says which wavelength was actually read.
            x_label: "Delay (ps)".to_string(),
            y_label: format!("ΔOD at {}", trace.label),
            x: trace.delay.clone(),
            y: trace.signal.clone(),
        }
    }
}

impl From<&Spectrum> for Series {
    fn from(spectrum: &Spectrum) -> Self {
        Series {
            x_label: "Wavelength (nm)".to_string(),
            y_label: format!("ΔOD at {}", spectrum.label),
            x: spectrum.wavelength.clone(),
            y: spectrum.signal.clone(),
        }
    }
}

fn create(path: &Path) -> Result<BufWriter<std::fs::File>> {
    let file = std::fs::File::create(path).map_err(|source| CoreError::Io {
        path: path.display().to_string(),
        source,
    })?;
    Ok(BufWriter::new(file))
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> CoreError + '_ {
    move |source| CoreError::Io {
        path: path.display().to_string(),
        source,
    }
}

/// Writes one `x`/`y` series. The number of rows is the shorter of the two
/// columns, so a caller cannot write a file whose columns disagree.
pub fn write_series(path: &Path, series: &Series, options: ExportOptions) -> Result<()> {
    let mut writer = create(path)?;
    let separator = options.delimiter.as_char();
    if options.header {
        writeln!(writer, "{}{separator}{}", series.x_label, series.y_label)
            .map_err(io(path))?;
    }
    for index in 0..series.len() {
        writeln!(
            writer,
            "{}{separator}{}",
            format_number(series.x[index], options.precision),
            format_number(series.y[index], options.precision),
        )
        .map_err(io(path))?;
    }
    writer.flush().map_err(io(path))
}

/// One curve in a multi-series export.
///
/// Curves measured on different grids cannot share a table without one of them
/// being interpolated, so a multi-series file is written as labelled blocks: one
/// `x`/`y` pair per curve, each at its own resolution. Nothing is resampled, and
/// the file says which curve is which.
#[derive(Clone, Debug, PartialEq)]
pub struct SeriesBlock {
    pub label: String,
    pub x_label: String,
    pub y_label: String,
    pub x: Vec<f64>,
    pub y: Vec<f64>,
}

impl SeriesBlock {
    pub fn new(
        label: impl Into<String>,
        x_label: impl Into<String>,
        y_label: impl Into<String>,
        x: Vec<f64>,
        y: Vec<f64>,
    ) -> Self {
        Self {
            label: label.into(),
            x_label: x_label.into(),
            y_label: y_label.into(),
            x,
            y,
        }
    }

    pub fn len(&self) -> usize {
        self.x.len().min(self.y.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Writes several curves into one self-describing file: the notes first, then a
/// `# series:` line and a header for each block.
///
/// Returns how many points were written.
pub fn write_blocks(
    path: &Path,
    blocks: &[SeriesBlock],
    notes: &[String],
    options: ExportOptions,
) -> Result<usize> {
    let mut writer = create(path)?;
    let separator = options.delimiter.as_char();
    for note in notes {
        writeln!(writer, "# {note}").map_err(io(path))?;
    }
    let mut written = 0usize;
    for block in blocks {
        writeln!(writer, "# series: {}", block.label).map_err(io(path))?;
        if options.header {
            writeln!(writer, "{}{separator}{}", block.x_label, block.y_label).map_err(io(path))?;
        }
        for index in 0..block.len() {
            writeln!(
                writer,
                "{}{separator}{}",
                format_number(block.x[index], options.precision),
                format_number(block.y[index], options.precision),
            )
            .map_err(io(path))?;
            written += 1;
        }
        writeln!(writer).map_err(io(path))?;
    }
    writer.flush().map_err(io(path))?;
    Ok(written)
}

/// Writes an axis as its own file, which §18 wants beside a matrix.
pub fn write_axis(path: &Path, axis: &Axis, options: ExportOptions) -> Result<()> {
    let mut writer = create(path)?;
    if options.header {
        writeln!(writer, "{} ({})", axis.name(), axis.unit().symbol()).map_err(io(path))?;
    }
    for value in axis.values() {
        writeln!(writer, "{}", format_number(*value, options.precision)).map_err(io(path))?;
    }
    writer.flush().map_err(io(path))
}

/// A matrix on its way out, with the axes that make its numbers meaningful.
pub struct MatrixExport<'a> {
    pub wavelength: &'a Axis,
    pub delay: &'a Axis,
    pub data: &'a Array2<f64>,
    pub unit: SignalUnit,
    /// The wavelengths to write, or every one. §18's "only the visible range".
    pub rows: Option<Vec<usize>>,
    /// The delays to write, or every one.
    pub columns: Option<Vec<usize>>,
    /// Lines written above the table: what the numbers are and where they came
    /// from, because a file that cannot describe itself is not reproducible.
    pub notes: Vec<String>,
}

/// Writes a matrix as a labelled table: one row per wavelength, one column per
/// delay, the wavelength in the first column.
///
/// This is the transpose of the screen's mental model, and deliberately so: a
/// table with the wavelength down the left edge is what a reader of the file
/// expects, and the header row names every delay.
pub fn write_matrix(path: &Path, export: &MatrixExport<'_>, options: ExportOptions) -> Result<()> {
    let (rows, columns) = export.data.dim();
    if rows != export.wavelength.len() || columns != export.delay.len() {
        return Err(CoreError::ShapeMismatch {
            rows,
            cols: columns,
            wavelengths: export.wavelength.len(),
            delays: export.delay.len(),
        });
    }
    let mut writer = create(path)?;
    let separator = options.delimiter.as_char();

    for note in &export.notes {
        writeln!(writer, "# {note}").map_err(io(path))?;
    }

    let rows: Vec<usize> = export
        .rows
        .clone()
        .unwrap_or_else(|| (0..rows).collect());
    let columns: Vec<usize> = export
        .columns
        .clone()
        .unwrap_or_else(|| (0..columns).collect());
    if rows.iter().any(|row| *row >= export.data.nrows())
        || columns.iter().any(|column| *column >= export.data.ncols())
    {
        return Err(CoreError::ShapeMismatch {
            rows: export.data.nrows(),
            cols: export.data.ncols(),
            wavelengths: rows.len(),
            delays: columns.len(),
        });
    }

    if options.header {
        write!(writer, "{} ({})", export.wavelength.name(), export.wavelength.unit().symbol())
            .map_err(io(path))?;
        for column in &columns {
            write!(
                writer,
                "{separator}{}",
                format_number(export.delay.values()[*column], options.precision)
            )
            .map_err(io(path))?;
        }
        writeln!(writer).map_err(io(path))?;
    }

    for row in &rows {
        write!(
            writer,
            "{}",
            format_number(export.wavelength.values()[*row], options.precision)
        )
        .map_err(io(path))?;
        for column in &columns {
            write!(
                writer,
                "{separator}{}",
                format_number(export.data[[*row, *column]], options.precision)
            )
            .map_err(io(path))?;
        }
        writeln!(writer).map_err(io(path))?;
    }
    writer.flush().map_err(io(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::parse_numeric_lines;
    use ndarray::array;
    use ta_core::{AxisKind, AxisUnit};

    fn temp_path(name: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("ta-export-test-{}-{name}", std::process::id()));
        path
    }

    #[test]
    fn a_multi_series_export_keeps_each_curve_at_its_own_resolution() {
        let path = temp_path("blocks.csv");
        let blocks = vec![
            SeriesBlock::new(
                "sample A · 600 nm",
                "Delay (ps)",
                "ΔOD",
                vec![-1.0, 0.0, 1.0],
                vec![0.0, 0.1, 0.05],
            ),
            SeriesBlock::new(
                "sample B · 700 nm",
                "Time (ps)",
                "ΔOD",
                vec![0.0, 2.0],
                vec![0.2, 0.1],
            ),
        ];
        let notes = vec!["project: test".to_string(), "version: cropped".to_string()];
        let written = write_blocks(&path, &blocks, &notes, ExportOptions::default()).unwrap();
        assert_eq!(written, 5);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# project: test\n# version: cropped\n"));
        assert!(text.contains("# series: sample A · 600 nm"));
        assert!(text.contains("# series: sample B · 700 nm"));
        // Each block keeps its own delay column, which is the point.
        assert!(text.contains("Time (ps),ΔOD"));
        assert!(text.contains("2.000000,0.100000"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_matrix_export_can_write_one_window_and_its_own_provenance() {
        let wavelength = Axis::new(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            "Wavelength",
            vec![400.0, 500.0, 600.0],
        )
        .unwrap();
        let delay = Axis::new(
            AxisKind::Delay,
            AxisUnit::Picosecond,
            "Delay",
            vec![0.0, 1.0],
        )
        .unwrap();
        let data = array![[1.0, 2.0], [3.0, f64::NAN], [5.0, 6.0]];
        let path = temp_path("window.csv");
        write_matrix(
            &path,
            &MatrixExport {
                wavelength: &wavelength,
                delay: &delay,
                data: &data,
                unit: SignalUnit::DeltaOpticalDensity,
                rows: Some(vec![1, 2]),
                columns: Some(vec![1]),
                notes: vec!["figure: contour".to_string()],
            },
            ExportOptions::default(),
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# figure: contour\n"));
        let lines: Vec<&str> = text.lines().collect();
        // One heading naming the single delay, and two rows.
        assert_eq!(lines[1], "Wavelength (nm),1.000000");
        assert_eq!(lines.len(), 4);
        assert!(lines[2].contains("nan"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn an_axis_can_be_written_on_its_own() {
        let axis = Axis::new(
            AxisKind::Delay,
            AxisUnit::Picosecond,
            "Delay",
            vec![-1.0, 0.0, 1.0],
        )
        .unwrap();
        let path = temp_path("axis.csv");
        write_axis(&path, &axis, ExportOptions::default()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 4);
        assert!(text.starts_with("Delay (ps)"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_series_writes_a_header_and_one_row_per_point() {
        let series = Series::new(
            "Delay (ps)",
            "ΔOD",
            vec![-1.0, 0.0, 1.5],
            vec![0.001, f64::NAN, -0.002],
        );
        let path = temp_path("series.csv");
        write_series(&path, &series, ExportOptions::default()).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Delay (ps),ΔOD");
        assert_eq!(lines.len(), 4);
        assert!(lines[2].contains("nan"));
        assert!(lines[3].starts_with("1.500000,-0.002000"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_matrix_writes_one_row_per_wavelength_and_one_column_per_delay() {
        let wavelength = Axis::new(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            "Wavelength",
            vec![500.0, 510.0],
        )
        .unwrap();
        let delay = Axis::new(
            AxisKind::Delay,
            AxisUnit::Picosecond,
            "Delay",
            vec![-1.0, 0.0, 1.0],
        )
        .unwrap();
        let data = array![[0.1, 0.2, 0.3], [0.4, 0.5, 0.6]];
        let path = temp_path("matrix.csv");
        write_matrix(
            &path,
            &MatrixExport {
                rows: None,
                columns: None,
                notes: Vec::new(),
                wavelength: &wavelength,
                delay: &delay,
                data: &data,
                unit: SignalUnit::DeltaOpticalDensity,
            },
            ExportOptions::default(),
        )
        .unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Wavelength (nm),-1.000000,0.000000,1.000000");
        assert!(lines[1].starts_with("500.000000,0.100000,0.200000,0.300000"));
        assert!(lines[2].starts_with("510.000000,0.400000,0.500000,0.600000"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_tab_export_can_be_read_straight_back_in() {
        let series = Series::new("d", "s", vec![0.0, 1.0], vec![2.0, 3.0]);
        let path = temp_path("series.tsv");
        write_series(
            &path,
            &series,
            ExportOptions {
                delimiter: Delimiter::Tab,
                header: false,
                precision: 3,
            },
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let lines = parse_numeric_lines(&text, "series.tsv").unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].values, vec![1.0, 3.0]);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn exporting_a_matrix_whose_shape_disagrees_with_its_axes_is_refused() {
        let wavelength = Axis::new(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            "Wavelength",
            vec![500.0, 510.0, 520.0],
        )
        .unwrap();
        let delay = Axis::new(
            AxisKind::Delay,
            AxisUnit::Picosecond,
            "Delay",
            vec![-1.0, 0.0, 1.0],
        )
        .unwrap();
        let data = array![[0.1, 0.2, 0.3], [0.4, 0.5, 0.6]];
        let path = temp_path("bad.csv");
        assert!(write_matrix(
            &path,
            &MatrixExport {
                rows: None,
                columns: None,
                notes: Vec::new(),
                wavelength: &wavelength,
                delay: &delay,
                data: &data,
                unit: SignalUnit::DeltaOpticalDensity,
            },
            ExportOptions::default(),
        )
        .is_err());
        std::fs::remove_file(&path).ok();
    }
}
