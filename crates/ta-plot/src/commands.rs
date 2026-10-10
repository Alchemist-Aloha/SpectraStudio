//! A recorded drawing, so one description can reach two backends.
//!
//! `spec.md` §19 gives `ta-plot` the render orchestration and `ta-plot-gpui`
//! the interactive wrapper. Rather than fork Plotters to reach a window, a
//! Plotters [`DrawingBackend`](plotters::prelude::DrawingBackend) records the
//! primitives into a buffer: SVG and PNG export replay it through Plotters' own
//! backends, and `ta-plot-gpui` replays it through GPUI paths. Screen and paper
//! therefore cannot drift apart.

use ta_core::Rgba;

/// Which side of the anchor point the text sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HAlign {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VAlign {
    Top,
    Center,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextAnchor {
    pub horizontal: HAlign,
    pub vertical: VAlign,
}

impl Default for TextAnchor {
    fn default() -> Self {
        Self {
            horizontal: HAlign::Left,
            vertical: VAlign::Top,
        }
    }
}

/// A stroke: colour and width in whole pixels, matching what Plotters takes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeStyle {
    pub color: Rgba,
    pub width: u32,
}

impl StrokeStyle {
    pub fn new(color: Rgba, width: u32) -> Self {
        Self {
            color,
            width: width.max(1),
        }
    }
}

/// One drawing primitive.
#[derive(Clone, Debug, PartialEq)]
pub enum DrawCommand {
    Pixel {
        point: (i32, i32),
        color: Rgba,
    },
    Line {
        from: (i32, i32),
        to: (i32, i32),
        style: StrokeStyle,
    },
    Rect {
        corners: [(i32, i32); 2],
        style: StrokeStyle,
        fill: bool,
    },
    Circle {
        center: (i32, i32),
        radius: u32,
        style: StrokeStyle,
        fill: bool,
    },
    Path {
        points: Vec<(i32, i32)>,
        style: StrokeStyle,
    },
    Polygon {
        vertices: Vec<(i32, i32)>,
        style: StrokeStyle,
    },
    Text {
        text: String,
        position: (i32, i32),
        size: f64,
        family: String,
        anchor: TextAnchor,
        color: Rgba,
        /// Degrees clockwise, one of 0, 90, 180, 270.
        rotation: u16,
    },
    /// An RGB image, three bytes per pixel, top row first.
    Bitmap {
        position: (i32, i32),
        size: (u32, u32),
        pixels: Vec<u8>,
    },
}

impl DrawCommand {
    /// A conservative bounding box, used by tests and by the dirty-region
    /// reporting the interaction layer does.
    pub fn bounds(&self) -> ((i32, i32), (i32, i32)) {
        match self {
            DrawCommand::Pixel { point, .. } => (*point, *point),
            DrawCommand::Line { from, to, style } => {
                let pad = style.width as i32 / 2;
                (
                    (from.0.min(to.0) - pad, from.1.min(to.1) - pad),
                    (from.0.max(to.0) + pad, from.1.max(to.1) + pad),
                )
            }
            DrawCommand::Rect {
                corners, style, ..
            } => (
                (corners[0].0.min(corners[1].0) - 1, corners[0].1.min(corners[1].1) - 1),
                (corners[0].0.max(corners[1].0) + style.width as i32, corners[0].1.max(corners[1].1) + style.width as i32),
            ),
            DrawCommand::Circle {
                center,
                radius,
                style,
                ..
            } => {
                let reach = *radius as i32 + style.width as i32 / 2;
                (
                    (center.0 - reach, center.1 - reach),
                    (center.0 + reach, center.1 + reach),
                )
            }
            DrawCommand::Path { points, style } | DrawCommand::Polygon { vertices: points, style } => {
                let pad = style.width as i32 / 2 + 1;
                let mut low = (i32::MAX, i32::MAX);
                let mut high = (i32::MIN, i32::MIN);
                for (x, y) in points {
                    low = (low.0.min(*x), low.1.min(*y));
                    high = (high.0.max(*x), high.1.max(*y));
                }
                if points.is_empty() {
                    return ((0, 0), (0, 0));
                }
                ((low.0 - pad, low.1 - pad), (high.0 + pad, high.1 + pad))
            }
            DrawCommand::Text {
                text,
                position,
                size,
                anchor,
                ..
            } => {
                // No font metrics here, so an estimate based on the character
                // count; the interaction layer only uses it for hit testing.
                let width = (text.chars().count() as f64 * size * 0.6).ceil() as i32;
                let height = (*size * 1.4).ceil() as i32;
                let x0 = match anchor.horizontal {
                    HAlign::Left => position.0,
                    HAlign::Center => position.0 - width / 2,
                    HAlign::Right => position.0 - width,
                };
                let y0 = match anchor.vertical {
                    VAlign::Top => position.1,
                    VAlign::Center => position.1 - height / 2,
                    VAlign::Bottom => position.1 - height,
                };
                ((x0, y0), (x0 + width, y0 + height))
            }
            DrawCommand::Bitmap {
                position,
                size,
                ..
            } => (
                *position,
                (
                    position.0 + size.0 as i32,
                    position.1 + size.1 as i32,
                ),
            ),
        }
    }

    pub const fn kind(&self) -> &'static str {
        match self {
            DrawCommand::Pixel { .. } => "pixel",
            DrawCommand::Line { .. } => "line",
            DrawCommand::Rect { .. } => "rect",
            DrawCommand::Circle { .. } => "circle",
            DrawCommand::Path { .. } => "path",
            DrawCommand::Polygon { .. } => "polygon",
            DrawCommand::Text { .. } => "text",
            DrawCommand::Bitmap { .. } => "bitmap",
        }
    }
}

/// Everything one frame draws.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommandBuffer {
    pub size: (u32, u32),
    pub commands: Vec<DrawCommand>,
}

impl CommandBuffer {
    pub fn new(size: (u32, u32)) -> Self {
        Self {
            size,
            commands: Vec::new(),
        }
    }

    pub fn push(&mut self, command: DrawCommand) {
        self.commands.push(command);
    }

    pub fn len(&self) -> usize {
        self.commands.len()
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// How many commands of each kind, which is what a test asserts on when it
    /// wants to know that text was drawn and not only lines.
    pub fn counts(&self) -> Vec<(&'static str, usize)> {
        let mut kinds: Vec<&'static str> = Vec::new();
        let mut counts: Vec<usize> = Vec::new();
        for command in &self.commands {
            let kind = command.kind();
            match kinds.iter().position(|existing| *existing == kind) {
                Some(index) => counts[index] += 1,
                None => {
                    kinds.push(kind);
                    counts.push(1);
                }
            }
        }
        kinds.into_iter().zip(counts).collect()
    }

    pub fn count_of(&self, kind: &str) -> usize {
        self.commands
            .iter()
            .filter(|command| command.kind() == kind)
            .count()
    }

    /// The text drawn in this frame, for tests and for accessibility.
    pub fn texts(&self) -> Vec<&str> {
        self.commands
            .iter()
            .filter_map(|command| match command {
                DrawCommand::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    /// The bounding box of everything drawn.
    pub fn bounds(&self) -> ((i32, i32), (i32, i32)) {
        let mut low = (i32::MAX, i32::MAX);
        let mut high = (i32::MIN, i32::MIN);
        for command in &self.commands {
            let (command_low, command_high) = command.bounds();
            low = (low.0.min(command_low.0), low.1.min(command_low.1));
            high = (high.0.max(command_high.0), high.1.max(command_high.1));
        }
        if self.commands.is_empty() {
            return ((0, 0), (0, 0));
        }
        (low, high)
    }

    /// The total pixel payload of the bitmaps in this frame, which is what
    /// matters for memory when the heatmap is on screen.
    pub fn bitmap_bytes(&self) -> usize {
        self.commands
            .iter()
            .map(|command| match command {
                DrawCommand::Bitmap { pixels, .. } => pixels.len(),
                _ => 0,
            })
            .sum()
    }

    /// The commands in paint order: bitmaps first, then everything else.
    ///
    /// A renderer that draws a bitmap late would cover the axes with it, so the
    /// ordering rule lives here rather than in each backend.
    pub fn in_paint_order(&self) -> Vec<&DrawCommand> {
        let mut ordered: Vec<&DrawCommand> = self
            .commands
            .iter()
            .filter(|c| matches!(c, DrawCommand::Bitmap { .. } | DrawCommand::Pixel { .. }))
            .collect();
        ordered.extend(self.commands.iter().filter(|c| {
            !matches!(c, DrawCommand::Bitmap { .. } | DrawCommand::Pixel { .. })
        }));
        ordered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_counts_what_it_drew() {
        let mut buffer = CommandBuffer::new((100, 50));
        assert!(buffer.is_empty());
        buffer.push(DrawCommand::Line {
            from: (0, 0),
            to: (10, 10),
            style: StrokeStyle::new(Rgba::rgb(0, 0, 0), 1),
        });
        buffer.push(DrawCommand::Text {
            text: "Delay".into(),
            position: (10, 20),
            size: 8.0,
            family: "Helvetica".into(),
            anchor: TextAnchor::default(),
            color: Rgba::rgb(0, 0, 0),
            rotation: 0,
        });
        buffer.push(DrawCommand::Text {
            text: "ps".into(),
            position: (30, 20),
            size: 8.0,
            family: "Helvetica".into(),
            anchor: TextAnchor::default(),
            color: Rgba::rgb(0, 0, 0),
            rotation: 0,
        });

        assert_eq!(buffer.len(), 3);
        assert_eq!(buffer.count_of("line"), 1);
        assert_eq!(buffer.count_of("text"), 2);
        assert_eq!(buffer.texts(), vec!["Delay", "ps"]);
    }

    #[test]
    fn a_text_bounding_box_respects_the_anchor() {
        let text = |anchor: TextAnchor, position| DrawCommand::Text {
            text: "500".into(),
            position,
            size: 10.0,
            family: "Helvetica".into(),
            anchor,
            color: Rgba::rgb(0, 0, 0),
            rotation: 0,
        };
        let left = text(TextAnchor::default(), (100, 50)).bounds();
        assert_eq!(left.0, (100, 50));
        let right = text(
            TextAnchor {
                horizontal: HAlign::Right,
                vertical: VAlign::Center,
            },
            (100, 50),
        )
        .bounds();
        assert!(right.0 .0 < 100);
        assert!(right.0 .1 < 50 && right.1 .1 > 50);
    }

    #[test]
    fn bitmaps_are_painted_before_everything_else() {
        let mut buffer = CommandBuffer::new((10, 10));
        buffer.push(DrawCommand::Line {
            from: (0, 0),
            to: (1, 1),
            style: StrokeStyle::new(Rgba::rgb(0, 0, 0), 1),
        });
        buffer.push(DrawCommand::Bitmap {
            position: (0, 0),
            size: (2, 2),
            pixels: vec![0; 12],
        });
        let order: Vec<&str> = buffer
            .in_paint_order()
            .into_iter()
            .map(|command| command.kind())
            .collect();
        assert_eq!(order, vec!["bitmap", "line"]);
        assert_eq!(buffer.bitmap_bytes(), 12);
    }

    #[test]
    fn the_frame_bounds_cover_every_command() {
        let mut buffer = CommandBuffer::new((100, 100));
        buffer.push(DrawCommand::Rect {
            corners: [(10, 10), (90, 90)],
            style: StrokeStyle::new(Rgba::rgb(0, 0, 0), 1),
            fill: false,
        });
        buffer.push(DrawCommand::Line {
            from: (0, 5),
            to: (3, 7),
            style: StrokeStyle::new(Rgba::rgb(0, 0, 0), 1),
        });
        let (low, high) = buffer.bounds();
        assert!(low.0 <= 9 && low.1 <= 5);
        assert!(high.0 >= 90 && high.1 >= 90);
    }

    #[test]
    fn an_empty_frame_has_no_bounds_rather_than_a_lie() {
        let buffer = CommandBuffer::new((10, 10));
        assert_eq!(buffer.bounds(), ((0, 0), (0, 0)));
        assert_eq!(buffer.bitmap_bytes(), 0);
    }
}
