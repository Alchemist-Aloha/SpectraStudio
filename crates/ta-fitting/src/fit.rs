//! Turning a trace and a request into a fit result.
//!
//! The result is `ta_core::FitResult`, the record a dataset keeps, so what the
//! workbench shows and what a project saves are the same object.

use crate::model::{self, ModelParams};
use crate::solver::{least_squares, standard_errors, ParameterSpec, SolverOptions};
use crate::{FitError, Result};
use ta_core::{
    FitModelKind, FitParameter, FitResult, FitStats, ResidualData, Trace, TraceSelection,
    Weighting,
};

/// Everything a fit needs besides the data.
#[derive(Clone, Debug, PartialEq)]
pub struct FitRequest {
    pub dataset: ta_core::DatasetId,
    pub version: ta_core::VersionId,
    pub selection: TraceSelection,
    pub model: FitModelKind,
    pub components: u8,
    /// Starting values, bounds and fixed flags.
    pub parameters: Vec<FitParameter>,
    /// The delay interval to fit, or the whole trace.
    pub fit_range: Option<(f64, f64)>,
    /// Delay intervals to leave out.
    pub excluded: Vec<(f64, f64)>,
    pub weighting: Weighting,
    pub options: SolverOptions,
}

impl FitRequest {
    /// A request that starts from the reference implementation's defaults.
    pub fn new(
        dataset: ta_core::DatasetId,
        version: ta_core::VersionId,
        selection: TraceSelection,
        model: FitModelKind,
        components: u8,
    ) -> Self {
        Self {
            dataset,
            version,
            selection,
            model,
            components: components.clamp(1, 4),
            parameters: ta_core::legacy_parameter_template(components),
            fit_range: None,
            excluded: Vec::new(),
            weighting: Weighting::Uniform,
            options: SolverOptions::default(),
        }
    }
}

/// The model's curve over a set of delays, for the overlay.
pub fn model_curve(
    model: FitModelKind,
    parameters: &[FitParameter],
    delays: &[f64],
) -> Result<Vec<f64>> {
    let params = ModelParams::from_parameters(parameters)?;
    Ok(model::evaluate(model.into(), &params, delays))
}

/// Which points a fit uses, and with what weight.
struct Selection {
    delays: Vec<f64>,
    values: Vec<f64>,
    weights: Vec<f64>,
}

fn select(trace: &Trace, request: &FitRequest) -> Result<Selection> {
    let mut selection = Selection {
        delays: Vec::new(),
        values: Vec::new(),
        weights: Vec::new(),
    };
    let mut used = 0usize;
    for (delay, value) in trace.delay.iter().zip(&trace.signal) {
        if !delay.is_finite() || !value.is_finite() {
            continue;
        }
        if let Some((low, high)) = request.fit_range {
            if *delay < low || *delay > high {
                continue;
            }
        }
        if request
            .excluded
            .iter()
            .any(|(low, high)| *delay >= *low && *delay <= *high)
        {
            continue;
        }
        let weight = if let Weighting::Explicit(weights) = &request.weighting {
            match weights.get(used) {
                Some(weight) if *weight > 0.0 && weight.is_finite() => *weight,
                Some(_) => {
                    return Err(FitError::Invalid(
                        "a weight is zero or not a number".to_string(),
                    ))
                }
                None => {
                    return Err(FitError::Invalid(format!(
                        "{} weights were given for {} fitted points",
                        weights.len(),
                        used + 1
                    )))
                }
            }
        } else if matches!(request.weighting, Weighting::Poisson) {
            // Counting-like data: the uncertainty is the square root of the
            // signal, so the weight is its inverse.
            let magnitude = value.abs();
            if magnitude <= 0.0 {
                continue;
            }
            1.0 / magnitude.sqrt()
        } else {
            // Uniform weighting, and the exclusions variant, whose intervals
            // were already filtered out above.
            1.0
        };
        selection.delays.push(*delay);
        selection.values.push(*value);
        selection.weights.push(weight);
        used += 1;
    }
    if selection.delays.len() < 2 {
        return Err(FitError::Invalid(
            "a fit needs at least two finite points in the range".to_string(),
        ));
    }
    Ok(selection)
}

fn specs_for(parameters: &[FitParameter]) -> Vec<ParameterSpec> {
    parameters
        .iter()
        .map(|parameter| {
            let spec = ParameterSpec {
                start: parameter.value,
                lower: parameter.min.unwrap_or(f64::NEG_INFINITY),
                upper: parameter.max.unwrap_or(f64::INFINITY),
                fixed: parameter.fixed,
            };
            spec
        })
        .collect()
}

/// Fits one trace.
pub fn fit_trace(trace: &Trace, request: &FitRequest) -> Result<FitResult> {
    let selection = select(trace, request)?;
    let specs = specs_for(&request.parameters);
    if specs.is_empty() {
        return Err(FitError::Invalid("there are no parameters".to_string()));
    }
    let model = request.model.into();

    let solution = least_squares(&specs, &request.options, |values| {
        let Ok(params) = ModelParams::from_vector(values) else {
            return vec![f64::NAN; selection.delays.len()];
        };
        let curve = model::evaluate(model, &params, &selection.delays);
        curve
            .iter()
            .zip(&selection.values)
            .zip(&selection.weights)
            .map(|((model_value, measured), weight)| (model_value - measured) * weight)
            .collect()
    })?;

    // The parameters as a record, with the uncertainty the solver could
    // estimate — and none where it could not.
    let errors = standard_errors(&solution, &specs);
    let params = ModelParams::from_vector(&solution.parameters)?;
    let curve_params = params.clone();
    let mut parameters = params.into_parameters(&request.parameters);
    for (index, parameter) in parameters.iter_mut().enumerate() {
        parameter.value = solution.parameters[index];
        // A parameter sitting on a bound has no meaningful uncertainty: the
        // solver did not have room to estimate one, and reporting the large
        // number it would produce would be an invention.
        parameter.stderr = if parameter.at_bound() { None } else { errors[index] };
    }

    let fitted_curve = model::evaluate(model, &curve_params, &selection.delays);
    let residuals: Vec<f64> = fitted_curve
        .iter()
        .zip(&selection.values)
        .zip(&selection.weights)
        .map(|((model_value, measured), weight)| (model_value - measured) * weight)
        .collect();

    let degrees = selection.delays.len().saturating_sub(
        specs.iter().filter(|spec| !spec.fixed).count(),
    );
    let weighted = !matches!(request.weighting, Weighting::Uniform);
    let sse: f64 = residuals.iter().map(|value| value * value).sum();
    let mean = selection.values.iter().sum::<f64>() / selection.values.len() as f64;
    let total: f64 = selection
        .values
        .iter()
        .map(|value| (value - mean) * (value - mean))
        .sum();

    let stats = FitStats {
        sse,
        chi_square: weighted.then_some(sse),
        reduced_chi_square: (degrees > 0).then(|| sse / degrees as f64),
        r_squared: (total > 0.0).then(|| 1.0 - sse / total),
        dof: degrees,
        iterations: solution.iterations,
        converged: solution.converged,
        message: solution.message.clone(),
        covariance: solution.covariance.clone(),
    };

    Ok(FitResult {
        id: ta_core::FitId::new(),
        dataset: request.dataset,
        version: request.version,
        selection: request.selection.clone(),
        model: request.model,
        components: request.components,
        params: parameters,
        stats,
        residuals: Some(ResidualData {
            x: selection.delays.clone(),
            y: residuals,
        }),
        fit_range: request.fit_range,
        excluded: request.excluded.clone(),
        weighting: request.weighting.clone(),
        created_at: ta_core::dataset::now_seconds(),
    })
}

/// The time zero a fit found, with the quality that produced it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeZeroEstimate {
    pub time_zero: f64,
    /// The sum of squared residuals of the fit, used to weight the curve.
    pub sse: f64,
    pub converged: bool,
}

/// Fits a trace to estimate its time zero, which is what the automatic
/// chirp correction runs at each wavelength.
///
/// The reference implementation fits with a general-purpose optimiser and keeps
/// a result when its chi-square is below 0.15; here the same shape is followed
/// with this crate's solver, and the caller decides what quality to keep.
pub fn estimate_time_zero(
    trace: &Trace,
    components: u8,
    options: SolverOptions,
) -> Result<TimeZeroEstimate> {
    let request = FitRequest {
        dataset: ta_core::DatasetId::new(),
        version: trace.version,
        selection: TraceSelection::new(
            ta_core::DatasetId::new(),
            trace.version,
            ta_core::AxisRequest::nearest(trace.sampled.coordinate),
        ),
        model: FitModelKind::LegacyGaussianCdfIrf,
        components: components.clamp(1, 4),
        parameters: ta_core::legacy_parameter_template(components),
        fit_range: None,
        excluded: Vec::new(),
        weighting: Weighting::Uniform,
        options,
    };
    let result = fit_trace(trace, &request)?;
    let time_zero = result
        .parameter_value("time_zero")
        .ok_or_else(|| FitError::MissingParameter("time_zero".to_string()))?;
    Ok(TimeZeroEstimate {
        time_zero,
        sse: result.stats.sse,
        converged: result.stats.converged,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{evaluate, Component, Model, ModelParams};
    use ta_core::{SignalUnit, VersionId};

    /// A synthetic trace built from the model itself, so the truth is known.
    fn synthetic(
        amplitudes: &[f64],
        lifetimes: &[f64],
        irf_fwhm: f64,
        time_zero: f64,
        noise: f64,
    ) -> Trace {
        let mut components: Vec<Component> = amplitudes
            .iter()
            .zip(lifetimes)
            .map(|(amplitude, lifetime)| Component {
                amplitude: *amplitude,
                lifetime: *lifetime,
            })
            .collect();
        while components.len() < 4 {
            components.push(Component {
                amplitude: 0.0,
                lifetime: 0.0,
            });
        }
        let params = ModelParams {
            irf_fwhm,
            amplitude_scale: 1.0,
            components,
            time_zero,
            pre_zero: 0.0,
            long_term: 0.0,
        };
        let delays: Vec<f64> = (0..300).map(|i| -1.0 + i as f64 * 0.02).collect();
        let mut signal = evaluate(Model::LegacyGaussianCdfIrf, &params, &delays);
        // A deterministic wobble, so the test does not depend on a random seed.
        if noise > 0.0 {
            for (index, value) in signal.iter_mut().enumerate() {
                *value += noise * ((index as f64 * 12.9898).sin() * 43758.5453).fract();
            }
        }
        Trace {
            label: "synthetic".to_string(),
            delay: delays,
            signal,
            unit: SignalUnit::DeltaOpticalDensity,
            version: VersionId::new(),
            sampled: ta_core::Sample {
                coordinate: 0.0,
                indices: vec![0],
                weights: vec![1.0],
                coordinates: vec![0.0],
                mode: ta_core::SampleMode::Nearest,
            },
        }
    }

    #[test]
    fn known_lifetimes_are_recovered_from_a_noisy_trace() {
        let trace = synthetic(&[0.4, 0.2], &[1.5, 30.0], 0.12, 0.05, 3e-4);
        let request = FitRequest::new(
            ta_core::DatasetId::new(),
            trace.version,
            TraceSelection::new(
                ta_core::DatasetId::new(),
                trace.version,
                ta_core::AxisRequest::nearest(0.0),
            ),
            FitModelKind::LegacyGaussianCdfIrf,
            2,
        );
        let result = fit_trace(&trace, &request).unwrap();
        assert!(result.stats.converged, "{}", result.stats.message);
        let tau1 = result.lifetime(1).unwrap();
        let tau2 = result.lifetime(2).unwrap();
        assert!((tau1 - 1.5).abs() / 1.5 < 0.1, "tau1 was {tau1}");
        assert!((tau2 - 30.0).abs() / 30.0 < 0.1, "tau2 was {tau2}");
        let a1 = result.amplitude(1).unwrap();
        assert!((a1 - 0.4).abs() < 0.05, "amplitude 1 was {a1}");
        // The instrument response width and the time zero come back too.
        let irf = result.parameter_value("irf_fwhm").unwrap();
        assert!((irf - 0.12).abs() < 0.03, "irf was {irf}");
        let t0 = result.parameter_value("time_zero").unwrap();
        assert!((t0 - 0.05).abs() < 0.03, "time zero was {t0}");
        // And the report carries residuals and an R².
        assert_eq!(result.residuals.as_ref().unwrap().x.len(), 300);
        assert!(result.stats.r_squared.unwrap() > 0.99);
    }

    #[test]
    fn a_fit_range_and_an_excluded_interval_leave_points_out() {
        let trace = synthetic(&[0.5], &[2.0], 0.1, 0.0, 0.0);
        let mut request = FitRequest::new(
            ta_core::DatasetId::new(),
            trace.version,
            TraceSelection::new(
                ta_core::DatasetId::new(),
                trace.version,
                ta_core::AxisRequest::nearest(0.0),
            ),
            FitModelKind::LegacyGaussianCdfIrf,
            1,
        );
        request.fit_range = Some((0.5, 3.0));
        request.excluded = vec![(1.0, 1.5)];
        let result = fit_trace(&trace, &request).unwrap();
        let delays = &result.residuals.as_ref().unwrap().x;
        assert!(delays.iter().all(|delay| *delay >= 0.5 && *delay <= 3.0));
        assert!(delays.iter().all(|delay| *delay < 1.0 || *delay > 1.5));
        assert!(result.stats.dof > 0);
    }

    #[test]
    fn a_range_with_too_few_points_is_an_error_rather_than_a_fit_on_one_point() {
        let trace = synthetic(&[0.5], &[2.0], 0.1, 0.0, 0.0);
        let mut request = FitRequest::new(
            ta_core::DatasetId::new(),
            trace.version,
            TraceSelection::new(
                ta_core::DatasetId::new(),
                trace.version,
                ta_core::AxisRequest::nearest(0.0),
            ),
            FitModelKind::LegacyGaussianCdfIrf,
            1,
        );
        request.fit_range = Some((0.0, 0.005));
        assert!(matches!(
            fit_trace(&trace, &request),
            Err(FitError::Invalid(_))
        ));
    }

    #[test]
    fn uniform_weighting_reports_no_chi_square_and_weighted_weighting_does() {
        let trace = synthetic(&[0.3], &[5.0], 0.1, 0.0, 1e-4);
        let mut request = FitRequest::new(
            ta_core::DatasetId::new(),
            trace.version,
            TraceSelection::new(
                ta_core::DatasetId::new(),
                trace.version,
                ta_core::AxisRequest::nearest(0.0),
            ),
            FitModelKind::LegacyGaussianCdfIrf,
            1,
        );
        let unweighted = fit_trace(&trace, &request).unwrap();
        assert_eq!(unweighted.stats.chi_square, None);
        assert!(unweighted.stats.reduced_chi_square.is_some());

        request.weighting = Weighting::Poisson;
        let weighted = fit_trace(&trace, &request).unwrap();
        assert!(weighted.stats.chi_square.is_some());
    }

    #[test]
    fn explicit_weights_that_do_not_match_the_points_are_refused() {
        let trace = synthetic(&[0.3], &[5.0], 0.1, 0.0, 0.0);
        let mut request = FitRequest::new(
            ta_core::DatasetId::new(),
            trace.version,
            TraceSelection::new(
                ta_core::DatasetId::new(),
                trace.version,
                ta_core::AxisRequest::nearest(0.0),
            ),
            FitModelKind::LegacyGaussianCdfIrf,
            1,
        );
        request.weighting = Weighting::Explicit(vec![1.0, 1.0]);
        assert!(fit_trace(&trace, &request).is_err());
    }

    #[test]
    fn the_time_zero_estimator_returns_the_offset_it_found() {
        let trace = synthetic(&[0.5], &[3.0], 0.1, 0.25, 1e-5);
        let estimate = estimate_time_zero(&trace, 1, SolverOptions::default()).unwrap();
        assert!((estimate.time_zero - 0.25).abs() < 0.05, "{estimate:?}");
        assert!(estimate.converged);
    }

    #[test]
    fn the_model_curve_is_available_for_the_overlay() {
        let parameters = ta_core::legacy_parameter_template(1);
        let curve = model_curve(
            FitModelKind::LegacyGaussianCdfIrf,
            &parameters,
            &[0.0, 1.0, 10.0],
        )
        .unwrap();
        assert_eq!(curve.len(), 3);
        assert!(curve.iter().all(|value| value.is_finite()));
    }
}
