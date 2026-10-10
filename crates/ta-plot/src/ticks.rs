//! Tick positions and labels.
//!
//! Generated here rather than left to a renderer, because the scales this
//! application offers — symmetric-logarithmic and split linear/logarithmic —
//! need ticks that a plain linear or logarithmic axis generator cannot produce,
//! and because `spec.md` §17.5 wants control over the tick count, the direction
//! and the label format.
//!
//! Every function returns positions in *physical* coordinates; the caller maps
//! them with its scale.

use crate::scale::Scale;

/// Rounds a span to a 1, 2 or 5 times a power of ten.
fn nice_step(raw: f64) -> f64 {
    if !(raw.is_finite() && raw > 0.0) {
        return 1.0;
    }
    let exponent = raw.log10().floor();
    let magnitude = 10f64.powf(exponent);
    let normalised = raw / magnitude;
    let factor = if normalised <= 1.0 {
        1.0
    } else if normalised <= 2.0 {
        2.0
    } else if normalised <= 5.0 {
        5.0
    } else {
        10.0
    };
    factor * magnitude
}

/// Evenly spaced, human-readable ticks across a linear range.
pub fn linear_ticks(low: f64, high: f64, target: usize) -> Vec<f64> {
    let (low, high) = (low.min(high), low.max(high));
    if !(low.is_finite() && high.is_finite()) {
        return Vec::new();
    }
    let target = target.max(2);
    let span = high - low;
    if span <= 0.0 {
        return vec![low];
    }
    let step = nice_step(span / target as f64);
    let first = (low / step).ceil() * step;
    let mut ticks = Vec::new();
    let mut value = first;
    // A hard cap keeps a pathological range from producing a million ticks.
    while value <= high + step * 1e-9 && ticks.len() < 4096 {
        // Snap the tiny floating-point residue off round numbers.
        let snapped = (value / step).round() * step;
        ticks.push(if snapped.abs() < step * 1e-9 { 0.0 } else { snapped });
        value += step;
    }
    ticks
}

/// Ticks across a range in layout space, which is what a symmetric-logarithmic
/// or split axis actually needs: equal visual spacing means equal steps in the
/// transformed coordinate.
pub fn ticks_for_scale(scale: Scale, low: f64, high: f64, target: usize) -> Vec<f64> {
    let Some((t_low, t_high)) = scale.forward_range(low, high) else {
        return Vec::new();
    };
    let mut physical: Vec<f64> = match scale {
        // A logarithmic axis wants its decades, not 1-2-5 steps.
        Scale::Logarithmic => decades(t_low, t_high)
            .into_iter()
            .map(|t| scale.inverse(t))
            .collect(),
        _ => {
            // A split axis is two different regimes, so each side gets the tick
            // generator that suits it and the round numbers stay round.
            if let Scale::SplitLinearLog { break_at } = scale {
                let mut ticks = linear_ticks(low.min(break_at), break_at, target);
                for decade in decades(break_at.ln(), t_high) {
                    ticks.push(scale.inverse(decade));
                }
                ticks.retain(|value| *value >= low.min(high) && *value <= low.max(high));
                ticks.sort_by(f64::total_cmp);
                ticks.dedup_by(|a, b| (*a - *b).abs() <= b.abs() * 1e-12 + 1e-300);
                return ticks;
            }
            linear_ticks(t_low, t_high, target)
                .into_iter()
                .map(|t| scale.inverse(t))
                .collect()
        }
    };
    physical.retain(|value| *value >= low.min(high) && *value <= low.max(high));
    physical.sort_by(f64::total_cmp);
    physical.dedup_by(|a, b| (*a - *b).abs() <= b.abs() * 1e-12 + 1e-300);
    physical
}

/// Log decades inside a layout range, rounded to integers.
fn decades(low: f64, high: f64) -> Vec<f64> {
    let (low, high) = (low.min(high), low.max(high));
    if !(low.is_finite() && high.is_finite()) {
        return Vec::new();
    }
    let first = low.ceil();
    let last = high.floor();
    if last < first {
        // Less than one decade visible: fall back to a linear split of the
        // layout span, which keeps the labels meaningful.
        return linear_ticks(low, high, 4);
    }
    let mut ticks = Vec::new();
    let mut exponent = first;
    while exponent <= last && ticks.len() < 512 {
        ticks.push(exponent);
        exponent += 1.0;
    }
    ticks
}

/// A label for a tick, short enough to read at 8 pt and never scientific
/// notation for the magnitudes a TA measurement uses.
pub fn format_tick(value: f64, scale: Scale) -> String {
    if !value.is_finite() {
        return String::new();
    }
    // A symmetric-logarithmic or split axis passes through zero, so the digits
    // after the point depend on how far from zero the tick is.
    let magnitude = value.abs();
    if magnitude == 0.0 {
        return "0".to_string();
    }
    let precision = match scale {
        _ if magnitude >= 1000.0 => 0,
        _ if magnitude >= 100.0 => 0,
        _ if magnitude >= 10.0 => 1,
        _ if magnitude >= 1.0 => 2,
        _ if magnitude >= 0.1 => 3,
        _ if magnitude >= 0.01 => 4,
        _ => 5,
    };
    let text = format!("{value:.precision$}");
    // Trim trailing zeros so "1.00" reads as "1".
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    };
    if text == "-0" {
        "0".to_string()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_core::AxisScale;

    #[test]
    fn nice_steps_are_one_two_or_five_times_a_power_of_ten() {
        assert_eq!(nice_step(1.0), 1.0);
        assert_eq!(nice_step(1.5), 2.0);
        assert_eq!(nice_step(3.0), 5.0);
        assert_eq!(nice_step(7.0), 10.0);
        assert_eq!(nice_step(0.023), 0.02 + 1e-12 - 1e-12);
        assert!(nice_step(0.023) > 0.019 && nice_step(0.023) < 0.026);
    }

    #[test]
    fn linear_ticks_land_on_round_numbers_inside_the_range() {
        let ticks = linear_ticks(0.0, 10.0, 5);
        assert_eq!(ticks, vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0]);
        let ticks = linear_ticks(-1.0, 1.0, 4);
        assert!(ticks.contains(&0.0));
        assert!(*ticks.first().unwrap() >= -1.0);
        assert!(*ticks.last().unwrap() <= 1.0);
    }

    #[test]
    fn linear_ticks_survive_a_degenerate_range() {
        assert_eq!(linear_ticks(5.0, 5.0, 5), vec![5.0]);
        assert!(linear_ticks(f64::NAN, 1.0, 5).is_empty());
    }

    #[test]
    fn a_logarithmic_axis_ticks_on_decades() {
        let scale = Scale::Logarithmic;
        let ticks = ticks_for_scale(scale, 1.0, 1000.0, 4);
        assert_eq!(ticks, vec![1.0, 10.0, 100.0, 1000.0]);
    }

    #[test]
    fn a_symmetric_logarithmic_axis_ticks_through_zero() {
        let scale = Scale::SymmetricLogarithmic { linear_width: 1.0 };
        let ticks = ticks_for_scale(scale, -100.0, 100.0, 5);
        assert!(ticks.contains(&0.0), "a symlog axis must be able to label zero");
        assert!(ticks.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(ticks.len() >= 5);
        // Ticks are spread evenly in layout space, which is what makes a symlog
        // axis readable.
        let mut layout: Vec<f64> = ticks.iter().filter_map(|t| scale.forward(*t)).collect();
        layout.sort_by(f64::total_cmp);
        let gaps: Vec<f64> = layout.windows(2).map(|pair| pair[1] - pair[0]).collect();
        let first = gaps[0];
        for gap in &gaps {
            assert!(
                (gap - first).abs() <= first * 0.05,
                "uneven spacing: {gaps:?}"
            );
        }
    }

    #[test]
    fn a_split_axis_keeps_round_numbers_below_the_break() {
        let scale = Scale::SplitLinearLog { break_at: 10.0 };
        let ticks = ticks_for_scale(scale, 0.0, 1000.0, 5);
        assert!(ticks.contains(&5.0), "ticks below the break: {ticks:?}");
        assert!(ticks.contains(&10.0));
        assert!(ticks.contains(&100.0), "decades above the break: {ticks:?}");
        assert!(ticks.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn ticks_stay_inside_the_requested_range() {
        for scale in [
            Scale::Linear,
            Scale::Logarithmic,
            Scale::SymmetricLogarithmic { linear_width: 0.5 },
            Scale::SplitLinearLog { break_at: 2.0 },
        ] {
            let ticks = ticks_for_scale(scale, 1.0, 100.0, 6);
            for tick in &ticks {
                assert!(
                    *tick >= 1.0 - 1e-9 && *tick <= 100.0 + 1e-9,
                    "{scale:?} produced a tick outside the range: {tick}"
                );
            }
        }
    }

    #[test]
    fn a_scale_with_no_definition_has_no_ticks() {
        assert!(ticks_for_scale(Scale::Logarithmic, -10.0, 0.0, 5).is_empty());
        assert!(Scale::new(AxisScale::Linear).is_ok());
    }

    #[test]
    fn labels_are_short_but_keep_the_digits_that_matter() {
        assert_eq!(format_tick(0.0, Scale::Linear), "0");
        assert_eq!(format_tick(-0.0, Scale::Linear), "0");
        assert_eq!(format_tick(1.0, Scale::Linear), "1");
        assert_eq!(format_tick(1.5, Scale::Linear), "1.5");
        assert_eq!(format_tick(500.0, Scale::Linear), "500");
        assert_eq!(format_tick(-1079.7752, Scale::Linear), "-1080");
        assert_eq!(format_tick(0.05, Scale::Linear), "0.05");
        assert_eq!(format_tick(0.0005, Scale::Linear), "0.0005");
        assert_eq!(format_tick(f64::NAN, Scale::Linear), "");
    }
}
