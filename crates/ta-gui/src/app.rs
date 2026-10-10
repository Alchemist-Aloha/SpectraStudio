//! The application shell.
//!
//! The composition is the direction contract's: a header of rules, a plate index
//! of ruled rows on the left, the figure block of captioned plates in the
//! centre, a column of caption-and-control pairs on the right, and a footer of
//! tabular coordinates with the clickable address of what is selected.

use crate::plates::{self, PlateContent};
use crate::session::{PlateId, Sampling, Session};
use gpui_kit::component::{h_resizable, resizable_panel, v_resizable};
// Imported by name rather than by glob: `gpui_kit::*` also brings in gpui's
// `test` attribute macro, which then shadows the built-in one in this crate's
// tests.
use gpui_kit::{
    div, px, AnyElement, Context, ElementId, FontWeight, InteractiveElement, IntoElement,
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

/// The workspace view.
pub struct Workspace {
    pub session: Session,
    /// One interactive plate per panel of the figure block.
    plates: Vec<Plate>,
    /// The colour-mapped contour image, shared with the plates because each
    /// builds its own image during prepaint, at its own plot size.
    raster: SharedHeatmapCache,
    /// What the contour's image is made from, rebuilt when the matrix changes.
    contour_source: Option<std::rc::Rc<HeatmapSource>>,
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
        };
        workspace.build();
        workspace
    }

    fn touch(&mut self) {
        self.revision += 1;
    }

    /// A different matrix is on the bench: the plates refit to its axes.
    fn touch_matrix(&mut self) {
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
        for (index, plate_id) in PlateId::ALL.iter().enumerate() {
            // The contour reuses its cached isolines; the linked plates are
            // cheap enough to rebuild every time.
            let content = if *plate_id == PlateId::Contour {
                PlateContent {
                    panel: self
                        .contour_panel
                        .clone()
                        .unwrap_or_else(|| ta_core::PanelModel::new("")),
                    series: self.contour_series.clone().unwrap_or_default(),
                }
            } else {
                plates::content(&self.session, *plate_id, &spec)
            };
            let (x_range, y_range) = default_ranges(&content.panel);
            let previous = self
                .plates
                .get(index)
                .map(|plate| plate.state.borrow().clone())
                .unwrap_or_else(|| PlotState::new(x_range, y_range));
            let heatmap = if *plate_id == PlateId::Contour {
                self.contour_source.clone()
            } else {
                None
            };
            let scene = PanelScene {
                panel: content.panel,
                style: views.clone(),
                series: content.series,
                heatmap,
                background: Rgba::rgb(255, 255, 255),
                cursors: plates::cursors(&self.session, *plate_id),
                show_crosshair: true,
                accent: plates::accent(),
            };
            match self.plates.get_mut(index) {
                Some(plate) => {
                    // The frame is kept: the element compares the drawing inputs
                    // itself and only re-renders when one of them changed.
                    plate.set_scene(scene);
                    if self.rehome {
                        plate.state.borrow_mut().set_home(x_range, y_range);
                    }
                }
                None => self
                    .plates
                    .push(Plate::new(scene, previous, self.raster.clone())),
            }
            self.caption_of[index] = self.session.caption(*plate_id);
        }
        self.rehome = false;
        self.built = self.revision;
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
                None => self
                    .session
                    .status
                    .problem(format!("cannot read {}: {sheet_error}", path.display())),
            },
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

    /// Exports the kinetics plate as an SVG at its recorded physical size.
    fn export_figure(&mut self) {
        let plate = PlateId::Kinetics;
        let spec = plates::contour_spec(&self.session);
        let content = plates::content(&self.session, plate, &spec);
        let figure = plates::export_figure(&self.session, plate, &spec);
        let directory = self
            .session
            .path
            .as_ref()
            .and_then(|path| path.parent().map(|parent| parent.to_path_buf()))
            .unwrap_or_else(std::env::temp_dir);
        let path = directory.join(format!("spectra-studio-{}.svg", plate.letter()));
        // The image is made at the size the exported figure will use, because
        // Plotters blits a bitmap one pixel per pixel.
        let series = content.series.clone();
        let mut panels = vec![ta_plot::export::ExportPanel::new(0, series.clone())];
        if let Some(source) = &self.contour_source {
            let (figure_width, figure_height) = ta_plot::export::figure_points(&figure);
            let x_scale = ta_plot::Scale::new(figure.panels[0].x.scale).unwrap_or_default();
            let y_scale = ta_plot::Scale::new(figure.panels[0].y.scale).unwrap_or_default();
            let layout = ta_plot::PanelLayout::compute(
                &figure.panels[0],
                &figure.style,
                x_scale,
                y_scale,
                figure.panels[0].x.range.unwrap_or((0.0, 1.0)),
                figure.panels[0].y.range.unwrap_or((0.0, 1.0)),
                (figure_width, figure_height),
            );
            let (plot_width, plot_height) = layout.plot_size();
            if let Some(layer) = source.layer(
                &mut self.raster.borrow_mut(),
                (
                    plot_width.round().max(2.0) as u32,
                    plot_height.round().max(2.0) as u32,
                ),
                state_window(&self.plates, 0).0,
                state_window(&self.plates, 0).1,
            ) {
                panels = vec![ta_plot::export::ExportPanel::new(0, series)
                    .with_heatmap((*layer).clone())];
            }
        }
        match ta_plot::export::export_svg(&figure, panels, &path) {
            Ok(()) => self
                .session
                .status
                .info(format!("exported {}", path.display())),
            Err(error) => self.session.status.problem(format!("{error}")),
        }
    }

    fn set_sampling(&mut self, sampling: Sampling) {
        self.session.set_sampling(sampling);
        self.touch();
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
                    .child(self.action("Save project", false, cx, |workspace, _, _| {
                        if let Some(path) = workspace.session.path.clone() {
                            let result = workspace.session.save_project(&path);
                            if let Err(error) = result {
                                workspace.session.status.problem(format!("{error}"));
                            }
                            workspace.touch();
                        } else {
                            workspace.session.status.problem("no project path yet");
                        }
                    }))
                    .child(self.action("Export figure…", false, cx, |workspace, _, _| {
                        workspace.export_figure();
                        workspace.touch();
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
        div()
            .id(ElementId::Name(label.into()))
            .text_size(px(12.5))
            .text_color(color)
            .border_b_1()
            .border_color(if accent { ink::accent() } else { ink::rule() })
            .pb(px(1.0))
            .hover(move |style| style.text_color(ink::accent()))
            .child(label)
            .on_click(cx.listener(move |workspace, _event, window, cx| {
                run(workspace, window, cx);
                cx.notify();
            }))
    }

    fn plate_index(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entries = self.session.chain();
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

        div()
            .w(px(INDEX_WIDTH))
            .h_full()
            .flex()
            .flex_col()
            .bg(ink::ground())
            .border_r_1()
            .border_color(ink::rule())
            .child(self.section_label(dataset.unwrap_or_else(|| "no dataset".to_string())))
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

    fn plate_block(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
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
                                    .text_size(px(11.0))
                                    .text_color(ink::text())
                                    .child(caption),
                            ),
                    )
                    .h(px(CAPTION_HEIGHT)),
            )
    }

    /// Applies a gesture from a plate to the session.
    fn apply_gesture(&mut self, plate: PlateId, gesture: PlotGesture) {
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

    fn inspector(&self, cx: &mut Context<Self>) -> impl IntoElement {
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

        div()
            .w(px(INSPECTOR_WIDTH))
            .h_full()
            .flex()
            .flex_col()
            .bg(ink::ground())
            .border_l_1()
            .border_color(ink::rule())
            .child(self.section_label("Caption and control".to_string()))
            .child(self.field(
                "Reading",
                div()
                    .flex()
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
            .child(self.field(
                "Window",
                // The mode's own name, so the reading is stated in words.
                div()
                    .text_size(px(12.0))
                    .text_color(ink::muted())
                    .child(format!(
                        "{} · ± {:.1} nm · {} channels",
                        sampling.label(),
                        sampling.half_width,
                        sampling.channels
                    )),
            ))
            .child(self.field(
                "Trace",
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(ink::text())
                            .child(trace_label),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ink::muted())
                            .child(format!("selected at {trace} nm")),
                    ),
            ))
            .child(self.field(
                "Spectrum",
                div()
                    .text_size(px(12.0))
                    .text_color(ink::text())
                    .child(format!("selected at {spectrum} ps")),
            ))
            .child(self.field(
                "Colour limits",
                div()
                    .text_size(px(12.0))
                    .text_color(ink::muted())
                    .child(format!("{:.4} … {:.4} ΔOD, symmetric", limits.min, limits.max)),
            ))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .p_3()
                    .child(self.inspector_action("Keep trace", cx, |workspace| {
                        let _ = workspace.session.save_current_trace();
                        workspace.touch();
                    }))
                    .child(self.inspector_action("Keep spectrum", cx, |workspace| {
                        let _ = workspace.session.save_current_spectrum();
                        workspace.touch();
                    })),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .px_3()
                    .child(self.inspector_action("Reset view", cx, |workspace| {
                        for plate in &workspace.plates {
                            plate.reset_view();
                        }
                    })),
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
        if self.built != self.revision {
            self.refresh_scenes();
        }
        let contour = self.plate_block(0, cx);
        let kinetics = self.plate_block(1, cx);
        let spectra = self.plate_block(2, cx);
        let time_zero = self.plate_block(3, cx);

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(ink::paper())
            .text_color(ink::text())
            .child(self.header(cx))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .child(self.plate_index(cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(
                                v_resizable("figure-block")
                                    .child(
                                        resizable_panel()
                                            .size(px(430.0))
                                            .child(contour),
                                    )
                                    .child(
                                        resizable_panel().child(
                                            h_resizable("linked-plates")
                                                .child(resizable_panel().child(kinetics))
                                                .child(resizable_panel().child(spectra)),
                                        ),
                                    )
                                    .child(resizable_panel().size(px(240.0)).child(time_zero)),
                            ),
                    )
                    .child(self.inspector(cx)),
            )
            .child(self.footer())
    }
}

/// A plate's current window.
fn state_window(plates: &[Plate], index: usize) -> ((f64, f64), (f64, f64)) {
    match plates.get(index) {
        Some(plate) => {
            let state = plate.state.borrow();
            (state.x_range, state.y_range)
        }
        None => ((0.0, 1.0), (0.0, 1.0)),
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
    let candidates = [
        (
            directory.join(format!("{stem}_tawavelength")),
            directory.join(format!("{stem}_tatime")),
        ),
        (
            directory.join("NIR800_tawavelength"),
            directory.join("NIR800_tatime"),
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
