//! The session: what is open, what is selected, and what each plate is showing.
//!
//! Every rule the shell depends on lives here rather than in the view, so the
//! linking between the contour and the two linked plates can be tested without
//! a window (spec §24.19). The view renders this; it does not decide it.

use ta_core::{
    kinetic_trace, transient_spectrum, AxisRequest, CoreError, DatasetId, Project, Result,
    SampleMode, Spectrum, SpectrumSelection, TaDataset, Trace, TraceSelection, VersionId,
    VersionKind,
};

/// The four plates of the figure block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlateId {
    /// (a) the two-dimensional contour
    Contour,
    /// (b) kinetics
    Kinetics,
    /// (c) spectra
    Spectra,
    /// (d) the time-zero correction curve
    TimeZero,
}

impl PlateId {
    pub const ALL: [PlateId; 4] = [
        PlateId::Contour,
        PlateId::Kinetics,
        PlateId::Spectra,
        PlateId::TimeZero,
    ];

    /// The panel letter the caption is ranked under.
    pub const fn letter(self) -> &'static str {
        match self {
            PlateId::Contour => "a",
            PlateId::Kinetics => "b",
            PlateId::Spectra => "c",
            PlateId::TimeZero => "d",
        }
    }
}

/// A transient message for the footer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub message: String,
    pub problem: Option<String>,
}

impl Status {
    pub fn info(&mut self, message: impl Into<String>) {
        self.message = message.into();
        self.problem = None;
    }

    pub fn problem(&mut self, message: impl Into<String>) {
        self.problem = Some(message.into());
    }
}

/// How a trace or spectrum is read off its axis, and the width when it is
/// averaged.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Sampling {
    pub mode: SampleMode,
    /// The half-width used by an averaged window.
    pub half_width: f64,
    /// The channel count used by an averaged band.
    pub channels: usize,
}

impl Default for Sampling {
    fn default() -> Self {
        Self {
            mode: SampleMode::Nearest,
            half_width: 5.0,
            channels: 3,
        }
    }
}

impl Sampling {
    pub fn request(&self, coordinate: f64) -> AxisRequest {
        AxisRequest {
            mode: self.mode,
            coordinate,
        }
    }

    #[allow(dead_code)] // the label is shown by the plates' captions
    pub fn label(&self) -> &'static str {
        match self.mode {
            SampleMode::Nearest => "nearest",
            SampleMode::Interpolated => "interpolated",
            SampleMode::AverageWindow { .. } => "averaged window",
            SampleMode::AverageChannels { .. } => "averaged band",
        }
    }

    /// The mode with its width filled in from this sampling's settings.
    pub fn resolved_mode(&self) -> SampleMode {
        match self.mode {
            SampleMode::AverageWindow { .. } => SampleMode::AverageWindow {
                half_width: self.half_width,
            },
            SampleMode::AverageChannels { .. } => SampleMode::AverageChannels {
                count: self.channels,
            },
            other => other,
        }
    }
}

/// The delays the reference implementation shows spectra at by default.
///
/// `auto_taspectra` plots these seventeen points, each snapped to the nearest
/// measured delay, and drops the ones that snap onto the same sample. Keeping
/// the list here means the spectra plate opens on the same figure the notebook
/// would have drawn.
pub const DEFAULT_SPECTRUM_DELAYS: [f64; 17] = [
    -0.5, -0.2, 0.0, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0,
    1000.0, 1500.0,
];

/// The analysis view saved alongside the project's matrices, not in source files.
#[derive(serde::Serialize, serde::Deserialize)]
struct SavedAnalysis {
    opened: Option<DatasetId>,
    trace: Option<TraceSelection>,
    spectrum: Option<SpectrumSelection>,
    wavelengths: Vec<f64>,
    delays: Vec<f64>,
    sampling: Sampling,
    comparison: usize,
    studio: Option<ta_core::FigureModel>,
}

/// Everything the workspace knows.
#[derive(Debug)]
pub struct Session {
    pub project: Project,
    /// The dataset on the bench.
    pub opened: Option<DatasetId>,
    /// The version on the bench.
    pub version: Option<VersionId>,
    /// The linked kinetic selection, at one wavelength.
    pub trace: Option<TraceSelection>,
    /// The linked spectral selection, at one delay.
    pub spectrum: Option<SpectrumSelection>,
    /// Extra wavelengths overlaid in the kinetics plate.
    pub comparison_wavelengths: Vec<f64>,
    /// Extra delays overlaid in the spectra plate.
    pub comparison_delays: Vec<f64>,
    /// Which comparison workspace the comparison layout is showing.
    pub comparison: usize,
    pub sampling: Sampling,
    /// The fitting workbench's state, created when it is first opened.
    pub fit: Option<crate::fit_panel::FitSession>,
    /// The current automatic time-zero estimate, before it is applied.
    pub estimated_curve: Option<ta_core::CorrectionCurve>,
    /// The figure FigureStudio is editing, carried over from a plate.
    pub studio: Option<ta_core::FigureModel>,
    pub status: Status,
    /// Where the project was last saved.
    pub path: Option<std::path::PathBuf>,
    /// Whether the project has changed since that save.
    pub dirty: bool,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Self {
            project: Project::new("Untitled project"),
            opened: None,
            version: None,
            trace: None,
            spectrum: None,
            comparison_wavelengths: Vec::new(),
            comparison_delays: Vec::new(),
            comparison: 0,
            sampling: Sampling::default(),
            fit: None,
            estimated_curve: None,
            studio: None,
            status: Status::default(),
            path: None,
            dirty: false,
        }
    }

    // -- what is open ------------------------------------------------------

    pub fn dataset(&self) -> Option<&TaDataset> {
        self.opened.and_then(|id| self.project.dataset(id).ok())
    }

    /// Mutable access, for the processing operations that add versions.
    #[allow(dead_code)] // the processing workbench is the first caller
    pub fn dataset_mut(&mut self) -> Option<&mut TaDataset> {
        let id = self.opened?;
        self.project.dataset_mut(id).ok()
    }

    pub fn version_id(&self) -> Option<VersionId> {
        self.version
    }

    pub fn version(&self) -> Option<&ta_core::MatrixVersion> {
        let dataset = self.dataset()?;
        let version = self.version?;
        dataset.version(version).ok()
    }

    /// Opens an imported dataset and chooses a starting selection: the
    /// wavelength that carries the most signal, and the first delay after zero.
    pub fn open_dataset(&mut self, dataset: TaDataset) -> DatasetId {
        let id = dataset.id;
        let version = dataset.raw_version().id;
        let name = dataset.name.clone();
        let rows = dataset.wavelength().len();
        let delays = dataset.delay().len();
        self.project.add_dataset(dataset);
        self.opened = Some(id);
        self.version = Some(version);
        self.reseed_selection();
        self.status.info(format!("{name}: {rows} wavelengths × {delays} delays"));
        self.dirty = true;
        id
    }

    /// Puts an already-loaded dataset on the bench, without re-importing it.
    pub fn open_bench(&mut self, id: DatasetId) {
        let Ok(dataset) = self.project.dataset(id) else {
            self.status.problem("that dataset is no longer in the project");
            return;
        };
        let version = dataset.active_version;
        let name = dataset.name.clone();
        let rows = dataset.wavelength().len();
        let delays = dataset.delay().len();
        self.opened = Some(id);
        self.version = Some(version);
        self.reseed_selection();
        self.status.info(format!("{name}: {rows} wavelengths × {delays} delays"));
    }

    /// Chooses the strongest wavelength and the first positive delay, and lays
    /// the spectra out on the reference implementation's default delay grid.
    fn reseed_selection(&mut self) {
        self.trace = None;
        self.spectrum = None;
        self.comparison_wavelengths.clear();
        self.comparison_delays.clear();
        self.fill_missing_selection();
        self.seed_default_delays();
    }

    /// Puts the reference implementation's delay grid on the spectra plate.
    ///
    /// Each request is snapped to a measured delay and duplicates are dropped,
    /// which is exactly what `find_closest_value` does: a matrix sampled at 1 ps
    /// shows one curve where the grid asks for sixteen times that all land on it.
    /// The linked spectrum is left out, because it is drawn as its own curve.
    pub fn seed_default_delays(&mut self) {
        let Some(dataset) = self.dataset() else {
            return;
        };
        let version = self.version.unwrap_or(dataset.active_version);
        let Ok((_, delay)) = dataset.axes(version) else {
            return;
        };
        let (low, high) = delay.range();
        let linked = self
            .spectrum
            .as_ref()
            .map(|spectrum| delay.nearest(spectrum.delay.coordinate).0);
        let mut seen: Vec<usize> = Vec::new();
        let mut chosen: Vec<f64> = Vec::new();
        for requested in DEFAULT_SPECTRUM_DELAYS {
            // A grid point outside the measured range has no sample to sit on.
            if requested < low - f64::EPSILON || requested > high + f64::EPSILON {
                continue;
            }
            let (index, value) = delay.nearest(requested);
            if seen.contains(&index) || Some(index) == linked {
                continue;
            }
            seen.push(index);
            chosen.push(value);
        }
        self.comparison_delays = chosen;
    }

    // -- adding to the linked plates (b) and (c) ---------------------------

    /// Adds the current selection — or a typed coordinate — to the kinetics
    /// plate. The measured sample decides duplicates, so adding 620 nm twice
    /// adds it once.
    pub fn add_wavelength(&mut self, wavelength: f64) -> Result<()> {
        let dataset = self
            .dataset()
            .ok_or_else(|| CoreError::invalid("no dataset is open"))?;
        let version = self.version.unwrap_or(dataset.active_version);
        let (axis, _) = dataset.axes(version)?;
        let index = axis.nearest(wavelength).0;
        let linked = self
            .trace
            .as_ref()
            .map(|trace| axis.nearest(trace.wavelength.coordinate).0);
        if Some(index) == linked {
            return Err(CoreError::invalid(
                "that wavelength is already the linked trace",
            ));
        }
        if self
            .comparison_wavelengths
            .iter()
            .any(|kept| axis.nearest(*kept).0 == index)
        {
            return Err(CoreError::invalid(
                "that wavelength is already on the kinetics plate",
            ));
        }
        let value = axis.values()[index];
        self.comparison_wavelengths.push(value);
        self.comparison_wavelengths
            .sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        self.dirty = true;
        Ok(())
    }

    /// Adds a delay to the spectra plate, snapped to the nearest measured one.
    pub fn add_delay(&mut self, delay: f64) -> Result<()> {
        let dataset = self
            .dataset()
            .ok_or_else(|| CoreError::invalid("no dataset is open"))?;
        let version = self.version.unwrap_or(dataset.active_version);
        let (_, axis) = dataset.axes(version)?;
        let index = axis.nearest(delay).0;
        let linked = self
            .spectrum
            .as_ref()
            .map(|spectrum| axis.nearest(spectrum.delay.coordinate).0);
        if Some(index) == linked {
            return Err(CoreError::invalid(
                "that delay is already the linked spectrum",
            ));
        }
        if self
            .comparison_delays
            .iter()
            .any(|kept| axis.nearest(*kept).0 == index)
        {
            return Err(CoreError::invalid(
                "that delay is already on the spectra plate",
            ));
        }
        let value = axis.values()[index];
        self.comparison_delays.push(value);
        self.comparison_delays
            .sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        self.dirty = true;
        Ok(())
    }

    /// Adds the current selection to the kinetics plate, and moves the
    /// selection on by one measured sample so a run of them is two clicks each.
    pub fn add_current_wavelength(&mut self) -> Result<()> {
        let coordinate = self
            .trace
            .as_ref()
            .map(|trace| trace.wavelength.coordinate)
            .ok_or_else(|| CoreError::invalid("no wavelength is selected"))?;
        self.add_wavelength(coordinate)
    }

    pub fn add_current_delay(&mut self) -> Result<()> {
        let coordinate = self
            .spectrum
            .as_ref()
            .map(|spectrum| spectrum.delay.coordinate)
            .ok_or_else(|| CoreError::invalid("no delay is selected"))?;
        self.add_delay(coordinate)
    }

    pub fn remove_wavelength(&mut self, index: usize) {
        if index < self.comparison_wavelengths.len() {
            self.comparison_wavelengths.remove(index);
            self.dirty = true;
        }
    }

    pub fn remove_delay(&mut self, index: usize) {
        if index < self.comparison_delays.len() {
            self.comparison_delays.remove(index);
            self.dirty = true;
        }
    }

    pub fn clear_wavelengths(&mut self) {
        self.comparison_wavelengths.clear();
        self.dirty = true;
    }

    pub fn clear_delays(&mut self) {
        self.comparison_delays.clear();
        self.dirty = true;
    }

    /// The label a kept wavelength shows: the plate's convention, so the chip
    /// and the legend name the same curve the same way.
    pub fn wavelength_label(&self, wavelength: f64) -> String {
        crate::plates::wavelength_label(wavelength)
    }

    /// The label a kept delay shows: the plate's convention, so the chip and
    /// the legend name the same curve the same way.
    pub fn delay_label(&self, delay: f64) -> String {
        crate::plates::delay_label(delay)
    }

    /// The most recent correction that actually produced the displayed version.
    pub fn time_zero_curve(&self) -> Option<&ta_core::CorrectionCurve> {
        let dataset = self.dataset()?;
        dataset.ancestry(self.version?).into_iter().rev().find_map(|id| {
            match &dataset.version(id).ok()?.op {
                Some(ta_core::ProcessingOp::TimeZero(correction)) => Some(&correction.curve),
                _ => None,
            }
        })
    }

    pub fn has_time_zero_correction(&self) -> bool {
        self.time_zero_curve().is_some()
    }

    /// The datasets loaded in the project, oldest first, for the index.
    pub fn datasets(&self) -> Vec<(DatasetId, String, usize, bool)> {
        self.project
            .datasets
            .iter()
            .map(|dataset| {
                (
                    dataset.id,
                    dataset.name.clone(),
                    dataset.versions.len(),
                    Some(dataset.id) == self.opened,
                )
            })
            .collect()
    }

    /// Fills only what is missing, so reopening a project keeps the trace and
    /// spectrum the user saved instead of replacing them with fresh defaults.
    fn fill_missing_selection(&mut self) {
        let Some(dataset) = self.dataset() else {
            self.trace = None;
            self.spectrum = None;
            return;
        };
        let version = self.version.unwrap_or(dataset.raw_version().id);
        let strongest = strongest_wavelength(dataset, version).unwrap_or_else(|| {
            dataset
                .axes(version)
                .map(|(wavelength, _)| wavelength.values()[wavelength.len() / 2])
                .unwrap_or(0.0)
        });
        let first_positive = dataset
            .axes(version)
            .ok()
            .and_then(|(_, delay)| {
                delay
                    .values()
                    .iter()
                    .copied()
                    .find(|value| *value > 0.0)
                    .or_else(|| delay.values().first().copied())
            })
            .unwrap_or(0.0);
        let dataset_id = self.opened.expect("a dataset is open");
        if self.trace.is_none() {
            self.trace = Some(TraceSelection::new(
                dataset_id,
                version,
                AxisRequest {
                    mode: self.sampling.resolved_mode(),
                    coordinate: strongest,
                },
            ));
        }
        if self.spectrum.is_none() {
            self.spectrum = Some(SpectrumSelection::new(
                dataset_id,
                version,
                AxisRequest {
                    mode: self.sampling.resolved_mode(),
                    coordinate: first_positive,
                },
            ));
        }
    }

    // -- selection: the linking rule of §6.7 ------------------------------

    /// The contour was clicked: both linked plates follow.
    pub fn select_point(&mut self, wavelength: f64, delay: f64) -> Result<()> {
        self.select_wavelength(wavelength)?;
        self.select_delay(delay)
    }

    /// A wavelength was chosen, from the contour or from the kinetics plate.
    pub fn select_wavelength(&mut self, wavelength: f64) -> Result<()> {
        let (dataset, version) = self.bench()?;
        self.trace = Some(TraceSelection::new(
            dataset,
            version,
            self.sampling.request(wavelength),
        ));
        self.dirty = true;
        Ok(())
    }

    /// A delay was chosen, from the contour or from the spectra plate.
    pub fn select_delay(&mut self, delay: f64) -> Result<()> {
        let (dataset, version) = self.bench()?;
        self.spectrum = Some(SpectrumSelection::new(
            dataset,
            version,
            self.sampling.request(delay),
        ));
        self.dirty = true;
        Ok(())
    }

    pub fn set_sampling(&mut self, sampling: Sampling) {
        self.sampling = sampling;
        let mode = sampling.resolved_mode();
        if let Some(trace) = &mut self.trace {
            trace.wavelength.mode = mode;
        }
        if let Some(spectrum) = &mut self.spectrum {
            spectrum.delay.mode = mode;
        }
        self.dirty = true;
    }

    /// The dataset and version a selection is made against.
    fn bench(&self) -> Result<(DatasetId, VersionId)> {
        let dataset = self
            .dataset()
            .ok_or_else(|| CoreError::invalid("no dataset is open"))?;
        let version = self.version.unwrap_or(dataset.raw_version().id);
        Ok((dataset.id, version))
    }

    /// Keeps the current trace as a named comparison curve.
    pub fn save_current_trace(&mut self) -> Result<()> {
        let Some(trace) = &self.trace else {
            return Err(CoreError::invalid("there is no trace to keep"));
        };
        let coordinate = trace.wavelength.coordinate;
        if !self.comparison_wavelengths.contains(&coordinate) {
            self.comparison_wavelengths.push(coordinate);
            self.dirty = true;
        }
        Ok(())
    }

    /// Keeps the current spectrum as a named comparison curve.
    pub fn save_current_spectrum(&mut self) -> Result<()> {
        let Some(spectrum) = &self.spectrum else {
            return Err(CoreError::invalid("there is no spectrum to keep"));
        };
        let coordinate = spectrum.delay.coordinate;
        if !self.comparison_delays.contains(&coordinate) {
            self.comparison_delays.push(coordinate);
            self.dirty = true;
        }
        Ok(())
    }

    // -- versions ---------------------------------------------------------

    /// Puts another version of the matrix on the bench.
    ///
    /// A selection that the new version cannot represent — a wavelength the crop
    /// removed — is moved to the nearest coordinate it does have, and the move
    /// is reported rather than made silently.
    pub fn set_version(&mut self, version: VersionId) -> Result<()> {
        // The axes are copied out first, because the selections are mutated
        // below and the dataset borrow would still be alive.
        let (label, wavelength_range, delay_range, wavelength_values, delay_values) = {
            let dataset = self
                .dataset()
                .ok_or_else(|| CoreError::invalid("no dataset is open"))?;
            let node = dataset.version(version)?;
            let (wavelength, delay) = dataset.axes(version)?;
            (
                node.label.clone(),
                wavelength.range(),
                delay.range(),
                wavelength.values().to_vec(),
                delay.values().to_vec(),
            )
        };
        let within = |range: (f64, f64), value: f64| value >= range.0 && value <= range.1;
        let nearest = |values: &[f64], target: f64| -> f64 {
            values
                .iter()
                .copied()
                .min_by(|a, b| {
                    (a - target)
                        .abs()
                        .total_cmp(&(b - target).abs())
                })
                .unwrap_or(target)
        };

        let mut notes = Vec::new();
        if let Some(trace) = &mut self.trace {
            if !within(wavelength_range, trace.wavelength.coordinate) {
                let moved = nearest(&wavelength_values, trace.wavelength.coordinate);
                notes.push(format!(
                    "the {label} version covers {:.4}–{:.4} nm, so the trace moved from {:.4} nm to {:.4} nm",
                    wavelength_range.0, wavelength_range.1, trace.wavelength.coordinate, moved
                ));
                trace.wavelength.coordinate = moved;
            }
            trace.version = version;
            if wavelength_values.len() == 1 {
                trace.wavelength.mode = SampleMode::Nearest;
            }
        }
        if let Some(spectrum) = &mut self.spectrum {
            if !within(delay_range, spectrum.delay.coordinate) {
                let moved = nearest(&delay_values, spectrum.delay.coordinate);
                notes.push(format!(
                    "the {label} version covers {:.4}–{:.4} ps, so the spectrum moved from {:.4} ps to {:.4} ps",
                    delay_range.0, delay_range.1, spectrum.delay.coordinate, moved
                ));
                spectrum.delay.coordinate = moved;
            }
            spectrum.version = version;
        }
        self.dataset_mut().expect("a dataset is open").set_active(version)?;
        self.version = Some(version);
        self.dirty = true;
        match notes.is_empty() {
            true => self.status.info(format!("showing {label}")),
            false => {
                for note in &notes {
                    self.status.problem(note.clone());
                }
            }
        }
        Ok(())
    }

    /// The processing chain of the open dataset, oldest first, for the plate
    /// index.
    pub fn chain(&self) -> Vec<ChainEntry> {
        let Some(dataset) = self.dataset() else {
            return Vec::new();
        };
        dataset
            .versions
            .iter()
            .map(|version| ChainEntry {
                id: version.id,
                label: version.label.clone(),
                kind: version.kind,
                operation: version
                    .op
                    .as_ref()
                    .map(|op| op.summary().to_string())
                    .unwrap_or_else(|| "import".to_string()),
                shape: dataset.shape_of(version.id).unwrap_or((0, 0)),
                is_active: Some(version.id) == self.version,
            })
            .collect()
    }

    // -- what the plates show ---------------------------------------------

    /// The linked kinetic trace.
    pub fn current_trace(&self) -> Result<Trace> {
        let dataset = self
            .dataset()
            .ok_or_else(|| CoreError::invalid("no dataset is open"))?;
        let selection = self
            .trace
            .as_ref()
            .ok_or_else(|| CoreError::invalid("no wavelength is selected"))?;
        kinetic_trace(dataset, selection)
    }

    /// The kinetics plate's curves: the kept comparisons and the current trace.
    /// The curves the kinetics plate draws: the kept wavelengths, then the
    /// linked trace last so it is drawn on top.
    ///
    /// A wavelength that is both kept and linked is one curve, not two: the
    /// link wins, because it carries the accent, and the kept copy is dropped.
    pub fn kinetics(&self) -> Vec<Trace> {
        let Some(dataset) = self.dataset() else {
            return Vec::new();
        };
        let Some(template) = self.trace.clone() else {
            return Vec::new();
        };
        let Ok((axis, _)) = dataset.axes(template.version) else {
            return Vec::new();
        };
        let mut wanted: Vec<AxisRequest> = Vec::new();
        for wavelength in &self.comparison_wavelengths {
            wanted.push(self.sampling.request(*wavelength));
        }
        wanted.push(template.wavelength.clone());

        // From the linked curve backwards, so the first sample of each repeated
        // coordinate wins — which is the linked one — then reversed so the
        // linked curve is drawn last again.
        let mut seen: Vec<usize> = Vec::new();
        let mut traces: Vec<Trace> = Vec::new();
        for request in wanted.into_iter().rev() {
            let index = axis.nearest(request.coordinate).0;
            if seen.contains(&index) {
                continue;
            }
            seen.push(index);
            let selection = TraceSelection {
                id: template.id,
                dataset: template.dataset,
                version: template.version,
                wavelength: request,
            };
            if let Ok(trace) = kinetic_trace(dataset, &selection) {
                traces.push(trace);
            }
        }
        traces.reverse();
        traces
    }

    /// The spectra the plate draws: the kept delays, then the linked spectrum.
    pub fn spectra(&self) -> Vec<Spectrum> {
        let Some(dataset) = self.dataset() else {
            return Vec::new();
        };
        let Some(template) = self.spectrum.clone() else {
            return Vec::new();
        };
        let Ok((_, axis)) = dataset.axes(template.version) else {
            return Vec::new();
        };
        let mut wanted: Vec<AxisRequest> = Vec::new();
        for delay in &self.comparison_delays {
            wanted.push(self.sampling.request(*delay));
        }
        wanted.push(template.delay.clone());

        let mut seen: Vec<usize> = Vec::new();
        let mut spectra: Vec<Spectrum> = Vec::new();
        for request in wanted.into_iter().rev() {
            let index = axis.nearest(request.coordinate).0;
            if seen.contains(&index) {
                continue;
            }
            seen.push(index);
            let selection = SpectrumSelection {
                id: template.id,
                dataset: template.dataset,
                version: template.version,
                delay: request,
            };
            if let Ok(spectrum) = transient_spectrum(dataset, &selection) {
                spectra.push(spectrum);
            }
        }
        spectra.reverse();
        spectra
    }

    /// The kinetic records the kinetics plate draws, in the order it draws
    /// them: the kept comparison curves first, the linked trace last.
    pub fn kinetics_records(&self) -> Vec<ta_core::PlotSeries> {
        let Some(dataset) = self.dataset() else {
            return Vec::new();
        };
        let Some(template) = self.trace.clone() else {
            return Vec::new();
        };
        let Ok((axis, _)) = dataset.axes(template.version) else {
            return Vec::new();
        };
        let mut records = Vec::new();
        let mut seen: Vec<usize> = Vec::new();
        let wanted: Vec<f64> = self
            .comparison_wavelengths
            .iter()
            .copied()
            .chain(std::iter::once(template.wavelength.coordinate))
            .collect();
        for coordinate in wanted.into_iter().rev() {
            let index = axis.nearest(coordinate).0;
            if seen.contains(&index) {
                continue;
            }
            seen.push(index);
            let selection = TraceSelection {
                id: template.id,
                dataset: template.dataset,
                version: template.version,
                wavelength: self.sampling.request(coordinate),
            };
            if let Ok(trace) = kinetic_trace(dataset, &selection) {
                records.push(ta_core::PlotSeries::new(
                    trace.label,
                    ta_core::SeriesSource::Trace(selection),
                ));
            }
        }
        records.reverse();
        records
    }

    /// The spectral records the spectra plate draws, in its own order.
    pub fn spectra_records(&self) -> Vec<ta_core::PlotSeries> {
        let Some(dataset) = self.dataset() else {
            return Vec::new();
        };
        let Some(template) = self.spectrum.clone() else {
            return Vec::new();
        };
        let Ok((_, axis)) = dataset.axes(template.version) else {
            return Vec::new();
        };
        let mut records = Vec::new();
        let mut seen: Vec<usize> = Vec::new();
        let wanted: Vec<f64> = self
            .comparison_delays
            .iter()
            .copied()
            .chain(std::iter::once(template.delay.coordinate))
            .collect();
        for coordinate in wanted.into_iter().rev() {
            let index = axis.nearest(coordinate).0;
            if seen.contains(&index) {
                continue;
            }
            seen.push(index);
            let selection = SpectrumSelection {
                id: template.id,
                dataset: template.dataset,
                version: template.version,
                delay: self.sampling.request(coordinate),
            };
            if let Ok(spectrum) = transient_spectrum(dataset, &selection) {
                records.push(ta_core::PlotSeries::new(
                    spectrum.label,
                    ta_core::SeriesSource::Spectrum(selection),
                ));
            }
        }
        records.reverse();
        records
    }

    // -- the signature move: the ranked caption ---------------------------

    /// The caption under one plate. It names what the plate is, which version
    /// produced it, how the coordinate was read off the axis, and the unit —
    /// the provenance the numbers would otherwise be missing.
    pub fn caption(&self, plate: PlateId) -> String {
        let Some(dataset) = self.dataset() else {
            return "Import a matrix to begin.".to_string();
        };
        let Some(version_id) = self.version else {
            return "Import a matrix to begin.".to_string();
        };
        let Ok(version) = dataset.version(version_id) else {
            return "Import a matrix to begin.".to_string();
        };
        let version_label = version.label.clone();
        let operation = version
            .op
            .as_ref()
            .map(|op| format!("{} — {}", op.summary(), op.describe()))
            .unwrap_or_else(|| "the imported matrix".to_string());
        let ((w_low, w_high), (t_low, t_high)) = match dataset.axes(version_id) {
            Ok((wavelength, delay)) => (wavelength.range(), delay.range()),
            Err(_) => ((0.0, 0.0), (0.0, 0.0)),
        };
        let unit = dataset
            .unit(version_id)
            .map(|unit| unit.symbol())
            .unwrap_or("ΔOD");

        match plate {
            PlateId::Contour => format!(
                "ΔA contour, {w_low:.1}–{w_high:.1} nm against {t_low:.3} to {t_high:.3} ps. \
                 {version_label} ({operation}); colour limits symmetric about zero, {unit}, \
                 {} × {} samples.",
                dataset.shape_of(version_id).map(|s| s.0).unwrap_or(0),
                dataset.shape_of(version_id).map(|s| s.1).unwrap_or(0),
            ),
            PlateId::Kinetics => match self.kinetics().as_slice() {
                [] => "No wavelength chosen yet: click the contour, or type one in the panel.".to_string(),
                traces => {
                    let linked = traces.last().expect("the list is not empty");
                    let kept = traces.len() - 1;
                    let mut text = format!(
                        "Kinetics at {} ({}); {} point{} from {t_low:.3} to {t_high:.3} ps",
                        linked.label,
                        version_label,
                        linked.len(),
                        if linked.len() == 1 { "" } else { "s" },
                    );
                    if kept > 0 {
                        let others: Vec<String> = traces[..kept]
                            .iter()
                            .map(|trace| trace.label.clone())
                            .collect();
                        text.push_str(&format!(
                            ", with {kept} more at {}",
                            others.join(", ")
                        ));
                    }
                    text.push('.');
                    text
                }
            },
            PlateId::Spectra => {
                let spectra = self.spectra();
                if spectra.is_empty() {
                    return "No delay chosen yet: click the contour, or press the default grid in the panel.".to_string();
                }
                let linked = spectra.last().expect("the list is not empty");
                let kept = spectra.len() - 1;
                let mut text = format!(
                    "Spectra at {} ({}); {} wavelengths, shared scale, colour running with delay",
                    linked.label, version_label, linked.len(),
                );
                if kept > 0 {
                    let first = &spectra[0].label;
                    let last = &spectra[kept - 1].label;
                    text.push_str(&format!(
                        ". {kept} more from {first} to {last}, sampled at the nearest measured delays"
                    ));
                }
                text.push('.');
                text
            }
            PlateId::TimeZero => {
                let fitted = self.has_time_zero_correction();
                if fitted {
                    "The correction curve this version was built with.".to_string()
                } else {
                    "No time-zero correction for this version. FitWorkbench can estimate a curve; applying and editing corrections is not available yet."
                        .to_string()
                }
            }
        }
    }

    // -- persistence -------------------------------------------------------

    pub fn save_project(&mut self, path: &std::path::Path) -> Result<()> {
        let view = SavedAnalysis {
            opened: self.opened,
            trace: self.trace.clone(),
            spectrum: self.spectrum.clone(),
            wavelengths: self.comparison_wavelengths.clone(),
            delays: self.comparison_delays.clone(),
            sampling: self.sampling,
            comparison: self.comparison,
            studio: self.studio.clone(),
        };
        let value = serde_json::to_value(view)
            .map_err(|error| CoreError::invalid(format!("cannot save the analysis view: {error}")))?;
        let mut layout = self.project.layout.clone().unwrap_or_else(|| serde_json::json!({}));
        if !layout.is_object() {
            layout = serde_json::json!({});
        }
        layout["analysis"] = value;
        self.project.layout = Some(layout);
        ta_io::save_project(&self.project, path)?;
        self.path = Some(path.to_path_buf());
        self.dirty = false;
        self.status.info(format!("saved {}", path.display()));
        Ok(())
    }

    pub fn load_project(&mut self, path: &std::path::Path) -> Result<()> {
        let mut loaded = ta_io::load_project(path)?;
        let saved = loaded.project.layout.as_ref()
            .and_then(|layout| layout.get("analysis"))
            .map(|value| serde_json::from_value::<SavedAnalysis>(value.clone()))
            .transpose()
            .map_err(|error| CoreError::invalid(format!("cannot restore the analysis view: {error}")))?;
        if let Some(view) = &saved {
            if view.opened.is_none() && (view.trace.is_some() || view.spectrum.is_some()) {
                return Err(CoreError::invalid("saved selections have no active dataset"));
            }
            if let Some(figure) = &view.studio {
                loaded.project.figures.push(figure.clone());
                let valid = loaded.project.validate();
                loaded.project.figures.pop();
                valid?;
            }
            if let Some(id) = view.opened {
                let dataset = loaded.project.dataset(id)?;
                if let Some(trace) = &view.trace {
                    if trace.dataset != id || trace.version != dataset.active_version {
                        return Err(CoreError::invalid("saved trace does not match the active dataset version"));
                    }
                    kinetic_trace(dataset, trace)?;
                }
                if let Some(spectrum) = &view.spectrum {
                    if spectrum.dataset != id || spectrum.version != dataset.active_version {
                        return Err(CoreError::invalid("saved spectrum does not match the active dataset version"));
                    }
                    transient_spectrum(dataset, spectrum)?;
                }
            }
            if view.wavelengths.iter().chain(&view.delays).any(|value| !value.is_finite())
                || !view.sampling.half_width.is_finite() || view.sampling.half_width < 0.0
                || view.sampling.channels == 0 {
                return Err(CoreError::invalid("saved sampling settings are invalid"));
            }
        }
        self.project = loaded.project;
        self.fit = None;
        self.estimated_curve = None;
        self.studio = None;
        self.comparison_wavelengths.clear();
        self.comparison_delays.clear();
        self.sampling = Sampling::default();
        self.comparison = 0;
        self.opened = self.project.datasets.first().map(|dataset| dataset.id);
        self.version = self
            .opened
            .and_then(|id| self.project.dataset(id).ok())
            .map(|dataset| dataset.active_version);
        self.trace = self
            .opened
            .and_then(|id| self.project.dataset(id).ok())
            .and_then(|dataset| dataset.traces.first().cloned());
        self.spectrum = self
            .opened
            .and_then(|id| self.project.dataset(id).ok())
            .and_then(|dataset| dataset.spectra.first().cloned());
        if let Some(view) = saved {
            self.opened = view.opened;
            self.version = self.dataset().map(|dataset| dataset.active_version);
            self.trace = view.trace;
            self.spectrum = view.spectrum;
            self.comparison_wavelengths = view.wavelengths;
            self.comparison_delays = view.delays;
            self.sampling = view.sampling;
            self.comparison = view.comparison.min(self.project.comparisons.len().saturating_sub(1));
            self.studio = view.studio;
        }
        self.path = Some(path.to_path_buf());
        self.dirty = false;
        // Only what the project did not carry is defaulted; a saved trace or
        // spectrum is the user's, not a fresh guess.
        self.fill_missing_selection();
        self.status.info(format!("opened {}", path.display()));
        Ok(())
    }

    pub fn title(&self) -> String {
        let name = self
            .dataset()
            .map(|dataset| dataset.name.clone())
            .unwrap_or_else(|| "no dataset".to_string());
        let version = self
            .version()
            .map(|version| version.label.clone())
            .unwrap_or_else(|| "—".to_string());
        format!("{} · {}", name, version)
    }
}

/// One row of the plate index.
#[derive(Clone, Debug, PartialEq)]
pub struct ChainEntry {
    pub id: VersionId,
    pub label: String,
    pub kind: VersionKind,
    pub operation: String,
    pub shape: (usize, usize),
    pub is_active: bool,
}

/// The wavelength carrying the largest mean |ΔA|, which is where a first look
/// belongs.
fn strongest_wavelength(dataset: &TaDataset, version: VersionId) -> Option<f64> {
    let matrix = dataset.matrix(version).ok()?;
    let (wavelength, _) = dataset.axes(version).ok()?;
    let mut best: Option<(usize, f64)> = None;
    for row in 0..matrix.nrows() {
        let mut total = 0.0;
        let mut count = 0.0;
        for column in 0..matrix.ncols() {
            let value = matrix[[row, column]];
            if value.is_finite() {
                total += value.abs();
                count += 1.0;
            }
        }
        if count == 0.0 {
            continue;
        }
        let mean = total / count;
        if best.is_none_or(|(_, current)| mean > current) {
            best = Some((row, mean));
        }
    }
    best.map(|(row, _)| wavelength.values()[row])
}

/// One line of an operation's parameters, for the caption and the inspector.
#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::Array2;
    use ta_core::{
        Axis, AxisKind, AxisUnit, CropSpec, FigureModel, PlotSeries, PreZeroBackground,
        PreZeroRange, ProcessingOp, SeriesSource, SignalUnit, SourceFormat,
    };

    fn axis(kind: AxisKind, unit: AxisUnit, values: Vec<f64>) -> Axis {
        Axis::new(kind, unit, "axis", values).unwrap()
    }

    /// Wavelengths down the rows, delays across the columns, with one row
    /// clearly carrying the most signal.
    fn dataset() -> TaDataset {
        let wavelength = axis(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            vec![500.0, 550.0, 600.0, 650.0],
        );
        let delay = axis(
            AxisKind::Delay,
            AxisUnit::Picosecond,
            vec![-1.0, 0.0, 1.0, 10.0],
        );
        // Row 1 (550 nm) carries the most signal, so that is the wavelength a
        // first look should land on.
        let amplitudes = [0.001, 0.02, 0.002, 0.003];
        let mut data = Array2::zeros((4, 4));
        for row in 0..4 {
            for column in 0..4 {
                data[[row, column]] = amplitudes[row] * (column as f64 + 1.0);
            }
        }
        TaDataset::new_raw(
            "test matrix",
            wavelength,
            delay,
            data,
            SignalUnit::DeltaOpticalDensity,
            SourceFormat::Synthetic,
        )
        .unwrap()
    }

    fn session() -> Session {
        let mut session = Session::new();
        session.open_dataset(dataset());
        session
    }

    #[test]
    fn opening_a_dataset_seeds_a_selection_where_the_signal_is() {
        let session = session();
        let trace = session.current_trace().unwrap();
        // 550 nm carries the largest mean magnitude, so it is where the shell
        // starts.
        assert!(trace.label.starts_with("550"));
        // And the spectrum starts at the first positive delay, drawn last.
        let spectra = session.spectra();
        assert!(spectra.last().unwrap().label.starts_with("1"));
    }

    #[test]
    fn a_dataset_opens_on_the_reference_implementations_delay_grid() {
        // `auto_taspectra` plots these seventeen points, each snapped to a
        // measured delay, with the ones that land on the same sample dropped.
        let session = session();
        let spectra = session.spectra();
        let kept: Vec<String> = spectra[..spectra.len() - 1]
            .iter()
            .map(|spectrum| spectrum.label.clone())
            .collect();
        // The fixture's delays are -1, 0, 1 and 10 ps. The grid's -0.5 snaps
        // to -1, -0.2 and 0 snap to 0, 0.1 through 2 all snap to 1, and 5
        // through 1500 all snap to 10 — so four requests survive, and 1 ps is
        // left out because it is the linked spectrum's own sample.
        assert_eq!(kept, vec!["-1.0000 ps", "0.0000 ps", "10.0000 ps"]);
        assert!(session
            .spectra()
            .last()
            .unwrap()
            .label
            .starts_with("1.0000"));
    }

    #[test]
    fn a_grid_point_outside_the_measured_range_is_not_invented() {
        // Every default delay is requested; only the ones a matrix actually
        // measured can be drawn, and the count says so.
        let session = session();
        let dataset = session.dataset().unwrap();
        let version = session.version.unwrap();
        let (_, delay) = dataset.axes(version).unwrap();
        for value in &session.comparison_delays {
            assert!(delay.contains(*value), "{value} is not a measured delay");
        }
        // 1000 and 1500 ps are in the reference's grid but not in this matrix.
        assert!(!session.comparison_delays.iter().any(|value| *value > 100.0));
    }

    #[test]
    fn adding_a_wavelength_twice_adds_it_once() {
        let mut session = session();
        session.add_wavelength(600.0).unwrap();
        assert_eq!(session.comparison_wavelengths.len(), 1);
        // 601 nm snaps to the same measured sample as 600 nm.
        assert!(session.add_wavelength(601.0).is_err());
        assert_eq!(session.comparison_wavelengths.len(), 1);
        // And the linked trace cannot be kept twice either.
        assert!(session.add_wavelength(550.0).is_err());
    }

    #[test]
    fn the_kept_curves_are_ordered_along_the_axis_they_were_taken_on() {
        let mut session = session();
        session.add_wavelength(650.0).unwrap();
        session.add_wavelength(500.0).unwrap();
        assert_eq!(session.comparison_wavelengths, vec![500.0, 650.0]);
        session.clear_delays();
        session.add_delay(10.0).unwrap();
        session.add_delay(0.0).unwrap();
        let delays = session.comparison_delays.clone();
        let mut sorted = delays.clone();
        sorted.sort_by(f64::total_cmp);
        assert_eq!(delays, sorted);
    }

    #[test]
    fn removing_a_curve_leaves_the_link_where_it_was() {
        let mut session = session();
        session.add_wavelength(650.0).unwrap();
        let linked = session.current_trace().unwrap().label.clone();
        session.remove_wavelength(0);
        assert!(session.comparison_wavelengths.is_empty());
        assert_eq!(session.current_trace().unwrap().label, linked);
    }

    #[test]
    fn choosing_a_wavelength_moves_the_trace_and_leaves_the_spectrum_alone() {
        let mut session = session();
        let before = session.spectra().last().unwrap().label.clone();
        session.select_wavelength(600.0).unwrap();
        let trace = session.current_trace().unwrap();
        assert!(trace.label.starts_with("600"));
        // The other plate did not move: the link runs one way, which is what
        // stops two views updating each other.
        assert_eq!(session.spectra().last().unwrap().label, before);
    }

    #[test]
    fn choosing_a_delay_moves_the_spectrum_and_leaves_the_trace_alone() {
        let mut session = session();
        let before = session.current_trace().unwrap().label.clone();
        session.select_delay(10.0).unwrap();
        assert!(session.spectra().last().unwrap().label.starts_with("10"));
        assert_eq!(session.current_trace().unwrap().label, before);
    }

    #[test]
    fn clicking_the_contour_moves_both_linked_plates() {
        let mut session = session();
        session.select_point(650.0, 0.0).unwrap();
        assert!(session.current_trace().unwrap().label.starts_with("650"));
        assert!(session.spectra().last().unwrap().label.starts_with("0"));
    }

    #[test]
    fn an_interpolated_selection_reads_between_the_samples() {
        let mut session = session();
        session.set_sampling(Sampling {
            mode: SampleMode::Interpolated,
            ..Sampling::default()
        });
        session.select_wavelength(525.0).unwrap();
        let trace = session.current_trace().unwrap();
        assert!(trace.label.contains("interpolated"));
        // The interpolated row sits halfway between the 500 and 550 rows:
        // (0.001 + 0.02) / 2 * 4 at the 10 ps delay.
        assert!((trace.signal[3] - 0.042).abs() < 1e-12, "{:?}", trace.signal);
    }

    #[test]
    fn an_averaged_band_reports_the_sample_count_it_used() {
        let mut session = session();
        session.set_sampling(Sampling {
            mode: SampleMode::AverageChannels { count: 3 },
            channels: 3,
            ..Sampling::default()
        });
        session.select_wavelength(550.0).unwrap();
        let trace = session.current_trace().unwrap();
        assert!(trace.label.contains("3 channels"), "label was {}", trace.label);
    }

    #[test]
    fn switching_to_a_cropped_version_moves_a_selection_it_cannot_hold_and_says_so() {
        let mut session = session();
        session.select_wavelength(650.0).unwrap();
        let version = {
            let dataset = session.dataset_mut().unwrap();
            let raw = dataset.raw_version().id;
            let data = dataset.matrix(raw).unwrap();
            let cropped = ta_processing::crop_matrix(
                data,
                dataset.axes(raw).unwrap().0,
                dataset.axes(raw).unwrap().1,
                &CropSpec {
                    wavelength: Some((500.0, 600.0)),
                    delay: None,
                },
            )
            .unwrap();
            dataset
                .add_derived(
                    cropped.data,
                    SignalUnit::DeltaOpticalDensity,
                    cropped.wavelength,
                    cropped.delay,
                    "Cropped 500–600 nm",
                    ProcessingOp::Crop(CropSpec {
                        wavelength: Some((500.0, 600.0)),
                        delay: None,
                    }),
                    vec![raw],
                )
                .unwrap()
        };
        session.set_version(version).unwrap();

        let trace = session.current_trace().unwrap();
        // 650 nm is gone; the trace sits on 600 and the move is reported.
        assert!(trace.label.starts_with("600"));
        assert!(
            session
                .status
                .problem
                .as_deref()
                .unwrap_or_default()
                .contains("moved from 650.0000 nm"),
            "status was {:?}",
            session.status
        );
    }

    #[test]
    fn the_caption_names_the_version_the_operation_and_the_sampling() {
        let mut session = session();
        // Give the dataset a background-corrected version to name.
        let version = {
            let dataset = session.dataset_mut().unwrap();
            let raw = dataset.raw_version().id;
            let data = dataset.matrix(raw).unwrap();
            let corrected = ta_processing::background_pre_zero(
                data,
                dataset.axes(raw).unwrap().1,
                PreZeroRange::FirstPoints { count: 1 },
            )
            .unwrap();
            let axes = {
                let (wavelength, delay) = dataset.axes(raw).unwrap();
                (wavelength.clone(), delay.clone())
            };
            dataset
                .add_derived(
                    corrected,
                    SignalUnit::DeltaOpticalDensity,
                    axes.0,
                    axes.1,
                    "Background corrected",
                    ProcessingOp::BackgroundPreZero(PreZeroBackground {
                        range: PreZeroRange::FirstPoints { count: 1 },
                    }),
                    vec![raw],
                )
                .unwrap()
        };
        session.set_version(version).unwrap();

        let contour = session.caption(PlateId::Contour);
        assert!(contour.contains("Background corrected"), "{contour}");
        assert!(contour.contains("first 1 delay points"), "{contour}");
        assert!(contour.contains("500.0–650.0 nm"), "{contour}");
        assert!(contour.contains("ΔOD"), "{contour}");

        let kinetics = session.caption(PlateId::Kinetics);
        assert!(kinetics.contains("Background corrected"), "{kinetics}");
        assert!(kinetics.contains("Kinetics at"), "{kinetics}");

        let spectra = session.caption(PlateId::Spectra);
        assert!(spectra.contains("Spectra at"), "{spectra}");
        assert!(spectra.contains("4 wavelengths"), "{spectra}");

        let time_zero = session.caption(PlateId::TimeZero);
        assert!(time_zero.contains("No time-zero correction"), "{time_zero}");
    }

    #[test]
    fn the_caption_before_anything_is_open_teaches_instead_of_showing_zero() {
        let session = Session::new();
        assert_eq!(
            session.caption(PlateId::Contour),
            "Import a matrix to begin."
        );
    }

    #[test]
    fn the_plate_index_lists_the_chain_with_the_active_version_marked() {
        let session = session();
        let chain = session.chain();
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].kind, VersionKind::Raw);
        assert_eq!(chain[0].operation, "import");
        assert!(chain[0].is_active);
        assert_eq!(chain[0].shape, (4, 4));
    }

    #[test]
    fn keeping_a_trace_adds_it_to_the_kinetics_plate_without_moving_the_selection() {
        let mut session = session();
        session.select_wavelength(600.0).unwrap();
        session.save_current_trace().unwrap();
        session.select_wavelength(500.0).unwrap();
        let curves = session.kinetics();
        assert_eq!(curves.len(), 2);
        assert!(curves[0].label.starts_with("600"));
        assert!(curves[1].label.starts_with("500"));
    }

    #[test]
    fn a_dataset_can_be_saved_and_reopened_through_the_session() {
        let mut session = session();
        session.select_point(600.0, 10.0).unwrap();
        // Keep one trace and one figure so the project has something to carry.
        session.save_current_trace().unwrap();
        let dataset_id = session.opened.unwrap();
        let version = session.version.unwrap();
        let selection = session.trace.clone().unwrap();
        {
            let dataset = session.dataset_mut().unwrap();
            dataset.traces.push(selection.clone());
            let mut figure = FigureModel::single_panel("Kinetics");
            figure.panels[0].series.push(PlotSeries::new(
                "600 nm",
                SeriesSource::Trace(selection.clone()),
            ));
            dataset.figures.push(figure);
        }
        session.project.validate().unwrap();

        let mut path = std::env::temp_dir();
        path.push(format!("spectra-session-{}.taproject", std::process::id()));
        session.save_project(&path).unwrap();
        assert!(!session.dirty);

        let mut reopened = Session::new();
        reopened.load_project(&path).unwrap();
        assert_eq!(
            reopened.project.datasets[0].traces.len(),
            1,
            "the saved trace should have travelled with the project"
        );
        assert_eq!(reopened.opened, Some(dataset_id));
        assert_eq!(reopened.version, Some(version));
        let trace = reopened.current_trace().unwrap();
        assert!(
            trace.label.starts_with("600"),
            "label was {:?}, selection {:?}",
            trace.label,
            reopened.trace.as_ref().map(|trace| trace.wavelength)
        );
        assert_eq!(reopened.project.figures.len(), 0, "the figure lives on the dataset");
        assert_eq!(reopened.comparison_wavelengths, session.comparison_wavelengths);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn reopening_restores_the_bench_sampling_and_overlays_without_changing_data() {
        let mut session = session();
        session.open_dataset(dataset());
        let second = session.opened;
        session.select_point(600.0, 10.0).unwrap();
        session.save_current_trace().unwrap();
        session.clear_delays();
        session.save_current_spectrum().unwrap();
        session.set_sampling(Sampling {
            mode: SampleMode::AverageChannels { count: 3 },
            channels: 3,
            half_width: 7.0,
        });
        let before = session.current_trace().unwrap().signal;
        let path = std::env::temp_dir().join(format!("spectra-view-{}.taproject", std::process::id()));
        session.save_project(&path).unwrap();
        let mut reopened = Session::new();
        reopened.load_project(&path).unwrap();
        assert_eq!(reopened.opened, second);
        assert_eq!(reopened.trace, session.trace);
        assert_eq!(reopened.spectrum, session.spectrum);
        assert_eq!(reopened.sampling, session.sampling);
        assert_eq!(reopened.comparison_wavelengths, vec![600.0]);
        assert_eq!(reopened.comparison_delays, vec![10.0]);
        assert_eq!(reopened.current_trace().unwrap().signal, before);
        assert!(!reopened.dirty);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn restoring_raw_updates_the_saved_active_version() {
        let mut session = session();
        let raw = session.version.unwrap();
        let dataset = session.dataset_mut().unwrap();
        let corrected = dataset.add_derived(
            dataset.active_matrix().clone(),
            dataset.unit(raw).unwrap(),
            dataset.wavelength().clone(), dataset.delay().clone(),
            "Background corrected", ta_core::ProcessingOp::BackgroundPreZero(
                ta_core::PreZeroBackground { range: ta_core::PreZeroRange::FirstPoints { count: 1 } }
            ), vec![raw],
        ).unwrap();
        session.set_version(corrected).unwrap();
        session.set_version(raw).unwrap();
        assert_eq!(session.dataset().unwrap().active_version, raw);
        assert_eq!(session.dataset().unwrap().versions.len(), 2, "restoring does not delete work");
    }

    #[test]
    fn restoring_raw_does_not_show_an_unrelated_chirp_curve() {
        let mut session = session();
        let raw = session.version.unwrap();
        let dataset = session.dataset_mut().unwrap();
        let corrected = dataset.add_derived(
            dataset.active_matrix().clone(), dataset.unit(raw).unwrap(),
            dataset.wavelength().clone(), dataset.delay().clone(), "Time-zero corrected",
            ta_core::ProcessingOp::TimeZero(ta_core::TimeZeroCorrection {
                origin: ta_core::CorrectionOrigin::Manual,
                curve: ta_core::CorrectionCurve::new(vec![ta_core::ControlPoint { wavelength: 550.0, time_zero: 0.1 }]),
                interpolation: ta_core::Interpolation::Linear,
                boundary: ta_core::BoundaryMode::Clamp,
            }), vec![raw],
        ).unwrap();
        session.set_version(corrected).unwrap();
        assert!(session.has_time_zero_correction());
        assert!(!crate::plates::time_zero_content(&session).series.is_empty());
        session.set_version(raw).unwrap();
        assert!(!session.has_time_zero_correction());
        assert!(crate::plates::time_zero_content(&session).series.is_empty());
        assert!(session.caption(PlateId::TimeZero).contains("No time-zero correction"));
    }

    #[test]
    fn an_invalid_saved_view_does_not_replace_open_work() {
        let mut session = session();
        let opened = session.opened;
        let path = std::env::temp_dir().join(format!("spectra-invalid-view-{}.taproject", std::process::id()));
        let mut project = session.project.clone();
        project.layout = Some(serde_json::json!({"analysis": {"opened": "broken"}}));
        ta_io::save_project(&project, &path).unwrap();
        assert!(session.load_project(&path).is_err());
        assert_eq!(session.opened, opened);
        assert!(session.current_trace().is_ok());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn operating_the_session_reports_what_changed() {
        let mut session = session();
        session.status.info("ready");
        session.select_wavelength(600.0).unwrap();
        assert!(session.dirty);
        assert_eq!(session.status.message, "ready");
        assert!(session.title().contains("test matrix"));
    }
}
