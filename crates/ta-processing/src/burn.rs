//! Photodegradation ("burn") correction.
//!
//! The reference implementation measures how the signal at one wavelength
//! drifts between the *first* and the *last* of a set of parallel experiments,
//! turns that drift into a factor per delay point, and rescales the matrix with
//! it.
//!
//! Its own code cannot run as written: it multiplies a slice of the matrix
//! (`[:, 2:]`) by an array with one element per delay, and the two shapes cannot
//! broadcast. It also indexes `delay_index + 2` without a bound check. Both are
//! named in `docs/numerics.md`; what is implemented here is the intent those
//! lines describe.

use ndarray::Array2;
use ta_core::{Axis, CoreError, Result};

/// The factor applied to each delay column.
///
/// `first` and `last` are the same measurement repeated, earliest and latest.
/// The drift between them at one wavelength, as a fraction of the signal there,
/// is spread over the whole run in proportion to how far through it each delay
/// point sits — which is what a sample that is being damaged at a steady rate
/// looks like.
pub fn burn_correction_factors(
    first: &Array2<f64>,
    last: &Array2<f64>,
    wavelength: &Axis,
    delay: &Axis,
    wavelength_of_interest: f64,
    reference_delay: f64,
) -> Result<Vec<f64>> {
    if first.dim() != last.dim() {
        return Err(CoreError::InputShapeMismatch {
            first: first.dim(),
            second: last.dim(),
        });
    }
    if first.nrows() != wavelength.len() || first.ncols() != delay.len() {
        return Err(CoreError::ShapeMismatch {
            rows: first.nrows(),
            cols: first.ncols(),
            wavelengths: wavelength.len(),
            delays: delay.len(),
        });
    }
    let (row, _) = wavelength.nearest(wavelength_of_interest);
    let (reference, _) = delay.nearest(reference_delay);

    // The reference point sits a couple of points past the reference delay, so
    // the rise is over rather than under way.
    let probe = (reference + 2).min(delay.len() - 1);
    let earliest = first[[row, probe]];
    let latest = last[[row, probe]];
    if !(earliest.is_finite() && latest.is_finite()) {
        return Err(CoreError::invalid(format!(
            "the burn reference at {:.4} nm and {:.4} ps is missing from one of the experiments",
            wavelength.values()[row],
            delay.values()[probe]
        )));
    }
    if earliest == 0.0 {
        return Err(CoreError::invalid(format!(
            "the signal at {:.4} nm and {:.4} ps is zero, so a drift cannot be measured against it",
            wavelength.values()[row],
            delay.values()[probe]
        )));
    }

    let points = delay.len() as f64;
    let per_point = (earliest - latest) / earliest / points;
    Ok((0..delay.len())
        .map(|index| 1.0 + per_point * index as f64)
        .collect())
}

/// Applies the factors from the reference delay onwards.
///
/// Earlier columns are left alone: they are the baseline the drift is measured
/// against, and scaling them would move the baseline that background
/// subtraction already removed.
pub fn correct_burn(
    matrix: &Array2<f64>,
    first: &Array2<f64>,
    last: &Array2<f64>,
    wavelength: &Axis,
    delay: &Axis,
    wavelength_of_interest: f64,
    reference_delay: f64,
) -> Result<Array2<f64>> {
    let factors = burn_correction_factors(
        first,
        last,
        wavelength,
        delay,
        wavelength_of_interest,
        reference_delay,
    )?;
    if matrix.dim() != first.dim() {
        return Err(CoreError::InputShapeMismatch {
            first: matrix.dim(),
            second: first.dim(),
        });
    }
    let (reference, _) = delay.nearest(reference_delay);
    let mut corrected = matrix.clone();
    for row in 0..matrix.nrows() {
        for column in reference..matrix.ncols() {
            corrected[[row, column]] = matrix[[row, column]] * factors[column];
        }
    }
    Ok(corrected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::Array2;
    use ta_core::{AxisKind, AxisUnit};

    fn axes() -> (Axis, Axis) {
        (
            Axis::new(
                AxisKind::Wavelength,
                AxisUnit::Nanometer,
                "Wavelength",
                vec![500.0, 550.0, 600.0],
            )
            .unwrap(),
            Axis::new(
                AxisKind::Delay,
                AxisUnit::Picosecond,
                "Delay",
                vec![0.0, 1.0, 2.0, 5.0, 10.0],
            )
            .unwrap(),
        )
    }

    #[test]
    fn a_measurement_that_faded_between_the_first_and_last_experiment_is_scaled_back_up() {
        let (wavelength, delay) = axes();
        // The signal at 550 nm is 10 % smaller in the last experiment.
        let first = Array2::from_elem((3, 5), 1.0);
        let mut last = Array2::from_elem((3, 5), 1.0);
        last[[1, 2]] = 0.9;
        last[[1, 3]] = 0.9;
        last[[1, 4]] = 0.9;
        let factors =
            burn_correction_factors(&first, &last, &wavelength, &delay, 550.0, 0.0).unwrap();
        // Five points, a tenth of the signal lost across them: 2 % per point.
        assert!((factors[0] - 1.0).abs() < 1e-12);
        assert!((factors[1] - 1.02).abs() < 1e-12);
        assert!((factors[4] - 1.08).abs() < 1e-12);

        let corrected =
            correct_burn(&first, &first, &last, &wavelength, &delay, 550.0, 0.0).unwrap();
        // Columns before the reference delay are untouched.
        assert_eq!(corrected[[0, 0]], 1.0);
        // And the last column is lifted by the accumulated factor.
        assert!((corrected[[0, 4]] - 1.08).abs() < 1e-12);
    }

    #[test]
    fn the_probe_sits_two_points_past_the_reference_delay() {
        let (wavelength, delay) = axes();
        let first = Array2::from_elem((3, 5), 1.0);
        let mut last = Array2::from_elem((3, 5), 1.0);
        // Only the third point (2 ps) differs, which is the probe for a 0 ps
        // reference delay.
        last[[1, 2]] = 0.5;
        last[[1, 3]] = 0.5;
        last[[1, 4]] = 0.5;
        let factors =
            burn_correction_factors(&first, &last, &wavelength, &delay, 550.0, 0.0).unwrap();
        // 0.5 lost over five points is 10 % per point.
        assert!((factors[4] - 1.4).abs() < 1e-12, "{factors:?}");
    }

    #[test]
    fn a_reference_point_beyond_the_last_delay_is_clamped_rather_than_indexed_out_of_bounds() {
        let (wavelength, delay) = axes();
        let first = Array2::from_elem((3, 5), 1.0);
        let last = Array2::from_elem((3, 5), 0.8);
        // A reference delay past the end of the axis: the reference reader
        // would index past the array here.
        let factors =
            burn_correction_factors(&first, &last, &wavelength, &delay, 550.0, 9.5).unwrap();
        assert_eq!(factors.len(), 5);
        assert!(factors.iter().all(|factor| factor.is_finite()));
    }

    #[test]
    fn a_zero_reference_signal_is_refused_rather_than_dividing_by_it() {
        let (wavelength, delay) = axes();
        let first = Array2::zeros((3, 5));
        let last = Array2::from_elem((3, 5), 0.5);
        assert!(burn_correction_factors(&first, &last, &wavelength, &delay, 550.0, 0.0).is_err());
    }

    #[test]
    fn a_missing_reference_point_is_reported_with_its_coordinates() {
        let (wavelength, delay) = axes();
        let mut first = Array2::from_elem((3, 5), 1.0);
        first[[1, 2]] = f64::NAN;
        let last = Array2::from_elem((3, 5), 0.9);
        let error =
            burn_correction_factors(&first, &last, &wavelength, &delay, 550.0, 0.0).unwrap_err();
        let message = format!("{error}");
        assert!(message.contains("550"), "{message}");
        assert!(message.contains("2"), "{message}");
    }

    #[test]
    fn experiments_that_do_not_match_each_other_are_refused() {
        let (wavelength, delay) = axes();
        let first = Array2::from_elem((3, 5), 1.0);
        let last = Array2::from_elem((2, 5), 0.9);
        assert!(burn_correction_factors(&first, &last, &wavelength, &delay, 550.0, 0.0).is_err());
    }
}
