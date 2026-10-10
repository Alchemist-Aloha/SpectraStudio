//! Time-zero (chirp) correction.
//!
//! The reference implementation's resampling is three lines, and they define
//! the convention everything else follows:
//!
//! ```text
//! time_temp = tatime + interp(tawavelength[i], zerotime_x, zerotime_y)
//! tcorr[i, :] = interp(time_temp, tatime, oldmatrix[i, :])
//! ```
//!
//! So the output keeps the original delay axis, and the value at delay `t` is
//! the signal the measurement had at `t + offset`. `np.interp` clamps outside
//! its range, which is why [`BoundaryMode::Clamp`] is the faithful default.

use ndarray::Array2;
use ta_core::{Axis, BoundaryMode, ControlPoint, CorrectionCurve, CoreError, Interpolation, Result};

/// The time-zero offset at a wavelength, in the convention above.
///
/// Outside the control points' wavelength range the nearest point is held, which
/// is what `np.interp` does; a correction curve is only ever drawn over the
/// region a user could see, so extrapolating a chirp beyond it would be
/// invention.
pub fn offset_at(curve: &CorrectionCurve, wavelength: f64) -> Result<f64> {
    offset_with(curve, wavelength, Interpolation::Linear)
}

pub fn offset_with(
    curve: &CorrectionCurve,
    wavelength: f64,
    interpolation: Interpolation,
) -> Result<f64> {
    let points = curve.sorted();
    if points.is_empty() {
        return Err(CoreError::invalid(
            "a correction curve with no control points corrects nothing",
        ));
    }
    if !wavelength.is_finite() {
        return Err(CoreError::invalid(format!(
            "cannot read a correction curve at {wavelength} nm"
        )));
    }
    let wavelengths: Vec<f64> = points.iter().map(|point| point.wavelength).collect();
    let offsets: Vec<f64> = points.iter().map(|point| point.time_zero).collect();
    if points.len() == 1 {
        return Ok(offsets[0]);
    }
    if wavelength <= wavelengths[0] {
        return Ok(offsets[0]);
    }
    let last = wavelengths.len() - 1;
    if wavelength >= wavelengths[last] {
        return Ok(offsets[last]);
    }
    Ok(match interpolation {
        Interpolation::Linear => linear_eval(&wavelengths, &offsets, wavelength),
        Interpolation::ShapePreserving => {
            monotone_cubic_eval(&wavelengths, &offsets, wavelength)
        }
    })
}

fn linear_eval(xs: &[f64], ys: &[f64], x: f64) -> f64 {
    let mut low = 0usize;
    let mut high = xs.len() - 1;
    while high - low > 1 {
        let mid = low + (high - low) / 2;
        if xs[mid] <= x {
            low = mid;
        } else {
            high = mid;
        }
    }
    let span = xs[high] - xs[low];
    if span == 0.0 {
        return ys[low];
    }
    let t = (x - xs[low]) / span;
    ys[low] + (ys[high] - ys[low]) * t
}

/// Monotone cubic interpolation (Fritsch–Carlson), which cannot overshoot the
/// control points — a correction curve that bulged between the points a user
/// placed would shift the time zero nowhere near where they put it.
fn monotone_cubic_eval(xs: &[f64], ys: &[f64], x: f64) -> f64 {
    let n = xs.len();
    if n < 3 {
        return linear_eval(xs, ys, x);
    }
    let mut slopes = vec![0.0f64; n - 1];
    for index in 0..n - 1 {
        let span = xs[index + 1] - xs[index];
        if span == 0.0 {
            return linear_eval(xs, ys, x);
        }
        slopes[index] = (ys[index + 1] - ys[index]) / span;
    }
    let mut tangents = vec![0.0f64; n];
    tangents[0] = slopes[0];
    tangents[n - 1] = slopes[n - 2];
    for index in 1..n - 1 {
        if slopes[index - 1] * slopes[index] <= 0.0 {
            // A local extremum: flatten the tangent so the curve cannot leave
            // the interval between its neighbours.
            tangents[index] = 0.0;
        } else {
            let weight_a = 2.0 * (xs[index + 1] - xs[index]) + (xs[index] - xs[index - 1]);
            let weight_b = (xs[index + 1] - xs[index]) + 2.0 * (xs[index] - xs[index - 1]);
            tangents[index] = (weight_a + weight_b)
                / (weight_a / slopes[index - 1] + weight_b / slopes[index]);
        }
    }

    let mut low = 0usize;
    let mut high = n - 1;
    while high - low > 1 {
        let mid = low + (high - low) / 2;
        if xs[mid] <= x {
            low = mid;
        } else {
            high = mid;
        }
    }
    let span = xs[high] - xs[low];
    if span == 0.0 {
        return ys[low];
    }
    let t = (x - xs[low]) / span;
    let t2 = t * t;
    let t3 = t2 * t;
    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h10 = t3 - 2.0 * t2 + t;
    let h01 = -2.0 * t3 + 3.0 * t2;
    let h11 = t3 - t2;
    h00 * ys[low]
        + h10 * span * tangents[low]
        + h01 * ys[high]
        + h11 * span * tangents[high]
}

/// Resamples one trace at the delays shifted by its offset.
fn resample(values: &[f64], delays: &[f64], offset: f64, boundary: BoundaryMode) -> Vec<f64> {
    delays
        .iter()
        .map(|delay| {
            let target = delay + offset;
            let (low, high) = (delays[0], delays[delays.len() - 1]);
            if target < low || target > high {
                return match boundary {
                    BoundaryMode::Clamp => {
                        if target < low {
                            values[0]
                        } else {
                            values[values.len() - 1]
                        }
                    }
                    BoundaryMode::Mask => f64::NAN,
                    BoundaryMode::Extrapolate => {
                        // The edge slope, extended.
                        if delays.len() < 2 {
                            values[0]
                        } else if target < low {
                            let slope = (values[1] - values[0]) / (delays[1] - delays[0]);
                            values[0] + slope * (target - low)
                        } else {
                            let last = delays.len() - 1;
                            let slope =
                                (values[last] - values[last - 1]) / (delays[last] - delays[last - 1]);
                            values[last] + slope * (target - high)
                        }
                    }
                };
            }
            if delays.len() == 1 {
                return values[0];
            }
            let value = linear_eval(delays, values, target);
            if value.is_finite() {
                value
            } else {
                f64::NAN
            }
        })
        .collect()
}

/// Applies a correction curve to a whole matrix.
///
/// The axes come back unchanged: the correction moves the signal onto the
/// measured delay grid, it does not redefine that grid (`spec.md` §9.2).
pub fn apply_time_zero(
    matrix: &Array2<f64>,
    wavelength: &Axis,
    delay: &Axis,
    correction: &CorrectionCurve,
    interpolation: Interpolation,
    boundary: BoundaryMode,
) -> Result<Array2<f64>> {
    let (rows, columns) = matrix.dim();
    if rows != wavelength.len() || columns != delay.len() {
        return Err(CoreError::ShapeMismatch {
            rows,
            cols: columns,
            wavelengths: wavelength.len(),
            delays: delay.len(),
        });
    }
    if correction.is_empty() {
        return Err(CoreError::invalid(
            "no correction curve has been set for this dataset",
        ));
    }
    let delays = delay.values();
    let mut corrected = Array2::zeros((rows, columns));
    for row in 0..rows {
        let offset = offset_with(correction, wavelength.values()[row], interpolation)?;
        let values: Vec<f64> = (0..columns).map(|column| matrix[[row, column]]).collect();
        let resampled = resample(&values, delays, offset, boundary);
        for column in 0..columns {
            corrected[[row, column]] = resampled[column];
        }
    }
    Ok(corrected)
}

/// A control point read out of a drawn correction line: `(wavelength, time
/// zero)` pairs, which is how the reference implementation's editor saves them.
pub fn curve_from_points(points: &[(f64, f64)]) -> CorrectionCurve {
    CorrectionCurve::new(
        points
            .iter()
            .filter(|(wavelength, time_zero)| wavelength.is_finite() && time_zero.is_finite())
            .map(|(wavelength, time_zero)| ControlPoint {
                wavelength: *wavelength,
                time_zero: *time_zero,
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_core::{AxisKind, AxisUnit};

    fn curve() -> CorrectionCurve {
        curve_from_points(&[(450.0, -0.5), (600.0, 0.0), (750.0, 0.5)])
    }

    fn axes() -> (Axis, Axis) {
        (
            Axis::new(
                AxisKind::Wavelength,
                AxisUnit::Nanometer,
                "Wavelength",
                vec![450.0, 600.0, 750.0],
            )
            .unwrap(),
            Axis::new(
                AxisKind::Delay,
                AxisUnit::Picosecond,
                "Delay",
                vec![-1.0, 0.0, 1.0, 2.0],
            )
            .unwrap(),
        )
    }

    #[test]
    fn the_offset_is_linear_between_control_points() {
        let curve = curve();
        assert_eq!(offset_at(&curve, 450.0).unwrap(), -0.5);
        assert_eq!(offset_at(&curve, 600.0).unwrap(), 0.0);
        assert!((offset_at(&curve, 525.0).unwrap() + 0.25).abs() < 1e-12);
    }

    #[test]
    fn outside_the_curve_the_nearest_point_is_held_like_numpy_interp() {
        let curve = curve();
        assert_eq!(offset_at(&curve, 300.0).unwrap(), -0.5);
        assert_eq!(offset_at(&curve, 2000.0).unwrap(), 0.5);
    }

    #[test]
    fn a_single_control_point_corrects_everywhere() {
        let curve = curve_from_points(&[(550.0, 0.25)]);
        assert_eq!(offset_at(&curve, 400.0).unwrap(), 0.25);
        assert_eq!(offset_at(&curve, 800.0).unwrap(), 0.25);
    }

    #[test]
    fn an_empty_curve_is_an_error_rather_than_a_silent_no_op() {
        let curve = curve_from_points(&[]);
        assert!(offset_at(&curve, 500.0).is_err());
        let (wavelength, delay) = axes();
        let matrix = Array2::zeros((3, 4));
        assert!(apply_time_zero(
            &matrix,
            &wavelength,
            &delay,
            &curve,
            Interpolation::Linear,
            BoundaryMode::Clamp
        )
        .is_err());
    }

    #[test]
    fn the_shape_preserving_curve_cannot_overshoot_its_points() {
        let curve = curve_from_points(&[(400.0, -0.5), (500.0, -0.4), (600.0, 0.4), (700.0, 0.5)]);
        for step in 0..=600 {
            let wavelength = 400.0 + step as f64 * 0.5;
            let offset = offset_with(&curve, wavelength, Interpolation::ShapePreserving).unwrap();
            assert!(
                (-0.5..=0.5).contains(&offset),
                "at {wavelength} nm the curve left its points: {offset}"
            );
        }
        // And it still passes through the control points.
        assert!((offset_with(&curve, 500.0, Interpolation::ShapePreserving).unwrap() + 0.4).abs() < 1e-12);
    }

    #[test]
    fn a_correction_shifts_each_wavelength_by_its_own_offset() {
        let (wavelength, delay) = axes();
        // A signal that rises at t = 0 everywhere.
        let mut matrix = Array2::zeros((3, 4));
        for row in 0..3 {
            for column in 0..4 {
                matrix[[row, column]] = delay.values()[column];
            }
        }
        let corrected = apply_time_zero(
            &matrix,
            &wavelength,
            &delay,
            &curve(),
            Interpolation::Linear,
            BoundaryMode::Clamp,
        )
        .unwrap();

        // The 600 nm row has no offset, so it is unchanged.
        for column in 0..4 {
            assert!((corrected[[1, column]] - matrix[[1, column]]).abs() < 1e-12);
        }
        // The 450 nm row is shifted by -0.5 ps: its value at delay t is what
        // the raw trace held at t - 0.5.
        for (column, t) in delay.values().iter().enumerate() {
            let offset = offset_at(&curve(), 450.0).unwrap();
            let expected = t + offset;
            let expected = if expected < -1.0 { -1.0 } else { expected };
            assert!(
                (corrected[[0, column]] - expected).abs() < 1e-12,
                "at {t}: {} against {expected}",
                corrected[[0, column]]
            );
        }
        // The shape is preserved.
        assert_eq!(corrected.dim(), matrix.dim());
    }

    #[test]
    fn the_boundary_mode_decides_what_leaves_the_trace_missing() {
        let (wavelength, delay) = axes();
        let mut matrix = Array2::zeros((3, 4));
        for column in 0..4 {
            matrix[[0, column]] = delay.values()[column];
        }
        // An offset of -3 ps takes the first column from -1 ps to -4 ps, which
        // is below the measured range.
        let curve = curve_from_points(&[(450.0, -3.0)]);
        let masked = apply_time_zero(
            &matrix,
            &wavelength,
            &delay,
            &curve,
            Interpolation::Linear,
            BoundaryMode::Mask,
        )
        .unwrap();
        assert!(masked[[0, 0]].is_nan(), "a masked edge must be missing");
        assert!(!masked[[0, 3]].is_nan(), "the interior is measured");

        let clamped = apply_time_zero(
            &matrix,
            &wavelength,
            &delay,
            &curve,
            Interpolation::Linear,
            BoundaryMode::Clamp,
        )
        .unwrap();
        assert_eq!(clamped[[0, 0]], -1.0, "the edge value is held");

        let extrapolated = apply_time_zero(
            &matrix,
            &wavelength,
            &delay,
            &curve,
            Interpolation::Linear,
            BoundaryMode::Extrapolate,
        )
        .unwrap();
        assert!(
            (extrapolated[[0, 0]] + 4.0).abs() < 1e-12,
            "the edge slope extended: {}",
            extrapolated[[0, 0]]
        );
    }

    #[test]
    fn a_matrix_whose_shape_disagrees_with_its_axes_is_refused() {
        let (wavelength, delay) = axes();
        let matrix = Array2::zeros((2, 4));
        assert!(apply_time_zero(
            &matrix,
            &wavelength,
            &delay,
            &curve(),
            Interpolation::Linear,
            BoundaryMode::Clamp
        )
        .is_err());
    }

    #[test]
    fn a_curve_built_from_drawn_points_drops_the_missing_ones() {
        let curve = curve_from_points(&[
            (450.0, -0.5),
            (f64::NAN, 0.0),
            (600.0, 0.0),
            (f64::INFINITY, 1.0),
        ]);
        assert_eq!(curve.points.len(), 2);
    }
}
