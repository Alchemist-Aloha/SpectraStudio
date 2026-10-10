//! Averaging parallel experiments, and subtracting one matrix from another.

use ndarray::Array2;
use ta_core::{CoreError, Result};

/// Element-wise sum of several matrices, optionally divided by their count.
///
/// Every input must have the same shape: averaging matrices that describe
/// different wavelength or delay points would silently line up the wrong
/// numbers, so it is refused instead.
pub fn average_matrices(matrices: &[&Array2<f64>], mean: bool) -> Result<Array2<f64>> {
    let mut inputs = matrices.iter();
    let Some(first) = inputs.next() else {
        return Err(CoreError::invalid(
            "averaging needs at least one matrix",
        ));
    };
    if matrices.len() < 2 {
        return Err(CoreError::NotEnoughInputs { got: matrices.len() });
    }
    let shape = first.dim();
    for matrix in inputs {
        if matrix.dim() != shape {
            return Err(CoreError::InputShapeMismatch {
                first: shape,
                second: matrix.dim(),
            });
        }
    }

    let mut total = (*first).clone();
    for matrix in matrices.iter().skip(1) {
        total += *matrix;
    }
    if mean {
        let divisor = matrices.len() as f64;
        total.mapv_inplace(|value| value / divisor);
    }
    Ok(total)
}

/// `left - right * factor`, refusing mismatched shapes.
pub fn subtract_matrices(
    left: &Array2<f64>,
    right: &Array2<f64>,
    factor: f64,
) -> Result<Array2<f64>> {
    if left.dim() != right.dim() {
        return Err(CoreError::InputShapeMismatch {
            first: left.dim(),
            second: right.dim(),
        });
    }
    if !factor.is_finite() {
        return Err(CoreError::invalid(format!(
            "a subtraction factor of {factor} is not a number"
        )));
    }
    Ok(left - &(right * factor))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::array;

    #[test]
    fn averaging_divides_by_the_number_of_experiments() {
        let a = array![[1.0, 2.0], [3.0, 4.0]];
        let b = array![[3.0, 4.0], [5.0, 6.0]];
        let mean = average_matrices(&[&a, &b], true).unwrap();
        assert_eq!(mean, array![[2.0, 3.0], [4.0, 5.0]]);
        let sum = average_matrices(&[&a, &b], false).unwrap();
        assert_eq!(sum, array![[4.0, 6.0], [8.0, 10.0]]);
    }

    #[test]
    fn averaging_one_matrix_is_refused_rather_than_returned_unchanged() {
        let a = array![[1.0]];
        assert!(matches!(
            average_matrices(&[&a], true),
            Err(CoreError::NotEnoughInputs { got: 1 })
        ));
        assert!(average_matrices(&[], true).is_err());
    }

    #[test]
    fn averaging_matrices_of_different_shapes_is_refused() {
        let a = array![[1.0, 2.0]];
        let b = array![[1.0], [2.0]];
        assert!(matches!(
            average_matrices(&[&a, &b], true),
            Err(CoreError::InputShapeMismatch { .. })
        ));
    }

    #[test]
    fn a_missing_sample_in_one_experiment_makes_the_average_missing() {
        let a = array![[1.0, 2.0]];
        let b = array![[3.0, f64::NAN]];
        let mean = average_matrices(&[&a, &b], true).unwrap();
        assert_eq!(mean[[0, 0]], 2.0);
        // Not silently treated as 1.5: a missing measurement is not a zero.
        assert!(mean[[0, 1]].is_nan());
    }

    #[test]
    fn subtraction_applies_the_factor_and_checks_the_shape() {
        let sample = array![[1.0, 2.0]];
        let reference = array![[0.5, 0.5]];
        assert_eq!(
            subtract_matrices(&sample, &reference, 2.0).unwrap(),
            array![[0.0, 1.0]]
        );
        let wrong = array![[1.0], [2.0]];
        assert!(subtract_matrices(&sample, &wrong, 1.0).is_err());
        assert!(subtract_matrices(&sample, &reference, f64::NAN).is_err());
    }
}
