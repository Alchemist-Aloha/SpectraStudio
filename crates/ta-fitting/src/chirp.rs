//! Automatic time-zero (chirp) estimation.
//!
//! This is the reference implementation's `fit_correlation` workflow: fit the
//! kinetics at a sample of wavelengths, keep the ones that converged well, and
//! fit a smooth curve through the time zeros they found.
//!
//! Two things differ from the reference, and both are named in
//! `docs/numerics.md`:
//!
//! * it fits a polynomial in wavelength. The reference's `polynomial_func` is
//!   `a / (1e-9 + λ²) + c` — an inverse-square in the wavelength, with its
//!   second parameter dead — which is not the chirp shape and is one of the
//!   auxiliary functions `spec.md` §2.1 asks to investigate;
//! * it reports every point it rejected and why, rather than silently keeping
//!   only the ones that passed a fixed threshold.

use crate::fit::{fit_trace, FitRequest};
use crate::solver::{cholesky_solve, SolverOptions};
use crate::{FitError, Result};
use ndarray::Array2;
use ta_core::{
    Axis, ControlPoint, CorrectionCurve, FitModelKind, SignalUnit, Trace, TraceSelection,
    Weighting,
};

/// How the automatic estimate is made.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoTimeZeroOptions {
    /// Exponentials in the fitting model.
    pub components: u8,
    /// Fit every `n`-th wavelength, as the reference does with 20.
    pub wavelength_stride: usize,
    /// Skip wavelengths whose signal at the reference delay is below this
    /// fraction of the strongest one.
    pub signal_fraction: f64,
    /// The delay the signal strength is judged at, in ps.
    pub reference_delay: f64,
    /// Reject a wavelength whose fit leaves a sum of squares above this.
    pub max_sse: f64,
    /// Control points to put into the correction curve.
    pub curve_points: usize,
    /// Degree of the curve fitted through the time zeros: 1 for a linear
    /// chirp, 2 for the usual quadratic.
    pub degree: usize,
}

impl Default for AutoTimeZeroOptions {
    fn default() -> Self {
        Self {
            components: 2,
            wavelength_stride: 20,
            signal_fraction: 0.1,
            reference_delay: 1.0,
            max_sse: 0.15,
            curve_points: 11,
            degree: 2,
        }
    }
}

/// One wavelength the estimator looked at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoTimeZeroPoint {
    pub wavelength: f64,
    pub time_zero: f64,
    pub sse: f64,
    pub accepted: bool,
}

/// What the estimator found.
#[derive(Clone, Debug, PartialEq)]
pub struct AutoTimeZero {
    pub curve: CorrectionCurve,
    pub points: Vec<AutoTimeZeroPoint>,
    /// The polynomial's coefficients in the centred variable, highest degree
    /// last.
    pub coefficients: Vec<f64>,
    pub accepted: usize,
    pub rejected: usize,
}

/// Estimates a correction curve from the matrix itself.
pub fn estimate_time_zero_curve(
    wavelength: &Axis,
    delay: &Axis,
    matrix: &Array2<f64>,
    options: &AutoTimeZeroOptions,
) -> Result<AutoTimeZero> {
    let (rows, columns) = matrix.dim();
    if rows != wavelength.len() || columns != delay.len() {
        return Err(FitError::Invalid(format!(
            "the matrix is {rows}×{columns} but its axes describe {}×{}",
            wavelength.len(),
            delay.len()
        )));
    }
    if wavelength.len() < 3 {
        return Err(FitError::Invalid(
            "an automatic estimate needs at least three wavelengths".to_string(),
        ));
    }
    let stride = options.wavelength_stride.max(1);
    let (reference_index, _) = delay.nearest(options.reference_delay);

    // How strong the signal is at the reference delay, so the weak wavelengths
    // can be left alone.
    let mut strongest = 0.0f64;
    let mut strongest_row = 0usize;
    for row in 0..rows {
        let value = matrix[[row, reference_index]].abs();
        if value.is_finite() && value > strongest {
            strongest = value;
            strongest_row = row;
        }
    }
    if strongest <= 0.0 {
        return Err(FitError::Invalid(format!(
            "no signal at {:.3} ps, so there is no time zero to find",
            options.reference_delay
        )));
    }
    let threshold = strongest * options.signal_fraction;

    // The fit starts from the strongest wavelength and carries its parameters to
    // the next one, as the reference does.
    let mut running = ta_core::legacy_parameter_template(options.components);
    let mut points: Vec<AutoTimeZeroPoint> = Vec::new();

    let mut order: Vec<usize> = (0..rows).step_by(stride).collect();
    if !order.contains(&strongest_row) {
        order.push(strongest_row);
        order.sort_unstable();
    }
    for row in order {
        let strength = matrix[[row, reference_index]].abs();
        if !strength.is_finite() || strength < threshold {
            points.push(AutoTimeZeroPoint {
                wavelength: wavelength.values()[row],
                time_zero: f64::NAN,
                sse: f64::NAN,
                accepted: false,
            });
            continue;
        }
        let trace = row_trace(matrix, wavelength, delay, row)?;
        let request = FitRequest {
            dataset: ta_core::DatasetId::new(),
            version: trace.version,
            selection: TraceSelection::new(
                ta_core::DatasetId::new(),
                trace.version,
                ta_core::AxisRequest::nearest(trace.sampled.coordinate),
            ),
            model: FitModelKind::LegacyGaussianCdfIrf,
            components: options.components.clamp(1, 4),
            parameters: running.clone(),
            fit_range: None,
            excluded: Vec::new(),
            weighting: Weighting::Uniform,
            options: SolverOptions::default(),
        };
        match fit_trace(&trace, &request) {
            Ok(result) => {
                let time_zero = result.parameter_value("time_zero").unwrap_or(f64::NAN);
                let accepted = result.stats.converged && result.stats.sse < options.max_sse;
                if accepted {
                    running = result.params.clone();
                }
                points.push(AutoTimeZeroPoint {
                    wavelength: wavelength.values()[row],
                    time_zero,
                    sse: result.stats.sse,
                    accepted,
                });
            }
            Err(error) => {
                points.push(AutoTimeZeroPoint {
                    wavelength: wavelength.values()[row],
                    time_zero: f64::NAN,
                    sse: f64::NAN,
                    accepted: false,
                });
                let _ = error;
            }
        }
    }

    let accepted: Vec<AutoTimeZeroPoint> = points
        .iter()
        .copied()
        .filter(|point| point.accepted && point.time_zero.is_finite())
        .collect();
    if accepted.len() < options.degree + 1 {
        return Err(FitError::Invalid(format!(
            "only {} wavelengths gave a usable time zero, which is not enough for a \
             degree-{} curve",
            accepted.len(),
            options.degree
        )));
    }

    let coefficients = weighted_polynomial(&accepted, options.degree)?;
    let (low, high) = (
        accepted.first().map(|point| point.wavelength).unwrap_or(0.0),
        accepted.last().map(|point| point.wavelength).unwrap_or(0.0),
    );
    // The fit is done in a centred variable, so the samples use it too.
    let centre = (low + high) / 2.0;
    let scale = ((high - low) / 2.0).max(1e-9);
    let count = options.curve_points.max(2);
    let curve = CorrectionCurve::new(
        (0..count)
            .map(|index| {
                let fraction = index as f64 / (count - 1) as f64;
                let sample = low + (high - low) * fraction;
                let centred = (sample - centre) / scale;
                let mut value = 0.0;
                for (power, coefficient) in coefficients.iter().enumerate() {
                    value += coefficient * centred.powi(power as i32);
                }
                ControlPoint {
                    wavelength: sample,
                    time_zero: value,
                }
            })
            .collect(),
    );

    let rejected = points.len() - accepted.len();
    Ok(AutoTimeZero {
        curve,
        points,
        coefficients,
        accepted: accepted.len(),
        rejected,
    })
}

/// One matrix row as a trace, so the ordinary fitter can be used on it.
fn row_trace(
    matrix: &Array2<f64>,
    wavelength: &Axis,
    delay: &Axis,
    row: usize,
) -> Result<Trace> {
    let signal: Vec<f64> = (0..matrix.ncols()).map(|column| matrix[[row, column]]).collect();
    Ok(Trace {
        label: format!("{:.4} nm", wavelength.values()[row]),
        delay: delay.values().to_vec(),
        signal,
        unit: SignalUnit::DeltaOpticalDensity,
        version: ta_core::VersionId::new(),
        sampled: ta_core::Sample {
            coordinate: wavelength.values()[row],
            indices: vec![row],
            weights: vec![1.0],
            coordinates: vec![wavelength.values()[row]],
            mode: ta_core::SampleMode::Nearest,
        },
    })
}

/// A weighted least-squares polynomial in a centred wavelength.
///
/// The weight is the reciprocal of each point's residual sum of squares, which
/// is what the reference passes to its curve fit: a wavelength whose own fit was
/// poor pulls the curve less.
fn weighted_polynomial(points: &[AutoTimeZeroPoint], degree: usize) -> Result<Vec<f64>> {
    let low = points.first().map(|point| point.wavelength).unwrap_or(0.0);
    let high = points.last().map(|point| point.wavelength).unwrap_or(0.0);
    let centre = (low + high) / 2.0;
    let scale = ((high - low) / 2.0).max(1e-9);
    let terms = degree + 1;

    let mut normal = vec![vec![0.0f64; terms]; terms];
    let mut rhs = vec![0.0f64; terms];
    for point in points {
        let x = (point.wavelength - centre) / scale;
        let weight = if point.sse > 0.0 && point.sse.is_finite() {
            1.0 / point.sse
        } else {
            1.0
        };
        let basis: Vec<f64> = (0..terms).map(|power| x.powi(power as i32)).collect();
        for a in 0..terms {
            rhs[a] += weight * basis[a] * point.time_zero;
            for b in 0..terms {
                normal[a][b] += weight * basis[a] * basis[b];
            }
        }
    }
    cholesky_solve(&normal, &rhs)
        .ok_or_else(|| FitError::Solver("the time-zero points could not be fitted".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{evaluate, Component, Model, ModelParams};
    use ta_core::{AxisKind, AxisUnit};

    /// A matrix whose every wavelength has a known time zero.
    fn synthetic(chirp: impl Fn(f64) -> f64, noise: f64) -> (Axis, Axis, Array2<f64>) {
        let wavelengths: Vec<f64> = (0..60).map(|i| 450.0 + i as f64 * 5.0).collect();
        let delays: Vec<f64> = (0..400).map(|i| -1.0 + i as f64 * 0.02).collect();
        let wavelength = Axis::new(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            "Wavelength",
            wavelengths.clone(),
        )
        .unwrap();
        let delay = Axis::new(
            AxisKind::Delay,
            AxisUnit::Picosecond,
            "Delay",
            delays.clone(),
        )
        .unwrap();
        let mut matrix = Array2::zeros((wavelengths.len(), delays.len()));
        for (row, value) in wavelengths.iter().enumerate() {
            let params = ModelParams {
                irf_fwhm: 0.12,
                amplitude_scale: 1.0,
                components: vec![
                    Component {
                        amplitude: -0.4,
                        lifetime: 1.5,
                    },
                    Component {
                        amplitude: 0.2,
                        lifetime: 25.0,
                    },
                    Component {
                        amplitude: 0.0,
                        lifetime: 0.0,
                    },
                    Component {
                        amplitude: 0.0,
                        lifetime: 0.0,
                    },
                ],
                time_zero: chirp(*value),
                pre_zero: 0.0,
                long_term: 0.0,
            };
            let curve = evaluate(Model::LegacyGaussianCdfIrf, &params, &delays);
            for (column, signal) in curve.iter().enumerate() {
                let wobble = if noise > 0.0 {
                    noise * (((row * 31 + column) as f64 * 12.9898).sin() * 43758.5453).fract()
                } else {
                    0.0
                };
                matrix[[row, column]] = signal + wobble;
            }
        }
        (wavelength, delay, matrix)
    }

    #[test]
    fn a_linear_chirp_is_recovered() {
        // 0 ps at 450 nm, rising by 2 fs per nm — 0.5 ps across this range.
        let (wavelength, delay, matrix) = synthetic(|nm| 0.002 * (nm - 450.0), 0.0);
        let options = AutoTimeZeroOptions {
            components: 2,
            wavelength_stride: 5,
            curve_points: 5,
            ..AutoTimeZeroOptions::default()
        };
        let outcome = estimate_time_zero_curve(&wavelength, &delay, &matrix, &options).unwrap();
        assert!(outcome.accepted >= 5, "accepted {}", outcome.accepted);
        // The curve follows the truth at both ends.
        let sorted = outcome.curve.sorted();
        let first = sorted.first().unwrap();
        let last = sorted.last().unwrap();
        assert!(
            (first.time_zero - 0.002 * (first.wavelength - 450.0)).abs() < 0.03,
            "at {} nm the estimate was {} against {}",
            first.wavelength,
            first.time_zero,
            0.002 * (first.wavelength - 450.0)
        );
        assert!(
            (last.time_zero - 0.002 * (last.wavelength - 450.0)).abs() < 0.03,
            "at {} nm the estimate was {} against {}",
            last.wavelength,
            last.time_zero,
            0.002 * (last.wavelength - 450.0)
        );
    }

    #[test]
    fn a_flat_signal_gives_a_flat_curve() {
        let (wavelength, delay, matrix) = synthetic(|_| 0.05, 0.0);
        let options = AutoTimeZeroOptions {
            wavelength_stride: 10,
            ..AutoTimeZeroOptions::default()
        };
        let outcome = estimate_time_zero_curve(&wavelength, &delay, &matrix, &options).unwrap();
        let spread = outcome
            .curve
            .points
            .iter()
            .map(|point| point.time_zero)
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), value| {
                (low.min(value), high.max(value))
            });
        assert!(
            spread.1 - spread.0 < 0.05,
            "a flat matrix produced a curve spanning {:?}",
            spread
        );
    }

    #[test]
    fn a_wavelength_with_no_signal_is_skipped_rather_than_fitted() {
        let (wavelength, delay, mut matrix) = synthetic(|nm| 0.001 * (nm - 450.0), 0.0);
        // Take every tenth row down to the noise floor.
        for row in (0..matrix.nrows()).step_by(10) {
            for column in 0..matrix.ncols() {
                matrix[[row, column]] = 1e-7;
            }
        }
        let options = AutoTimeZeroOptions {
            components: 2,
            wavelength_stride: 1,
            curve_points: 5,
            ..AutoTimeZeroOptions::default()
        };
        let outcome = estimate_time_zero_curve(&wavelength, &delay, &matrix, &options).unwrap();
        assert!(outcome.rejected >= 6, "rejected {}", outcome.rejected);
        // The skipped rows are reported, not silently dropped.
        let skipped = outcome
            .points
            .iter()
            .filter(|point| !point.accepted)
            .count();
        assert_eq!(skipped, outcome.rejected);
    }

    #[test]
    fn too_few_usable_wavelengths_is_an_error_not_a_curve_from_one_point() {
        let (wavelength, delay, mut matrix) = synthetic(|_| 0.0, 0.0);
        // Every row past the first two is flat zero, so only the strong rows
        // would fit; with a stride that skips them there is nothing left.
        for row in 2..matrix.nrows() {
            for column in 0..matrix.ncols() {
                matrix[[row, column]] = 0.0;
            }
        }
        let options = AutoTimeZeroOptions {
            components: 2,
            wavelength_stride: 20,
            degree: 2,
            ..AutoTimeZeroOptions::default()
        };
        let result = estimate_time_zero_curve(&wavelength, &delay, &matrix, &options);
        assert!(result.is_err());
    }

    #[test]
    fn a_matrix_whose_shape_disagrees_with_its_axes_is_refused() {
        let (wavelength, delay, matrix) = synthetic(|_| 0.0, 0.0);
        let smaller = Array2::zeros((2, 2));
        assert!(estimate_time_zero_curve(&wavelength, &delay, &smaller, &AutoTimeZeroOptions::default())
            .is_err());
        let _ = matrix;
    }
}
