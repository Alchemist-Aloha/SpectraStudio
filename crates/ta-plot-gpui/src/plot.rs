//! The interactive plot plate.
//!
//! One element owns three things: the frame it draws (a recorded command list
//! plus the layout that produced it), the interaction state (viewport,
//! crosshair, drag, selection), and the pointer handling that turns gestures
//! into changes. The pointer and the renderer read the *same* layout, so a
//! coordinate the crosshair reports is the coordinate that was drawn.

use crate::cache::{HeatmapSource, SharedHeatmapCache};
use crate::replay::{paint_buffer, ReplayTransform};
use gpui_kit::{
    fill, point, px, App, Bounds, DispatchPhase, Element, ElementId, GlobalElementId,
    Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollDelta, ScrollWheelEvent,
    Size, Style, Window,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use ta_core::{PanelModel, PublicationStyle, Rgba as InkColor};
use ta_plot::commands::CommandBuffer;
use ta_plot::render::{HeatmapLayer, PanelLayout, PlottedSeries};
use ta_plot::{record_panel, PlotViewport, Scale};

/// What the pointer did, handed to the owner so it can update its own state and
/// repaint. The element never mutates the project.
#[derive(Clone, Debug, PartialEq)]
pub enum PlotGesture {
    /// The window moved: keep the coordinate under the anchor pixel still.
    Zoom {
        anchor: (f64, f64),
        factors: (f64, f64),
    },
    Pan {
        dx: f64,
        dy: f64,
    },
    BoxZoom {
        from: (f64, f64),
        to: (f64, f64),
    },
    /// The pointer is over this physical coordinate, or has left the plate.
    Hover(Option<(f64, f64)>),
    /// The user chose a coordinate: the wavelength and delay a linked view
    /// needs.
    Select {
        data: (f64, f64),
        extend: bool,
    },
    /// A rectangular selection is being dragged, so the plate repaints.
    RegionPreview {
        from: (f64, f64),
        to: (f64, f64),
    },
    /// A rectangular selection finished, in physical coordinates.
    Region {
        from: (f64, f64),
        to: (f64, f64),
    },
    /// The nearest drawn point to the click, when one was within reach.
    Nearest {
        point: (f64, f64),
        series: usize,
        index: usize,
    },
}

/// Which drag the next press begins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interaction {
    /// Drag moves the window.
    Pan,
    /// Drag draws a zoom rectangle.
    BoxZoom,
    /// Drag draws a selection rectangle, for averaging or cropping.
    Region,
    /// A click selects; dragging does nothing.
    Cursor,
}

/// A drag in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DragState {
    Idle,
    Panning { last: (f64, f64) },
    Box { from: (f64, f64) },
}

/// Everything about one plate that survives between frames.
#[derive(Clone, Debug, PartialEq)]
pub struct PlotState {
    pub x_range: (f64, f64),
    pub y_range: (f64, f64),
    pub interaction: Interaction,
    /// The physical coordinate under the pointer, for the status bar.
    pub hover: Option<(f64, f64)>,
    /// A rectangle being dragged, in physical coordinates.
    pub region: Option<((f64, f64), (f64, f64))>,
    /// The last rectangle the user completed.
    pub selected_region: Option<((f64, f64), (f64, f64))>,
    pub drag: DragState,
    /// The initial window, so "reset view" has somewhere to go.
    pub home: ((f64, f64), (f64, f64)),
    /// Whether the coordinate grid is ruled.
    pub show_grid: bool,
    /// How much larger than "fit to the pane" a figure preview is drawn.
    ///
    /// This is the *page* zoom: it moves the whole sheet, its axes and its
    /// text, and leaves the data window exactly as the figure sets it. A
    /// working plate never changes it.
    pub image_zoom: f32,
    /// Where the zoomed page has been dragged to, in plate pixels.
    pub image_offset: (f32, f32),
    /// The pane this plate was last laid out in, so a zoom from a button can
    /// hold the pane's centre still the way the wheel holds the pointer.
    pub last_pane: Option<(f64, f64)>,
}

impl PlotState {
    pub fn new(x_range: (f64, f64), y_range: (f64, f64)) -> Self {
        Self {
            x_range,
            y_range,
            interaction: Interaction::Pan,
            hover: None,
            region: None,
            selected_region: None,
            drag: DragState::Idle,
            home: (x_range, y_range),
            show_grid: true,
            image_zoom: 1.0,
            image_offset: (0.0, 0.0),
            last_pane: None,
        }
    }

    /// Sets the window and remembers it as home.
    pub fn set_home(&mut self, x_range: (f64, f64), y_range: (f64, f64)) {
        self.x_range = x_range;
        self.y_range = y_range;
        self.home = (x_range, y_range);
    }

    pub fn reset_view(&mut self) {
        self.x_range = self.home.0;
        self.y_range = self.home.1;
        self.region = None;
        // A preview goes back to the whole page, which is what "reset" should
        // mean for a sheet of paper too.
        self.image_zoom = 1.0;
        self.image_offset = (0.0, 0.0);
    }

    /// Scales a figure preview about a point of the pane, so the paper under
    /// the pointer stays under it.
    ///
    /// `magnification` is what the page is multiplied by: more than one makes
    /// the sheet bigger, which is what a wheel notch up means everywhere else in
    /// the application.
    pub fn zoom_image_at(&mut self, anchor: (f64, f64), magnification: f32, pane: (f64, f64)) {
        if magnification <= 0.0 || !magnification.is_finite() {
            return;
        }
        let before = self.image_zoom;
        self.image_zoom = (self.image_zoom * magnification).clamp(0.2, 8.0);
        let applied = if before > 0.0 { self.image_zoom / before } else { 1.0 };
        // The page is drawn with its origin at `pane centre + offset`, at
        // `zoom` pixels per point, so the figure coordinate under a pane point
        // is `(point - centre - offset) / zoom`. Holding that still for the
        // anchor is what keeps the paper under the pointer.
        let centre = (pane.0 / 2.0, pane.1 / 2.0);
        let anchor = (anchor.0 - centre.0, anchor.1 - centre.1);
        self.image_offset = (
            self.image_offset.0 * applied + anchor.0 as f32 * (1.0 - applied),
            self.image_offset.1 * applied + anchor.1 as f32 * (1.0 - applied),
        );
    }

    /// Drags a figure preview, which moves the paper rather than the data.
    pub fn pan_image(&mut self, dx: f32, dy: f32) {
        self.image_offset = (self.image_offset.0 + dx, self.image_offset.1 + dy);
    }

    /// The page zoom a preview is showing, as a percentage of "fit".
    pub fn image_zoom_percent(&self) -> f32 {
        self.image_zoom * 100.0
    }

    fn viewport(&self, x_scale: Scale, y_scale: Scale, pixels: (f64, f64)) -> PlotViewport {
        PlotViewport::new(
            self.x_range,
            self.y_range,
            x_scale,
            y_scale,
            pixels,
        )
    }

    /// Zooms about a pixel inside the plot rectangle.
    pub fn zoom_at(
        &mut self,
        anchor_pixels: (f64, f64),
        factors: (f64, f64),
        x_scale: Scale,
        y_scale: Scale,
        pixels: (f64, f64),
    ) {
        let mut viewport = self.viewport(x_scale, y_scale, pixels);
        viewport.zoom_at(anchor_pixels, factors);
        self.x_range = viewport.x_range();
        self.y_range = viewport.y_range();
    }

    /// Moves the window by a pixel delta.
    pub fn pan_pixels(
        &mut self,
        dx: f64,
        dy: f64,
        x_scale: Scale,
        y_scale: Scale,
        pixels: (f64, f64),
    ) {
        let mut viewport = self.viewport(x_scale, y_scale, pixels);
        viewport.pan_pixels(dx, dy);
        self.x_range = viewport.x_range();
        self.y_range = viewport.y_range();
    }

    /// Zooms to a rectangle given in pixels inside the plot rectangle.
    pub fn box_zoom(
        &mut self,
        corner_a: (f64, f64),
        corner_b: (f64, f64),
        x_scale: Scale,
        y_scale: Scale,
        pixels: (f64, f64),
    ) -> bool {
        let mut viewport = self.viewport(x_scale, y_scale, pixels);
        let zoomed = viewport.box_zoom(corner_a, corner_b);
        if zoomed {
            self.x_range = viewport.x_range();
            self.y_range = viewport.y_range();
            self.region = None;
        }
        zoomed
    }

    /// The physical coordinate at a pixel inside the plot rectangle.
    pub fn data_at(
        &self,
        pixel: (f64, f64),
        x_scale: Scale,
        y_scale: Scale,
        pixels: (f64, f64),
    ) -> (f64, f64) {
        self.viewport(x_scale, y_scale, pixels)
            .pixel_to_data(pixel.0, pixel.1)
    }

    /// What a pointer move does: hover, pan, or drag a rectangle.
    ///
    /// This lives on the state rather than in the event handler so the handler
    /// takes exactly one borrow — which is what a `match` on a borrowed cell
    /// cannot do — and so the behaviour can be tested without a window.
    pub fn on_pointer_move(
        &mut self,
        local: (f64, f64),
        data: Option<(f64, f64)>,
        x_scale: Scale,
        y_scale: Scale,
        pixels: (f64, f64),
    ) -> Option<PlotGesture> {
        match self.drag {
            DragState::Idle => {
                if self.hover == data {
                    return None;
                }
                self.hover = data;
                Some(PlotGesture::Hover(data))
            }
            DragState::Panning { last } => {
                let (dx, dy) = (local.0 - last.0, local.1 - last.1);
                if dx == 0.0 && dy == 0.0 {
                    return None;
                }
                self.pan_pixels(dx, dy, x_scale, y_scale, pixels);
                self.drag = DragState::Panning { last: local };
                Some(PlotGesture::Pan { dx, dy })
            }
            DragState::Box { from } => {
                let region = self.region_between(from, local, x_scale, y_scale, pixels);
                if self.region == Some(region) {
                    return None;
                }
                self.region = Some(region);
                Some(PlotGesture::RegionPreview {
                    from: region.0,
                    to: region.1,
                })
            }
        }
    }

    /// What a press begins.
    pub fn on_press(
        &mut self,
        local: (f64, f64),
        data: Option<(f64, f64)>,
        extend: bool,
    ) -> Option<PlotGesture> {
        match self.interaction {
            Interaction::Pan => {
                self.drag = DragState::Panning { last: local };
                None
            }
            Interaction::BoxZoom | Interaction::Region => {
                self.drag = DragState::Box { from: local };
                self.region = None;
                None
            }
            Interaction::Cursor => data.map(|data| PlotGesture::Select { data, extend }),
        }
    }

    /// What a release finishes: a drag, a zoom, a region, or a click.
    pub fn on_release(
        &mut self,
        to: (f64, f64),
        data: Option<(f64, f64)>,
        extend: bool,
        x_scale: Scale,
        y_scale: Scale,
        pixels: (f64, f64),
    ) -> Option<PlotGesture> {
        let drag = std::mem::replace(&mut self.drag, DragState::Idle);
        let DragState::Box { from } = drag else {
            return None;
        };
        // A couple of pixels is a click, not a drag.
        let too_small = (to.0 - from.0).abs() < 3.0 || (to.1 - from.1).abs() < 3.0;
        if self.interaction == Interaction::BoxZoom && !too_small {
            if self.box_zoom(from, to, x_scale, y_scale, pixels) {
                return Some(PlotGesture::BoxZoom {
                    from: self.x_range,
                    to: self.y_range,
                });
            }
            return None;
        }
        let region = self.region_between(from, to, x_scale, y_scale, pixels);
        self.region = None;
        if too_small {
            return data.map(|data| PlotGesture::Select { data, extend });
        }
        self.selected_region = Some(region);
        Some(PlotGesture::Region {
            from: region.0,
            to: region.1,
        })
    }

    /// What the wheel does: zoom about the pointer, on one axis or both.
    pub fn on_wheel(
        &mut self,
        anchor: (f64, f64),
        steps: f64,
        x_only: bool,
        y_only: bool,
        x_scale: Scale,
        y_scale: Scale,
        pixels: (f64, f64),
    ) -> Option<PlotGesture> {
        if steps == 0.0 {
            return None;
        }
        // A notch is about a tenth of the window; scrolling up zooms in.
        let factor = 0.9f64.powf(steps);
        let factors = if x_only {
            (factor, 1.0)
        } else if y_only {
            (1.0, factor)
        } else {
            (factor, factor)
        };
        self.zoom_at(anchor, factors, x_scale, y_scale, pixels);
        Some(PlotGesture::Zoom { anchor, factors })
    }

    /// The physical coordinate behind two pixels, in either drag direction.
    pub fn region_between(
        &self,
        corner_a: (f64, f64),
        corner_b: (f64, f64),
        x_scale: Scale,
        y_scale: Scale,
        pixels: (f64, f64),
    ) -> ((f64, f64), (f64, f64)) {
        let a = self.data_at(corner_a, x_scale, y_scale, pixels);
        let b = self.data_at(corner_b, x_scale, y_scale, pixels);
        (
            (a.0.min(b.0), a.1.min(b.1)),
            (a.0.max(b.0), a.1.max(b.1)),
        )
    }
}

/// A persistent cursor line: the selected wavelength or delay, drawn over the
/// frame rather than into it, so moving a selection costs a repaint and not a
/// full re-render of the contours.
#[derive(Clone, Debug, PartialEq)]
pub struct PlotCursor {
    /// Which axis the line stands on.
    pub axis: ta_core::AxisKind,
    pub value: f64,
    pub label: String,
}

/// Everything a plate needs to draw itself.
#[derive(Clone)]
pub struct PanelScene {
    pub panel: PanelModel,
    pub style: PublicationStyle,
    pub series: Vec<PlottedSeries>,
    /// The matrix to colour-map, if this plate shows one. The image itself is
    /// built during prepaint, at the plot rectangle's size.
    pub heatmap: Option<Rc<HeatmapSource>>,
    pub background: InkColor,
    /// Selection cursors, in the accent colour.
    pub cursors: Vec<PlotCursor>,
    /// Whether a pointer crosshair with its coordinates in the margin is drawn.
    pub show_crosshair: bool,
    /// The colour the crosshair and cursors use.
    pub accent: InkColor,
    /// The physical size of the figure this plate is a preview of, in points.
    ///
    /// A working plate draws itself at one point per pixel and treats the
    /// type sizes as pixels, which is what a screen wants. A *preview* has to
    /// show the figure as it will be printed: the same layout, zoomed to fit
    /// the pane, so a change of type size or of figure size is visible as the
    /// change it will be on paper.
    pub figure: Option<(f64, f64)>,
    /// Whether the pointer can move the *data* window: pan, data zoom, region
    /// selection, crosshair. A figure preview does not — its limits are the
    /// figure's, and the editor sets them.
    pub interactive: bool,
    /// Whether the pointer can move the *page*: the wheel zooms the sheet and a
    /// drag slides it. This is what a figure preview is for.
    pub image_view: bool,
    /// The colour of the page a figure is drawn on, when the plate is previewing
    /// one. The element keeps the shell's ground, so the page and its margins
    /// are visible as the paper they are.
    pub page: Option<InkColor>,
}

/// One computed frame: the layout, the viewport it used, and the commands.
pub struct PlotFrame {
    pub layout: PanelLayout,
    pub viewport: PlotViewport,
    pub buffer: CommandBuffer,
    /// The figure size in points, which the recorded frame is in.
    pub points: (f64, f64),
    /// How many plate pixels one point of the figure takes.
    pub zoom: f32,
    /// Where the figure's own origin sits inside the plate, when the figure is
    /// letterboxed into a pane of a different shape.
    pub offset: (f32, f32),
    /// The page zoom and drag this frame was drawn at, so a wheel notch or a
    /// drag of the sheet recomputes it.
    image_zoom: f32,
    image_offset: (f32, f32),
    /// The plate's size in device pixels.
    pub size: (f64, f64),
    panel: PanelModel,
    series: Vec<PlottedSeries>,
    x_range: (f64, f64),
    y_range: (f64, f64),
    show_grid: bool,
    /// Which raster was drawn, when one was: the identity the export audit and
    /// the tests ask about.
    pub heatmap: Option<ta_plot::colormap::RasterKey>,
}

impl PlotFrame {
    /// How the recorded figure maps into the plate.
    /// How the recorded frame maps into the plate.
    ///
    /// The recording is already in device pixels — the canvas of a figure
    /// preview *is* the figure at its zoom — so the map is the offset and
    /// nothing else. Scaling here as well drew a preview at zoom squared, which
    /// put the page's margins and axis titles outside the pane: "fit to pane"
    /// then looked like a close-up of the axis region.
    pub fn transform(&self, bounds: Bounds<Pixels>) -> crate::replay::ReplayTransform {
        let plot = self.layout.plot;
        crate::replay::ReplayTransform::new(
            gpui_kit::point(
                bounds.origin.x + px(self.offset.0),
                bounds.origin.y + px(self.offset.1),
            ),
            1.0,
        )
        .with_plot((plot.x0, plot.y0, plot.x1, plot.y1))
    }

    /// Whether this frame still describes the given inputs.
    fn describes(
        &self,
        scene: &PanelScene,
        state: &PlotState,
        size: (f64, f64),
        show_grid: bool,
    ) -> bool {
        // Cursors are deliberately absent: they paint over the frame, so moving
        // a selection does not re-render the contours.
        self.size == size
            && self.x_range == state.x_range
            && self.y_range == state.y_range
            && self.show_grid == show_grid
            // The page itself moves too, so a wheel notch or a drag of the
            // sheet has to invalidate the frame it was drawn from.
            && self.image_zoom == state.image_zoom
            && self.image_offset == state.image_offset
            && self.panel == scene.panel
            && self.series == scene.series
    }

    /// The physical coordinate of a window position.
    fn data_at(&self, window_position: Point<Pixels>, origin: Point<Pixels>) -> Option<(f64, f64)> {
        let local = window_position - origin;
        // The pointer, the recorded layout and the offset are all in plate
        // pixels, so the only conversion is the offset.
        let x = (local.x.as_f32() - self.offset.0) as f64 - self.layout.plot.x0;
        let y = (local.y.as_f32() - self.offset.1) as f64 - self.layout.plot.y0;
        if !(0.0..=self.layout.plot.width()).contains(&x)
            || !(0.0..=(self.layout.plot.height())).contains(&y)
        {
            return None;
        }
        Some(self.viewport.pixel_to_data(x, y))
    }

    fn plot_pixels(&self) -> (f64, f64) {
        self.layout.plot_size()
    }
}

fn compute_frame(
    scene: &PanelScene,
    state: &PlotState,
    size: (f64, f64),
    show_grid: bool,
    heatmap_cache: &SharedHeatmapCache,
) -> PlotFrame {
    let x_scale = Scale::new(scene.panel.x.scale).unwrap_or(Scale::Linear);
    let y_scale = Scale::new(scene.panel.y.scale).unwrap_or(Scale::Linear);

    // A working plate is recorded at one point per device pixel, so the grid
    // lines land where the pointer thinks they do. A figure preview is
    // recorded at the figure's own size in points and then zoomed to fit, type
    // and all, which is what makes a type size mean something on screen.
    let (zoom, offset) = match scene.figure {
        Some((figure_width, figure_height))
            if figure_width > 1.0 && figure_height > 1.0 =>
        {
            // The fit scale shows the whole page; the page zoom on top of it is
            // the researcher's, and a zoomed page is allowed to run past the
            // pane — the pane clips it, and the offset slides it.
            let fit = (size.0 / figure_width).min(size.1 / figure_height);
            let zoom = (fit * state.image_zoom as f64).clamp(0.02, 40.0);
            let (drawn_width, drawn_height) = (figure_width * zoom, figure_height * zoom);
            (
                zoom as f32,
                (
                    ((size.0 - drawn_width) / 2.0) as f32 + state.image_offset.0,
                    ((size.1 - drawn_height) / 2.0) as f32 + state.image_offset.1,
                ),
            )
        }
        _ => (1.0f32, (0.0f32, 0.0f32)),
    };
    // A zoomed preview records the same figure with its type, its line widths
    // and its markers multiplied by the zoom, so the layout it computes is the
    // layout the printer will use, seen larger.
    let original = scene;
    let magnified: PanelScene;
    let scene: &PanelScene = if (zoom - 1.0).abs() > 1e-6 {
        // The panel carries the per-axis type sizes as well as the style, and
        // both have to be multiplied: scaling only the style left the axis
        // titles at their unzoomed size, tiny beside the tick labels that did
        // scale. The raster export already scales the panel for the same reason.
        magnified = PanelScene {
            panel: ta_plot::export::scaled_panel(&original.panel, zoom as f64),
            style: ta_plot::export::scaled_style(&original.style, zoom as f64),
            series: ta_plot::export::scaled_series(&original.series, zoom as f64),
            ..(*original).clone()
        };
        &magnified
    } else {
        original
    };
    let (width, height) = recorded_size(scene, size, zoom as f64);
    if std::env::var_os("SPECTRA_TRACE").is_some() {
    }
    let layout = PanelLayout::compute(
        &scene.panel,
        &scene.style,
        x_scale,
        y_scale,
        state.x_range,
        state.y_range,
        (width, height),
    );
    let mut layout = layout;
    // A crosshair or a cursor whose coordinate lands on a tick would write two
    // numbers on top of each other. The marker's number is the more precise one,
    // so the tick steps aside; the axis keeps every other tick it had.
    {
        let mut marks: Vec<(f64, f64)> = Vec::new();
        if scene.show_crosshair {
            if let Some((x, y)) = state.hover {
                marks.push((x, y));
            }
        }
        for cursor in &scene.cursors {
            // A cursor on the wavelength axis is a vertical line, so only the x
            // coordinate matters; a delay cursor is the other way round.
            match cursor.axis {
                ta_core::AxisKind::Wavelength => marks.push((cursor.value, state.y_range.0)),
                _ => marks.push((state.x_range.0, cursor.value)),
            }
        }
        for (x, y) in marks {
            if let Some(pixel) = layout
                .viewport(state.x_range, state.y_range, x_scale, y_scale)
                .data_to_pixel(x, y)
            {
                let tolerance = scene.style.tick_label_size.max(6.0) as f64 * 2.5;
                let x_pixels: Vec<f64> = layout
                    .x_ticks
                    .iter()
                    .filter_map(|tick| {
                        layout_viewport_x(&layout, state, x_scale, y_scale, *tick)
                    })
                    .collect();
                let keep_x: Vec<f64> = layout
                    .x_ticks
                    .iter()
                    .zip(x_pixels.iter())
                    .filter(|(_, px)| (**px - pixel.0).abs() > tolerance)
                    .map(|(tick, _)| *tick)
                    .collect();
                layout.x_ticks = keep_x;
                let y_pixels: Vec<f64> = layout
                    .y_ticks
                    .iter()
                    .filter_map(|tick| {
                        layout_viewport_y(&layout, state, x_scale, y_scale, *tick)
                    })
                    .collect();
                let keep_y: Vec<f64> = layout
                    .y_ticks
                    .iter()
                    .zip(y_pixels.iter())
                    .filter(|(_, px)| (**px - pixel.1).abs() > tolerance)
                    .map(|(tick, _)| *tick)
                    .collect();
                layout.y_ticks = keep_y;
            }
        }
    }
    if std::env::var_os("SPECTRA_TRACE").is_some() {
        eprintln!(
            "frame: pane {size:?} zoom {zoom:.4} offset {offset:?} page {width:.1}x{height:.1} plot {:?} colorbar {:?} available {:?}",
            (layout.plot.x0, layout.plot.y0, layout.plot.x1, layout.plot.y1),
            layout.colorbar.map(|bar| (bar.x0, bar.y0, bar.x1, bar.y1)),
            layout.available,
        );
    }
    let viewport = layout.viewport(state.x_range, state.y_range, x_scale, y_scale)
        .with_reversed(scene.panel.x.reversed, scene.panel.y.reversed);

    let mut panel = scene.panel.clone();
    panel.y.grid &= show_grid;
    panel.x.grid &= show_grid;

    // The image is built now, when the plot rectangle's size is known, and at
    // exactly that size: Plotters blits a bitmap one pixel per pixel.
    let (plot_width, plot_height) = layout.plot_size();
    let layer: Option<Arc<HeatmapLayer>> = scene.heatmap.as_ref().and_then(|source| {
        source.layer_with_scales(
            &mut heatmap_cache.borrow_mut(),
            (
                plot_width.round().max(2.0) as u32,
                plot_height.round().max(2.0) as u32,
            ),
            state.x_range,
            state.y_range,
            x_scale,
            y_scale,
        )
    });
    // The key the cache built under, kept so the frame can say which image it
    // drew without holding the pixels twice.
    let heatmap_key = if layer.is_some() {
        heatmap_cache.borrow().key().cloned()
    } else {
        None
    };

    let buffer = record_panel(
        &panel,
        &scene.style,
        &layout,
        &viewport,
        &scene.series,
        layer.as_deref(),
        (width.round() as u32, height.round() as u32),
    );

    if std::env::var_os("SPECTRA_TRACE").is_some() {
        for command in buffer.in_paint_order() {
            if let ta_plot::commands::DrawCommand::Text {
                text,
                position,
                size,
                rotation,
                ..
            } = command
            {
                eprintln!("  text {text:?} at {position:?} size {size} rotation {rotation}");
            }
        }
    }
    PlotFrame {
        layout,
        viewport,
        buffer,
        points: (width, height),
        zoom,
        offset,
        image_zoom: state.image_zoom,
        image_offset: state.image_offset,
        size,
        // The *unscaled* panel and series: a frame is stale when the scene it
        // was built from changed, not when the zoom changed the copy it drew.
        panel: original.panel.clone(),
        series: original.series.clone(),
        heatmap: heatmap_key,
        x_range: state.x_range,
        y_range: state.y_range,
        show_grid,
    }
}

/// Where a tick lands on the x axis, in plot pixels.
fn layout_viewport_x(
    layout: &ta_plot::PanelLayout,
    state: &PlotState,
    x_scale: Scale,
    y_scale: Scale,
    tick: f64,
) -> Option<f64> {
    layout
        .viewport(state.x_range, state.y_range, x_scale, y_scale)
        .data_to_pixel(tick, state.y_range.0)
        .map(|pixel| pixel.0)
}

/// Where a tick lands on the y axis, in plot pixels.
fn layout_viewport_y(
    layout: &ta_plot::PanelLayout,
    state: &PlotState,
    x_scale: Scale,
    y_scale: Scale,
    tick: f64,
) -> Option<f64> {
    layout
        .viewport(state.x_range, state.y_range, x_scale, y_scale)
        .data_to_pixel(state.x_range.0, tick)
        .map(|pixel| pixel.1)
}

/// The size the frame is recorded at: the figure's own size scaled by the zoom
/// when this plate previews a figure, and the plate's device size otherwise.
fn recorded_size(scene: &PanelScene, size: (f64, f64), zoom: f64) -> (f64, f64) {
    match scene.figure {
        Some((figure_width, figure_height))
            if figure_width > 1.0 && figure_height > 1.0 =>
        {
            ((figure_width * zoom).max(1.0), (figure_height * zoom).max(1.0))
        }
        _ => (size.0.max(1.0), size.1.max(1.0)),
    }
}

/// The plot element: a plate that draws a frame and reports gestures.
pub struct PlotElement {
    id: ElementId,
    scene: Rc<PanelScene>,
    state: Rc<RefCell<PlotState>>,
    frame: Rc<RefCell<Option<PlotFrame>>>,
    heatmap_cache: SharedHeatmapCache,
    hitbox: Rc<RefCell<Option<Hitbox>>>,
    on_gesture: Option<Rc<dyn Fn(PlotGesture, &mut Window, &mut App)>>,
}

impl PlotElement {
    pub fn new(
        id: impl Into<ElementId>,
        scene: Rc<PanelScene>,
        state: Rc<RefCell<PlotState>>,
        frame: Rc<RefCell<Option<PlotFrame>>>,
        heatmap_cache: SharedHeatmapCache,
    ) -> Self {
        Self {
            id: id.into(),
            scene,
            state,
            frame,
            heatmap_cache,
            hitbox: Rc::new(RefCell::new(None)),
            on_gesture: None,
        }
    }

    pub fn on_gesture(mut self, handler: impl Fn(PlotGesture, &mut Window, &mut App) + 'static) -> Self {
        self.on_gesture = Some(Rc::new(handler));
        self
    }

}

impl IntoElement for PlotElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for PlotElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let style = Style {
            size: Size::full(),
            ..Default::default()
        };
        (window.request_layout(style, None, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> () {
        let size = (
            bounds.size.width.as_f32() as f64,
            bounds.size.height.as_f32() as f64,
        );
        self.state.borrow_mut().last_pane = Some(size);
        let show_grid = self.state.borrow().show_grid;
        let stale = {
            let frame = self.frame.borrow();
            match frame.as_ref() {
                Some(frame) => !frame.describes(&self.scene, &self.state.borrow(), size, show_grid),
                None => true,
            }
        };
        if stale {
            let computed = compute_frame(
                &self.scene,
                &self.state.borrow(),
                size,
                show_grid,
                &self.heatmap_cache,
            );
            *self.frame.borrow_mut() = Some(computed);
        }
        *self.hitbox.borrow_mut() = Some(window.insert_hitbox(bounds, HitboxBehavior::Normal));
        let _ = cx;
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let background = self.scene.background;
        window.paint_quad(fill(bounds, to_hsla(background)));

        // Replay the frame this element laid out.
        {
            let frame = self.frame.borrow();
            if let Some(frame) = frame.as_ref() {
                let transform = frame.transform(bounds);
                // A preview sits on a page: painting it here means the sheet and
                // its margins are visible, which is what the figure will be.
                if let Some(page) = self.scene.page {
                    window.paint_quad(fill(
                        Bounds::new(
                            transform.map((0.0, 0.0)),
                            gpui_kit::size(
                                transform.length(frame.points.0 as f32),
                                transform.length(frame.points.1 as f32),
                            ),
                        ),
                        to_hsla(page),
                    ));
                }
                paint_buffer(window, cx, &frame.buffer, transform);
                // Then the things that move with the pointer: cursors, the
                // selection rectangle and the crosshair with the coordinates
                // written into the plate's margin.
                paint_overlay(
                    window,
                    cx,
                    bounds,
                    frame,
                    &self.scene,
                    &self.state.borrow(),
                    transform,
                );
            }
        }

        if self.scene.image_view {
            // A preview: the wheel and the drag move the paper.
            register_page_handlers(window, bounds, self.hitbox.clone(), self.state.clone());
        } else if self.scene.interactive {
            register_pointer_handlers(
                window,
                bounds,
                self.hitbox.clone(),
                self.frame.clone(),
                self.state.clone(),
                self.scene.clone(),
                self.on_gesture.clone(),
            );
        }
    }
}

/// Paints the selection cursors, the rectangle being dragged, and the pointer
/// crosshair with its coordinates in the margin.
fn paint_overlay(
    window: &mut Window,
    cx: &mut App,
    bounds: Bounds<Pixels>,
    frame: &PlotFrame,
    scene: &PanelScene,
    state: &PlotState,
    transform: ReplayTransform,
) {
    use gpui_kit::{hsla, PathBuilder};
    let plot = frame.layout.plot;
    let accent = scene.accent;
    let ink = to_hsla(accent);

    // A line helper. The figure's own coordinates are mapped through the same
    // transform the recorded frame used, so a cursor sits on the curve it was
    // placed on whatever the figure is zoomed to.
    let line = |window: &mut Window, width: f32, from: (f32, f32), to: (f32, f32)| {
        let mut builder = PathBuilder::stroke(transform.length(width));
        builder.move_to(transform.map((from.0 as f64, from.1 as f64)));
        builder.line_to(transform.map((to.0 as f64, to.1 as f64)));
        if let Ok(path) = builder.build() {
            window.paint_path(path, ink);
        }
    };

    // The cursors the session holds.
    for cursor in &scene.cursors {
        match cursor.axis {
            ta_core::AxisKind::Delay => {
                if let Some((_, y)) = frame.viewport.data_to_pixel(
                    frame.viewport.x_range().0,
                    cursor.value,
                ) {
                    let y = plot.y0 + y;
                    if y >= plot.y0 && y <= plot.y1 {
                        line(window, 1.0, (plot.x0 as f32, y as f32), (plot.x1 as f32, y as f32));
                        paint_margin_label(
                            window,
                            cx,
                            bounds,
                            (plot.x0 as f32 - 4.0, y as f32),
                            &cursor.label,
                            ink,
                            10.0,
                            true,
                            transform,
                        );
                    }
                }
            }
            _ => {
                if let Some((x, _)) = frame.viewport.data_to_pixel(
                    cursor.value,
                    frame.viewport.y_range().0,
                ) {
                    let x = plot.x0 + x;
                    if x >= plot.x0 && x <= plot.x1 {
                        line(window, 1.0, (x as f32, plot.y0 as f32), (x as f32, plot.y1 as f32));
                    }
                }
            }
        }
    }

    // The rectangle being dragged.
    if let Some((from, to)) = &state.region {
        if let (Some(a), Some(b)) = (
            frame.viewport.data_to_pixel(from.0, from.1),
            frame.viewport.data_to_pixel(to.0, to.1),
        ) {
            let a = (plot.x0 + a.0, plot.y0 + a.1);
            let b = (plot.x0 + b.0, plot.y0 + b.1);
            let corners = (
                (a.0.min(b.0) as f32, a.1.min(b.1) as f32),
                (a.0.max(b.0) as f32, a.1.max(b.1) as f32),
            );
            line(window, 1.0, (corners.0 .0, corners.0 .1), (corners.1 .0, corners.0 .1));
            line(window, 1.0, (corners.1 .0, corners.0 .1), (corners.1 .0, corners.1 .1));
            line(window, 1.0, (corners.1 .0, corners.1 .1), (corners.0 .0, corners.1 .1));
            line(window, 1.0, (corners.0 .0, corners.1 .1), (corners.0 .0, corners.0 .1));
        }
    }

    // The pointer: a hairline crosshair and its coordinates written into the
    // plate's margin, which is where a ruled plate keeps its numbers.
    if scene.show_crosshair {
        if let Some((x, y)) = state.hover {
            if let Some(pixel) = frame.viewport.data_to_pixel(x, y) {
                let px_x = plot.x0 + pixel.0;
                let px_y = plot.y0 + pixel.1;
                if frame.layout.plot.contains(px_x, px_y) {
                    let ghost = hsla(0.0, 0.0, 0.45, 0.35);
                    let mut builder = PathBuilder::stroke(transform.length(1.0));
                    builder.move_to(transform.map((px_x, plot.y0)));
                    builder.line_to(transform.map((px_x, plot.y1)));
                    if let Ok(path) = builder.build() {
                        window.paint_path(path, ghost);
                    }
                    let mut builder = PathBuilder::stroke(transform.length(1.0));
                    builder.move_to(transform.map((plot.x0, px_y)));
                    builder.line_to(transform.map((plot.x1, px_y)));
                    if let Ok(path) = builder.build() {
                        window.paint_path(path, ghost);
                    }
                    let x_label = format!("{:.4}", x);
                    let y_label = format!("{:.4}", y);
                    paint_margin_label(
                        window,
                        cx,
                        bounds,
                        (px_x as f32, plot.y1 as f32 + 3.0),
                        &x_label,
                        ink,
                        10.0,
                        false,
                        transform,
                    );
                    paint_margin_label(
                        window,
                        cx,
                        bounds,
                        (plot.x0 as f32 - 4.0, px_y as f32),
                        &y_label,
                        ink,
                        10.0,
                        true,
                        transform,
                    );
                }
            }
        }
    }
}

/// A short label placed in the plate's margin.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn paint_margin_label(
    window: &mut Window,
    cx: &mut App,
    _bounds: Bounds<Pixels>,
    at: (f32, f32),
    text: &str,
    color: gpui_kit::Hsla,
    font_size: f32,
    right_aligned: bool,
    transform: ReplayTransform,
) {
    use gpui_kit::{point, px, TextAlign};
    // A read-out is a screen affordance, not part of the figure, so it keeps
    // its own size while its place follows the figure.
    let size = px(font_size);
    let line = crate::replay::shape(window, text, size, color);
    let anchor = transform.map((at.0 as f64, at.1 as f64));
    let x = if right_aligned {
        anchor.x - line.width()
    } else {
        anchor.x - line.width() / 2.0
    };
    let _ = line.paint(point(x, anchor.y), size, TextAlign::Left, None, window, cx);
}

fn to_hsla(color: InkColor) -> gpui_kit::Hsla {
    let rgba = gpui_kit::Rgba {
        r: color.r as f32 / 255.0,
        g: color.g as f32 / 255.0,
        b: color.b as f32 / 255.0,
        a: color.a as f32 / 255.0,
    };
    let mut value: gpui_kit::Hsla = rgba.into();
    value.a = color.a as f32 / 255.0;
    value
}

/// Registers the two things a figure preview answers to: the wheel zooms the
/// page, and a drag slides it. The data window is not touched, so a preview
/// cannot be panned off the limits the figure sets.
fn register_page_handlers(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    hitbox: Rc<RefCell<Option<Hitbox>>>,
    state_handle: Rc<RefCell<PlotState>>,
) {
    {
        let hitbox = hitbox.clone();
        let state_handle = state_handle.clone();
        let pane = (
            bounds.size.width.as_f32() as f64,
            bounds.size.height.as_f32() as f64,
        );
        window.on_mouse_event(
            move |event: &ScrollWheelEvent, _: DispatchPhase, window: &mut Window, _: &mut App| {
                let hovered = hitbox
                    .borrow()
                    .as_ref()
                    .map(|hitbox| hitbox.is_hovered(window))
                    .unwrap_or(false);
                if !hovered {
                    return;
                }
                let steps = match event.delta {
                    ScrollDelta::Lines(delta) => delta.y as f64,
                    ScrollDelta::Pixels(delta) => delta.y.as_f32() as f64 / 40.0,
                };
                if steps == 0.0 {
                    return;
                }
                // A notch is about a twentieth; scrolling up makes the page
                // bigger, which is what scrolling up means everywhere else.
                let magnification = 1.05f64.powf(steps) as f32;
                let anchor = (
                    event.position.x.as_f32() as f64 - bounds.origin.x.as_f32() as f64,
                    event.position.y.as_f32() as f64 - bounds.origin.y.as_f32() as f64,
                );
                state_handle
                    .borrow_mut()
                    .zoom_image_at(anchor, magnification, pane);
                window.refresh();
            },
        );
    }
    {
        let hitbox = hitbox.clone();
        let state_handle = state_handle.clone();
        let dragging = Rc::new(std::cell::Cell::new(false));
        let last = Rc::new(std::cell::Cell::new((0.0f32, 0.0f32)));
        let dragging_down = dragging.clone();
        let dragging_move = dragging.clone();
        let last_down = last.clone();
        let last_move = last.clone();
        window.on_mouse_event(
            move |event: &MouseDownEvent, _: DispatchPhase, window: &mut Window, _: &mut App| {
                if event.button != MouseButton::Left {
                    return;
                }
                let hovered = hitbox
                    .borrow()
                    .as_ref()
                    .map(|hitbox| hitbox.is_hovered(window))
                    .unwrap_or(false);
                if !hovered {
                    return;
                }
                dragging_down.set(true);
                last_down.set((event.position.x.as_f32(), event.position.y.as_f32()));
            },
        );
        window.on_mouse_event(
            move |event: &MouseMoveEvent, _: DispatchPhase, _: &mut Window, _: &mut App| {
                if !dragging_move.get() || event.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                let (x, y) = (event.position.x.as_f32(), event.position.y.as_f32());
                let (was_x, was_y) = last_move.get();
                last_move.set((x, y));
                state_handle
                    .borrow_mut()
                    .pan_image(x - was_x, y - was_y);
            },
        );
        window.on_mouse_event(
            move |_: &MouseUpEvent, _: DispatchPhase, _: &mut Window, _: &mut App| {
                dragging.set(false);
            },
        );
    }
}

/// Registers hover, press, release and wheel handling against the layout that
/// was just computed.
///
/// Every handler follows the same shape: read what it needs out of the frame
/// (a different cell), take exactly one borrow of the state, ask the state what
/// the gesture means, release the borrow, then tell the owner. A `match` on a
/// borrowed cell would hold that borrow through every arm.
fn register_pointer_handlers(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    hitbox: Rc<RefCell<Option<Hitbox>>>,
    frame_handle: Rc<RefCell<Option<PlotFrame>>>,
    state_handle: Rc<RefCell<PlotState>>,
    scene: Rc<PanelScene>,
    on_gesture: Option<Rc<dyn Fn(PlotGesture, &mut Window, &mut App)>>,
) {
    let emit = move |gesture: PlotGesture, window: &mut Window, cx: &mut App| {
        if let Some(handler) = &on_gesture {
            handler(gesture, window, cx);
        }
    };
    let scales = {
        let scene = scene.clone();
        move || {
            (
                Scale::new(scene.panel.x.scale).unwrap_or(Scale::Linear),
                Scale::new(scene.panel.y.scale).unwrap_or(Scale::Linear),
            )
        }
    };

    // Hover, and the three drags.
    {
        let hitbox = hitbox.clone();
        let frame_handle = frame_handle.clone();
        let state_handle = state_handle.clone();
        let emit = emit.clone();
        let scales = scales.clone();
        window.on_mouse_event(
            move |event: &MouseMoveEvent, _: DispatchPhase, window: &mut Window, cx: &mut App| {
                let inside = hitbox
                    .borrow()
                    .as_ref()
                    .map(|hitbox| hitbox.is_hovered(window))
                    .unwrap_or(false);
                let (plot, pixels, data) = {
                    let frame = frame_handle.borrow();
                    let Some(frame) = frame.as_ref() else {
                        return;
                    };
                    (
                        frame.layout.plot,
                        frame.plot_pixels(),
                        if inside {
                            frame.data_at(event.position, bounds.origin)
                        } else {
                            None
                        },
                    )
                };
                let local = plot_local(event.position, bounds.origin, plot);
                let (x_scale, y_scale) = scales();
                let gesture = state_handle.borrow_mut().on_pointer_move(
                    local, data, x_scale, y_scale, pixels,
                );
                if let Some(gesture) = gesture {
                    emit(gesture, window, cx);
                }
            },
        );
    }

    // The press begins the drag the current tool implies.
    {
        let hitbox = hitbox.clone();
        let frame_handle = frame_handle.clone();
        let state_handle = state_handle.clone();
        let emit = emit.clone();
        window.on_mouse_event(
            move |event: &MouseDownEvent, _: DispatchPhase, window: &mut Window, cx: &mut App| {
                if event.button != MouseButton::Left {
                    return;
                }
                let hovered = hitbox
                    .borrow()
                    .as_ref()
                    .map(|hitbox| hitbox.is_hovered(window))
                    .unwrap_or(false);
                if !hovered {
                    return;
                }
                let (plot, data) = {
                    let frame = frame_handle.borrow();
                    let Some(frame) = frame.as_ref() else {
                        return;
                    };
                    (
                        frame.layout.plot,
                        frame.data_at(event.position, bounds.origin),
                    )
                };
                let local = plot_local(event.position, bounds.origin, plot);
                let gesture = state_handle
                    .borrow_mut()
                    .on_press(local, data, event.modifiers.shift);
                if let Some(gesture) = gesture {
                    emit(gesture, window, cx);
                }
            },
        );
    }

    // The release ends it, and reports what the user chose.
    {
        let frame_handle = frame_handle.clone();
        let state_handle = state_handle.clone();
        let emit = emit.clone();
        let scales = scales.clone();
        window.on_mouse_event(
            move |event: &MouseUpEvent, _: DispatchPhase, window: &mut Window, cx: &mut App| {
                if event.button != MouseButton::Left {
                    return;
                }
                let (plot, pixels, data) = {
                    let frame = frame_handle.borrow();
                    let Some(frame) = frame.as_ref() else {
                        return;
                    };
                    (
                        frame.layout.plot,
                        frame.plot_pixels(),
                        frame.data_at(bounds_clamp(event.position, bounds), bounds.origin),
                    )
                };
                let to = plot_local(event.position, bounds.origin, plot);
                let (x_scale, y_scale) = scales();
                let gesture = state_handle.borrow_mut().on_release(
                    to,
                    data,
                    event.modifiers.shift,
                    x_scale,
                    y_scale,
                    pixels,
                );
                if let Some(gesture) = gesture {
                    emit(gesture, window, cx);
                }
            },
        );
    }

    // The wheel zooms about the pointer.
    {
        let hitbox = hitbox.clone();
        let frame_handle = frame_handle.clone();
        let state_handle = state_handle.clone();
        let emit = emit.clone();
        let scales = scales.clone();
        window.on_mouse_event(
            move |event: &ScrollWheelEvent, _: DispatchPhase, window: &mut Window, cx: &mut App| {
                let hovered = hitbox
                    .borrow()
                    .as_ref()
                    .map(|hitbox| hitbox.is_hovered(window))
                    .unwrap_or(false);
                if !hovered {
                    return;
                }
                let (plot, pixels) = {
                    let frame = frame_handle.borrow();
                    let Some(frame) = frame.as_ref() else {
                        return;
                    };
                    (frame.layout.plot, frame.plot_pixels())
                };
                let anchor = plot_local(event.position, bounds.origin, plot);
                let steps = match event.delta {
                    ScrollDelta::Lines(delta) => delta.y as f64,
                    ScrollDelta::Pixels(delta) => delta.y.as_f32() as f64 / 40.0,
                };
                let gesture = state_handle.borrow_mut().on_wheel(
                    anchor,
                    steps,
                    event.modifiers.shift,
                    event.modifiers.alt || event.modifiers.control,
                    scales().0,
                    scales().1,
                    pixels,
                );
                if let Some(gesture) = gesture {
                    emit(gesture, window, cx);
                }
            },
        );
    }
}

/// A window position in the plot rectangle's own pixels.
fn plot_local(
    position: Point<Pixels>,
    origin: Point<Pixels>,
    plot: ta_plot::render::Rect,
) -> (f64, f64) {
    (
        position.x.as_f32() as f64 - origin.x.as_f32() as f64 - plot.x0,
        position.y.as_f32() as f64 - origin.y.as_f32() as f64 - plot.y0,
    )
}

fn bounds_clamp(position: Point<Pixels>, bounds: Bounds<Pixels>) -> Point<Pixels> {
    let x = position.x.as_f32().clamp(
        bounds.origin.x.as_f32(),
        bounds.origin.x.as_f32() + bounds.size.width.as_f32(),
    );
    let y = position.y.as_f32().clamp(
        bounds.origin.y.as_f32(),
        bounds.origin.y.as_f32() + bounds.size.height.as_f32(),
    );
    point(px(x), px(y))
}

/// A plate ready to be added to a view: the scene, its state and its frame
/// cache, kept together so a view can hand the same three to every frame.
pub struct Plate {
    pub scene: Rc<PanelScene>,
    pub state: Rc<RefCell<PlotState>>,
    pub frame: Rc<RefCell<Option<PlotFrame>>>,
    pub heatmap_cache: SharedHeatmapCache,
}

impl Plate {
    pub fn new(scene: PanelScene, state: PlotState, heatmap_cache: SharedHeatmapCache) -> Self {
        Self {
            scene: Rc::new(scene),
            state: Rc::new(RefCell::new(state)),
            frame: Rc::new(RefCell::new(None)),
            heatmap_cache,
        }
    }

    /// Replaces the drawing inputs, keeping the interaction state.
    pub fn set_scene(&mut self, scene: PanelScene) {
        self.scene = Rc::new(scene);
    }

    pub fn replace(&mut self, scene: PanelScene) {
        self.set_scene(scene);
        *self.frame.borrow_mut() = None;
    }

    pub fn element(&self, id: impl Into<ElementId>) -> PlotElement {
        PlotElement::new(
            id,
            self.scene.clone(),
            self.state.clone(),
            self.frame.clone(),
            self.heatmap_cache.clone(),
        )
    }

    pub fn hover(&self) -> Option<(f64, f64)> {
        self.state.borrow().hover
    }

    pub fn drag(&self) -> DragState {
        self.state.borrow().drag
    }

    pub fn region(&self) -> Option<((f64, f64), (f64, f64))> {
        self.state.borrow().region
    }

    pub fn is_zoomed(&self) -> bool {
        let state = self.state.borrow();
        state.x_range != state.home.0 || state.y_range != state.home.1
    }

    pub fn interaction(&self) -> Interaction {
        self.state.borrow().interaction
    }

    pub fn set_interaction(&self, interaction: Interaction) {
        self.state.borrow_mut().interaction = interaction;
    }

    pub fn reset_view(&self) {
        self.state.borrow_mut().reset_view();
    }

    pub fn command_count(&self) -> usize {
        self.frame
            .borrow()
            .as_ref()
            .map(|frame| frame.buffer.len())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::HeatmapCache;
    use ta_core::SeriesStyle;

    /// The sizes the recorded text was drawn at.
    fn text_sizes(buffer: &ta_plot::CommandBuffer) -> Vec<f64> {
        buffer
            .in_paint_order()
            .into_iter()
            .filter_map(|command| match command {
                ta_plot::commands::DrawCommand::Text { size, .. } => Some(*size),
                _ => None,
            })
            .collect()
    }

    /// A scene of one line, with the type sizes and figure size a test sets.
    fn line_scene(style: PublicationStyle, figure: Option<(f64, f64)>) -> (PanelScene, PlotState) {
        let mut panel = PanelModel::new("");
        panel.x = ta_core::AxisModel {
            label: "Delay".into(),
            unit: Some("ps".into()),
            range: Some((-1.0, 10.0)),
            ..ta_core::AxisModel::default()
        };
        panel.y = ta_core::AxisModel {
            label: "ΔOD".into(),
            range: Some((-0.1, 0.1)),
            ..ta_core::AxisModel::default()
        };
        let points: Vec<(f64, f64)> = (0..40).map(|i| (-1.0 + i as f64 * 0.3, 0.05)).collect();
        let scene = PanelScene {
            panel,
            style,
            series: vec![PlottedSeries::new("a", points, SeriesStyle::default())],
            heatmap: None,
            background: InkColor::rgb(255, 255, 255),
            cursors: Vec::new(),
            show_crosshair: false,
            accent: InkColor::rgb(176, 42, 55),
            page: figure.map(|_| InkColor::rgb(255, 255, 255)),
            figure,
            interactive: true,
            image_view: false,
        };
        let state = PlotState::new((-1.0, 10.0), (-0.1, 0.1));
        (scene, state)
    }

    #[test]
    fn a_figure_preview_draws_its_type_at_the_figures_own_size() {
        // The defect this catches: the studio drew the working screen's
        // typography at one point per pixel, so changing the figure's type size
        // changed nothing on screen and the preview was not the figure.
        let style = PublicationStyle {
            base_font_size: 8.0,
            axis_title_size: 8.0,
            tick_label_size: 8.0,
            legend_font_size: 8.0,
            ..PublicationStyle::default()
        };
        let (scene, state) = line_scene(style.clone(), Some((234.0, 180.0)));
        let cache: SharedHeatmapCache = Rc::new(RefCell::new(HeatmapCache::new()));
        let frame = compute_frame(&scene, &state, (702.0, 540.0), true, &cache);
        // 702 / 234 = 3, and 540 / 180 = 3: the whole figure fits at that zoom.
        assert!((frame.zoom - 3.0).abs() < 1e-6, "zoom was {}", frame.zoom);
        assert_eq!(frame.offset, (0.0, 0.0));
        assert!((frame.points.0 - 702.0).abs() < 1e-6);
        // The recorded type is the figure's type, three times over — the tick
        // labels, the axis titles and the legend alike. A title left unscaled
        // is a title that reads as a footnote beside its own axis.
        let sizes = text_sizes(&frame.buffer);
        assert!(
            sizes.iter().any(|size| (size - 8.0 * 3.0).abs() < 1e-6),
            "no text was drawn at 8 pt of a 3× figure: {sizes:?}"
        );
        let axis_titles: Vec<f64> = frame
            .buffer
            .in_paint_order()
            .into_iter()
            .filter_map(|command| match command {
                ta_plot::commands::DrawCommand::Text { text, size, .. }
                    if text.contains("Delay") || text.contains("ΔOD") =>
                {
                    Some(*size)
                }
                _ => None,
            })
            .collect();
        assert!(!axis_titles.is_empty(), "the axis titles are drawn");
        for size in &axis_titles {
            assert!(
                (size - 8.0 * 3.0).abs() < 1e-6,
                "an axis title was drawn at {size} where the figure's own type is 24"
            );
        }
        assert!(
            frame
                .buffer
                .texts()
                .iter()
                .any(|text| text.contains("ΔOD") || text.contains("Delay")),
            "the axis labels are drawn"
        );
    }

    #[test]
    fn the_type_on_screen_is_the_configured_size_at_the_panes_zoom() {
        // The claim the studio makes: the number in the panel is the number of
        // points the drawn text is, in the figure's own coordinates, whatever
        // size that figure is and however big the pane is.
        let cache: SharedHeatmapCache = Rc::new(RefCell::new(HeatmapCache::new()));
        let style = |size: f32| PublicationStyle {
            axis_title_size: size,
            tick_label_size: size,
            base_font_size: size,
            legend_font_size: size,
            ..PublicationStyle::default()
        };
        let mut drawn_ratios = Vec::new();
        for (figure, configured, pane) in [
            ((234.0, 180.0), 8.0f32, (702.0, 540.0)),
            ((504.0, 216.0), 8.0f32, (702.0, 540.0)),
            ((234.0, 180.0), 12.0f32, (400.0, 900.0)),
            ((504.0, 216.0), 12.0f32, (1000.0, 1000.0)),
        ] {
            let (scene, state) = line_scene(style(configured), Some(figure));
            let frame = compute_frame(&scene, &state, pane, true, &cache);
            let biggest = text_sizes(&frame.buffer)
                .into_iter()
                .fold(0.0f64, f64::max);
            // Ignore the panel title, which is drawn at the base size.
            let ratio = biggest / frame.zoom as f64;
            drawn_ratios.push(ratio);
            assert!(
                (ratio - configured as f64).abs() < 1e-6,
                "a {configured} pt figure was drawn at {ratio} pt at zoom {}",
                frame.zoom
            );
        }
        assert_eq!(drawn_ratios.len(), 4);
    }

    #[test]
    fn a_bigger_figure_gets_less_zoom_in_the_same_pane() {
        // Two figures in one pane: the wider one is shown smaller, so the same
        // pane can preview 3.25-inch and 7-inch figures honestly.
        let cache: SharedHeatmapCache = Rc::new(RefCell::new(HeatmapCache::new()));
        let (scene, state) = line_scene(PublicationStyle::default(), Some((234.0, 180.0)));
        let small = compute_frame(&scene, &state, (702.0, 540.0), true, &cache);
        let (scene, state) = line_scene(PublicationStyle::default(), Some((504.0, 216.0)));
        let large = compute_frame(&scene, &state, (702.0, 540.0), true, &cache);
        assert!(large.zoom < small.zoom);
        // And the drawn figure fills the pane in the dimension that binds.
        assert!((504.0 * large.zoom as f64 - 702.0).abs() < 1.0);
    }

    #[test]
    fn a_preview_is_letterboxed_rather_than_stretched() {
        // A 3.25 × 2.5 in figure in a square pane: the figure keeps its shape
        // and is centred, so what is on screen has the proportions of the paper.
        let (scene, state) = line_scene(PublicationStyle::default(), Some((234.0, 180.0)));
        let cache: SharedHeatmapCache = Rc::new(RefCell::new(HeatmapCache::new()));
        let frame = compute_frame(&scene, &state, (600.0, 600.0), true, &cache);
        assert!((frame.zoom - 600.0 / 234.0).abs() < 1e-6);
        let drawn_height = 180.0 * frame.zoom as f64;
        assert!(
            (frame.offset.1 as f64 - (600.0 - drawn_height) / 2.0).abs() < 1.0,
            "the figure is centred vertically"
        );
        assert_eq!(frame.offset.0, 0.0);
    }

    #[test]
    fn a_fitted_page_lands_inside_the_pane_it_is_fitted_to() {
        // The defect this catches: the recording is already in device pixels,
        // and the replay scaled it by the zoom a second time, so a fitted page
        // was drawn zoom times too large and its margins fell outside the pane.
        // "Fit to pane" has to mean the whole sheet is inside the pane.
        let cache: SharedHeatmapCache = Rc::new(RefCell::new(HeatmapCache::new()));
        let figure = (234.0, 180.0);
        let (scene, state) = line_scene(PublicationStyle::default(), Some(figure));
        for pane in [(371.0, 256.0), (702.0, 540.0), (903.0, 903.0), (200.0, 700.0)] {
            let frame = compute_frame(&scene, &state, pane, true, &cache);
            let bounds = Bounds::new(
                gpui_kit::point(px(0.0), px(0.0)),
                gpui_kit::size(px(pane.0 as f32), px(pane.1 as f32)),
            );
            // The drawn page is the recorded canvas, one for one.
            let transform = frame.transform(bounds);
            assert_eq!(
                transform.scale, 1.0,
                "the recorded frame is in device pixels and is not scaled again"
            );
            let drawn_width = transform.length(frame.points.0 as f32);
            let drawn_height = transform.length(frame.points.1 as f32);
            assert!(
                drawn_width.as_f32() <= pane.0 as f32 + 1.0
                    && drawn_height.as_f32() <= pane.1 as f32 + 1.0,
                "a fitted page of {drawn_width:?} × {drawn_height:?} does not fit a pane of {pane:?}"
            );
            // And it is inside the pane, not hanging off it.
            let corner = transform.map((frame.points.0, frame.points.1));
            assert!(corner.x.as_f32() <= pane.0 as f32 + 1.0);
            assert!(corner.y.as_f32() <= pane.1 as f32 + 1.0);
            // One of the two dimensions is filled: a fit uses the room it has.
            let fills_width = (drawn_width.as_f32() - pane.0 as f32).abs() < 1.0;
            let fills_height = (drawn_height.as_f32() - pane.1 as f32).abs() < 1.0;
            assert!(fills_width || fills_height, "a fit fills one dimension");
        }
    }

    #[test]
    fn a_preview_layout_is_the_exports_layout_magnified() {
        // §17.11: the editor preview and the exported file share their layout
        // definitions. The way to keep that true is to check it: the layout the
        // preview computes at a pane of `figure × zoom` must be the layout the
        // export computes at `figure`, scaled by the zoom.
        let cache: SharedHeatmapCache = Rc::new(RefCell::new(HeatmapCache::new()));
        let style = PublicationStyle {
            base_font_size: 8.0,
            axis_title_size: 8.0,
            tick_label_size: 8.0,
            legend_font_size: 8.0,
            ..PublicationStyle::default()
        };
        let figure = (234.0, 180.0);
        let (scene, state) = line_scene(style.clone(), Some(figure));
        let zoom = 3.0;
        let preview = compute_frame(
            &scene,
            &state,
            (figure.0 * zoom, figure.1 * zoom),
            true,
            &cache,
        );
        // The same panel and style at the figure's own size, which is what
        // `ta_plot::export` lays out before it writes a file.
        let x_scale = Scale::Linear;
        let y_scale = Scale::Linear;
        let exported = PanelLayout::compute(
            &scene.panel,
            &style,
            x_scale,
            y_scale,
            state.x_range,
            state.y_range,
            figure,
        );
        let plot = preview.layout.plot;
        let expected = exported.plot;
        // Every margin is expressed in the type size, so the layout is linear
        // and this is exact to rounding: the preview *is* the exported figure,
        // drawn larger.
        let tolerance = 1.0;
        for (actual, reference, edge) in [
            (plot.x0, expected.x0, "left"),
            (plot.y0, expected.y0, "top"),
            (plot.x1, expected.x1, "right"),
            (plot.y1, expected.y1, "bottom"),
        ] {
            assert!(
                (actual - reference * zoom).abs() < tolerance,
                "the {edge} edge is at {actual}, and the export puts it at {}",
                reference * zoom
            );
        }
        // The plot's own size agrees to the same order, so the figure's
        // composition is what the export will have.
        assert!(
            ((plot.x1 - plot.x0) - (expected.x1 - expected.x0) * zoom).abs() < tolerance,
            "the plot is {} wide where the export's is {}",
            plot.x1 - plot.x0,
            (expected.x1 - expected.x0) * zoom
        );
        assert!((preview.zoom as f64 - zoom).abs() < 1e-6);
    }

    #[test]
    fn the_page_zooms_without_moving_the_data_window() {
        // The complaint this answers: the wheel over a figure preview changed
        // the region of interest — the axis limits — instead of the size of the
        // sheet. A preview zooms the page and leaves the limits alone.
        let cache: SharedHeatmapCache = Rc::new(RefCell::new(HeatmapCache::new()));
        let (scene, mut state) = line_scene(PublicationStyle::default(), Some((234.0, 180.0)));
        let pane = (702.0, 540.0);
        let fitted = compute_frame(&scene, &state, pane, true, &cache);
        assert!((fitted.zoom - 3.0).abs() < 1e-6);

        // One wheel notch up, with the pointer at a quarter of the pane.
        let anchor = (pane.0 * 0.25, pane.1 * 0.5);
        state.zoom_image_at(anchor, 1.2, pane);
        assert!(state.image_zoom > 1.0, "the page grew");
        assert_eq!(state.x_range, (-1.0, 10.0), "the data window did not move");
        assert_eq!(state.y_range, (-0.1, 0.1));

        let zoomed = compute_frame(&scene, &state, pane, true, &cache);
        assert!(
            zoomed.zoom > fitted.zoom,
            "the page is drawn larger: {} against {}",
            zoomed.zoom,
            fitted.zoom
        );
        // The layout is the same figure: the page grew, not the plot's share of
        // the window.
        assert!(
            ((zoomed.layout.plot.x1 - zoomed.layout.plot.x0)
                - (fitted.layout.plot.x1 - fitted.layout.plot.x0) * 1.0)
                .abs()
                < 220.0
        );
        // The point of the *paper* under the pointer stayed put: the pane
        // point maps back to the same figure coordinate before and after.
        let figure_at = |frame: &PlotFrame, pane_point: (f64, f64)| {
            (
                (pane_point.0 - frame.offset.0 as f64) / frame.zoom as f64,
                (pane_point.1 - frame.offset.1 as f64) / frame.zoom as f64,
            )
        };
        let before = figure_at(&fitted, anchor);
        let after = figure_at(&zoomed, anchor);
        assert!(
            (before.0 - after.0).abs() < 0.05,
            "the paper moved sideways under the pointer: {before:?} against {after:?}"
        );
        assert!(
            (before.1 - after.1).abs() < 0.05,
            "the paper moved vertically under the pointer: {before:?} against {after:?}"
        );
    }

    #[test]
    fn fitting_the_page_puts_it_back_and_resetting_does_too() {
        let (scene, mut state) = line_scene(PublicationStyle::default(), Some((234.0, 180.0)));
        let cache: SharedHeatmapCache = Rc::new(RefCell::new(HeatmapCache::new()));
        let pane = (600.0, 400.0);
        let fitted = compute_frame(&scene, &state, pane, true, &cache);
        state.zoom_image_at((300.0, 200.0), 2.0, pane);
        state.pan_image(30.0, -20.0);
        assert!(state.image_zoom > 1.0);
        assert_ne!(state.image_offset, (0.0, 0.0));

        // "Fit" is the page at 100%, which is what the frame draws at the fit
        // scale.
        state.image_zoom = 1.0;
        state.image_offset = (0.0, 0.0);
        let again = compute_frame(&scene, &state, pane, true, &cache);
        assert_eq!(again.zoom, fitted.zoom);
        assert_eq!(again.offset, fitted.offset);

        // And a full reset does the same, because the page is part of the view.
        state.zoom_image_at((300.0, 200.0), 2.0, pane);
        state.reset_view();
        assert_eq!(state.image_zoom, 1.0);
        assert_eq!(state.image_offset, (0.0, 0.0));
        assert!(state.image_zoom_percent() == 100.0);
    }

    #[test]
    fn a_working_plate_is_unchanged_by_any_of_this() {
        let (scene, state) = line_scene(PublicationStyle::default(), None);
        let cache: SharedHeatmapCache = Rc::new(RefCell::new(HeatmapCache::new()));
        let frame = compute_frame(&scene, &state, (400.0, 300.0), true, &cache);
        assert_eq!(frame.zoom, 1.0);
        assert_eq!(frame.offset, (0.0, 0.0));
        assert_eq!(frame.points, (400.0, 300.0));
    }

    fn state() -> PlotState {
        PlotState::new((-10.0, 10.0), (-0.5, 0.5))
    }

    #[test]
    fn a_wheel_zoom_keeps_the_coordinate_under_the_pointer() {
        let mut state = state();
        let pixels = (400.0, 200.0);
        let anchor = (120.0, 60.0);
        let before = state.data_at(anchor, Scale::Linear, Scale::Linear, pixels);
        state.zoom_at(anchor, (0.5, 0.5), Scale::Linear, Scale::Linear, pixels);
        let after = state.data_at(anchor, Scale::Linear, Scale::Linear, pixels);
        assert!((before.0 - after.0).abs() < 1e-9);
        assert!((before.1 - after.1).abs() < 1e-9);
        assert!(state.x_range.1 - state.x_range.0 < 20.0);
    }

    #[test]
    fn one_axis_can_be_zoomed_on_its_own() {
        let mut state = state();
        let pixels = (400.0, 200.0);
        state.zoom_at((200.0, 100.0), (0.5, 1.0), Scale::Linear, Scale::Linear, pixels);
        assert!(state.x_range.1 - state.x_range.0 < 20.0);
        assert_eq!(state.y_range, (-0.5, 0.5));
    }

    #[test]
    fn panning_moves_the_window_against_the_drag() {
        let mut state = state();
        let pixels = (400.0, 200.0);
        let before = state.x_range;
        // Dragging the data to the left moves the window to the right.
        state.pan_pixels(-40.0, 0.0, Scale::Linear, Scale::Linear, pixels);
        assert!(
            state.x_range.0 > before.0,
            "the window should have moved right"
        );
        // The width is untouched by a pan.
        assert!(
            ((state.x_range.1 - state.x_range.0) - (before.1 - before.0)).abs() < 1e-9
        );
    }

    #[test]
    fn a_box_zoom_shows_the_dragged_rectangle() {
        let mut state = state();
        let pixels = (400.0, 200.0);
        let expected_low = state.data_at((100.0, 150.0), Scale::Linear, Scale::Linear, pixels);
        let expected_high = state.data_at((300.0, 50.0), Scale::Linear, Scale::Linear, pixels);
        assert!(state.box_zoom((100.0, 50.0), (300.0, 150.0), Scale::Linear, Scale::Linear, pixels));
        assert!((state.x_range.0 - expected_low.0).abs() < 1e-9);
        assert!((state.x_range.1 - expected_high.0).abs() < 1e-9);
        assert!((state.y_range.0 - expected_low.1).abs() < 1e-9);
        assert!((state.y_range.1 - expected_high.1).abs() < 1e-9);
    }

    #[test]
    fn a_click_sized_box_does_not_zoom() {
        let mut state = state();
        let pixels = (400.0, 200.0);
        assert!(!state.box_zoom((100.0, 50.0), (100.5, 50.5), Scale::Linear, Scale::Linear, pixels));
        assert_eq!(state.x_range, (-10.0, 10.0));
    }

    #[test]
    fn reset_view_returns_to_where_the_plate_started() {
        let mut state = state();
        let pixels = (400.0, 200.0);
        state.zoom_at((200.0, 100.0), (0.2, 0.2), Scale::Linear, Scale::Linear, pixels);
        assert!(state.x_range != (-10.0, 10.0));
        state.reset_view();
        assert_eq!(state.x_range, (-10.0, 10.0));
        assert_eq!(state.y_range, (-0.5, 0.5));
    }

    #[test]
    fn a_region_is_reported_in_physical_coordinates_whichever_way_it_was_dragged() {
        let state = state();
        let pixels = (400.0, 200.0);
        let forward = state.region_between((50.0, 20.0), (250.0, 180.0), Scale::Linear, Scale::Linear, pixels);
        let backward = state.region_between((250.0, 180.0), (50.0, 20.0), Scale::Linear, Scale::Linear, pixels);
        assert_eq!(forward, backward);
        assert!(forward.0 .0 < forward.1 .0);
        assert!(forward.0 .1 < forward.1 .1);
    }

    #[test]
    fn the_frame_builds_its_image_at_the_plot_rectangles_size() {
        // The defect this catches: an image built at the sample count was
        // blitted at its own pixel size, so the colour field covered only part
        // of the plate and the rest showed blank paper.
        use crate::cache::{HeatmapCache, HeatmapSource};
        use ndarray::Array2 as Grid;
        use ta_core::{Axis, AxisKind, AxisUnit, AutoRange, ColorLimits, Colormap, ContourSpec};

        let wavelength = Axis::new(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            "w",
            (0..40).map(|i| 500.0 + i as f64).collect(),
        )
        .unwrap();
        let delay = Axis::new(
            AxisKind::Delay,
            AxisUnit::Picosecond,
            "t",
            (0..20).map(|i| i as f64).collect(),
        )
        .unwrap();
        let matrix = Grid::from_elem((40, 20), 0.2);

        let mut panel = PanelModel::new("");
        panel.x = ta_core::AxisModel {
            label: "Wavelength".into(),
            unit: Some("nm".into()),
            range: Some((500.0, 539.0)),
            ..ta_core::AxisModel::default()
        };
        panel.y = ta_core::AxisModel {
            label: "Delay".into(),
            unit: Some("ps".into()),
            range: Some((0.0, 19.0)),
            ..ta_core::AxisModel::default()
        };
        panel.contour = Some(ContourSpec {
            limits: ColorLimits {
                min: -1.0,
                max: 1.0,
                symmetric_about_zero: true,
                auto: AutoRange::Manual,
            },
            colormap: Colormap::preset("coolwarm").unwrap(),
            ..ContourSpec::default()
        });

        let scene = PanelScene {
            panel,
            style: ta_core::PublicationStyle::default(),
            series: Vec::new(),
            heatmap: Some(Rc::new(HeatmapSource {
                wavelength,
                delay,
                matrix: Arc::new(matrix),
                spec: ContourSpec::default(),
                sampling: ta_plot::RasterSampling::Nearest,
                background: InkColor::rgb(255, 255, 255),
                source: None,
            })),
            background: InkColor::rgb(255, 255, 255),
            cursors: Vec::new(),
            show_crosshair: false,
            accent: InkColor::rgb(0, 0, 0),
            figure: None,
            interactive: true,
            image_view: false,
            page: None,
        };
        let state = PlotState::new((500.0, 539.0), (0.0, 19.0));
        let cache: SharedHeatmapCache = Rc::new(RefCell::new(HeatmapCache::new()));
        let frame = compute_frame(&scene, &state, (400.0, 300.0), true, &cache);

        let (plot_width, plot_height) = frame.layout.plot_size();
        let bitmap = frame
            .buffer
            .commands
            .iter()
            .find_map(|command| match command {
                ta_plot::DrawCommand::Bitmap { size, .. } => Some(*size),
                _ => None,
            })
            .expect("the frame carries a colour-mapped image");
        assert_eq!(
            (bitmap.0 as f64, bitmap.1 as f64),
            (
                plot_width.round().max(2.0),
                plot_height.round().max(2.0)
            ),
            "the image is {bitmap:?} but the plot rectangle is {plot_width} × {plot_height}"
        );
        // And it is placed at the plot rectangle's corner.
        let position = frame
            .buffer
            .commands
            .iter()
            .find_map(|command| match command {
                ta_plot::DrawCommand::Bitmap { position, .. } => Some(*position),
                _ => None,
            })
            .unwrap();
        assert_eq!(position.0, frame.layout.plot.x0.round() as i32);
        assert_eq!(position.1, frame.layout.plot.y0.round() as i32);
        assert!(frame.heatmap.is_some(), "the frame remembers which raster it drew");
    }

    #[test]
    fn a_drag_pans_and_a_release_ends_it_without_touching_the_cell_twice() {
        // The handler used to hold a borrow of the state while an arm took a
        // second one, which panicked on the first mouse move. The state machine
        // is what the handler now calls, so this is the path that crashed.
        let mut state = state();
        let pixels = (400.0, 200.0);
        let (x_scale, y_scale) = (Scale::Linear, Scale::Linear);

        assert!(state.on_press((200.0, 100.0), Some((0.0, 0.0)), false).is_none());
        let before = state.x_range;

        let moved = state.on_pointer_move(
            (240.0, 100.0),
            Some((2.0, 0.0)),
            x_scale,
            y_scale,
            pixels,
        );
        assert!(matches!(moved, Some(PlotGesture::Pan { .. })));
        // Dragging the data right moves the window left.
        assert!(state.x_range.0 < before.0);

        assert!(state
            .on_release((240.0, 100.0), Some((2.0, 0.0)), false, x_scale, y_scale, pixels)
            .is_none());
        assert_eq!(state.drag, DragState::Idle);

        // A hover after the drag reports the new coordinate and cannot panic.
        let hover = state.on_pointer_move(
            (300.0, 120.0),
            Some((5.0, 0.1)),
            x_scale,
            y_scale,
            pixels,
        );
        assert_eq!(hover, Some(PlotGesture::Hover(Some((5.0, 0.1)))));
        // Repeating the same position reports nothing, so no repaint is asked
        // for on every idle frame.
        assert_eq!(
            state.on_pointer_move((301.0, 121.0), Some((5.0, 0.1)), x_scale, y_scale, pixels),
            None
        );
    }

    #[test]
    fn a_box_zoom_drag_reports_the_new_window_and_a_click_selects_instead() {
        let mut zooming = state();
        zooming.interaction = Interaction::BoxZoom;
        let pixels = (400.0, 200.0);
        let (x_scale, y_scale) = (Scale::Linear, Scale::Linear);

        zooming.on_press((100.0, 40.0), Some((-5.0, 0.3)), false);
        let preview = zooming.on_pointer_move(
            (300.0, 160.0),
            Some((5.0, -0.3)),
            x_scale,
            y_scale,
            pixels,
        );
        assert!(matches!(preview, Some(PlotGesture::RegionPreview { .. })));
        assert!(zooming.region.is_some(), "the drag paints a rectangle");

        let finished = zooming.on_release(
            (300.0, 160.0),
            Some((5.0, -0.3)),
            false,
            x_scale,
            y_scale,
            pixels,
        );
        match finished {
            Some(PlotGesture::BoxZoom { from, to }) => {
                assert!(from.0 < from.1 && to.0 < to.1);
            }
            other => panic!("expected a box zoom, got {other:?}"),
        }
        assert!(zooming.region.is_none(), "the rectangle is gone after the zoom");
        assert!(zooming.x_range.1 - zooming.x_range.0 < 20.0);

        // A press and release in the same place selects rather than zooming.
        let mut clicking = state();
        clicking.interaction = Interaction::BoxZoom;
        let start = (200.0, 100.0);
        clicking.on_press(start, Some((1.0, 0.0)), true);
        let gesture =
            clicking.on_release(start, Some((1.0, 0.0)), true, x_scale, y_scale, pixels);
        assert_eq!(
            gesture,
            Some(PlotGesture::Select {
                data: (1.0, 0.0),
                extend: true
            })
        );
        assert_eq!(clicking.x_range, (-10.0, 10.0), "a click must not zoom");
    }

    #[test]
    fn a_region_drag_is_kept_for_the_owner_as_the_selection() {
        let mut state = state();
        state.interaction = Interaction::Region;
        let pixels = (400.0, 200.0);
        let (x_scale, y_scale) = (Scale::Linear, Scale::Linear);
        state.on_press((120.0, 60.0), None, false);
        state.on_pointer_move((280.0, 140.0), None, x_scale, y_scale, pixels);
        let gesture =
            state.on_release((280.0, 140.0), None, false, x_scale, y_scale, pixels);
        match gesture {
            Some(PlotGesture::Region { from, to }) => {
                assert!(from.0 < to.0 && from.1 < to.1);
            }
            other => panic!("expected a region, got {other:?}"),
        }
        assert!(state.selected_region.is_some());
    }

    #[test]
    fn the_wheel_zooms_on_the_axes_the_modifier_asks_for() {
        let mut horizontal = state();
        let pixels = (400.0, 200.0);
        let (x_scale, y_scale) = (Scale::Linear, Scale::Linear);

        // Shift: the delay axis alone.
        let gesture =
            horizontal.on_wheel((200.0, 100.0), 1.0, true, false, x_scale, y_scale, pixels);
        assert!(matches!(gesture, Some(PlotGesture::Zoom { .. })));
        assert!(horizontal.x_range.1 - horizontal.x_range.0 < 20.0);
        assert_eq!(horizontal.y_range, (-0.5, 0.5));

        // Alt or control: the absorbance axis alone.
        let mut vertical = state();
        vertical.on_wheel((200.0, 100.0), 1.0, false, true, x_scale, y_scale, pixels);
        assert_eq!(vertical.x_range, (-10.0, 10.0));
        assert!(vertical.y_range.1 - vertical.y_range.0 < 1.0);

        // No movement is not a gesture.
        assert_eq!(
            vertical.on_wheel((200.0, 100.0), 0.0, false, false, x_scale, y_scale, pixels),
            None
        );
    }

    #[test]
    fn a_logarithmic_delay_plate_reports_real_coordinates() {
        let mut state = PlotState::new((-100.0, 1000.0), (-0.1, 0.1));
        state.set_home((-100.0, 1000.0), (-0.1, 0.1));
        let scale = Scale::SymmetricLogarithmic { linear_width: 1.0 };
        let pixels = (500.0, 300.0);
        // The middle of a symlog window is not the middle of its numbers: the
        // centre pixel sits just above the 1 ps linear region, not at 450 ps.
        let middle = state.data_at((250.0, 150.0), scale, Scale::Linear, pixels);
        assert!(
            (1.0..2.0).contains(&middle.0),
            "the centre of a symlog window is {} ps",
            middle.0
        );
        assert!((middle.0 - 450.0).abs() > 100.0);
        state.zoom_at((250.0, 150.0), (0.5, 0.5), scale, Scale::Linear, pixels);
        assert!(state.x_range.0 < 1000.0 && state.x_range.1 > -100.0);
    }
}
