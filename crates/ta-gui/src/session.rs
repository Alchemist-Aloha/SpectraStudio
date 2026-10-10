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
#[derive(Clone, Copy, Debug, PartialEq)]
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
    pub sampling: Sampling,
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
            sampling: Sampling::default(),
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
        self.comparison_wavelengths.clear();
        self.comparison_delays.clear();
        self.reseed_selection();
        self.status.info(format!("{name}: {rows} wavelengths × {delays} delays"));
        self.dirty = true;
        id
    }

    /// Chooses the strongest wavelength and the first positive delay.
    fn reseed_selection(&mut self) {
        self.trace = None;
        self.spectrum = None;
        self.fill_missing_selection();
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
    pub fn kinetics(&self) -> Vec<Trace> {
        let Some(dataset) = self.dataset() else {
            return Vec::new();
        };
        let Some(template) = self.trace.clone() else {
            return Vec::new();
        };
        let mut wanted: Vec<AxisRequest> = Vec::new();
        for wavelength in &self.comparison_wavelengths {
            wanted.push(self.sampling.request(*wavelength));
        }
        wanted.push(template.wavelength);
        wanted
            .into_iter()
            .filter_map(|request| {
                let selection = TraceSelection {
                    id: template.id,
                    dataset: template.dataset,
                    version: template.version,
                    wavelength: request,
                };
                kinetic_trace(dataset, &selection).ok()
            })
            .collect()
    }

    /// The spectra plate's curves: the kept comparisons and the current delay.
    pub fn spectra(&self) -> Vec<Spectrum> {
        let Some(dataset) = self.dataset() else {
            return Vec::new();
        };
        let Some(template) = self.spectrum.clone() else {
            return Vec::new();
        };
        let mut wanted: Vec<AxisRequest> = Vec::new();
        for delay in &self.comparison_delays {
            wanted.push(self.sampling.request(*delay));
        }
        wanted.push(template.delay);
        wanted
            .into_iter()
            .filter_map(|request| {
                let selection = SpectrumSelection {
                    id: template.id,
                    dataset: template.dataset,
                    version: template.version,
                    delay: request,
                };
                transient_spectrum(dataset, &selection).ok()
            })
            .collect()
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
            .map(|op| format!("{} — {}", op.summary(), describe_op(op)))
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
            PlateId::Kinetics => match &self.trace {
                Some(selection) => match kinetic_trace(dataset, selection) {
                    Ok(trace) => format!(
                        "Kinetics at {}. {version_label}; {} points from {t_low:.3} to {t_high:.3} ps.",
                        trace.label,
                        trace.len(),
                    ),
                    Err(error) => format!("Kinetics unavailable: {error}"),
                },
                None => "No wavelength selected.".to_string(),
            },
            PlateId::Spectra => {
                let spectra = self.spectra();
                if spectra.is_empty() {
                    return "No delay selected.".to_string();
                }
                let labels: Vec<String> = spectra
                    .iter()
                    .map(|spectrum| spectrum.label.clone())
                    .collect();
                format!(
                    "Spectra at {}. {version_label}; {} wavelengths, shared scale.",
                    labels.join(", "),
                    spectra[0].len(),
                )
            }
            PlateId::TimeZero => "No time-zero correction has been set for this dataset.".to_string(),
        }
    }

    // -- persistence -------------------------------------------------------

    pub fn save_project(&mut self, path: &std::path::Path) -> Result<()> {
        ta_io::save_project(&self.project, path)?;
        self.path = Some(path.to_path_buf());
        self.dirty = false;
        self.status.info(format!("saved {}", path.display()));
        Ok(())
    }

    pub fn load_project(&mut self, path: &std::path::Path) -> Result<()> {
        let loaded = ta_io::load_project(path)?;
        self.project = loaded.project;
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
pub fn describe_op(op: &ta_core::ProcessingOp) -> String {
    use ta_core::ProcessingOp as Op;
    match op {
        Op::BackgroundPreZero(background) => match background.range {
            ta_core::PreZeroRange::FirstPoints { count } => format!("first {count} delay points"),
            ta_core::PreZeroRange::Interval { start, end } => {
                format!("{start:.3} to {end:.3} ps")
            }
        },
        Op::BackgroundReference(reference) => format!(
            "reference dataset, factor {:.3}{}",
            reference.factor,
            if reference.normalize { ", normalised" } else { "" }
        ),
        Op::TimeZero(correction) => format!(
            "{} control point(s), {:?}, {:?}",
            correction.curve.points.len(),
            correction.interpolation,
            correction.boundary
        ),
        Op::Crop(crop) => {
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
        Op::Average(average) => {
            if average.mean {
                "mean of the inputs".to_string()
            } else {
                "sum of the inputs".to_string()
            }
        }
        Op::BurnCorrect(burn) => format!(
            "at {:.1} nm, reference delay {:.3} ps",
            burn.wavelength, burn.reference_delay
        ),
        Op::Stitch(stitch) => format!(
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
        // And the spectrum starts at the first positive delay.
        let spectra = session.spectra();
        assert_eq!(spectra.len(), 1);
        assert!(spectra[0].label.starts_with("1"));
    }

    #[test]
    fn choosing_a_wavelength_moves_the_trace_and_leaves_the_spectrum_alone() {
        let mut session = session();
        let before = session.spectra()[0].label.clone();
        session.select_wavelength(600.0).unwrap();
        let trace = session.current_trace().unwrap();
        assert!(trace.label.starts_with("600"));
        // The other plate did not move: the link runs one way, which is what
        // stops two views updating each other.
        assert_eq!(session.spectra()[0].label, before);
    }

    #[test]
    fn choosing_a_delay_moves_the_spectrum_and_leaves_the_trace_alone() {
        let mut session = session();
        let before = session.current_trace().unwrap().label.clone();
        session.select_delay(10.0).unwrap();
        assert!(session.spectra()[0].label.starts_with("10"));
        assert_eq!(session.current_trace().unwrap().label, before);
    }

    #[test]
    fn clicking_the_contour_moves_both_linked_plates() {
        let mut session = session();
        session.select_point(650.0, 0.0).unwrap();
        assert!(session.current_trace().unwrap().label.starts_with("650"));
        assert!(session.spectra()[0].label.starts_with("0"));
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
        assert_eq!(reopened.comparison_wavelengths.len(), 0);
        std::fs::remove_file(&path).ok();
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
