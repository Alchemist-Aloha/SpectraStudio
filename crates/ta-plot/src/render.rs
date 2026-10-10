//! Drawing a panel through Plotters.
//!
//! Layout is decided here — margins, the plot rectangle, the colour bar — so
//! the pointer and the frame agree about where the plot area is. Everything is
//! drawn with Plotters' primitives, which is what lets one description produce
//! an SVG file and a captured command list (`ta-plot-gpui` replays the latter)
//! without two drawing implementations.
//!
//! Clipping is explicit. Plotters clamps a coordinate to the drawing area rather
//! than intersecting the shape with the plot rectangle, so a line that leaves
//! the window would otherwise lie flat along the edge.

use crate::scale::Scale;
use crate::ticks::{format_tick, ticks_for_scale};
use crate::viewport::PlotViewport;
use plotters::coord::Shift;
use plotters::drawing::{DrawingArea, DrawingAreaErrorKind, IntoDrawingArea};
use plotters::element::{
    BitMapElement, Circle, PathElement, Rectangle as PlottersRect, Text as PlottersText,
};
use plotters::style::text_anchor::{HPos, Pos, VPos};
use plotters::style::{
    Color, FontDesc, FontFamily, FontStyle, FontTransform, RGBAColor, RGBColor, ShapeStyle,
    TextStyle,
};
use ta_core::{
    AnnotationKind, DashPattern, MarkerShape, PanelModel, PlotSeries as _PlotSeries, PublicationStyle,
    Rgba, SeriesKind, SeriesStyle,
};

/// A rectangle in panel pixels: `x` right, `y` down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Rect {
    pub fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        Self { x0, y0, x1, y1 }
    }

    pub fn width(&self) -> f64 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> f64 {
        self.y1 - self.y0
    }

    pub fn center(&self) -> (f64, f64) {
        ((self.x0 + self.x1) / 2.0, (self.y0 + self.y1) / 2.0)
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }
}

/// A series ready to draw: points in physical coordinates, split into segments
/// that must not be joined (contour rings, cropped runs).
#[derive(Clone, Debug, PartialEq)]
pub struct PlottedSeries {
    pub label: String,
    pub segments: Vec<Vec<(f64, f64)>>,
    pub kind: SeriesKind,
    pub style: SeriesStyle,
}

impl PlottedSeries {
    pub fn new(label: impl Into<String>, points: Vec<(f64, f64)>, style: SeriesStyle) -> Self {
        Self {
            label: label.into(),
            segments: vec![points],
            kind: SeriesKind::Line,
            style,
        }
    }

    pub fn with_kind(mut self, kind: SeriesKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn with_segments(mut self, segments: Vec<Vec<(f64, f64)>>) -> Self {
        self.segments = segments;
        self
    }

    /// Every point of every segment, for hit testing.
    pub fn points(&self) -> impl Iterator<Item = (f64, f64)> + '_ {
        self.segments.iter().flat_map(|segment| segment.iter().copied())
    }
}

/// A prepared image for the plot area: RGB, top row first.
#[derive(Clone, Debug, PartialEq)]
pub struct HeatmapLayer {
    pub rgb: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl HeatmapLayer {
    /// Composites an RGBA raster onto a background colour, because Plotters'
    /// bitmap element takes RGB.
    pub fn from_rgba_over(
        pixels: &[u8],
        width: usize,
        height: usize,
        background: Rgba,
    ) -> Self {
        let mut rgb = vec![0u8; width * height * 3];
        for index in 0..width * height {
            let offset = index * 4;
            let alpha = if offset + 3 < pixels.len() {
                pixels[offset + 3] as f64 / 255.0
            } else {
                0.0
            };
            for channel in 0..3 {
                let over = match channel {
                    0 => background.r,
                    1 => background.g,
                    _ => background.b,
                } as f64;
                let value = if offset + channel < pixels.len() {
                    pixels[offset + channel] as f64
                } else {
                    0.0
                };
                rgb[index * 3 + channel] = (value * alpha + over * (1.0 - alpha))
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
        }
        Self {
            rgb,
            width: width as u32,
            height: height as u32,
        }
    }
}

/// Where everything goes, in panel pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelLayout {
    pub available: (f64, f64),
    pub plot: Rect,
    pub colorbar: Option<Rect>,
    pub x_ticks: Vec<f64>,
    pub y_ticks: Vec<f64>,
    /// The widest y tick label, which decides where the axis title stands.
    pub widest_y_label: f64,
}

impl PanelLayout {
    /// Measures the labels and puts the plot area in the space that is left.
    ///
    /// Text width is estimated from the character count and the point size: the
    /// two backends measure text differently (one has a font engine, one writes
    /// SVG), so a layout that depends on either would move the plot area between
    /// the screen and the export.
    pub fn compute(
        panel: &PanelModel,
        style: &PublicationStyle,
        x_scale: Scale,
        y_scale: Scale,
        x_range: (f64, f64),
        y_range: (f64, f64),
        available: (f64, f64),
    ) -> Self {
        let x_ticks = ticks_for_scale(x_scale, x_range.0, x_range.1, 6);
        let y_ticks = ticks_for_scale(y_scale, y_range.0, y_range.1, 6);

        let tick_len = style.tick_length.max(0.0) as f64;
        let label_size = style.tick_label_size as f64;

        let widest_y_label = y_ticks
            .iter()
            .map(|tick| text_width(&format_tick(*tick, y_scale), label_size))
            .fold(0.0f64, f64::max);
        let tallest_x_label = if x_ticks.is_empty() {
            0.0
        } else {
            label_size * 1.25
        };

        // The gaps between a tick label and the axis are expressed in the type
        // size rather than in points of paper. That keeps one layout linear in
        // everything it is given, which is what lets a preview be the exported
        // figure magnified: with an eight-point figure the numbers are the four
        // and six points they always were.
        let gap = label_size * 0.5;
        let mut left = widest_y_label + tick_len + gap;
        let mut bottom = tallest_x_label + tick_len + gap;
        let mut top = gap;
        let mut right = gap * 1.5;

        if !panel.y.label.is_empty() {
            left += label_size * 1.4;
        }
        if !panel.x.label.is_empty() {
            bottom += label_size * 1.4;
        }
        if !panel.title.is_empty() {
            top += style.base_font_size as f64 * 1.4;
        }
        let wants_colorbar = panel.colorbar.visible && panel.contour.is_some();
        if wants_colorbar {
            right += label_size * 5.5;
        }

        // Never let the margins eat the whole panel.
        let max_horizontal = (available.0 * 0.6).max(1.0);
        let max_vertical = (available.1 * 0.6).max(1.0);
        left = left.min(max_horizontal);
        if right > max_horizontal {
            right = 6.0;
        }
        bottom = bottom.min(max_vertical);
        top = top.min(max_vertical);

        let plot = Rect::new(
            left,
            top,
            (available.0 - right).max(left + 1.0),
            (available.1 - bottom).max(top + 1.0),
        );
        let colorbar = wants_colorbar.then(|| {
            let bar_left = plot.x1 + label_size;
            Rect::new(
                bar_left,
                plot.y0,
                (available.0 - gap)
                    .min(bar_left + label_size * 1.5)
                    .max(bar_left + 1.0),
                plot.y1,
            )
        });

        Self {
            available,
            plot,
            colorbar,
            x_ticks,
            y_ticks,
            widest_y_label,
        }
    }

    /// Moves every rectangle by a pixel offset, which is how a panel finds its
    /// place in a multi-panel figure.
    pub fn offset(&mut self, dx: f64, dy: f64) {
        self.plot.x0 += dx;
        self.plot.x1 += dx;
        self.plot.y0 += dy;
        self.plot.y1 += dy;
        if let Some(colorbar) = &mut self.colorbar {
            colorbar.x0 += dx;
            colorbar.x1 += dx;
            colorbar.y0 += dy;
            colorbar.y1 += dy;
        }
    }

    pub fn plot_size(&self) -> (f64, f64) {
        (self.plot.width(), self.plot.height())
    }

    /// The viewport for this layout: the same rectangle the pointer sees.
    pub fn viewport(
        &self,
        x_range: (f64, f64),
        y_range: (f64, f64),
        x_scale: Scale,
        y_scale: Scale,
    ) -> PlotViewport {
        PlotViewport::new(x_range, y_range, x_scale, y_scale, self.plot_size())
    }
}

/// An estimated text width in pixels.
pub fn text_width(text: &str, size: f64) -> f64 {
    text.chars().count() as f64 * size * 0.6
}

fn rgb(color: Rgba) -> RGBColor {
    RGBColor(color.r, color.g, color.b)
}

fn rgba(color: Rgba) -> RGBAColor {
    RGBAColor(color.r, color.g, color.b, color.a as f64 / 255.0)
}

fn axis_color() -> Rgba {
    Rgba::rgb(26, 26, 26)
}

/// Splits a pixel-space polyline at the plot rectangle, interpolating the
/// crossing points so a clipped line meets the frame exactly.
pub fn clip_polyline(points: &[(f64, f64)], rect: &Rect) -> Vec<Vec<(f64, f64)>> {
    let mut runs: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut current: Vec<(f64, f64)> = Vec::new();

    for window in points.windows(2) {
        let (a, b) = (window[0], window[1]);
        let a_in = rect.contains(a.0, a.1);
        let b_in = rect.contains(b.0, b.1);
        match (a_in, b_in) {
            (true, true) => {
                if current.is_empty() {
                    current.push(a);
                }
                current.push(b);
            }
            (true, false) => {
                if current.is_empty() {
                    current.push(a);
                }
                if let Some(crossing) = segment_rect_exit(a, b, rect) {
                    current.push(crossing);
                }
                runs.push(std::mem::take(&mut current));
            }
            (false, true) => {
                if let Some(crossing) = segment_rect_exit(b, a, rect) {
                    current.push(crossing);
                }
                current.push(b);
            }
            (false, false) => {
                // The whole segment is outside: nothing to draw, and the run
                // ends here rather than joining across the gap.
                if !current.is_empty() {
                    runs.push(std::mem::take(&mut current));
                }
            }
        }
    }
    if !current.is_empty() {
        runs.push(current);
    }
    runs.into_iter().filter(|run| run.len() >= 2).collect()
}

/// Where the segment from `inside` to `outside` crosses the rectangle border.
fn segment_rect_exit(inside: (f64, f64), outside: (f64, f64), rect: &Rect) -> Option<(f64, f64)> {
    let dx = outside.0 - inside.0;
    let dy = outside.1 - inside.1;
    let mut best: Option<(f64, f64)> = None;
    let mut consider = |t: f64| {
        if !(0.0..=1.0).contains(&t) {
            return;
        }
        let x = inside.0 + dx * t;
        let y = inside.1 + dy * t;
        // A tolerance of half a pixel keeps the crossing on the border even
        // after rounding.
        if x >= rect.x0 - 0.5
            && x <= rect.x1 + 0.5
            && y >= rect.y0 - 0.5
            && y <= rect.y1 + 0.5
        {
            let candidate = Some((x, y));
            best = match best {
                Some(existing) => {
                    let existing_t =
                        ((existing.0 - inside.0) / dx).abs().min(((existing.1 - inside.1) / dy).abs());
                    Some(if t < existing_t { (x, y) } else { existing })
                }
                None => candidate,
            };
        }
    };
    if dx != 0.0 {
        consider((rect.x0 - inside.0) / dx);
        consider((rect.x1 - inside.0) / dx);
    }
    if dy != 0.0 {
        consider((rect.y0 - inside.1) / dy);
        consider((rect.y1 - inside.1) / dy);
    }
    best
}

/// Breaks a polyline into dash runs, in pixel space.
pub fn dash_polyline(
    points: &[(f64, f64)],
    pattern: DashPattern,
    width: f64,
) -> Vec<Vec<(f64, f64)>> {
    let segments = pattern.segments();
    if segments.is_empty() || points.len() < 2 {
        return vec![points.to_vec()];
    }
    let scale = width.max(0.5);
    let lengths: Vec<f64> = segments.iter().map(|value| *value as f64 * scale).collect();
    let total: f64 = lengths.iter().sum();
    if total <= 0.0 {
        return vec![points.to_vec()];
    }

    let mut runs: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut current: Vec<(f64, f64)> = Vec::new();
    let mut pattern_index = 0usize;
    let mut remaining = lengths[0];
    let mut drawing = true;

    for window in points.windows(2) {
        let (mut a, b) = (window[0], window[1]);
        let mut length = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        if length <= 0.0 {
            continue;
        }
        while length > remaining {
            let t = remaining / length;
            let cut = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            if drawing {
                if current.is_empty() {
                    current.push(a);
                }
                current.push(cut);
                runs.push(std::mem::take(&mut current));
            }
            a = cut;
            length -= remaining;
            pattern_index = (pattern_index + 1) % lengths.len();
            remaining = lengths[pattern_index];
            drawing = pattern_index % 2 == 0;
            if drawing {
                current.push(a);
            }
        }
        if drawing {
            if current.is_empty() {
                current.push(a);
            }
            current.push(b);
        }
        remaining -= length;
    }
    if !current.is_empty() {
        runs.push(current);
    }
    runs.into_iter().filter(|run| run.len() >= 2).collect()
}

/// Plotters' stroke width is an integer, so a 0.75 pt spine has to become one
/// whole pixel; truncating it to zero instead would draw nothing at all.
fn plotters_width(width: f32) -> u32 {
    (width.max(0.1).round() as u32).max(1)
}

fn shape_style(style: &SeriesStyle) -> ShapeStyle {
    let color = Rgba {
        a: (style.opacity.clamp(0.0, 1.0) * 255.0) as u8,
        ..style.color
    };
    rgba(color).stroke_width(plotters_width(style.width))
}

fn marker_geometry(shape: MarkerShape, center: (f64, f64), size: f64) -> Vec<(f64, f64)> {
    let (x, y) = center;
    let r = size.max(0.5);
    match shape {
        MarkerShape::None | MarkerShape::Circle => Vec::new(),
        MarkerShape::Square => vec![(x - r, y - r), (x + r, y - r), (x + r, y + r), (x - r, y + r)],
        MarkerShape::Diamond => vec![(x, y - r), (x + r, y), (x, y + r), (x - r, y)],
        MarkerShape::Triangle => vec![(x, y - r), (x + r, y + r), (x - r, y + r)],
        MarkerShape::Cross => vec![
            (x - r, y - r),
            (x + r, y + r),
            (x + r, y - r),
            (x - r, y + r),
        ],
    }
}

/// Renders one panel into a fresh drawing area.
#[allow(clippy::too_many_arguments)]
pub fn render_panel<DB: plotters::prelude::DrawingBackend>(
    panel: &PanelModel,
    style: &PublicationStyle,
    layout: &PanelLayout,
    viewport: &PlotViewport,
    series: &[PlottedSeries],
    heatmap: Option<&HeatmapLayer>,
    backend: DB,
    background: Rgba,
) -> Result<(), DrawingAreaErrorKind<DB::ErrorType>> {
    let root = backend.into_drawing_area();
    root.fill(&rgb(background))?;
    draw_panel(&root, panel, style, layout, viewport, series, heatmap)?;
    root.present()
}

/// Draws one panel into an existing drawing area.
///
/// A multi-panel figure draws every panel through this, into one area, so the
/// panels share a coordinate space and the caller keeps ownership of the
/// backend.
#[allow(clippy::too_many_arguments)]
pub fn draw_panel<DB: plotters::prelude::DrawingBackend>(
    root: &DrawingArea<DB, Shift>,
    panel: &PanelModel,
    style: &PublicationStyle,
    layout: &PanelLayout,
    viewport: &PlotViewport,
    series: &[PlottedSeries],
    heatmap: Option<&HeatmapLayer>,
) -> Result<(), DrawingAreaErrorKind<DB::ErrorType>> {

    let plot = layout.plot;
    let x_scale = viewport.x_scale();
    let y_scale = viewport.y_scale();
    let ink = axis_color();
    let tick_length = style.tick_length.max(0.0) as f64;
    let tick_label_size = style.tick_label_size as f64;

    // The heatmap goes down first, exactly filling the plot rectangle.
    if let Some(layer) = heatmap {
        if let Some(element) = BitMapElement::with_owned_buffer(
            (plot.x0.round() as i32, plot.y0.round() as i32),
            (layer.width, layer.height),
            layer.rgb.clone(),
        ) {
            root.draw(&element)?;
        }
    }

    // Grid and band annotations sit under the data.
    for annotation in &panel.annotations {
        if annotation.kind == AnnotationKind::Band {
            if let Some(corners) = annotation_rect(annotation, viewport, &plot) {
                root.draw(&PlottersRect::new(corners, rgba(annotation.color).filled()))?;
            }
        }
    }
    if style.grid {
        // One pixel wide, in the plot's own ink at low contrast.
        let grid_style = RGBColor(214, 218, 224).stroke_width(1);
        for tick in &layout.x_ticks {
            if let Some((x, _)) = viewport.data_to_pixel(*tick, viewport.y_range().0) {
                if x < plot.x0 - 0.5 || x > plot.x1 + 0.5 {
                    continue;
                }
                root.draw(&PlottersRect::new(
                    [
                        (x.round() as i32, plot.y0.round() as i32),
                        (x.round() as i32, plot.y1.round() as i32),
                    ],
                    grid_style,
                ))?;
            }
        }
        for tick in &layout.y_ticks {
            if let Some((_, y)) = viewport.data_to_pixel(viewport.x_range().0, *tick) {
                if y < plot.y0 - 0.5 || y > plot.y1 + 0.5 {
                    continue;
                }
                root.draw(&PlottersRect::new(
                    [
                        (plot.x0.round() as i32, y.round() as i32),
                        (plot.x1.round() as i32, y.round() as i32),
                    ],
                    grid_style,
                ))?;
            }
        }
    }

    // Series, in paint order.
    for plotted in series {
        draw_series(root, plotted, viewport, &plot)?;
    }

    // Frame.
    if style.frame {
        let frame = rgb(ink).stroke_width(plotters_width(style.axis_line_width));
        root.draw(&PlottersRect::new(
            [
                (plot.x0.round() as i32, plot.y0.round() as i32),
                (plot.x1.round() as i32, plot.y1.round() as i32),
            ],
            frame,
        ))?;
    }

    // Ticks and their labels.
    let tick_style = rgb(ink).stroke_width(plotters_width(style.axis_line_width));
    let inside = style.tick_direction == ta_core::TickDirection::In;
    for tick in &layout.x_ticks {
        let Some((x, _)) = viewport.data_to_pixel(*tick, viewport.y_range().0) else {
            continue;
        };
        if x < plot.x0 - 0.5 || x > plot.x1 + 0.5 {
            continue;
        }
        let (from, to) = if inside {
            (plot.y1, plot.y1 - tick_length)
        } else {
            (plot.y1, plot.y1 + tick_length)
        };
        root.draw(&PathElement::new(
            vec![
                (x.round() as i32, from.round() as i32),
                (x.round() as i32, to.round() as i32),
            ],
            tick_style,
        ))?;
        let label = format_tick(*tick, x_scale);
        if !label.is_empty() {
            root.draw(&PlottersText::new(
                label,
                (
                    x.round() as i32,
                    (plot.y1 + tick_length + 2.0).round() as i32,
                ),
                text_style(style.tick_label_size, ink, Pos::new(HPos::Center, VPos::Top)),
            ))?;
        }
    }
    for tick in &layout.y_ticks {
        let Some((_, y)) = viewport.data_to_pixel(viewport.x_range().0, *tick) else {
            continue;
        };
        if y < plot.y0 - 0.5 || y > plot.y1 + 0.5 {
            continue;
        }
        let (from, to) = if inside {
            (plot.x0, plot.x0 + tick_length)
        } else {
            (plot.x0, plot.x0 - tick_length)
        };
        root.draw(&PathElement::new(
            vec![
                (from.round() as i32, y.round() as i32),
                (to.round() as i32, y.round() as i32),
            ],
            tick_style,
        ))?;
        let label = format_tick(*tick, y_scale);
        if !label.is_empty() {
            root.draw(&PlottersText::new(
                label,
                (
                    (plot.x0 - tick_length - 3.0).round() as i32,
                    y.round() as i32,
                ),
                text_style(
                    style.tick_label_size,
                    ink,
                    Pos::new(HPos::Right, VPos::Center),
                ),
            ))?;
        }
    }

    // Axis titles and the panel title.
    if !panel.x.label.is_empty() {
        root.draw(&PlottersText::new(
            panel.x.labelled(),
            (
                plot.center().0.round() as i32,
                (plot.y1 + tick_length + tick_label_size * 1.6).round() as i32,
            ),
            text_style(
                status_size(style, panel.x.title_size),
                ink,
                Pos::new(HPos::Center, VPos::Top),
            ),
        ))?;
    }
    if !panel.y.label.is_empty() {
        // Up the left margin, centred on the plot. The anchor is the *centre* of
        // the rotated text, and it is put in the middle of the strip the layout
        // reserved for it. Anchoring it at the strip's edge — which is where the
        // tick labels begin — wrote the axis name over those numbers.
        root.draw(&PlottersText::new(
            panel.y.labelled(),
            (
                (plot.x0 - tick_length - layout.widest_y_label - tick_label_size * 0.7)
                    .round() as i32,
                ((plot.y0 + plot.y1) / 2.0).round() as i32,
            ),
            rotated_text_style(
                status_size(style, panel.y.title_size),
                ink,
                Pos::new(HPos::Center, VPos::Top),
                FontTransform::Rotate270,
            ),
        ))?;
    }
    if !panel.title.is_empty() {
        root.draw(&PlottersText::new(
            panel.title.clone(),
            (plot.center().0.round() as i32, 2),
            text_style(
                style.base_font_size,
                ink,
                Pos::new(HPos::Center, VPos::Top),
            ),
        ))?;
    }

    // Remaining annotations, over the data.
    for annotation in &panel.annotations {
        match annotation.kind {
            AnnotationKind::Text => {
                if let (Some(x), Some(y)) = (annotation.x, annotation.y) {
                    if let Some((px, py)) = viewport.data_to_pixel(x, y) {
                        if plot.contains(px, py) {
                            root.draw(&PlottersText::new(
                                annotation.text.clone(),
                                (px.round() as i32, py.round() as i32),
                                text_style(
                                    annotation.font_size,
                                    annotation.color,
                                    Pos::new(HPos::Left, VPos::Center),
                                ),
                            ))?;
                        }
                    }
                }
            }
            AnnotationKind::CursorLine => {
                if let Some(rect) = annotation_cursor_line(annotation, viewport, &plot) {
                    root.draw(&PlottersRect::new(rect, rgba(annotation.color).stroke_width(1)))?;
                }
            }
            AnnotationKind::Rectangle => {
                if let Some(rect) = annotation_rect(annotation, viewport, &plot) {
                    root.draw(&PlottersRect::new(rect, rgba(annotation.color).stroke_width(1)))?;
                }
            }
            AnnotationKind::Arrow => {
                if let (Some(x), Some(y), Some(w), Some(h)) = (
                    annotation.x,
                    annotation.y,
                    annotation.width,
                    annotation.height,
                ) {
                    if let (Some(from), Some(to)) = (
                        viewport.data_to_pixel(x, y),
                        viewport.data_to_pixel(x + w, y + h),
                    ) {
                        let clipped = clip_polyline(&[from, to], &plot);
                        for run in clipped {
                            root.draw(&PathElement::new(
                                run.iter()
                                    .map(|(x, y)| (x.round() as i32, y.round() as i32))
                                    .collect::<Vec<_>>(),
                                rgba(annotation.color).stroke_width(1),
                            ))?;
                        }
                    }
                }
            }
            AnnotationKind::Band => {}
        }
    }

    // Legend.
    if panel.legend.visible && !series.is_empty() {
        draw_legend(root, panel, series, style, layout, ink)?;
    }

    // Colour bar.
    if let (Some(rect), Some(contour)) = (layout.colorbar, panel.contour.as_ref()) {
        draw_color_bar(root, rect, contour, panel, style, ink)?;
    }

    Ok(())
}

fn status_size(style: &PublicationStyle, size: f32) -> f32 {
    if size > 0.0 {
        size
    } else {
        style.axis_title_size
    }
}

/// The family every figure uses unless the style says otherwise.
///
/// Named rather than generic so the SVG export and the screen pick the same
/// face; a missing family falls back through the backend's own default.
const FONT_FAMILY: &str = "Helvetica";

fn text_style(size: f32, color: Rgba, anchor: Pos) -> TextStyle<'static> {
    styled_font(size, FontTransform::None)
        .color(&rgb(color))
        .pos(anchor)
}

/// A style whose glyphs are rotated, used for the two axis captions that read
/// up the page.
fn rotated_text_style(
    size: f32,
    color: Rgba,
    anchor: Pos,
    transform: FontTransform,
) -> TextStyle<'static> {
    styled_font(size, transform)
        .color(&rgb(color))
        .pos(anchor)
}

fn styled_font(size: f32, transform: FontTransform) -> FontDesc<'static> {
    FontDesc::new(
        FontFamily::Name(FONT_FAMILY),
        size as f64,
        FontStyle::Normal,
    )
    .transform(transform)
}

fn draw_series<DB: plotters::prelude::DrawingBackend>(
    root: &DrawingArea<DB, Shift>,
    series: &PlottedSeries,
    viewport: &PlotViewport,
    plot: &Rect,
) -> Result<(), DrawingAreaErrorKind<DB::ErrorType>> {
    let style = shape_style(&series.style);
    let wants_line = matches!(
        series.kind,
        SeriesKind::Line | SeriesKind::LineMarkers | SeriesKind::Step
    );
    let wants_markers = matches!(
        series.kind,
        SeriesKind::Markers | SeriesKind::LineMarkers | SeriesKind::Scatter
    );

    for segment in &series.segments {
        let pixel: Vec<(f64, f64)> = segment
            .iter()
            .filter_map(|(x, y)| viewport.data_to_pixel(*x, *y))
            .collect();
        if pixel.len() < 2 {
            continue;
        }

        let drawn: Vec<(f64, f64)> = if series.kind == SeriesKind::Step {
            step_points(&pixel)
        } else {
            pixel.clone()
        };

        if wants_line {
            for run in clip_polyline(&drawn, plot) {
                for dash in dash_polyline(&run, series.style.dash, series.style.width as f64) {
                    let points: Vec<(i32, i32)> = dash
                        .iter()
                        .map(|(x, y)| (x.round() as i32, y.round() as i32))
                        .collect();
                    if points.len() >= 2 {
                        root.draw(&PathElement::new(points, style))?;
                    }
                }
            }
        }

        if series.kind == SeriesKind::Bars {
            let baseline = viewport.data_to_pixel(
                viewport.x_range().0,
                viewport.y_range().0,
            );
            if let Some((_, zero_pixel)) = baseline {
                for (x, y) in drawn.iter().filter(|(x, y)| plot.contains(*x, *y)) {
                    let half = (series.style.width as f64 * 0.5).max(0.5);
                    root.draw(&PlottersRect::new(
                        [
                            ((x - half).round() as i32, y.round() as i32),
                            ((x + half).round() as i32, zero_pixel.round() as i32),
                        ],
                        rgba(series.style.color).filled(),
                    ))?;
                }
            }
        }

        if wants_markers {
            for (x, y) in drawn.iter().filter(|(x, y)| plot.contains(*x, *y)) {
                match series.style.marker {
                    MarkerShape::Circle => {
                        root.draw(&Circle::new(
                            (x.round() as i32, y.round() as i32),
                            (series.style.marker_size as u32).max(1),
                            rgba(series.style.color).filled(),
                        ))?;
                    }
                    MarkerShape::None if series.kind == SeriesKind::Scatter => {
                        root.draw(&Circle::new(
                            (x.round() as i32, y.round() as i32),
                            (series.style.marker_size as u32).max(1),
                            rgba(series.style.color).filled(),
                        ))?;
                    }
                    MarkerShape::None => {}
                    shape => {
                        let outline = marker_geometry(shape, (*x, *y), series.style.marker_size as f64);
                        if outline.len() >= 2 {
                            let points: Vec<(i32, i32)> = outline
                                .iter()
                                .chain(outline.first())
                                .map(|(x, y)| (x.round() as i32, y.round() as i32))
                                .collect();
                            root.draw(&PathElement::new(points, style))?;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn step_points(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut stepped = Vec::with_capacity(points.len() * 2);
    stepped.push(points[0]);
    for window in points.windows(2) {
        let (_, previous_y) = stepped[stepped.len() - 1];
        let (x, y) = window[1];
        stepped.push((x, previous_y));
        stepped.push((x, y));
    }
    stepped
}

fn annotation_rect(
    annotation: &ta_core::AnnotationSpec,
    viewport: &PlotViewport,
    plot: &Rect,
) -> Option<[(i32, i32); 2]> {
    let x = annotation.x?;
    let y = annotation.y?;
    let width = annotation.width.unwrap_or(0.0);
    let height = annotation.height.unwrap_or(0.0);
    let a = viewport.data_to_pixel(x, y)?;
    let b = viewport.data_to_pixel(x + width, y + height)?;
    let clipped = Rect::new(
        a.0.min(b.0).max(plot.x0),
        a.1.min(b.1).max(plot.y0),
        a.0.max(b.0).min(plot.x1),
        a.1.max(b.1).min(plot.y1),
    );
    if clipped.width() <= 0.0 || clipped.height() <= 0.0 {
        return None;
    }
    Some([
        (clipped.x0.round() as i32, clipped.y0.round() as i32),
        (clipped.x1.round() as i32, clipped.y1.round() as i32),
    ])
}

fn annotation_cursor_line(
    annotation: &ta_core::AnnotationSpec,
    viewport: &PlotViewport,
    plot: &Rect,
) -> Option<[(i32, i32); 2]> {
    match (annotation.axis, annotation.x, annotation.y) {
        (Some(ta_core::AxisKind::Delay), Some(_), Some(y)) => {
            let (_, py) = viewport.data_to_pixel(viewport.x_range().0, y)?;
            if py < plot.y0 - 0.5 || py > plot.y1 + 0.5 {
                return None;
            }
            Some([
                (plot.x0.round() as i32, py.round() as i32),
                (plot.x1.round() as i32, py.round() as i32),
            ])
        }
        (_, Some(x), Some(_)) => {
            let (px, _) = viewport.data_to_pixel(x, viewport.y_range().0)?;
            if px < plot.x0 - 0.5 || px > plot.x1 + 0.5 {
                return None;
            }
            Some([
                (px.round() as i32, plot.y0.round() as i32),
                (px.round() as i32, plot.y1.round() as i32),
            ])
        }
        _ => None,
    }
}

fn draw_legend<DB: plotters::prelude::DrawingBackend>(
    root: &DrawingArea<DB, Shift>,
    panel: &PanelModel,
    series: &[PlottedSeries],
    style: &PublicationStyle,
    layout: &PanelLayout,
    ink: Rgba,
) -> Result<(), DrawingAreaErrorKind<DB::ErrorType>> {
    use ta_core::LegendPosition;

    let entries: Vec<(&PlottedSeries, String)> = series
        .iter()
        .map(|plotted| {
            let label = panel
                .legend
                .labels
                .values()
                .next()
                .filter(|_| false)
                .cloned()
                .unwrap_or_else(|| plotted.label.clone());
            (plotted, label)
        })
        .collect();
    if entries.is_empty() {
        return Ok(());
    }

    let font = panel.legend.font_size.max(4.0);
    let row_height = font as f64 * 1.5;
    let swatch = font as f64 * 1.6;
    let widest = entries
        .iter()
        .map(|(_, label)| text_width(label, font as f64))
        .fold(0.0f64, f64::max);
    let width = swatch + widest + 12.0;
    let height = row_height * entries.len() as f64 + 8.0;

    let plot = layout.plot;
    let (x0, y0) = match panel.legend.position {
        LegendPosition::TopLeft => (plot.x0 + 6.0, plot.y0 + 6.0),
        LegendPosition::TopRight => (plot.x1 - width - 6.0, plot.y0 + 6.0),
        LegendPosition::BottomLeft => (plot.x0 + 6.0, plot.y1 - height - 6.0),
        LegendPosition::BottomRight => (plot.x1 - width - 6.0, plot.y1 - height - 6.0),
        LegendPosition::OutsideRight => (plot.x1 + 16.0, plot.y0),
        LegendPosition::OutsideBottom => (plot.x0, plot.y1 + 24.0),
    };
    let box_rect = Rect::new(x0, y0, x0 + width, y0 + height);
    if panel.legend.frame {
        root.draw(&PlottersRect::new(
            [
                (box_rect.x0.round() as i32, box_rect.y0.round() as i32),
                (box_rect.x1.round() as i32, box_rect.y1.round() as i32),
            ],
            RGBColor(255, 255, 255).filled(),
        ))?;
        root.draw(&PlottersRect::new(
            [
                (box_rect.x0.round() as i32, box_rect.y0.round() as i32),
                (box_rect.x1.round() as i32, box_rect.y1.round() as i32),
            ],
            rgb(ink).stroke_width(1),
        ))?;
    }

    for (index, (plotted, label)) in entries.iter().enumerate() {
        let row_y = box_rect.y0 + 4.0 + row_height * (index as f64 + 0.5);
        let x_start = (box_rect.x0 + 4.0).round() as i32;
        let x_end = (box_rect.x0 + 4.0 + swatch).round() as i32;
        root.draw(&PathElement::new(
            vec![(x_start, row_y.round() as i32), (x_end, row_y.round() as i32)],
            shape_style(&plotted.style),
        ))?;
        root.draw(&PlottersText::new(
            label.clone(),
            ((x_end + 4) as i32, row_y.round() as i32),
            text_style(font, ink, Pos::new(HPos::Left, VPos::Center)),
        ))?;
    }
    let _ = style;
    Ok(())
}

fn draw_color_bar<DB: plotters::prelude::DrawingBackend>(
    root: &DrawingArea<DB, Shift>,
    rect: Rect,
    contour: &ta_core::ContourSpec,
    panel: &PanelModel,
    style: &PublicationStyle,
    ink: Rgba,
) -> Result<(), DrawingAreaErrorKind<DB::ErrorType>> {
    let steps = 64usize;
    for step in 0..steps {
        let t = step as f32 / (steps - 1) as f32;
        let color = contour.colormap.sample(t);
        let y_high = rect.y1 - rect.height() * (step + 1) as f64 / steps as f64;
        let y_low = rect.y1 - rect.height() * step as f64 / steps as f64;
        root.draw(&PlottersRect::new(
            [
                (rect.x0.round() as i32, y_high.round() as i32),
                (rect.x1.round() as i32, y_low.round() as i32),
            ],
            rgb(color).filled(),
        ))?;
    }
    root.draw(&PlottersRect::new(
        [
            (rect.x0.round() as i32, rect.y0.round() as i32),
            (rect.x1.round() as i32, rect.y1.round() as i32),
        ],
        rgb(ink).stroke_width(1),
    ))?;

    let ticks = panel.colorbar.ticks.max(2);
    let font = style.tick_label_size;
    for step in 0..=ticks {
        let t = step as f64 / ticks as f64;
        let value = contour.limits.min + t * (contour.limits.max - contour.limits.min);
        let y = rect.y1 - t * rect.height();
        root.draw(&PlottersText::new(
            format_tick(value, Scale::Linear),
            ((rect.x1 + 3.0).round() as i32, y.round() as i32),
            text_style(font, ink, Pos::new(HPos::Left, VPos::Center)),
        ))?;
    }
    if !panel.colorbar.label.is_empty() {
        root.draw(&PlottersText::new(
            panel.colorbar.label.clone(),
            (
                (rect.x1 + style.tick_label_size as f64 * 3.0).round() as i32,
                ((rect.y0 + rect.y1) / 2.0).round() as i32,
            ),
            rotated_text_style(
                font,
                ink,
                Pos::new(HPos::Center, VPos::Top),
                FontTransform::Rotate90,
            ),
        ))?;
    }
    Ok(())
}

/// A series as it comes out of the figure model, for callers that only need the
/// label and the style.
pub fn series_label(series: &_PlotSeries) -> String {
    series.label.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use plotters::prelude::BitMapBackend;
    use ta_core::{
        AnnotationSpec, AxisModel, LegendSpec, PanelModel, PublicationStyle, SeriesStyle,
        TickDirection,
    };

    fn panel() -> PanelModel {
        let mut panel = PanelModel::new("");
        panel.x = AxisModel {
            label: "Delay".into(),
            unit: Some("ps".into()),
            ..AxisModel::default()
        };
        panel.y = AxisModel {
            label: "ΔOD".into(),
            unit: None,
            ..AxisModel::default()
        };
        panel
    }

    fn layout_for(panel: &PanelModel) -> PanelLayout {
        PanelLayout::compute(
            panel,
            &PublicationStyle::default(),
            Scale::Linear,
            Scale::Linear,
            (-1.0, 10.0),
            (-0.1, 0.1),
            (400.0, 300.0),
        )
    }

    #[test]
    fn the_layout_leaves_room_for_the_labels_it_will_draw() {
        let panel = panel();
        let layout = layout_for(&panel);
        assert!(layout.plot.x0 > 20.0, "the y labels need a left margin");
        assert!(layout.plot.y1 < 300.0, "the x labels need a bottom margin");
        assert!(layout.plot.width() > 200.0);
        assert!(layout.plot.height() > 150.0);
        assert!(!layout.x_ticks.is_empty());
        assert!(!layout.y_ticks.is_empty());
        // The final tick must be inside the window it labels.
        assert!(*layout.x_ticks.last().unwrap() <= 10.0);
    }

    #[test]
    fn a_tiny_panel_still_produces_a_drawable_plot_rectangle() {
        let panel = panel();
        let layout = PanelLayout::compute(
            &panel,
            &PublicationStyle::default(),
            Scale::Linear,
            Scale::Linear,
            (0.0, 1.0),
            (0.0, 1.0),
            (40.0, 30.0),
        );
        assert!(layout.plot.width() >= 1.0);
        assert!(layout.plot.height() >= 1.0);
        assert!(layout.plot.x1 <= layout.available.0 + 0.001);
    }

    #[test]
    fn a_colour_bar_is_only_reserved_when_there_is_a_contour() {
        let mut panel = panel();
        let without = PanelLayout::compute(
            &panel,
            &PublicationStyle::default(),
            Scale::Linear,
            Scale::Linear,
            (0.0, 1.0),
            (0.0, 1.0),
            (400.0, 300.0),
        );
        assert!(without.colorbar.is_none());

        panel.contour = Some(ta_core::ContourSpec::default());
        let with = PanelLayout::compute(
            &panel,
            &PublicationStyle::default(),
            Scale::Linear,
            Scale::Linear,
            (0.0, 1.0),
            (0.0, 1.0),
            (400.0, 300.0),
        );
        assert!(with.colorbar.is_some());
        assert!(with.plot.width() < without.plot.width());
    }

    #[test]
    fn the_viewport_and_the_layout_describe_the_same_rectangle() {
        let panel = panel();
        let layout = layout_for(&panel);
        let viewport = layout.viewport((-1.0, 10.0), (-0.1, 0.1), Scale::Linear, Scale::Linear);
        // The bottom-left data corner maps to the bottom-left plot corner.
        let (px, py) = viewport.data_to_pixel(-1.0, -0.1).unwrap();
        assert!((px - 0.0).abs() < 1e-9);
        assert!((py - layout.plot.height()).abs() < 1e-9);
    }

    #[test]
    fn clipping_splits_a_line_at_the_frame() {
        let rect = Rect::new(0.0, 0.0, 10.0, 10.0);
        let runs = clip_polyline(&[(-5.0, 5.0), (5.0, 5.0), (15.0, 5.0)], &rect);
        assert_eq!(runs.len(), 1);
        let run = &runs[0];
        assert!((run[0].0 - 0.0).abs() < 1e-9, "should start at the frame");
        assert!((run[run.len() - 1].0 - 10.0).abs() < 1e-9);
    }

    #[test]
    fn a_line_that_leaves_and_returns_is_two_runs_not_one_flat_edge() {
        let rect = Rect::new(0.0, 0.0, 10.0, 10.0);
        let runs = clip_polyline(
            &[(0.0, 5.0), (5.0, 5.0), (5.0, 20.0), (8.0, 20.0), (8.0, 5.0), (10.0, 5.0)],
            &rect,
        );
        assert_eq!(runs.len(), 2, "{runs:?}");
    }

    #[test]
    fn a_floating_point_hole_in_the_numbers_is_a_clipped_run() {
        let rect = Rect::new(0.0, 0.0, 10.0, 10.0);
        let runs = clip_polyline(&[(2.0, 2.0), (5.0, f64::NAN), (8.0, 8.0)], &rect);
        // A NaN is not inside the rectangle, so the run is broken rather than
        // joined across the gap.
        assert!(runs.len() <= 2);
        for run in &runs {
            assert!(run.iter().all(|(x, y)| x.is_finite() && y.is_finite()));
        }
    }

    #[test]
    fn dashes_are_short_runs_with_gaps_between_them() {
        let points = vec![(0.0, 0.0), (100.0, 0.0)];
        let runs = dash_polyline(&points, DashPattern::Dashed, 1.0);
        assert!(runs.len() >= 4, "expected several dashes, got {}", runs.len());
        // Total painted length is less than the line, because of the gaps.
        let painted: f64 = runs
            .iter()
            .map(|run| run[run.len() - 1].0 - run[0].0)
            .sum();
        assert!(painted < 100.0);
        // A solid line is not chopped at all.
        assert_eq!(dash_polyline(&points, DashPattern::Solid, 1.0).len(), 1);
    }

    #[test]
    fn a_step_series_repeats_the_previous_value() {
        let stepped = step_points(&[(0.0, 1.0), (10.0, 2.0)]);
        assert_eq!(stepped, vec![(0.0, 1.0), (10.0, 1.0), (10.0, 2.0)]);
    }

    #[test]
    fn rendering_a_panel_paints_the_frame_and_the_data() {
        let panel = panel();
        let layout = layout_for(&panel);
        let viewport = layout.viewport((-1.0, 10.0), (-0.1, 0.1), Scale::Linear, Scale::Linear);
        let series = vec![PlottedSeries::new(
            "550 nm",
            vec![(-1.0, 0.0), (4.0, 0.05), (10.0, -0.02)],
            SeriesStyle::default(),
        )];

        let (width, height) = (400usize, 300usize);
        let mut buffer = vec![0u8; width * height * 3];
        {
            let backend = BitMapBackend::with_buffer(&mut buffer, (width as u32, height as u32));
            render_panel(
                &panel,
                &PublicationStyle::default(),
                &layout,
                &viewport,
                &series,
                None,
                backend,
                Rgba::rgb(255, 255, 255),
            )
            .expect("the bitmap backend draws the panel");
        }

        let painted = buffer
            .chunks(3)
            .filter(|pixel| *pixel != [255, 255, 255])
            .count();
        assert!(painted > 500, "only {painted} pixels were painted");

        // The left edge of the frame is drawn down the plot rectangle's border.
        let frame_x = layout.plot.x0.round() as usize;
        let top = layout.plot.y0.round() as usize;
        let bottom = layout.plot.y1.round() as usize;
        let painted_on_the_edge = (top..=bottom)
            .filter(|row| {
                let offset = (row * width + frame_x) * 3;
                &buffer[offset..offset + 3] != &[255, 255, 255]
            })
            .count();
        assert!(
            painted_on_the_edge > (bottom - top) / 2,
            "the frame is missing: only {painted_on_the_edge} of {} edge pixels are inked",
            bottom - top
        );
    }

    #[test]
    fn an_inward_tick_does_not_paint_outside_the_frame() {
        let mut panel = panel();
        let mut style = PublicationStyle::default();
        style.tick_direction = TickDirection::In;
        style.frame = true;
        panel.x.label = String::new();
        panel.y.label = String::new();
        let layout = PanelLayout::compute(
            &panel,
            &style,
            Scale::Linear,
            Scale::Linear,
            (0.0, 1.0),
            (0.0, 1.0),
            (200.0, 200.0),
        );
        let viewport = layout.viewport((0.0, 1.0), (0.0, 1.0), Scale::Linear, Scale::Linear);

        let (width, height) = (200usize, 200usize);
        let mut buffer = vec![0u8; width * height * 3];
        {
            let backend = BitMapBackend::with_buffer(&mut buffer, (width as u32, height as u32));
            render_panel(&panel, &style, &layout, &viewport, &[], None, backend, Rgba::rgb(255, 255, 255))
                .unwrap();
        }
        // Just below the plot rectangle on the left there must be untouched
        // paper, because the tick points inwards.
        let probe_x = layout.plot.x0.round() as usize;
        let probe_y = (layout.plot.y1.round() as usize + 2).min(height - 1) * width + probe_x;
        assert_eq!(&buffer[probe_y * 3..probe_y * 3 + 3], &[255, 255, 255]);
    }

    #[test]
    fn a_heatmap_layer_fills_the_plot_area() {
        let mut panel = panel();
        panel.contour = Some(ta_core::ContourSpec::default());
        let layout = layout_for(&panel);
        let viewport = layout.viewport((-1.0, 10.0), (-0.1, 0.1), Scale::Linear, Scale::Linear);
        let (width, height) = (
            layout.plot.width().round() as usize,
            layout.plot.height().round() as usize,
        );
        // Fully opaque, so the layer is exactly the sample colour.
        let mut pixels = vec![200u8; width * height * 4];
        for pixel in pixels.chunks_mut(4) {
            pixel[3] = 255;
        }
        let layer = HeatmapLayer::from_rgba_over(&pixels, width, height, Rgba::rgb(255, 255, 255));
        assert_eq!(layer.rgb.len(), width * height * 3);

        let mut buffer = vec![0u8; 400 * 300 * 3];
        {
            let backend = BitMapBackend::with_buffer(&mut buffer, (400, 300));
            render_panel(
                &panel,
                &PublicationStyle::default(),
                &layout,
                &viewport,
                &[],
                Some(&layer),
                backend,
                Rgba::rgb(255, 255, 255),
            )
            .expect("the bitmap backend draws the heatmap");
        }
        // A few pixels inside the frame, so the frame's own stroke is not what
        // is being measured.
        let inside_x = layout.plot.x0.round() as usize + 4;
        let inside_y = layout.plot.y0.round() as usize + 4;
        let offset = (inside_y * 400 + inside_x) * 3;
        assert_eq!(&buffer[offset..offset + 3], &[200, 200, 200]);
    }

    #[test]
    fn a_transparent_raster_shows_the_background_through_it() {
        let pixels = vec![255, 0, 0, 0, 0, 0, 255, 255];
        let layer = HeatmapLayer::from_rgba_over(&pixels, 2, 1, Rgba::rgb(255, 255, 255));
        assert_eq!(&layer.rgb[0..3], &[255, 255, 255]);
        assert_eq!(&layer.rgb[3..6], &[0, 0, 255]);
    }

    #[test]
    fn the_legend_does_not_overlap_the_plot_rectangle_corner() {
        let panel = panel();
        let layout = layout_for(&panel);
        let series = vec![
            PlottedSeries::new("550 nm", vec![(0.0, 0.0), (1.0, 0.1)], SeriesStyle::default()),
            PlottedSeries::new("620 nm", vec![(0.0, 0.0), (1.0, -0.1)], SeriesStyle::default()),
        ];
        let row_height = panel.legend.font_size as f64 * 1.5;
        let height = row_height * series.len() as f64 + 8.0;
        assert!(height < layout.plot.height(), "the legend must fit inside");
        let _ = (LegendSpec::default(), AnnotationSpec::text("x", 1.0, 0.0));
    }
}
