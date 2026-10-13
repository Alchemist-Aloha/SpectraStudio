//! The application shell.
//!
//! The composition is the direction contract's: a header of rules, a plate index
//! of ruled rows on the left, the figure block of captioned plates in the
//! centre, a column of caption-and-control pairs on the right, and a footer of
//! tabular coordinates with the clickable address of what is selected.

use crate::plates::{self, PlateContent};
use crate::widgets;
use crate::session::{PlateId, Sampling, Session};
use gpui_kit::component::{h_resizable, resizable_panel, v_resizable};
// Imported by name rather than by glob: `gpui_kit::*` also brings in gpui's
// `test` attribute macro, which then shadows the built-in one in this crate's
// tests.
use gpui_kit::{
    div, px, AnyElement, App, Context, ElementId, FontWeight, InteractiveElement, IntoElement,
    ParentElement, PathPromptOptions, Render, StatefulInteractiveElement, Styled, Window,
};
use std::path::PathBuf;
use std::rc::Rc;
use ta_core::{Rgba, SampleMode};
use ta_plot_gpui::{
    HeatmapCache, HeatmapSource, PanelScene, Plate, PlotGesture, PlotState, SharedHeatmapCache,
};

const INDEX_WIDTH: f32 = 208.0;
const INSPECTOR_WIDTH: f32 = 316.0;
const HEADER_HEIGHT: f32 = 52.0;
const FOOTER_HEIGHT: f32 = 30.0;
const CAPTION_HEIGHT: f32 = 46.0;

/// The direction's palette.
pub mod ink {
    use gpui_kit::Hsla;
    use ta_core::Rgba;

    /// Paper white: the plate ground.
    pub fn paper() -> Hsla {
        rgb(255, 255, 255)
    }
    /// Chart paper: the shell ground behind the plates.
    pub fn ground() -> Hsla {
        rgb(238, 241, 246)
    }
    /// The ink of text and rules.
    pub fn text() -> Hsla {
        rgb(16, 21, 38)
    }
    /// Secondary text, tinted from the ground rather than greyed.
    pub fn muted() -> Hsla {
        rgb(91, 100, 120)
    }
    /// The one accent: structure and the current selection.
    pub fn accent() -> Hsla {
        rgb(176, 42, 55)
    }
    /// A value pinned at a bound.
    pub fn amber() -> Hsla {
        rgb(184, 121, 31)
    }
    /// Hairline rules.
    pub fn rule() -> Hsla {
        rgb(201, 206, 217)
    }
    pub fn hair() -> Hsla {
        rgb(226, 230, 238)
    }

    /// A data colour as a UI colour, for the swatches that name a series.
    pub fn hsla(color: Rgba) -> Hsla {
        let rgba = gpui_kit::Rgba {
            r: color.r as f32 / 255.0,
            g: color.g as f32 / 255.0,
            b: color.b as f32 / 255.0,
            a: color.a as f32 / 255.0,
        };
        let mut value: Hsla = rgba.into();
        value.a = 1.0;
        value
    }

    fn rgb(r: u8, g: u8, b: u8) -> Hsla {
        let rgba = gpui_kit::Rgba {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: 1.0,
        };
        let mut value: Hsla = rgba.into();
        value.a = 1.0;
        value
    }
}

// Each step is its own action, so the key map stays a plain table of keys and
// actions and the handler has nothing to decode.
gpui_kit::actions!(
    spectra_studio,
    [
        WavelengthBack,
        WavelengthForward,
        DelayBack,
        DelayForward,
        KeepWavelength,
        KeepDelay,
        FitTrace,
        ResetViews,
        SaveProject
    ]
);

/// Every key the workspace answers to, so the handler is one match rather than
/// eight closures.
pub enum ShortcutAction {
    WavelengthBack,
    WavelengthForward,
    DelayBack,
    DelayForward,
    KeepWavelength,
    KeepDelay,
    FitTrace,
    ResetViews,
}

/// The export formats, re-exported so the shell and the studio agree on one
/// list.
pub use ta_core::ExportFormat;

/// The layouts of the window: one dataset at a time, several at once, the
/// publication editor, and the fitting workbench.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Figure,
    Compare,
    Studio,
    Fit,
}

impl Layout {
    pub const ALL: [Layout; 4] = [Layout::Figure, Layout::Compare, Layout::Studio, Layout::Fit];

    pub const fn label(self) -> &'static str {
        match self {
            Layout::Figure => "Figure",
            Layout::Compare => "Compare",
            Layout::Studio => "FigureStudio",
            Layout::Fit => "FitWorkbench",
        }
    }

    /// What the layout is for, in the one line the header shows.
    pub const fn note(self) -> &'static str {
        match self {
            Layout::Figure => "one dataset, four linked plates",
            Layout::Compare => "many objects, native coordinates",
            Layout::Studio => "the same figure model, at publication size",
            Layout::Fit => "kinetics and their model, off the UI thread",
        }
    }
}

/// The workspace view.
pub struct Workspace {
    pub session: Session,
    /// One interactive plate per panel of the figure block.
    pub plates: Vec<Plate>,
    /// The colour-mapped contour image, shared with the plates because each
    /// builds its own image during prepaint, at its own plot size.
    pub raster: SharedHeatmapCache,
    /// What the contour's image is made from, rebuilt when the matrix changes.
    pub contour_source: Option<std::rc::Rc<HeatmapSource>>,
    /// Bumped whenever the project changed, so the scenes are rebuilt once.
    revision: u64,
    built: u64,
    caption_of: [String; 4],
    /// The contour's isolines, which cost a marching-squares pass over the
    /// whole matrix and only change when the matrix or the levels do.
    contour_panel: Option<ta_core::PanelModel>,
    contour_series: Option<Vec<ta_plot_gpui::PlottedSeries>>,
    contour_key: Option<(ta_core::DatasetId, ta_core::VersionId)>,
    /// Set when a different matrix arrived: the plates then take their window
    /// from the new axes instead of keeping the one they had, which is what a
    /// selection change must do and a dataset change must not.
    rehome: bool,
    /// The layout the window is showing.
    pub layout: Layout,
    /// The two comparison panes.
    pub compare_plates: Vec<Plate>,
    /// What "add the selection" adds to a comparison.
    pub compare_target: crate::compare::SeriesTarget,
    /// The workbench's data plate and its residual plate.
    pub fit_plates: Vec<Plate>,
    /// Every editable number in the interface, keyed by what it edits.
    pub fields: crate::widgets::Fields,
    /// The caption under each comparison pane and the workbench.
    pub compare_caption: (String, String),
    pub fit_caption: String,
    /// Which plate the studio was opened on, which decides what it exports.
    pub current_plate: PlateId,
    /// The studio's preview plate: the figure drawn at its own physical size.
    pub studio_plates: Vec<Plate>,
    /// Whether a data export writes the visible window or everything measured.
    pub studio_visible_only: bool,
    pub studio_selection: std::collections::HashSet<ta_core::SeriesId>,
    pub inspector_sections: std::collections::HashMap<String, bool>,
    /// Whether the workbench shows the columns a fit is set up with, or only
    /// the values it produced.
    pub fit_edit_parameters: bool,
    /// A page zoom to apply to the studio preview once its plate exists, which
    /// is set by the command line for a scripted check of the preview.
    pub pending_page_zoom: Option<f32>,
    /// What the title bar currently says, so it is only set when it changes.
    window_title: String,
    /// Whether this dataset has a time-zero correction to show. The fourth
    /// plate only earns its space when it has something to say.
    show_time_zero: bool,
    /// Set when the user asks for the fourth plate without a correction to put
    /// in it, so the empty axis can still be studied.
    show_time_zero_plate: bool,
    pub show_index: bool,
    pub show_inspector: bool,
    focus: Option<gpui_kit::FocusHandle>,
}

impl Workspace {
    pub fn new(session: Session) -> Self {
        let mut workspace = Self {
            session,
            plates: Vec::new(),
            raster: std::rc::Rc::new(std::cell::RefCell::new(HeatmapCache::new())),
            contour_source: None,
            revision: 1,
            built: 0,
            caption_of: Default::default(),
            contour_panel: None,
            contour_series: None,
            contour_key: None,
            rehome: true,
            layout: Layout::Figure,
            show_index: true,
            show_inspector: true,
            focus: None,
            compare_plates: Vec::new(),
            compare_target: crate::compare::SeriesTarget::Trace,
            fit_plates: Vec::new(),
            fields: crate::widgets::Fields::default(),
            compare_caption: (String::new(), String::new()),
            fit_caption: String::new(),
            current_plate: PlateId::Kinetics,
            studio_plates: Vec::new(),
            studio_visible_only: true,
            studio_selection: Default::default(),
            inspector_sections: Default::default(),
            fit_edit_parameters: true,
            pending_page_zoom: None,
            window_title: String::new(),
            show_time_zero: false,
            show_time_zero_plate: false,
        };
        workspace.build();
        workspace
    }

    pub(crate) fn touch(&mut self) {
        self.revision += 1;
    }

    /// A different matrix is on the bench: the plates refit to its axes.
    pub(crate) fn touch_matrix(&mut self) {
        self.contour_key = None;
        self.rehome = true;
        self.touch();
    }

    /// Builds every plate's scene from the session.
    fn build(&mut self) {
        self.plates.clear();
        self.refresh_scenes();
    }

    /// The key the contour's isolines depend on.
    fn contour_key(&self) -> Option<(ta_core::DatasetId, ta_core::VersionId)> {
        Some((self.session.opened?, self.session.version?))
    }

    /// Recomputes the contour's isolines when the matrix or the levels changed.
    fn ensure_contour(&mut self, spec: &ta_core::ContourSpec) {
        let key = self.contour_key();
        if self.contour_key == key && self.contour_panel.is_some() {
            return;
        }
        let content = plates::contour_content(&self.session, spec);
        self.contour_panel = Some(content.panel);
        self.contour_series = Some(content.series);
        self.contour_key = key;
    }

    /// Keeps the interaction state when only the drawing changed.
    fn refresh_scenes(&mut self) {
        let spec = plates::contour_spec(&self.session);
        let views = plates::style();
        self.ensure_contour(&spec);
        self.ensure_contour_source(&spec);
        let rehome = self.rehome;
        let raster = self.raster.clone();

        // The four plates of the single-dataset layout.
        for (index, plate_id) in PlateId::ALL.iter().enumerate() {
            let content = if *plate_id == PlateId::Contour {
                PlateContent {
                    panel: self
                        .contour_panel
                        .clone()
                        .unwrap_or_else(|| ta_core::PanelModel::new("")),
                    series: self.contour_series.clone().unwrap_or_default(),
                    records: Vec::new(),
                }
            } else {
                plates::content(&self.session, *plate_id, &spec)
            };
            let heatmap = if *plate_id == PlateId::Contour {
                self.contour_source.clone()
            } else {
                None
            };
            let scene = scene_of(
                content,
                views.clone(),
                heatmap,
                plates::cursors(&self.session, *plate_id),
                true,
            );
            sync_plate(&mut self.plates, index, scene, raster.clone(), rehome);
            self.caption_of[index] = self.session.caption(*plate_id);
        }

        // The comparison layout: spectra, then kinetics.
        for (index, spectra) in [true, false].into_iter().enumerate() {
            let content = crate::compare::pane_content(&self.session, spectra, views.clone());
            let scene = scene_of(content, views.clone(), None, Vec::new(), true);
            sync_plate(&mut self.compare_plates, index, scene, raster.clone(), rehome);
        }

        // The workbench: the data with its fit, then the residuals.
        if let Some((data, residuals)) = crate::fit_panel::fit_content(&self.session) {
            let scene = scene_of(data, views.clone(), None, Vec::new(), true);
            sync_plate(&mut self.fit_plates, 0, scene, raster.clone(), rehome);
            match residuals {
                Some(residuals) => {
                    let scene = scene_of(residuals, views.clone(), None, Vec::new(), true);
                    sync_plate(&mut self.fit_plates, 1, scene, raster.clone(), rehome);
                }
                None => self.fit_plates.truncate(1),
            }
        } else {
            self.fit_plates.clear();
        }
        self.fit_caption = crate::fit_panel::fit_caption(&self.session);
        self.show_time_zero = self.session.has_time_zero_correction();
        // The studio's preview is the figure at its own physical size, with the
        // figure's own type: changing the type size or the paper size has to be
        // visible here, which is the whole point of the pane.
        match (self.studio_preview(), self.session.studio.clone()) {
            (Some(content), Some(figure)) => {
                let size = ta_plot::export::figure_points(&figure);
                // A contour preview needs the matrix as well as its isolines:
                // without it the field is a blank page with outlines on it.
                let heatmap = self.studio_contour_source();
                let scene = scene_at_size(
                    content,
                    figure.style.clone(),
                    heatmap,
                    Vec::new(),
                    false,
                    Some(size),
                );
                let page_view = self.studio_plates.first().map(|plate| {
                    let state = plate.state.borrow();
                    (state.image_zoom, state.image_offset)
                });
                sync_plate(&mut self.studio_plates, 0, scene, raster.clone(), true);
                if let (Some((zoom, offset)), Some(plate)) = (page_view, self.studio_plates.first()) {
                    let mut state = plate.state.borrow_mut();
                    state.image_zoom = zoom;
                    state.image_offset = offset;
                }
                if let Some(zoom) = self.pending_page_zoom.take() {
                    if let Some(plate) = self.studio_plates.first() {
                        plate.state.borrow_mut().image_zoom = zoom;
                    }
                }
            }
            _ => self.studio_plates.clear(),
        }
        self.compare_caption = (
            self.session.comparison_caption(false),
            self.session.comparison_caption(true),
        );

        self.rehome = false;
        self.built = self.revision;
    }

    /// Puts every plate back on the window it opened with: the four figure
    /// plates, the comparison panes, the workbench and the studio preview. One
    /// action for one idea — "show me everything again".
    pub fn reset_all_views(&mut self) {
        for plate in &self.plates {
            plate.reset_view();
        }
        for plate in &self.compare_plates {
            plate.reset_view();
        }
        for plate in &self.fit_plates {
            plate.reset_view();
        }
        for plate in &self.studio_plates {
            plate.reset_view();
        }
        self.session.status.info("every view is back to its full range");
        self.touch();
    }

    /// Keeps the current wavelength on (b), or the delay on (c), from the
    /// keyboard or the panel.
    pub fn keep_current(&mut self, wavelength: bool) {
        // Pinning is not the same as adding: the linked curve is already on the
        // plate, so keeping it means it stays when the selection moves on.
        let outcome = if wavelength {
            self.session.save_current_trace()
        } else {
            self.session.save_current_spectrum()
        };
        match outcome {
            Ok(()) => {
                let count = if wavelength {
                    self.session.comparison_wavelengths.len()
                } else {
                    self.session.comparison_delays.len()
                };
                self.session.status.info(format!(
                    "kept on ({}) — {count} curve(s)",
                    if wavelength { "b" } else { "c" }
                ));
                self.touch();
            }
            Err(error) => self.session.status.problem(format!("{error}")),
        }
    }

    /// Steps the linked selection along an axis, which is what the arrow keys
    /// do. `delta` is in measured samples, so a step is one measurement.
    pub fn step_linked(&mut self, wavelength: bool, delta: isize) {
        let Some(dataset) = self.session.dataset() else {
            return;
        };
        let version = self.session.version.unwrap_or(dataset.active_version);
        let Ok((wavelength_axis, delay_axis)) = dataset.axes(version) else {
            return;
        };
        let (axis, wanted) = if wavelength {
            (
                &wavelength_axis,
                self.session
                    .trace
                    .as_ref()
                    .map(|trace| trace.wavelength.coordinate),
            )
        } else {
            (
                &delay_axis,
                self.session
                    .spectrum
                    .as_ref()
                    .map(|spectrum| spectrum.delay.coordinate),
            )
        };
        let Some(wanted) = wanted else {
            return;
        };
        let (index, _) = axis.nearest(wanted);
        let next = (index as isize + delta).clamp(0, axis.len() as isize - 1) as usize;
        let value = axis.values()[next];
        let outcome = if wavelength {
            self.session.select_wavelength(value)
        } else {
            self.session.select_delay(value)
        };
        if let Err(error) = outcome {
            self.session.status.problem(format!("{error}"));
        }
        self.touch();
    }

    /// Seeds the comparison with what is loaded, for a window that opens
    /// directly into the comparison layout.
    pub fn seed_comparison(&mut self) {
        self.session.comparison_mut();
        if self.session.comparison_series().is_empty() {
            let _ = self.session.add_every_dataset();
        }
    }

    /// Fits the current selection once, synchronously.
    ///
    /// The interactive path runs on the background executor; this exists for a
    /// smoke test that has no window loop to post an answer back to, and for a
    /// scripted check of the fitted curve's drawing.
    pub fn fit_once(&mut self) -> ta_fitting::Result<ta_core::FitResult> {
        let (request, trace) = self
            .session
            .fit_request()
            .map_err(|error| ta_fitting::FitError::Invalid(format!("{error}")))?;
        let result = ta_fitting::fit_trace(&trace, &request)?;
        self.session.retain_fit(result.clone()).map_err(|error| {
            ta_fitting::FitError::Invalid(format!("the fit could not be stored: {error}"))
        })?;
        self.session.fit_session().result = Some(result.clone());
        self.session.fit_session().state = crate::fit_panel::FitState::Done;
        Ok(result)
    }

    /// Applies an edit to one editable number, wherever it was typed.
    pub fn on_field_change(&mut self, id: crate::widgets::FieldId, text: &str, cx: &mut Context<Self>) {
        use crate::widgets::FieldId;
        match id {
            FieldId::Wavelength => {
                let values = crate::widgets::parse_numbers(text);
                // Each entry adds a trace; the last becomes the highlighted link.
                for coordinate in &values {
                    if let Err(error) = self.session.select_wavelength(*coordinate) {
                        self.session.status.problem(format!("{error}"));
                    }
                }
                self.touch();
            }
            FieldId::Delay => {
                let values = crate::widgets::parse_numbers(text);
                self.session.comparison_delays =
                    values.iter().copied().take(values.len().saturating_sub(1)).collect();
                if let Some(coordinate) = values.last() {
                    if let Err(error) = self.session.select_delay(*coordinate) {
                        self.session.status.problem(format!("{error}"));
                    }
                }
                self.touch();
            }
            FieldId::HalfWidth => {
                if let Some(value) = crate::widgets::parse_number(text) {
                    let mut sampling = self.session.sampling;
                    sampling.half_width = value.abs();
                    self.set_sampling(sampling);
                }
            }
            FieldId::Channels => {
                if let Some(value) = crate::widgets::parse_number(text) {
                    let mut sampling = self.session.sampling;
                    sampling.channels = (value.abs() as usize).max(1);
                    self.set_sampling(sampling);
                }
            }
            FieldId::SeriesLabel(index) => {
                if let Some(series) = self.session.comparison_series_at_mut(index) {
                    series.label = text.trim().to_string();
                }
                self.touch();
            }
            FieldId::SeriesScale(index) => {
                if let Some(value) = crate::widgets::parse_number(text) {
                    if let Some(series) = self.session.comparison_series_at_mut(index) {
                        series.display.scale = value;
                    }
                }
                self.touch();
            }
            FieldId::SeriesOffset(index) => {
                if let Some(value) = crate::widgets::parse_number(text) {
                    if let Some(series) = self.session.comparison_series_at_mut(index) {
                        series.display.offset = value;
                    }
                }
                self.touch();
            }
            FieldId::SeriesShift(index) => {
                if let Some(value) = crate::widgets::parse_number(text) {
                    if let Some(series) = self.session.comparison_series_at_mut(index) {
                        series.display.coordinate_offset = value;
                    }
                }
                self.touch();
            }
            FieldId::FitValue(index) => {
                if let Some(value) = crate::widgets::parse_number(text) {
                    if let Some(fit) = self.session.fit.as_mut() {
                        if let Some(parameter) = fit.parameters.get_mut(index) {
                            parameter.value = value;
                        }
                    }
                }
            }
            FieldId::FitBounds(index) => {
                let (low, high) = crate::widgets::parse_bounds(text);
                if let Some(fit) = self.session.fit.as_mut() {
                    if let Some(parameter) = fit.parameters.get_mut(index) {
                        parameter.min = low;
                        parameter.max = high;
                    }
                }
            }
            FieldId::FitRange => {
                let range = crate::widgets::parse_range(text).map(|(low, high)| {
                    (low, if high.is_finite() { high } else { f64::MAX })
                });
                if let Some(fit) = self.session.fit.as_mut() {
                    fit.fit_range = range;
                }
            }
            FieldId::FitExcluded => {
                let intervals = crate::widgets::parse_intervals(text);
                if let Some(fit) = self.session.fit.as_mut() {
                    fit.excluded = intervals;
                }
            }
            FieldId::FitReferenceDelay => {}
            FieldId::FigureWidth
            | FieldId::FigureHeight
            | FieldId::FigureFontSize
            | FieldId::FigureTickSize
            | FieldId::LegendFontSize
            | FieldId::AxisXLabel
            | FieldId::AxisYLabel
            | FieldId::AxisXRange
            | FieldId::AxisYRange
            | FieldId::ContourMin
            | FieldId::ContourMax
            | FieldId::ContourLevels
            | FieldId::AnnotationText
            | FieldId::AnnotationX
            | FieldId::AnnotationY
            | FieldId::LegendLabel(_) => self.on_figure_field(id, text),
        }
        cx.notify();
    }

    /// Rebuilds what the contour's image is made from.
    ///
    /// The image itself is built later, by the plate, at the plot rectangle's
    /// size — an image built here would be blitted at its own pixel size and
    /// cover only part of the plate.
    fn ensure_contour_source(&mut self, spec: &ta_core::ContourSpec) {
        let key = self.contour_key();
        if self.contour_source.is_some() && self.contour_key == key {
            return;
        }
        self.contour_source = None;
        let Some(dataset) = self.session.dataset() else {
            return;
        };
        let Some(version) = self.session.version_id() else {
            return;
        };
        let (Ok((wavelength, delay)), Ok(matrix)) = (dataset.axes(version), dataset.matrix(version))
        else {
            return;
        };
        self.contour_source = Some(std::rc::Rc::new(HeatmapSource {
            wavelength: wavelength.clone(),
            delay: delay.clone(),
            matrix: std::sync::Arc::new(matrix.clone()),
            spec: spec.clone(),
            sampling: ta_plot::RasterSampling::Nearest,
            background: Rgba::rgb(255, 255, 255),
            source: Some((dataset.id, version)),
        }));
    }

    // -- actions -----------------------------------------------------------

    fn import_matrix(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose a TA matrix".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let answer = receiver.await;
            let _ = this.update_in(cx, |workspace, _window, cx| {
                match answer {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.first() {
                            workspace.open_path(path);
                        }
                    }
                    Ok(Ok(None)) => workspace.session.status.info("import cancelled"),
                    Ok(Err(error)) => workspace
                        .session
                        .status
                        .problem(format!("the file dialog failed: {error}")),
                    Err(_) => workspace
                        .session
                        .status
                        .problem("the file dialog was dismissed"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Imports a matrix from a path.
    ///
    /// Two layouts are read: the two-column-header sheet, and a plain matrix
    /// with its axis files beside it. The first is tried first, and any failure
    /// falls through to the second rather than being reported as fatal — a file
    /// that is not a sheet is usually the other shape.
    pub fn open_path(&mut self, path: &std::path::Path) {
        if path.extension().is_some_and(|extension| extension == "taproject") {
            match self.session.load_project(path) {
                Ok(()) => {
                    self.restore_workspace();
                    self.fields = widgets::Fields::default();
                    self.touch_matrix();
                }
                Err(error) => self.session.status.problem(format!("{error}")),
            }
            return;
        }
        let sheet = ta_io::import_ta_sheet(path, ta_io::Units::default());
        match sheet {
            Ok(dataset) => {
                self.session.open_dataset(dataset);
                self.touch_matrix();
            }
            Err(sheet_error) => match stacked_axes(path) {
                Some((matrix, wavelength, delay)) => {
                    match ta_io::import_matrix_with_axes(
                        &ta_io::name_from_path(path),
                        &matrix,
                        &wavelength,
                        &delay,
                        ta_io::Units::default(),
                    ) {
                        Ok(dataset) => {
                            self.session.open_dataset(dataset);
                            self.touch_matrix();
                        }
                        Err(matrix_error) => self.session.status.problem(format!(
                            "{} is neither a TA sheet ({sheet_error}) nor a matrix with axis files ({matrix_error})",
                            path.display()
                        )),
                    }
                }
                None => self.session.status.problem(format!(
                    "Cannot import {}: {sheet_error}. For a plain matrix, put matching <matrix>_tawavelength and <matrix>_tatime files beside it, or import a TA sheet with axes.",
                    path.display()
                )),
            },
        }
    }

    fn save_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.record_workspace();
        if let Some(path) = self.session.path.clone() {
            if let Err(error) = self.session.save_project(&path) {
                self.session.status.problem(format!("{error}"));
            }
            return;
        }
        let directory = std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir());
        let receiver = cx.prompt_for_new_path(&directory, Some("Untitled.taproject"));
        cx.spawn_in(window, async move |this, cx| {
            let answer = receiver.await;
            let _ = this.update_in(cx, |workspace, _, cx| {
                match answer {
                    Ok(Ok(Some(path))) => {
                        workspace.record_workspace();
                        if let Err(error) = workspace.session.save_project(&path) {
                            workspace.session.status.problem(format!("{error}"));
                        }
                    }
                    Ok(Ok(None)) => workspace.session.status.info("save cancelled; your work is still open"),
                    Ok(Err(error)) => workspace.session.status.problem(format!("cannot open the save dialog: {error}")),
                    Err(_) => workspace.session.status.problem("save dialog closed; your work is still open"),
                }
                cx.notify();
            });
        }).detach();
    }

    fn record_workspace(&mut self) {
        let layout = self.session.project.layout.get_or_insert_with(|| serde_json::json!({}));
        if !layout.is_object() {
            *layout = serde_json::json!({});
        }
        layout["workspace"] = serde_json::json!({
            "layout": self.layout.label(),
            "index": self.show_index,
            "inspector": self.show_inspector,
            "plate": self.current_plate.letter(),
        });
    }

    pub fn restore_workspace(&mut self) {
        if let Some(view) = self.session.project.layout.as_ref().and_then(|layout| layout.get("workspace")) {
            self.layout = Layout::ALL.into_iter()
                .find(|layout| Some(layout.label()) == view.get("layout").and_then(|value| value.as_str()))
                .unwrap_or(Layout::Figure);
            self.show_index = view.get("index").and_then(|value| value.as_bool()).unwrap_or(true);
            self.show_inspector = view.get("inspector").and_then(|value| value.as_bool()).unwrap_or(true);
            self.current_plate = PlateId::ALL.into_iter()
                .find(|plate| Some(plate.letter()) == view.get("plate").and_then(|value| value.as_str()))
                .unwrap_or(PlateId::Kinetics);
        } else {
            self.layout = Layout::Figure;
            self.show_index = true;
            self.show_inspector = true;
        }
    }

    fn open_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose a project".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let answer = receiver.await;
            let _ = this.update_in(cx, |workspace, _window, cx| {
                match answer {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.first() {
                            let result = workspace.session.load_project(path);
                            if let Err(error) = result {
                                workspace.session.status.problem(format!("{error}"));
                            } else {
                                workspace.restore_workspace();
                                workspace.fields = widgets::Fields::default();
                            }
                            workspace.touch_matrix();
                        }
                    }
                    Ok(Ok(None)) => workspace.session.status.info("open cancelled"),
                    Ok(Err(error)) => workspace
                        .session
                        .status
                        .problem(format!("the file dialog failed: {error}")),
                    Err(_) => workspace
                        .session
                        .status
                        .problem("the file dialog was dismissed"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_sampling(&mut self, sampling: Sampling) {
        self.session.set_sampling(sampling);
        self.touch();
    }

    /// Registers the workspace's keys. Called once, before the window opens.
    pub fn bind_shortcuts(cx: &mut App) {
        use gpui_kit::KeyBinding;
        cx.bind_keys([
            // The arrows walk the two axes, one measured sample at a time.
            KeyBinding::new("left", WavelengthBack, Some("SpectraStudio && !Input")),
            KeyBinding::new("right", WavelengthForward, Some("SpectraStudio && !Input")),
            KeyBinding::new("up", DelayBack, Some("SpectraStudio && !Input")),
            KeyBinding::new("down", DelayForward, Some("SpectraStudio && !Input")),
            KeyBinding::new("b", KeepWavelength, Some("SpectraStudio && !Input")),
            KeyBinding::new("c", KeepDelay, Some("SpectraStudio && !Input")),
            KeyBinding::new("f", FitTrace, Some("SpectraStudio && !Input")),
            KeyBinding::new("0", ResetViews, Some("SpectraStudio && !Input")),
            KeyBinding::new("ctrl-s", SaveProject, Some("SpectraStudio")),
            KeyBinding::new("cmd-s", SaveProject, Some("SpectraStudio")),
        ]);
    }

    /// Where the key actions land. Each one moves the selection or opens a
    /// workspace; none of them changes a number.
    fn handle_shortcut(&mut self, action: &ShortcutAction, cx: &mut Context<Self>) {
        match action {
            ShortcutAction::WavelengthBack => self.step_linked(true, -1),
            ShortcutAction::WavelengthForward => self.step_linked(true, 1),
            ShortcutAction::DelayBack => self.step_linked(false, -1),
            ShortcutAction::DelayForward => self.step_linked(false, 1),
            ShortcutAction::KeepWavelength => self.keep_current(true),
            ShortcutAction::KeepDelay => self.keep_current(false),
            ShortcutAction::FitTrace => self.open_fit(),
            ShortcutAction::ResetViews => self.reset_all_views(),
        }
        cx.notify();
    }

    // -- the regions of the shell -----------------------------------------

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self.session.title();
        let dirty = if self.session.dirty { " · unsaved" } else { "" };
        div()
            .h(px(HEADER_HEIGHT))
            .w_full()
            .flex()
            .items_center()
            .gap_5()
            .px_4()
            .bg(ink::ground())
            .border_b_1()
            .border_color(ink::text())
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ink::text())
                    .child("SPECTRA STUDIO"),
            )
            .child(
                div()
                    .text_size(px(12.5))
                    .text_color(ink::muted())
                    .child(format!("{title}{dirty}")),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .justify_end()
                    .items_center()
                    .gap_5()
                    .child(self.action("Import matrix…", true, cx, |workspace, window, cx| {
                        workspace.import_matrix(window, cx);
                    }))
                    .child(self.action("Open project…", false, cx, |workspace, window, cx| {
                        workspace.open_project(window, cx);
                    }))
                    .child(self.action("Save project", false, cx, |workspace, window, cx| {
                        workspace.save_project(window, cx);
                    }))
                    .child(self.action("Export figure…", false, cx, |workspace, _, _| {
                        if workspace.layout == Layout::Studio {
                            let format = workspace.studio_figure().map(|figure| figure.export.format)
                                .unwrap_or(ExportFormat::Svg);
                            workspace.export_studio(format);
                        } else if workspace.layout == Layout::Compare {
                            workspace.open_comparison_studio(workspace.current_plate == PlateId::Spectra);
                        } else {
                            workspace.open_studio(workspace.current_plate);
                        }
                    })),
            )
    }

    /// A label on a hairline rule: the world's control, not a boxed button.
    fn action(
        &self,
        label: &'static str,
        accent: bool,
        cx: &mut Context<Self>,
        run: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        let color = if accent { ink::accent() } else { ink::text() };
        let run = std::rc::Rc::new(run);
        gpui_kit::base::Button::new(ElementId::Name(label.into()))
            .accessibility_label(label)
            .cursor_pointer()
            .flex_shrink_0()
            .h(px(24.0))
            .px_1()
            .text_size(px(12.5))
            .text_color(color)
            .border_b_1()
            .border_color(if accent { ink::accent() } else { ink::rule() })
            .pb(px(1.0))
            .hover(move |style| style.text_color(ink::accent()).bg(ink::paper()))
            .focus(|style| style.border_b_2().border_color(ink::accent()).bg(ink::paper()))
            .child(label)
            .on_click(cx.listener(move |workspace, _event, window, cx| {
                run(workspace, window, cx);
                workspace.touch();
                workspace.sync_studio_fields(window, cx);
                cx.notify();
            }))
    }

    fn plate_index(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let entries = self.session.chain();
        // Every dataset in the project, so a second import does not make the
        // first one unreachable.
        let datasets: Vec<AnyElement> = self
            .session
            .datasets()
            .into_iter()
            .map(|(id, name, versions, active)| {
                div()
                    .id(ElementId::Name(format!("dataset-{id}").into()))
                    .flex()
                    .items_baseline()
                    .justify_between()
                    .px_3()
                    .py_1()
                    .bg(if active { ink::paper() } else { ink::ground() })
                    .border_b_1()
                    .border_color(ink::hair())
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(if active { ink::accent() } else { ink::text() })
                            .child(name),
                    )
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(ink::muted())
                            .child(format!("{versions} versions")),
                    )
                    .on_click(cx.listener(move |workspace, _event, _window, cx| {
                        if Some(id) != workspace.session.opened {
                            workspace.session.open_bench(id);
                            workspace.touch_matrix();
                        }
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect();
        let dataset = self.session.dataset().map(|dataset| dataset.name.clone());
        let dataset_open = dataset.is_some();
        let mut rows: Vec<AnyElement> = Vec::new();
        for entry in entries {
            let active = entry.is_active;
            let id = entry.id;
            let color = if active { ink::accent() } else { ink::text() };
            rows.push(
                div()
                    .id(ElementId::Name(format!("version-{id}").into()))
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_3()
                    .py_2()
                    .bg(if active { ink::paper() } else { ink::ground() })
                    .border_b_1()
                    .border_color(ink::hair())
                            .hover(|style| style.bg(ink::paper()))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .text_color(color)
                                    .child(entry.label.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .text_color(ink::muted())
                                    .child(format!("{}×{}", entry.shape.0, entry.shape.1)),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ink::muted())
                            .child(entry.operation.clone()),
                    )
                    .on_click(cx.listener(move |workspace, _event, _window, cx| {
                        let result = workspace.session.set_version(id);
                        if let Err(error) = result {
                            workspace.session.status.problem(format!("{error}"));
                        }
                        workspace.touch_matrix();
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }

        let many = datasets.len() > 1;
        let first_label = if many {
            "datasets".to_string()
        } else {
            dataset.clone().unwrap_or_else(|| "no dataset".to_string())
        };
        let second_label = "versions — click to restore".to_string();
        div()
            .id(ElementId::Name("dataset-index".into()))
            .overflow_y_scroll()
            .w(px(INDEX_WIDTH))
            .h_full()
            .flex()
            .flex_col()
            .bg(ink::ground())
            .border_r_1()
            .border_color(ink::rule())
            .child(self.section_label(first_label))
            .children(datasets)
            .child(self.section_label(second_label))
            .children(rows)
            .children(if dataset_open {
                Vec::new()
            } else {
                vec![div()
                    .p_3()
                    .text_size(px(11.0))
                    .text_color(ink::muted())
                    .child("Import a matrix to add the first plate.")
                    .into_any_element()]
            })
    }

    fn section_label(&self, text: String) -> impl IntoElement {
        div()
            .px_3()
            .py_2()
            .text_size(px(10.0))
            .text_color(ink::muted())
            .border_b_1()
            .border_color(ink::rule())
            .child(text.to_uppercase())
    }

    fn plate_block(&mut self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let plate_id = PlateId::ALL[index];
        let caption = self.caption_of[index].clone();
        let letter = plate_id.letter().to_string();
        let plate = &self.plates[index];
        let weak = cx.weak_entity();
        let element = plate
            .element(ElementId::Name(format!("plate-{letter}").into()))
            .on_gesture(move |gesture, _window, cx| {
                let Some(view) = weak.upgrade() else {
                    return;
                };
                view.update(cx, |workspace, cx| {
                    if matches!(gesture, PlotGesture::Select { .. } | PlotGesture::Region { .. }) {
                        if let Some(focus) = &workspace.focus {
                            focus.focus(_window, cx);
                        }
                    }
                    workspace.apply_gesture(plate_id, gesture);
                    cx.notify();
                });
            });
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(ink::paper())
            .child(div().flex_1().w_full().min_h(px(40.0)).child(element))
            .child(
                div()
                    .px_2()
                    .py_1()
                    .border_t_1()
                    .border_color(ink::hair())
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(ink::accent())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!("({letter})")),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .text_size(px(11.0))
                                    .text_color(ink::text())
                                    .child(caption),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .justify_end()
                            .gap_2()
                            // §17.1: every plate reaches the studio, so the way
                            // in is on the plate rather than in a menu. The
                            // controls say what they do: "studio" and "data"
                            // named nothing a new user could act on.
                            .child(widgets::action_id(
                                "FigureStudio",
                                ElementId::Name(format!("studio-{letter}").into()),
                                false,
                                cx,
                                move |workspace, _, _| workspace.open_studio(plate_id),
                            ))
                            .child(widgets::action_id(
                                "Export data",
                                ElementId::Name(format!("data-{letter}").into()),
                                false,
                                cx,
                                move |workspace, _, _| workspace.export_plate_data(plate_id),
                            ))
                            // §16.5: preparing Glotaran input is on the plate
                            // itself, not behind a workspace switch.
                            .child(if plate_id == PlateId::Contour {
                                widgets::action_id(
                                    "Glotaran input",
                                    ElementId::Name("glotaran-export".into()),
                                    false,
                                    cx,
                                    |workspace, _, _| workspace.export_glotaran(),
                                )
                                .into_any_element()
                            } else {
                                div().into_any_element()
                            }),
                    )
                    .min_h(px(CAPTION_HEIGHT))
                    .flex_shrink_0(),
            )
    }

    /// Applies a gesture from a plate to the session.
    fn apply_gesture(&mut self, plate: PlateId, gesture: PlotGesture) {
        if matches!(gesture, PlotGesture::Select { .. } | PlotGesture::Region { .. }) {
            self.current_plate = plate;
        }
        match gesture {
            PlotGesture::Select { data, .. } => {
                let result = match plate {
                    PlateId::Contour => self.session.select_point(data.0, data.1),
                    PlateId::Kinetics => self.session.select_delay(data.0),
                    PlateId::Spectra => self.session.select_wavelength(data.0),
                    PlateId::TimeZero => Ok(()),
                };
                if let Err(error) = result {
                    self.session.status.problem(format!("{error}"));
                }
                self.touch();
            }
            PlotGesture::Region { from, to } => {
                // A region on the contour averages its wavelengths into a
                // window; on the other plates it selects the middle.
                match plate {
                    PlateId::Contour => {
                        let centre_wavelength = (from.0 + to.0) / 2.0;
                        let centre_delay = (from.1 + to.1) / 2.0;
                        let half_width = ((to.0 - from.0).abs() / 2.0).max(0.5);
                        self.session.set_sampling(Sampling {
                            mode: SampleMode::AverageWindow { half_width },
                            half_width,
                            channels: self.session.sampling.channels,
                        });
                        let result = self.session.select_point(centre_wavelength, centre_delay);
                        if let Err(error) = result {
                            self.session.status.problem(format!("{error}"));
                        }
                    }
                    PlateId::Kinetics => {
                        let _ = self.session.select_delay((from.0 + to.0) / 2.0);
                    }
                    PlateId::Spectra => {
                        let _ = self.session.select_wavelength((from.0 + to.0) / 2.0);
                    }
                    PlateId::TimeZero => {}
                }
                self.touch();
            }
            PlotGesture::Nearest { point, .. } => {
                self.session
                    .status
                    .info(format!("{:.4} · {:.4}", point.0, point.1));
            }
            PlotGesture::Zoom { .. }
            | PlotGesture::Pan { .. }
            | PlotGesture::BoxZoom { .. }
            | PlotGesture::Hover(_)
            | PlotGesture::RegionPreview { .. } => {}
        }
    }

    fn inspector(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sampling = self.session.sampling;
        let trace = self
            .session
            .trace
            .as_ref()
            .map(|trace| format!("{:.4}", trace.wavelength.coordinate))
            .unwrap_or_else(|| "—".to_string());
        let spectrum = self
            .session
            .spectrum
            .as_ref()
            .map(|spectrum| format!("{:.4}", spectrum.delay.coordinate))
            .unwrap_or_else(|| "—".to_string());
        let trace_label = self
            .session
            .current_trace()
            .map(|trace| trace.label)
            .unwrap_or_else(|_| "—".to_string());
        let limits = plates::colour_limits(&self.session);

        // The two coordinates are typeset here as well as read out: a figure
        // is often wanted at exactly 620 nm, not at whatever the pointer found.
        let wavelength_state = self.field_state(
            widgets::FieldId::Wavelength,
            trace.clone(),
            "600, 620, 700",
            window,
            cx,
        );
        let delay_state = self.field_state(
            widgets::FieldId::Delay,
            spectrum.clone(),
            "-1, 1, 100",
            window,
            cx,
        );
        let half_width_state = self.field_state(
            widgets::FieldId::HalfWidth,
            format!("{:.2}", sampling.half_width),
            "5",
            window,
            cx,
        );
        let channels_state = self.field_state(
            widgets::FieldId::Channels,
            format!("{}", sampling.channels),
            "3",
            window,
            cx,
        );

        div()
            .id(ElementId::Name("inspector".into()))
            .w(px(INSPECTOR_WIDTH))
            .h_full()
            .flex()
            .flex_col()
            // The panel is taller than a laptop window, so it scrolls rather
            // than hiding its last rows below the edge.
            .overflow_y_scroll()
            .bg(ink::ground())
            .border_l_1()
            .border_color(ink::rule())
            .child(self.inspector_section("figure-selection", "Selection", true,
                div().flex().flex_col().child(self.field(
                "Wavelengths (nm)",
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(widgets::number_input(&wavelength_state, 250.0))
                    .child(widgets::readout(
                        "selections accumulate; the last is highlighted",
                    )),
            ))
            .child(self.field(
                "Delays (ps)",
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(widgets::number_input(&delay_state, 250.0))
                    .child(widgets::readout(
                        "the last is the linked spectrum; negatives are kept",
                    )),
            )), cx))
            .child(self.inspector_section("figure-sampling", "Sampling & averaging", false,
                div().flex().flex_col().child(self.field(
                "Reading",
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children(
                        [
                            ("nearest", SampleMode::Nearest),
                            ("interpolated", SampleMode::Interpolated),
                            (
                                "window",
                                SampleMode::AverageWindow {
                                    half_width: sampling.half_width,
                                },
                            ),
                            (
                                "band",
                                SampleMode::AverageChannels {
                                    count: sampling.channels,
                                },
                            ),
                        ]
                        .into_iter()
                        .map(|(label, mode)| {
                            let active = sampling.mode == mode
                                || matches!(
                                    (sampling.mode, mode),
                                    (
                                        SampleMode::AverageWindow { .. },
                                        SampleMode::AverageWindow { .. }
                                    ) | (
                                        SampleMode::AverageChannels { .. },
                                        SampleMode::AverageChannels { .. }
                                    )
                                );
                            let chosen = Sampling {
                                mode,
                                ..sampling
                            };
                            let id = ElementId::Name(format!("mode-{label}").into());
                            div()
                                .id(id)
                                .px_2()
                                .py_1()
                                .text_size(px(11.5))
                                .text_color(if active { ink::paper() } else { ink::text() })
                                .bg(if active { ink::accent() } else { ink::paper() })
                                .border_1()
                                .border_color(if active { ink::accent() } else { ink::rule() })
                                .child(label)
                                .on_click(cx.listener(move |workspace, _event, _window, cx| {
                                    workspace.set_sampling(chosen);
                                    cx.notify();
                                }))
                                .into_any_element()
                        }),
                    ),
            ))
            // The window fields are shown only when the reading averages over
            // one: a control that cannot change the answer is noise.
            .child(if matches!(
                sampling.mode,
                SampleMode::AverageWindow { .. } | SampleMode::AverageChannels { .. }
            ) {
                self.field(
                    "Window",
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(widgets::number_input(&half_width_state, 62.0))
                        .child(widgets::readout("nm ±"))
                        .child(widgets::number_input(&channels_state, 48.0))
                        .child(widgets::readout("channels")),
                )
                .into_any_element()
            } else {
                div().into_any_element()
            }), cx))
            .child(self.inspector_section("figure-kinetics", "Kinetics · retained wavelengths", true,
                self.kept_block(
                "b",
                "Kinetics",
                trace_label.clone(),
                self.session
                    .comparison_wavelengths
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        (self.session.wavelength_label(*value), *value, index)
                    })
                    .collect(),
                true,
                cx,
            ), cx))
            .child(self.inspector_section("figure-spectra", "Spectra · retained delays", false,
                self.kept_block(
                "c",
                "Spectra",
                self.session
                    .spectrum
                    .as_ref()
                    .map(|spectrum| self.session.delay_label(spectrum.delay.coordinate))
                    .unwrap_or_else(|| "—".to_string()),
                self.session
                    .comparison_delays
                    .iter()
                    .enumerate()
                    .map(|(index, value)| (self.session.delay_label(*value), *value, index))
                    .collect(),
                false,
                cx,
            ), cx))
            .child(self.inspector_section("figure-display", "Display options", false,
                div().flex().flex_col().child(self.field(
                "Colour limits",
                widgets::readout(format!(
                    "{:.4} … {:.4} ΔOD, symmetric",
                    limits.min, limits.max
                )),
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(ink::hair())
                    .child(widgets::switch(
                        "(d) time-zero plate",
                        self.show_time_zero_plate || self.show_time_zero,
                        cx,
                        |workspace| {
                            workspace.show_time_zero_plate = !workspace.show_time_zero_plate;
                            workspace.touch();
                        },
                    )),
            ), cx))
            .child(self.inspector_section("figure-actions", "Analysis & export", true,
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .p_3()
                    .child(self.inspector_action("Fit this trace", cx, |workspace| {
                        workspace.open_fit();
                    }))
                    .child(self.inspector_action("FigureStudio…", cx, |workspace| {
                        let plate = workspace.current_plate;
                        workspace.open_studio(plate);
                    }))
                    .child(self.inspector_action("Reset all views", cx, |workspace| {
                        workspace.reset_all_views();
                    })),
                cx))
            .child(self.inspector_section("figure-keys", "Keyboard shortcuts", false, self.shortcut_list(), cx))
    }

    /// One linked plate's selection: what is drawn, how to add another, and the
    /// list of what is already there. Removing a curve is one click on it.
    fn kept_block(
        &self,
        letter: &'static str,
        title: &'static str,
        linked: String,
        kept: Vec<(String, f64, usize)>,
        kinetics: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let count = kept.len();
        let chips: Vec<AnyElement> = kept
            .into_iter()
            .map(|(label, _value, index)| {
                // A chip reads as the number it removes rather than as an
                // anonymous cross, so the list can be edited without hovering.
                div()
                    .id(ElementId::Name(format!("kept-{letter}-{index}").into()))
                    .px_2()
                    .py_1()
                    .text_size(px(11.0))
                    .text_color(ink::text())
                    .bg(ink::paper())
                    .border_1()
                    .border_color(ink::rule())
                    .hover(|style| style.border_color(ink::accent()).text_color(ink::accent()))
                    .child(format!("{label} ✕"))
                    .on_click(cx.listener(move |workspace, _event, _window, cx| {
                        if kinetics {
                            workspace.session.remove_wavelength(index);
                        } else {
                            workspace.session.remove_delay(index);
                        }
                        workspace.touch();
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect();
        let summary = if count == 0 {
            "nothing added yet".to_string()
        } else {
            format!("{count} added")
        };
        let default_grid = !kinetics;

        div()
            .flex()
            .flex_col()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(ink::hair())
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(ink::muted())
                            .child(format!("({letter}) {}", title.to_uppercase())),
                    )
                    .child(widgets::readout(summary)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(widgets::value_text(linked))
                    .child(widgets::readout(if kinetics {
                        "the linked trace"
                    } else {
                        "the linked spectrum"
                    })),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(widgets::action(
                        if kinetics {
                            "Add this wavelength"
                        } else {
                            "Add this delay"
                        },
                        false,
                        cx,
                        move |workspace, _, _| {
                            let outcome = if kinetics {
                                workspace.session.add_current_wavelength()
                            } else {
                                workspace.session.add_current_delay()
                            };
                            match outcome {
                                Ok(()) => {
                                    workspace.session.status.info(if kinetics {
                                        "wavelength kept on the kinetics plate"
                                    } else {
                                        "delay kept on the spectra plate"
                                    });
                                    workspace.touch();
                                }
                                Err(error) => workspace
                                    .session
                                    .status
                                    .problem(format!("{error}")),
                            }
                        },
                    ))
                    .child(if default_grid {
                        widgets::action("Default grid", false, cx, |workspace, _, _| {
                            workspace.session.seed_default_delays();
                            workspace.session.status.info(format!(
                                "the spectra plate is showing the default grid",
                            ));
                            workspace.touch();
                        })
                        .into_any_element()
                    } else {
                        widgets::action("Clear", false, cx, |workspace, _, _| {
                            workspace.session.clear_wavelengths();
                            workspace.touch();
                        })
                        .into_any_element()
                    })
                    .child(if default_grid {
                        widgets::action("Clear", false, cx, |workspace, _, _| {
                            workspace.session.clear_delays();
                            workspace.touch();
                        })
                        .into_any_element()
                    } else {
                        // A run of wavelengths is added one after another: the
                        // selection steps on to the next measured sample.
                        widgets::action("Next sample →", false, cx, |workspace, _, _| {
                            workspace.step_selection(1)
                        })
                        .into_any_element()
                    }),
            )
            .child(div().flex().flex_wrap().gap_1().children(chips))
    }

    /// Moves the linked selection one measured sample along the axis, so a run
    /// of curves can be built by clicking Add and then this.
    fn step_selection(&mut self, delta: isize) {
        let Some(dataset) = self.session.dataset() else {
            return;
        };
        let version = self.session.version.unwrap_or(dataset.active_version);
        let Ok((wavelength, delay)) = dataset.axes(version) else {
            return;
        };
        let Some(wanted) = self
            .session
            .trace
            .as_ref()
            .map(|trace| trace.wavelength.coordinate)
        else {
            return;
        };
        let _ = delay;
        let (index, _) = wavelength.nearest(wanted);
        let next = (index as isize + delta).clamp(0, wavelength.len() as isize - 1) as usize;
        let value = wavelength.values()[next];
        let outcome = self.session.select_wavelength(value);
        if let Err(error) = outcome {
            self.session.status.problem(format!("{error}"));
        }
        self.touch();
    }

    /// The keys, listed where a user looks when they are stuck. A shortcut
    /// nobody can find is a shortcut nobody uses, so the list stays short and
    /// sits in the panel rather than in a dialog.
    fn shortcut_list(&self) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(ink::hair())
            .child(widgets::small_caps("keys"))
            .child(widgets::readout("Ctrl/Cmd+S  save project"))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ink::muted())
                    .child("← →  wavelength   ↑ ↓  delay"),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ink::muted())
                    .child("B  keep on (b)   C  keep on (c)"),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ink::muted())
                    .child("F  fit this trace   0  reset views"),
            )
    }

    fn field(&self, label: &str, value: impl IntoElement) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(ink::hair())
            .child(
                div()
                    .text_size(px(10.5))
                    .text_color(ink::muted())
                    .child(label.to_uppercase()),
            )
            .child(value)
    }

    fn inspector_action(
        &self,
        label: &'static str,
        cx: &mut Context<Self>,
        run: impl Fn(&mut Self) + 'static,
    ) -> impl IntoElement {
        let run = Rc::new(run);
        div()
            .id(ElementId::Name(label.into()))
            .px_2()
            .py_1()
            .text_size(px(11.5))
            .text_color(ink::text())
            .bg(ink::paper())
            .border_1()
            .border_color(ink::rule())
            .hover(|style| style.border_color(ink::accent()).text_color(ink::accent()))
            .child(label)
            .on_click(cx.listener(move |workspace, _event, _window, cx| {
                run(workspace);
                cx.notify();
            }))
    }

    fn footer(&self) -> impl IntoElement {
        let hovered = self
            .plates
            .first()
            .and_then(|plate| plate.hover())
            .map(|(x, y)| format!("λ {x:.2} nm · t {y:.4} ps"))
            .unwrap_or_else(|| "λ — · t —".to_string());
        let address = self.session.trace.as_ref().map(|trace| {
            format!(
                "{} ▸ {} ▸ {:.2} nm",
                self.session
                    .dataset()
                    .map(|dataset| dataset.name.clone())
                    .unwrap_or_else(|| "—".into()),
                self.session
                    .version()
                    .map(|version| version.label.clone())
                    .unwrap_or_else(|| "—".into()),
                trace.wavelength.coordinate
            )
        });
        let status = self.session.status.clone();
        let unit = self
            .session
            .dataset()
            .map(|dataset| dataset.metadata.source_format.label().to_string())
            .unwrap_or_default();

        div()
            .h(px(FOOTER_HEIGHT))
            .w_full()
            .flex()
            .items_center()
            .gap_5()
            .px_4()
            .bg(ink::ground())
            .border_t_1()
            .border_color(ink::rule())
            .text_size(px(11.5))
            .child(div().text_color(ink::text()).child(hovered))
            .child(div().text_color(ink::muted()).child(unit))
            .child(
                div()
                    .text_color(match status.problem {
                        Some(_) => ink::amber(),
                        None => ink::muted(),
                    })
                    .child(status.problem.unwrap_or(status.message)),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .justify_end()
                    .text_color(ink::accent())
                    .child(address.unwrap_or_default()),
            )
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focus = self.focus.get_or_insert_with(|| {
            let focus = cx.focus_handle();
            focus.focus(_window, cx);
            focus
        }).clone();
        if self.built != self.revision {
            self.refresh_scenes();
        }
        // The title bar names the dataset and the layout, so a window among
        // several says which one it is.
        let title = format!("{} — Spectra Studio", self.session.title());
        if title != self.window_title {
            _window.set_window_title(&title);
            self.window_title = title;
        }
        // The coordinate fields follow the selection, so a click on a plate
        // writes the number the user would otherwise have typed.
        self.sync_coordinate_fields(_window, cx);
        let strip = self.layout_strip(cx);
        let body = match self.layout {
            Layout::Compare => self.compare_layout(_window, cx),
            Layout::Fit => self.fit_layout(_window, cx),
            Layout::Studio => self.studio_layout(_window, cx),
            Layout::Figure => self.figure_layout(_window, cx),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(ink::paper())
            .text_color(ink::text())
            // The keyboard belongs to the workspace: the root element is what
            // GPUI walks up from the focused node looking for a handler.
            .track_focus(&focus)
            .key_context("SpectraStudio")
            .on_action(cx.listener(|workspace, _: &WavelengthBack, _window, cx| {
                workspace.handle_shortcut(&ShortcutAction::WavelengthBack, cx);
            }))
            .on_action(cx.listener(|workspace, _: &WavelengthForward, _window, cx| {
                workspace.handle_shortcut(&ShortcutAction::WavelengthForward, cx);
            }))
            .on_action(cx.listener(|workspace, _: &DelayBack, _window, cx| {
                workspace.handle_shortcut(&ShortcutAction::DelayBack, cx);
            }))
            .on_action(cx.listener(|workspace, _: &DelayForward, _window, cx| {
                workspace.handle_shortcut(&ShortcutAction::DelayForward, cx);
            }))
            .on_action(cx.listener(|workspace, _: &KeepWavelength, _window, cx| {
                workspace.handle_shortcut(&ShortcutAction::KeepWavelength, cx);
            }))
            .on_action(cx.listener(|workspace, _: &KeepDelay, _window, cx| {
                workspace.handle_shortcut(&ShortcutAction::KeepDelay, cx);
            }))
            .on_action(cx.listener(|workspace, _: &FitTrace, _window, cx| {
                workspace.handle_shortcut(&ShortcutAction::FitTrace, cx);
            }))
            .on_action(cx.listener(|workspace, _: &ResetViews, _window, cx| {
                workspace.handle_shortcut(&ShortcutAction::ResetViews, cx);
            }))
            .on_action(cx.listener(|workspace, _: &SaveProject, window, cx| {
                workspace.save_project(window, cx);
                cx.notify();
            }))
            .child(self.header(cx))
            .child(strip)
            .child(body)
            .child(self.footer())
    }
}

impl Workspace {
    /// The single-dataset layout: the plate index, the figure block and the
    /// inspector, which is what every other layout is a specialisation of.
    fn figure_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let contour = self.plate_block(0, cx);
        let kinetics = self.plate_block(1, cx);
        let spectra = self.plate_block(2, cx);
        let inspector = self.inspector(window, cx);
        // The two linked plates are what a user reads after the contour, so
        // they take the space that is left rather than a fixed slice of it. The
        // time-zero plate only appears when there is a curve to show, or when
        // the user asks for it.
        let block = v_resizable("figure-block")
            .child(resizable_panel().size(px(380.0)).child(contour))
            .child(resizable_panel().child(
                h_resizable("linked-plates")
                    .child(resizable_panel().child(kinetics))
                    .child(resizable_panel().child(spectra)),
            ));
        let block = if self.show_time_zero || self.show_time_zero_plate {
            block.child(resizable_panel().size(px(190.0)).child(self.plate_block(3, cx)))
        } else {
            block
        };
        div()
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .flex()
            .child(if self.show_index {
                self.plate_index(cx).into_any_element()
            } else {
                div().into_any_element()
            })
            .child(div().flex_1().min_w(px(0.0)).child(block))
            .child(if self.show_inspector {
                inspector.into_any_element()
            } else {
                div().into_any_element()
            })
            .into_any_element()
    }
}

/// A scene from a plate's content, so every plate is built the same way.
fn scene_of(
    content: PlateContent,
    style: ta_core::PublicationStyle,
    heatmap: Option<std::rc::Rc<HeatmapSource>>,
    cursors: Vec<ta_plot_gpui::PlotCursor>,
    show_crosshair: bool,
) -> PanelScene {
    scene_at_size(content, style, heatmap, cursors, show_crosshair, None)
}

/// The same, for a plate that previews a figure at its own physical size.
#[allow(clippy::too_many_arguments)]
fn scene_at_size(
    content: PlateContent,
    style: ta_core::PublicationStyle,
    heatmap: Option<std::rc::Rc<HeatmapSource>>,
    cursors: Vec<ta_plot_gpui::PlotCursor>,
    show_crosshair: bool,
    figure: Option<(f64, f64)>,
) -> PanelScene {
    PanelScene {
        panel: content.panel,
        style,
        series: content.series,
        heatmap,
        // A figure preview keeps the shell's ground and paints its page, so the
        // sheet and its margins can be seen; a working plate is its own paper.
        background: if figure.is_some() {
            Rgba::rgb(238, 241, 246)
        } else {
            Rgba::rgb(255, 255, 255)
        },
        cursors,
        show_crosshair,
        accent: plates::accent(),
        page: figure.map(|_| Rgba::rgb(255, 255, 255)),
        figure,
        interactive: figure.is_none(),
        image_view: figure.is_some(),
    }
}

/// Brings one plate in line with a scene, keeping its window unless the matrix
/// underneath it changed.
fn sync_plate(
    plates: &mut Vec<Plate>,
    index: usize,
    scene: PanelScene,
    raster: SharedHeatmapCache,
    rehome: bool,
) {
    let (x_range, y_range) = default_ranges(&scene.panel);
    match plates.get_mut(index) {
        Some(plate) => {
            plate.set_scene(scene);
            if rehome {
                plate.state.borrow_mut().set_home(x_range, y_range);
            }
        }
        None => {
            let state = PlotState::new(x_range, y_range);
            plates.push(Plate::new(scene, state, raster));
        }
    }
}

/// The default window of a plate: the whole range of the panel, or a unit box
/// when nothing is loaded.
fn default_ranges(panel: &ta_core::PanelModel) -> ((f64, f64), (f64, f64)) {
    (
        panel.x.range.unwrap_or((0.0, 1.0)),
        panel.y.range.unwrap_or((0.0, 1.0)),
    )
}

/// The axis files that usually sit beside a plain matrix.
fn stacked_axes(
    matrix: &std::path::Path,
) -> Option<(PathBuf, PathBuf, PathBuf)> {
    let stem = matrix.file_name()?.to_string_lossy().to_string();
    let directory = matrix.parent()?;
    let base = stem.strip_suffix("_tcorr").unwrap_or(&stem);
    let candidates = [
        (
            directory.join(format!("{stem}_tawavelength")),
            directory.join(format!("{stem}_tatime")),
        ),
        (
            directory.join(format!("{base}_tawavelength")),
            directory.join(format!("{base}_tatime")),
        ),
    ];
    for (wavelength, delay) in candidates {
        if wavelength.exists() && delay.exists() {
            return Some((matrix.to_path_buf(), wavelength, delay));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::Array2;
    use ta_core::{
        Axis, AxisKind, AxisUnit, SignalUnit, SourceFormat, TaDataset,
    };

    #[gpui_kit::test]
    fn rendered_coordinate_inputs_preserve_drafts_until_enter(cx: &mut gpui_kit::TestAppContext) {
        use crate::widgets::FieldId;
        use gpui_kit::{test::TestWindowExt, AppContext};
        let (handle, workspace) = cx.update(|cx| {
            gpui_kit::init(cx);
            Workspace::bind_shortcuts(cx);
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| workspace_with_dataset())
            }).unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let state = workspace.read(cx).fields.get(FieldId::Wavelength).unwrap().clone();
            window.click(("input", state.entity_id()), cx);
        }).unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            let state = workspace.read(cx).fields.get(FieldId::Wavelength).unwrap().clone();
            let original = workspace.read(cx).session.trace.as_ref().unwrap().wavelength.coordinate;
            window.press("ctrl-a", cx);
            window.input("600", cx);
            assert_eq!(state.read(cx).value().as_ref(), "600");
            assert_eq!(workspace.read(cx).session.trace.as_ref().unwrap().wavelength.coordinate, original);
            window.press("enter", cx);
        }).unwrap();
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(workspace.read(cx).session.trace.as_ref().unwrap().wavelength.coordinate, 600.0);
            window.press("ctrl-a", cx);
            window.input("650", cx);
        }).unwrap();
        cx.update_window(handle, |_, window, cx| {
            let state = workspace.read(cx).fields.get(FieldId::Wavelength).unwrap();
            assert_eq!(state.read(cx).value().as_ref(), "650");
            assert_eq!(workspace.read(cx).session.trace.as_ref().unwrap().wavelength.coordinate, 600.0);
            window.press("enter", cx);
        }).unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(workspace.read(cx).session.trace.as_ref().unwrap().wavelength.coordinate, 650.0);
            assert_eq!(workspace.read(cx).plates[1].scene.series.len(), 2);
            let state = workspace.read(cx).fields.get(FieldId::Wavelength).unwrap();
            assert_eq!(state.read(cx).value().as_ref(), "600.0000, 650.0000");
            window.press("ctrl-a", cx);
            window.input("500, 550", cx);
            window.press("enter", cx);
        }).unwrap();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(workspace.read(cx).plates[1].scene.series.len(), 4);
            assert_eq!(workspace.read(cx).session.kinetics_records().len(), 4);
            assert!(workspace.read(cx).plates[1].scene.series.last().unwrap().label.starts_with("550"));
            window.within("figure-kinetics").click("Clear", cx);
        }).unwrap();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(workspace.read(cx).plates[1].scene.series.len(), 1);
            let state = workspace.read(cx).fields.get(FieldId::Wavelength).unwrap();
            assert_eq!(state.read(cx).value().as_ref(), "550.0000");
        }).unwrap();
    }

    #[gpui_kit::test]
    fn inspector_disclosures_toggle_without_changing_analysis(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{test::TestWindowExt, AppContext};
        let (handle, workspace) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| workspace_with_dataset())
            }).unwrap()
        });
        for expected in [true, false] {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.within("figure-sampling").click(("trigger", 0usize), cx);
            }).unwrap();
            cx.update(|cx| {
                assert_eq!(workspace.read(cx).inspector_sections.get("figure-sampling"), Some(&expected));
                assert_eq!(workspace.read(cx).session.trace.as_ref().unwrap().wavelength.coordinate, 650.0);
            });
        }
    }

    /// A 4 × 4 matrix whose axes are wide enough to tell a fitted window from
    /// the empty-state 0…1 box.
    fn dataset() -> TaDataset {
        let wavelength = Axis::new(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            "Wavelength",
            vec![500.0, 550.0, 600.0, 650.0],
        )
        .unwrap();
        let delay = Axis::new(
            AxisKind::Delay,
            AxisUnit::Picosecond,
            "Delay",
            vec![-1.0, 0.0, 1.0, 10.0],
        )
        .unwrap();
        let mut data = Array2::zeros((4, 4));
        for row in 0..4 {
            for column in 0..4 {
                data[[row, column]] = (row as f64 + 1.0) * 0.001 * (column as f64 + 1.0);
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

    fn workspace_with_dataset() -> Workspace {
        let mut workspace = Workspace::new(Session::new());
        workspace.session.open_dataset(dataset());
        workspace.touch_matrix();
        workspace.refresh_scenes();
        workspace
    }

    fn window_of(workspace: &Workspace, index: usize) -> ((f64, f64), (f64, f64)) {
        let state = workspace.plates[index].state.borrow();
        (state.x_range, state.y_range)
    }

    #[test]
    fn studio_zoom_and_pan_survive_control_refreshes() {
        let mut workspace = workspace_with_dataset();
        workspace.open_studio(PlateId::Kinetics);
        workspace.refresh_scenes();
        workspace.zoom_preview(1.2);
        workspace.refresh_scenes();
        assert!((workspace.studio_plates[0].state.borrow().image_zoom - 1.2).abs() < 1e-5);
        workspace.studio_plates[0].state.borrow_mut().image_offset = (15.0, -8.0);
        workspace.apply_style_preset("Presentation");
        workspace.refresh_scenes();
        let state = workspace.studio_plates[0].state.borrow();
        assert!((state.image_zoom - 1.2).abs() < 1e-5);
        assert_eq!(state.image_offset, (15.0, -8.0));
        drop(state);
        workspace.fit_preview();
        workspace.refresh_scenes();
        assert_eq!(workspace.studio_plates[0].state.borrow().image_zoom, 1.0);
        assert_eq!(workspace.studio_plates[0].state.borrow().image_offset, (0.0, 0.0));
    }

    #[test]
    fn studio_auto_ranges_and_cropped_exports_use_the_displayed_figure() {
        let mut workspace = workspace_with_dataset();
        workspace.open_studio(PlateId::Kinetics);
        workspace.edit_axis(true, |axis| axis.range = None);
        workspace.refresh_scenes();
        let range = workspace.studio_plates[0].scene.panel.x.range.unwrap();
        assert!(range.0 <= -1.0 && range.1 >= 10.0);
        let directory = std::env::temp_dir().join(format!("studio-controls-{}", ta_core::DatasetId::new()));
        std::fs::create_dir_all(&directory).unwrap();
        workspace.session.path = Some(directory.join("test.taproject"));
        workspace.session.studio.as_mut().unwrap().title = "controls".into();
        workspace.edit_axis(true, |axis| axis.range = Some((0.0, 1.0)));
        workspace.refresh_scenes();
        workspace.export_studio(ExportFormat::Svg);
        assert!(workspace.session.status.problem.is_none(), "{}", workspace.session.status.message);
        let svg = std::fs::read_to_string(directory.join("controls.svg")).unwrap();
        assert!(svg.contains("<svg"));
        workspace.export_studio(ExportFormat::Png);
        assert!(workspace.session.status.problem.is_none(), "{}", workspace.session.status.message);
        assert_eq!(&std::fs::read(directory.join("controls.png")).unwrap()[..8], b"\x89PNG\r\n\x1a\n");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn studio_visibility_and_layer_order_reach_the_preview() {
        let mut workspace = workspace_with_dataset();
        workspace.keep_current(true);
        workspace.step_linked(true, -1);
        workspace.open_studio(PlateId::Kinetics);
        workspace.refresh_scenes();
        let second = workspace.session.studio.as_ref().unwrap().panels[0].series[1].id;
        workspace.move_series(0, 1);
        workspace.refresh_scenes();
        assert_eq!(workspace.session.studio.as_ref().unwrap().panels[0].draw_order()[0].id, second);
        for series in &mut workspace.session.studio.as_mut().unwrap().panels[0].series {
            series.visible = false;
        }
        workspace.touch();
        workspace.refresh_scenes();
        assert!(workspace.studio_plates[0].scene.series.is_empty());
        assert!(workspace.studio_preview().unwrap().series.is_empty());
    }

    #[test]
    fn studio_style_marker_kind_and_axis_controls_rebuild_scenes_without_editing_data() {
        let mut workspace = workspace_with_dataset();
        let version = workspace.session.version.unwrap();
        let original = workspace.session.dataset().unwrap().matrix(version).unwrap().clone();
        workspace.open_studio(PlateId::Kinetics);
        let id = workspace.session.studio.as_ref().unwrap().panels[0].series[0].id;
        for (name, style) in crate::studio::style_presets() {
            workspace.apply_style_preset(name);
            workspace.refresh_scenes();
            let scene = &workspace.studio_plates[0].scene;
            assert_eq!(scene.style.font_family, style.font_family);
            assert_eq!(scene.panel.x.tick_label_size, style.tick_label_size);
            assert_eq!(scene.series[0].style.width, style.series_width);
        }
        for color in crate::studio::series_colors() {
            workspace.edit_series(id, |series| series.style.color = color);
            workspace.refresh_scenes();
            assert_eq!(workspace.studio_plates[0].scene.series[0].style.color, color);
        }
        for (_, dash) in crate::studio::dashes() {
            workspace.edit_series(id, |series| series.style.dash = dash);
            workspace.refresh_scenes();
            assert_eq!(workspace.studio_plates[0].scene.series[0].style.dash, dash);
        }
        for (_, marker) in crate::studio::markers() {
            workspace.set_series_marker(id, marker);
            workspace.refresh_scenes();
            assert_eq!(workspace.studio_plates[0].scene.series[0].style.marker, marker);
        }
        for (_, kind) in crate::studio::kinds() {
            workspace.set_series_kind(id, kind);
            workspace.refresh_scenes();
            assert_eq!(workspace.studio_plates[0].scene.series[0].kind, kind);
            if matches!(kind, ta_core::SeriesKind::Markers | ta_core::SeriesKind::Scatter | ta_core::SeriesKind::LineMarkers) {
                assert_ne!(workspace.studio_plates[0].scene.series[0].style.marker, ta_core::MarkerShape::None);
            }
        }
        workspace.edit_axis(false, |axis| { axis.grid = true; axis.reversed = true; });
        workspace.refresh_scenes();
        assert!(workspace.studio_plates[0].scene.panel.y.reversed);
        assert!(workspace.studio_plates[0].scene.style.grid);
        assert_eq!(&original, workspace.session.dataset().unwrap().matrix(version).unwrap());
    }

    #[gpui_kit::test]
    fn selected_line_controls_only_change_checked_curves(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{test::TestWindowExt, AppContext};
        cx.update(|cx| cx.set_reduce_motion(true));
        let (handle, workspace) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| {
                    let mut workspace = workspace_with_dataset();
                    workspace.keep_current(true);
                    workspace.session.select_wavelength(500.0).unwrap();
                    workspace.open_studio(PlateId::Kinetics);
                    workspace
                })
            }).unwrap()
        });
        let (id, before) = cx.update(|cx| {
            let series = &workspace.read(cx).session.studio.as_ref().unwrap().panel(0).unwrap().series;
            (series[0].id, series.clone())
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(ElementId::Name(format!("select-line-{id}").into()), cx);
        }).unwrap();
        cx.update_window(handle, |_, window, cx| {
            assert!(workspace.read(cx).studio_selection.contains(&id));
            window.click("selected-thicker", cx);
        }).unwrap();
        cx.update(|cx| {
            let series = &workspace.read(cx).session.studio.as_ref().unwrap().panel(0).unwrap().series;
            assert_eq!(series[0].style.width, before[0].style.width + 0.5);
            assert_eq!(series[1], before[1]);
        });
    }

    #[test]
    fn selected_line_edits_preserve_other_styles_and_unselected_curves() {
        let mut workspace = workspace_with_dataset();
        workspace.keep_current(true);
        workspace.session.select_wavelength(500.0).unwrap();
        workspace.keep_current(true);
        workspace.session.select_wavelength(550.0).unwrap();
        workspace.open_studio(PlateId::Kinetics);
        let panel = workspace.session.studio.as_mut().unwrap().panel_mut(0).unwrap();
        assert_eq!(panel.series.len(), 3);
        panel.series[0].style.width = 1.0;
        panel.series[1].style.width = 3.0;
        panel.series[1].style.dash = ta_core::DashPattern::Dotted;
        let before = panel.series.clone();
        workspace.studio_selection.extend([before[0].id, before[1].id]);
        workspace.edit_selected_series(|series| series.style.width += 0.5);
        let after = &workspace.session.studio.as_ref().unwrap().panel(0).unwrap().series;
        for index in 0..2 {
            let mut expected = before[index].clone();
            expected.style.width += 0.5;
            assert_eq!(after[index], expected);
        }
        assert_eq!(after[2], before[2]);
        let after = after.clone();
        workspace.studio_selection.clear();
        workspace.edit_selected_series(|series| series.style.dash = ta_core::DashPattern::Dashed);
        assert_eq!(workspace.session.studio.as_ref().unwrap().panel(0).unwrap().series, after);
    }

    #[test]
    fn studio_contour_controls_use_the_edited_spec_not_the_bench_spec() {
        let mut workspace = workspace_with_dataset();
        workspace.open_studio(PlateId::Contour);
        workspace.refresh_scenes();
        for name in ["coolwarm", "viridis", "magma", "grayscale"] {
            workspace.edit_contour(|spec| spec.colormap = ta_core::Colormap::preset(name).unwrap());
            workspace.refresh_scenes();
            let scene = &workspace.studio_plates[0].scene;
            assert_eq!(scene.heatmap.as_ref().unwrap().spec.colormap.name, name);
        }
        workspace.edit_contour(|spec| {
            spec.show_lines = false;
            spec.limits.min = -0.02;
            spec.limits.max = 0.02;
            spec.level_count = 6;
        });
        workspace.refresh_scenes();
        assert!(workspace.studio_plates[0].scene.series.is_empty());
        let spec = &workspace.studio_plates[0].scene.heatmap.as_ref().unwrap().spec;
        assert_eq!((spec.limits.min, spec.limits.max, spec.level_count), (-0.02, 0.02, 6));
    }

    #[test]
    fn marking_the_selection_uses_the_axis_of_the_opened_plate() {
        let mut workspace = workspace_with_dataset();
        for plate in [PlateId::Kinetics, PlateId::Spectra, PlateId::Contour] {
            workspace.open_studio(plate);
            workspace.mark_selection();
            workspace.refresh_scenes();
            let annotation = workspace.studio_plates[0].scene.panel.annotations.last().unwrap();
            match plate {
                PlateId::Spectra => assert_eq!(annotation.x, Some(650.0)),
                PlateId::Contour => { assert!(annotation.x.is_none()); assert!(annotation.y.is_some()); },
                _ => { assert!(annotation.x.is_some()); assert!(annotation.y.is_none()); }
            }
        }
    }

    #[test]
    fn axis_discovery_never_borrows_an_unrelated_experiments_axes() {
        let directory = std::env::temp_dir().join(format!("spectra-axes-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for name in ["NIR800_tawavelength", "NIR800_tatime", "sample_tawavelength", "sample_tatime"] {
            std::fs::write(directory.join(name), "0").unwrap();
        }
        assert!(stacked_axes(&directory.join("unrelated")).is_none());
        let (_, wavelength, delay) = stacked_axes(&directory.join("sample_tcorr")).unwrap();
        assert_eq!(wavelength, directory.join("sample_tawavelength"));
        assert_eq!(delay, directory.join("sample_tatime"));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn the_workspace_and_panel_choices_survive_reopening() {
        let mut workspace = workspace_with_dataset();
        workspace.layout = Layout::Compare;
        workspace.current_plate = PlateId::Spectra;
        workspace.show_index = false;
        workspace.show_inspector = false;
        workspace.record_workspace();
        let path = std::env::temp_dir().join(format!("spectra-workspace-{}.taproject", std::process::id()));
        workspace.session.save_project(&path).unwrap();
        let mut reopened = Workspace::new(Session::new());
        reopened.open_path(&path);
        assert_eq!(reopened.layout, Layout::Compare);
        assert_eq!(reopened.current_plate, PlateId::Spectra);
        assert!(!reopened.show_index);
        assert!(!reopened.show_inspector);
        assert!(reopened.session.status.problem.is_none());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn selecting_a_plot_is_not_overwritten_by_building_the_other_plots() {
        let mut workspace = workspace_with_dataset();
        workspace.apply_gesture(PlateId::Contour, PlotGesture::Select {
            data: (550.0, 1.0),
            extend: false,
        });
        workspace.refresh_scenes();
        assert_eq!(workspace.current_plate, PlateId::Contour);
    }

    #[test]
    fn the_arrow_keys_walk_the_two_axes_one_measured_sample_at_a_time() {
        let mut workspace = workspace_with_dataset();
        // The fixture's brightest row is the last one, so the walk starts at
        // 650 nm and the useful direction is back.
        let first = workspace.session.trace.as_ref().unwrap().wavelength.coordinate;
        assert_eq!(first, 650.0);
        workspace.step_linked(true, -1);
        let second = workspace.session.trace.as_ref().unwrap().wavelength.coordinate;
        assert_ne!(first, second, "a step moves the selection");
        // The axis is 500, 550, 600, 650: one step is one measured sample, not
        // one nanometre.
        assert_eq!(second, 600.0);
        // And the delay axis moves on its own: the trace does not follow.
        let trace_before = workspace.session.trace.as_ref().unwrap().wavelength.coordinate;
        workspace.step_linked(false, 1);
        assert_eq!(
            workspace.session.trace.as_ref().unwrap().wavelength.coordinate,
            trace_before
        );
        // Stepping past either end stops there rather than wrapping.
        for _ in 0..20 {
            workspace.step_linked(true, -1);
        }
        assert_eq!(
            workspace.session.trace.as_ref().unwrap().wavelength.coordinate,
            500.0
        );
        for _ in 0..20 {
            workspace.step_linked(true, 1);
        }
        assert_eq!(
            workspace.session.trace.as_ref().unwrap().wavelength.coordinate,
            650.0
        );
    }

    #[test]
    fn the_keep_keys_add_the_selection_and_say_so() {
        let mut workspace = workspace_with_dataset();
        assert!(workspace.session.comparison_wavelengths.is_empty());
        workspace.keep_current(true);
        assert_eq!(workspace.session.comparison_wavelengths.len(), 1);
        assert!(workspace.session.status.message.contains("kept on (b)"));
        // Keeping the same curve twice is harmless rather than an error: the
        // key is idempotent, so a second press cannot fill the plate with
        // copies of one trace.
        workspace.keep_current(true);
        assert_eq!(workspace.session.comparison_wavelengths.len(), 1);
        assert!(workspace.session.status.problem.is_none());
        // The delay axis keeps its own list. The default grid is already in it,
        // so the check is that keeping adds the current sample to that list
        // rather than replacing it.
        let before = workspace.session.comparison_delays.len();
        workspace.keep_current(false);
        let current = workspace
            .session
            .spectrum
            .as_ref()
            .unwrap()
            .delay
            .coordinate;
        assert!(workspace.session.comparison_delays.len() >= before);
        assert!(
            workspace
                .session
                .comparison_delays
                .iter()
                .any(|delay| (*delay - current).abs() < 1e-9),
            "the linked delay should be in the kept list"
        );
    }

    #[test]
    fn one_action_puts_every_view_back() {
        let mut workspace = workspace_with_dataset();
        // Pan a figure plate and a comparison pane off the matrix they show,
        // the way a drag would, without touching where "home" is.
        {
            let mut state = workspace.plates[0].state.borrow_mut();
            state.x_range = (520.0, 620.0);
            state.y_range = (-1.0, 5.0);
        }
        workspace.reset_all_views();
        assert_eq!(window_of(&workspace, 0), ((500.0, 650.0), (-1.0, 10.0)));
        for plate in &workspace.compare_plates {
            let state = plate.state.borrow();
            assert_eq!(state.x_range, state.home.0);
        }
        assert!(workspace.session.status.message.contains("full range"));
    }

    #[test]
    fn a_plate_opens_on_the_matrix_it_is_showing() {
        let workspace = Workspace::new(Session::new());
        // With nothing loaded every plate is a unit box, which is honest.
        assert_eq!(window_of(&workspace, 0), ((0.0, 1.0), (0.0, 1.0)));

        let workspace = workspace_with_dataset();
        // The contour opens on the wavelength and delay axes, not on 0…1.
        assert_eq!(window_of(&workspace, 0), ((500.0, 650.0), (-1.0, 10.0)));
        // And so do the linked plates.
        assert_eq!(window_of(&workspace, 1).0, (-1.0, 10.0));
        assert_eq!(window_of(&workspace, 2).0, (500.0, 650.0));
    }

    #[test]
    fn opening_a_dataset_rehomes_a_plate_that_already_had_a_window() {
        // This is the defect the first render showed: a shell built before the
        // dataset arrived kept its 0…1 window.
        let mut workspace = Workspace::new(Session::new());
        assert_eq!(window_of(&workspace, 0), ((0.0, 1.0), (0.0, 1.0)));
        workspace.session.open_dataset(dataset());
        workspace.touch_matrix();
        workspace.refresh_scenes();
        assert_eq!(window_of(&workspace, 0), ((500.0, 650.0), (-1.0, 10.0)));
    }

    #[test]
    fn a_selection_change_keeps_the_window_the_user_zoomed_to() {
        let mut workspace = workspace_with_dataset();
        workspace.plates[0]
            .state
            .borrow_mut()
            .set_home((520.0, 620.0), (-1.0, 5.0));
        // Choosing a wavelength is not a reason to throw away a window.
        workspace.session.select_wavelength(600.0).unwrap();
        workspace.touch();
        workspace.refresh_scenes();
        assert_eq!(window_of(&workspace, 0), ((520.0, 620.0), (-1.0, 5.0)));
        // And the cursors followed the selection.
        let cursors = &workspace.plates[0].scene.cursors;
        assert!(cursors
            .iter()
            .any(|cursor| cursor.value == 600.0));
    }

    #[test]
    fn the_contour_has_an_image_for_a_loaded_matrix_and_none_without_one() {
        let mut workspace = Workspace::new(Session::new());
        workspace.refresh_scenes();
        assert!(workspace.plates[0].scene.heatmap.is_none());

        let workspace = workspace_with_dataset();
        assert!(
            workspace.plates[0].scene.heatmap.is_some(),
            "the colour-mapped matrix should be there"
        );
    }

    #[test]
    fn the_contour_image_covers_the_window_the_plate_is_showing() {
        // The defect this catches: the image was built from the window the
        // plate had before the dataset arrived, so a two-pixel-wide smear was
        // stretched across the plot.
        let workspace = workspace_with_dataset();
        let source = workspace.plates[0]
            .scene
            .heatmap
            .as_ref()
            .expect("the contour has a matrix to colour-map");
        assert_eq!(source.matrix.dim(), (4, 4));
        assert_eq!(source.wavelength.len(), 4);
        assert_eq!(source.delay.len(), 4);
        // And no other plate carries one.
        assert!(workspace.plates[1].scene.heatmap.is_none());
    }

    #[test]
    fn every_plate_carries_its_caption_under_it() {
        let workspace = workspace_with_dataset();
        assert!(workspace.caption_of[0].contains("ΔA contour"));
        assert!(workspace.caption_of[1].contains("Kinetics at"));
        assert!(workspace.caption_of[2].contains("Spectra at"));
        assert!(workspace.caption_of[3].contains("No time-zero correction"));
    }

    #[test]
    fn switching_to_a_cropped_version_refits_the_window_to_the_new_axis() {
        let mut workspace = workspace_with_dataset();
        let version = {
            let dataset = workspace.session.dataset_mut().unwrap();
            let raw = dataset.raw_version().id;
            let axes = {
                let (wavelength, delay) = dataset.axes(raw).unwrap();
                (wavelength.clone(), delay.clone())
            };
            let data = dataset.matrix(raw).unwrap().clone();
            let cropped = ta_processing::crop_matrix(
                &data,
                &axes.0,
                &axes.1,
                &ta_core::CropSpec {
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
                    ta_core::ProcessingOp::Crop(ta_core::CropSpec {
                        wavelength: Some((500.0, 600.0)),
                        delay: None,
                    }),
                    vec![raw],
                )
                .unwrap()
        };
        workspace.session.set_version(version).unwrap();
        workspace.touch_matrix();
        workspace.refresh_scenes();
        assert_eq!(window_of(&workspace, 0).0, (500.0, 600.0));
        assert_eq!(workspace.session.chain().len(), 2);
    }
}
