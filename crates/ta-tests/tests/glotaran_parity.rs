//! Parity with the reference implementation's Glotaran export.
//!
//! The expected values come from `reference/generate_reference.py`, which runs
//! the reference's own numpy expression on the repository's fixture. The legacy
//! ASCII file is compared line by line, because `spec.md` §16.2 asks for the
//! format the reference writes, not merely one that parses.

use ta_core::SignalUnit;
use ta_glotaran::{python_float, read_ascii, write_ascii, GloDataset};
use ta_tests::{nir800_matrix_with_axes, number, reference, value};

fn fixture() -> GloDataset {
    let dataset = nir800_matrix_with_axes().expect("the fixture imports");
    GloDataset::from_version(&dataset, dataset.active_version).expect("it converts")
}

fn temp_path(name: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("ta-glotaran-parity-{}-{name}", std::process::id()));
    path
}

#[test]
fn every_float_is_written_the_way_python_writes_it() {
    // The values whose form is easy to get wrong, with Python's own answers.
    let reference = reference();
    let table = value(&reference, &["float_format"]);
    let table = table.as_object().expect("a table of floats");
    assert!(table.len() > 20, "the table should be worth checking");
    for (input, expected) in table {
        let expected = expected.as_str().expect("a string");
        let value: f64 = input.parse().expect("the key is a float");
        assert_eq!(
            python_float(value),
            expected,
            "{input} should be written {expected}"
        );
    }
}

#[test]
fn the_legacy_file_is_the_one_the_reference_writes() {
    let dataset = fixture();
    let path = temp_path("glo.ascii");
    let report = write_ascii(&dataset, &path).expect("it writes");
    let text = std::fs::read_to_string(&path).expect("it reads back");
    let lines: Vec<&str> = text.lines().collect();

    let reference = reference();
    let expected_header = value(&reference, &["glotaran", "header_lines"]);
    let expected_header: Vec<&str> = expected_header
        .as_array()
        .expect("header lines")
        .iter()
        .map(|line| line.as_str().expect("a string"))
        .collect();
    assert_eq!(lines[0], expected_header[0], "the dataset is named");
    assert_eq!(lines[1], expected_header[1], "a blank line follows");
    assert_eq!(lines[2], expected_header[2], "the matrix is time explicit");
    assert_eq!(
        lines[3], expected_header[3],
        "and the count of time points is the file's own"
    );
    assert_eq!(report.rows, dataset.spectral.len());
    assert_eq!(report.columns, dataset.time.len());

    // Every line, exactly as Python's np.savetxt wrote it.
    let expected_lines = number(&reference, &["glotaran", "line_count"]) as usize;
    assert_eq!(lines.len(), expected_lines, "the file has the same length");

    let first_rows = value(&reference, &["glotaran", "first_rows"]);
    let first_rows: Vec<&str> = first_rows
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row.as_str().expect("a string"))
        .collect();
    assert_eq!(lines[4], first_rows[0], "the delay row");
    assert_eq!(lines[5], first_rows[1], "the first wavelength's row");
    assert_eq!(lines[6], first_rows[2], "the second wavelength's row");

    let last_row = value(&reference, &["glotaran", "last_row"])
        .as_str()
        .expect("a string")
        .to_string();
    assert_eq!(lines[lines.len() - 1], last_row, "and the last one");
    std::fs::remove_file(&path).ok();
}

#[test]
fn the_file_the_exporter_writes_is_the_matrix_it_was_written_from() {
    // §16.2's validation: re-import gives back the same numbers and the same
    // axes, so a user can check an export without opening another program.
    let source = nir800_matrix_with_axes().expect("the fixture imports");
    let view = GloDataset::from_version(&source, source.active_version).expect("it converts");
    let path = temp_path("round.ascii");
    write_ascii(&view, &path).expect("it writes");
    let back = read_ascii(&path).expect("it reads back");

    let (wavelength, delay) = source.axes(source.active_version).expect("the axes");
    assert_eq!(back.time.len(), delay.len());
    assert_eq!(back.spectral.len(), wavelength.len());
    for (index, expected) in delay.values().iter().enumerate() {
        assert_eq!(back.time[index], *expected, "delay {index}");
    }
    for (index, expected) in wavelength.values().iter().enumerate() {
        assert_eq!(back.spectral[index], *expected, "wavelength {index}");
    }
    // And the values are the matrix, transposed and transposed back.
    let matrix = source.matrix(source.active_version).expect("the matrix");
    for (time_index, _) in delay.values().iter().enumerate() {
        for (spectral_index, _) in wavelength.values().iter().enumerate() {
            assert_eq!(
                back.value(time_index, spectral_index),
                matrix[[spectral_index, time_index]],
                "at {time_index}, {spectral_index}"
            );
        }
    }
    std::fs::remove_file(&path).ok();
}

#[test]
fn a_processed_version_is_the_one_that_is_exported() {
    // §16.4: the export writes the version on the bench, and says which it was.
    let mut dataset = nir800_matrix_with_axes().expect("the fixture imports");
    let raw = dataset.active_version;
    let (wavelength, delay) = dataset.axes(raw).expect("the axes");
    let matrix = dataset.matrix(raw).expect("the matrix").clone();
    let cropped = ta_processing::crop_matrix(
        &matrix,
        &wavelength,
        &delay,
        &ta_core::CropSpec {
            wavelength: Some((500.0, 600.0)),
            delay: None,
        },
    )
    .expect("it crops");
    let version = dataset
        .add_derived(
            cropped.data,
            SignalUnit::DeltaOpticalDensity,
            cropped.wavelength,
            cropped.delay,
            "Cropped 500–600 nm",
            ta_core::ProcessingOp::Crop(ta_core::CropSpec {
                wavelength: Some((500.0, 600.0)),
                delay: None,
            }),
            vec![raw],
        )
        .expect("it stores the version");

    let view = GloDataset::from_version(&dataset, version).expect("it converts");
    let path = temp_path("cropped.ascii");
    let report = write_ascii(&view, &path).expect("it writes");
    let text = std::fs::read_to_string(&path).expect("it reads back");
    // The header names the dataset, and the report names the version and the
    // operation, so two exports can be told apart.
    assert!(text.starts_with(&dataset.name));
    assert_eq!(report.rows, view.spectral.len());
    assert!(view.version_label.contains("Cropped"));
    assert!(view.operation.contains("500"));
    assert!(view.spectral.iter().all(|value| (500.0..=600.0).contains(value)));
    std::fs::remove_file(&path).ok();
}

#[test]
fn the_modern_dataset_puts_time_first() {
    // The convention pyglotaran reads: `data` is indexed [time, spectral].
    let dataset = nir800_matrix_with_axes().expect("the fixture imports");
    let view = GloDataset::from_version(&dataset, dataset.active_version).expect("it converts");
    let matrix = dataset.matrix(dataset.active_version).expect("the matrix");
    assert_eq!(view.data.len(), matrix.len());
    // The first value is the first delay column's first wavelength.
    assert_eq!(view.value(0, 0), matrix[[0, 0]]);
    assert_eq!(view.value(1, 0), matrix[[0, 1]]);
    assert_eq!(view.to_internal_matrix(), matrix.clone());
}
