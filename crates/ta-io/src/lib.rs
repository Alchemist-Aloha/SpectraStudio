//! Reading the text formats transient absorption files actually arrive in.
//!
//! Two layouts cover the reference implementation and the fixture data:
//!
//! * the **TA sheet** — a two-header-column table: column 0 is the delay axis,
//!   column 1 is the wavelength axis, and every column after that holds ΔA at
//!   one delay (`tamatrix_importer` in the Python code);
//! * a **plain matrix** plus separate axis files (`load_spectra`).
//!
//! Both are read without assuming the axes are sorted, and both keep every
//! wavelength the file contains.

use ndarray::Array2;
use std::path::Path;
use ta_core::{
    Axis, AxisKind, AxisUnit, CoreError, Result, SignalUnit, SourceFormat, TaDataset,
};

pub mod export;
pub mod project_file;
pub mod text;

use text::{parse_numeric_lines, read_to_string, require_uniform_width};

pub use project_file::{load_project, save_project, LoadedProject};

/// What a TA sheet's axes and matrix are measured in, when the file does not
/// say.
///
/// The reference implementation labels its y axis ΔOD and its time axis in
/// picoseconds, so those are the defaults rather than a guess made from the
/// magnitude of the numbers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Units {
    pub wavelength: AxisUnit,
    pub delay: AxisUnit,
    pub signal: SignalUnit,
}

impl Default for Units {
    fn default() -> Self {
        Self {
            wavelength: AxisUnit::Nanometer,
            delay: AxisUnit::Picosecond,
            signal: SignalUnit::DeltaOpticalDensity,
        }
    }
}

/// A sheet read into memory, before it becomes a dataset.
#[derive(Clone, Debug, PartialEq)]
pub struct TaSheet {
    pub wavelength: Axis,
    pub delay: Axis,
    pub data: Array2<f64>,
    pub unit: SignalUnit,
    /// Anything notable about the file, kept and shown rather than swallowed.
    pub notes: Vec<String>,
}

/// Reads a two-header-column TA sheet.
pub fn read_ta_sheet(path: &Path, units: Units) -> Result<TaSheet> {
    let path_text = path.display().to_string();
    let text = read_to_string(path)?;
    let lines = parse_numeric_lines(&text, &path_text)?;
    let width = require_uniform_width(&lines, &path_text)?;

    if width < 3 {
        return Err(CoreError::invalid(format!(
            "{path_text} has {width} columns; a TA sheet needs a delay column, a wavelength \
             column, and at least one data column"
        )));
    }

    let delay_count = width - 2;
    if lines.len() < delay_count {
        return Err(CoreError::invalid(format!(
            "{path_text} has {} wavelength rows but {delay_count} delay columns",
            lines.len()
        )));
    }

    let mut notes = Vec::new();
    let mut delay_values = Vec::with_capacity(delay_count);
    for line in &lines[..delay_count] {
        delay_values.push(line.values[0]);
    }

    // Rows below the delay axis are the padding that makes the sheet
    // rectangular. Non-zero values there mean the layout is not what it looks
    // like, which the caller needs to know about.
    let stray = lines[delay_count..]
        .iter()
        .filter(|line| line.values[0] != 0.0)
        .count();
    if stray > 0 {
        notes.push(format!(
            "{stray} rows below the delay axis have a non-zero first column"
        ));
    }

    let wavelength_values: Vec<f64> = lines.iter().map(|line| line.values[1]).collect();
    let wavelength = Axis::new(
        AxisKind::Wavelength,
        units.wavelength,
        "Wavelength",
        wavelength_values,
    )?;
    let delay = Axis::new(AxisKind::Delay, units.delay, "Delay", delay_values)?;

    let rows = lines.len();
    let mut data = Array2::zeros((rows, delay_count));
    for (row, line) in lines.iter().enumerate() {
        for column in 0..delay_count {
            data[[row, column]] = line.values[2 + column];
        }
    }

    Ok(TaSheet {
        wavelength,
        delay,
        data,
        unit: units.signal,
        notes,
    })
}

/// Reads a TA sheet and turns it into a dataset, using the file stem as the
/// dataset name.
pub fn import_ta_sheet(path: &Path, units: Units) -> Result<TaDataset> {
    let sheet = read_ta_sheet(path, units)?;
    let name = name_from_path(path);
    let mut dataset = TaDataset::new_raw(
        &name,
        sheet.wavelength,
        sheet.delay,
        sheet.data,
        sheet.unit,
        SourceFormat::TextMatrix,
    )?;
    dataset.source_paths = vec![path.display().to_string()];
    if !sheet.notes.is_empty() {
        dataset
            .metadata
            .extra
            .insert("import_notes".to_string(), sheet.notes.join("; "));
    }
    Ok(dataset)
}

/// Reads a one-value-per-line axis file, ignoring blank lines and comments.
pub fn read_axis_file(
    path: &Path,
    kind: AxisKind,
    unit: AxisUnit,
    name: &str,
) -> Result<Axis> {
    let path_text = path.display().to_string();
    let text = read_to_string(path)?;
    let lines = parse_numeric_lines(&text, &path_text)?;
    if lines.is_empty() {
        return Err(CoreError::EmptyAxis {
            axis: match kind {
                AxisKind::Wavelength => "wavelength",
                AxisKind::Delay => "delay",
                _ => "custom",
            },
        });
    }
    let values: Vec<f64> = lines.iter().map(|line| line.values[0]).collect();
    Axis::new(kind, unit, name, values)
}

/// Reads a plain numeric matrix, requiring every row to have the same width.
pub fn read_matrix_file(path: &Path) -> Result<Array2<f64>> {
    let path_text = path.display().to_string();
    let text = read_to_string(path)?;
    let lines = parse_numeric_lines(&text, &path_text)?;
    let width = require_uniform_width(&lines, &path_text)?;
    if lines.is_empty() || width == 0 {
        return Err(CoreError::invalid(format!("{path_text} holds no matrix")));
    }
    let mut data = Array2::zeros((lines.len(), width));
    for (row, line) in lines.iter().enumerate() {
        for (column, value) in line.values.iter().enumerate() {
            data[[row, column]] = *value;
        }
    }
    Ok(data)
}

/// Reads a matrix file together with its two axis files.
///
/// The matrix is stored wavelength-major, which is the layout both the
/// reference implementation and this application use.
pub fn import_matrix_with_axes(
    name: &str,
    matrix_path: &Path,
    wavelength_path: &Path,
    delay_path: &Path,
    units: Units,
) -> Result<TaDataset> {
    let wavelength = read_axis_file(
        wavelength_path,
        AxisKind::Wavelength,
        units.wavelength,
        "Wavelength",
    )?;
    let delay = read_axis_file(delay_path, AxisKind::Delay, units.delay, "Delay")?;
    let data = read_matrix_file(matrix_path)?;

    let (rows, columns) = data.dim();
    if rows != wavelength.len() || columns != delay.len() {
        // The likely mistake is a transposed matrix, so say so explicitly.
        if rows == delay.len() && columns == wavelength.len() {
            return Err(CoreError::invalid(format!(
                "{} is {}×{} which is delay-major; Spectra Studio stores wavelength-major, \
                 so the file needs transposing before import",
                matrix_path.display(),
                rows,
                columns
            )));
        }
        return Err(CoreError::ShapeMismatch {
            rows,
            cols: columns,
            wavelengths: wavelength.len(),
            delays: delay.len(),
        });
    }

    let mut dataset = TaDataset::new_raw(
        name,
        wavelength,
        delay,
        data,
        units.signal,
        SourceFormat::TextMatrix,
    )?;
    dataset.source_paths = vec![
        matrix_path.display().to_string(),
        wavelength_path.display().to_string(),
        delay_path.display().to_string(),
    ];
    Ok(dataset)
}

/// The file stem, which is what the reference implementation calls a dataset.
pub fn name_from_path(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp(name: &str, contents: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("ta-io-test-{}-{name}", std::process::id()));
        let mut file = std::fs::File::create(&path).expect("temp file");
        file.write_all(contents.as_bytes()).expect("write");
        path
    }

    #[test]
    fn a_three_row_sheet_becomes_a_wavelength_major_matrix() {
        // Two delays, three wavelengths: column 0 is the delay axis, column 1
        // the wavelength axis, columns 2 and 3 the two delay columns.
        let sheet = "\
            0\t500.0\t0.01\t0.02\n\
            1\t510.0\t0.03\t0.04\n\
            0\t520.0\t0.05\t0.06\n";
        let path = write_temp("sheet.txt", sheet);
        let sheet = read_ta_sheet(&path, Units::default()).unwrap();
        assert_eq!(sheet.wavelength.values(), &[500.0, 510.0, 520.0]);
        assert_eq!(sheet.delay.values(), &[0.0, 1.0]);
        assert_eq!(sheet.data.dim(), (3, 2));
        assert_eq!(sheet.data[[2, 1]], 0.06);
        assert!(sheet.notes.is_empty());
    }

    #[test]
    fn the_delay_axis_comes_from_the_first_rows_not_from_every_row() {
        // Six rows, three data columns: the delay axis is the first three
        // values of column 0, and the rest is padding.
        let sheet = "\
            0\t500\t1\t2\t3\n\
            1\t510\t1\t2\t3\n\
            2\t520\t1\t2\t3\n\
            0\t530\t1\t2\t3\n\
            0\t540\t1\t2\t3\n\
            0\t550\t1\t2\t3\n";
        let path = write_temp("padding.txt", sheet);
        let sheet = read_ta_sheet(&path, Units::default()).unwrap();
        assert_eq!(sheet.delay.values(), &[0.0, 1.0, 2.0]);
        assert_eq!(sheet.wavelength.len(), 6);
        assert!(sheet.notes.is_empty());
    }

    #[test]
    fn a_padded_sheet_with_stray_delay_values_says_so() {
        let sheet = "\
            0\t500\t1\n\
            1\t510\t1\n\
            9\t520\t1\n";
        let path = write_temp("stray.txt", sheet);
        let sheet = read_ta_sheet(&path, Units::default()).unwrap();
        assert_eq!(sheet.notes.len(), 1);
        assert!(sheet.notes[0].contains("non-zero first column"));
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let sheet = "\
            # a comment\n\
            \n\
            0\t500\t1\t2\n\
            % another\n\
            1\t510\t3\t4\n";
        let path = write_temp("comments.txt", sheet);
        let sheet = read_ta_sheet(&path, Units::default()).unwrap();
        assert_eq!(sheet.wavelength.len(), 2);
        assert_eq!(sheet.delay.values(), &[0.0, 1.0]);
    }

    #[test]
    fn a_non_numeric_field_is_reported_with_its_line_and_column() {
        let sheet = "0\t500\t1\t2\n1\t510\tnope\t4\n";
        let path = write_temp("bad.txt", sheet);
        let error = read_ta_sheet(&path, Units::default()).unwrap_err();
        match error {
            CoreError::MalformedNumber { line, field, text, .. } => {
                assert_eq!(line, 2);
                assert_eq!(field, 2);
                assert_eq!(text, "nope");
            }
            other => panic!("expected MalformedNumber, got {other:?}"),
        }
    }

    #[test]
    fn ragged_rows_are_refused_rather_than_truncated() {
        let sheet = "0\t500\t1\t2\n1\t510\t3\n";
        let path = write_temp("ragged.txt", sheet);
        assert!(matches!(
            read_ta_sheet(&path, Units::default()).unwrap_err(),
            CoreError::RecordWidth { .. }
        ));
    }

    #[test]
    fn a_sheet_with_no_data_columns_is_refused() {
        let sheet = "0\t500\n1\t510\n";
        let path = write_temp("narrow.txt", sheet);
        assert!(read_ta_sheet(&path, Units::default()).is_err());
    }

    #[test]
    fn importing_a_sheet_records_where_it_came_from() {
        let sheet = "0\t500\t1\t2\n1\t510\t3\t4\n";
        let path = write_temp("import.txt", sheet);
        let dataset = import_ta_sheet(&path, Units::default()).unwrap();
        assert_eq!(dataset.shape(), (2, 2));
        assert_eq!(dataset.source_paths.len(), 1);
        assert_eq!(dataset.metadata.source_format, SourceFormat::TextMatrix);
        assert!(dataset.name.starts_with("ta-io-test-"));
    }

    #[test]
    fn axis_files_and_a_plain_matrix_import_together() {
        let matrix = write_temp("axes-matrix.txt", "1 2 3\n4 5 6\n");
        let wavelength = write_temp("axes-wavelength.txt", "500\n510\n");
        let delay = write_temp("axes-delay.txt", "-1\n0\n1\n");
        let dataset =
            import_matrix_with_axes("tcorr", &matrix, &wavelength, &delay, Units::default())
                .unwrap();
        assert_eq!(dataset.shape(), (2, 3));
        assert_eq!(dataset.wavelength().values(), &[500.0, 510.0]);
        assert_eq!(dataset.delay().values(), &[-1.0, 0.0, 1.0]);
    }

    #[test]
    fn a_transposed_matrix_is_rejected_with_a_message_that_names_the_cause() {
        let matrix = write_temp("transposed-matrix.txt", "1 2\n3 4\n5 6\n");
        let wavelength = write_temp("transposed-wavelength.txt", "500\n510\n");
        let delay = write_temp("transposed-delay.txt", "-1\n0\n1\n");
        let error =
            import_matrix_with_axes("x", &matrix, &wavelength, &delay, Units::default())
                .unwrap_err();
        assert!(format!("{error}").contains("transposing"));
    }
}
