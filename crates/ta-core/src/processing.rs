//! Processing operations and the derived-version graph.
//!
//! The raw measurement is immutable. Everything a user does produces a new
//! [`MatrixVersion`] that names the operation, its parameters and the versions
//! it consumed. The graph is what makes preview, undo, and reproducibility
//! possible without ever touching the raw numbers.

use crate::axis::Interpolation;
use crate::ids::{DatasetId, VersionId};
use serde::{Deserialize, Serialize};

/// What kind of result a version holds, used for icons, labels and grouping in
/// the dataset tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionKind {
    Raw,
    BackgroundCorrected,
    TimeZeroCorrected,
    Cropped,
    Averaged,
    BurnCorrected,
    Stitched,
    Resampled,
}

impl VersionKind {
    pub const fn label(self) -> &'static str {
        match self {
            VersionKind::Raw => "Raw matrix",
            VersionKind::BackgroundCorrected => "Background corrected",
            VersionKind::TimeZeroCorrected => "Time-zero corrected",
            VersionKind::Cropped => "Cropped",
            VersionKind::Averaged => "Averaged",
            VersionKind::BurnCorrected => "Photodegradation corrected",
            VersionKind::Stitched => "Stitched",
            VersionKind::Resampled => "Resampled",
        }
    }
}

/// Which delay columns form the pre-time-zero baseline.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreZeroRange {
    /// The first `count` delay points, which is what the Python reference does
    /// by default.
    FirstPoints { count: usize },
    /// Every delay column inside an explicit interval.
    Interval { start: f64, end: f64 },
}

/// Baseline subtraction against the dataset's own pre-zero region.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreZeroBackground {
    pub range: PreZeroRange,
}

/// Subtraction of another dataset's reference spectrum.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReferenceBackground {
    pub reference_dataset: DatasetId,
    pub reference_version: VersionId,
    /// Multiplicative factor applied to the reference before subtraction.
    pub factor: f64,
    /// Scale the reference to the sample's pre-zero region before subtracting.
    pub normalize: bool,
    pub interpolation: Interpolation,
}

/// A point on the hand-drawn or fitted time-zero curve.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ControlPoint {
    pub wavelength: f64,
    /// Delay of the signal onset at this wavelength.
    pub time_zero: f64,
}

/// The wavelength-dependent correction curve, in physical coordinates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CorrectionCurve {
    pub points: Vec<ControlPoint>,
}

impl CorrectionCurve {
    pub fn new(points: Vec<ControlPoint>) -> Self {
        Self { points }
    }

    /// The curve as `(wavelength, time_zero)` pairs in ascending wavelength
    /// order, which is the order interpolation needs.
    pub fn sorted(&self) -> Vec<ControlPoint> {
        let mut points = self.points.clone();
        points.sort_by(|a, b| a.wavelength.total_cmp(&b.wavelength));
        points
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }
}

/// What happens to a trace outside the corrected range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryMode {
    /// Hold the edge value.
    Clamp,
    /// Mark the sample as missing.
    Mask,
    /// Extend the edge slope.
    Extrapolate,
}

/// How a correction curve was obtained. The data is the same either way, so
/// the editor is the same either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CorrectionOrigin {
    Manual,
    Automatic,
}

/// Wavelength-dependent time-zero (chirp) correction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimeZeroCorrection {
    pub origin: CorrectionOrigin,
    pub curve: CorrectionCurve,
    pub interpolation: Interpolation,
    pub boundary: BoundaryMode,
}

/// Restriction of the matrix to a wavelength and/or delay interval.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CropSpec {
    pub wavelength: Option<(f64, f64)>,
    pub delay: Option<(f64, f64)>,
}

/// Averaging of parallel experiments. Inputs live on the version, not here.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AverageSpec {
    /// Divide by the number of inputs to keep the signal scale.
    pub mean: bool,
}

/// Photodegradation correction: rescale each delay column so a reference band
/// returns to its early-time value.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BurnCorrection {
    /// Wavelength whose kinetics drive the correction factor.
    pub wavelength: f64,
    /// Delay used as the reference amplitude.
    pub reference_delay: f64,
}

/// Stitching of overlapping visible and NIR datasets into one matrix.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StitchSpec {
    /// Wavelength where the two matrices cross over.
    pub crossover_nm: f64,
    /// How the overlap is combined.
    pub blend: BlendMode,
    /// Scale the NIR matrix so the overlap matches the visible one.
    pub match_overlap: bool,
    /// Whether the stitched result keeps the interpolated delay grid.
    pub delay_grid: StitchDelayGrid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    /// Visible below the crossover, NIR above it.
    Hard,
    /// Cross-fade across the overlapping wavelength band.
    Linear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StitchDelayGrid {
    /// Require identical delay axes.
    Exact,
    /// Resample onto the finer of the two delay grids.
    Finer,
}

/// Everything a user can do to a matrix.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ProcessingOp {
    BackgroundPreZero(PreZeroBackground),
    BackgroundReference(ReferenceBackground),
    TimeZero(TimeZeroCorrection),
    Crop(CropSpec),
    Average(AverageSpec),
    BurnCorrect(BurnCorrection),
    Stitch(StitchSpec),
}

impl ProcessingOp {
    /// The name shown in the dataset tree and the processing history.
    pub const fn summary(&self) -> &'static str {
        match self {
            ProcessingOp::BackgroundPreZero(_) => "Background subtraction",
            ProcessingOp::BackgroundReference(_) => "Reference subtraction",
            ProcessingOp::TimeZero(_) => "Time-zero correction",
            ProcessingOp::Crop(_) => "Crop",
            ProcessingOp::Average(_) => "Average experiments",
            ProcessingOp::BurnCorrect(_) => "Photodegradation correction",
            ProcessingOp::Stitch(_) => "VIS/NIR stitching",
        }
    }

    /// What the operation did, in words, with the parameters that make it
    /// reproducible: "Crop" alone does not say what was kept.
    pub fn describe(&self) -> String {
        match self {
            ProcessingOp::BackgroundPreZero(background) => match background.range {
                PreZeroRange::FirstPoints { count } => format!("first {count} delay points"),
                PreZeroRange::Interval { start, end } => format!("{start:.3} to {end:.3} ps"),
            },
            ProcessingOp::BackgroundReference(reference) => format!(
                "reference dataset, factor {:.3}{}",
                reference.factor,
                if reference.normalize {
                    ", normalised"
                } else {
                    ""
                }
            ),
            ProcessingOp::TimeZero(correction) => format!(
                "{} control point(s), {:?}, {:?}",
                correction.curve.points.len(),
                correction.interpolation,
                correction.boundary
            ),
            ProcessingOp::Crop(crop) => {
                let wavelength = crop
                    .wavelength
                    .map(|(low, high)| format!("{low:.1}–{high:.1} nm"))
                    .unwrap_or_else(|| "all wavelengths".to_string());
                let delay = crop
                    .delay
                    .map(|(low, high)| format!("{low:.3}–{high:.3} ps"))
                    .unwrap_or_else(|| "all delays".to_string());
                format!("{wavelength}, {delay}")
            }
            ProcessingOp::Average(average) => {
                if average.mean {
                    "mean of the inputs".to_string()
                } else {
                    "sum of the inputs".to_string()
                }
            }
            ProcessingOp::BurnCorrect(burn) => format!(
                "at {:.1} nm, reference delay {:.3} ps",
                burn.wavelength, burn.reference_delay
            ),
            ProcessingOp::Stitch(stitch) => format!(
                "crossover {:.1} nm, {:?}{}",
                stitch.crossover_nm,
                stitch.blend,
                if stitch.match_overlap {
                    ", overlap matched"
                } else {
                    ""
                }
            ),
        }
    }

    pub const fn version_kind(&self) -> VersionKind {
        match self {
            ProcessingOp::BackgroundPreZero(_) | ProcessingOp::BackgroundReference(_) => {
                VersionKind::BackgroundCorrected
            }
            ProcessingOp::TimeZero(_) => VersionKind::TimeZeroCorrected,
            ProcessingOp::Crop(_) => VersionKind::Cropped,
            ProcessingOp::Average(_) => VersionKind::Averaged,
            ProcessingOp::BurnCorrect(_) => VersionKind::BurnCorrected,
            ProcessingOp::Stitch(_) => VersionKind::Stitched,
        }
    }
}

/// One node in the derived-data graph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MatrixVersion {
    pub id: VersionId,
    pub label: String,
    pub kind: VersionKind,
    /// Index into the dataset's block store.
    pub block: usize,
    /// `None` for the raw import.
    pub op: Option<ProcessingOp>,
    /// Versions this one was computed from, in the order the operation wants.
    pub inputs: Vec<VersionId>,
    /// Seconds since the Unix epoch, for the processing history.
    pub created_at: u64,
}

impl MatrixVersion {
    pub fn is_raw(&self) -> bool {
        self.op.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correction_curve_sorts_by_wavelength_without_dropping_points() {
        let curve = CorrectionCurve::new(vec![
            ControlPoint {
                wavelength: 600.0,
                time_zero: 0.2,
            },
            ControlPoint {
                wavelength: 450.0,
                time_zero: -0.1,
            },
            ControlPoint {
                wavelength: 520.0,
                time_zero: 0.05,
            },
        ]);
        let sorted = curve.sorted();
        assert_eq!(sorted.len(), 3);
        assert_eq!(sorted[0].wavelength, 450.0);
        assert_eq!(sorted[2].wavelength, 600.0);
        // The original keeps the user's editing order.
        assert_eq!(curve.points[0].wavelength, 600.0);
    }

    #[test]
    fn every_operation_declares_a_kind_and_a_summary() {
        let ops = [
            ProcessingOp::BackgroundPreZero(PreZeroBackground {
                range: PreZeroRange::FirstPoints { count: 5 },
            }),
            ProcessingOp::Crop(CropSpec {
                wavelength: Some((450.0, 700.0)),
                delay: None,
            }),
            ProcessingOp::Stitch(StitchSpec {
                crossover_nm: 950.0,
                blend: BlendMode::Linear,
                match_overlap: true,
                delay_grid: StitchDelayGrid::Finer,
            }),
        ];
        for op in ops {
            assert!(!op.summary().is_empty());
            assert_ne!(op.version_kind(), VersionKind::Raw);
        }
    }
}
