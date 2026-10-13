//! The project: every dataset, comparison workspace and figure, in the order
//! the user made them.
//!
//! Removing an object never touches the files it came from, and prunes the
//! references that pointed at it, reporting what it dropped.

use crate::dataset::TaDataset;
use crate::error::{CoreError, Result};
use crate::figure::FigureModel;
use crate::figure::SeriesSource;
use crate::ids::{ComparisonId, DatasetId, FigureId, VersionId};
use serde::{Deserialize, Serialize};

/// The schema version written into a `.taproject` manifest.
pub const PROJECT_SCHEMA: u32 = 1;

/// How a comparison normalizes the curves it holds.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Normalization {
    None,
    /// Divide by the largest magnitude in the visible delay range.
    MaxAbs,
    /// Divide by the amplitude magnitude at one delay, preserving the signal sign.
    AtDelay(f64),
}

/// How a comparison lines up datasets that were measured on different clocks.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Alignment {
    /// Leave the axes alone.
    AsMeasured,
    /// Shift each dataset so its fitted time zero lands on zero.
    ToTimeZero,
}

/// A workspace that compares several datasets without copying their matrices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComparisonWorkspace {
    pub id: ComparisonId,
    pub name: String,
    pub series: Vec<crate::figure::PlotSeries>,
    pub normalization: Normalization,
    pub alignment: Alignment,
}

impl ComparisonWorkspace {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: ComparisonId::new(),
            name: name.into(),
            series: Vec::new(),
            normalization: Normalization::None,
            alignment: Alignment::AsMeasured,
        }
    }

    /// The datasets this workspace references, without holding their data.
    pub fn dataset_references(&self) -> Vec<DatasetId> {
        let mut out: Vec<DatasetId> = Vec::new();
        for series in &self.series {
            let id = match &series.source {
                SeriesSource::Trace(selection) => selection.dataset,
                SeriesSource::Spectrum(selection) => selection.dataset,
                SeriesSource::Heatmap { dataset, .. }
                | SeriesSource::Contour { dataset, .. }
                | SeriesSource::ContourLines { dataset, .. }
                | SeriesSource::Fit { dataset, .. } => *dataset,
                SeriesSource::Curve { .. } => continue,
            };
            if !out.contains(&id) {
                out.push(id);
            }
        }
        out
    }
}

/// What removing an object took with it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RemovalReport {
    pub pruned_series: usize,
    pub removed_figures: Vec<FigureId>,
    pub removed_comparisons: Vec<ComparisonId>,
}

/// A whole session's work.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub schema: u32,
    pub name: String,
    pub datasets: Vec<TaDataset>,
    pub comparisons: Vec<ComparisonWorkspace>,
    pub figures: Vec<FigureModel>,
    /// Named parameter sets the user kept, so a fit can be repeated elsewhere.
    #[serde(default)]
    pub presets: Vec<crate::fitting::ParameterPreset>,
    /// The window's panel arrangement. Opaque to the data layer: the interface
    /// owns its own chrome.
    pub layout: Option<serde_json::Value>,
}

impl Project {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            schema: PROJECT_SCHEMA,
            name: name.into(),
            datasets: Vec::new(),
            comparisons: Vec::new(),
            figures: Vec::new(),
            presets: Vec::new(),
            layout: None,
        }
    }

    pub fn add_dataset(&mut self, dataset: TaDataset) -> DatasetId {
        let id = dataset.id;
        self.datasets.push(dataset);
        id
    }

    pub fn dataset(&self, id: DatasetId) -> Result<&TaDataset> {
        self.datasets
            .iter()
            .find(|dataset| dataset.id == id)
            .ok_or_else(|| CoreError::UnknownDataset(id.to_string()))
    }

    pub fn dataset_mut(&mut self, id: DatasetId) -> Result<&mut TaDataset> {
        self.datasets
            .iter_mut()
            .find(|dataset| dataset.id == id)
            .ok_or_else(|| CoreError::UnknownDataset(id.to_string()))
    }

    pub fn rename_dataset(&mut self, id: DatasetId, name: impl Into<String>) -> Result<()> {
        self.dataset_mut(id)?.name = name.into();
        Ok(())
    }

    pub fn figure(&self, id: FigureId) -> Result<&FigureModel> {
        self.figures
            .iter()
            .find(|figure| figure.id == id)
            .ok_or_else(|| CoreError::invalid(format!("no figure {id} in this project")))
    }

    pub fn add_figure(&mut self, figure: FigureModel) -> FigureId {
        let id = figure.id;
        self.figures.push(figure);
        id
    }

    /// Removes a version and everything that read it.
    ///
    /// The graph, the saved traces and spectra, the fits, and every figure
    /// series pointing at this version go together: a project that kept a curve
    /// whose matrix is gone could not be reopened.
    pub fn remove_version(
        &mut self,
        dataset: DatasetId,
        version: VersionId,
    ) -> Result<RemovalReport> {
        let removal = self.dataset_mut(dataset)?.remove_version(version)?;
        let mut report = RemovalReport {
            pruned_series: removal.pruned_series,
            ..RemovalReport::default()
        };

        for figure in &mut self.figures {
            for panel in &mut figure.panels {
                let before = panel.series.len();
                panel.series.retain(|series| {
                    !crate::figure::series_references_version(series, dataset, version)
                });
                report.pruned_series += before - panel.series.len();
            }
        }
        for comparison in &mut self.comparisons {
            let before = comparison.series.len();
            comparison.series.retain(|series| {
                !crate::figure::series_references_version(series, dataset, version)
            });
            report.pruned_series += before - comparison.series.len();
        }
        report.removed_figures = self.prune_empty_figures();
        report.removed_comparisons = self.prune_empty_comparisons();
        Ok(report)
    }

    /// Drops figures that no longer have any series, and reports which.
    fn prune_empty_figures(&mut self) -> Vec<FigureId> {
        let empty: Vec<FigureId> = self
            .figures
            .iter()
            .filter(|figure| {
                figure
                    .panels
                    .iter()
                    .all(|panel| panel.series.is_empty())
            })
            .map(|figure| figure.id)
            .collect();
        self.figures.retain(|figure| !empty.contains(&figure.id));
        empty
    }

    fn prune_empty_comparisons(&mut self) -> Vec<ComparisonId> {
        let empty: Vec<ComparisonId> = self
            .comparisons
            .iter()
            .filter(|comparison| comparison.series.is_empty())
            .map(|comparison| comparison.id)
            .collect();
        self.comparisons
            .retain(|comparison| !empty.contains(&comparison.id));
        empty
    }

    /// Removes a dataset and every reference to it. Source files are not
    /// touched: that is a separate, explicit operation.
    pub fn remove_dataset(&mut self, id: DatasetId) -> Result<RemovalReport> {
        let position = self
            .datasets
            .iter()
            .position(|dataset| dataset.id == id)
            .ok_or_else(|| CoreError::UnknownDataset(id.to_string()))?;
        self.datasets.remove(position);

        let mut report = RemovalReport::default();
        for figure in &mut self.figures {
            for panel in &mut figure.panels {
                let before = panel.series.len();
                panel.series
                    .retain(|series| !series_references(series, id));
                report.pruned_series += before - panel.series.len();
            }
        }
        report.removed_figures = self.prune_empty_figures();

        for comparison in &mut self.comparisons {
            let before = comparison.series.len();
            comparison
                .series
                .retain(|series| !series_references(series, id));
            report.pruned_series += before - comparison.series.len();
        }
        report.removed_comparisons = self.prune_empty_comparisons();

        Ok(report)
    }

    /// Every version id in the project, for integrity checks.
    pub fn version_ids(&self) -> Vec<(DatasetId, VersionId)> {
        let mut out = Vec::new();
        for dataset in &self.datasets {
            for version in &dataset.versions {
                out.push((dataset.id, version.id));
            }
        }
        out
    }

    /// Checks every cross-reference in the project.
    ///
    /// A project that loads with a dangling reference is worse than one that
    /// refuses to load, so this is run on save, on load, and in tests.
    pub fn validate(&self) -> Result<()> {
        if self.schema > PROJECT_SCHEMA {
            return Err(CoreError::ProjectTooNew {
                found: self.schema,
                supported: PROJECT_SCHEMA,
            });
        }
        for dataset in &self.datasets {
            dataset.validate()?;
            for figure in &dataset.figures {
                self.validate_figure(figure)?;
            }
        }
        for figure in &self.figures {
            self.validate_figure(figure)?;
        }
        for comparison in &self.comparisons {
            for series in &comparison.series {
                self.validate_series(series)?;
            }
        }
        Ok(())
    }

    fn validate_figure(&self, figure: &FigureModel) -> Result<()> {
        for panel in &figure.panels {
            for series in &panel.series {
                self.validate_series(series)?;
            }
        }
        Ok(())
    }

    fn validate_series(&self, series: &crate::figure::PlotSeries) -> Result<()> {
        let (dataset_id, version) = match &series.source {
            SeriesSource::Trace(selection) => (selection.dataset, Some(selection.version)),
            SeriesSource::Spectrum(selection) => (selection.dataset, Some(selection.version)),
            SeriesSource::Heatmap { dataset, version }
            | SeriesSource::Contour { dataset, version }
            | SeriesSource::ContourLines { dataset, version } => (*dataset, Some(*version)),
            SeriesSource::Fit {
                dataset,
                version,
                selection,
                ..
            } => {
                self.dataset(*dataset)?.version(selection.version)?;
                (*dataset, Some(*version))
            }
            SeriesSource::Curve { .. } => return Ok(()),
        };
        let dataset = self.dataset(dataset_id)?;
        if let Some(version) = version {
            dataset.version(version)?;
        }
        Ok(())
    }

    pub fn dataset_count(&self) -> usize {
        self.datasets.len()
    }

    /// One line for the status bar.
    pub fn summary(&self) -> String {
        format!(
            "{} · {} dataset(s) · {} figure(s)",
            self.name,
            self.datasets.len(),
            self.figures.len()
        )
    }
}

fn series_references(series: &crate::figure::PlotSeries, id: DatasetId) -> bool {
    crate::figure::series_references_dataset(series, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::axis::Axis;
    use crate::dataset::SourceFormat;
    use crate::figure::{FigureModel, PlotSeries, SeriesSource};
    use crate::selection::{AxisRequest, TraceSelection};
    use crate::units::{AxisKind, AxisUnit, SignalUnit};
    use ndarray::array;

    fn dataset(name: &str) -> TaDataset {
        let wavelength = Axis::new(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            "Wavelength",
            vec![500.0, 510.0],
        )
        .unwrap();
        let delay = Axis::new(
            AxisKind::Delay,
            AxisUnit::Picosecond,
            "Delay",
            vec![0.0, 1.0],
        )
        .unwrap();
        TaDataset::new_raw(
            name,
            wavelength,
            delay,
            array![[0.0, 1.0], [0.1, 1.1]],
            SignalUnit::DeltaOpticalDensity,
            SourceFormat::Synthetic,
        )
        .unwrap()
    }

    fn figure_with_trace(dataset: &TaDataset) -> FigureModel {
        let mut figure = FigureModel::single_panel("kinetics");
        let selection = TraceSelection::new(
            dataset.id,
            dataset.raw_version().id,
            AxisRequest::nearest(510.0),
        );
        figure.panels[0].series.push(PlotSeries::new(
            "1 ps",
            SeriesSource::Trace(selection),
        ));
        figure
    }

    #[test]
    fn a_valid_project_validates_and_an_empty_one_too() {
        let project = Project::new("untitled");
        project.validate().expect("an empty project is valid");
    }

    #[test]
    fn a_dangling_series_reference_is_caught() {
        let mut project = Project::new("p");
        let dataset = dataset("a");
        let figure = figure_with_trace(&dataset);
        project.add_dataset(dataset);
        project.figures.push(figure);

        // Corrupt the figure's reference on purpose.
        if let SeriesSource::Trace(selection) = &mut project.figures[0].panels[0].series[0].source {
            selection.version = VersionId::new();
        }
        assert!(project.validate().is_err());
    }

    #[test]
    fn removing_a_dataset_prunes_its_series_and_reports_what_went() {
        let mut project = Project::new("p");
        let dataset = dataset("a");
        let id = dataset.id;
        let figure = figure_with_trace(&dataset);
        let figure_id = figure.id;
        project.add_dataset(dataset);
        project.figures.push(figure);
        project.validate().unwrap();

        let report = project.remove_dataset(id).unwrap();
        assert_eq!(report.pruned_series, 1);
        assert_eq!(report.removed_figures, vec![figure_id]);
        assert!(project.figures.is_empty());
        project.validate().unwrap();
    }

    #[test]
    fn removing_a_dataset_that_does_not_exist_is_an_error() {
        let mut project = Project::new("p");
        assert!(project.remove_dataset(DatasetId::new()).is_err());
    }

    #[test]
    fn renaming_a_dataset_keeps_its_references_intact() {
        let mut project = Project::new("p");
        let dataset = dataset("a");
        let id = dataset.id;
        project.add_dataset(dataset);
        let figure = figure_with_trace(project.dataset(id).unwrap());
        project.figures.push(figure);

        project.rename_dataset(id, "sample A, 480 nm").unwrap();
        assert_eq!(project.dataset(id).unwrap().name, "sample A, 480 nm");
        project.validate().unwrap();
    }

    #[test]
    fn a_project_from_a_newer_build_refuses_to_load() {
        let mut project = Project::new("future");
        project.schema = PROJECT_SCHEMA + 1;
        assert!(matches!(
            project.validate(),
            Err(CoreError::ProjectTooNew { .. })
        ));
    }

    #[test]
    fn a_block_that_disagrees_with_its_axes_fails_validation() {
        let mut project = Project::new("p");
        let mut dataset = dataset("a");
        // A payload that no longer matches the axes it claims to be sampled on.
        dataset.blocks[0].data = ndarray::Array2::zeros((7, 2));
        project.add_dataset(dataset);
        assert!(matches!(
            project.validate(),
            Err(CoreError::ShapeMismatch { .. })
        ));
    }
}
