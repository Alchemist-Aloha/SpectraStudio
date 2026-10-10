//! The dataset object.
//!
//! A [`TaDataset`] owns an immutable raw matrix, every derived version of that
//! matrix, and the selections, fits and figures that point at them.
//!
//! Matrices are stored wavelength-major: entry `(i, j)` is the signal at
//! `wavelength[i]` and `delay[j]`. The axes live on the *block*, not on the
//! dataset, because cropping changes the grid: every version therefore carries
//! the coordinates it was actually computed on, and no lookup can read a
//! wavelength that version does not have.

use crate::axis::Axis;
use crate::error::{CoreError, Result};
use crate::figure::FigureModel;
use crate::fitting::FitResult;
use crate::ids::{DatasetId, VersionId, FitId};
use crate::processing::{MatrixVersion, ProcessingOp, VersionKind};
use crate::selection::{SpectrumSelection, TraceSelection};
use crate::units::SignalUnit;
use ndarray::Array2;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Where a dataset came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceFormat {
    /// Whitespace- or tab-delimited matrix with separate axis files.
    TextMatrix,
    StreakCamera,
    GlotaranResult,
    /// Reconstructed from a saved project.
    Project,
    Synthetic,
    Unknown,
}

impl Default for SourceFormat {
    fn default() -> Self {
        SourceFormat::Unknown
    }
}

impl SourceFormat {
    pub const fn label(self) -> &'static str {
        match self {
            SourceFormat::TextMatrix => "Text matrix",
            SourceFormat::StreakCamera => "Streak camera",
            SourceFormat::GlotaranResult => "Glotaran result",
            SourceFormat::Project => "Project",
            SourceFormat::Synthetic => "Synthetic",
            SourceFormat::Unknown => "Unknown",
        }
    }
}

/// Everything known about a measurement that is not a number axis or matrix.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DatasetMetadata {
    pub source_format: SourceFormat,
    /// Pump wavelength, when the file or the user says what it was.
    pub excitation_wavelength_nm: Option<f64>,
    pub solvent: Option<String>,
    pub temperature_k: Option<f64>,
    pub notes: Option<String>,
    /// Header fields the importer did not recognise, kept verbatim rather than
    /// dropped.
    pub extra: BTreeMap<String, String>,
}

/// One stored matrix, with the coordinates it is sampled on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MatrixBlock {
    pub unit: SignalUnit,
    pub wavelength: Axis,
    pub delay: Axis,
    /// The numbers themselves. Not part of the JSON manifest: matrices live in
    /// the project's binary data layer, and a source-referenced project leaves
    /// this empty until the dataset is relinked.
    #[serde(skip)]
    pub data: Array2<f64>,
}

impl MatrixBlock {
    /// Builds a block, refusing a matrix that does not match its axes.
    pub fn new(
        wavelength: Axis,
        delay: Axis,
        data: Array2<f64>,
        unit: SignalUnit,
    ) -> Result<Self> {
        let (rows, cols) = data.dim();
        if rows != wavelength.len() || cols != delay.len() {
            return Err(CoreError::ShapeMismatch {
                rows,
                cols,
                wavelengths: wavelength.len(),
                delays: delay.len(),
            });
        }
        Ok(Self {
            unit,
            wavelength,
            delay,
            data,
        })
    }

    pub fn rows(&self) -> usize {
        self.wavelength.len()
    }

    pub fn cols(&self) -> usize {
        self.delay.len()
    }

    pub fn shape(&self) -> (usize, usize) {
        (self.rows(), self.cols())
    }

    /// Whether the numbers are present. False for a source-referenced project
    /// that has not been relinked yet.
    pub fn has_data(&self) -> bool {
        self.data.nrows() == self.rows() && self.data.ncols() == self.cols()
    }

    pub fn value(&self, row: usize, column: usize) -> Result<f64> {
        if !self.has_data() {
            return Err(CoreError::invalid(format!(
                "this matrix ({rows}×{cols}) has no numbers loaded",
                rows = self.rows(),
                cols = self.cols()
            )));
        }
        Ok(self.data[[row, column]])
    }

    /// The finite values, for colour-limit fitting and statistics.
    pub fn finite_values(&self) -> impl Iterator<Item = f64> + '_ {
        self.data.iter().copied().filter(|value| value.is_finite())
    }

    pub fn range(&self) -> Option<(f64, f64)> {
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        let mut seen = false;
        for value in self.finite_values() {
            seen = true;
            min = min.min(value);
            max = max.max(value);
        }
        seen.then_some((min, max))
    }
}

/// What removing a matrix version took with it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VersionRemoval {
    /// The version and everything derived from it, in insertion order.
    pub versions: Vec<VersionId>,
    pub pruned_traces: usize,
    pub pruned_spectra: usize,
    pub pruned_fits: usize,
    pub pruned_series: usize,
}

/// A complete imported experiment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaDataset {
    pub id: DatasetId,
    pub name: String,
    /// Files this dataset was read from, so a project can report a missing
    /// source instead of silently holding stale data.
    pub source_paths: Vec<String>,
    pub metadata: DatasetMetadata,
    /// The data layer. Versions index into this.
    pub blocks: Vec<MatrixBlock>,
    /// The derived-data graph, oldest first.
    pub versions: Vec<MatrixVersion>,
    pub active_version: VersionId,
    pub traces: Vec<TraceSelection>,
    pub spectra: Vec<SpectrumSelection>,
    pub fits: Vec<FitResult>,
    pub figures: Vec<FigureModel>,
}

impl TaDataset {
    /// Builds a dataset from an imported raw matrix.
    pub fn new_raw(
        name: impl Into<String>,
        wavelength: Axis,
        delay: Axis,
        data: Array2<f64>,
        unit: SignalUnit,
        source_format: SourceFormat,
    ) -> Result<Self> {
        let block = MatrixBlock::new(wavelength, delay, data, unit)?;
        let version_id = VersionId::new();
        Ok(Self {
            id: DatasetId::new(),
            name: name.into(),
            source_paths: Vec::new(),
            metadata: DatasetMetadata {
                source_format,
                ..DatasetMetadata::default()
            },
            blocks: vec![block],
            versions: vec![MatrixVersion {
                id: version_id,
                label: VersionKind::Raw.label().to_string(),
                kind: VersionKind::Raw,
                block: 0,
                op: None,
                inputs: Vec::new(),
                created_at: 0,
            }],
            active_version: version_id,
            traces: Vec::new(),
            spectra: Vec::new(),
            fits: Vec::new(),
            figures: Vec::new(),
        })
    }

    /// The axes of the version that is currently displayed.
    pub fn wavelength(&self) -> &Axis {
        &self
            .block(self.active_version)
            .expect("the active version always has a block")
            .wavelength
    }

    pub fn delay(&self) -> &Axis {
        &self
            .block(self.active_version)
            .expect("the active version always has a block")
            .delay
    }

    pub fn axes(&self, version: VersionId) -> Result<(&Axis, &Axis)> {
        let block = self.block(version)?;
        Ok((&block.wavelength, &block.delay))
    }

    pub fn shape(&self) -> (usize, usize) {
        match self.block(self.active_version) {
            Ok(block) => block.shape(),
            Err(_) => (0, 0),
        }
    }

    pub fn shape_of(&self, version: VersionId) -> Result<(usize, usize)> {
        Ok(self.block(version)?.shape())
    }

    /// A retained fitting session.
    pub fn fit(&self, id: FitId) -> Option<&FitResult> {
        self.fits.iter().find(|fit| fit.id == id)
    }

    pub fn version(&self, id: VersionId) -> Result<&MatrixVersion> {
        self.versions
            .iter()
            .find(|version| version.id == id)
            .ok_or_else(|| CoreError::UnknownVersion {
                dataset: self.name.clone(),
                version: id.to_string(),
            })
    }

    pub fn version_mut(&mut self, id: VersionId) -> Result<&mut MatrixVersion> {
        let name = self.name.clone();
        self.versions
            .iter_mut()
            .find(|version| version.id == id)
            .ok_or_else(|| CoreError::UnknownVersion {
                dataset: name,
                version: id.to_string(),
            })
    }

    pub fn raw_version(&self) -> &MatrixVersion {
        self.versions
            .iter()
            .find(|version| version.is_raw())
            .unwrap_or(&self.versions[0])
    }

    pub fn active_version(&self) -> &MatrixVersion {
        self.version(self.active_version)
            .unwrap_or_else(|_| self.raw_version())
    }

    pub fn block(&self, version: VersionId) -> Result<&MatrixBlock> {
        let version = self.version(version)?;
        self.blocks
            .get(version.block)
            .ok_or_else(|| CoreError::UnknownBlock {
                version: version.id.to_string(),
                block: version.block,
            })
    }

    pub fn matrix(&self, version: VersionId) -> Result<&Array2<f64>> {
        let block = self.block(version)?;
        if !block.has_data() {
            return Err(CoreError::MissingMatrixData {
                dataset: self.name.clone(),
                version: version.to_string(),
            });
        }
        Ok(&block.data)
    }

    pub fn active_matrix(&self) -> &Array2<f64> {
        self.matrix(self.active_version)
            .expect("the active version is always resolvable")
    }

    pub fn unit(&self, version: VersionId) -> Result<SignalUnit> {
        Ok(self.block(version)?.unit)
    }

    /// Adds a derived matrix as a new version and makes it active.
    pub fn add_derived(
        &mut self,
        data: Array2<f64>,
        unit: SignalUnit,
        wavelength: Axis,
        delay: Axis,
        label: impl Into<String>,
        op: ProcessingOp,
        inputs: Vec<VersionId>,
    ) -> Result<VersionId> {
        for input in &inputs {
            self.version(*input)?;
        }
        let block = MatrixBlock::new(wavelength, delay, data, unit)?;
        let id = VersionId::new();
        let block_index = self.blocks.len();
        self.blocks.push(block);
        self.versions.push(MatrixVersion {
            id,
            label: label.into(),
            kind: op.version_kind(),
            block: block_index,
            op: Some(op),
            inputs,
            created_at: now_seconds(),
        });
        self.active_version = id;
        Ok(id)
    }

    pub fn set_active(&mut self, version: VersionId) -> Result<()> {
        self.version(version)?;
        self.active_version = version;
        Ok(())
    }

    /// Every version derived from `version`, directly.
    pub fn children(&self, version: VersionId) -> Vec<VersionId> {
        self.versions
            .iter()
            .filter(|candidate| candidate.inputs.contains(&version))
            .map(|candidate| candidate.id)
            .collect()
    }

    /// `version` and everything it was computed from, oldest first. This is the
    /// processing history the inspector shows.
    pub fn ancestry(&self, version: VersionId) -> Vec<VersionId> {
        let mut ordered: Vec<VersionId> = Vec::new();
        let mut pending = vec![version];
        while let Some(next) = pending.pop() {
            if ordered.contains(&next) {
                continue;
            }
            if let Ok(node) = self.version(next) {
                ordered.push(next);
                pending.extend(node.inputs.iter().copied());
            }
        }
        // Oldest first: provenance order matches insertion order.
        ordered.sort_by_key(|id| {
            self.versions
                .iter()
                .position(|version| version.id == *id)
                .unwrap_or(usize::MAX)
        });
        ordered
    }

    /// Removes a version and everything derived from it, so a dataset can never
    /// hold a node whose inputs are gone. The matrix blocks that are no longer
    /// referenced go too, so undoing a chain actually frees its memory.
    ///
    /// Saved traces, spectra, fits and figure series that read the removed
    /// version are pruned with it and counted in the report. Source files are
    /// never touched.
    pub fn remove_version(&mut self, version: VersionId) -> Result<VersionRemoval> {
        if self.version(version)?.is_raw() {
            return Err(CoreError::invalid(
                "the raw matrix is the floor of the graph and cannot be removed",
            ));
        }
        let mut doomed = vec![version];
        let mut index = 0;
        while index < doomed.len() {
            let current = doomed[index];
            for child in self.children(current) {
                if !doomed.contains(&child) {
                    doomed.push(child);
                }
            }
            index += 1;
        }
        // Detach dependants that are not themselves removed.
        for node in &mut self.versions {
            if !doomed.contains(&node.id) {
                node.inputs.retain(|input| !doomed.contains(input));
            }
        }
        // The report lists versions oldest first, matching insertion order.
        let ordered: Vec<VersionId> = self
            .versions
            .iter()
            .filter(|node| doomed.contains(&node.id))
            .map(|node| node.id)
            .collect();
        self.versions.retain(|node| !doomed.contains(&node.id));
        self.compact_blocks();

        let mut report = VersionRemoval {
            versions: ordered,
            ..VersionRemoval::default()
        };
        let traces = self.traces.len();
        self.traces
            .retain(|trace| !doomed.contains(&trace.version));
        report.pruned_traces = traces - self.traces.len();
        let spectra = self.spectra.len();
        self.spectra
            .retain(|spectrum| !doomed.contains(&spectrum.version));
        report.pruned_spectra = spectra - self.spectra.len();
        let fits = self.fits.len();
        self.fits.retain(|fit| {
            !doomed.contains(&fit.version) && !doomed.contains(&fit.selection.version)
        });
        report.pruned_fits = fits - self.fits.len();

        let id = self.id;
        for figure in &mut self.figures {
            for panel in &mut figure.panels {
                let before = panel.series.len();
                panel.series.retain(|series| {
                    !doomed.iter().any(|version| {
                        crate::figure::series_references_version(series, id, *version)
                    })
                });
                report.pruned_series += before - panel.series.len();
            }
        }
        self.figures.retain(|figure| {
            figure
                .panels
                .iter()
                .any(|panel| !panel.series.is_empty())
        });

        if doomed.contains(&self.active_version) {
            self.active_version = self.raw_version().id;
        }
        Ok(report)
    }

    /// Drops unreferenced blocks and renumbers the versions that remain.
    fn compact_blocks(&mut self) {
        let mut keep: Vec<usize> = self.versions.iter().map(|version| version.block).collect();
        keep.sort_unstable();
        keep.dedup();

        let mut taken: Vec<Option<MatrixBlock>> =
            std::mem::take(&mut self.blocks).into_iter().map(Some).collect();
        let mut mapping = vec![usize::MAX; taken.len()];
        let mut kept = Vec::with_capacity(keep.len());
        for (new_index, old_index) in keep.iter().enumerate() {
            mapping[*old_index] = new_index;
            if let Some(block) = taken[*old_index].take() {
                kept.push(block);
            }
        }
        self.blocks = kept;
        for version in &mut self.versions {
            version.block = mapping[version.block];
        }
    }

    /// The wavelength range of the version on display.
    pub fn wavelength_bounds(&self) -> (f64, f64) {
        self.wavelength().range()
    }

    /// The delay range of the version on display.
    pub fn delay_bounds(&self) -> (f64, f64) {
        self.delay().range()
    }

    /// A copy that shares no identity, used by "duplicate dataset".
    pub fn duplicate(&self, name: impl Into<String>) -> Self {
        let mut copy = self.clone();
        copy.id = DatasetId::new();
        copy.name = name.into();
        copy
    }

    /// Checks the invariants this type maintains internally.
    pub fn validate(&self) -> Result<()> {
        for block in &self.blocks {
            block.wavelength.validate()?;
            block.delay.validate()?;
            if !block.data.is_empty() && !block.has_data() {
                return Err(CoreError::ShapeMismatch {
                    rows: block.data.nrows(),
                    cols: block.data.ncols(),
                    wavelengths: block.rows(),
                    delays: block.cols(),
                });
            }
        }
        self.version(self.active_version)?;
        for version in &self.versions {
            if version.block >= self.blocks.len() {
                return Err(CoreError::UnknownBlock {
                    version: version.id.to_string(),
                    block: version.block,
                });
            }
            for input in &version.inputs {
                self.version(*input)?;
            }
            if version.inputs.contains(&version.id) {
                return Err(CoreError::SelfReferentialVersion);
            }
        }
        for trace in &self.traces {
            self.version(trace.version)?;
        }
        for spectrum in &self.spectra {
            self.version(spectrum.version)?;
        }
        for fit in &self.fits {
            self.version(fit.version)?;
            self.version(fit.selection.version)?;
        }
        Ok(())
    }

    /// The status line shown under the explorer.
    pub fn summary(&self) -> String {
        let (wavelengths, delays) = self.shape();
        format!(
            "{wavelengths} wavelengths × {delays} delays · {} · {} versions",
            self.metadata.source_format.label(),
            self.versions.len()
        )
    }
}

pub fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::processing::{CropSpec, PreZeroBackground, PreZeroRange};
    use crate::units::{AxisKind, AxisUnit};
    use ndarray::array;

    fn axes() -> (Axis, Axis) {
        (
            Axis::new(
                AxisKind::Wavelength,
                AxisUnit::Nanometer,
                "Wavelength",
                vec![500.0, 510.0, 520.0],
            )
            .unwrap(),
            Axis::new(
                AxisKind::Delay,
                AxisUnit::Picosecond,
                "Delay",
                vec![-1.0, 0.0, 1.0, 2.0],
            )
            .unwrap(),
        )
    }

    fn dataset() -> TaDataset {
        let (wavelength, delay) = axes();
        let data = array![
            [0.01, 0.02, 0.03, 0.04],
            [0.05, 0.06, 0.07, 0.08],
            [0.09, 0.10, 0.11, 0.12]
        ];
        TaDataset::new_raw(
            "NIR800",
            wavelength,
            delay,
            data,
            SignalUnit::MilliDeltaOpticalDensity,
            SourceFormat::TextMatrix,
        )
        .unwrap()
    }

    fn crop_op() -> ProcessingOp {
        ProcessingOp::Crop(CropSpec {
            wavelength: None,
            delay: None,
        })
    }

    #[test]
    fn a_matrix_whose_shape_disagrees_with_its_axes_is_refused() {
        let (wavelength, delay) = axes();
        let transposed = array![[0.0, 0.0], [0.0, 0.0], [0.0, 0.0], [0.0, 0.0]];
        assert!(matches!(
            MatrixBlock::new(
                wavelength,
                delay,
                transposed,
                SignalUnit::DeltaOpticalDensity
            ),
            Err(CoreError::ShapeMismatch { .. })
        ));
    }

    #[test]
    fn derived_versions_form_a_graph_that_can_be_walked_forwards_and_back() {
        let mut dataset = dataset();
        let raw = dataset.raw_version().id;
        let (wavelength, delay) = axes();
        let background = dataset
            .add_derived(
                dataset.active_matrix().clone(),
                SignalUnit::MilliDeltaOpticalDensity,
                wavelength.clone(),
                delay.clone(),
                "Background corrected",
                ProcessingOp::BackgroundPreZero(PreZeroBackground {
                    range: PreZeroRange::FirstPoints { count: 1 },
                }),
                vec![raw],
            )
            .unwrap();
        let cropped = dataset
            .add_derived(
                dataset.active_matrix().clone(),
                SignalUnit::MilliDeltaOpticalDensity,
                wavelength,
                delay,
                "Cropped",
                crop_op(),
                vec![background],
            )
            .unwrap();

        assert_eq!(dataset.children(raw), vec![background]);
        assert_eq!(dataset.children(background), vec![cropped]);
        assert_eq!(dataset.ancestry(cropped), vec![raw, background, cropped]);
        assert_eq!(dataset.active_version, cropped);
    }

    #[test]
    fn each_version_keeps_the_axes_it_was_computed_on() {
        let mut dataset = dataset();
        let raw = dataset.raw_version().id;
        let (wavelength, delay) = axes();
        // A crop that keeps only the middle wavelength and the last two delays.
        let cropped_wavelength = Axis::new(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            "Wavelength",
            vec![510.0],
        )
        .unwrap();
        let cropped_delay = Axis::new(
            AxisKind::Delay,
            AxisUnit::Picosecond,
            "Delay",
            vec![1.0, 2.0],
        )
        .unwrap();
        let cropped = dataset
            .add_derived(
                array![[0.07, 0.08]],
                SignalUnit::MilliDeltaOpticalDensity,
                cropped_wavelength,
                cropped_delay,
                "Cropped",
                crop_op(),
                vec![raw],
            )
            .unwrap();

        // The version's own axes describe its matrix ...
        assert_eq!(dataset.axes(cropped).unwrap().0.values(), &[510.0]);
        assert_eq!(dataset.axes(cropped).unwrap().1.values(), &[1.0, 2.0]);
        assert_eq!(dataset.shape_of(cropped).unwrap(), (1, 2));
        // ... and the raw version keeps the original grid.
        assert_eq!(dataset.axes(raw).unwrap().0.values(), &[500.0, 510.0, 520.0]);
        assert_eq!(dataset.shape_of(raw).unwrap(), (3, 4));
        // The dataset-level accessors follow the active version.
        assert_eq!(dataset.wavelength().values(), &[510.0]);
        assert_eq!(dataset.delay().values(), &[1.0, 2.0]);
        let _ = (wavelength, delay);
    }

    #[test]
    fn removing_a_version_removes_its_descendants_and_frees_their_blocks() {
        let mut dataset = dataset();
        let raw = dataset.raw_version().id;
        let (wavelength, delay) = axes();
        let a = dataset
            .add_derived(
                dataset.active_matrix().clone(),
                SignalUnit::DeltaOpticalDensity,
                wavelength.clone(),
                delay.clone(),
                "a",
                crop_op(),
                vec![raw],
            )
            .unwrap();
        let b = dataset
            .add_derived(
                dataset.active_matrix().clone(),
                SignalUnit::DeltaOpticalDensity,
                wavelength,
                delay,
                "b",
                crop_op(),
                vec![a],
            )
            .unwrap();

        assert_eq!(dataset.blocks.len(), 3);
        let removed = dataset.remove_version(a).unwrap();
        assert_eq!(removed.versions, vec![a, b]);
        assert!(dataset.version(a).is_err());
        assert!(dataset.version(b).is_err());
        assert_eq!(dataset.active_version, raw);
        assert!(dataset.children(raw).is_empty());
        // Only the raw block survives, and the raw version still points at it.
        assert_eq!(dataset.blocks.len(), 1);
        assert_eq!(dataset.version(raw).unwrap().block, 0);
        assert_eq!(dataset.matrix(raw).unwrap().dim(), (3, 4));
    }

    #[test]
    fn a_block_that_has_no_numbers_refuses_before_returning_zeros() {
        let mut dataset = dataset();
        dataset.blocks[0].data = Array2::zeros((0, 0));
        assert!(!dataset.blocks[0].has_data());
        assert!(matches!(
            dataset.matrix(dataset.raw_version().id),
            Err(CoreError::MissingMatrixData { .. })
        ));
        assert!(dataset.validate().is_ok());
    }

    #[test]
    fn the_raw_matrix_cannot_be_removed() {
        let mut dataset = dataset();
        let raw = dataset.raw_version().id;
        assert!(dataset.remove_version(raw).is_err());
    }

    #[test]
    fn duplicating_a_dataset_gives_it_a_new_identity() {
        let dataset = dataset();
        let copy = dataset.duplicate("NIR800 copy");
        assert_ne!(copy.id, dataset.id);
        assert_eq!(copy.name, "NIR800 copy");
        assert_eq!(copy.shape(), dataset.shape());
        assert_eq!(copy.active_matrix(), dataset.active_matrix());
    }

    #[test]
    fn block_range_ignores_missing_values() {
        let (wavelength, delay) = axes();
        let block = MatrixBlock::new(
            wavelength,
            delay,
            array![
                [1.0, f64::NAN, 0.0, 0.0],
                [-3.0, 2.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 0.0]
            ],
            SignalUnit::DeltaOpticalDensity,
        )
        .unwrap();
        assert_eq!(block.range(), Some((-3.0, 2.0)));
    }
}
