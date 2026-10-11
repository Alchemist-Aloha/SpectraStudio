//! The figure model: a serializable description of what a plot shows.
//!
//! This is data only. Transforms and rendering live in `ta-plot`, interaction in
//! `ta-plot-gpui`. Keeping the description here is what lets one model drive the
//! screen, the SVG export and the saved project without three copies drifting
//! apart.

use crate::ids::{AnnotationId, FigureId, SeriesId};
use crate::selection::{SpectrumSelection, TraceSelection};
use crate::units::SignalUnit;
use crate::ids::DatasetId;
use crate::ids::VersionId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// An 8-bit-per-channel colour. Serializable and comparable, so style presets
/// survive a project round-trip exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Parses `#rgb`, `#rrggbb` and `#rrggbbaa`.
    pub fn from_hex(text: &str) -> Option<Self> {
        let hex = text.strip_prefix('#').unwrap_or(text);
        let byte = |slice: &str| u8::from_str_radix(slice, 16).ok();
        match hex.len() {
            3 => {
                let expand = |c: char| {
                    let value = c.to_digit(16)? as u8;
                    Some(value * 16 + value)
                };
                let mut chars = hex.chars();
                Some(Self::rgb(
                    expand(chars.next()?)?,
                    expand(chars.next()?)?,
                    expand(chars.next()?)?,
                ))
            }
            6 | 8 => {
                let parsed = Self::rgba(
                    byte(&hex[0..2])?,
                    byte(&hex[2..4])?,
                    byte(&hex[4..6])?,
                    if hex.len() == 8 { byte(&hex[6..8])? } else { 255 },
                );
                Some(parsed)
            }
            _ => None,
        }
    }

    pub fn to_hex(self) -> String {
        if self.a == 255 {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
        }
    }

    /// Linear blend in sRGB space, `t = 0` returning `self`.
    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let mix = |a: u8, b: u8| -> u8 {
            let a = a as f32;
            let b = b as f32;
            (a + (b - a) * t).round().clamp(0.0, 255.0) as u8
        };
        Self::rgba(
            mix(self.r, other.r),
            mix(self.g, other.g),
            mix(self.b, other.b),
            mix(self.a, other.a),
        )
    }

    /// Straight alpha `(r, g, b)`, which is what exported raster formats want.
    pub fn to_rgb(self) -> [u8; 3] {
        [self.r, self.g, self.b]
    }
}

/// One stop of a colour map.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColorStop {
    /// Position in the map, 0 to 1.
    pub at: f32,
    pub color: Rgba,
}

/// A named, ordered set of colour stops.
///
/// Transient absorption is signed data, so the presets that matter are
/// diverging: the map has to say "no signal" at zero without hiding which side
/// is which.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Colormap {
    pub name: String,
    pub stops: Vec<ColorStop>,
    pub reversed: bool,
}

impl Colormap {
    pub fn new(name: impl Into<String>, stops: Vec<ColorStop>) -> Self {
        Self {
            name: name.into(),
            stops,
            reversed: false,
        }
    }

    /// The diverging presets the contour viewer offers by default.
    pub fn preset(name: &str) -> Option<Self> {
        let stops: &[(f32, [u8; 3])] = match name {
            // Cool-to-warm diverging pair, the default for signed ΔA.
            "coolwarm" => &[
                (0.0, [59, 76, 192]),
                (0.25, [144, 178, 254]),
                (0.5, [221, 221, 221]),
                (0.75, [245, 156, 125]),
                (1.0, [180, 4, 38]),
            ],
            "bwr" => &[
                (0.0, [0, 0, 255]),
                (0.5, [255, 255, 255]),
                (1.0, [255, 0, 0]),
            ],
            "seismic" => &[
                (0.0, [0, 0, 76]),
                (0.25, [0, 0, 255]),
                (0.5, [255, 255, 255]),
                (0.75, [255, 0, 0]),
                (1.0, [128, 0, 0]),
            ],
            // Perceptually ordered, for magnitude-only views such as |ΔA|.
            "viridis" => &[
                (0.0, [68, 1, 84]),
                (0.25, [59, 82, 139]),
                (0.5, [33, 145, 140]),
                (0.75, [94, 201, 98]),
                (1.0, [253, 231, 37]),
            ],
            "magma" => &[
                (0.0, [0, 0, 4]),
                (0.25, [81, 18, 124]),
                (0.5, [183, 55, 121]),
                (0.75, [251, 136, 97]),
                (1.0, [252, 253, 191]),
            ],
            "grey" | "grayscale" => &[(0.0, [0, 0, 0]), (1.0, [255, 255, 255])],
            _ => return None,
        };
        Some(Self::new(
            name,
            stops
                .iter()
                .map(|(at, [r, g, b])| ColorStop {
                    at: *at,
                    color: Rgba::rgb(*r, *g, *b),
                })
                .collect(),
        ))
    }

    /// The names offered in the contour inspector, in menu order.
    pub const PRESETS: &'static [&'static str] =
        &["coolwarm", "bwr", "seismic", "viridis", "magma", "grey"];

    /// The stops in the order they are drawn, honouring `reversed`.
    pub fn effective_stops(&self) -> Vec<ColorStop> {
        let mut stops = self.stops.clone();
        if self.reversed {
            for stop in &mut stops {
                stop.at = 1.0 - stop.at;
            }
            stops.reverse();
        }
        stops
    }

    /// Samples the map at `t` in 0..=1. Values outside are clamped, never
    /// wrapped, so a mis-set colour limit cannot paint a false extreme.
    pub fn sample(&self, t: f32) -> Rgba {
        let stops = self.effective_stops();
        if stops.is_empty() {
            return Rgba::rgb(0, 0, 0);
        }
        let t = t.clamp(0.0, 1.0);
        if t <= stops[0].at {
            return stops[0].color;
        }
        if t >= stops[stops.len() - 1].at {
            return stops[stops.len() - 1].color;
        }
        for pair in stops.windows(2) {
            let (low, high) = (pair[0], pair[1]);
            if t >= low.at && t <= high.at {
                let span = high.at - low.at;
                let fraction = if span <= 0.0 { 0.0 } else { (t - low.at) / span };
                return low.color.lerp(high.color, fraction);
            }
        }
        stops[stops.len() - 1].color
    }
}

/// How automatic colour limits are chosen.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoRange {
    /// The user's explicit limits.
    Manual,
    /// The data minimum and maximum.
    Full,
    /// The given percentile of the magnitude, per side.
    Percentile(f64),
}

/// The colour axis of a contour or heatmap.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColorLimits {
    pub min: f64,
    pub max: f64,
    /// Force `min == -max` so zero sits in the middle of a diverging map.
    pub symmetric_about_zero: bool,
    pub auto: AutoRange,
}

impl Default for ColorLimits {
    fn default() -> Self {
        Self {
            min: -1.0,
            max: 1.0,
            symmetric_about_zero: true,
            auto: AutoRange::Full,
        }
    }
}

impl ColorLimits {
    /// Normalizes a value into 0..=1 for the colour map, or `None` for a
    /// missing value, which callers paint as transparent rather than as a
    /// value.
    pub fn normalize(&self, value: f64) -> Option<f32> {
        if !value.is_finite() {
            return None;
        }
        let span = self.max - self.min;
        if span <= 0.0 {
            return Some(0.5);
        }
        Some(((value - self.min) / span) as f32)
    }

    /// Fits the limits to the data.
    pub fn fit(&mut self, values: impl Iterator<Item = f64>) {
        let finite: Vec<f64> = values.filter(|v| v.is_finite()).collect();
        if finite.is_empty() {
            return;
        }
        let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
        for value in &finite {
            min = min.min(*value);
            max = max.max(*value);
        }
        // The outlier guard runs first, so symmetry is applied to the trimmed
        // extent rather than letting one bad pixel set the scale.
        if let AutoRange::Percentile(percent) = self.auto {
            let mut magnitudes: Vec<f64> = finite.iter().map(|v| v.abs()).collect();
            magnitudes.sort_by(f64::total_cmp);
            let index = ((percent.clamp(0.0, 100.0) / 100.0) * (magnitudes.len() - 1) as f64)
                .round() as usize;
            let bound = magnitudes[index.min(magnitudes.len() - 1)];
            min = min.max(-bound);
            max = max.min(bound);
        }
        if self.symmetric_about_zero {
            let extent = min.abs().max(max.abs());
            min = -extent;
            max = extent;
        }
        self.min = min;
        self.max = max;
    }
}

/// A plot axis' scale.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AxisScale {
    Linear,
    /// Requires strictly positive coordinates.
    Logarithmic,
    /// Linear within `± linear_width`, logarithmic outside it. The default for
    /// TA delay axes, which contain zero and negative delays.
    SymmetricLogarithmic { linear_width: f64 },
    /// Linear below `break_at`, logarithmic above it.
    SplitLinearLog { break_at: f64 },
}

impl Default for AxisScale {
    fn default() -> Self {
        AxisScale::Linear
    }
}

/// One axis of one panel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AxisModel {
    pub label: String,
    pub unit: Option<String>,
    pub scale: AxisScale,
    /// Explicit limits, or `None` to fit the data.
    pub range: Option<(f64, f64)>,
    pub reversed: bool,
    pub major_ticks: Option<usize>,
    pub minor_ticks: bool,
    pub grid: bool,
    /// Draw the opposite spine, with ticks but no second label.
    #[serde(default)]
    pub mirror: bool,
    pub tick_label_size: f32,
    pub title_size: f32,
}

impl Default for AxisModel {
    fn default() -> Self {
        Self {
            label: String::new(),
            unit: None,
            scale: AxisScale::Linear,
            range: None,
            reversed: false,
            major_ticks: None,
            minor_ticks: false,
            grid: false,
            mirror: false,
            tick_label_size: 8.0,
            title_size: 8.0,
        }
    }
}

impl AxisModel {
    pub fn labelled(&self) -> String {
        match &self.unit {
            Some(unit) if !unit.is_empty() => format!("{} ({})", self.label, unit),
            _ => self.label.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DashPattern {
    Solid,
    Dashed,
    Dotted,
    DashDot,
}

impl DashPattern {
    /// Dash lengths in multiples of the line width, which keeps dashes legible
    /// at every figure size.
    pub const fn segments(self) -> &'static [f32] {
        match self {
            DashPattern::Solid => &[],
            DashPattern::Dashed => &[4.0, 2.5],
            DashPattern::Dotted => &[1.0, 2.0],
            DashPattern::DashDot => &[4.0, 2.0, 1.0, 2.0],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkerShape {
    None,
    Circle,
    Square,
    Triangle,
    Diamond,
    Cross,
}

/// How one series is drawn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeriesStyle {
    pub color: Rgba,
    pub width: f32,
    pub dash: DashPattern,
    pub marker: MarkerShape,
    pub marker_size: f32,
    /// Opacity 0..=1, applied to line, markers and fill.
    pub opacity: f32,
}

impl Default for SeriesStyle {
    fn default() -> Self {
        Self {
            color: Rgba::rgb(31, 41, 55),
            width: 1.5,
            dash: DashPattern::Solid,
            marker: MarkerShape::None,
            marker_size: 4.0,
            opacity: 1.0,
        }
    }
}

impl SeriesStyle {
    pub fn with_color(mut self, color: Rgba) -> Self {
        self.color = color;
        self
    }
}

/// How a series' numbers are moved for display.
///
/// These are display transforms: the stored matrix and the stored versions are
/// never modified. They are recorded with the series so a comparison can be
/// reproduced exactly, and so a normalized curve is never mistaken for data.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeriesTransform {
    /// Multiplies the values.
    pub scale: f64,
    /// Added after scaling.
    pub offset: f64,
    /// Added to the series' own coordinate: a trace slides along its delay
    /// axis, a spectrum along its wavelength axis.
    pub coordinate_offset: f64,
}

impl Default for SeriesTransform {
    fn default() -> Self {
        Self {
            scale: 1.0,
            offset: 0.0,
            coordinate_offset: 0.0,
        }
    }
}

impl SeriesTransform {
    pub fn is_identity(&self) -> bool {
        self.scale == 1.0 && self.offset == 0.0 && self.coordinate_offset == 0.0
    }

    /// The transform in words, for a caption or a legend.
    pub fn describe(&self) -> String {
        if self.is_identity() {
            return "as measured".to_string();
        }
        let mut parts = Vec::new();
        if self.scale != 1.0 {
            parts.push(format!("×{:.4}", self.scale));
        }
        if self.offset != 0.0 {
            parts.push(format!("{:+.4}", self.offset));
        }
        if self.coordinate_offset != 0.0 {
            parts.push(format!("{:+.4} on the axis", self.coordinate_offset));
        }
        parts.join(", ")
    }
}

/// Where a series' numbers come from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum SeriesSource {
    Trace(TraceSelection),
    Spectrum(SpectrumSelection),
    /// The whole matrix as an image.
    Heatmap {
        dataset: DatasetId,
        version: VersionId,
    },
    /// Filled contour bands derived from the matrix.
    Contour {
        dataset: DatasetId,
        version: VersionId,
    },
    /// Contour isolines derived from the matrix.
    ContourLines {
        dataset: DatasetId,
        version: VersionId,
    },
    /// Numbers that are not a selection of a matrix: reference curves, imported
    /// kinetic files, decay-associated spectra.
    Curve {
        x: Vec<f64>,
        y: Vec<f64>,
        x_label: String,
        y_label: String,
        unit: SignalUnit,
    },
    /// A fitted model evaluated over the fit's own time range.
    Fit {
        dataset: DatasetId,
        version: VersionId,
        selection: TraceSelection,
        fit: crate::ids::FitId,
    },
}

/// How the series' coordinates are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeriesKind {
    Line,
    Markers,
    LineMarkers,
    Scatter,
    Step,
    Bars,
}

/// One drawable element of a panel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlotSeries {
    pub id: SeriesId,
    pub label: String,
    pub kind: SeriesKind,
    pub source: SeriesSource,
    pub style: SeriesStyle,
    pub visible: bool,
    /// Paint order; higher is later.
    pub z: i32,
    /// Display-only scaling and offsets. Absent in older files, where the
    /// identity is the right answer.
    #[serde(default)]
    pub display: SeriesTransform,
}

impl PlotSeries {
    pub fn new(label: impl Into<String>, source: SeriesSource) -> Self {
        Self {
            id: SeriesId::new(),
            label: label.into(),
            kind: SeriesKind::Line,
            source,
            style: SeriesStyle::default(),
            visible: true,
            z: 0,
            display: SeriesTransform::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegendPosition {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    OutsideRight,
    OutsideBottom,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegendSpec {
    pub visible: bool,
    pub position: LegendPosition,
    pub frame: bool,
    pub font_size: f32,
    /// Per-series label overrides, keyed by series id.
    pub labels: BTreeMap<SeriesId, String>,
}

impl Default for LegendSpec {
    fn default() -> Self {
        Self {
            visible: true,
            position: LegendPosition::TopRight,
            frame: false,
            font_size: 7.0,
            labels: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationKind {
    Text,
    /// A horizontal or vertical line marking a selected coordinate.
    CursorLine,
    /// A shaded band marking an averaged window or an excluded interval.
    Band,
    Arrow,
    Rectangle,
}

/// An annotation in data coordinates, so it survives zoom, crop and export.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnnotationSpec {
    pub id: AnnotationId,
    pub kind: AnnotationKind,
    pub text: String,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    /// Which axis a cursor line or band refers to.
    pub axis: Option<crate::units::AxisKind>,
    pub color: Rgba,
    pub font_size: f32,
}

impl AnnotationSpec {
    pub fn text(text: impl Into<String>, x: f64, y: f64) -> Self {
        Self {
            id: AnnotationId::new(),
            kind: AnnotationKind::Text,
            text: text.into(),
            x: Some(x),
            y: Some(y),
            width: None,
            height: None,
            axis: None,
            color: Rgba::rgb(17, 24, 39),
            font_size: 8.0,
        }
    }
}

/// The contour-specific controls of §7.3.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContourSpec {
    pub colormap: Colormap,
    pub limits: ColorLimits,
    /// Explicit level values. Empty means "derive from the limits".
    pub levels: Vec<f64>,
    pub level_count: usize,
    pub show_lines: bool,
    pub line_color: Rgba,
    pub line_width: f32,
    /// Smooth the image between samples. Off by default: a heatmap of a
    /// measurement should not invent gradients between measured points.
    pub smooth: bool,
}

impl Default for ContourSpec {
    fn default() -> Self {
        Self {
            colormap: Colormap::preset("coolwarm").expect("coolwarm is a built-in preset"),
            limits: ColorLimits::default(),
            levels: Vec::new(),
            level_count: 12,
            show_lines: false,
            line_color: Rgba::rgba(17, 24, 39, 90),
            line_width: 0.5,
            smooth: false,
        }
    }
}

/// Physical size, in the unit the user typed.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LengthUnit {
    Inches,
    Centimeters,
    Millimeters,
    Pixels,
}

impl LengthUnit {
    pub const fn symbol(self) -> &'static str {
        match self {
            LengthUnit::Inches => "in",
            LengthUnit::Centimeters => "cm",
            LengthUnit::Millimeters => "mm",
            LengthUnit::Pixels => "px",
        }
    }

    /// How many of this unit make one inch.
    pub const fn per_inch(self) -> f64 {
        match self {
            LengthUnit::Inches => 1.0,
            LengthUnit::Centimeters => 2.54,
            LengthUnit::Millimeters => 25.4,
            LengthUnit::Pixels => 96.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FigureSize {
    pub width: f64,
    pub height: f64,
    pub unit: LengthUnit,
}

impl Default for FigureSize {
    fn default() -> Self {
        Self {
            width: 3.25,
            height: 2.5,
            unit: LengthUnit::Inches,
        }
    }
}

impl FigureSize {
    pub fn in_inches(&self) -> (f64, f64) {
        let per_inch = self.unit.per_inch();
        (self.width / per_inch, self.height / per_inch)
    }

    pub fn in_points(&self) -> (f64, f64) {
        let (width, height) = self.in_inches();
        (width * 72.0, height * 72.0)
    }

    pub fn in_same_unit(&self, unit: LengthUnit) -> (f64, f64) {
        let (width, height) = self.in_inches();
        let per_inch = unit.per_inch();
        (width * per_inch, height * per_inch)
    }
}

/// The journal presets of §17.3.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalPreset {
    JacsSingleColumn,
    JacsDoubleColumn,
    Custom,
}

impl JournalPreset {
    pub fn size(self) -> FigureSize {
        match self {
            JournalPreset::JacsSingleColumn => FigureSize {
                width: 3.25,
                height: 2.5,
                unit: LengthUnit::Inches,
            },
            JournalPreset::JacsDoubleColumn => FigureSize {
                width: 7.0,
                height: 3.0,
                unit: LengthUnit::Inches,
            },
            JournalPreset::Custom => FigureSize::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TickDirection {
    In,
    Out,
}

/// The typographic and line conventions every element inherits.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PublicationStyle {
    pub font_family: String,
    pub base_font_size: f32,
    pub axis_title_size: f32,
    pub tick_label_size: f32,
    pub legend_font_size: f32,
    pub series_width: f32,
    pub axis_line_width: f32,
    pub tick_length: f32,
    pub tick_direction: TickDirection,
    pub frame: bool,
    pub grid: bool,
    pub preset: JournalPreset,
}

impl Default for PublicationStyle {
    fn default() -> Self {
        Self {
            font_family: "Helvetica".to_string(),
            base_font_size: 8.0,
            axis_title_size: 8.0,
            tick_label_size: 7.0,
            legend_font_size: 7.0,
            series_width: 1.5,
            axis_line_width: 0.75,
            tick_length: 3.0,
            tick_direction: TickDirection::Out,
            frame: true,
            grid: false,
            preset: JournalPreset::JacsSingleColumn,
        }
    }
}

impl PublicationStyle {
    /// The style the JACS presets imply: 8 pt type, thin spines, ticks out.
    pub fn jacs(font_size: f32) -> Self {
        Self {
            base_font_size: font_size,
            axis_title_size: font_size,
            tick_label_size: font_size,
            legend_font_size: font_size,
            ..Default::default()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Svg,
    Png,
    Pdf,
    Eps,
}

/// A rectangle in the figure's own coordinates (0..1 per side), used by
/// multi-panel layouts.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl PanelRect {
    pub fn full() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        }
    }
}

/// One panel of a figure. A single-plot figure has exactly one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelModel {
    pub id: SeriesId,
    pub title: String,
    pub x: AxisModel,
    pub y: AxisModel,
    pub series: Vec<PlotSeries>,
    pub legend: LegendSpec,
    pub annotations: Vec<AnnotationSpec>,
    /// Present when the panel draws a matrix.
    pub contour: Option<ContourSpec>,
    pub rect: PanelRect,
    /// Colour bar controls, for panels with a contour.
    pub colorbar: ColorbarSpec,
}

impl PanelModel {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            id: SeriesId::new(),
            title: title.into(),
            x: AxisModel::default(),
            y: AxisModel::default(),
            series: Vec::new(),
            legend: LegendSpec::default(),
            annotations: Vec::new(),
            contour: None,
            rect: PanelRect::full(),
            colorbar: ColorbarSpec::default(),
        }
    }

    /// The series in paint order.
    pub fn draw_order(&self) -> Vec<&PlotSeries> {
        let mut series: Vec<&PlotSeries> = self.series.iter().filter(|s| s.visible).collect();
        series.sort_by_key(|s| s.z);
        series
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColorbarSpec {
    pub visible: bool,
    pub ticks: usize,
    pub show_unit: bool,
    pub label: String,
}

impl Default for ColorbarSpec {
    fn default() -> Self {
        Self {
            visible: true,
            ticks: 5,
            show_unit: true,
            label: String::new(),
        }
    }
}

/// A complete saved figure.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FigureModel {
    pub id: FigureId,
    pub title: String,
    pub size: FigureSize,
    pub style: PublicationStyle,
    pub panels: Vec<PanelModel>,
    pub export: ExportSpec,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportSpec {
    pub format: ExportFormat,
    /// Raster resolution; vector formats ignore it.
    pub dpi: u32,
    pub transparent: bool,
}

impl Default for ExportSpec {
    fn default() -> Self {
        Self {
            format: ExportFormat::Svg,
            dpi: 600,
            transparent: false,
        }
    }
}

impl FigureModel {
    /// A single-panel figure with the publication defaults.
    pub fn single_panel(title: impl Into<String>) -> Self {
        Self {
            id: FigureId::new(),
            title: title.into(),
            size: JournalPreset::JacsSingleColumn.size(),
            style: PublicationStyle::default(),
            panels: vec![PanelModel::new("")],
            export: ExportSpec::default(),
        }
    }

    pub fn panel(&self, index: usize) -> Option<&PanelModel> {
        self.panels.get(index)
    }

    pub fn panel_mut(&mut self, index: usize) -> Option<&mut PanelModel> {
        self.panels.get_mut(index)
    }

    /// Applies a journal preset to the size and the type sizes together, so a
    /// figure cannot end up sized for one convention and typed for another.
    pub fn apply_preset(&mut self, preset: JournalPreset) {
        self.size = preset.size();
        self.style.preset = preset;
        let font = match preset {
            JournalPreset::JacsSingleColumn => 8.0,
            JournalPreset::JacsDoubleColumn => 8.0,
            JournalPreset::Custom => self.style.base_font_size,
        };
        self.style.base_font_size = font;
        self.style.axis_title_size = font;
        self.style.tick_label_size = font;
        self.style.legend_font_size = font;
        for panel in &mut self.panels {
            panel.x.title_size = font;
            panel.x.tick_label_size = font;
            panel.y.title_size = font;
            panel.y.tick_label_size = font;
            panel.legend.font_size = font;
        }
    }

    /// Sets the type size everywhere, which is what "a figure sized for one
    /// convention and typed for another" must not be able to do.
    pub fn set_type_size(&mut self, font: f32) {
        self.style.base_font_size = font;
        self.style.axis_title_size = font;
        self.style.tick_label_size = font;
        self.style.legend_font_size = font;
        for panel in &mut self.panels {
            panel.x.title_size = font;
            panel.x.tick_label_size = font;
            panel.y.title_size = font;
            panel.y.tick_label_size = font;
            panel.legend.font_size = font;
        }
    }

    /// Every dataset this figure reads, so a project can tell whether the
    /// figure is still resolvable after a dataset is removed.
    pub fn dataset_references(&self) -> Vec<DatasetId> {
        let mut out = Vec::new();
        let mut push = |id: DatasetId| {
            if !out.contains(&id) {
                out.push(id);
            }
        };
        for panel in &self.panels {
            for series in &panel.series {
                match &series.source {
                    SeriesSource::Trace(selection) => push(selection.dataset),
                    SeriesSource::Spectrum(selection) => push(selection.dataset),
                    SeriesSource::Heatmap { dataset, .. }
                    | SeriesSource::Contour { dataset, .. }
                    | SeriesSource::ContourLines { dataset, .. }
                    | SeriesSource::Fit { dataset, .. } => push(*dataset),
                    SeriesSource::Curve { .. } => {}
                }
            }
        }
        out
    }
}

/// Whether a series reads one particular matrix version.
///
/// Removing a version has to use this rather than the dataset check: once
/// anything has been cropped, one dataset holds several grids at once.
pub fn series_references_version(
    series: &PlotSeries,
    dataset: DatasetId,
    version: VersionId,
) -> bool {
    let matches = |candidate: DatasetId, candidate_version: VersionId| {
        candidate == dataset && candidate_version == version
    };
    match &series.source {
        SeriesSource::Trace(selection) => matches(selection.dataset, selection.version),
        SeriesSource::Spectrum(selection) => matches(selection.dataset, selection.version),
        SeriesSource::Heatmap {
            dataset: source,
            version: source_version,
        }
        | SeriesSource::Contour {
            dataset: source,
            version: source_version,
        }
        | SeriesSource::ContourLines {
            dataset: source,
            version: source_version,
        } => matches(*source, *source_version),
        SeriesSource::Fit {
            dataset: source,
            version: source_version,
            selection,
            ..
        } => matches(*source, *source_version) || matches(selection.dataset, selection.version),
        SeriesSource::Curve { .. } => false,
    }
}

/// Whether a series reads anything from a dataset at all.
pub fn series_references_dataset(series: &PlotSeries, dataset: DatasetId) -> bool {
    match &series.source {
        SeriesSource::Trace(selection) => selection.dataset == dataset,
        SeriesSource::Spectrum(selection) => selection.dataset == dataset,
        SeriesSource::Heatmap { dataset: source, .. }
        | SeriesSource::Contour { dataset: source, .. }
        | SeriesSource::ContourLines { dataset: source, .. }
        | SeriesSource::Fit { dataset: source, .. } => *source == dataset,
        SeriesSource::Curve { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_in_all_three_forms() {
        assert_eq!(Rgba::from_hex("#f00"), Some(Rgba::rgb(255, 0, 0)));
        assert_eq!(Rgba::from_hex("#ff0000"), Some(Rgba::rgb(255, 0, 0)));
        assert_eq!(
            Rgba::from_hex("ff000080"),
            Some(Rgba::rgba(255, 0, 0, 128))
        );
        assert_eq!(Rgba::from_hex("#12"), None);
        assert_eq!(Rgba::rgb(255, 0, 0).to_hex(), "#ff0000");
        assert_eq!(Rgba::rgba(0, 0, 0, 0).to_hex(), "#00000000");
    }

    #[test]
    fn colormap_sampling_hits_stops_and_clamps_outside() {
        let map = Colormap::preset("bwr").unwrap();
        assert_eq!(map.sample(0.0), Rgba::rgb(0, 0, 255));
        assert_eq!(map.sample(0.5), Rgba::rgb(255, 255, 255));
        assert_eq!(map.sample(1.0), Rgba::rgb(255, 0, 0));
        // Out-of-range input clamps rather than wrapping into a false colour.
        assert_eq!(map.sample(-5.0), Rgba::rgb(0, 0, 255));
        assert_eq!(map.sample(5.0), Rgba::rgb(255, 0, 0));
        // Midway between two stops is a real blend, not a snap.
        let midway = map.sample(0.25);
        assert!(midway.b > 200 && midway.g > 100);
    }

    #[test]
    fn reversing_a_colormap_keeps_zero_in_the_middle() {
        let mut map = Colormap::preset("coolwarm").unwrap();
        let before = map.sample(0.5);
        map.reversed = true;
        assert_eq!(map.sample(0.5), before);
        assert_eq!(map.sample(0.0), Colormap::preset("coolwarm").unwrap().sample(1.0));
    }

    #[test]
    fn symmetric_limits_put_zero_in_the_middle() {
        let mut limits = ColorLimits {
            symmetric_about_zero: true,
            auto: AutoRange::Full,
            ..ColorLimits::default()
        };
        limits.fit([-0.3, 0.1, 0.05].into_iter());
        assert_eq!(limits.min, -0.3);
        assert_eq!(limits.max, 0.3);
        assert_eq!(limits.normalize(0.0), Some(0.5));
    }

    #[test]
    fn percentile_limits_ignore_a_single_outlier() {
        let mut values: Vec<f64> = (0..1000).map(|i| (i as f64 / 1000.0) * 0.1).collect();
        values.push(99.0);
        let mut limits = ColorLimits {
            symmetric_about_zero: true,
            auto: AutoRange::Percentile(99.0),
            ..ColorLimits::default()
        };
        limits.fit(values.into_iter());
        assert!(limits.max < 1.0, "outlier leaked into the limits: {limits:?}");
        assert!(limits.max > 0.09);
    }

    #[test]
    fn missing_values_normalize_to_nothing_rather_than_to_a_colour() {
        let limits = ColorLimits::default();
        assert_eq!(limits.normalize(f64::NAN), None);
        assert!(limits.normalize(0.0).is_some());
    }

    #[test]
    fn figure_sizes_convert_between_units_and_points() {
        let size = JournalPreset::JacsSingleColumn.size();
        assert_eq!(size.in_inches(), (3.25, 2.5));
        let (width_pt, height_pt) = size.in_points();
        assert!((width_pt - 234.0).abs() < 1e-9);
        assert!((height_pt - 180.0).abs() < 1e-9);

        let metric = FigureSize {
            width: 8.255,
            height: 6.35,
            unit: LengthUnit::Centimeters,
        };
        let (width, _) = metric.in_inches();
        assert!((width - 3.25).abs() < 1e-9);
    }

    #[test]
    fn a_journal_preset_sets_size_and_type_together() {
        let mut figure = FigureModel::single_panel("Kinetics at 550 nm");
        figure.apply_preset(JournalPreset::JacsDoubleColumn);
        assert_eq!(figure.size.width, 7.0);
        assert_eq!(figure.style.base_font_size, 8.0);
        assert_eq!(figure.panels[0].x.tick_label_size, 8.0);
    }

    #[test]
    fn hidden_series_are_not_drawn_but_are_kept() {
        let mut panel = PanelModel::new("t");
        let series = PlotSeries::new(
            "550 nm",
            SeriesSource::Curve {
                x: vec![0.0, 1.0],
                y: vec![0.0, 1.0],
                x_label: "Delay".into(),
                y_label: "ΔOD".into(),
                unit: SignalUnit::DeltaOpticalDensity,
            },
        );
        panel.series.push(series);
        assert_eq!(panel.draw_order().len(), 1);
        panel.series[0].visible = false;
        assert_eq!(panel.draw_order().len(), 0);
        assert_eq!(panel.series.len(), 1);
    }
}
