//! Cropping a matrix to a wavelength and/or delay interval.

use ndarray::Array2;
use ta_core::{Axis, CoreError, CropSpec, Result};

/// A cropped matrix and the axes that describe it.
#[derive(Clone, Debug, PartialEq)]
pub struct Cropped {
    pub wavelength: Axis,
    pub delay: Axis,
    pub data: Array2<f64>,
    /// The wavelength indices kept, in order.
    pub wavelength_indices: Vec<usize>,
    /// The delay indices kept, in order.
    pub delay_indices: Vec<usize>,
}

/// Keeps the samples that fall inside the requested intervals.
///
/// Both endpoints are inclusive, a reversed pair of limits means the same
/// interval, and descending axes are kept descending: cropping selects, it does
/// not reorder. An interval that would leave nothing is an error rather than an
/// empty matrix.
pub fn crop_matrix(
    data: &Array2<f64>,
    wavelength: &Axis,
    delay: &Axis,
    spec: &CropSpec,
) -> Result<Cropped> {
    if data.nrows() != wavelength.len() || data.ncols() != delay.len() {
        return Err(CoreError::ShapeMismatch {
            rows: data.nrows(),
            cols: data.ncols(),
            wavelengths: wavelength.len(),
            delays: delay.len(),
        });
    }

    let wavelength_indices = select(wavelength, spec.wavelength, "wavelength")?;
    let delay_indices = select(delay, spec.delay, "delay")?;

    let mut cropped = Array2::zeros((wavelength_indices.len(), delay_indices.len()));
    for (row, source_row) in wavelength_indices.iter().enumerate() {
        for (column, source_column) in delay_indices.iter().enumerate() {
            cropped[[row, column]] = data[[*source_row, *source_column]];
        }
    }

    let new_wavelength = Axis::new(
        wavelength.kind(),
        wavelength.unit(),
        wavelength.name(),
        wavelength_indices
            .iter()
            .map(|index| wavelength.values()[*index])
            .collect(),
    )?;
    let new_delay = Axis::new(
        delay.kind(),
        delay.unit(),
        delay.name(),
        delay_indices
            .iter()
            .map(|index| delay.values()[*index])
            .collect(),
    )?;

    Ok(Cropped {
        wavelength: new_wavelength,
        delay: new_delay,
        data: cropped,
        wavelength_indices,
        delay_indices,
    })
}

fn select(axis: &Axis, limits: Option<(f64, f64)>, name: &'static str) -> Result<Vec<usize>> {
    let Some((first, second)) = limits else {
        return Ok((0..axis.len()).collect());
    };
    let low = first.min(second);
    let high = first.max(second);
    if !(low.is_finite() && high.is_finite()) {
        return Err(CoreError::invalid(format!(
            "the {name} crop interval {first}..={second} is not a finite range"
        )));
    }
    let indices: Vec<usize> = axis
        .values()
        .iter()
        .enumerate()
        .filter(|(_, value)| **value >= low && **value <= high)
        .map(|(index, _)| index)
        .collect();
    if indices.is_empty() {
        return Err(CoreError::EmptyWindow {
            axis: name,
            center: (low + high) / 2.0,
        });
    }
    Ok(indices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::array;
    use ta_core::{AxisKind, AxisUnit};

    fn axes() -> (Axis, Axis) {
        (
            Axis::new(
                AxisKind::Wavelength,
                AxisUnit::Nanometer,
                "Wavelength",
                vec![500.0, 510.0, 520.0, 530.0],
            )
            .unwrap(),
            Axis::new(
                AxisKind::Delay,
                AxisUnit::Picosecond,
                "Delay",
                vec![-1.0, 0.0, 1.0],
            )
            .unwrap(),
        )
    }

    fn matrix() -> Array2<f64> {
        array![
            [1.0, 2.0, 3.0],
            [4.0, 5.0, 6.0],
            [7.0, 8.0, 9.0],
            [10.0, 11.0, 12.0]
        ]
    }

    #[test]
    fn an_empty_crop_keeps_everything() {
        let (wavelength, delay) = axes();
        let cropped = crop_matrix(
            &matrix(),
            &wavelength,
            &delay,
            &CropSpec {
                wavelength: None,
                delay: None,
            },
        )
        .unwrap();
        assert_eq!(cropped.data, matrix());
        assert_eq!(cropped.wavelength_indices, vec![0, 1, 2, 3]);
    }

    #[test]
    fn cropping_selects_both_axes_and_keeps_axis_order() {
        let (wavelength, delay) = axes();
        let cropped = crop_matrix(
            &matrix(),
            &wavelength,
            &delay,
            &CropSpec {
                wavelength: Some((505.0, 525.0)),
                delay: Some((0.0, 1.0)),
            },
        )
        .unwrap();
        assert_eq!(cropped.wavelength.values(), &[510.0, 520.0]);
        assert_eq!(cropped.delay.values(), &[0.0, 1.0]);
        assert_eq!(cropped.data, array![[5.0, 6.0], [8.0, 9.0]]);
    }

    #[test]
    fn a_reversed_pair_of_limits_means_the_same_interval() {
        let (wavelength, delay) = axes();
        let cropped = crop_matrix(
            &matrix(),
            &wavelength,
            &delay,
            &CropSpec {
                wavelength: Some((525.0, 505.0)),
                delay: None,
            },
        )
        .unwrap();
        assert_eq!(cropped.wavelength.values(), &[510.0, 520.0]);
    }

    #[test]
    fn a_descending_axis_stays_descending_after_cropping() {
        let wavelength = Axis::new(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            "Wavelength",
            vec![530.0, 520.0, 510.0, 500.0],
        )
        .unwrap();
        let (_, delay) = axes();
        let cropped = crop_matrix(
            &matrix(),
            &wavelength,
            &delay,
            &CropSpec {
                wavelength: Some((505.0, 525.0)),
                delay: None,
            },
        )
        .unwrap();
        assert_eq!(cropped.wavelength.values(), &[520.0, 510.0]);
        assert!(!cropped.wavelength.is_ascending());
    }

    #[test]
    fn a_crop_that_keeps_nothing_is_an_error() {
        let (wavelength, delay) = axes();
        assert!(crop_matrix(
            &matrix(),
            &wavelength,
            &delay,
            &CropSpec {
                wavelength: Some((900.0, 1000.0)),
                delay: None,
            },
        )
        .is_err());
    }

    #[test]
    fn a_shape_that_disagrees_with_its_axes_is_refused() {
        let (wavelength, delay) = axes();
        assert!(crop_matrix(
            &array![[1.0, 2.0]],
            &wavelength,
            &delay,
            &CropSpec {
                wavelength: None,
                delay: None,
            },
        )
        .is_err());
    }
}
