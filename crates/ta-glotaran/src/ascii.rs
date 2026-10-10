//! The legacy time-explicit ASCII file the Java Glotaran reads.
//!
//! The layout is the reference implementation's `glotaran()`:
//!
//! ```text
//! <name>
//! <blank>
//! Time explicit
//! intervalnr <number of time points>
//! <tab> t0 <tab> t1 ...
//! w0 <tab> m[0,0] <tab> m[0,1] ...
//! w1 <tab> m[1,0] <tab> m[1,1] ...
//! ```
//!
//! The first row carries the delays with an empty first cell, and every later
//! row starts with a wavelength. Numbers are written the way Python's `str()`
//! writes them, because that is what `np.savetxt(fmt="%s")` produces.

use crate::format::python_float;
use crate::matrix_view::GloDataset;
use std::io::{BufWriter, Write};
use std::path::Path;
use ta_core::{CoreError, Result};

/// What was written, for the report the interface shows.
#[derive(Clone, Debug, PartialEq)]
pub struct AsciiReport {
    pub path: String,
    /// The header lines, without their line endings.
    pub header: Vec<String>,
    pub rows: usize,
    pub columns: usize,
    pub values: usize,
}

/// The header of a legacy file, as the reference builds it.
pub fn header_lines(dataset: &GloDataset) -> Vec<String> {
    vec![
        dataset.name.clone(),
        String::new(),
        "Time explicit".to_string(),
        format!("intervalnr {}", dataset.time.len()),
    ]
}

/// Writes the legacy file.
pub fn write_ascii(dataset: &GloDataset, path: &Path) -> Result<AsciiReport> {
    let header = header_lines(dataset);
    let file = std::fs::File::create(path).map_err(|source| CoreError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let mut writer = BufWriter::new(file);
    let io = |source: std::io::Error| CoreError::Io {
        path: path.display().to_string(),
        source,
    };

    for line in &header {
        writeln!(writer, "{line}").map_err(io)?;
    }
    // The row of delays, with the empty leading cell the reference's transpose
    // produces.
    write!(writer, "\t").map_err(io)?;
    for (index, time) in dataset.time.iter().enumerate() {
        if index > 0 {
            write!(writer, "\t").map_err(io)?;
        }
        write!(writer, "{}", python_float(*time)).map_err(io)?;
    }
    writeln!(writer).map_err(io)?;

    for (spectral_index, wavelength) in dataset.spectral.iter().enumerate() {
        write!(writer, "{}", python_float(*wavelength)).map_err(io)?;
        for time_index in 0..dataset.time.len() {
            write!(
                writer,
                "\t{}",
                python_float(dataset.value(time_index, spectral_index))
            )
            .map_err(io)?;
        }
        writeln!(writer).map_err(io)?;
    }
    writer.flush().map_err(io)?;

    Ok(AsciiReport {
        path: path.display().to_string(),
        header,
        rows: dataset.spectral.len(),
        columns: dataset.time.len(),
        values: dataset.data.len(),
    })
}

/// Reads a legacy file back.
///
/// This is the validation §16.2 asks for: what the exporter writes has to be
/// readable as the matrix and the axes it was written from. It also accepts the
/// files the reference implementation produces, which is how the two are
/// compared.
pub fn read_ascii(path: &Path) -> Result<GloDataset> {
    let text = std::fs::read_to_string(path).map_err(|source| CoreError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let mut lines = text.lines();
    let name = lines
        .next()
        .ok_or_else(|| CoreError::invalid("the file is empty"))?
        .to_string();
    // The blank line, then the two header lines.
    let mut header = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    for line in lines.by_ref() {
        if line.trim_start().starts_with(|character: char| character.is_ascii_digit())
            || line.trim_start().starts_with('-')
        {
            pending.push(line.to_string());
            break;
        }
        header.push(line.to_string());
        if header.len() >= 3 {
            break;
        }
    }
    if header.len() < 3 {
        return Err(CoreError::invalid(
            "the header does not name a time-explicit matrix",
        ));
    }
    if !header.iter().any(|line| line.contains("Time explicit")) {
        return Err(CoreError::invalid(
            "the header does not say the matrix is time explicit",
        ));
    }

    let mut rows: Vec<Vec<f64>> = Vec::new();
    for line in pending.into_iter().chain(lines.map(str::to_string)) {
        if line.trim().is_empty() {
            continue;
        }
        let mut row: Vec<f64> = Vec::new();
        let mut ok = true;
        for cell in line.split('\t') {
            let cell = cell.trim();
            if cell.is_empty() {
                // The leading cell of the delay row.
                continue;
            }
            match cell.parse::<f64>() {
                Ok(value) => row.push(value),
                Err(_) => {
                    ok = false;
                    break;
                }
            }
        }
        if ok && !row.is_empty() {
            rows.push(row);
        }
    }
    if rows.len() < 2 {
        return Err(CoreError::invalid(
            "the file has no delay row and wavelength rows",
        ));
    }
    let time = rows[0].clone();
    let mut spectral = Vec::with_capacity(rows.len() - 1);
    // The file is wavelength-major, one row per wavelength; the view both
    // writers use is time-major, so the transpose happens here — once, on the
    // way in, exactly as it happens once on the way out.
    let mut by_wavelength: Vec<Vec<f64>> = Vec::with_capacity(rows.len() - 1);
    for row in &rows[1..] {
        if row.len() != time.len() + 1 {
            return Err(CoreError::invalid(format!(
                "a row has {} values where the delay row has {}",
                row.len() - 1,
                time.len()
            )));
        }
        spectral.push(row[0]);
        by_wavelength.push(row[1..].to_vec());
    }
    let mut data = Vec::with_capacity(by_wavelength.len() * time.len());
    for time_index in 0..time.len() {
        for row in &by_wavelength {
            data.push(row[time_index]);
        }
    }
    GloDataset::new(name, time, spectral, data, ta_core::SignalUnit::DeltaOpticalDensity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_core::SignalUnit;

    fn temp_path(name: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("ta-glotaran-{}-{name}", std::process::id()));
        path
    }

    fn dataset() -> GloDataset {
        // 2 delays × 3 wavelengths: the small case whose transpose is easy to
        // check by eye.
        GloDataset::new(
            "NIR800_tcorr",
            vec![-1.0, 0.5],
            vec![400.38315, 500.0, 799.61685],
            vec![
                -0.00011, 0.00028, -8e-5, // at -1.0 ps
                -6e-5, 0.0, 4e-5,         // at 0.5 ps
            ],
            SignalUnit::DeltaOpticalDensity,
        )
        .unwrap()
    }

    #[test]
    fn the_header_and_the_layout_are_the_references() {
        let dataset = dataset();
        assert_eq!(
            header_lines(&dataset),
            vec!["NIR800_tcorr", "", "Time explicit", "intervalnr 2"]
        );
        let path = temp_path("legacy.ascii");
        let report = write_ascii(&dataset, &path).unwrap();
        assert_eq!(report.rows, 3);
        assert_eq!(report.columns, 2);
        assert_eq!(report.values, 6);
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "NIR800_tcorr");
        assert_eq!(lines[1], "");
        assert_eq!(lines[2], "Time explicit");
        assert_eq!(lines[3], "intervalnr 2");
        // The delay row leads with an empty cell, and every later row leads
        // with a wavelength.
        assert_eq!(lines[4], "\t-1.0\t0.5");
        assert_eq!(lines[5], "400.38315\t-0.00011\t-6e-05");
        assert_eq!(lines[7], "799.61685\t-8e-05\t4e-05");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn what_the_writer_writes_the_reader_reads_back() {
        let dataset = dataset();
        let path = temp_path("round.ascii");
        write_ascii(&dataset, &path).unwrap();
        let back = read_ascii(&path).unwrap();
        assert_eq!(back.name, dataset.name);
        assert_eq!(back.time, dataset.time);
        assert_eq!(back.spectral, dataset.spectral);
        assert_eq!(back.data, dataset.data);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_file_with_rows_that_disagree_with_the_delay_row_is_refused() {
        let path = temp_path("ragged.ascii");
        std::fs::write(
            &path,
            "x\n\nTime explicit\nintervalnr 3\n\t0.0\t1.0\n500.0\t1.0\n",
        )
        .unwrap();
        assert!(read_ascii(&path).is_err());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_file_that_is_not_time_explicit_is_refused() {
        let path = temp_path("wrong.ascii");
        std::fs::write(&path, "x\n\nSpectral\nintervalnr 2\n").unwrap();
        assert!(read_ascii(&path).is_err());
        std::fs::remove_file(&path).ok();
    }
}
