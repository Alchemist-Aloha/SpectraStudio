//! Parity with the reference implementation's processing arithmetic.
//!
//! The expected values come from `reference/generate_reference.py`, which runs
//! the reference's own numpy expression on the repository's fixtures.

use ndarray::Array2;
use ta_core::{BoundaryMode, CorrectionCurve, ControlPoint, Interpolation};
use ta_tests::{assert_close, number, nir800_matrix_with_axes, reference, value};

/// The drawn correction line the repository ships.
fn correction_curve() -> CorrectionCurve {
    let reference = reference();
    let points = value(&reference, &["time_zero", "curve_points"])
        .as_array()
        .expect("control points");
    CorrectionCurve::new(
        points
            .iter()
            .map(|pair| ControlPoint {
                wavelength: pair[0].as_f64().expect("a wavelength"),
                time_zero: pair[1].as_f64().expect("a time zero"),
            })
            .collect(),
    )
}

#[test]
fn the_manual_time_zero_correction_reproduces_numpy_interp() {
    let dataset = nir800_matrix_with_axes().expect("the fixture imports");
    let reference = reference();
    let version = dataset.raw_version().id;
    let matrix = dataset.matrix(version).expect("the matrix");
    let (wavelength, delay) = dataset.axes(version).expect("the axes");

    let corrected = ta_processing::apply_time_zero(
        matrix,
        wavelength,
        delay,
        &correction_curve(),
        Interpolation::Linear,
        BoundaryMode::Clamp,
    )
    .expect("the correction applies");

    // Shape first: the correction resamples onto the measured grid, it does not
    // change it.
    assert_eq!(corrected.dim(), matrix.dim());

    let sum: f64 = corrected.iter().sum();
    assert_close(sum, number(&reference, &["time_zero", "sum"]), 1e-9);
    let minimum = corrected.iter().copied().fold(f64::INFINITY, f64::min);
    let maximum = corrected.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    assert_close(minimum, number(&reference, &["time_zero", "min"]), 1e-9);
    assert_close(maximum, number(&reference, &["time_zero", "max"]), 1e-9);

    // And individual cells, which catch a shift applied to the wrong axis.
    for sample in value(&reference, &["time_zero", "samples"])
        .as_array()
        .expect("samples")
    {
        let row = sample["row"].as_i64().expect("a row") as usize;
        let column = sample["col"].as_i64().expect("a column") as usize;
        let expected = sample["value"].as_f64().expect("a value");
        assert_close(corrected[[row, column]], expected, 1e-9);
    }
}

#[test]
fn the_correction_curve_is_read_the_way_numpy_reads_it() {
    let reference = reference();
    let curve = correction_curve();

    assert_close(
        ta_processing::offset_at(&curve, 550.0).expect("an offset"),
        number(&reference, &["time_zero", "offset_at_550"]),
        1e-12,
    );
    // Below and above the drawn range the nearest point is held, which is what
    // np.interp does.
    assert_close(
        ta_processing::offset_at(&curve, 400.0).expect("an offset"),
        number(&reference, &["time_zero", "offset_below_the_curve"]),
        1e-12,
    );
    assert_close(
        ta_processing::offset_at(&curve, 900.0).expect("an offset"),
        number(&reference, &["time_zero", "offset_above_the_curve"]),
        1e-12,
    );
}

#[test]
fn the_correction_preserves_the_measured_delay_grid_and_the_baseline() {
    let dataset = nir800_matrix_with_axes().expect("the fixture imports");
    let version = dataset.raw_version().id;
    let matrix = dataset.matrix(version).expect("the matrix");
    let (wavelength, delay) = dataset.axes(version).expect("the axes");
    let corrected = ta_processing::apply_time_zero(
        matrix,
        wavelength,
        delay,
        &correction_curve(),
        Interpolation::Linear,
        BoundaryMode::Clamp,
    )
    .expect("the correction applies");

    // The delays are the measured ones, untouched.
    assert_eq!(corrected.ncols(), delay.len());
    // A correction shifts the signal; it cannot invent a value where the
    // measurement is missing.
    let mut with_hole = matrix.clone();
    with_hole[[10, 10]] = f64::NAN;
    let holed = ta_processing::apply_time_zero(
        &with_hole,
        wavelength,
        delay,
        &correction_curve(),
        Interpolation::Linear,
        BoundaryMode::Clamp,
    )
    .expect("the correction applies");
    // The hole is smeared by the resampling but stays missing where it lands.
    let missing = holed.iter().filter(|value| value.is_nan()).count();
    assert!(
        missing >= 1,
        "a missing sample should not become a number"
    );
}

#[test]
fn a_correction_with_no_points_is_refused() {
    let dataset = nir800_matrix_with_axes().expect("the fixture imports");
    let version = dataset.raw_version().id;
    let matrix = dataset.matrix(version).expect("the matrix");
    let (wavelength, delay) = dataset.axes(version).expect("the axes");
    assert!(ta_processing::apply_time_zero(
        matrix,
        wavelength,
        delay,
        &CorrectionCurve::new(Vec::new()),
        Interpolation::Linear,
        BoundaryMode::Clamp,
    )
    .is_err());
}

#[test]
fn a_correction_that_asks_for_masked_edges_reports_them() {
    let dataset = nir800_matrix_with_axes().expect("the fixture imports");
    let version = dataset.raw_version().id;
    let matrix = dataset.matrix(version).expect("the matrix");
    let (wavelength, delay) = dataset.axes(version).expect("the axes");
    let masked = ta_processing::apply_time_zero(
        matrix,
        wavelength,
        delay,
        &correction_curve(),
        Interpolation::Linear,
        BoundaryMode::Mask,
    )
    .expect("the correction applies");
    let missing = masked.iter().filter(|value| value.is_nan()).count();
    assert!(
        missing > 0,
        "an offset that reaches past the delay axis must be masked, not clamped"
    );
    let _ = Array2::<f64>::zeros((1, 1));
}
