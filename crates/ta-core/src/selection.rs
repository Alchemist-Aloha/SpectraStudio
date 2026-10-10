//! A saved kinetic-trace or transient-spectrum selection.
//!
//! A selection is a *request* in physical coordinates, not a captured array.
//! When the dataset's processing changes, the request is re-resolved against
//! the new matrix version, so a saved trace never shows stale numbers.

use crate::axis::SampleMode;
use crate::ids::{DatasetId, SeriesId, VersionId};
use serde::{Deserialize, Serialize};

/// A coordinate plus the way it should be read off the axis.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AxisRequest {
    pub mode: SampleMode,
    pub coordinate: f64,
}

impl AxisRequest {
    pub fn nearest(coordinate: f64) -> Self {
        Self {
            mode: SampleMode::Nearest,
            coordinate,
        }
    }

    pub fn interpolated(coordinate: f64) -> Self {
        Self {
            mode: SampleMode::Interpolated,
            coordinate,
        }
    }

    pub fn window(coordinate: f64, half_width: f64) -> Self {
        Self {
            mode: SampleMode::AverageWindow { half_width },
            coordinate,
        }
    }

    pub fn channels(coordinate: f64, count: usize) -> Self {
        Self {
            mode: SampleMode::AverageChannels { count },
            coordinate,
        }
    }
}

/// One saved kinetic trace: this dataset, this processing version, and the
/// wavelength it is taken at.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceSelection {
    pub id: SeriesId,
    pub dataset: DatasetId,
    pub version: VersionId,
    pub wavelength: AxisRequest,
}

impl TraceSelection {
    pub fn new(dataset: DatasetId, version: VersionId, wavelength: AxisRequest) -> Self {
        Self {
            id: SeriesId::new(),
            dataset,
            version,
            wavelength,
        }
    }
}

/// One saved transient spectrum: this dataset, this processing version, and the
/// delay it is taken at.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpectrumSelection {
    pub id: SeriesId,
    pub dataset: DatasetId,
    pub version: VersionId,
    pub delay: AxisRequest,
}

impl SpectrumSelection {
    pub fn new(dataset: DatasetId, version: VersionId, delay: AxisRequest) -> Self {
        Self {
            id: SeriesId::new(),
            dataset,
            version,
            delay,
        }
    }
}
