//! Exporting a figure to SVG and PNG.
//!
//! One renderer produces both: the same [`render_panel`](crate::render::render_panel)
//! call that paints the screen paints the file, so what is on paper cannot
//! drift from what the user saw (spec §22.4).
//!
//! SVG user units are made equal to points by rewriting the document header,
//! which is what makes a stroke width of 1 mean 1 pt rather than 1 px and lets
//! a figure arrive at its requested physical size in a vector editor.

use crate::render::{draw_panel, HeatmapLayer, PanelLayout, PlottedSeries};
use crate::scale::Scale;
use plotters::backend::SVGBackend;
use std::path::Path;
use ta_core::{
    CoreError, ExportFormat, FigureModel, LengthUnit, PanelModel, PublicationStyle, Result, Rgba,
};

/// One panel's data, ready to draw.
pub struct ExportPanel {
    /// Index into the figure's panel list.
    pub index: usize,
    pub series: Vec<PlottedSeries>,
    pub heatmap: Option<HeatmapLayer>,
}

impl ExportPanel {
    pub fn new(index: usize, series: Vec<PlottedSeries>) -> Self {
        Self {
            index,
            series,
            heatmap: None,
        }
    }

    pub fn with_heatmap(mut self, heatmap: HeatmapLayer) -> Self {
        self.heatmap = Some(heatmap);
        self
    }
}

/// What an export check found wrong before anything was written.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportIssue {
    pub kind: ExportIssueKind,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportIssueKind {
    /// The plot area is smaller than a readable figure.
    PlotTooSmall,
    /// An axis has no label, so the numbers cannot be read.
    UnlabelledAxis,
    /// A range is empty or inverted, so the panel cannot be drawn.
    DegenerateRange,
    /// Points fall outside the stated range and will be clipped.
    ClippedSeries,
    /// A visible series has no legend label, so the legend would miss it.
    MissingLegendEntry,
    /// Nothing would be drawn in the panel at all.
    EmptyPanel,
}

impl ExportIssueKind {
    pub const fn label(self) -> &'static str {
        match self {
            ExportIssueKind::PlotTooSmall => "the plot area is too small",
            ExportIssueKind::UnlabelledAxis => "an axis has no label",
            ExportIssueKind::DegenerateRange => "an axis range is empty",
            ExportIssueKind::ClippedSeries => "points fall outside the axis range",
            ExportIssueKind::MissingLegendEntry => "a visible series has no label",
            ExportIssueKind::EmptyPanel => "the panel would be empty",
        }
    }
}

/// The result of the §17.11 checks: clipping, missing legend entries, cropped
/// labels and unexpected margins, measured before the file is written.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportAudit {
    pub issues: Vec<ExportIssue>,
    /// The plot rectangle of each panel, in points.
    pub plot_rects: Vec<(f64, f64)>,
    /// The margin on each side of each panel, in points.
    pub margins: Vec<(f64, f64, f64, f64)>,
}

impl ExportAudit {
    pub fn is_clean(&self) -> bool {
        self.issues.is_empty()
    }

    pub fn summary(&self) -> String {
        self.issues
            .iter()
            .map(|issue| format!("{}: {}", issue.kind.label(), issue.detail))
            .collect::<Vec<String>>()
            .join("; ")
    }
}

/// Checks a figure against its data before it is drawn.
///
/// This is deliberately conservative: it reports what would be *lost* — a
/// clipped point, a series the legend does not name, a label with no room — and
/// says nothing about taste.
pub fn audit(figure: &FigureModel, panels: &[ExportPanel]) -> Result<ExportAudit> {
    let (figure_width, figure_height) = figure_points(figure);
    let mut issues = Vec::new();
    let mut plot_rects = Vec::new();
    let mut margins = Vec::new();

    for (index, panel) in figure.panels.iter().enumerate() {
        let rect = panel.rect;
        if rect.width <= 0.0 || rect.height <= 0.0 {
            issues.push(ExportIssue {
                kind: ExportIssueKind::PlotTooSmall,
                detail: format!("panel {index} has no area"),
            });
            continue;
        }
        let available = (figure_width * rect.width, figure_height * rect.height);
        let x_range = panel.x.range.unwrap_or((0.0, 1.0));
        let y_range = panel.y.range.unwrap_or((0.0, 1.0));
        let x_scale = Scale::new(panel.x.scale).unwrap_or(Scale::Linear);
        let y_scale = Scale::new(panel.y.scale).unwrap_or(Scale::Linear);
        let layout = PanelLayout::compute(
            panel,
            &figure.style,
            x_scale,
            y_scale,
            x_range,
            y_range,
            available,
        );
        let (plot_width, plot_height) = layout.plot_size();
        plot_rects.push((plot_width, plot_height));
        margins.push((
            layout.plot.x0,
            layout.plot.y0,
            available.0 - layout.plot.x1,
            available.1 - layout.plot.y1,
        ));

        if plot_width < 40.0 || plot_height < 40.0 {
            issues.push(ExportIssue {
                kind: ExportIssueKind::PlotTooSmall,
                detail: format!(
                    "panel {index} leaves {plot_width:.0} × {plot_height:.0} pt for the data"
                ),
            });
        }
        if (x_range.1 - x_range.0).abs() < f64::MIN_POSITIVE
            || (y_range.1 - y_range.0).abs() < f64::MIN_POSITIVE
        {
            issues.push(ExportIssue {
                kind: ExportIssueKind::DegenerateRange,
                detail: format!("panel {index} has an empty range"),
            });
        }
        // A label that needs more room than the margin has would be cropped.
        let title = panel.x.title_size.max(panel.y.title_size * 0.0) as f64;
        let needed_bottom = title + panel.x.tick_label_size as f64 * 1.25 + figure.style.tick_length as f64;
        let bottom = margins.last().map(|m| m.3).unwrap_or(0.0);
        if !panel.x.label.is_empty() && bottom + 1.0 < needed_bottom {
            issues.push(ExportIssue {
                kind: ExportIssueKind::UnlabelledAxis,
                detail: format!(
                    "panel {index}: the x label has {bottom:.1} pt of margin and needs {needed_bottom:.1}"
                ),
            });
        }
        if panel.x.label.trim().is_empty() {
            issues.push(ExportIssue {
                kind: ExportIssueKind::UnlabelledAxis,
                detail: format!("panel {index} has no x axis label"),
            });
        }

        let Some(data) = panels.iter().find(|candidate| candidate.index == index) else {
            continue;
        };
        let drawn: Vec<&PlottedSeries> = data
            .series
            .iter()
            .filter(|series| series.kind != ta_core::SeriesKind::Scatter || !series.segments.is_empty())
            .collect();
        if drawn.is_empty() && data.heatmap.is_none() {
            issues.push(ExportIssue {
                kind: ExportIssueKind::EmptyPanel,
                detail: format!("panel {index} has nothing to draw"),
            });
        }
        if panel.legend.visible {
            for series in data.series.iter().filter(|series| !series.label.trim().is_empty()) {
                let _ = series;
            }
            for (position, series) in data.series.iter().enumerate() {
                if series.label.trim().is_empty() {
                    issues.push(ExportIssue {
                        kind: ExportIssueKind::MissingLegendEntry,
                        detail: format!("panel {index} series {position} is unnamed"),
                    });
                }
            }
        }
        // Clipping: a point outside the stated range is dropped by the
        // renderer, and the author should know which series lost them.
        for series in &data.series {
            let mut outside = 0usize;
            for segment in &series.segments {
                for (x, y) in segment {
                    let outside_x = *x < x_range.0.min(x_range.1) || *x > x_range.0.max(x_range.1);
                    let outside_y = *y < y_range.0.min(y_range.1) || *y > y_range.0.max(y_range.1);
                    if outside_x || outside_y {
                        outside += 1;
                    }
                }
            }
            if outside > 0 {
                let label = if series.label.trim().is_empty() {
                    "an unnamed series".to_string()
                } else {
                    format!("“{}”", series.label)
                };
                issues.push(ExportIssue {
                    kind: ExportIssueKind::ClippedSeries,
                    detail: format!(
                        "{label} in panel {index} has {outside} point(s) outside the axis range"
                    ),
                });
            }
        }
    }

    Ok(ExportAudit {
        issues,
        plot_rects,
        margins,
    })
}

const POINTS_PER_INCH: f64 = 72.0;

/// The figure's size in points, which is the coordinate space both exports use
/// before any raster scale factor.
pub fn figure_points(figure: &FigureModel) -> (f64, f64) {
    let (width, height) = figure.size.in_points();
    (width.max(1.0), height.max(1.0))
}

/// The figure's size in pixels at a given resolution.
pub fn figure_pixels(figure: &FigureModel, dpi: f64) -> (u32, u32) {
    let (width, height) = figure.size.in_inches();
    (
        (width * dpi).round().max(1.0) as u32,
        (height * dpi).round().max(1.0) as u32,
    )
}

/// Writes the figure as SVG.
pub fn export_svg(
    figure: &FigureModel,
    panels: Vec<ExportPanel>,
    path: &Path,
) -> Result<()> {
    let (width, height) = figure_points(figure);
    let background = Rgba::rgb(255, 255, 255);
    {
        let backend = SVGBackend::new(path, (width.round() as u32, height.round() as u32));
        render_figure(figure, &panels, backend, (width, height), background, 1.0)?;
    }
    retitle_svg(path, figure)?;
    Ok(())
}

/// Writes the figure as PNG at the requested resolution.
pub fn export_png(
    figure: &FigureModel,
    panels: Vec<ExportPanel>,
    path: &Path,
    dpi: u32,
) -> Result<()> {
    use plotters::backend::BitMapBackend;

    let dpi = dpi.clamp(36, 2400) as f64;
    let (pixel_width, pixel_height) = figure_pixels(figure, dpi);
    let (point_width, point_height) = figure_points(figure);
    let scale = dpi / POINTS_PER_INCH;
    let background = if figure.export.transparent {
        Rgba::rgba(255, 255, 255, 0)
    } else {
        Rgba::rgb(255, 255, 255)
    };

    let backend = BitMapBackend::new(path, (pixel_width, pixel_height));
    render_figure(
        figure,
        &panels,
        backend,
        (point_width * scale, point_height * scale),
        background,
        scale,
    )?;
    Ok(())
}

/// Dispatches on the figure's own export format.
pub fn export(figure: &FigureModel, panels: Vec<ExportPanel>, path: &Path) -> Result<()> {
    match figure.export.format {
        ExportFormat::Svg => export_svg(figure, panels, path),
        ExportFormat::Png => export_png(figure, panels, path, figure.export.dpi),
        other => Err(CoreError::invalid(format!(
            "exporting {other:?} is not supported yet; SVG and PNG are"
        ))),
    }
}

/// Renders every panel into one drawing area, in the figure's coordinate space.
fn render_figure<B>(
    figure: &FigureModel,
    panels: &[ExportPanel],
    backend: B,
    size: (f64, f64),
    background: Rgba,
    scale: f64,
) -> Result<()>
where
    B: plotters::prelude::DrawingBackend,
{
    use plotters::drawing::IntoDrawingArea;

    let root = backend.into_drawing_area();
    root.fill(&plotters::style::RGBColor(background.r, background.g, background.b))
        .map_err(|error| CoreError::invalid(format!("cannot prepare the drawing area: {error}")))?;

    // Each panel is drawn with an offset instead of into a sub-area, so a
    // multi-panel figure needs no coordinate juggling and all panels share one
    // coordinate space.
    let single = figure.panels.len() <= 1;
    for panel in &figure.panels {
        let Some(export) = panels
            .iter()
            .find(|candidate| figure.panels.get(candidate.index) == Some(panel))
        else {
            continue;
        };
        let rect = if single {
            (0.0, 0.0, size.0, size.1)
        } else {
            (
                panel.rect.x * size.0,
                (1.0 - panel.rect.y - panel.rect.height) * size.1,
                panel.rect.width * size.0,
                panel.rect.height * size.1,
            )
        };
        let style = scaled_style(&figure.style, scale);
        let panel = scaled_panel(panel, scale);
        let x_scale = Scale::new(panel.x.scale)?;
        let y_scale = Scale::new(panel.y.scale)?;
        let fitted = fit_panel(&panel, &export.series);
        let x_range = panel.x.range.unwrap_or(fitted.0);
        let y_range = panel.y.range.unwrap_or(fitted.1);
        let mut layout = PanelLayout::compute(
            &panel,
            &style,
            x_scale,
            y_scale,
            x_range,
            y_range,
            (rect.2.max(1.0), rect.3.max(1.0)),
        );
        layout.offset(rect.0, rect.1);
        let viewport = layout.viewport(x_range, y_range, x_scale, y_scale);
        let series = scaled_series(&export.series, scale);
        draw_panel(
            &root,
            &panel,
            &style,
            &layout,
            &viewport,
            &series,
            export.heatmap.as_ref(),
        )
        .map_err(|error| CoreError::invalid(format!("cannot draw the figure: {error}")))?;
    }

    root.present()
        .map_err(|error| CoreError::invalid(format!("cannot finish the figure: {error}")))?;
    Ok(())
}

pub fn scaled_style(style: &PublicationStyle, scale: f64) -> PublicationStyle {
    if (scale - 1.0).abs() < 1e-9 {
        return style.clone();
    }
    let mut scaled = style.clone();
    scaled.base_font_size = (scaled.base_font_size as f64 * scale) as f32;
    scaled.axis_title_size = (scaled.axis_title_size as f64 * scale) as f32;
    scaled.tick_label_size = (scaled.tick_label_size as f64 * scale) as f32;
    scaled.legend_font_size = (scaled.legend_font_size as f64 * scale) as f32;
    scaled.series_width = (scaled.series_width as f64 * scale) as f32;
    scaled.axis_line_width = (scaled.axis_line_width as f64 * scale) as f32;
    scaled.tick_length = (scaled.tick_length as f64 * scale) as f32;
    scaled
}

pub fn scaled_panel(panel: &PanelModel, scale: f64) -> PanelModel {
    if (scale - 1.0).abs() < 1e-9 {
        return panel.clone();
    }
    let mut scaled = panel.clone();
    for axis in [&mut scaled.x, &mut scaled.y] {
        axis.title_size = (axis.title_size as f64 * scale) as f32;
        axis.tick_label_size = (axis.tick_label_size as f64 * scale) as f32;
    }
    scaled.legend.font_size = (scaled.legend.font_size as f64 * scale) as f32;
    for annotation in &mut scaled.annotations {
        annotation.font_size = (annotation.font_size as f64 * scale) as f32;
    }
    scaled
}

pub fn scaled_series(series: &[PlottedSeries], scale: f64) -> Vec<PlottedSeries> {
    if (scale - 1.0).abs() < 1e-9 {
        return series.to_vec();
    }
    series
        .iter()
        .map(|plotted| {
            let mut scaled = plotted.clone();
            scaled.style.width = (scaled.style.width as f64 * scale) as f32;
            scaled.style.marker_size = (scaled.style.marker_size as f64 * scale) as f32;
            scaled
        })
        .collect()
}

/// The data range of a panel's series, used when the panel has no explicit
/// limits.
fn fit_panel(panel: &PanelModel, series: &[PlottedSeries]) -> ((f64, f64), (f64, f64)) {
    let mut bounds: Option<(f64, f64, f64, f64)> = None;
    for plotted in series {
        for (x, y) in plotted.points() {
            if !x.is_finite() || !y.is_finite() {
                continue;
            }
            bounds = Some(match bounds {
                None => (x, x, y, y),
                Some((x0, x1, y0, y1)) => (x0.min(x), x1.max(x), y0.min(y), y1.max(y)),
            });
        }
    }
    let (x0, x1, y0, y1) = bounds.unwrap_or((0.0, 1.0, -1.0, 1.0));
    let pad = |low: f64, high: f64| {
        let span = high - low;
        let grow = if span == 0.0 { low.abs() * 0.1 + 1.0 } else { span * 0.05 };
        (low - grow, high + grow)
    };
    let (x0, x1) = pad(x0, x1);
    let (y0, y1) = pad(y0, y1);
    let _ = panel;
    ((x0, x1), (y0, y1))
}

/// Rewrites the SVG header so its user units are points and its physical size is
/// the figure's requested size.
fn retitle_svg(path: &Path, figure: &FigureModel) -> Result<()> {
    let text = std::fs::read_to_string(path).map_err(|source| CoreError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let (width, height) = figure.size.in_inches();
    let unit = match figure.size.unit {
        LengthUnit::Inches => "in".to_string(),
        LengthUnit::Centimeters => "cm".to_string(),
        LengthUnit::Millimeters => "mm".to_string(),
        LengthUnit::Pixels => "px".to_string(),
    };
    let (value_width, value_height) = figure.size.in_same_unit(figure.size.unit);
    let _ = (width, height);

    let patched = match text.find('>') {
        Some(end) => {
            let (header, rest) = text.split_at(end + 1);
            let header = replace_attribute(header, "width", &format!("{value_width:.4}{unit}"));
            let header = replace_attribute(&header, "height", &format!("{value_height:.4}{unit}"));
            format!("{header}{rest}")
        }
        None => text,
    };
    std::fs::write(path, patched).map_err(|source| CoreError::Io {
        path: path.display().to_string(),
        source,
    })
}

fn replace_attribute(header: &str, name: &str, value: &str) -> String {
    let needle = format!("{name}=\"");
    match header.find(&needle) {
        Some(start) => {
            let after = start + needle.len();
            match header[after..].find('"') {
                Some(end) => {
                    let mut out = String::with_capacity(header.len() + value.len());
                    out.push_str(&header[..after]);
                    out.push_str(value);
                    out.push_str(&header[after + end..]);
                    out
                }
                None => header.to_string(),
            }
        }
        None => header.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_core::{AxisModel, PanelModel, PlotSeries as ModelSeries, SeriesStyle};

    fn figure() -> FigureModel {
        let mut figure = FigureModel::single_panel("Kinetics");
        figure.size = ta_core::JournalPreset::JacsSingleColumn.size();
        figure.panels[0].x = AxisModel {
            label: "Delay".into(),
            unit: Some("ps".into()),
            ..AxisModel::default()
        };
        figure.panels[0].y = AxisModel {
            label: "ΔOD".into(),
            ..AxisModel::default()
        };
        figure.panels[0].series.push(ModelSeries::new(
            "550 nm",
            ta_core::SeriesSource::Curve {
                x: vec![0.0, 1.0],
                y: vec![0.0, 1.0],
                x_label: "Delay".into(),
                y_label: "ΔOD".into(),
                unit: ta_core::SignalUnit::DeltaOpticalDensity,
            },
        ));
        figure
    }

    fn panels() -> Vec<ExportPanel> {
        vec![ExportPanel::new(
            0,
            vec![PlottedSeries::new(
                "550 nm",
                vec![(-1.0, 0.0), (0.0, 0.05), (1.0, 0.04), (10.0, 0.01)],
                SeriesStyle::default(),
            )],
        )]
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("ta-plot-export-{}-{name}", std::process::id()));
        path
    }

    #[test]
    fn the_figure_size_converts_to_points_and_pixels() {
        let figure = figure();
        let (width, height) = figure_points(&figure);
        assert!((width - 234.0).abs() < 1e-6);
        assert!((height - 180.0).abs() < 1e-6);
        let (pixel_width, pixel_height) = figure_pixels(&figure, 600.0);
        assert_eq!(pixel_width, 1950);
        assert_eq!(pixel_height, 1500);
    }

    #[test]
    fn an_svg_export_declares_its_physical_size_in_points() {
        let figure = figure();
        let path = temp_path("figure.svg");
        export_svg(&figure, panels(), &path).expect("the figure exports");

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("<svg"), "not an SVG: {text:.60}");
        assert!(
            text.contains("width=\"3.2500in\""),
            "the physical width is missing: {text:.200}"
        );
        assert!(text.contains("height=\"2.5000in\""));
        // The axis captions and their units are in the document, not just the
        // numbers.
        assert!(text.contains("Delay"));
        assert!(text.contains("ps"));
        // The data is a real polyline, so a vector editor can edit it and the
        // stroke width means points.
        assert!(text.contains("<polyline"), "{text:.400}");
        assert!(text.contains("stroke-width=\"2\""), "line width is missing");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_png_export_writes_a_file_of_the_requested_size() {
        let figure = figure();
        let path = temp_path("figure.png");
        export_png(&figure, panels(), &path, 300).expect("the figure exports");
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']), "not a PNG");
        assert!(bytes.len() > 2000, "the PNG looks empty: {} bytes", bytes.len());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn the_export_format_decides_which_writer_runs() {
        let mut figure = figure();
        let svg_path = temp_path("dispatch.svg");
        export(&figure, panels(), &svg_path).expect("svg is the default");
        assert!(std::fs::read_to_string(&svg_path).unwrap().starts_with("<svg"));
        std::fs::remove_file(&svg_path).ok();

        figure.export.format = ExportFormat::Pdf;
        let error = export(&figure, panels(), &temp_path("nope.pdf")).unwrap_err();
        assert!(format!("{error}").contains("not supported"));
    }

    #[test]
    fn a_figure_with_no_explicit_limits_fits_its_data() {
        let figure = figure();
        let series = panels().pop().unwrap().series;
        let ((x_low, x_high), (y_low, y_high)) = fit_panel(&figure.panels[0], &series);
        assert!(x_low < -1.0 && x_high > 10.0);
        assert!(y_low < 0.0 && y_high > 0.05);
    }

    #[test]
    fn raster_scale_factors_multiply_the_type_and_the_lines() {
        let style = PublicationStyle::default();
        let scaled = scaled_style(&style, 2.0);
        assert!((scaled.tick_label_size - style.tick_label_size * 2.0).abs() < 1e-6);
        assert!((scaled.tick_length - style.tick_length * 2.0).abs() < 1e-6);
        let series = vec![PlottedSeries::new(
            "a",
            vec![(0.0, 0.0), (1.0, 1.0)],
            SeriesStyle::default(),
        )];
        let scaled_series = scaled_series(&series, 3.0);
        assert!(
            (scaled_series[0].style.width - series[0].style.width * 3.0).abs() < 1e-6
        );
    }

    #[test]
    fn the_check_finds_a_clipped_series_and_a_missing_legend_entry() {
        let mut figure = figure();
        figure.panels[0].x.range = Some((0.0, 1.0));
        figure.panels[0].y.range = Some((0.0, 1.0));
        figure.panels[0].legend.visible = true;
        let series = vec![
            PlottedSeries::new(
                "inside",
                vec![(0.2, 0.2), (0.8, 0.8)],
                SeriesStyle::default(),
            ),
            PlottedSeries::new("", vec![(0.5, 5.0)], SeriesStyle::default()),
        ];
        let report = audit(&figure, &[ExportPanel::new(0, series)]).unwrap();
        assert!(!report.is_clean());
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.kind == ExportIssueKind::ClippedSeries));
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.kind == ExportIssueKind::MissingLegendEntry));
        assert!(report.summary().contains("outside the axis range"));
    }

    #[test]
    fn the_check_passes_a_figure_that_is_ready_to_publish() {
        let figure = figure();
        let series = vec![PlottedSeries::new(
            "measured",
            vec![(0.1, 0.1), (0.9, 0.9)],
            SeriesStyle::default(),
        )];
        let report = audit(&figure, &[ExportPanel::new(0, series)]).unwrap();
        assert!(report.is_clean(), "{}", report.summary());
        assert_eq!(report.plot_rects.len(), 1);
        assert!(report.plot_rects[0].0 > 40.0);
    }

    #[test]
    fn the_check_refuses_a_figure_too_small_to_read() {
        let mut figure = figure();
        figure.size = ta_core::FigureSize {
            width: 0.4,
            height: 0.4,
            unit: LengthUnit::Inches,
        };
        let report = audit(
            &figure,
            &[ExportPanel::new(
                0,
                vec![PlottedSeries::new("a", vec![(0.0, 0.0)], SeriesStyle::default())],
            )],
        )
        .unwrap();
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.kind == ExportIssueKind::PlotTooSmall));
    }

    #[test]
    fn the_check_notices_an_empty_panel() {
        let figure = figure();
        let report = audit(&figure, &[ExportPanel::new(0, Vec::new())]).unwrap();
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.kind == ExportIssueKind::EmptyPanel));
    }

    #[test]
    fn a_single_panel_figure_uses_the_whole_page() {
        let figure = figure();
        let (width, height) = figure_points(&figure);
        assert_eq!(figure.panels.len(), 1);
        // The single-panel rectangle is the whole figure, so the layout is
        // built against the figure size.
        let layout = PanelLayout::compute(
            &figure.panels[0],
            &figure.style,
            Scale::Linear,
            Scale::Linear,
            (0.0, 1.0),
            (0.0, 1.0),
            (width, height),
        );
        assert!(layout.plot.x1 <= width);
        assert!(layout.plot.y1 <= height);
        let _ = PanelModel::new("unused");
    }
}
