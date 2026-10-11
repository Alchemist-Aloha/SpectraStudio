//! The plot viewport: the window onto the data, and the only thing that knows
//! how a physical coordinate becomes a pixel.
//!
//! Zoom, pan, box zoom, crosshair readout, hit testing and export all ask this
//! object, so two of them cannot disagree about where a number sits on screen.

use crate::scale::Scale;

/// A rectangle of physical coordinates drawn into a rectangle of pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct PlotViewport {
    /// Physical x range, low to high.
    x: (f64, f64),
    /// Physical y range, low to high.
    y: (f64, f64),
    x_scale: Scale,
    y_scale: Scale,
    /// The drawing area, in pixels. `(width, height)`.
    pixels: (f64, f64),
    /// Whether either axis had to move its range onto the part of the domain the
    /// scale can represent.
    domain_clamped: bool,
    reversed: (bool, bool),
}

impl PlotViewport {
    /// Builds a viewport, clamping any range the scales cannot represent.
    pub fn new(
        x_range: (f64, f64),
        y_range: (f64, f64),
        x_scale: Scale,
        y_scale: Scale,
        pixels: (f64, f64),
    ) -> Self {
        let ((x_low, x_high), x_clamped) = x_scale.clamp_domain(x_range.0, x_range.1);
        let ((y_low, y_high), y_clamped) = y_scale.clamp_domain(y_range.0, y_range.1);
        Self {
            x: (x_low, x_high),
            y: (y_low, y_high),
            x_scale,
            y_scale,
            pixels: (pixels.0.max(1.0), pixels.1.max(1.0)),
            domain_clamped: x_clamped || y_clamped,
            reversed: (false, false),
        }
    }

    pub fn with_reversed(mut self, x: bool, y: bool) -> Self {
        self.reversed = (x, y);
        self
    }

    pub fn x_range(&self) -> (f64, f64) {
        self.x
    }

    pub fn y_range(&self) -> (f64, f64) {
        self.y
    }

    pub fn x_scale(&self) -> Scale {
        self.x_scale
    }

    pub fn y_scale(&self) -> Scale {
        self.y_scale
    }

    pub fn pixels(&self) -> (f64, f64) {
        self.pixels
    }

    pub fn set_pixels(&mut self, width: f64, height: f64) {
        self.pixels = (width.max(1.0), height.max(1.0));
    }

    /// Whether the requested range extended outside what the scales can
    /// represent.
    pub fn domain_clamped(&self) -> bool {
        self.domain_clamped
    }

    /// Sets both ranges, clamping as the constructor does.
    pub fn set_ranges(&mut self, x_range: (f64, f64), y_range: (f64, f64)) {
        let ((x_low, x_high), x_clamped) = self.x_scale.clamp_domain(x_range.0, x_range.1);
        let ((y_low, y_high), y_clamped) = self.y_scale.clamp_domain(y_range.0, y_range.1);
        self.x = (x_low, x_high);
        self.y = (y_low, y_high);
        self.domain_clamped = x_clamped || y_clamped;
    }

    /// The physical x range in layout space, which is what a renderer needs.
    pub fn x_layout_range(&self) -> (f64, f64) {
        let (low, high) = self.x_scale.forward_range(self.x.0, self.x.1).unwrap_or((0.0, 1.0));
        (low, high)
    }

    pub fn y_layout_range(&self) -> (f64, f64) {
        let (low, high) = self.y_scale.forward_range(self.y.0, self.y.1).unwrap_or((0.0, 1.0));
        (low, high)
    }

    /// A physical coordinate pair to a pixel, or `None` where the map has no
    /// value (a logarithmic axis at or below zero).
    pub fn data_to_pixel(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let (x_low, x_high) = self.x_layout_range();
        let (y_low, y_high) = self.y_layout_range();
        let tx = self.x_scale.forward(x)?;
        let ty = self.y_scale.forward(y)?;
        let x_span = x_high - x_low;
        let y_span = y_high - y_low;
        if x_span <= 0.0 || y_span <= 0.0 {
            return None;
        }
        let px = (tx - x_low) / x_span * self.pixels.0;
        // Screen y grows downwards, so the physical maximum sits at the top.
        let py = (1.0 - (ty - y_low) / y_span) * self.pixels.1;
        Some((if self.reversed.0 { self.pixels.0 - px } else { px },
              if self.reversed.1 { self.pixels.1 - py } else { py }))
    }

    /// A pixel to a physical coordinate. Always defined inside the drawing area.
    pub fn pixel_to_data(&self, px: f64, py: f64) -> (f64, f64) {
        let px = if self.reversed.0 { self.pixels.0 - px } else { px };
        let py = if self.reversed.1 { self.pixels.1 - py } else { py };
        let (x_low, x_high) = self.x_layout_range();
        let (y_low, y_high) = self.y_layout_range();
        let tx = x_low + (px / self.pixels.0) * (x_high - x_low);
        let ty = y_low + (1.0 - py / self.pixels.1) * (y_high - y_low);
        (self.x_scale.inverse(tx), self.y_scale.inverse(ty))
    }

    /// Zooms about a pixel, keeping the physical coordinate under that pixel
    /// where it is. `factor < 1` zooms in, `> 1` zooms out.
    ///
    /// `factors` is `(x, y)` so an axis can be zoomed on its own.
    pub fn zoom_at(&mut self, pixel: (f64, f64), factors: (f64, f64)) {
        let pixel = (if self.reversed.0 { self.pixels.0 - pixel.0 } else { pixel.0 },
                     if self.reversed.1 { self.pixels.1 - pixel.1 } else { pixel.1 });
        self.x = self.zoom_axis(
            self.x,
            self.x_scale,
            pixel.0,
            self.pixels.0,
            factors.0,
        );
        // The pixel origin is the top, so the anchor for y needs flipping.
        self.y = self.zoom_axis(
            self.y,
            self.y_scale,
            self.pixels.1 - pixel.1,
            self.pixels.1,
            factors.1,
        );
        self.domain_clamped = false;
    }

    fn zoom_axis(
        &self,
        range: (f64, f64),
        scale: Scale,
        anchor_pixels: f64,
        total_pixels: f64,
        factor: f64,
    ) -> (f64, f64) {
        let (low, high) = scale.forward_range(range.0, range.1).unwrap_or(range);
        let span = high - low;
        if span <= 0.0 || !factor.is_finite() || factor <= 0.0 {
            return range;
        }
        let fraction = (anchor_pixels / total_pixels).clamp(0.0, 1.0);
        let anchor = low + fraction * span;
        let new_low = anchor - (anchor - low) * factor;
        let new_high = anchor + (high - anchor) * factor;
        scale.inverse_range(new_low, new_high)
    }

    /// Moves the window by a pixel delta, the way a drag does: dragging right
    /// moves the data right, which means the window moves left.
    pub fn pan_pixels(&mut self, dx: f64, dy: f64) {
        let dx = if self.reversed.0 { -dx } else { dx };
        let dy = if self.reversed.1 { -dy } else { dy };
        self.x = self.pan_axis(self.x, self.x_scale, -dx, self.pixels.0);
        self.y = self.pan_axis(self.y, self.y_scale, dy, self.pixels.1);
    }

    fn pan_axis(
        &self,
        range: (f64, f64),
        scale: Scale,
        delta_pixels: f64,
        total_pixels: f64,
    ) -> (f64, f64) {
        let (low, high) = scale.forward_range(range.0, range.1).unwrap_or(range);
        let span = high - low;
        let shift = delta_pixels / total_pixels * span;
        scale.inverse_range(low + shift, high + shift)
    }

    /// Zooms to the rectangle between two pixels, in either drag direction.
    pub fn box_zoom(&mut self, corner_a: (f64, f64), corner_b: (f64, f64)) -> bool {
        if (corner_a.0 - corner_b.0).abs() < 1.0 || (corner_a.1 - corner_b.1).abs() < 1.0 {
            return false;
        }
        let (x_a, y_a) = self.pixel_to_data(corner_a.0, corner_a.1);
        let (x_b, y_b) = self.pixel_to_data(corner_b.0, corner_b.1);
        if !(x_a.is_finite() && x_b.is_finite() && y_a.is_finite() && y_b.is_finite()) {
            return false;
        }
        self.set_ranges((x_a.min(x_b), x_a.max(x_b)), (y_a.min(y_b), y_a.max(y_b)));
        true
    }

    /// Zooms to the data range of a set of points, with a margin.
    pub fn fit_points(&mut self, points: impl IntoIterator<Item = (f64, f64)>, margin: f64) {
        let mut bounds: Option<(f64, f64, f64, f64)> = None;
        for (x, y) in points {
            if !x.is_finite() || !y.is_finite() {
                continue;
            }
            bounds = Some(match bounds {
                None => (x, x, y, y),
                Some((x0, x1, y0, y1)) => (x0.min(x), x1.max(x), y0.min(y), y1.max(y)),
            });
        }
        let Some((x_low, x_high, y_low, y_high)) = bounds else {
            return;
        };
        let pad = |low: f64, high: f64| {
            let span = high - low;
            let grow = if span == 0.0 {
                low.abs() * 0.05 + 1.0
            } else {
                span * margin
            };
            (low - grow, high + grow)
        };
        let (x_low, x_high) = pad(x_low, x_high);
        let (y_low, y_high) = pad(y_low, y_high);
        self.set_ranges((x_low, x_high), (y_low, y_high));
    }

    /// Whether a physical coordinate is inside the window.
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x.0 && x <= self.x.1 && y >= self.y.0 && y <= self.y.1
    }

    /// The index of the point nearest a pixel, within `tolerance` pixels.
    ///
    /// Distance is measured in pixels so a "near enough" answer means the same
    /// thing on a tall narrow axis and a short wide one.
    pub fn nearest_point(
        &self,
        points: &[(f64, f64)],
        pixel: (f64, f64),
        tolerance: f64,
    ) -> Option<usize> {
        let mut best: Option<(usize, f64)> = None;
        for (index, (x, y)) in points.iter().enumerate() {
            let Some((px, py)) = self.data_to_pixel(*x, *y) else {
                continue;
            };
            let distance = ((px - pixel.0).powi(2) + (py - pixel.1).powi(2)).sqrt();
            if distance <= tolerance && best.is_none_or(|(_, current)| distance < current) {
                best = Some((index, distance));
            }
        }
        best.map(|(index, _)| index)
    }

    /// Indices of the points inside a pixel rectangle, used by ROI selection
    /// and by the fit workbench's range picker.
    pub fn points_in_rect(
        &self,
        points: &[(f64, f64)],
        corner_a: (f64, f64),
        corner_b: (f64, f64),
    ) -> Vec<usize> {
        let x_low = corner_a.0.min(corner_b.0);
        let x_high = corner_a.0.max(corner_b.0);
        let y_low = corner_a.1.min(corner_b.1);
        let y_high = corner_a.1.max(corner_b.1);
        points
            .iter()
            .enumerate()
            .filter(|(_, (x, y))| match self.data_to_pixel(*x, *y) {
                Some((px, py)) => {
                    px >= x_low && px <= x_high && py >= y_low && py <= y_high
                }
                None => false,
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// A one-line description for the status bar.
    pub fn summary(&self) -> String {
        format!(
            "x {:.6}..{:.6} ({}), y {:.6}..{:.6} ({})",
            self.x.0,
            self.x.1,
            self.x_scale.label(),
            self.y.0,
            self.y.1,
            self.y_scale.label()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewport() -> PlotViewport {
        PlotViewport::new((-10.0, 10.0), (-1.0, 3.0), Scale::Linear, Scale::Linear, (200.0, 100.0))
    }

    #[test]
    fn reversed_axes_keep_drawing_hit_testing_zoom_and_pan_consistent() {
        let mut reversed = viewport().with_reversed(true, true);
        assert_eq!(reversed.data_to_pixel(-10.0, -1.0), Some((200.0, 0.0)));
        let pixel = (140.0, 30.0);
        let physical = reversed.pixel_to_data(pixel.0, pixel.1);
        assert_eq!(reversed.data_to_pixel(physical.0, physical.1), Some(pixel));
        reversed.zoom_at(pixel, (0.5, 0.5));
        let zoomed = reversed.pixel_to_data(pixel.0, pixel.1);
        assert!((physical.0 - zoomed.0).abs() < 1e-9);
        assert!((physical.1 - zoomed.1).abs() < 1e-9);
        let before = reversed.data_to_pixel(physical.0, physical.1).unwrap();
        reversed.pan_pixels(10.0, 5.0);
        let after = reversed.data_to_pixel(physical.0, physical.1).unwrap();
        assert!((after.0 - before.0 - 10.0).abs() < 1e-9);
        assert!((after.1 - before.1 - 5.0).abs() < 1e-9);
    }

    #[test]
    fn the_corners_map_to_the_corners_of_the_drawing_area() {
        let viewport = viewport();
        assert_eq!(viewport.data_to_pixel(-10.0, 3.0), Some((0.0, 0.0)));
        assert_eq!(viewport.data_to_pixel(10.0, -1.0), Some((200.0, 100.0)));
        assert_eq!(viewport.data_to_pixel(0.0, 1.0), Some((100.0, 50.0)));
    }

    #[test]
    fn pixel_and_data_coordinates_round_trip() {
        let viewport = viewport();
        for (px, py) in [(0.0, 0.0), (100.0, 50.0), (199.0, 99.0), (33.0, 77.0)] {
            let (x, y) = viewport.pixel_to_data(px, py);
            let (back_x, back_y) = viewport.data_to_pixel(x, y).unwrap();
            assert!((back_x - px).abs() < 1e-9, "{px} → {back_x}");
            assert!((back_y - py).abs() < 1e-9, "{py} → {back_y}");
        }
    }

    #[test]
    fn zooming_keeps_the_coordinate_under_the_pointer_still() {
        let mut viewport = viewport();
        let anchor_pixel = (140.0, 30.0);
        let before = viewport.pixel_to_data(anchor_pixel.0, anchor_pixel.1);
        viewport.zoom_at(anchor_pixel, (0.5, 0.5));
        let after = viewport.pixel_to_data(anchor_pixel.0, anchor_pixel.1);

        assert!((before.0 - after.0).abs() < 1e-9, "{before:?} → {after:?}");
        assert!((before.1 - after.1).abs() < 1e-9, "{before:?} → {after:?}");
        // And the window really did get smaller.
        assert!(viewport.x_range().1 - viewport.x_range().0 < 20.0);
        assert!(viewport.y_range().1 - viewport.y_range().0 < 4.0);
    }

    #[test]
    fn zooming_one_axis_leaves_the_other_alone() {
        let mut viewport = viewport();
        viewport.zoom_at((100.0, 50.0), (0.5, 1.0));
        assert!(viewport.x_range().1 - viewport.x_range().0 < 20.0);
        assert_eq!(viewport.y_range(), (-1.0, 3.0));
    }

    #[test]
    fn the_pointer_anchor_holds_on_a_symmetric_logarithmic_axis_too() {
        let mut viewport = PlotViewport::new(
            (-100.0, 1000.0),
            (0.0, 1.0),
            Scale::SymmetricLogarithmic { linear_width: 1.0 },
            Scale::Linear,
            (400.0, 200.0),
        );
        let anchor = (250.0, 100.0);
        let before = viewport.pixel_to_data(anchor.0, anchor.1);
        viewport.zoom_at(anchor, (0.25, 0.25));
        let after = viewport.pixel_to_data(anchor.0, anchor.1);
        assert!((before.0 - after.0).abs() <= before.0.abs() * 1e-9 + 1e-12);
    }

    #[test]
    fn panning_moves_the_window_opposite_to_the_drag() {
        let mut viewport = viewport();
        let (before, _) = viewport.pixel_to_data(0.0, 0.0);
        // Drag the data 20 pixels to the right.
        viewport.pan_pixels(20.0, 0.0);
        assert!(viewport.x_range().0 < before, "the window should have moved left");
        // The range keeps its width.
        let span = viewport.x_range().1 - viewport.x_range().0;
        assert!((span - 20.0).abs() < 1e-9);
    }

    #[test]
    fn dragging_down_shows_earlier_y_values() {
        let mut viewport = viewport();
        let (_, before) = viewport.pixel_to_data(0.0, 0.0);
        viewport.pan_pixels(0.0, 20.0);
        let (_, after) = viewport.pixel_to_data(0.0, 0.0);
        assert!(after > before, "dragging down should raise the visible y");
    }

    #[test]
    fn a_box_zoom_shows_exactly_the_dragged_rectangle() {
        let mut viewport = viewport();
        let corner_a = (50.0, 25.0);
        let corner_b = (150.0, 75.0);
        let expected_low = viewport.pixel_to_data(50.0, 75.0);
        let expected_high = viewport.pixel_to_data(150.0, 25.0);

        assert!(viewport.box_zoom(corner_a, corner_b));
        assert!((viewport.x_range().0 - expected_low.0).abs() < 1e-9);
        assert!((viewport.x_range().1 - expected_high.0).abs() < 1e-9);
        assert!((viewport.y_range().0 - expected_low.1).abs() < 1e-9);
        assert!((viewport.y_range().1 - expected_high.1).abs() < 1e-9);
    }

    #[test]
    fn a_box_zoom_dragged_backwards_selects_the_same_rectangle() {
        let mut forwards = viewport();
        let mut backwards = viewport();
        forwards.box_zoom((50.0, 25.0), (150.0, 75.0));
        backwards.box_zoom((150.0, 75.0), (50.0, 25.0));
        assert_eq!(forwards.x_range(), backwards.x_range());
        assert_eq!(forwards.y_range(), backwards.y_range());
    }

    #[test]
    fn a_click_sized_box_zoom_is_ignored_rather_than_zooming_to_nothing() {
        let mut viewport = viewport();
        assert!(!viewport.box_zoom((50.0, 25.0), (50.5, 25.5)));
        assert_eq!(viewport.x_range(), (-10.0, 10.0));
    }

    #[test]
    fn fitting_to_points_adds_a_margin_and_ignores_missing_values() {
        let mut viewport = viewport();
        viewport.fit_points(
            [(-2.0, 0.0), (2.0, 1.0), (f64::NAN, 5.0), (0.0, f64::INFINITY)],
            0.1,
        );
        let (x_low, x_high) = viewport.x_range();
        assert!(x_low < -2.0 && x_high > 2.0);
        let (y_low, y_high) = viewport.y_range();
        assert!(y_low < 0.0 && y_high > 1.0);
    }

    #[test]
    fn a_single_point_still_produces_a_non_degenerate_window() {
        let mut viewport = viewport();
        viewport.fit_points([(5.0, 1.0)], 0.1);
        let (x_low, x_high) = viewport.x_range();
        let (y_low, y_high) = viewport.y_range();
        assert!(x_high > x_low);
        assert!(y_high > y_low);
    }

    #[test]
    fn the_nearest_point_is_found_only_within_the_tolerance() {
        let viewport = viewport();
        let points = vec![(-5.0, 1.0), (0.0, 1.0), (5.0, 1.0)];
        let (px, py) = viewport.data_to_pixel(0.2, 1.0).unwrap();
        assert_eq!(viewport.nearest_point(&points, (px, py), 10.0), Some(1));
        assert_eq!(viewport.nearest_point(&points, (0.0, 0.0), 10.0), None);
    }

    #[test]
    fn a_rectangular_selection_returns_the_points_inside_it() {
        let viewport = viewport();
        let points = vec![(-5.0, 1.0), (0.0, 1.0), (5.0, 1.0)];
        let (x0, y0) = viewport.data_to_pixel(-6.0, 2.0).unwrap();
        let (x1, y1) = viewport.data_to_pixel(1.0, 0.0).unwrap();
        assert_eq!(
            viewport.points_in_rect(&points, (x0, y0), (x1, y1)),
            vec![0, 1]
        );
    }

    #[test]
    fn a_logarithmic_axis_reports_that_its_domain_was_clamped() {
        let viewport = PlotViewport::new(
            (-5.0, 100.0),
            (0.0, 1.0),
            Scale::Logarithmic,
            Scale::Linear,
            (100.0, 100.0),
        );
        assert!(viewport.domain_clamped());
        assert!(viewport.x_range().0 > 0.0);
        assert!(viewport.data_to_pixel(1.0, 0.5).is_some());
        // A coordinate the scale cannot represent has no pixel at all.
        assert_eq!(viewport.data_to_pixel(-1.0, 0.5), None);
    }

    #[test]
    fn resizing_the_drawing_area_keeps_the_same_window() {
        let mut viewport = viewport();
        let (x, y) = viewport.pixel_to_data(100.0, 50.0);
        viewport.set_pixels(400.0, 200.0);
        let (px, py) = viewport.data_to_pixel(x, y).unwrap();
        assert!((px - 200.0).abs() < 1e-9);
        assert!((py - 100.0).abs() < 1e-9);
    }
}
