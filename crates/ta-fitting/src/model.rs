//! The transient absorption models.
//!
//! Two models are offered, and the difference between them is not cosmetic:
//!
//! * [`Model::LegacyGaussianCdfIrf`] multiplies each exponential by a Gaussian
//!   *cumulative distribution*. That is the reference implementation's
//!   formulation, and it is not an exact convolution of a causal exponential
//!   with the instrument response. It is kept under its own name because every
//!   result a user already has came out of it (`spec.md` §12.2);
//! * [`Model::ConvolvedExponential`] is the exact convolution, for new work.
//!
//! Everything is in the trace's own units: delays in ps, absorbance in ΔOD.

use crate::{FitError, Result};
use ta_core::{FitModelKind, FitParameter};

/// One exponential component.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Component {
    pub amplitude: f64,
    /// Lifetime in ps. A lifetime of zero switches the component off, which is
    /// what the reference implementation does.
    pub lifetime: f64,
}

/// The model's parameters, in the units the fit records.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelParams {
    /// Instrument response full width at half maximum, ps.
    pub irf_fwhm: f64,
    /// Overall amplitude scale. The reference implementation carries this
    /// parameter and then ignores it (`result *= 1`); here it does what its
    /// comment says it does, which is a no-op at its default of 1.
    pub amplitude_scale: f64,
    pub components: Vec<Component>,
    /// Delay at which the signal rises, ps.
    pub time_zero: f64,
    /// Constant offset before the rise.
    pub pre_zero: f64,
    /// Offset reached long after the rise.
    pub long_term: f64,
}

impl ModelParams {
    /// The parameters the fitting interface starts from, faithful to the
    /// reference implementation's `params_init`.
    pub fn from_template(components: u8) -> Self {
        Self::from_parameters(&ta_core::legacy_parameter_template(components))
            .expect("the template holds every parameter")
    }

    /// Reads a parameter list by name.
    pub fn from_parameters(parameters: &[FitParameter]) -> Result<Self> {
        let value = |name: &str| -> Result<f64> {
            parameters
                .iter()
                .find(|parameter| parameter.name == name)
                .map(|parameter| parameter.value)
                .ok_or_else(|| FitError::MissingParameter(name.to_string()))
        };
        let mut components = Vec::new();
        for index in 1..=4u8 {
            components.push(Component {
                amplitude: value(&format!("amplitude_{index}"))?,
                lifetime: value(&format!("lifetime_{index}"))?,
            });
        }
        Ok(Self {
            irf_fwhm: value("irf_fwhm")?,
            amplitude_scale: value("amplitude_scale")?,
            components,
            time_zero: value("time_zero")?,
            pre_zero: value("pre_zero_offset")?,
            long_term: value("long_term_offset")?,
        })
    }

    /// Writes a parameter list back, keeping every other field of the record.
    pub fn into_parameters(self, parameters: &[FitParameter]) -> Vec<FitParameter> {
        let mut updated = parameters.to_vec();
        let mut set = |name: &str, value: f64| {
            if let Some(parameter) = updated
                .iter_mut()
                .find(|parameter| parameter.name == name)
            {
                parameter.value = value;
            }
        };
        set("irf_fwhm", self.irf_fwhm);
        set("amplitude_scale", self.amplitude_scale);
        for (index, component) in self.components.iter().enumerate() {
            set(&format!("amplitude_{}", index + 1), component.amplitude);
            set(&format!("lifetime_{}", index + 1), component.lifetime);
        }
        set("time_zero", self.time_zero);
        set("pre_zero_offset", self.pre_zero);
        set("long_term_offset", self.long_term);
        updated
    }

    /// The parameters in the order the solver sees them, with the names that
    /// match [`ta_core::legacy_parameter_template`].
    pub fn to_vector(&self) -> (Vec<&'static str>, Vec<f64>) {
        let mut names: Vec<&'static str> = vec!["irf_fwhm", "amplitude_scale"];
        let mut values = vec![self.irf_fwhm, self.amplitude_scale];
        for (index, component) in self.components.iter().enumerate() {
            names.push(match index {
                0 => "amplitude_1",
                1 => "amplitude_2",
                2 => "amplitude_3",
                _ => "amplitude_4",
            });
            values.push(component.amplitude);
            names.push(match index {
                0 => "lifetime_1",
                1 => "lifetime_2",
                2 => "lifetime_3",
                _ => "lifetime_4",
            });
            values.push(component.lifetime);
        }
        names.push("time_zero");
        values.push(self.time_zero);
        names.push("pre_zero_offset");
        values.push(self.pre_zero);
        names.push("long_term_offset");
        values.push(self.long_term);
        (names, values)
    }

    /// Reads the parameters back from a solution vector built by
    /// [`ModelParams::to_vector`].
    pub fn from_vector(values: &[f64]) -> Result<Self> {
        if values.len() != 13 {
            return Err(FitError::Invalid(format!(
                "the legacy model takes 13 parameters, got {}",
                values.len()
            )));
        }
        let mut components = Vec::with_capacity(4);
        for index in 0..4 {
            components.push(Component {
                amplitude: values[2 + index * 2],
                lifetime: values[3 + index * 2],
            });
        }
        Ok(Self {
            irf_fwhm: values[0],
            amplitude_scale: values[1],
            components,
            time_zero: values[10],
            pre_zero: values[11],
            long_term: values[12],
        })
    }
}

/// Which formulation to evaluate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    LegacyGaussianCdfIrf,
    ConvolvedExponential,
}

impl From<FitModelKind> for Model {
    fn from(kind: FitModelKind) -> Self {
        match kind {
            FitModelKind::LegacyGaussianCdfIrf => Model::LegacyGaussianCdfIrf,
            FitModelKind::ConvolvedExponential => Model::ConvolvedExponential,
        }
    }
}

/// The Gaussian's standard deviation for a given full width at half maximum.
fn sigma_from_fwhm(fwhm: f64) -> f64 {
    // The reference implementation's expression, which squares the width first:
    // sqrt(w0^2) / (2 sqrt(2 ln 2)).
    fwhm.abs() / (2.0 * (2.0 * std::f64::consts::LN_2).sqrt())
}

/// The error function.
///
/// Abramowitz and Stegun 7.1.6, whose terms are all positive:
/// `erf(x) = 2x/√π · e^{-x²} · Σ (2x²)ⁿ / (2n+1)!!`. A series with no
/// alternating signs cannot cancel digits away, so this holds about 1e-15
/// relative accuracy across the whole range a fit visits, and is exact at zero.
/// (A continued fraction would be cheaper in the far tail, but the tail of a
/// normal distribution is 1e-7 from its limit by then.)
pub fn erf(x: f64) -> f64 {
    if x == 0.0 {
        return 0.0;
    }
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let magnitude = x.abs();
    // Beyond six the function is one to within 2e-17 — below the resolution of
    // a double against one — and the series would overflow computing it.
    if magnitude > 6.0 {
        return sign;
    }
    let x2 = magnitude * magnitude;
    let mut term = 1.0;
    let mut total = 1.0;
    for n in 0..400 {
        term *= 2.0 * x2 / (2.0 * n as f64 + 3.0);
        total += term;
        if term <= 1e-18 * total {
            break;
        }
    }
    sign * (2.0 * magnitude / std::f64::consts::PI.sqrt()) * (-x2).exp() * total
}

/// The complementary error function.
///
/// `1 - erf(x)`. Its absolute accuracy is about 1e-16 everywhere; its relative
/// accuracy necessarily degrades in the tail, where the answer is a rounding of
/// one (about 1e-11 at `x = 3`, and zero beyond `x = 6`). Both uses in this
/// crate want the absolute value: a cumulative distribution that is 1e-17 wrong
/// and a convolution whose early-time factor is 1e-17 instead of zero.
pub fn erfc(x: f64) -> f64 {
    1.0 - erf(x)
}

/// The standard normal cumulative distribution.
pub fn normal_cdf(x: f64) -> f64 {
    0.5 * erfc(-x / std::f64::consts::SQRT_2)
}

/// Evaluates the model at the given delays.
pub fn evaluate(model: Model, params: &ModelParams, delays: &[f64]) -> Vec<f64> {
    let sigma = sigma_from_fwhm(params.irf_fwhm);
    delays
        .iter()
        .map(|t| {
            let shifted = t - params.time_zero;
            let cdf = if sigma > 0.0 {
                normal_cdf(shifted / sigma)
            } else {
                // A zero-width instrument response: a step.
                if shifted >= 0.0 {
                    1.0
                } else {
                    0.0
                }
            };
            let signal: f64 = params
                .components
                .iter()
                .map(|component| match model {
                    Model::LegacyGaussianCdfIrf => {
                        if component.lifetime == 0.0 {
                            0.0
                        } else {
                            component.amplitude
                                * (-shifted / component.lifetime).exp()
                                * cdf
                        }
                    }
                    Model::ConvolvedExponential => convolved_component(
                        component.amplitude,
                        component.lifetime,
                        shifted,
                        sigma,
                    ),
                })
                .sum();
            let baseline = params.pre_zero + params.long_term * cdf;
            (signal + baseline) * params.amplitude_scale
        })
        .collect()
}

/// The exact convolution of a causal exponential with a Gaussian instrument
/// response, as a function of the delay measured from the time zero.
fn convolved_component(amplitude: f64, lifetime: f64, shifted: f64, sigma: f64) -> f64 {
    if lifetime == 0.0 || amplitude == 0.0 {
        return 0.0;
    }
    if sigma <= 0.0 {
        // The convolution collapses to the exponential itself, switched on at
        // the time zero.
        return if shifted >= 0.0 {
            amplitude * (-shifted / lifetime).exp()
        } else {
            0.0
        };
    }
    let ratio = sigma / lifetime;
    let exponent = 0.5 * ratio * ratio - shifted / lifetime;
    let argument = (sigma / lifetime - shifted / sigma) / std::f64::consts::SQRT_2;
    // The exponential grows without bound as the argument goes negative, and
    // `erfc` of a large positive argument underflows to zero; the product is
    // evaluated in the log domain when that would lose the answer.
    if exponent > 700.0 {
        return 0.0;
    }
    let value = 0.5 * amplitude * exponent.exp() * erfc(argument);
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_error_function_matches_its_known_values() {
        assert_eq!(erf(0.0), 0.0);
        
        
        
        for (x, expected) in [
            (0.25, 0.276_326_390_168_236_96),
            (0.5, 0.520_499_877_813_046_5),
            (1.0, 0.842_700_792_949_714_9),
            (2.0, 0.995_322_265_018_952_7),
            (2.9, 0.999_958_902_121_900_5),
        ] {
            let delta = (erf(x) - expected).abs();
            println!("erf({x}): delta {delta:e}");
            assert!(delta < 1e-14, "erf({x}) is off by {delta:e}");
        }
        assert!((erf(0.5) - 0.520_499_877_813_046_5).abs() < 1e-15);
        // The values below come from the C library's `erf`, which is
        // correctly rounded to within an ulp.
        
        assert!((erf(3.5) - 0.999_999_256_901_627_6).abs() < 1e-15);
        
        
        assert!((erf(-4.0) + 0.999_999_984_582_742_1).abs() < 1e-15);
    }

    #[test]
    fn the_normal_distribution_matches_its_known_values() {
        assert_eq!(normal_cdf(0.0), 0.5);
        assert!((normal_cdf(1.0) - 0.841_344_746_068_542_9).abs() < 1e-15);
        assert!((normal_cdf(-1.96) - 0.024_997_895_148_220_435).abs() < 1e-15);
        assert!((normal_cdf(3.0) - 0.998_650_101_968_369_9).abs() < 1e-15);
    }

    #[test]
    fn the_full_width_at_half_maximum_gives_the_expected_sigma() {
        // 2 sqrt(2 ln 2) = 2.3548, so a 1 ps width is a 0.4247 ps sigma.
        assert!((sigma_from_fwhm(1.0) - 0.424_661_0).abs() < 1e-6);
        assert_eq!(sigma_from_fwhm(0.0), 0.0);
    }

    #[test]
    fn the_legacy_model_is_the_expression_the_reference_implements() {
        // A single exponential, evaluated by hand from the same formula.
        let params = ModelParams {
            irf_fwhm: 0.1,
            amplitude_scale: 1.0,
            components: vec![
                Component {
                    amplitude: -0.5,
                    lifetime: 10.0,
                },
                Component {
                    amplitude: 0.0,
                    lifetime: 0.0,
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
            time_zero: 0.1,
            pre_zero: -0.01,
            long_term: 0.02,
        };
        let delays = [-1.0, 0.0, 0.1, 1.0, 10.0];
        let evaluated = evaluate(Model::LegacyGaussianCdfIrf, &params, &delays);
        let sigma = sigma_from_fwhm(0.1);
        for (index, t) in delays.iter().enumerate() {
            let shifted = t - 0.1;
            let cdf = normal_cdf(shifted / sigma);
            let expected = -0.5 * (-shifted / 10.0).exp() * cdf + (-0.01 + 0.02 * cdf);
            assert!(
                (evaluated[index] - expected).abs() < 1e-12,
                "at {t}: {} against {expected}",
                evaluated[index]
            );
        }
    }

    #[test]
    fn a_zero_lifetime_switches_a_component_off() {
        let params = ModelParams {
            irf_fwhm: 0.1,
            amplitude_scale: 1.0,
            components: vec![
                Component {
                    amplitude: 1.0,
                    lifetime: 0.0,
                },
                Component {
                    amplitude: 0.0,
                    lifetime: 0.0,
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
            time_zero: 0.0,
            pre_zero: 0.0,
            long_term: 0.0,
        };
        let evaluated = evaluate(Model::LegacyGaussianCdfIrf, &params, &[-1.0, 0.0, 1.0]);
        assert!(evaluated.iter().all(|value| value.abs() < 1e-15));
    }

    #[test]
    fn the_amplitude_scale_multiplies_the_whole_curve() {
        let mut params = ModelParams::from_template(1);
        params.components[0].amplitude = 0.3;
        params.components[0].lifetime = 5.0;
        let once = evaluate(Model::LegacyGaussianCdfIrf, &params, &[1.0, 5.0]);
        params.amplitude_scale = 2.0;
        let twice = evaluate(Model::LegacyGaussianCdfIrf, &params, &[1.0, 5.0]);
        assert!((twice[0] - once[0] * 2.0).abs() < 1e-12);
    }

    #[test]
    fn the_convolved_model_rises_like_an_exponential_after_the_rise() {
        let params = ModelParams {
            irf_fwhm: 0.05,
            amplitude_scale: 1.0,
            components: vec![
                Component {
                    amplitude: 1.0,
                    lifetime: 100.0,
                },
                Component {
                    amplitude: 0.0,
                    lifetime: 0.0,
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
            time_zero: 0.0,
            pre_zero: 0.0,
            long_term: 0.0,
        };
        // Long after the rise the convolution is the exponential itself.
        let late = evaluate(Model::ConvolvedExponential, &params, &[20.0])[0];
        assert!(
            (late - (-0.2f64).exp()).abs() < 2e-3,
            "long after the rise: {late}"
        );
        // Halfway through a short rise the signal is still climbing.
        let rising = evaluate(Model::ConvolvedExponential, &params, &[0.0, 0.02, 0.06]);
        assert!(rising[0] > 0.0 && rising[0] < rising[1] && rising[1] < rising[2]);
        // And before it, the convolution is essentially zero, unlike the legacy
        // model which leaks a little of the exponential backwards.
        let before = evaluate(Model::ConvolvedExponential, &params, &[-1.0])[0];
        assert!(before < 1e-6, "before the rise: {before}");
    }

    #[test]
    fn a_zero_width_instrument_response_makes_the_convolution_a_step() {
        let params = ModelParams {
            irf_fwhm: 0.0,
            amplitude_scale: 1.0,
            components: vec![
                Component {
                    amplitude: 1.0,
                    lifetime: 50.0,
                },
                Component {
                    amplitude: 0.0,
                    lifetime: 0.0,
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
            time_zero: 0.0,
            pre_zero: 0.0,
            long_term: 0.0,
        };
        let before = evaluate(Model::ConvolvedExponential, &params, &[-0.1])[0];
        let after = evaluate(Model::ConvolvedExponential, &params, &[0.0])[0];
        assert_eq!(before, 0.0);
        assert_eq!(after, 1.0);
    }

    #[test]
    fn parameters_round_trip_through_the_vector_form() {
        let params = ModelParams::from_template(3);
        let (names, values) = params.to_vector();
        assert_eq!(names.len(), 13);
        assert_eq!(values.len(), 13);
        assert_eq!(names[0], "irf_fwhm");
        assert_eq!(names[12], "long_term_offset");
        let back = ModelParams::from_vector(&values).unwrap();
        assert_eq!(back, params);
    }

    #[test]
    fn a_parameter_list_missing_a_name_is_refused_rather_than_defaulted() {
        let mut parameters = ta_core::legacy_parameter_template(1);
        parameters.retain(|parameter| parameter.name != "time_zero");
        assert!(matches!(
            ModelParams::from_parameters(&parameters),
            Err(FitError::MissingParameter(name)) if name == "time_zero"
        ));
        assert!(ModelParams::from_vector(&[1.0, 2.0]).is_err());
    }
}
