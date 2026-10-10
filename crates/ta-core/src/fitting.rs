//! Fit results as data.
//!
//! The solver lives in `ta-fitting`; the *record* of a fit lives here, because
//! a dataset owns its fitting sessions and a project has to persist them.
//! Standard errors are `Option` on purpose: a solver that cannot identify a
//! parameter must be able to say so instead of reporting a number.

use crate::ids::{DatasetId, FitId, VersionId};
use crate::selection::TraceSelection;
use serde::{Deserialize, Serialize};

/// Which model was fitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FitModelKind {
    /// The reference implementation's model: each exponential multiplied by a
    /// Gaussian CDF. Kept by name because it is *not* an exact Gaussian
    /// convolution, and users' existing results come from it.
    LegacyGaussianCdfIrf,
    /// Physically convolved exponential response with a Gaussian IRF.
    ConvolvedExponential,
}

impl FitModelKind {
    pub const fn label(self) -> &'static str {
        match self {
            FitModelKind::LegacyGaussianCdfIrf => "Legacy Gaussian-CDF IRF",
            FitModelKind::ConvolvedExponential => "Convolved exponential / Gaussian IRF",
        }
    }

    /// Component counts this model accepts.
    pub const fn component_range(self) -> (u8, u8) {
        (1, 4)
    }

    pub const fn is_legacy(self) -> bool {
        matches!(self, FitModelKind::LegacyGaussianCdfIrf)
    }
}

/// One fitted parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FitParameter {
    /// The readable name used throughout the interface.
    pub name: String,
    /// The reference implementation's slot name (`w0`..`w12`), kept so results
    /// can be compared against Python without a translation table.
    pub legacy_name: Option<String>,
    pub value: f64,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub fixed: bool,
    /// Present only when the solver could estimate it.
    pub stderr: Option<f64>,
    pub unit: Option<String>,
}

impl FitParameter {
    pub fn new(name: impl Into<String>, value: f64) -> Self {
        Self {
            name: name.into(),
            legacy_name: None,
            value,
            min: None,
            max: None,
            fixed: false,
            stderr: None,
            unit: None,
        }
    }

    pub fn legacy(mut self, slot: impl Into<String>) -> Self {
        self.legacy_name = Some(slot.into());
        self
    }

    pub fn with_bounds(mut self, min: f64, max: f64) -> Self {
        self.min = Some(min);
        self.max = Some(max);
        self
    }

    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }

    pub fn fixed(mut self) -> Self {
        self.fixed = true;
        self
    }

    pub fn free(mut self) -> Self {
        self.fixed = false;
        self
    }

    /// `value` moved inside this parameter's bounds.
    pub fn clamped(&self, value: f64) -> f64 {
        let lower = self.min.unwrap_or(f64::NEG_INFINITY);
        let upper = self.max.unwrap_or(f64::INFINITY);
        value.clamp(lower, upper)
    }

    /// Whether the parameter sits on a bound, which is a real fit problem the
    /// interface has to show rather than hide.
    pub fn at_bound(&self) -> bool {
        let tolerance = self.value.abs() * 1e-6 + 1e-12;
        self.min.is_some_and(|min| (self.value - min).abs() <= tolerance)
            || self.max.is_some_and(|max| (self.value - max).abs() <= tolerance)
    }
}

/// A named set of starting values, bounds and fixed flags, so a fit can be
/// repeated on another trace without retyping thirteen numbers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParameterPreset {
    pub name: String,
    pub model: FitModelKind,
    pub components: u8,
    pub parameters: Vec<FitParameter>,
}

impl ParameterPreset {
    /// The preset that a component count implies, which is what the reference
    /// implementation's defaults are.
    pub fn from_template(name: impl Into<String>, components: u8) -> Self {
        Self {
            name: name.into(),
            model: FitModelKind::LegacyGaussianCdfIrf,
            components,
            parameters: legacy_parameter_template(components),
        }
    }
}

/// The starting point for the legacy model, faithful to the reference
/// implementation's `params_init`.
///
/// Bounds and initial values are the Python defaults, so a user's existing
/// habits in the notebook carry over unchanged.
pub fn legacy_parameter_template(components: u8) -> Vec<FitParameter> {
    let components = components.clamp(1, 4);
    let amplitudes = [0.0, 0.0, 0.0, 0.0];
    let lifetimes = [1.0, 10.0, 50.0, 500.0];

    let mut params = vec![
        FitParameter::new("irf_fwhm", 0.1)
            .legacy("w0")
            .with_bounds(0.05, 0.2)
            .with_unit("ps"),
        FitParameter::new("amplitude_scale", 1.0).legacy("w1").fixed(),
    ];
    for index in 0..4 {
        let slot = index + 1;
        let is_active = (index as u8) < components;
        let amplitude = FitParameter::new(format!("amplitude_{slot}"), amplitudes[index])
            .legacy(format!("w{}", 2 + index * 2))
            .with_bounds(-0.5, 0.5)
            .with_unit("ΔOD");
        let lifetime = FitParameter::new(format!("lifetime_{slot}"), lifetimes[index])
            .legacy(format!("w{}", 3 + index * 2))
            .with_bounds(0.01, 5000.0)
            .with_unit("ps");
        params.push(if is_active {
            amplitude
        } else {
            amplitude.fixed()
        });
        params.push(if is_active {
            lifetime
        } else {
            lifetime.fixed()
        });
    }
    params.push(
        FitParameter::new("time_zero", 0.0)
            .legacy("w10")
            .with_bounds(-0.5, 0.5)
            .with_unit("ps"),
    );
    params.push(
        FitParameter::new("pre_zero_offset", 0.0)
            .legacy("w11")
            .with_bounds(-0.1, 0.1)
            .with_unit("ΔOD"),
    );
    params.push(
        FitParameter::new("long_term_offset", 0.0)
            .legacy("w12")
            .with_bounds(-0.5, 0.5)
            .with_unit("ΔOD")
            .fixed(),
    );
    params
}

/// How residual points were weighted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Weighting {
    Uniform,
    /// Weights equal to the signal magnitude.
    Poisson,
    /// Explicit per-point weights, one per fitted delay.
    Explicit(Vec<f64>),
    /// Excluded time intervals, applied on top of any weighting.
    IntervalExclusions(Vec<(f64, f64)>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResidualData {
    pub x: Vec<f64>,
    pub y: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FitStats {
    pub sse: f64,
    /// Only meaningful with informative per-point errors.
    pub chi_square: Option<f64>,
    pub reduced_chi_square: Option<f64>,
    pub r_squared: Option<f64>,
    /// Degrees of freedom actually used.
    pub dof: usize,
    pub iterations: usize,
    pub converged: bool,
    pub message: String,
    /// Parameter covariance, when the solver could produce it.
    pub covariance: Option<Vec<Vec<f64>>>,
}

impl Default for FitStats {
    fn default() -> Self {
        Self {
            sse: f64::NAN,
            chi_square: None,
            reduced_chi_square: None,
            r_squared: None,
            dof: 0,
            iterations: 0,
            converged: false,
            message: String::new(),
            covariance: None,
        }
    }
}

/// A retained fitting session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FitResult {
    pub id: FitId,
    pub dataset: DatasetId,
    pub version: VersionId,
    /// Which trace was fitted, in physical coordinates.
    pub selection: TraceSelection,
    pub model: FitModelKind,
    pub components: u8,
    pub params: Vec<FitParameter>,
    pub stats: FitStats,
    pub residuals: Option<ResidualData>,
    /// Delay interval that was fitted.
    pub fit_range: Option<(f64, f64)>,
    /// Delay intervals that were excluded.
    pub excluded: Vec<(f64, f64)>,
    pub weighting: Weighting,
    pub created_at: u64,
}

impl FitResult {
    pub fn parameter(&self, name: &str) -> Option<&FitParameter> {
        self.params.iter().find(|param| param.name == name)
    }

    pub fn parameter_value(&self, name: &str) -> Option<f64> {
        self.parameter(name).map(|param| param.value)
    }

    /// The `index`-th exponential lifetime, 1-based.
    pub fn lifetime(&self, index: u8) -> Option<f64> {
        self.parameter_value(&format!("lifetime_{index}"))
    }

    /// The `index`-th exponential amplitude, 1-based.
    pub fn amplitude(&self, index: u8) -> Option<f64> {
        self.parameter_value(&format!("amplitude_{index}"))
    }

    /// Parameters that hit a bound, which the interface must surface.
    pub fn pinned_parameters(&self) -> Vec<&FitParameter> {
        self.params
            .iter()
            .filter(|param| !param.fixed && param.at_bound())
            .collect()
    }

    pub fn converged(&self) -> bool {
        self.stats.converged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_template_keeps_the_reference_slot_names() {
        let params = legacy_parameter_template(2);
        assert_eq!(params.len(), 13);
        assert_eq!(params[0].name, "irf_fwhm");
        assert_eq!(params[0].legacy_name.as_deref(), Some("w0"));
        assert_eq!(params[12].legacy_name.as_deref(), Some("w12"));
        assert_eq!(params[12].name, "long_term_offset");
    }

    #[test]
    fn unused_components_are_present_but_fixed() {
        let params = legacy_parameter_template(1);
        let amplitude_2 = params
            .iter()
            .find(|p| p.name == "amplitude_2")
            .expect("slot 2 exists");
        assert!(amplitude_2.fixed);
        let amplitude_1 = params.iter().find(|p| p.name == "amplitude_1").unwrap();
        assert!(!amplitude_1.fixed);
    }

    #[test]
    fn a_parameter_on_its_bound_is_reported() {
        let param = FitParameter::new("lifetime_1", 5000.0).with_bounds(0.01, 5000.0);
        assert!(param.at_bound());
        let interior = FitParameter::new("lifetime_1", 12.0).with_bounds(0.01, 5000.0);
        assert!(!interior.at_bound());
    }

    #[test]
    fn clamping_stays_inside_the_bounds() {
        let param = FitParameter::new("amplitude_1", 0.0).with_bounds(-0.5, 0.5);
        assert_eq!(param.clamped(9.0), 0.5);
        assert_eq!(param.clamped(-9.0), -0.5);
        assert_eq!(param.clamped(0.2), 0.2);
    }
}
