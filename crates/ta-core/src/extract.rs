//! Extraction of kinetic traces and transient spectra.
//!
//! A kinetic trace is taken *at a wavelength* and runs over delay; a transient
//! spectrum is taken *at a delay* and runs over wavelength. Both are the same
//! operation on different axes: the requested physical coordinate resolves to
//! axis indices and weights, and the matrix is combined along the other axis
//! with them.
//!
//! Averaging over a window renormalises over the samples that exist, so a
//! masked point never drags a curve towards zero, and a window with nothing
//! finite in it stays missing rather than becoming a number.

use crate::axis::Sample;
use crate::dataset::TaDataset;
use crate::error::Result;
use crate::ids::VersionId;
use crate::selection::{SpectrumSelection, TraceSelection};
use crate::units::SignalUnit;
use ndarray::Array2;

/// Signal against delay, taken at one wavelength (or an averaged band of them).
#[derive(Clone, Debug, PartialEq)]
pub struct Trace {
    /// What was actually read, for the plot label and the fit record.
    pub label: String,
    pub delay: Vec<f64>,
    pub signal: Vec<f64>,
    pub unit: SignalUnit,
    pub version: VersionId,
    /// The wavelength sample this trace was taken from.
    pub sampled: Sample,
}

/// Signal against wavelength, taken at one delay (or an averaged window of them).
#[derive(Clone, Debug, PartialEq)]
pub struct Spectrum {
    pub label: String,
    pub wavelength: Vec<f64>,
    pub signal: Vec<f64>,
    pub unit: SignalUnit,
    pub version: VersionId,
    /// The delay sample this spectrum was taken from.
    pub sampled: Sample,
}

impl Trace {
    pub fn len(&self) -> usize {
        self.signal.len()
    }

    pub fn is_empty(&self) -> bool {
        self.signal.is_empty()
    }

    /// Finite signal range, ignoring missing values.
    pub fn range(&self) -> Option<(f64, f64)> {
        finite_range(self.signal.iter().copied())
    }

    /// The delays and signals inside `range`, which is what a fit consumes.
    pub fn within(&self, range: (f64, f64)) -> Trace {
        let mut filtered = self.clone();
        filtered.delay.clear();
        filtered.signal.clear();
        for (delay, signal) in self.delay.iter().zip(&self.signal) {
            if *delay >= range.0 && *delay <= range.1 {
                filtered.delay.push(*delay);
                filtered.signal.push(*signal);
            }
        }
        filtered
    }

    /// Pairs of delay and signal that are both finite: the points a fit can
    /// actually use.
    pub fn finite_points(&self) -> Vec<(f64, f64)> {
        self.delay
            .iter()
            .zip(&self.signal)
            .filter(|(delay, signal)| delay.is_finite() && signal.is_finite())
            .map(|(delay, signal)| (*delay, *signal))
            .collect()
    }
}

impl Spectrum {
    pub fn len(&self) -> usize {
        self.signal.len()
    }

    pub fn is_empty(&self) -> bool {
        self.signal.is_empty()
    }

    pub fn range(&self) -> Option<(f64, f64)> {
        finite_range(self.signal.iter().copied())
    }
}

fn finite_range(values: impl Iterator<Item = f64>) -> Option<(f64, f64)> {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    let mut seen = false;
    for value in values {
        if value.is_finite() {
            seen = true;
            min = min.min(value);
            max = max.max(value);
        }
    }
    seen.then_some((min, max))
}

/// Reduces the sampled rows of one matrix column to a single value.
fn weighted_over_rows(matrix: &Array2<f64>, sample: &Sample, column: usize) -> f64 {
    let mut total = 0.0;
    let mut used = 0.0;
    for (index, weight) in sample.indices.iter().zip(&sample.weights) {
        let value = matrix[[*index, column]];
        if value.is_finite() {
            total += weight * value;
            used += weight;
        }
    }
    if used > 0.0 {
        total / used
    } else {
        f64::NAN
    }
}

/// Reduces the sampled columns of one matrix row to a single value.
fn weighted_over_columns(matrix: &Array2<f64>, sample: &Sample, row: usize) -> f64 {
    let mut total = 0.0;
    let mut used = 0.0;
    for (index, weight) in sample.indices.iter().zip(&sample.weights) {
        let value = matrix[[row, *index]];
        if value.is_finite() {
            total += weight * value;
            used += weight;
        }
    }
    if used > 0.0 {
        total / used
    } else {
        f64::NAN
    }
}

/// Extracts a kinetic trace at the selection's wavelength.
pub fn kinetic_trace(dataset: &TaDataset, selection: &TraceSelection) -> Result<Trace> {
    let (wavelength_axis, delay_axis) = dataset.axes(selection.version)?;
    let sampled = wavelength_axis.sample(selection.wavelength.mode, selection.wavelength.coordinate)?;
    let matrix = dataset.matrix(selection.version)?;
    let unit = dataset.unit(selection.version)?;
    let label = sampled.label(wavelength_axis);

    let signal: Vec<f64> = (0..matrix.ncols())
        .map(|column| weighted_over_rows(matrix, &sampled, column))
        .collect();

    Ok(Trace {
        label,
        delay: delay_axis.values().to_vec(),
        signal,
        unit,
        version: selection.version,
        sampled,
    })
}

/// Extracts a transient spectrum at the selection's delay.
pub fn transient_spectrum(dataset: &TaDataset, selection: &SpectrumSelection) -> Result<Spectrum> {
    let (wavelength_axis, delay_axis) = dataset.axes(selection.version)?;
    let sampled = delay_axis.sample(selection.delay.mode, selection.delay.coordinate)?;
    let matrix = dataset.matrix(selection.version)?;
    let unit = dataset.unit(selection.version)?;
    let label = sampled.label(delay_axis);

    let signal: Vec<f64> = (0..matrix.nrows())
        .map(|row| weighted_over_columns(matrix, &sampled, row))
        .collect();

    Ok(Spectrum {
        label,
        wavelength: wavelength_axis.values().to_vec(),
        signal,
        unit,
        version: selection.version,
        sampled,
    })
}

/// The values of one matrix column, used by the fit workbench and by import
/// checks that want raw numbers rather than an average.
pub fn delay_column(dataset: &TaDataset, version: VersionId, index: usize) -> Vec<f64> {
    match dataset.matrix(version) {
        Ok(matrix) if index < matrix.ncols() => (0..matrix.nrows())
            .map(|row| matrix[[row, index]])
            .collect(),
        _ => Vec::new(),
    }
}

/// The values of one matrix row.
pub fn wavelength_row(dataset: &TaDataset, version: VersionId, index: usize) -> Vec<f64> {
    match dataset.matrix(version) {
        Ok(matrix) if index < matrix.nrows() => matrix.row(index).to_vec(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::axis::{Axis, SampleMode};
    use crate::dataset::SourceFormat;
    use crate::error::CoreError;
    use crate::ids::{DatasetId, SeriesId};
    use crate::selection::AxisRequest;
    use crate::units::{AxisKind, AxisUnit};
    use ndarray::array;

    /// Wavelengths 500/510/520 run down the rows, delays -1/0/1/2 across the
    /// columns, so row `i` is a kinetic trace and column `j` is a spectrum.
    fn dataset() -> TaDataset {
        let wavelength = Axis::new(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            "Wavelength",
            vec![500.0, 510.0, 520.0],
        )
        .unwrap();
        let delay = Axis::new(
            AxisKind::Delay,
            AxisUnit::Picosecond,
            "Delay",
            vec![-1.0, 0.0, 1.0, 2.0],
        )
        .unwrap();
        let data = array![
            [0.00, 1.00, 2.00, 3.00],
            [0.10, 1.10, 2.10, 3.10],
            [0.20, 1.20, 2.20, 3.20]
        ];
        TaDataset::new_raw(
            "test",
            wavelength,
            delay,
            data,
            SignalUnit::DeltaOpticalDensity,
            SourceFormat::Synthetic,
        )
        .unwrap()
    }

    fn trace_at(dataset: &TaDataset, wavelength: f64, mode: SampleMode) -> Trace {
        let selection = TraceSelection {
            id: SeriesId::new(),
            dataset: DatasetId::new(),
            version: dataset.raw_version().id,
            wavelength: AxisRequest {
                mode,
                coordinate: wavelength,
            },
        };
        kinetic_trace(dataset, &selection).unwrap()
    }

    fn spectrum_at(dataset: &TaDataset, delay: f64, mode: SampleMode) -> Spectrum {
        let selection = SpectrumSelection {
            id: SeriesId::new(),
            dataset: DatasetId::new(),
            version: dataset.raw_version().id,
            delay: AxisRequest {
                mode,
                coordinate: delay,
            },
        };
        transient_spectrum(dataset, &selection).unwrap()
    }

    fn assert_close(actual: &[f64], expected: &[f64]) {
        assert_eq!(actual.len(), expected.len(), "{actual:?} against {expected:?}");
        for (index, (a, b)) in actual.iter().zip(expected).enumerate() {
            assert!(
                (a - b).abs() < 1e-12,
                "index {index}: {a} against {b} in {actual:?}"
            );
        }
    }

    #[test]
    fn a_trace_runs_over_delay_and_reads_one_matrix_row() {
        let dataset = dataset();
        let trace = trace_at(&dataset, 510.0, SampleMode::Nearest);
        assert_eq!(trace.delay, vec![-1.0, 0.0, 1.0, 2.0]);
        assert_eq!(trace.signal, vec![0.1, 1.1, 2.1, 3.1]);
        assert_eq!(trace.len(), trace.delay.len());
        assert!(trace.label.contains("510"));
    }

    #[test]
    fn an_interpolated_trace_blends_the_two_bracketing_wavelengths() {
        let dataset = dataset();
        let trace = trace_at(&dataset, 505.0, SampleMode::Interpolated);
        assert_eq!(trace.signal, vec![0.05, 1.05, 2.05, 3.05]);
        assert!(trace.label.contains("interpolated"));
    }

    #[test]
    fn a_windowed_trace_averages_every_wavelength_in_the_band() {
        let dataset = dataset();
        let trace = trace_at(&dataset, 510.0, SampleMode::AverageWindow { half_width: 10.0 });
        assert_close(&trace.signal, &[0.1, 1.1, 2.1, 3.1]);
        assert!(trace.label.contains("3 samples"));
        let (low, high) = trace.sampled.span();
        assert_eq!((low, high), (500.0, 520.0));
    }

    #[test]
    fn a_spectrum_runs_over_wavelength_and_reads_one_matrix_column() {
        let dataset = dataset();
        let spectrum = spectrum_at(&dataset, 0.0, SampleMode::Nearest);
        assert_eq!(spectrum.wavelength, vec![500.0, 510.0, 520.0]);
        assert_eq!(spectrum.signal, vec![1.0, 1.1, 1.2]);
        assert!(spectrum.label.contains("0"));
    }

    #[test]
    fn an_interpolated_spectrum_blends_the_two_bracketing_delays() {
        let dataset = dataset();
        let spectrum = spectrum_at(&dataset, 0.5, SampleMode::Interpolated);
        assert_close(&spectrum.signal, &[1.5, 1.6, 1.7]);
    }

    #[test]
    fn a_missing_sample_inside_a_window_is_left_out_rather_than_counted_as_zero() {
        let mut dataset = dataset();
        // Wavelength 510 is missing at delay -1 ps.
        dataset.blocks[0].data[[1, 0]] = f64::NAN;
        let trace = trace_at(&dataset, 505.0, SampleMode::AverageWindow { half_width: 5.0 });

        // At -1 ps only wavelength 500 survives, so the value is 0.00 alone
        // rather than (0.00 + NaN) / 2.
        assert_eq!(trace.signal[0], 0.0);
        // Everywhere else the average is over both wavelengths.
        assert!((trace.signal[1] - 1.05).abs() < 1e-12);
        assert_eq!(trace.finite_points().len(), 4);
    }

    #[test]
    fn a_wholly_missing_trace_stays_missing_instead_of_becoming_zero() {
        let mut dataset = dataset();
        for column in 0..4 {
            dataset.blocks[0].data[[2, column]] = f64::NAN;
        }
        let trace = trace_at(&dataset, 520.0, SampleMode::Nearest);
        assert!(trace.signal.iter().all(|value| value.is_nan()));
        assert_eq!(trace.range(), None);
        assert!(trace.finite_points().is_empty());
    }

    #[test]
    fn restricting_a_trace_to_a_delay_range_keeps_only_those_points() {
        let dataset = dataset();
        let trace = trace_at(&dataset, 510.0, SampleMode::Nearest);
        let windowed = trace.within((0.0, 1.0));
        assert_eq!(windowed.delay, vec![0.0, 1.0]);
        assert_eq!(windowed.signal, vec![1.1, 2.1]);
        // The original is untouched.
        assert_eq!(trace.len(), 4);
    }

    #[test]
    fn requesting_an_interpolated_trace_outside_the_axis_is_an_error() {
        let dataset = dataset();
        let selection = TraceSelection {
            id: SeriesId::new(),
            dataset: DatasetId::new(),
            version: dataset.raw_version().id,
            wavelength: AxisRequest::interpolated(50.0),
        };
        assert!(kinetic_trace(&dataset, &selection).is_err());
    }

    #[test]
    fn a_version_with_no_numbers_loaded_reports_that_instead_of_reading_zeros() {
        let mut dataset = dataset();
        dataset.blocks[0].data = ndarray::Array2::zeros((0, 0));
        let selection = TraceSelection {
            id: SeriesId::new(),
            dataset: DatasetId::new(),
            version: dataset.raw_version().id,
            wavelength: AxisRequest::nearest(510.0),
        };
        assert!(matches!(
            kinetic_trace(&dataset, &selection),
            Err(CoreError::MissingMatrixData { .. })
        ));
    }

    #[test]
    fn a_nearest_trace_outside_the_axis_clamps_and_says_so() {
        let dataset = dataset();
        let trace = trace_at(&dataset, 900.0, SampleMode::Nearest);
        assert_eq!(trace.signal, vec![0.2, 1.2, 2.2, 3.2]);
        assert!(!trace.sampled.is_exact());
        assert!(trace.label.contains("nearest sample"));
    }

    #[test]
    fn raw_matrix_access_helpers_return_the_expected_slices() {
        let dataset = dataset();
        let raw = dataset.raw_version().id;
        assert_eq!(delay_column(&dataset, raw, 2), vec![2.0, 2.1, 2.2]);
        assert_eq!(wavelength_row(&dataset, raw, 1), vec![0.1, 1.1, 2.1, 3.1]);
        assert!(delay_column(&dataset, raw, 99).is_empty());
    }
}
