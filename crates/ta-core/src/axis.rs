//! Physical coordinate axes.
//!
//! The axis, not the array index, is authoritative. Every lookup in this
//! module answers in the axis' own physical coordinate and reports which
//! sampled coordinate it actually used, so a caller can never quietly read the
//! wrong column.

use crate::error::{CoreError, Result};
use crate::units::{AxisKind, AxisUnit};
use serde::{Deserialize, Serialize};

/// Which way an axis runs. Both are first-class: descending wavelength axes are
/// common in real spectrometers and are never silently reordered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ordering {
    Ascending,
    Descending,
}

/// How a value between two sampled coordinates is obtained.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interpolation {
    /// Straight lines between samples. The default, because it cannot invent
    /// structure the measurement does not have.
    Linear,
    /// Monotone cubic (Fritsch–Carlson), used where a smooth correction curve
    /// is wanted without overshoot.
    ShapePreserving,
}

/// What a requested coordinate means.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleMode {
    /// The closest sampled coordinate. Never fails: near an edge it returns the
    /// boundary sample and says so through [`Sample::is_exact`].
    Nearest,
    /// Linear interpolation between the two bracketing samples. Refuses
    /// coordinates outside the sampled range instead of extrapolating.
    Interpolated,
    /// Every sample within `± half_width` of the coordinate, equally weighted.
    AverageWindow { half_width: f64 },
    /// The `count` samples nearest the coordinate, equally weighted.
    AverageChannels { count: usize },
}

/// The axis indices a requested coordinate resolved to, and how much each one
/// contributes.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    /// The coordinate the caller asked for.
    pub coordinate: f64,
    /// Indices into the axis, in ascending index order.
    pub indices: Vec<usize>,
    /// Weights in the same order as `indices`. Sums to 1.
    pub weights: Vec<f64>,
    /// The sampled coordinate at each index.
    pub coordinates: Vec<f64>,
    pub mode: SampleMode,
}

impl Sample {
    /// Whether the sample landed on the requested coordinate exactly.
    pub fn is_exact(&self) -> bool {
        self.indices.len() == 1 && self.coordinates[0] == self.coordinate
    }

    /// The physical span the sample covers, which for an averaged window is the
    /// width the plot label must report.
    pub fn span(&self) -> (f64, f64) {
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for value in &self.coordinates {
            lo = lo.min(*value);
            hi = hi.max(*value);
        }
        (lo, hi)
    }

    /// A short, factual description of what was actually read.
    pub fn label(&self, axis: &Axis) -> String {
        let unit = axis.unit().symbol();
        match self.mode {
            SampleMode::Nearest => {
                if self.is_exact() {
                    format!("{:.4} {}", self.coordinate, unit)
                        .trim_end()
                        .to_string()
                } else {
                    format!(
                        "{:.4} {} (nearest sample {:.4} {})",
                        self.coordinate, unit, self.coordinates[0], unit
                    )
                    .replace("  ", " ")
                }
            }
            SampleMode::Interpolated => format!("{:.4} {} (interpolated)", self.coordinate, unit),
            SampleMode::AverageWindow { half_width } => {
                let (lo, hi) = self.span();
                format!(
                    "{:.4} ± {:.4} {} ({} samples, {:.4}–{:.4} {})",
                    self.coordinate,
                    half_width,
                    unit,
                    self.indices.len(),
                    lo,
                    hi,
                    unit
                )
            }
            SampleMode::AverageChannels { count } => {
                let (lo, hi) = self.span();
                format!(
                    "{:.4} {} ({} channels, {:.4}–{:.4} {})",
                    self.coordinate,
                    unit,
                    count,
                    lo,
                    hi,
                    unit
                )
            }
        }
    }
}

/// A monotonic physical coordinate axis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Axis {
    kind: AxisKind,
    unit: AxisUnit,
    name: String,
    values: Vec<f64>,
}

impl Axis {
    /// Builds an axis, rejecting anything a lookup could misread: empty,
    /// non-finite, duplicate or non-monotonic coordinates.
    pub fn new(
        kind: AxisKind,
        unit: AxisUnit,
        name: impl Into<String>,
        values: Vec<f64>,
    ) -> Result<Self> {
        let axis = Self {
            kind,
            unit,
            name: name.into(),
            values,
        };
        axis.validate()?;
        Ok(axis)
    }

    /// Builds an index axis `0..len`, used only where a caller explicitly asks
    /// for an index view.
    pub fn index(len: usize) -> Result<Self> {
        Self::new(
            AxisKind::Index,
            AxisUnit::Unitless,
            "Index",
            (0..len).map(|i| i as f64).collect(),
        )
    }

    pub fn validate(&self) -> Result<()> {
        let label = self.axis_label();
        if self.values.is_empty() {
            return Err(CoreError::EmptyAxis { axis: label });
        }
        for (index, value) in self.values.iter().enumerate() {
            if !value.is_finite() {
                return Err(CoreError::NonFiniteCoordinate {
                    axis: label,
                    index,
                    value: *value,
                });
            }
        }
        for index in 1..self.values.len() {
            let previous = self.values[index - 1];
            let current = self.values[index];
            if previous == current {
                return Err(CoreError::DuplicateCoordinate {
                    axis: label,
                    value: current,
                    first: index - 1,
                    second: index,
                });
            }
            let ascending = self.ordering() == Ordering::Ascending;
            let step_is_forward = current > previous;
            if step_is_forward != ascending {
                return Err(CoreError::invalid(format!(
                    "axis {label} is not monotonic: index {index} ({current}) breaks the \
                     {} order set at index 0",
                    if ascending { "ascending" } else { "descending" }
                )));
            }
        }
        Ok(())
    }

    fn axis_label(&self) -> &'static str {
        // Static labels keep the error type allocation-free; the kind is what a
        // caller needs in order to know which axis to look at.
        match self.kind {
            AxisKind::Wavelength => "wavelength",
            AxisKind::Delay => "delay",
            AxisKind::Wavenumber => "wavenumber",
            AxisKind::Energy => "energy",
            AxisKind::Index => "index",
            AxisKind::Custom => "custom",
        }
    }

    pub fn kind(&self) -> AxisKind {
        self.kind
    }

    pub fn unit(&self) -> AxisUnit {
        self.unit
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Display name with its unit, for axis labels and tree rows.
    pub fn labelled_name(&self) -> String {
        match self.unit.symbol() {
            "" => self.name.clone(),
            symbol => format!("{} ({})", self.name, symbol),
        }
    }

    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    pub fn values(&self) -> &[f64] {
        &self.values
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn ordering(&self) -> Ordering {
        // Safe without a length check: `validate` rejects empty axes, and the
        // only constructor that can produce one is deserialization, which
        // `ordered` callers re-validate.
        if self.values.len() < 2 || self.values[1] > self.values[0] {
            Ordering::Ascending
        } else {
            Ordering::Descending
        }
    }

    pub fn is_ascending(&self) -> bool {
        self.ordering() == Ordering::Ascending
    }

    pub fn first(&self) -> f64 {
        self.values[0]
    }

    pub fn last(&self) -> f64 {
        self.values[self.values.len() - 1]
    }

    /// Inclusive sampled range, low to high regardless of axis direction.
    pub fn range(&self) -> (f64, f64) {
        if self.is_ascending() {
            (self.first(), self.last())
        } else {
            (self.last(), self.first())
        }
    }

    pub fn contains(&self, value: f64) -> bool {
        let (min, max) = self.range();
        value >= min && value <= max
    }

    /// The uniform step between samples, or `None` when the axis is non-uniform.
    ///
    /// TA delay axes are routinely non-uniform, which is why every consumer has
    /// to state what it does when this is `None`.
    pub fn uniform_step(&self) -> Option<f64> {
        if self.values.len() < 2 {
            return Some(0.0);
        }
        let step = (self.last() - self.first()) / (self.values.len() - 1) as f64;
        let tolerance = step.abs() * 1e-9 + f64::EPSILON;
        for window in self.values.windows(2) {
            if (window[1] - window[0] - step).abs() > tolerance {
                return None;
            }
        }
        Some(step)
    }

    pub fn is_uniform(&self) -> bool {
        self.uniform_step().is_some()
    }

    /// Index and coordinate of the sampled point closest to `value`.
    pub fn nearest(&self, value: f64) -> (usize, f64) {
        let mut best = 0;
        let mut best_distance = f64::INFINITY;
        for (index, sample) in self.values.iter().enumerate() {
            let distance = (sample - value).abs();
            if distance < best_distance {
                best_distance = distance;
                best = index;
            }
        }
        (best, self.values[best])
    }

    /// The two indices bracketing `value` and how far between them it sits.
    ///
    /// Returns an error outside the sampled range; nothing in this crate
    /// extrapolates without being told to.
    pub fn bracket(&self, value: f64) -> Result<(usize, usize, f64)> {
        let (min, max) = self.range();
        if !(value >= min && value <= max) {
            return Err(CoreError::OutOfRange {
                axis: self.axis_label(),
                value,
                min,
                max,
            });
        }
        if self.values.len() == 1 {
            return Ok((0, 0, 0.0));
        }
        let ascending = self.is_ascending();
        // Binary search on the physical coordinate, not the index, so a
        // descending axis needs no special case beyond the comparison direction.
        let mut low = 0usize;
        let mut high = self.values.len() - 1;
        while high - low > 1 {
            let mid = low + (high - low) / 2;
            let past = if ascending {
                self.values[mid] <= value
            } else {
                self.values[mid] >= value
            };
            if past {
                low = mid;
            } else {
                high = mid;
            }
        }
        let (left, right) = (self.values[low], self.values[high]);
        let span = right - left;
        let fraction = if span == 0.0 { 0.0 } else { (value - left) / span };
        Ok((low, high, fraction))
    }

    /// Resolves a requested coordinate into axis indices and weights.
    pub fn sample(&self, mode: SampleMode, value: f64) -> Result<Sample> {
        if !value.is_finite() {
            return Err(CoreError::invalid(format!(
                "cannot sample axis {} at {value}",
                self.axis_label()
            )));
        }
        match mode {
            SampleMode::Nearest => {
                let (index, coordinate) = self.nearest(value);
                Ok(Sample {
                    coordinate: value,
                    indices: vec![index],
                    weights: vec![1.0],
                    coordinates: vec![coordinate],
                    mode,
                })
            }
            SampleMode::Interpolated => {
                let (left, right, fraction) = self.bracket(value)?;
                if left == right {
                    return Ok(Sample {
                        coordinate: value,
                        indices: vec![left],
                        weights: vec![1.0],
                        coordinates: vec![self.values[left]],
                        mode,
                    });
                }
                Ok(Sample {
                    coordinate: value,
                    indices: vec![left, right],
                    weights: vec![1.0 - fraction, fraction],
                    coordinates: vec![self.values[left], self.values[right]],
                    mode,
                })
            }
            SampleMode::AverageWindow { half_width } => {
                if !(half_width.is_finite() && half_width >= 0.0) {
                    return Err(CoreError::invalid(format!(
                        "averaging half-width {half_width} is not a width"
                    )));
                }
                let (min, max) = self.range();
                let lo = value - half_width;
                let hi = value + half_width;
                if hi < min || lo > max {
                    return Err(CoreError::OutOfRange {
                        axis: self.axis_label(),
                        value,
                        min,
                        max,
                    });
                }
                let mut indices = Vec::new();
                for (index, sample) in self.values.iter().enumerate() {
                    if *sample >= lo && *sample <= hi {
                        indices.push(index);
                    }
                }
                // An explicit interval may legitimately cover no sample on a
                // non-uniform axis; that is a mistake the caller has to see.
                if indices.is_empty() {
                    return Err(CoreError::EmptyWindow {
                        axis: self.axis_label(),
                        center: value,
                    });
                }
                let weight = 1.0 / indices.len() as f64;
                Ok(Sample {
                    coordinate: value,
                    coordinates: indices.iter().map(|i| self.values[*i]).collect(),
                    weights: vec![weight; indices.len()],
                    indices,
                    mode,
                })
            }
            SampleMode::AverageChannels { count } => {
                if count == 0 {
                    return Err(CoreError::invalid("averaging 0 channels reads nothing"));
                }
                let (centre, _) = self.nearest(value);
                let count = count.min(self.values.len());
                let first = centre.saturating_sub((count - 1) / 2);
                let last = (first + count).min(self.values.len());
                let first = last - count;
                let indices: Vec<usize> = (first..last).collect();
                let weight = 1.0 / count as f64;
                Ok(Sample {
                    coordinate: value,
                    coordinates: indices.iter().map(|i| self.values[*i]).collect(),
                    weights: vec![weight; count],
                    indices,
                    mode,
                })
            }
        }
    }

    /// A comma-free one-line summary for status bars and test failures.
    pub fn summary(&self) -> String {
        let (min, max) = self.range();
        format!(
            "{}: {} points, {min}..={max} {}, {} order{}",
            self.name,
            self.values.len(),
            self.unit.symbol(),
            if self.is_ascending() {
                "ascending"
            } else {
                "descending"
            },
            if self.is_uniform() {
                ""
            } else {
                ", non-uniform"
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wavelength_axis() -> Axis {
        Axis::new(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            "Wavelength",
            vec![500.0, 510.0, 520.0, 530.0],
        )
        .unwrap()
    }

    fn descending_axis() -> Axis {
        Axis::new(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            "Wavelength",
            vec![530.0, 520.0, 510.0, 500.0],
        )
        .unwrap()
    }

    fn uneven_delay_axis() -> Axis {
        Axis::new(
            AxisKind::Delay,
            AxisUnit::Picosecond,
            "Delay",
            vec![-100.0, -10.0, -1.0, 0.0, 1.0, 10.0, 100.0],
        )
        .unwrap()
    }

    #[test]
    fn rejects_empty_non_finite_duplicate_and_unordered_axes() {
        let empty = Axis::new(AxisKind::Wavelength, AxisUnit::Nanometer, "w", vec![]);
        assert!(matches!(empty, Err(CoreError::EmptyAxis { .. })));

        let nan = Axis::new(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            "w",
            vec![1.0, f64::NAN],
        );
        assert!(matches!(nan, Err(CoreError::NonFiniteCoordinate { .. })));

        let duplicate = Axis::new(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            "w",
            vec![1.0, 2.0, 2.0, 3.0],
        );
        assert!(matches!(
            duplicate,
            Err(CoreError::DuplicateCoordinate { first: 1, second: 2, .. })
        ));

        let unordered = Axis::new(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            "w",
            vec![1.0, 3.0, 2.0],
        );
        assert!(matches!(unordered, Err(CoreError::Invalid(_))));
    }

    #[test]
    fn descending_axes_are_first_class_not_reordered() {
        let axis = descending_axis();
        assert_eq!(axis.ordering(), Ordering::Descending);
        assert_eq!(axis.range(), (500.0, 530.0));
        assert_eq!(axis.nearest(521.0), (1, 520.0));

        // Bracketing works in physical coordinates whichever way the axis runs.
        let (left, right, fraction) = axis.bracket(515.0).unwrap();
        assert_eq!((left, right), (1, 2));
        assert!((fraction - 0.5).abs() < 1e-12);
    }

    #[test]
    fn interpolated_sampling_weights_sum_to_one() {
        let axis = wavelength_axis();
        let sample = axis
            .sample(SampleMode::Interpolated, 505.0)
            .expect("505 nm is inside the axis");
        assert_eq!(sample.indices, vec![0, 1]);
        assert!((sample.weights[0] - 0.5).abs() < 1e-12);
        assert!((sample.weights.iter().sum::<f64>() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn interpolation_refuses_to_extrapolate_while_nearest_does_not() {
        let axis = wavelength_axis();
        assert!(matches!(
            axis.sample(SampleMode::Interpolated, 495.0),
            Err(CoreError::OutOfRange { .. })
        ));

        // Nearest clamps to the boundary sample and reports that it did.
        let sample = axis.sample(SampleMode::Nearest, 495.0).unwrap();
        assert_eq!(sample.indices, vec![0]);
        assert!(!sample.is_exact());
        assert_eq!(sample.coordinates, vec![500.0]);
        assert!(sample.label(&axis).contains("nearest sample"));
    }

    #[test]
    fn average_window_covers_the_requested_interval_on_an_uneven_axis() {
        let axis = uneven_delay_axis();
        let sample = axis
            .sample(SampleMode::AverageWindow { half_width: 2.0 }, 0.0)
            .unwrap();
        assert_eq!(sample.indices, vec![2, 3, 4]);
        assert!((sample.weights.iter().sum::<f64>() - 1.0).abs() < 1e-12);

        // A window that lands between samples on a non-uniform axis is an
        // error, not a silent empty read.
        assert!(matches!(
            axis.sample(SampleMode::AverageWindow { half_width: 0.1 }, 5.0),
            Err(CoreError::EmptyWindow { .. })
        ));
    }

    #[test]
    fn average_channels_stays_inside_the_axis_at_both_ends() {
        let axis = wavelength_axis();
        let low = axis
            .sample(SampleMode::AverageChannels { count: 3 }, 500.0)
            .unwrap();
        assert_eq!(low.indices, vec![0, 1, 2]);

        let high = axis
            .sample(SampleMode::AverageChannels { count: 3 }, 530.0)
            .unwrap();
        assert_eq!(high.indices, vec![1, 2, 3]);

        // Asking for more channels than exist uses what exists rather than
        // running off the end.
        let all = axis
            .sample(SampleMode::AverageChannels { count: 99 }, 520.0)
            .unwrap();
        assert_eq!(all.indices.len(), 4);
    }

    #[test]
    fn negative_delays_are_sampled_normally() {
        let axis = uneven_delay_axis();
        let sample = axis
            .sample(SampleMode::Interpolated, -55.0)
            .unwrap();
        assert_eq!(sample.indices, vec![0, 1]);
        assert!((sample.weights[0] - 0.5).abs() < 1e-12);
        assert!(!axis.is_uniform());
    }

    #[test]
    fn a_single_point_axis_brackets_itself() {
        let axis = Axis::new(
            AxisKind::Delay,
            AxisUnit::Picosecond,
            "Delay",
            vec![0.0],
        )
        .unwrap();
        assert_eq!(axis.bracket(0.0).unwrap(), (0, 0, 0.0));
        assert!(axis.bracket(0.1).is_err());
    }
}
