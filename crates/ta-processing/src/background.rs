//! Baseline subtraction.
//!
//! Two sources of baseline: the sample's own pre-time-zero region and an
//! external reference measurement.

use ndarray::Array2;
use ta_core::{Axis, CoreError, PreZeroRange, Result};

/// The delay columns a pre-zero baseline is averaged over.
///
/// `FirstPoints` reproduces the reference implementation's default of "the
/// first N time points"; `Interval` is the explicit interval the interface
/// offers instead. Both resolve to real column indices, in axis order, so the
/// overlay can be drawn on the contour from the same answer the arithmetic
/// uses.
pub fn pre_zero_columns(range: PreZeroRange, delay: &Axis) -> Result<Vec<usize>> {
    match range {
        PreZeroRange::FirstPoints { count } => {
            if count == 0 {
                return Err(CoreError::invalid(
                    "a baseline cannot be averaged over zero time points",
                ));
            }
            if count > delay.len() {
                return Err(CoreError::invalid(format!(
                    "a baseline cannot be averaged over {count} time points when the dataset \
                     has {}",
                    delay.len()
                )));
            }
            Ok((0..count).collect())
        }
        PreZeroRange::Interval { start, end } => {
            let low = start.min(end);
            let high = start.max(end);
            let columns: Vec<usize> = delay
                .values()
                .iter()
                .enumerate()
                .filter(|(_, value)| **value >= low && **value <= high)
                .map(|(index, _)| index)
                .collect();
            if columns.is_empty() {
                return Err(CoreError::EmptyWindow {
                    axis: "delay",
                    center: (low + high) / 2.0,
                });
            }
            Ok(columns)
        }
    }
}

/// The reference spectrum: one value per wavelength, the mean over `columns`.
///
/// A row whose baseline columns are all missing stays missing rather than
/// becoming zero, and a row with some missing columns averages what is there.
/// That is the one deliberate difference from the reference implementation,
/// which propagates `NaN` through the whole row; with complete data the two
/// agree to floating-point precision.
pub fn reference_columns(matrix: &Array2<f64>, columns: &[usize]) -> Array2<f64> {
    let mut reference = Array2::zeros((matrix.nrows(), 1));
    for row in 0..matrix.nrows() {
        let mut total = 0.0;
        let mut count = 0usize;
        for column in columns {
            let value = matrix[[row, *column]];
            if value.is_finite() {
                total += value;
                count += 1;
            }
        }
        reference[[row, 0]] = if count == 0 {
            f64::NAN
        } else {
            total / count as f64
        };
    }
    reference
}

/// Subtracts the mean of the chosen pre-zero columns from every column.
pub fn background_pre_zero(
    matrix: &Array2<f64>,
    delay: &Axis,
    range: PreZeroRange,
) -> Result<Array2<f64>> {
    let columns = pre_zero_columns(range, delay)?;
    let reference = reference_columns(matrix, &columns);
    Ok(subtract_reference(matrix, &reference))
}

/// Subtracts a reference spectrum (one value per wavelength, as a column
/// vector) from every delay column.
pub fn subtract_reference(matrix: &Array2<f64>, reference: &Array2<f64>) -> Array2<f64> {
    let mut corrected = matrix.clone();
    for row in 0..matrix.nrows() {
        let offset = reference[[row, 0]];
        for column in 0..matrix.ncols() {
            corrected[[row, column]] = matrix[[row, column]] - offset;
        }
    }
    corrected
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_core::{AxisKind, AxisUnit};
    use ndarray::array;

    fn delay_axis(values: Vec<f64>) -> Axis {
        Axis::new(AxisKind::Delay, AxisUnit::Picosecond, "Delay", values).unwrap()
    }

    #[test]
    fn first_n_points_are_the_leading_columns() {
        let axis = delay_axis(vec![-1.0, -0.5, 0.0, 1.0]);
        assert_eq!(
            pre_zero_columns(PreZeroRange::FirstPoints { count: 2 }, &axis).unwrap(),
            vec![0, 1]
        );
    }

    #[test]
    fn an_explicit_interval_selects_by_physical_coordinate() {
        let axis = delay_axis(vec![-10.0, -1.0, -0.5, 0.0, 1.0]);
        assert_eq!(
            pre_zero_columns(
                PreZeroRange::Interval {
                    start: -5.0,
                    end: -0.2
                },
                &axis
            )
            .unwrap(),
            vec![1, 2]
        );
        // Reversed endpoints describe the same interval.
        assert_eq!(
            pre_zero_columns(
                PreZeroRange::Interval {
                    start: -0.2,
                    end: -5.0
                },
                &axis
            )
            .unwrap(),
            vec![1, 2]
        );
    }

    #[test]
    fn an_interval_that_covers_nothing_is_an_error() {
        let axis = delay_axis(vec![0.0, 1.0]);
        assert!(pre_zero_columns(
            PreZeroRange::Interval {
                start: 50.0,
                end: 60.0
            },
            &axis
        )
        .is_err());
    }

    #[test]
    fn subtracting_the_first_points_matches_the_reference_arithmetic() {
        let axis = delay_axis(vec![-1.0, 0.0, 1.0]);
        let matrix = array![[0.0, 1.0, 2.0], [1.0, 3.0, 5.0]];
        let corrected =
            background_pre_zero(&matrix, &axis, PreZeroRange::FirstPoints { count: 1 }).unwrap();
        // Row 0 loses 0.0, row 1 loses 1.0; the original is untouched.
        assert_eq!(corrected, array![[0.0, 1.0, 2.0], [0.0, 2.0, 4.0]]);
        assert_eq!(matrix, array![[0.0, 1.0, 2.0], [1.0, 3.0, 5.0]]);
    }

    #[test]
    fn a_partly_missing_baseline_averages_what_is_there() {
        let axis = delay_axis(vec![-2.0, -1.0, 0.0]);
        let mut matrix = array![[0.0, 2.0, 10.0], [1.0, 3.0, 11.0]];
        matrix[[0, 1]] = f64::NAN;
        let corrected =
            background_pre_zero(&matrix, &axis, PreZeroRange::FirstPoints { count: 2 }).unwrap();
        // Row 0 averages only the finite baseline column, so it loses 0.0.
        assert_eq!(corrected[[0, 2]], 10.0);
        // Row 1 loses the mean of 1.0 and 3.0.
        assert_eq!(corrected[[1, 2]], 9.0);
    }

    #[test]
    fn a_row_with_no_finite_baseline_stays_missing() {
        let axis = delay_axis(vec![-2.0, -1.0, 0.0]);
        let mut matrix = array![[0.0, 0.0, 10.0]];
        matrix[[0, 0]] = f64::NAN;
        matrix[[0, 1]] = f64::NAN;
        let corrected =
            background_pre_zero(&matrix, &axis, PreZeroRange::FirstPoints { count: 2 }).unwrap();
        assert!(corrected[[0, 2]].is_nan());
    }

    #[test]
    fn asking_for_more_baseline_points_than_exist_is_refused() {
        let axis = delay_axis(vec![0.0, 1.0]);
        let matrix = array![[0.0, 1.0]];
        assert!(background_pre_zero(&matrix, &axis, PreZeroRange::FirstPoints { count: 5 }).is_err());
        assert!(background_pre_zero(&matrix, &axis, PreZeroRange::FirstPoints { count: 0 }).is_err());
    }
}
