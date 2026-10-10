//! Axis scales: the reversible map between a physical coordinate and the space
//! a plot is laid out in.
//!
//! Display, mouse hit testing, box selection and export all go through this one
//! object, so the number a crosshair reports is the number that was drawn
//! (spec §6.4).
//!
//! Transient absorption delay axes contain zero, negative values and a long
//! positive tail, and wavelength axes are frequently non-uniform. That is why
//! [`Scale`] offers a symmetric-logarithmic and a split linear/logarithmic map
//! rather than only `ln`.

use ta_core::AxisScale;

/// How a physical coordinate becomes a layout coordinate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scale {
    Linear,
    /// `ln(v)`, defined only for strictly positive coordinates.
    Logarithmic,
    /// Linear within `±linear_width`, logarithmic outside it. Defined
    /// everywhere, including at zero.
    SymmetricLogarithmic { linear_width: f64 },
    /// Linear up to `break_at`, logarithmic above it, with the two parts meeting
    /// at slope 1 so the join is smooth.
    SplitLinearLog { break_at: f64 },
}

impl Default for Scale {
    fn default() -> Self {
        Scale::Linear
    }
}

impl Scale {
    /// Builds a scale from the figure model, rejecting parameters that would
    /// make the map undefined or ambiguous.
    pub fn new(spec: AxisScale) -> ta_core::Result<Self> {
        let scale = match spec {
            AxisScale::Linear => Scale::Linear,
            AxisScale::Logarithmic => Scale::Logarithmic,
            AxisScale::SymmetricLogarithmic { linear_width } => {
                if !(linear_width.is_finite() && linear_width > 0.0) {
                    return Err(ta_core::CoreError::invalid(format!(
                        "a symmetric-logarithmic axis needs a positive linear width, got \
                         {linear_width}"
                    )));
                }
                Scale::SymmetricLogarithmic { linear_width }
            }
            AxisScale::SplitLinearLog { break_at } => {
                if !(break_at.is_finite() && break_at > 0.0) {
                    return Err(ta_core::CoreError::invalid(format!(
                        "a split linear/log axis needs a positive break point, got {break_at}"
                    )));
                }
                Scale::SplitLinearLog { break_at }
            }
        };
        Ok(scale)
    }

    /// Whether the map is defined at this coordinate.
    pub fn is_defined_at(&self, value: f64) -> bool {
        match self {
            Scale::Linear => value.is_finite(),
            Scale::Logarithmic => value.is_finite() && value > 0.0,
            Scale::SymmetricLogarithmic { .. } => value.is_finite(),
            Scale::SplitLinearLog { .. } => value.is_finite(),
        }
    }

    /// The lowest coordinate the map is defined at, or `None` when it is
    /// unbounded below.
    pub fn lowest_defined(&self) -> Option<f64> {
        match self {
            Scale::Logarithmic => Some(0.0),
            _ => None,
        }
    }

    /// The physical coordinate mapped into layout space, or `None` where the map
    /// has no value.
    pub fn forward(&self, value: f64) -> Option<f64> {
        if !self.is_defined_at(value) {
            return None;
        }
        match self {
            Scale::Linear => Some(value),
            Scale::Logarithmic => Some(value.ln()),
            Scale::SymmetricLogarithmic { linear_width } => {
                let magnitude = value.abs();
                if magnitude <= *linear_width {
                    Some(value)
                } else {
                    let sign = if value < 0.0 { -1.0 } else { 1.0 };
                    Some(sign * linear_width * (1.0 + (magnitude / linear_width).ln()))
                }
            }
            Scale::SplitLinearLog { break_at } => {
                if value <= *break_at {
                    Some(value)
                } else {
                    Some(break_at + break_at * (value / break_at).ln())
                }
            }
        }
    }

    /// The physical coordinate behind a layout coordinate. Always defined: the
    /// layout space is the whole real line for every scale here.
    pub fn inverse(&self, transformed: f64) -> f64 {
        if !transformed.is_finite() {
            return f64::NAN;
        }
        match self {
            Scale::Linear => transformed,
            Scale::Logarithmic => transformed.exp(),
            Scale::SymmetricLogarithmic { linear_width } => {
                let magnitude = transformed.abs();
                if magnitude <= *linear_width {
                    transformed
                } else {
                    let sign = if transformed < 0.0 { -1.0 } else { 1.0 };
                    sign * linear_width * (magnitude / linear_width - 1.0).exp()
                }
            }
            Scale::SplitLinearLog { break_at } => {
                if transformed <= *break_at {
                    transformed
                } else {
                    break_at * ((transformed - break_at) / break_at).exp()
                }
            }
        }
    }

    /// Moves a physical range onto the part of the domain where the map exists.
    ///
    /// Returns whether it had to clamp, so the interface can say so rather than
    /// showing an axis that quietly ignores half its data.
    pub fn clamp_domain(&self, low: f64, high: f64) -> ((f64, f64), bool) {
        let (mut low, mut high) = (low.min(high), low.max(high));
        let mut clamped = false;
        if let Scale::Logarithmic = self {
            if low <= 0.0 {
                // The lowest decade the data reaches, or just above zero when
                // there is no positive coordinate at all.
                let floor = if high > 0.0 {
                    high * 1e-12
                } else {
                    f64::MIN_POSITIVE
                };
                low = floor;
                clamped = true;
            }
            if high <= 0.0 {
                high = low * 10.0;
                clamped = true;
            }
        }
        if let Some(limit) = self.lowest_defined() {
            if high <= limit {
                high = limit + (limit.abs().max(1.0) * 1e-6);
                clamped = true;
            }
        }
        ((low, high), clamped)
    }

    /// The layout span of a physical range, or `None` when neither end has a
    /// value.
    pub fn forward_range(&self, low: f64, high: f64) -> Option<(f64, f64)> {
        let (low, high) = self.clamp_domain(low, high).0;
        let a = self.forward(low)?;
        let b = self.forward(high)?;
        if a.is_finite() && b.is_finite() {
            Some((a.min(b), a.max(b)))
        } else {
            None
        }
    }

    /// The physical range behind a layout range.
    pub fn inverse_range(&self, low: f64, high: f64) -> (f64, f64) {
        let a = self.inverse(low);
        let b = self.inverse(high);
        (a.min(b), a.max(b))
    }

    /// A short name for the scale, for axis captions and the inspector.
    pub fn label(&self) -> &'static str {
        match self {
            Scale::Linear => "linear",
            Scale::Logarithmic => "log",
            Scale::SymmetricLogarithmic { .. } => "symlog",
            Scale::SplitLinearLog { .. } => "linear/log",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_round_trip(scale: Scale, values: &[f64]) {
        for value in values {
            let Some(transformed) = scale.forward(*value) else {
                continue;
            };
            let back = scale.inverse(transformed);
            let tolerance = value.abs() * 1e-9 + 1e-12;
            assert!(
                (back - value).abs() <= tolerance,
                "{scale:?}: {value} → {transformed} → {back}"
            );
        }
    }

    #[test]
    fn every_scale_round_trips_through_its_transform() {
        let values = [-1000.0, -1.5, -0.001, 0.0, 0.001, 0.5, 1.5, 1000.0];
        assert_round_trip(Scale::Linear, &values);
        assert_round_trip(
            Scale::SymmetricLogarithmic { linear_width: 1.0 },
            &values,
        );
        assert_round_trip(Scale::SplitLinearLog { break_at: 1.0 }, &values);
        assert_round_trip(Scale::Logarithmic, &[0.001, 0.5, 1.0, 1000.0]);
    }

    #[test]
    fn every_scale_is_monotonic() {
        let scales = [
            Scale::Linear,
            Scale::Logarithmic,
            Scale::SymmetricLogarithmic { linear_width: 0.5 },
            Scale::SplitLinearLog { break_at: 2.0 },
        ];
        for scale in scales {
            let mut previous = f64::NEG_INFINITY;
            for step in -400..400 {
                let value = step as f64 * 0.25;
                if let Some(transformed) = scale.forward(value) {
                    assert!(
                        transformed > previous,
                        "{scale:?} went backwards at {value}: {transformed} after {previous}"
                    );
                    previous = transformed;
                }
            }
        }
    }

    #[test]
    fn a_logarithmic_axis_refuses_coordinates_it_cannot_represent() {
        let scale = Scale::Logarithmic;
        assert!(scale.forward(0.0).is_none());
        assert!(scale.forward(-1.0).is_none());
        assert!(scale.forward(1.0).is_some());
        assert!(!scale.is_defined_at(0.0));
    }

    #[test]
    fn a_logarithmic_axis_clamps_its_domain_and_says_that_it_did() {
        let scale = Scale::Logarithmic;
        let ((low, high), clamped) = scale.clamp_domain(-5.0, 100.0);
        assert!(clamped);
        assert!(low > 0.0);
        assert_eq!(high, 100.0);

        let ((_, _), clean) = scale.clamp_domain(0.1, 100.0);
        assert!(!clean, "a positive range needs no clamping");

        let ((low, high), clamped) = scale.clamp_domain(-5.0, -1.0);
        assert!(clamped);
        assert!(high > low);
        assert!(low > 0.0);
    }

    #[test]
    fn the_symmetric_logarithmic_scale_is_continuous_at_its_linear_width() {
        let width = 2.0;
        let scale = Scale::SymmetricLogarithmic { linear_width: width };
        for value in [width, -width] {
            let at = scale.forward(value).unwrap();
            let just_inside = scale.forward(value * 0.999).unwrap();
            let just_outside = scale.forward(value * 1.001).unwrap();
            assert!((at - value).abs() < 1e-12);
            assert!((at - just_inside).abs() < 0.01);
            assert!((at - just_outside).abs() < 0.01);
        }
        // Zero maps to zero, which is the whole point of the scale.
        assert_eq!(scale.forward(0.0), Some(0.0));
    }

    #[test]
    fn the_split_scale_keeps_slope_one_across_the_break() {
        let break_at = 3.0;
        let scale = Scale::SplitLinearLog { break_at };
        // Differentiate numerically on both sides of the break.
        let h = 1e-6;
        let below = (scale.forward(break_at).unwrap() - scale.forward(break_at - h).unwrap()) / h;
        let above = (scale.forward(break_at + h).unwrap() - scale.forward(break_at).unwrap()) / h;
        assert!((below - 1.0).abs() < 1e-3, "slope below the break is {below}");
        assert!((above - 1.0).abs() < 1e-3, "slope above the break is {above}");
    }

    #[test]
    fn parameters_that_would_make_a_map_undefined_are_refused() {
        assert!(Scale::new(AxisScale::SplitLinearLog { break_at: 0.0 }).is_err());
        assert!(Scale::new(AxisScale::SplitLinearLog { break_at: -1.0 }).is_err());
        assert!(Scale::new(AxisScale::SymmetricLogarithmic { linear_width: 0.0 }).is_err());
        assert!(Scale::new(AxisScale::SymmetricLogarithmic { linear_width: f64::NAN }).is_err());
        assert!(Scale::new(AxisScale::Linear).is_ok());
    }

    #[test]
    fn a_range_mapping_survives_both_directions() {
        let scale = Scale::SymmetricLogarithmic { linear_width: 1.0 };
        let (low, high) = scale.forward_range(-100.0, 100.0).unwrap();
        let (back_low, back_high) = scale.inverse_range(low, high);
        assert!((back_low + 100.0).abs() < 1e-9);
        assert!((back_high - 100.0).abs() < 1e-9);
    }

    #[test]
    fn a_reversed_pair_of_limits_still_produces_an_ordered_span() {
        let scale = Scale::Linear;
        let forward = scale.forward_range(10.0, -10.0).unwrap();
        assert_eq!(forward, (-10.0, 10.0));
    }
}
