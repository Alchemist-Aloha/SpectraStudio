//! Replaying a recorded frame through GPUI.
//!
//! This is the second backend for the one drawing description in
//! `ta_plot::commands`: the same code that produced an SVG file is painted here,
//! so what is on screen and what lands in the exported figure cannot drift.

use gpui_kit::{
    fill, point, px, App, Bounds, Corners, Hsla, Pixels, Point, RenderImage, TextAlign, TextRun,
    Window,
};
use std::sync::Arc;
use ta_core::Rgba as InkColor;
use ta_plot::commands::{DrawCommand, HAlign, StrokeStyle, VAlign};

/// How a recorded frame maps onto the element it is painted into.
#[derive(Clone, Copy, Debug)]
pub struct ReplayTransform {
    pub origin: Point<Pixels>,
    /// Recorded units to device pixels. One recorded unit is one point in the
    /// exported figure, so this is the panel's density, not a zoom.
    pub scale: f32,
    /// The plot rectangle, in recorded units. A caption drawn up the margin has
    /// to know how much room it has.
    pub plot: (f64, f64, f64, f64),
}

impl ReplayTransform {
    pub fn new(origin: Point<Pixels>, scale: f32) -> Self {
        Self {
            origin,
            scale: scale.max(0.01),
            plot: (0.0, 0.0, 0.0, 0.0),
        }
    }

    /// The same, told where the plot rectangle is.
    pub fn with_plot(mut self, plot: (f64, f64, f64, f64)) -> Self {
        self.plot = plot;
        self
    }

    fn place(&self, (x, y): (i32, i32)) -> Point<Pixels> {
        self.map((x as f64, y as f64))
    }

    /// A point of the recorded figure, in the element's own pixels.
    ///
    /// The overlay uses this as well as the buffer, so a cursor sits on the
    /// curve it was placed on whatever the figure is zoomed to.
    pub fn map(&self, (x, y): (f64, f64)) -> Point<Pixels> {
        point(
            self.origin.x + px(x as f32 * self.scale),
            self.origin.y + px(y as f32 * self.scale),
        )
    }

    /// A length of the recorded figure, in the element's own pixels.
    pub fn length(&self, value: f32) -> Pixels {
        px(value * self.scale)
    }
}

fn to_hsla(color: InkColor) -> Hsla {
    // Convert sRGB to HSLA through the same path GPUI uses for its own colours.
    let rgba = gpui_kit::Rgba {
        r: color.r as f32 / 255.0,
        g: color.g as f32 / 255.0,
        b: color.b as f32 / 255.0,
        a: color.a as f32 / 255.0,
    };
    let mut hsla_value: Hsla = rgba.into();
    hsla_value.a = color.a as f32 / 255.0;
    hsla_value
}

/// Paints a recorded frame.
pub fn paint_buffer(
    window: &mut Window,
    cx: &mut App,
    buffer: &ta_plot::CommandBuffer,
    transform: ReplayTransform,
) {
    for command in buffer.in_paint_order() {
        paint_command(window, cx, command, transform);
    }
}

fn paint_command(
    window: &mut Window,
    cx: &mut App,
    command: &DrawCommand,
    transform: ReplayTransform,
) {
    match command {
        DrawCommand::Pixel { point: at, color } => {
            let origin = transform.place(*at);
            window.paint_quad(fill(
                Bounds::new(origin, gpui_kit::size(px(transform.scale), px(transform.scale))),
                to_hsla(*color),
            ));
        }
        DrawCommand::Line { from, to, style } => {
            let mut builder = path_builder(*style, transform.scale);
            builder.move_to(transform.place(*from));
            builder.line_to(transform.place(*to));
            if let Ok(path) = builder.build() {
                window.paint_path(path, to_hsla(style.color));
            }
        }
        DrawCommand::Rect {
            corners,
            style,
            fill: filled,
        } => {
            let a = transform.place(corners[0]);
            let b = transform.place(corners[1]);
            let bounds = Bounds::from_corners(a, b);
            if *filled {
                window.paint_quad(fill(bounds, to_hsla(style.color)));
            } else {
                let mut builder = path_builder(*style, transform.scale);
                builder.add_polygon(
                    &[
                        point(bounds.origin.x, bounds.origin.y),
                        point(bounds.origin.x + bounds.size.width, bounds.origin.y),
                        point(
                            bounds.origin.x + bounds.size.width,
                            bounds.origin.y + bounds.size.height,
                        ),
                        point(bounds.origin.x, bounds.origin.y + bounds.size.height),
                    ],
                    true,
                );
                if let Ok(path) = builder.build() {
                    window.paint_path(path, to_hsla(style.color));
                }
            }
        }
        DrawCommand::Circle {
            center,
            radius,
            style,
            fill: filled,
        } => {
            let at = transform.place(*center);
            let r = *radius as f32 * transform.scale;
            let mut builder = path_builder(*style, transform.scale);
            add_circle(&mut builder, at, r);
            if let Ok(path) = builder.build() {
                if *filled {
                    window.paint_path(path, to_hsla(style.color));
                } else {
                    window.paint_path(path, to_hsla(style.color));
                }
            }
        }
        DrawCommand::Path { points, style } => {
            if points.len() < 2 {
                return;
            }
            let mut builder = path_builder(*style, transform.scale);
            let mut mapped = points.iter().map(|p| transform.place(*p));
            if let Some(first) = mapped.next() {
                builder.move_to(first);
            }
            for next in mapped {
                builder.line_to(next);
            }
            if let Ok(path) = builder.build() {
                window.paint_path(path, to_hsla(style.color));
            }
        }
        DrawCommand::Polygon { vertices, style } => {
            if vertices.len() < 3 {
                return;
            }
            let mapped: Vec<Point<Pixels>> = vertices.iter().map(|p| transform.place(*p)).collect();
            let mut builder = gpui_kit::PathBuilder::fill();
            builder.add_polygon(&mapped, true);
            if let Ok(path) = builder.build() {
                window.paint_path(path, to_hsla(style.color));
            }
        }
        DrawCommand::Text {
            text,
            position,
            size,
            anchor,
            color,
            rotation,
            ..
        } => {
            paint_text(window, cx, text, *position, *size, *anchor, *color, *rotation, transform);
        }
        DrawCommand::Bitmap {
            position,
            size,
            pixels,
        } => {
            let Some(image) = bitmap_image(pixels, *size) else {
                return;
            };
            let origin = transform.place(*position);
            let bounds = Bounds::new(
                origin,
                gpui_kit::size(
                    px(size.0 as f32 * transform.scale),
                    px(size.1 as f32 * transform.scale),
                ),
            );
            // A bitmap that fails to paint is a silently empty plot, so the
            // result is reported rather than dropped.
            if let Err(error) = window.paint_image(bounds, bounds, Corners::default(), image, 0, false)
            {
                if std::env::var_os("SPECTRA_TRACE").is_some() {
                    eprintln!("paint_image failed: {error}");
                }
            }
        }
    }
}

fn path_builder(style: StrokeStyle, scale: f32) -> gpui_kit::PathBuilder {
    gpui_kit::PathBuilder::stroke(px(style.width.max(1) as f32 * scale))
}

/// Four cubic Béziers, which is how a circle is drawn without an arc primitive.
fn add_circle(builder: &mut gpui_kit::PathBuilder, centre: Point<Pixels>, radius: f32) {
    const KAPPA: f32 = 0.552_284_75;
    let r = radius.max(0.5);
    let k = r * KAPPA;
    let (x, y) = (centre.x, centre.y);
    let right = point(x + px(r), y);
    let bottom = point(x, y + px(r));
    let left = point(x - px(r), y);
    let top = point(x, y - px(r));

    builder.move_to(right);
    builder.cubic_bezier_to(
        bottom,
        point(x + px(k), y + px(r)),
        point(x + px(r), y + px(k)),
    );
    builder.cubic_bezier_to(left, point(x - px(k), y + px(r)), point(x - px(r), y + px(k)));
    builder.cubic_bezier_to(top, point(x - px(k), y - px(r)), point(x - px(r), y - px(k)));
    builder.cubic_bezier_to(right, point(x + px(k), y - px(r)), point(x + px(r), y - px(k)));
    builder.close();
}

#[allow(clippy::too_many_arguments)]
fn paint_text(
    window: &mut Window,
    cx: &mut App,
    text: &str,
    position: (i32, i32),
    size: f64,
    anchor: ta_plot::TextAnchor,
    color: InkColor,
    rotation: u16,
    transform: ReplayTransform,
) {
    if text.is_empty() {
        return;
    }
    let font_size = px(size as f32 * transform.scale);
    let ink = to_hsla(color);
    let origin = transform.place(position);

    if rotation % 360 == 0 {
        let line = shape(window, text, font_size, ink);
        let width = line.width();
        let height = font_size;
        let x = match anchor.horizontal {
            HAlign::Left => origin.x,
            HAlign::Center => origin.x - width / 2.0,
            HAlign::Right => origin.x - width,
        };
        let y = match anchor.vertical {
            VAlign::Top => origin.y,
            VAlign::Center => origin.y - height / 2.0,
            VAlign::Bottom => origin.y - height,
        };
        let _ = line.paint(point(x, y), font_size, TextAlign::Left, None, window, cx);
        return;
    }

    // A caption that is vertical in the figure stays vertical on screen: the
    // words are handed to the SVG renderer as the same rotated text the export
    // writes, and painted through its own transformation. GPUI's shaped lines
    // cannot be turned, but its SVG renderer can turn anything, and it resolves
    // the same system font.
    if paint_rotated_svg_text(window, cx, text, font_size, ink, origin, rotation, transform) {
        return;
    }
    // Should the SVG renderer be unavailable, the caption is still set upright
    // in the margin rather than dropped.
    let line = shape(window, text, font_size, ink);
    let y = match anchor.vertical {
        VAlign::Top => origin.y,
        VAlign::Center => origin.y - font_size / 2.0,
        VAlign::Bottom => origin.y - font_size,
    };
    let x = origin.x - line.width();
    let _ = line.paint(point(x, y), font_size, TextAlign::Left, None, window, cx);
}

/// Paints a caption turned by `rotation` degrees, through the SVG renderer.
///
/// Returns whether it was drawn: the caller keeps an upright fallback for the
/// case where the renderer cannot resolve a font.
#[allow(clippy::too_many_arguments)]
fn paint_rotated_svg_text(
    window: &mut Window,
    cx: &mut App,
    text: &str,
    font_size: Pixels,
    color: Hsla,
    origin: Point<Pixels>,
    rotation: u16,
    transform: ReplayTransform,
) -> bool {
    use gpui_kit::{size, Bounds, SharedString, TransformationMatrix};

    // The line box the text needs, measured by the same shaper the rest of the
    // frame uses.
    let measured = shape(window, text, font_size, color);
    let width = measured.width();
    let ascent = font_size * 0.8;
    let descent = font_size * 0.25;

    // The SVG is written the way the exporter writes it: the glyphs laid out
    // along a baseline that is then turned, so the file and the screen turn the
    // same string by the same angle.
    // Which way the text reads: a y axis title climbs the margin, a colour bar
    // label comes down the page. The angle matches the transform the exporter
    // writes for the same caption.
    let angle = match rotation % 360 {
        90 => 90.0,
        _ => -90.0,
    };
    let box_width = ascent + descent;
    let box_height = width;
    // Written with string building rather than a format literal: the SVG has
    // more quotation marks than a readable format string wants. The text is
    // centred in its box and the box is centred on the anchor, so where the
    // caption lands depends on the anchor alone.
    let centre_x = ascent.as_f32();
    let centre_y = box_height.as_f32() / 2.0;
    let mut svg = String::new();
    svg.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">",
        w = box_width.as_f32(),
        h = box_height.as_f32(),
    ));
    svg.push_str(&format!(
        "<text x=\"{x}\" y=\"{y}\" text-anchor=\"middle\" font-family=\"Helvetica\" font-size=\"{size}\" fill=\"#ffffff\" transform=\"rotate({angle} {x} {y})\">{escaped}</text>",
        x = centre_x,
        y = centre_y,
        size = font_size.as_f32(),
        angle = angle,
        escaped = escape_svg_text(text),
    ));
    svg.push_str("</svg>");
    // A rotated caption is centred on its anchor in both directions, which is
    // the same rule the exporter follows.
    let bounds = Bounds::new(
        point(
            origin.x - transform.length(box_width.as_f32()) / 2.0,
            origin.y - box_height / 2.0,
        ),
        size(
            transform.length(box_width.as_f32()),
            transform.length(box_height.as_f32()),
        ),
    );
    let key = SharedString::from(format!(
        "caption:{angle}:{}:{text}",
        font_size.as_f32().round() as i32
    ));
    window
        .paint_svg(bounds, key, Some(svg.as_bytes()), TransformationMatrix::unit(), color, cx)
        .is_ok()
}

/// The five characters an XML document cannot hold as themselves.
fn escape_svg_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub(crate) fn shape(
    window: &mut Window,
    text: &str,
    font_size: Pixels,
    color: Hsla,
) -> gpui_kit::ShapedLine {
    let run = TextRun {
        len: text.len(),
        font: window.text_style().font(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(text.to_string().into(), font_size, &[run], None)
}

/// An RGB buffer as the BGRA image GPUI's atlas expects.
fn bitmap_image(pixels: &[u8], (width, height): (u32, u32)) -> Option<Arc<RenderImage>> {
    let expected = width as usize * height as usize * 3;
    if pixels.len() < expected || width == 0 || height == 0 {
        return None;
    }
    let mut bgra = Vec::with_capacity(width as usize * height as usize * 4);
    for chunk in pixels[..expected].chunks_exact(3) {
        bgra.push(chunk[2]);
        bgra.push(chunk[1]);
        bgra.push(chunk[0]);
        bgra.push(255);
    }
    let buffer =
        image::ImageBuffer::from_raw(width, height, bgra).expect("the buffer is the right size");
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_rgb_buffer_becomes_a_bgra_image() {
        let image = bitmap_image(&[10, 20, 30, 40, 50, 60], (2, 1)).expect("two pixels");
        assert_eq!(image.size(0).width.0, 2);
        assert_eq!(image.size(0).height.0, 1);
        // Blue and red are swapped, alpha is opaque.
        assert_eq!(
            &image.as_bytes(0).expect("a frame")[0..8],
            &[30, 20, 10, 255, 60, 50, 40, 255]
        );
    }

    #[test]
    fn a_truncated_buffer_is_refused_rather_than_padded() {
        assert!(bitmap_image(&[1, 2, 3], (4, 4)).is_none());
        assert!(bitmap_image(&[], (0, 0)).is_none());
    }

    #[test]
    fn the_transform_maps_recorded_units_to_pixels() {
        let transform = ReplayTransform::new(point(px(10.0), px(20.0)), 2.0);
        let placed = transform.place((3, 4));
        assert_eq!(placed.x, px(16.0));
        assert_eq!(placed.y, px(28.0));
    }

    #[test]
    fn a_zero_scale_is_refused_so_a_frame_cannot_vanish() {
        let transform = ReplayTransform::new(point(px(0.0), px(0.0)), 0.0);
        assert!(transform.scale > 0.0);
    }
}
