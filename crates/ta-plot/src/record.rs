//! A Plotters backend that records instead of painting.
//!
//! Every primitive Plotters can emit is captured, so a recorded frame is a
//! complete description of the drawing and a renderer can replay it without
//! re-running the layout code.

use crate::commands::{CommandBuffer, DrawCommand, HAlign, StrokeStyle, TextAnchor, VAlign};
use plotters::style::text_anchor::{HPos, VPos};
use plotters_backend::{
    BackendColor, BackendCoord, BackendStyle, BackendTextStyle, DrawingBackend,
    DrawingErrorKind, FontTransform,
};
use std::io;
use ta_core::Rgba;

/// Records drawing calls into a [`CommandBuffer`].
///
/// The buffer is shared through an `Rc`, because a drawing area takes its
/// backend by value and offers no way to hand it back: the recorder itself is
/// the handle the caller keeps.
#[derive(Clone)]
pub struct CommandRecorder {
    buffer: std::rc::Rc<std::cell::RefCell<CommandBuffer>>,
}

impl CommandRecorder {
    pub fn new(size: (u32, u32)) -> Self {
        Self {
            buffer: std::rc::Rc::new(std::cell::RefCell::new(CommandBuffer::new(size))),
        }
    }

    /// A copy of the frame recorded so far.
    pub fn snapshot(&self) -> CommandBuffer {
        self.buffer.borrow().clone()
    }

    pub fn len(&self) -> usize {
        self.buffer.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.borrow().is_empty()
    }

    fn push(&mut self, command: DrawCommand) {
        self.buffer.borrow_mut().push(command);
    }
}

fn color_of(style: &impl BackendStyle) -> Rgba {
    let color: BackendColor = style.color();
    let (r, g, b) = color.rgb;
    Rgba::rgba(
        r,
        g,
        b,
        (color.alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

fn stroke_of(style: &impl BackendStyle) -> StrokeStyle {
    StrokeStyle::new(color_of(style), style.stroke_width())
}

fn anchor_of(pos: plotters_backend::text_anchor::Pos) -> TextAnchor {
    TextAnchor {
        horizontal: match pos.h_pos {
            HPos::Left => HAlign::Left,
            HPos::Center => HAlign::Center,
            HPos::Right => HAlign::Right,
        },
        vertical: match pos.v_pos {
            VPos::Top => VAlign::Top,
            VPos::Center => VAlign::Center,
            VPos::Bottom => VAlign::Bottom,
        },
    }
}

fn rotation_of(transform: FontTransform) -> u16 {
    match transform {
        FontTransform::None => 0,
        FontTransform::Rotate90 => 90,
        FontTransform::Rotate180 => 180,
        FontTransform::Rotate270 => 270,
    }
}

impl DrawingBackend for CommandRecorder {
    type ErrorType = io::Error;

    fn get_size(&self) -> (u32, u32) {
        self.buffer.borrow().size
    }

    fn ensure_prepared(&mut self) -> Result<(), DrawingErrorKind<Self::ErrorType>> {
        Ok(())
    }

    fn present(&mut self) -> Result<(), DrawingErrorKind<Self::ErrorType>> {
        Ok(())
    }

    fn draw_pixel(
        &mut self,
        point: BackendCoord,
        color: BackendColor,
    ) -> Result<(), DrawingErrorKind<Self::ErrorType>> {
        let (r, g, b) = color.rgb;
        self.push(DrawCommand::Pixel {
            point,
            color: Rgba::rgba(
                r,
                g,
                b,
                (color.alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
            ),
        });
        Ok(())
    }

    fn draw_line<S: BackendStyle>(
        &mut self,
        from: BackendCoord,
        to: BackendCoord,
        style: &S,
    ) -> Result<(), DrawingErrorKind<Self::ErrorType>> {
        if style.color().alpha == 0.0 {
            return Ok(());
        }
        self.push(DrawCommand::Line {
            from,
            to,
            style: stroke_of(style),
        });
        Ok(())
    }

    fn draw_rect<S: BackendStyle>(
        &mut self,
        upper_left: BackendCoord,
        bottom_right: BackendCoord,
        style: &S,
        fill: bool,
    ) -> Result<(), DrawingErrorKind<Self::ErrorType>> {
        if style.color().alpha == 0.0 {
            return Ok(());
        }
        self.push(DrawCommand::Rect {
            corners: [upper_left, bottom_right],
            style: stroke_of(style),
            fill,
        });
        Ok(())
    }

    fn draw_circle<S: BackendStyle>(
        &mut self,
        center: BackendCoord,
        radius: u32,
        style: &S,
        fill: bool,
    ) -> Result<(), DrawingErrorKind<Self::ErrorType>> {
        if style.color().alpha == 0.0 {
            return Ok(());
        }
        self.push(DrawCommand::Circle {
            center,
            radius,
            style: stroke_of(style),
            fill,
        });
        Ok(())
    }

    fn draw_path<S: BackendStyle, I: IntoIterator<Item = BackendCoord>>(
        &mut self,
        path: I,
        style: &S,
    ) -> Result<(), DrawingErrorKind<Self::ErrorType>> {
        if style.color().alpha == 0.0 {
            return Ok(());
        }
        let points: Vec<BackendCoord> = path.into_iter().collect();
        if points.len() < 2 {
            return Ok(());
        }
        self.push(DrawCommand::Path {
            points,
            style: stroke_of(style),
        });
        Ok(())
    }

    fn fill_polygon<S: BackendStyle, I: IntoIterator<Item = BackendCoord>>(
        &mut self,
        vertices: I,
        style: &S,
    ) -> Result<(), DrawingErrorKind<Self::ErrorType>> {
        if style.color().alpha == 0.0 {
            return Ok(());
        }
        let vertices: Vec<BackendCoord> = vertices.into_iter().collect();
        if vertices.len() < 3 {
            return Ok(());
        }
        self.push(DrawCommand::Polygon {
            vertices,
            style: stroke_of(style),
        });
        Ok(())
    }

    fn draw_text<TStyle: BackendTextStyle>(
        &mut self,
        text: &str,
        style: &TStyle,
        pos: BackendCoord,
    ) -> Result<(), DrawingErrorKind<Self::ErrorType>> {
        let color: BackendColor = style.color();
        if color.alpha == 0.0 {
            return Ok(());
        }
        let (r, g, b) = color.rgb;
        self.push(DrawCommand::Text {
            text: text.to_string(),
            position: pos,
            size: style.size(),
            family: style.family().as_str().to_string(),
            anchor: anchor_of(style.anchor()),
            color: Rgba::rgba(
                r,
                g,
                b,
                (color.alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
            ),
            rotation: rotation_of(style.transform()),
        });
        Ok(())
    }

    fn estimate_text_size<TStyle: BackendTextStyle>(
        &self,
        text: &str,
        style: &TStyle,
    ) -> Result<(u32, u32), DrawingErrorKind<Self::ErrorType>> {
        // No font engine here, so the same estimate the layout uses; a recorder
        // that asked a font would make the captured frame disagree with the
        // layout that produced it.
        let size = style.size();
        let width = (text.chars().count() as f64 * size * 0.6).round() as u32;
        let height = (size * 1.4).round() as u32;
        Ok((width.max(1), height.max(1)))
    }

    fn blit_bitmap(
        &mut self,
        pos: BackendCoord,
        (width, height): (u32, u32),
        src: &[u8],
    ) -> Result<(), DrawingErrorKind<Self::ErrorType>> {
        let expected = (width as usize) * (height as usize) * 3;
        if src.len() < expected {
            return Err(DrawingErrorKind::DrawingError(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "a {width}×{height} bitmap needs {expected} bytes, got {}",
                    src.len()
                ),
            )));
        }
        self.push(DrawCommand::Bitmap {
            position: pos,
            size: (width, height),
            pixels: src[..expected].to_vec(),
        });
        Ok(())
    }
}

/// Records one panel into a command buffer.
///
/// The single entry point the interactive layer needs: it never has to know
/// that Plotters is behind it.
#[allow(clippy::too_many_arguments)]
pub fn record_panel(
    panel: &ta_core::PanelModel,
    style: &ta_core::PublicationStyle,
    layout: &crate::render::PanelLayout,
    viewport: &crate::viewport::PlotViewport,
    series: &[crate::render::PlottedSeries],
    heatmap: Option<&crate::render::HeatmapLayer>,
    size: (u32, u32),
) -> CommandBuffer {
    use plotters::drawing::IntoDrawingArea;
    let recorder = CommandRecorder::new(size);
    let root = recorder.clone().into_drawing_area();
    let _ = crate::render::draw_panel(&root, panel, style, layout, viewport, series, heatmap);
    recorder.snapshot()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{
        draw_panel, HeatmapLayer, PanelLayout, PlottedSeries,
    };
    use crate::scale::Scale;
    use plotters::coord::Shift;
    use plotters::drawing::{DrawingArea, IntoDrawingArea};
    use ta_core::{AxisModel, PanelModel, PublicationStyle, SeriesStyle};

    fn panel() -> PanelModel {
        let mut panel = PanelModel::new("Kinetics");
        panel.x = AxisModel {
            label: "Delay".into(),
            unit: Some("ps".into()),
            ..AxisModel::default()
        };
        panel.y = AxisModel {
            label: "ΔOD".into(),
            ..AxisModel::default()
        };
        panel
    }

    #[test]
    fn a_recorded_panel_captures_text_lines_and_the_frame() {
        let panel = panel();
        let style = PublicationStyle::default();
        let size = (400.0, 300.0);
        let layout = PanelLayout::compute(
            &panel,
            &style,
            Scale::Linear,
            Scale::Linear,
            (-1.0, 10.0),
            (-0.1, 0.1),
            size,
        );
        let viewport = layout.viewport((-1.0, 10.0), (-0.1, 0.1), Scale::Linear, Scale::Linear);
        let series = vec![PlottedSeries::new(
            "550 nm",
            vec![(-1.0, 0.0), (2.0, 0.05), (10.0, -0.02)],
            SeriesStyle::default(),
        )];

        let recorder = CommandRecorder::new((400, 300));
        let root: DrawingArea<CommandRecorder, Shift> = recorder.clone().into_drawing_area();
        draw_panel(&root, &panel, &style, &layout, &viewport, &series, None)
            .expect("recording cannot fail");
        let buffer = recorder.snapshot();

        assert!(buffer.len() > 10, "only {} commands", buffer.len());
        assert!(buffer.count_of("text") >= 5, "labels are missing");
        assert!(buffer.count_of("path") >= 1, "the series is missing");
        assert!(buffer.count_of("rect") >= 1, "the frame is missing");
        // The axis captions and the panel title are in the frame.
        let texts = buffer.texts();
        assert!(texts.contains(&"ΔOD"));
        assert!(texts.contains(&"Delay (ps)"));
        assert!(texts.contains(&"Kinetics"));
        // Nothing was rasterised per pixel: a recorded line stays a line.
        assert_eq!(buffer.count_of("pixel"), 0);
    }

    #[test]
    fn a_recorded_heatmap_carries_the_image_once() {
        let mut panel = panel();
        panel.contour = Some(ta_core::ContourSpec::default());
        let style = PublicationStyle::default();
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
        let width = layout.plot.width().round() as usize;
        let height = layout.plot.height().round() as usize;
        let mut pixels = vec![128u8; width * height * 4];
        for pixel in pixels.chunks_mut(4) {
            pixel[3] = 255;
        }
        let layer = HeatmapLayer::from_rgba_over(&pixels, width, height, ta_core::Rgba::rgb(255, 255, 255));

        let recorder = CommandRecorder::new((200, 200));
        let root: DrawingArea<CommandRecorder, Shift> = recorder.clone().into_drawing_area();
        draw_panel(&root, &panel, &style, &layout, &viewport, &[], Some(&layer))
            .expect("recording cannot fail");
        let buffer = recorder.snapshot();

        assert_eq!(buffer.count_of("bitmap"), 1);
        assert_eq!(buffer.bitmap_bytes(), width * height * 3);
        // The bitmap is painted first and the colour bar after it.
        let order = buffer.in_paint_order();
        assert_eq!(order[0].kind(), "bitmap");
    }

    #[test]
    fn a_truncated_bitmap_is_an_error_rather_than_a_half_image() {
        let mut recorder = CommandRecorder::new((10, 10));
        let result = recorder.blit_bitmap((0, 0), (4, 4), &[0u8; 10]);
        assert!(result.is_err());
    }

    #[test]
    fn a_transparent_primitive_is_not_recorded_at_all() {
        let mut recorder = CommandRecorder::new((10, 10));
        let invisible = plotters::style::ShapeStyle {
            color: plotters::style::RGBAColor(0, 0, 0, 0.0),
            filled: false,
            stroke_width: 1,
        };
        recorder.draw_line((0, 0), (5, 5), &invisible).unwrap();
        recorder.draw_rect((0, 0), (5, 5), &invisible, false).unwrap();
        assert!(recorder.is_empty());
    }
}
