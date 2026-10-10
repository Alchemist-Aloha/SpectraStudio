//! Units and calibration factors.
//!
//! Every stored quantity keeps the unit it arrived in, and conversion happens
//! only where a caller asks for it. Nothing in this crate silently rescales a
//! matrix.

use serde::{Deserialize, Serialize};

/// The quantity a matrix describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalUnit {
    /// ΔOD, also written ΔA. The canonical transient absorption unit.
    DeltaOpticalDensity,
    /// mOD, one thousandth of a ΔOD.
    MilliDeltaOpticalDensity,
    /// ΔT/T.
    DeltaTransmission,
    Counts,
    Volts,
    Arbitrary,
}

impl SignalUnit {
    pub const fn symbol(self) -> &'static str {
        match self {
            SignalUnit::DeltaOpticalDensity => "ΔOD",
            SignalUnit::MilliDeltaOpticalDensity => "mOD",
            SignalUnit::DeltaTransmission => "ΔT/T",
            SignalUnit::Counts => "counts",
            SignalUnit::Volts => "V",
            SignalUnit::Arbitrary => "a.u.",
        }
    }

    /// The factor that turns a value in this unit into canonical ΔOD.
    pub const fn to_delta_od(self) -> Option<f64> {
        match self {
            SignalUnit::DeltaOpticalDensity => Some(1.0),
            SignalUnit::MilliDeltaOpticalDensity => Some(1e-3),
            SignalUnit::DeltaTransmission => None,
            SignalUnit::Counts => None,
            SignalUnit::Volts => None,
            SignalUnit::Arbitrary => None,
        }
    }

    /// The unit that makes `symbol` retrievable again, for display.
    pub const fn is_delta_od_like(self) -> bool {
        matches!(
            self,
            SignalUnit::DeltaOpticalDensity | SignalUnit::MilliDeltaOpticalDensity
        )
    }
}

/// The physical kind of an axis, which decides what its numbers mean.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AxisKind {
    Wavelength,
    Delay,
    Wavenumber,
    Energy,
    Index,
    Custom,
}

impl AxisKind {
    pub const fn default_name(self) -> &'static str {
        match self {
            AxisKind::Wavelength => "Wavelength",
            AxisKind::Delay => "Delay",
            AxisKind::Wavenumber => "Wavenumber",
            AxisKind::Energy => "Energy",
            AxisKind::Index => "Index",
            AxisKind::Custom => "Axis",
        }
    }

    /// Delay axes carry negative values and are frequently non-uniform, so
    /// they are the one axis kind where an index view must be opt-in.
    pub const fn allows_negative(self) -> bool {
        matches!(self, AxisKind::Delay | AxisKind::Energy)
    }
}

/// The unit an axis is measured in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AxisUnit {
    Nanometer,
    Micrometer,
    Wavenumber,
    ElectronVolt,
    Femtosecond,
    Picosecond,
    Nanosecond,
    Microsecond,
    Millisecond,
    Second,
    DeltaOpticalDensity,
    MilliDeltaOpticalDensity,
    Unitless,
}

impl AxisUnit {
    pub const fn symbol(self) -> &'static str {
        match self {
            AxisUnit::Nanometer => "nm",
            AxisUnit::Micrometer => "µm",
            AxisUnit::Wavenumber => "cm⁻¹",
            AxisUnit::ElectronVolt => "eV",
            AxisUnit::Femtosecond => "fs",
            AxisUnit::Picosecond => "ps",
            AxisUnit::Nanosecond => "ns",
            AxisUnit::Microsecond => "µs",
            AxisUnit::Millisecond => "ms",
            AxisUnit::Second => "s",
            AxisUnit::DeltaOpticalDensity => "ΔOD",
            AxisUnit::MilliDeltaOpticalDensity => "mOD",
            AxisUnit::Unitless => "",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn milli_absorbance_converts_to_absorbance() {
        assert_eq!(
            SignalUnit::MilliDeltaOpticalDensity.to_delta_od(),
            Some(1e-3)
        );
        assert_eq!(SignalUnit::Arbitrary.to_delta_od(), None);
    }

    #[test]
    fn delay_axis_is_the_one_that_carries_negative_coordinates() {
        assert!(AxisKind::Delay.allows_negative());
        assert!(!AxisKind::Wavelength.allows_negative());
    }
}
