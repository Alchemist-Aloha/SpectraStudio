//! The dataset both writers consume.
//!
//! Glotaran's modern convention is time along the first axis of the data and
//! wavelength along the second, which is the transpose of how this project
//! stores a matrix. Doing the transpose once, here, is what keeps the two
//! writers from each having their own idea of which axis is which.

use ndarray::Array2;
use ta_core::{Axis, CoreError, Result, SignalUnit, TaDataset, VersionId};

/// The dimension names of the modern dataset, which is what a reader of the
/// file sees.
pub const TIME_DIMENSION: &str = "time";
pub const SPECTRAL_DIMENSION: &str = "spectral";
pub const MATRIX_VARIABLE: &str = "data";

/// One matrix, oriented the way Glotaran reads it, with the provenance a file
/// has to carry to be reproducible.
#[derive(Clone, Debug, PartialEq)]
pub struct GloDataset {
    /// The dataset's name, which is the file's own title.
    pub name: String,
    /// The version's label, so two exports can be told apart.
    pub version_label: String,
    /// What produced the version, in words.
    pub operation: String,
    /// Delay points, in picoseconds.
    pub time: Vec<f64>,
    /// Wavelength points, in nanometres.
    pub spectral: Vec<f64>,
    /// The signal, time-major: `data[t * spectral.len() + w]`.
    pub data: Vec<f64>,
    pub unit: SignalUnit,
}

impl GloDataset {
    /// Reads one version of one dataset into the Glotaran orientation.
    pub fn from_version(dataset: &TaDataset, version: VersionId) -> Result<Self> {
        let node = dataset.version(version)?;
        let (wavelength, delay) = dataset.axes(version)?;
        let matrix = dataset.matrix(version)?;
        let (rows, columns) = matrix.dim();
        if rows != wavelength.len() || columns != delay.len() {
            return Err(CoreError::ShapeMismatch {
                rows,
                cols: columns,
                wavelengths: wavelength.len(),
                delays: delay.len(),
            });
        }
        // time-major: the delay varies slowest, which is the convention both
        // writers use.
        let mut data = Vec::with_capacity(rows * columns);
        for column in 0..columns {
            for row in 0..rows {
                data.push(matrix[[row, column]]);
            }
        }
        Ok(Self {
            name: dataset.name.clone(),
            version_label: node.label.clone(),
            // The parameters are part of the provenance: "Crop" alone does not
            // say what was kept.
            operation: node
                .op
                .as_ref()
                .map(|op| format!("{} — {}", op.summary(), op.describe()))
                .unwrap_or_else(|| "import".to_string()),
            time: delay.values().to_vec(),
            spectral: wavelength.values().to_vec(),
            data,
            unit: dataset.unit(version).unwrap_or(SignalUnit::DeltaOpticalDensity),
        })
    }

    /// Builds the view directly, which is what the writer tests use.
    pub fn new(
        name: impl Into<String>,
        time: Vec<f64>,
        spectral: Vec<f64>,
        data: Vec<f64>,
        unit: SignalUnit,
    ) -> Result<Self> {
        let expected = time.len() * spectral.len();
        if data.len() != expected {
            return Err(CoreError::invalid(format!(
                "a {} × {} dataset needs {expected} values, not {}",
                time.len(),
                spectral.len(),
                data.len()
            )));
        }
        Ok(Self {
            name: name.into(),
            version_label: "as given".to_string(),
            operation: "given".to_string(),
            time,
            spectral,
            data,
            unit,
        })
    }

    /// The value at one time and one wavelength.
    pub fn value(&self, time_index: usize, spectral_index: usize) -> f64 {
        self.data[time_index * self.spectral.len() + spectral_index]
    }

    /// The matrix as this project stores it: wavelength-major.
    pub fn to_internal_matrix(&self) -> Array2<f64> {
        let mut matrix = Array2::zeros((self.spectral.len(), self.time.len()));
        for (time_index, _) in self.time.iter().enumerate() {
            for (spectral_index, _) in self.spectral.iter().enumerate() {
                matrix[[spectral_index, time_index]] = self.value(time_index, spectral_index);
            }
        }
        matrix
    }

    /// The axes this dataset describes, in this project's own terms.
    pub fn axes(&self) -> Result<(Axis, Axis)> {
        let wavelength = Axis::new(
            ta_core::AxisKind::Wavelength,
            ta_core::AxisUnit::Nanometer,
            "Wavelength",
            self.spectral.clone(),
        )?;
        let delay = Axis::new(
            ta_core::AxisKind::Delay,
            ta_core::AxisUnit::Picosecond,
            "Delay",
            self.time.clone(),
        )?;
        Ok((wavelength, delay))
    }

    /// The shortest description of where the numbers came from.
    pub fn provenance(&self) -> String {
        format!(
            "{} · {} · {} · {} × {} ({})",
            self.name,
            self.version_label,
            self.operation,
            self.time.len(),
            self.spectral.len(),
            self.unit.symbol()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::Array2;

    #[test]
    fn a_matrix_is_transposed_into_the_time_major_convention() {
        // Internal storage is [wavelength, delay]; Glotaran's is [time, spectral].
        let wavelength = Axis::new(
            ta_core::AxisKind::Wavelength,
            ta_core::AxisUnit::Nanometer,
            "Wavelength",
            vec![500.0, 600.0, 700.0],
        )
        .unwrap();
        let delay = Axis::new(
            ta_core::AxisKind::Delay,
            ta_core::AxisUnit::Picosecond,
            "Delay",
            vec![-1.0, 1.0],
        )
        .unwrap();
        let mut matrix = Array2::zeros((3, 2));
        for row in 0..3 {
            for column in 0..2 {
                matrix[[row, column]] = (row as f64 + 1.0) * 10.0 + column as f64;
            }
        }
        let dataset = TaDataset::new_raw(
            "sample",
            wavelength.clone(),
            delay.clone(),
            matrix.clone(),
            SignalUnit::DeltaOpticalDensity,
            ta_core::SourceFormat::Synthetic,
        )
        .unwrap();
        let view = GloDataset::from_version(&dataset, dataset.active_version).unwrap();
        assert_eq!(view.time, vec![-1.0, 1.0]);
        assert_eq!(view.spectral, vec![500.0, 600.0, 700.0]);
        // The first delay column is now the first block of the flat array.
        assert_eq!(view.value(0, 0), 10.0);
        assert_eq!(view.value(0, 2), 30.0);
        assert_eq!(view.value(1, 0), 11.0);
        // And it transposes back without losing anything.
        assert_eq!(view.to_internal_matrix(), matrix);
        assert!(view.provenance().contains("2 × 3"));
    }

    #[test]
    fn a_flat_array_of_the_wrong_length_is_refused() {
        assert!(GloDataset::new("bad", vec![0.0, 1.0], vec![500.0], vec![0.0], SignalUnit::Counts)
            .is_err());
    }
}
