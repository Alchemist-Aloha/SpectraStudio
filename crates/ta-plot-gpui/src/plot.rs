//! The interactive plot plate.
//!
//! One element owns three things: the frame it draws (a recorded command list
//! plus the layout that produced it), the interaction state (viewport,
//! crosshair, drag, selection), and the pointer handling that turns gestures
//! into changes. The pointer and the renderer read the *same* layout, so a
//! coordinate the crosshair reports is the coordinate that was drawn.

use crate::cache::HeatmapData;
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
use ta_plot::render::{PanelLayout, PlottedSeries};
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
pub struct PanelScene {
    pub panel: PanelModel,
    pub style: PublicationStyle,
    pub series: Vec<PlottedSeries>,
    pub heatmap: Option<Arc<HeatmapData>>,
    pub background: InkColor,
    /// Selection cursors, in the accent colour.
    pub cursors: Vec<PlotCursor>,
    /// Whether a pointer crosshair with its coordinates in the margin is drawn.
    pub show_crosshair: bool,
    /// The colour the crosshair and cursors use.
    pub accent: InkColor,
}

/// One computed frame: the layout, the viewport it used, and the commands.
pub struct PlotFrame {
    pub layout: PanelLayout,
    pub viewport: PlotViewport,
    pub buffer: CommandBuffer,
    /// The figure size in points, which the recorded frame is in.
    pub points: (f64, f64),
    /// The plate's size in device pixels.
    pub size: (f64, f64),
    panel: PanelModel,
    series: Vec<PlottedSeries>,
    heatmap: Option<usize>,
    x_range: (f64, f64),
    y_range: (f64, f64),
    show_grid: bool,
}

impl PlotFrame {
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
            && self.panel == scene.panel
            && self.series == scene.series
            && self.heatmap
                == scene
                    .heatmap
                    .as_ref()
                    .map(|data| Arc::as_ptr(data) as usize)
    }

    /// The physical coordinate of a window position.
    fn data_at(&self, window_position: Point<Pixels>, origin: Point<Pixels>) -> Option<(f64, f64)> {
        let local = window_position - origin;
        let x = local.x.as_f32() as f64 - self.layout.plot.x0;
        let y = local.y.as_f32() as f64 - self.layout.plot.y0;
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
) -> PlotFrame {
    let x_scale = Scale::new(scene.panel.x.scale).unwrap_or(Scale::Linear);
    let y_scale = Scale::new(scene.panel.y.scale).unwrap_or(Scale::Linear);

    // The frame is recorded in figure points, and the plate is drawn at the
    // same density: one point, one device pixel, so the grid lines land where
    // the pointer thinks they do.
    let (width, height) = figure_points(scene, x_scale, y_scale, state, size);
    let layout = PanelLayout::compute(
        &scene.panel,
        &scene.style,
        x_scale,
        y_scale,
        state.x_range,
        state.y_range,
        (width, height),
    );
    let viewport = layout.viewport(state.x_range, state.y_range, x_scale, y_scale);

    let mut panel = scene.panel.clone();
    panel.y.grid = show_grid;
    panel.x.grid = show_grid;

    let buffer = record_panel(
        &panel,
        &scene.style,
        &layout,
        &viewport,
        &scene.series,
        scene.heatmap.as_ref().map(|data| &data.layer),
        (width.round() as u32, height.round() as u32),
    );

    PlotFrame {
        layout,
        viewport,
        buffer,
        points: (width, height),
        size,
        panel: scene.panel.clone(),
        series: scene.series.clone(),
        heatmap: scene
            .heatmap
            .as_ref()
            .map(|data| Arc::as_ptr(data) as usize),
        x_range: state.x_range,
        y_range: state.y_range,
        show_grid,
    }
}

/// The recorded width and height: the plate's device size, so the two agree.
fn figure_points(
    _scene: &PanelScene,
    _x_scale: Scale,
    _y_scale: Scale,
    _state: &PlotState,
    size: (f64, f64),
) -> (f64, f64) {
    (size.0.max(1.0), size.1.max(1.0))
}

/// The plot element: a plate that draws a frame and reports gestures.
pub struct PlotElement {
    id: ElementId,
    scene: Rc<PanelScene>,
    state: Rc<RefCell<PlotState>>,
    frame: Rc<RefCell<Option<PlotFrame>>>,
    hitbox: Rc<RefCell<Option<Hitbox>>>,
    on_gesture: Option<Rc<dyn Fn(PlotGesture, &mut Window, &mut App)>>,
}

impl PlotElement {
    pub fn new(
        id: impl Into<ElementId>,
        scene: Rc<PanelScene>,
        state: Rc<RefCell<PlotState>>,
        frame: Rc<RefCell<Option<PlotFrame>>>,
    ) -> Self {
        Self {
            id: id.into(),
            scene,
            state,
            frame,
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
        let show_grid = self.state.borrow().show_grid;
        let stale = {
            let frame = self.frame.borrow();
            match frame.as_ref() {
                Some(frame) => !frame.describes(&self.scene, &self.state.borrow(), size, show_grid),
                None => true,
            }
        };
        if stale {
            let computed = compute_frame(&self.scene, &self.state.borrow(), size, show_grid);
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
                let scale = bounds.size.width.as_f32() / frame.points.0 as f32;
                paint_buffer(
                    window,
                    cx,
                    &frame.buffer,
                    ReplayTransform::new(bounds.origin, scale),
                );
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
                    scale,
                );
            }
        }

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

/// Paints the selection cursors, the rectangle being dragged, and the pointer
/// crosshair with its coordinates in the margin.
fn paint_overlay(
    window: &mut Window,
    cx: &mut App,
    bounds: Bounds<Pixels>,
    frame: &PlotFrame,
    scene: &PanelScene,
    state: &PlotState,
    scale: f32,
) {
    use gpui_kit::{hsla, px, PathBuilder};
    let plot = frame.layout.plot;
    let accent = scene.accent;
    let ink = to_hsla(accent);

    // A line helper, in plate pixels.
    let line = |window: &mut Window, width: f32, from: (f32, f32), to: (f32, f32)| {
        let mut builder = PathBuilder::stroke(px(width));
        builder.move_to(point(bounds.origin.x + px(from.0), bounds.origin.y + px(from.1)));
        builder.line_to(point(bounds.origin.x + px(to.0), bounds.origin.y + px(to.1)));
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
                        paint_margin_label(window, cx, bounds, (plot.x0 as f32 - 4.0, y as f32), &cursor.label, ink, 10.0, true, scale);
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
                    let mut builder = PathBuilder::stroke(px(1.0));
                    builder.move_to(point(
                        bounds.origin.x + px(px_x as f32),
                        bounds.origin.y + px(plot.y0 as f32),
                    ));
                    builder.line_to(point(
                        bounds.origin.x + px(px_x as f32),
                        bounds.origin.y + px(plot.y1 as f32),
                    ));
                    if let Ok(path) = builder.build() {
                        window.paint_path(path, ghost);
                    }
                    let mut builder = PathBuilder::stroke(px(1.0));
                    builder.move_to(point(
                        bounds.origin.x + px(plot.x0 as f32),
                        bounds.origin.y + px(px_y as f32),
                    ));
                    builder.line_to(point(
                        bounds.origin.x + px(plot.x1 as f32),
                        bounds.origin.y + px(px_y as f32),
                    ));
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
                        scale,
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
                        scale,
                    );
                }
            }
        }
    }
}

/// A short label placed in the plate's margin.
#[allow(clippy::too_many_arguments)]
fn paint_margin_label(
    window: &mut Window,
    cx: &mut App,
    bounds: Bounds<Pixels>,
    at: (f32, f32),
    text: &str,
    color: gpui_kit::Hsla,
    font_size: f32,
    right_aligned: bool,
    scale: f32,
) {
    use gpui_kit::{point, px, TextAlign};
    let line = crate::replay::shape(window, text, px(font_size * scale), color);
    let x = if right_aligned {
        bounds.origin.x + px(at.0 * scale) - line.width()
    } else {
        bounds.origin.x + px(at.0 * scale) - line.width() / 2.0
    };
    let _ = line.paint(
        point(x, bounds.origin.y + px(at.1 * scale)),
        px(font_size * scale),
        TextAlign::Left,
        None,
        window,
        cx,
    );
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
}

impl Plate {
    pub fn new(scene: PanelScene, state: PlotState) -> Self {
        Self {
            scene: Rc::new(scene),
            state: Rc::new(RefCell::new(state)),
            frame: Rc::new(RefCell::new(None)),
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
        PlotElement::new(id, self.scene.clone(), self.state.clone(), self.frame.clone())
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
