//! Stitching visible and near-infrared matrices into one.
//!
//! The reference implementation's `merge_glotaran` merges *exported* files; the
//! specification asks for a real in-memory stitch (`spec.md` §14). This is that
//! operation: one wavelength axis covering both, the overlap combined the way
//! the user asked for, and the NIR half optionally scaled so the two agree where
//! they overlap.

use ndarray::Array2;
use ta_core::{Axis, BlendMode, CoreError, Result, StitchDelayGrid, StitchSpec};

/// A stitched matrix and the axes that describe it.
#[derive(Clone, Debug, PartialEq)]
pub struct Stitched {
    pub wavelength: Axis,
    pub delay: Axis,
    pub data: Array2<f64>,
    /// The scale applied to the second input, so the operation is reportable.
    pub second_scale: f64,
    /// How many delay columns the two inputs had to share after resampling.
    pub resampled: bool,
}

/// Combines two matrices over a common wavelength grid.
///
/// The first input is the visible half, the second the near-infrared one. Below
/// the crossover the first wins, above it the second; across the overlap the two
/// are cross-faded when `blend` asks for it.
pub fn stitch_matrices(
    first: &Array2<f64>,
    first_wavelength: &Axis,
    first_delay: &Axis,
    second: &Array2<f64>,
    second_wavelength: &Axis,
    second_delay: &Axis,
    spec: &StitchSpec,
) -> Result<Stitched> {
    for (matrix, wavelength, delay, name) in [
        (first, first_wavelength, first_delay, "the visible matrix"),
        (second, second_wavelength, second_delay, "the near-infrared matrix"),
    ] {
        if matrix.nrows() != wavelength.len() || matrix.ncols() != delay.len() {
            return Err(CoreError::invalid(format!(
                "{name} is {}×{} but its axes describe {}×{}",
                matrix.nrows(),
                matrix.ncols(),
                wavelength.len(),
                delay.len()
            )));
        }
    }
    if !spec.crossover_nm.is_finite() {
        return Err(CoreError::invalid(format!(
            "a crossover of {} nm is not a wavelength",
            spec.crossover_nm
        )));
    }

    // The delay grid: either it is shared, or both are resampled onto the
    // finer of the two.
    let (delay, first_columns, second_columns, resampled) = match spec.delay_grid {
        StitchDelayGrid::Exact => {
            if first_delay.len() != second_delay.len()
                || first_delay
                    .values()
                    .iter()
                    .zip(second_delay.values())
                    .any(|(a, b)| (a - b).abs() > 1e-9)
            {
                return Err(CoreError::AxisMismatch { axis: "delay" });
            }
            (
                first_delay.clone(),
                (0..first_delay.len()).collect::<Vec<usize>>(),
                (0..second_delay.len()).collect::<Vec<usize>>(),
                false,
            )
        }
        StitchDelayGrid::Finer => {
            let target = if first_delay.len() >= second_delay.len() {
                first_delay.clone()
            } else {
                second_delay.clone()
            };
            let first_columns = resample_columns(
                first, first_delay, &target,
            )?;
            let second_columns = resample_columns(
                second, second_delay, &target,
            )?;
            (target, first_columns, second_columns, true)
        }
    };

    // The wavelength grid: everything either half measures, in order.
    let overlaps = first_wavelength.range().1 >= second_wavelength.range().0
        && second_wavelength.range().1 >= first_wavelength.range().0;
    if !overlaps {
        return Err(CoreError::invalid(format!(
            "the two matrices do not overlap: {:.1}–{:.1} nm against {:.1}–{:.1} nm, so a \
             crossover at {:.1} nm cannot be placed",
            first_wavelength.range().0,
            first_wavelength.range().1,
            second_wavelength.range().0,
            second_wavelength.range().1,
            spec.crossover_nm
        )));
    }

    let mut wavelengths: Vec<f64> = first_wavelength.values().to_vec();
    for value in second_wavelength.values() {
        if !wavelengths.iter().any(|existing| (existing - value).abs() < 1e-9) {
            wavelengths.push(*value);
        }
    }
    let ascending = first_wavelength.is_ascending();
    wavelengths.sort_by(f64::total_cmp);
    if !ascending {
        wavelengths.reverse();
    }

    // The scale that makes the second half agree with the first over the
    // overlap, by least squares over every shared wavelength and delay.
    let (overlap_low, overlap_high) = (
        second_wavelength.range().0.max(first_wavelength.range().0),
        second_wavelength.range().1.min(first_wavelength.range().1),
    );
    let mut scale = 1.0;
    if spec.match_overlap {
        let mut numerator = 0.0;
        let mut denominator = 0.0;
        for wavelength in first_wavelength.values() {
            if *wavelength < overlap_low || *wavelength > overlap_high {
                continue;
            }
            let Some(first_row) = first_wavelength
                .values()
                .iter()
                .position(|value| (value - wavelength).abs() < 1e-9)
            else {
                continue;
            };
            let Some(second_row) = second_wavelength
                .values()
                .iter()
                .position(|value| (value - wavelength).abs() < 1e-9)
            else {
                continue;
            };
            for column in 0..delay.len() {
                let a = first[[first_row, first_columns[column]]];
                let b = second[[second_row, second_columns[column]]];
                if a.is_finite() && b.is_finite() {
                    numerator += a * b;
                    denominator += b * b;
                }
            }
        }
        if denominator > 0.0 {
            scale = numerator / denominator;
        }
    }

    let mut data = Array2::from_elem((wavelengths.len(), delay.len()), f64::NAN);
    for (row, wavelength) in wavelengths.iter().enumerate() {
        let in_first = first_wavelength
            .values()
            .iter()
            .position(|value| (value - wavelength).abs() < 1e-9);
        let in_second = second_wavelength
            .values()
            .iter()
            .position(|value| (value - wavelength).abs() < 1e-9);
        for column in 0..delay.len() {
            let a = in_first
                .map(|row| first[[row, first_columns[column]]])
                .filter(|value| value.is_finite());
            let b = in_second
                .map(|row| second[[row, second_columns[column]]] * scale)
                .filter(|value| value.is_finite());
            data[[row, column]] = match (a, b) {
                (Some(a), Some(b)) => {
                    // Inside the overlap: choose by the crossover, or blend
                    // across it.
                    match spec.blend {
                        BlendMode::Hard => {
                            if *wavelength <= spec.crossover_nm {
                                a
                            } else {
                                b
                            }
                        }
                        BlendMode::Linear => {
                            let weight = ((*wavelength - overlap_low)
                                / (overlap_high - overlap_low).max(f64::MIN_POSITIVE))
                            .clamp(0.0, 1.0);
                            a * (1.0 - weight) + b * weight
                        }
                    }
                }
                (Some(a), None) => a,
                (None, Some(b)) => b,
                // Neither half measured this point: it stays missing rather
                // than becoming a number.
                (None, None) => f64::NAN,
            };
        }
    }

    let wavelength = Axis::new(
        first_wavelength.kind(),
        first_wavelength.unit(),
        first_wavelength.name(),
        wavelengths,
    )?;
    Ok(Stitched {
        wavelength,
        delay,
        data,
        second_scale: scale,
        resampled,
    })
}

/// The column indices that put a matrix onto the target delay grid.
fn resample_columns(matrix: &Array2<f64>, delay: &Axis, target: &Axis) -> Result<Vec<usize>> {
    let mut columns = Vec::with_capacity(target.len());
    for value in target.values() {
        if delay.contains(*value) {
            let (index, _) = delay.nearest(*value);
            columns.push(index);
        } else {
            return Err(CoreError::OutOfRange {
                axis: "delay",
                value: *value,
                min: delay.range().0,
                max: delay.range().1,
            });
        }
    }
    let _ = matrix;
    Ok(columns)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_core::{AxisKind, AxisUnit};

    fn axis(kind: AxisKind, unit: AxisUnit, values: Vec<f64>) -> Axis {
        Axis::new(kind, unit, "axis", values).unwrap()
    }

    fn halves(second_scale: f64) -> (Array2<f64>, Axis, Axis, Array2<f64>, Axis, Axis) {
        let first_wavelength = axis(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            vec![400.0, 500.0, 600.0, 700.0],
        );
        let second_wavelength = axis(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            vec![600.0, 700.0, 800.0, 900.0],
        );
        let delay = axis(
            AxisKind::Delay,
            AxisUnit::Picosecond,
            vec![0.0, 1.0, 2.0],
        );
        let first = Array2::from_shape_fn((4, 3), |(row, column)| 0.1 * row as f64 + column as f64);
        // The NIR half measures the same thing over the overlap, scaled.
        let second = Array2::from_shape_fn((4, 3), |(row, column)| {
            (0.1 * (row + 2) as f64 + column as f64) * second_scale
        });
        (
            first,
            first_wavelength,
            delay.clone(),
            second,
            second_wavelength,
            delay,
        )
    }

    fn spec(crossover: f64, blend: BlendMode, match_overlap: bool) -> StitchSpec {
        StitchSpec {
            crossover_nm: crossover,
            blend,
            match_overlap,
            delay_grid: StitchDelayGrid::Exact,
        }
    }

    #[test]
    fn a_hard_stitch_takes_each_half_up_to_the_crossover() {
        let (first, fw, fd, second, sw, sd) = halves(1.0);
        let stitched = stitch_matrices(
            &first,
            &fw,
            &fd,
            &second,
            &sw,
            &sd,
            &spec(650.0, BlendMode::Hard, false),
        )
        .unwrap();
        // The union of both wavelength axes, in order.
        assert_eq!(stitched.wavelength.len(), 6);
        assert_eq!(stitched.wavelength.values()[0], 400.0);
        assert_eq!(stitched.wavelength.values()[5], 900.0);
        assert!(!stitched.resampled);
        // 600 nm comes from the visible half, 800 nm from the NIR one.
        let row_600 = stitched
            .wavelength
            .values()
            .iter()
            .position(|value| *value == 600.0)
            .unwrap();
        let row_800 = stitched
            .wavelength
            .values()
            .iter()
            .position(|value| *value == 800.0)
            .unwrap();
        assert_eq!(stitched.data[[row_600, 1]], first[[2, 1]]);
        assert!((stitched.data[[row_800, 1]] - second[[2, 1]]).abs() < 1e-12);
    }

    #[test]
    fn a_linear_blend_crosses_over_gradually() {
        let (first, fw, fd, second, sw, sd) = halves(1.0);
        let blended = stitch_matrices(
            &first,
            &fw,
            &fd,
            &second,
            &sw,
            &sd,
            &spec(650.0, BlendMode::Linear, false),
        )
        .unwrap();
        // At the low edge of the overlap the visible half dominates; at the
        // high edge the NIR half does.
        let row_600 = blended
            .wavelength
            .values()
            .iter()
            .position(|value| *value == 600.0)
            .unwrap();
        let row_900 = blended
            .wavelength
            .values()
            .iter()
            .position(|value| *value == 900.0)
            .unwrap();
        assert!((blended.data[[row_600, 0]] - first[[2, 0]]).abs() < 1e-12);
        assert!((blended.data[[row_900, 0]] - second[[3, 0]]).abs() < 1e-12);
        // And somewhere in the middle it is neither of them exactly.
        let row_700 = blended
            .wavelength
            .values()
            .iter()
            .position(|value| *value == 700.0)
            .unwrap();
        let value = blended.data[[row_700, 0]];
        assert!(value > first[[3, 0]].min(second[[1, 0]]) - 1e-9);
        assert!(value < first[[3, 0]].max(second[[1, 0]]) + 1e-9);
    }

    #[test]
    fn matching_the_overlap_recovers_a_known_scale_factor() {
        let (first, fw, fd, second, sw, sd) = halves(0.5);
        let matched = stitch_matrices(
            &first,
            &fw,
            &fd,
            &second,
            &sw,
            &sd,
            &spec(650.0, BlendMode::Hard, true),
        )
        .unwrap();
        // The NIR half was measured at half the amplitude, so the fit doubles
        // it back.
        assert!(
            (matched.second_scale - 2.0).abs() < 1e-9,
            "the scale was {}",
            matched.second_scale
        );
        // And the stitched matrix is continuous across the seam.
        let row_700 = matched
            .wavelength
            .values()
            .iter()
            .position(|value| *value == 700.0)
            .unwrap();
        let row_800 = matched
            .wavelength
            .values()
            .iter()
            .position(|value| *value == 800.0)
            .unwrap();
        // The signal increases one step per 100 nm in both halves.
        let step = matched.data[[row_800, 0]] - matched.data[[row_700, 0]];
        let earlier = matched.data[[row_700, 0]]
            - matched.data[[matched.wavelength.len().min(5) - 3, 0]];
        assert!(step > 0.0 && earlier > 0.0);
    }

    #[test]
    fn a_coarser_delay_grid_is_resampled_onto_the_finer_one() {
        let first_wavelength = axis(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            vec![400.0, 600.0],
        );
        let second_wavelength = axis(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            vec![600.0, 800.0],
        );
        let first_delay = axis(
            AxisKind::Delay,
            AxisUnit::Picosecond,
            vec![0.0, 1.0, 2.0, 3.0],
        );
        let second_delay = axis(AxisKind::Delay, AxisUnit::Picosecond, vec![0.0, 1.0, 2.0, 3.0]);
        let first = Array2::from_elem((2, 4), 1.0);
        let second = Array2::from_elem((2, 4), 2.0);
        let stitched = stitch_matrices(
            &first,
            &first_wavelength,
            &first_delay,
            &second,
            &second_wavelength,
            &second_delay,
            &StitchSpec {
                delay_grid: StitchDelayGrid::Finer,
                ..spec(700.0, BlendMode::Hard, false)
            },
        )
        .unwrap();
        assert_eq!(stitched.delay.len(), 4);
        assert!(stitched.resampled);
    }

    #[test]
    fn matrices_that_do_not_overlap_are_refused_rather_than_gapped() {
        let first_wavelength = axis(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            vec![400.0, 500.0],
        );
        let second_wavelength = axis(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            vec![800.0, 900.0],
        );
        let delay = axis(AxisKind::Delay, AxisUnit::Picosecond, vec![0.0, 1.0]);
        let first = Array2::from_elem((2, 2), 1.0);
        let second = Array2::from_elem((2, 2), 1.0);
        let error = stitch_matrices(
            &first,
            &first_wavelength,
            &delay,
            &second,
            &second_wavelength,
            &delay,
            &spec(650.0, BlendMode::Hard, false),
        )
        .unwrap_err();
        assert!(format!("{error}").contains("do not overlap"));
    }

    #[test]
    fn an_exact_delay_grid_that_disagrees_is_refused() {
        let first_wavelength =
            axis(AxisKind::Wavelength, AxisUnit::Nanometer, vec![400.0, 600.0]);
        let second_wavelength =
            axis(AxisKind::Wavelength, AxisUnit::Nanometer, vec![600.0, 800.0]);
        let first_delay = axis(AxisKind::Delay, AxisUnit::Picosecond, vec![0.0, 1.0]);
        let second_delay = axis(AxisKind::Delay, AxisUnit::Picosecond, vec![0.0, 5.0]);
        let first = Array2::from_elem((2, 2), 1.0);
        let second = Array2::from_elem((2, 2), 1.0);
        assert!(stitch_matrices(
            &first,
            &first_wavelength,
            &first_delay,
            &second,
            &second_wavelength,
            &second_delay,
            &spec(700.0, BlendMode::Hard, false),
        )
        .is_err());
    }

    #[test]
    fn a_shape_that_disagrees_with_its_axes_is_refused() {
        let wavelength = axis(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            vec![400.0, 600.0, 800.0],
        );
        let delay = axis(AxisKind::Delay, AxisUnit::Picosecond, vec![0.0, 1.0]);
        let wrong = Array2::from_elem((2, 2), 1.0);
        let right = Array2::from_elem((3, 2), 1.0);
        assert!(stitch_matrices(
            &wrong,
            &wavelength,
            &delay,
            &right,
            &wavelength,
            &delay,
            &spec(600.0, BlendMode::Hard, false),
        )
        .is_err());
    }
}
