//! Parity with the Python reference implementation on the repository's own
//! fixture data (spec §22.1).
//!
//! Every expected number comes from `reference/nir800_reference.json`, which
//! `reference/generate_reference.py` produces with numpy.

use ta_core::{
    kinetic_trace, transient_spectrum, AxisRequest, SpectrumSelection, TraceSelection,
};
use ta_tests::{
    assert_close, assert_slice_close, fixture, integer, matrix_stats, nir800_matrix_with_axes,
    nir800_sheet, number, numbers, reference,
};

const TOLERANCE: f64 = 1e-9;

#[test]
fn the_sheet_fixture_is_the_file_the_reference_was_generated_from() {
    let sheet = fixture("NIR800");
    assert!(sheet.exists(), "{} is missing", sheet.display());
    let reference = reference();
    let text = std::fs::read_to_string(&sheet).unwrap();
    assert_eq!(
        text.lines().count(),
        integer(&reference, &["sheet", "wavelength_len"]) as usize,
        "the fixture file has changed, so the reference must be regenerated"
    );
}

#[test]
fn importing_the_sheet_matches_the_python_reference() {
    let dataset = nir800_sheet().expect("the fixture imports");
    let reference = reference();

    let (rows, cols) = dataset.shape();
    assert_eq!(rows, integer(&reference, &["sheet", "wavelength_len"]) as usize);
    assert_eq!(cols, integer(&reference, &["sheet", "delay_len"]) as usize);
    assert_eq!(dataset.wavelength().len(), 1340);
    assert_eq!(dataset.delay().len(), 152);

    assert_close(
        dataset.wavelength().first(),
        number(&reference, &["sheet", "wavelength_first"]),
        TOLERANCE,
    );
    assert_close(
        dataset.wavelength().last(),
        number(&reference, &["sheet", "wavelength_last"]),
        TOLERANCE,
    );
    assert_close(
        dataset.delay().first(),
        number(&reference, &["sheet", "delay_first"]),
        TOLERANCE,
    );
    assert_close(
        dataset.delay().last(),
        number(&reference, &["sheet", "delay_last"]),
        TOLERANCE,
    );

    let raw = dataset.raw_version().id;
    let block = dataset.block(raw).unwrap();
    let (rows, cols, sum, sum_sq, min, max, finite, missing) = matrix_stats(&block.data);
    assert_eq!(rows, integer(&reference, &["sheet", "matrix", "rows"]) as usize);
    assert_eq!(cols, integer(&reference, &["sheet", "matrix", "cols"]) as usize);
    assert_eq!(finite, integer(&reference, &["sheet", "matrix", "finite"]) as usize);
    assert_eq!(missing, integer(&reference, &["sheet", "matrix", "missing"]) as usize);
    assert_close(sum, number(&reference, &["sheet", "matrix", "sum"]), TOLERANCE);
    assert_close(
        sum_sq,
        number(&reference, &["sheet", "matrix", "sum_sq"]),
        TOLERANCE,
    );
    assert_close(min, number(&reference, &["sheet", "matrix", "min"]), TOLERANCE);
    assert_close(max, number(&reference, &["sheet", "matrix", "max"]), TOLERANCE);

    let samples = ta_tests::value(&reference, &["sheet", "samples"])
        .as_array()
        .expect("samples");
    for sample in samples {
        let row = sample["row"].as_i64().unwrap() as usize;
        let column = sample["col"].as_i64().unwrap() as usize;
        let expected = sample["value"].as_f64().unwrap();
        assert_close(block.value(row, column).unwrap(), expected, TOLERANCE);
    }
}

#[test]
fn the_importer_keeps_the_last_wavelength_the_reference_reader_drops() {
    // `tamatrix_importer.import_data_silent` computes
    // `max_rows = endrow - startrow`, which loses the final row. Spectra Studio
    // keeps it. This test exists so the divergence stays deliberate (spec §2.1).
    let dataset = nir800_sheet().unwrap();
    let reference = reference();
    let reference_rows = ta_tests::value(&reference, &["sheet", "reference_reader_shape"])
        [0]
    .as_i64()
    .unwrap() as usize;

    assert_eq!(reference_rows, 1339);
    assert_eq!(dataset.shape().0, 1340);
    assert_close(
        dataset.wavelength().last(),
        number(&reference, &["sheet", "wavelength_last"]),
        TOLERANCE,
    );
}

#[test]
fn importing_the_matrix_with_its_axis_files_matches_the_python_reference() {
    let dataset = nir800_matrix_with_axes().expect("the fixture imports");
    let reference = reference();

    let (rows, cols) = dataset.shape();
    assert_eq!(
        rows,
        integer(&reference, &["matrix_with_axes", "matrix", "rows"]) as usize
    );
    assert_eq!(
        cols,
        integer(&reference, &["matrix_with_axes", "matrix", "cols"]) as usize
    );
    assert_close(
        dataset.wavelength().first(),
        number(&reference, &["matrix_with_axes", "wavelength_first"]),
        TOLERANCE,
    );
    assert_close(
        dataset.wavelength().last(),
        number(&reference, &["matrix_with_axes", "wavelength_last"]),
        TOLERANCE,
    );
    assert_close(
        dataset.delay().last(),
        number(&reference, &["matrix_with_axes", "delay_last"]),
        TOLERANCE,
    );

    let raw = dataset.raw_version().id;
    let data = dataset.matrix(raw).unwrap();
    let (_, _, sum, sum_sq, min, max, finite, missing) = matrix_stats(data);
    assert_eq!(
        finite,
        integer(&reference, &["matrix_with_axes", "matrix", "finite"]) as usize
    );
    assert_eq!(
        missing,
        integer(&reference, &["matrix_with_axes", "matrix", "missing"]) as usize
    );
    assert_close(
        sum,
        number(&reference, &["matrix_with_axes", "matrix", "sum"]),
        TOLERANCE,
    );
    assert_close(
        sum_sq,
        number(&reference, &["matrix_with_axes", "matrix", "sum_sq"]),
        TOLERANCE,
    );
    assert_close(min, number(&reference, &["matrix_with_axes", "matrix", "min"]), TOLERANCE);
    assert_close(max, number(&reference, &["matrix_with_axes", "matrix", "max"]), TOLERANCE);

    for sample in ta_tests::value(&reference, &["matrix_with_axes", "samples"])
        .as_array()
        .unwrap()
    {
        let row = sample["row"].as_i64().unwrap() as usize;
        let column = sample["col"].as_i64().unwrap() as usize;
        assert_close(
            data[[row, column]],
            sample["value"].as_f64().unwrap(),
            TOLERANCE,
        );
    }
}

#[test]
fn a_nearest_trace_matches_numpy_argmin() {
    let dataset = nir800_sheet().unwrap();
    let reference = reference();
    let selection = TraceSelection::new(
        dataset.id,
        dataset.raw_version().id,
        AxisRequest::nearest(number(&reference, &["extraction", "trace_wavelength_nm"])),
    );
    let trace = kinetic_trace(&dataset, &selection).unwrap();

    assert_eq!(
        trace.sampled.indices[0],
        integer(&reference, &["extraction", "trace_sample_index"]) as usize
    );
    assert_close(
        trace.sampled.coordinates[0],
        number(&reference, &["extraction", "trace_sample_wavelength_nm"]),
        TOLERANCE,
    );
    assert_eq!(trace.len(), 152);
    assert_close(
        trace.signal[0],
        number(&reference, &["extraction", "trace_first"]),
        TOLERANCE,
    );
    assert_close(
        trace.signal[151],
        number(&reference, &["extraction", "trace_last"]),
        TOLERANCE,
    );

    let sum: f64 = trace.signal.iter().sum();
    assert_close(
        sum,
        number(&reference, &["extraction", "trace_sum"]),
        1e-9,
    );

    // Every fiftieth delay, which catches a systematically shifted axis.
    let sampled: Vec<f64> = trace.signal.iter().step_by(50).copied().collect();
    assert_slice_close(
        &sampled,
        &numbers(&reference, &["extraction", "trace_values"]),
        1e-9,
    );
}

#[test]
fn the_trace_keeps_the_first_two_delays_the_reference_drops() {
    // `get_traces` reads `tamatrix_avg[pt, 2:]`, so the reference never sees the
    // first two delays. Spectra Studio keeps them; a fit that needs a pre-zero
    // baseline depends on them being there.
    let dataset = nir800_sheet().unwrap();
    let reference = reference();
    let selection = TraceSelection::new(
        dataset.id,
        dataset.raw_version().id,
        AxisRequest::nearest(number(&reference, &["extraction", "trace_wavelength_nm"])),
    );
    let trace = kinetic_trace(&dataset, &selection).unwrap();

    assert_eq!(trace.len(), dataset.delay().len());
    assert_close(trace.delay[0], number(&reference, &["sheet", "delay_first"]), TOLERANCE);
    assert_eq!(trace.within((trace.delay[2], trace.delay[151])).len(), 150);
}

#[test]
fn a_spectrum_at_one_picosecond_matches_the_reference() {
    let dataset = nir800_matrix_with_axes().unwrap();
    let reference = reference();
    let selection = SpectrumSelection::new(
        dataset.id,
        dataset.raw_version().id,
        AxisRequest::nearest(number(&reference, &["extraction", "spectrum_delay_ps"])),
    );
    let spectrum = transient_spectrum(&dataset, &selection).unwrap();

    assert_eq!(
        spectrum.sampled.indices[0],
        integer(&reference, &["extraction", "spectrum_sample_index"]) as usize
    );
    assert_close(
        spectrum.sampled.coordinates[0],
        number(&reference, &["extraction", "spectrum_sample_delay_ps"]),
        TOLERANCE,
    );
    let sum: f64 = spectrum.signal.iter().sum();
    assert_close(sum, number(&reference, &["extraction", "spectrum_sum"]), 1e-9);

    let sampled: Vec<f64> = spectrum.signal.iter().step_by(100).copied().collect();
    assert_slice_close(
        &sampled,
        &numbers(&reference, &["extraction", "spectrum_values"]),
        1e-9,
    );
}

#[test]
fn an_averaged_wavelength_band_matches_the_reference() {
    let dataset = nir800_matrix_with_axes().unwrap();
    let reference = reference();
    let selection = TraceSelection::new(
        dataset.id,
        dataset.raw_version().id,
        AxisRequest::window(
            number(&reference, &["extraction", "band_centre_nm"]),
            number(&reference, &["extraction", "band_half_width_nm"]),
        ),
    );
    let trace = kinetic_trace(&dataset, &selection).unwrap();

    assert_eq!(
        trace.sampled.indices.len(),
        integer(&reference, &["extraction", "band_channels"]) as usize
    );
    let sum: f64 = trace.signal.iter().sum();
    assert_close(
        sum,
        number(&reference, &["extraction", "band_sum"]),
        1e-9,
    );
    let sampled: Vec<f64> = trace.signal.iter().step_by(200).copied().collect();
    assert_slice_close(
        &sampled,
        &numbers(&reference, &["extraction", "band_values"]),
        1e-9,
    );
}

#[test]
fn linear_interpolation_matches_numpy_searchsorted_and_lerp() {
    let dataset = nir800_matrix_with_axes().unwrap();
    let reference = reference();
    let selection = TraceSelection::new(
        dataset.id,
        dataset.raw_version().id,
        AxisRequest::interpolated(number(
            &reference,
            &["extraction", "interpolated_wavelength_nm"],
        )),
    );
    let trace = kinetic_trace(&dataset, &selection).unwrap();

    assert_eq!(
        trace.sampled.indices,
        vec![
            integer(&reference, &["extraction", "interpolated_lower_index"]) as usize,
            integer(&reference, &["extraction", "interpolated_upper_index"]) as usize,
        ]
    );
    assert_close(
        trace.sampled.weights[1],
        number(&reference, &["extraction", "interpolated_fraction"]),
        TOLERANCE,
    );
    assert_close(
        trace.signal[0],
        number(&reference, &["extraction", "interpolated_first"]),
        TOLERANCE,
    );
    let sum: f64 = trace.signal.iter().sum();
    assert_close(
        sum,
        number(&reference, &["extraction", "interpolated_sum"]),
        1e-9,
    );
}

#[test]
fn pre_zero_background_subtraction_matches_the_reference() {
    let dataset = nir800_matrix_with_axes().unwrap();
    let reference = reference();
    let raw = dataset.raw_version().id;
    let data = dataset.matrix(raw).unwrap();
    let first_points = integer(&reference, &["pre_zero_background", "first_points"]) as usize;

    let corrected = ta_processing::background_pre_zero(
        data,
        dataset.delay(),
        ta_core::PreZeroRange::FirstPoints {
            count: first_points,
        },
    )
    .unwrap();

    let columns = ta_processing::pre_zero_columns(
        ta_core::PreZeroRange::FirstPoints {
            count: first_points,
        },
        dataset.delay(),
    )
    .unwrap();
    let reference_spectrum = ta_processing::reference_columns(data, &columns);
    let expected_reference = numbers(&reference, &["pre_zero_background", "reference"]);
    for (index, expected) in expected_reference.iter().enumerate() {
        assert_close(reference_spectrum[[index * 100, 0]], *expected, 1e-6);
    }

    let expected_first = numbers(
        &reference,
        &["pre_zero_background", "corrected_first_column_first_values"],
    );
    for (index, expected) in expected_first.iter().enumerate() {
        // Column 5 is the first column after the baseline window.
        assert_close(corrected[[0, 5 + index]], *expected, 1e-6);
    }
}

#[test]
fn the_correction_line_fixture_parses_into_control_points() {
    let reference = reference();
    let points: Vec<(f64, f64)> = ta_tests::value(&reference, &["correction_line", "points"])
        .as_array()
        .unwrap()
        .iter()
        .map(|point| {
            (
                point["wavelength"].as_f64().unwrap(),
                point["time_zero"].as_f64().unwrap(),
            )
        })
        .collect();
    assert_eq!(points.len(), 10);

    let curve = ta_core::CorrectionCurve::new(
        points
            .iter()
            .map(|(wavelength, time_zero)| ta_core::ControlPoint {
                wavelength: *wavelength,
                time_zero: *time_zero,
            })
            .collect(),
    );
    let sorted = curve.sorted();
    assert!(sorted.windows(2).all(|pair| pair[0].wavelength < pair[1].wavelength));
    assert_close(sorted[0].wavelength, 405.6321, 1e-12);
    assert_close(sorted[9].time_zero, 0.51623, 1e-12);
}
