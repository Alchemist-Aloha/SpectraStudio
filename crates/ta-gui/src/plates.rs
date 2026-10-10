//! Turning the session into plate scenes.
//!
//! Each plate is a `PanelModel` (the figure description the renderer and the
//! exporter share) plus the resolved curves to draw in it. No GPUI here: the
//! scenes are built from the session and can be tested on their own.

use crate::session::{PlateId, Session};
use ta_core::{
    AxisKind, AxisModel, AxisScale, ColorLimits, Colormap, ContourSpec, DashPattern, PanelModel,
    PublicationStyle, Rgba, SeriesKind, SeriesStyle,
};
use ta_plot::{isolines, ContourGrid, PlottedSeries};

/// The type sizes of the direction: a working screen's scale, not a poster's.
pub fn style() -> PublicationStyle {
    PublicationStyle {
        font_family: "Helvetica".to_string(),
        base_font_size: 10.0,
        axis_title_size: 10.0,
        tick_label_size: 9.0,
        legend_font_size: 9.0,
        series_width: 1.5,
        axis_line_width: 1.0,
        tick_length: 4.0,
        tick_direction: ta_core::TickDirection::Out,
        frame: true,
        grid: true,
        preset: ta_core::JournalPreset::Custom,
    }
}

/// The plate's panel description and its curves.
pub struct PlateContent {
    pub panel: PanelModel,
    pub series: Vec<PlottedSeries>,
}

/// The accent the cursors and the crosshair use.
pub fn accent() -> Rgba {
    Rgba::from_hex("#b02a37").expect("the accent is a literal colour")
}

/// The ink of a hairline rule.
pub fn rule() -> Rgba {
    Rgba::from_hex("#c9ced9").expect("the rule colour is a literal")
}

/// The data colours, reserved for the sign of the signal.
pub fn cool() -> Rgba {
    Rgba::from_hex("#3b4cc0").expect("a literal colour")
}

pub fn warm() -> Rgba {
    Rgba::from_hex("#b40426").expect("a literal colour")
}

/// The colour limits of the open matrix, symmetric about zero.
///
/// The scale is trimmed to the 99th percentile of the magnitude rather than the
/// extremes: one hot pixel otherwise owns the whole scale and the field reads as
/// washed out. The limits are shown in the caption and the inspector, so nothing
/// is hidden by the trim.
pub fn colour_limits(session: &Session) -> ColorLimits {
    let mut limits = ColorLimits {
        min: -1.0,
        max: 1.0,
        symmetric_about_zero: true,
        auto: ta_core::AutoRange::Percentile(99.0),
    };
    if let Some(dataset) = session.dataset() {
        if let Some(version) = session.version_id() {
            if let Ok(block) = dataset.block(version) {
                limits.fit(block.finite_values());
            }
        }
    }
    limits
}

/// The contour's colour specification.
pub fn contour_spec(session: &Session) -> ContourSpec {
    ContourSpec {
        colormap: Colormap::preset("coolwarm").expect("a built-in preset"),
        limits: colour_limits(session),
        levels: Vec::new(),
        level_count: 8,
        show_lines: true,
        line_color: Rgba::rgba(rule().r, rule().g, rule().b, 150),
        line_width: 1.0,
        smooth: false,
    }
}

/// (a) the contour: the matrix as an image, ruled by its isolines.
pub fn contour_content(session: &Session, spec: &ContourSpec) -> PlateContent {
    let mut panel = PanelModel::new("");
    panel.x = axis_model("Wavelength", Some("nm"), AxisScale::Linear);
    // The delay axis of the contour stays linear: a contour is read for the
    // shape of the whole matrix, and the logarithmic view belongs to the trace.
    panel.y = axis_model("Delay", Some("ps"), AxisScale::Linear);
    panel.contour = Some(spec.clone());
    // A contour is read through its colour bar, not a legend.
    panel.legend.visible = false;

    let mut series = Vec::new();
    if let (Some(dataset), Some(version)) = (session.dataset(), session.version_id()) {
        if let (Ok((wavelength, delay)), Ok(matrix)) = (dataset.axes(version), dataset.matrix(version))
        {
            panel.x.range = Some(wavelength.range());
            panel.y.range = Some(delay.range());
            if spec.show_lines {
                let Ok(grid) = ContourGrid::from_matrix(wavelength, delay, matrix) else {
                    return PlateContent { panel, series };
                };
                let boundaries = ta_plot::colormap::band_levels(
                    &spec.limits,
                    &spec.levels,
                    spec.level_count,
                );
                let mut segments = Vec::new();
                for level in boundaries.iter().skip(1).take(boundaries.len().saturating_sub(2)) {
                    segments.extend(isolines(&grid, *level));
                }
                if !segments.is_empty() {
                    series.push(
                        PlottedSeries::new("", Vec::new(), line_style(spec.line_color, 1.0))
                            .with_segments(segments),
                    );
                }
            }
        }
    }
    PlateContent { panel, series }
}

/// (b) kinetics: the kept comparison curves and the linked trace.
pub fn kinetics_content(session: &Session) -> PlateContent {
    let mut panel = PanelModel::new("");
    panel.x = axis_model("Delay", Some("ps"), AxisScale::SplitLinearLog { break_at: 1.0 });
    panel.y = axis_model("ΔOD", None, AxisScale::Linear);
    let traces = session.kinetics();
    let mut series = Vec::new();
    let mut bounds: Option<(f64, f64)> = None;
    for (index, trace) in traces.iter().enumerate() {
        if index + 1 == traces.len() {
            panel.x.range = Some((trace.delay[0], trace.delay[trace.len() - 1]));
        }
        let points: Vec<(f64, f64)> = trace
            .delay
            .iter()
            .copied()
            .zip(trace.signal.iter().copied())
            .filter(|(x, y)| x.is_finite() && y.is_finite())
            .collect();
        // The comparison curves are dashed and cool; the linked trace is the
        // accented one.
        let style = if index + 1 == traces.len() {
            line_style(accent(), 1.5)
        } else {
            dashed(line_style(cool(), 1.0))
        };
        for (_, y) in &points {
            bounds = Some(match bounds {
                None => (*y, *y),
                Some((low, high)) => (low.min(*y), high.max(*y)),
            });
        }
        series.push(
            PlottedSeries::new(format!("{:.2}", trace.sampled.coordinate), points, style)
                .with_kind(SeriesKind::Line),
        );
    }
    if let Some((low, high)) = bounds {
        let pad = ((high - low).abs() * 0.08).max(1e-4);
        panel.y.range = Some((low - pad, high + pad));
    }
    PlateContent { panel, series }
}

/// (c) spectra: the kept comparison curves and the linked spectrum.
pub fn spectra_content(session: &Session) -> PlateContent {
    let mut panel = PanelModel::new("");
    panel.x = axis_model("Wavelength", Some("nm"), AxisScale::Linear);
    panel.y = axis_model("ΔOD", None, AxisScale::Linear);
    let spectra = session.spectra();
    let mut series = Vec::new();
    let mut bounds: Option<(f64, f64)> = None;
    for (index, spectrum) in spectra.iter().enumerate() {
        if index == 0 {
            panel.x.range = Some((spectrum.wavelength[0], spectrum.wavelength[spectrum.len() - 1]));
        }
        let points: Vec<(f64, f64)> = spectrum
            .wavelength
            .iter()
            .copied()
            .zip(spectrum.signal.iter().copied())
            .filter(|(x, y)| x.is_finite() && y.is_finite())
            .collect();
        let style = if index + 1 == spectra.len() {
            line_style(accent(), 1.5)
        } else {
            dashed(line_style(cool(), 1.0))
        };
        for (_, y) in &points {
            bounds = Some(match bounds {
                None => (*y, *y),
                Some((low, high)) => (low.min(*y), high.max(*y)),
            });
        }
        series.push(PlottedSeries::new(&spectrum.label, points, style));
    }
    if let Some((low, high)) = bounds {
        let pad = ((high - low).abs() * 0.08).max(1e-4);
        panel.y.range = Some((low - pad, high + pad));
    }
    PlateContent { panel, series }
}

/// (d) the time-zero correction curve, when the chain holds one.
pub fn time_zero_content(session: &Session) -> PlateContent {
    let mut panel = PanelModel::new("");
    panel.x = axis_model("Wavelength", Some("nm"), AxisScale::Linear);
    panel.y = axis_model("Time zero", Some("ps"), AxisScale::Linear);
    panel.legend.visible = false;

    let Some(dataset) = session.dataset() else {
        return PlateContent {
            panel,
            series: Vec::new(),
        };
    };
    let curve = dataset
        .versions
        .iter()
        .filter_map(|version| match &version.op {
            Some(ta_core::ProcessingOp::TimeZero(correction)) => Some(correction.curve.clone()),
            _ => None,
        })
        .next_back();
    let Some(curve) = curve else {
        // An axis waiting for a curve: the dataset's own wavelength range, so
        // the empty plate reads as "nothing is set yet" rather than as 0 to 1.
        if let (Some(dataset), Some(version)) = (session.dataset(), session.version_id()) {
            if let Ok((wavelength, _)) = dataset.axes(version) {
                panel.x.range = Some(wavelength.range());
            }
        }
        panel.y.range = Some((-0.5, 0.5));
        return PlateContent {
            panel,
            series: Vec::new(),
        };
    };
    let sorted = curve.sorted();
    let points: Vec<(f64, f64)> = sorted
        .iter()
        .map(|point| (point.wavelength, point.time_zero))
        .collect();
    if let (Some(first), Some(last)) = (points.first(), points.last()) {
        panel.x.range = Some((first.0, last.0));
        let low = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
        let high = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
        panel.y.range = Some((low - 0.05, high + 0.05));
        let _ = last;
    }
    let style = SeriesStyle {
        marker: ta_core::MarkerShape::Circle,
        ..line_style(warm(), 1.5)
    };
    PlateContent {
        panel,
        series: vec![PlottedSeries::new("correction", points, style)],
    }
}

/// The scene for one plate.
pub fn content(session: &Session, plate: PlateId, spec: &ContourSpec) -> PlateContent {
    match plate {
        PlateId::Contour => contour_content(session, spec),
        PlateId::Kinetics => kinetics_content(session),
        PlateId::Spectra => spectra_content(session),
        PlateId::TimeZero => time_zero_content(session),
    }
}

fn axis_model(label: &str, unit: Option<&str>, scale: AxisScale) -> AxisModel {
    AxisModel {
        label: label.to_string(),
        unit: unit.map(|unit| unit.to_string()),
        scale,
        range: None,
        reversed: false,
        major_ticks: Some(6),
        minor_ticks: false,
        grid: true,
        tick_label_size: 9.0,
        title_size: 10.0,
    }
}

fn dashed(mut style: SeriesStyle) -> SeriesStyle {
    style.dash = DashPattern::Dashed;
    style
}

fn line_style(color: Rgba, width: f32) -> SeriesStyle {
    SeriesStyle {
        color,
        width,
        dash: DashPattern::Solid,
        marker: ta_core::MarkerShape::None,
        marker_size: 3.0,
        opacity: 1.0,
    }
}

/// The cursor lines a plate shows for the current selection.
pub fn cursors(session: &Session, plate: PlateId) -> Vec<ta_plot_gpui::PlotCursor> {
    let mut cursors = Vec::new();
    match plate {
        PlateId::Contour => {
            if let Some(trace) = &session.trace {
                cursors.push(ta_plot_gpui::PlotCursor {
                    axis: AxisKind::Wavelength,
                    value: trace.wavelength.coordinate,
                    label: format!("{:.2} nm", trace.wavelength.coordinate),
                });
            }
            if let Some(spectrum) = &session.spectrum {
                cursors.push(ta_plot_gpui::PlotCursor {
                    axis: AxisKind::Delay,
                    value: spectrum.delay.coordinate,
                    label: format!("{:.3} ps", spectrum.delay.coordinate),
                });
            }
        }
        PlateId::Kinetics => {
            if let Some(spectrum) = &session.spectrum {
                cursors.push(ta_plot_gpui::PlotCursor {
                    axis: AxisKind::Delay,
                    value: spectrum.delay.coordinate,
                    label: format!("{:.3} ps", spectrum.delay.coordinate),
                });
            }
        }
        PlateId::Spectra => {
            if let Some(trace) = &session.trace {
                cursors.push(ta_plot_gpui::PlotCursor {
                    axis: AxisKind::Wavelength,
                    value: trace.wavelength.coordinate,
                    label: format!("{:.2} nm", trace.wavelength.coordinate),
                });
            }
        }
        PlateId::TimeZero => {}
    }
    cursors
}

/// The figure a plate exports, with the plate's caption kept as its title so an
/// exported file says what it is.
pub fn export_figure(session: &Session, plate: PlateId, spec: &ContourSpec) -> ta_core::FigureModel {
    let content = content(session, plate, spec);
    let mut figure = ta_core::FigureModel::single_panel(session.caption(plate));
    figure.size = ta_core::JournalPreset::JacsSingleColumn.size();
    figure.panels[0] = content.panel;
    figure
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;
    use ndarray::Array2;
    use ta_core::{
        Axis, AxisKind as Kind, AxisUnit, SignalUnit, SourceFormat, TaDataset,
    };

    fn session() -> Session {
        let wavelength = Axis::new(
            Kind::Wavelength,
            AxisUnit::Nanometer,
            "Wavelength",
            (0..40).map(|i| 500.0 + i as f64 * 5.0).collect(),
        )
        .unwrap();
        let delay = Axis::new(
            Kind::Delay,
            AxisUnit::Picosecond,
            "Delay",
            vec![-1.0, 0.0, 1.0, 10.0, 100.0],
        )
        .unwrap();
        let mut data = Array2::zeros((40, 5));
        for row in 0..40 {
            for column in 0..5 {
                let x = (row as f64 - 20.0) / 8.0;
                data[[row, column]] = (-x * x).exp() * 0.1 - column as f64 * 0.002;
            }
        }
        let dataset = TaDataset::new_raw(
            "synthetic",
            wavelength,
            delay,
            data,
            SignalUnit::DeltaOpticalDensity,
            SourceFormat::Synthetic,
        )
        .unwrap();
        let mut session = Session::new();
        session.open_dataset(dataset);
        session
    }

    #[test]
    fn the_contour_scene_carries_the_image_limits_and_its_isolines() {
        let session = session();
        let spec = contour_spec(&session);
        // Limits are symmetric about zero, which is what a signed field needs.
        assert!((spec.limits.min + spec.limits.max).abs() < 1e-12);
        assert!(spec.limits.max > 0.0);

        let content = contour_content(&session, &spec);
        assert!(content.panel.contour.is_some());
        assert_eq!(content.panel.x.range, Some((500.0, 695.0)));
        assert_eq!(content.panel.y.range, Some((-1.0, 100.0)));
        // The isolines are one series of many segments, not one series each.
        assert_eq!(content.series.len(), 1);
        assert!(content.series[0].segments.len() > 1);
    }

    #[test]
    fn the_kinetics_scene_draws_the_linked_trace_at_the_end() {
        let mut session = session();
        session.select_wavelength(600.0).unwrap();
        session.save_current_trace().unwrap();
        session.select_wavelength(550.0).unwrap();
        let content = kinetics_content(&session);
        assert_eq!(content.series.len(), 2);
        // The linked trace is last, so it is drawn on top.
        assert_eq!(content.series[1].style.color, accent());
        assert_eq!(content.series[0].style.dash, DashPattern::Dashed);
        // The delay axis uses the split linear/logarithmic convention.
        assert_eq!(
            content.panel.x.scale,
            AxisScale::SplitLinearLog { break_at: 1.0 }
        );
        assert!(content.panel.y.range.is_some());
    }

    #[test]
    fn a_kinetics_plate_with_nothing_selected_has_no_curves_rather_than_a_line_at_zero() {
        let session = Session::new();
        let content = kinetics_content(&session);
        assert!(content.series.is_empty());
        assert_eq!(content.panel.y.range, None);
    }

    #[test]
    fn the_spectra_scene_uses_the_wavelength_axis_and_shared_limits() {
        let session = session();
        let content = spectra_content(&session);
        assert_eq!(content.series.len(), 1);
        assert_eq!(content.panel.x.range, Some((500.0, 695.0)));
        let (low, high) = content.panel.y.range.unwrap();
        assert!(low < high);
    }

    #[test]
    fn the_time_zero_plate_is_empty_until_a_correction_exists() {
        let session = session();
        let content = time_zero_content(&session);
        assert!(content.series.is_empty());
        assert!(!content.panel.legend.visible);
        assert!(session
            .caption(PlateId::TimeZero)
            .contains("No time-zero correction"));
    }

    #[test]
    fn cursors_stand_on_the_axis_the_selection_belongs_to() {
        let session = session();
        let contour = cursors(&session, PlateId::Contour);
        assert_eq!(contour.len(), 2);
        assert!(contour.iter().any(|cursor| cursor.axis == AxisKind::Wavelength));
        assert!(contour.iter().any(|cursor| cursor.axis == AxisKind::Delay));

        let kinetics = cursors(&session, PlateId::Kinetics);
        assert_eq!(kinetics.len(), 1);
        assert_eq!(kinetics[0].axis, AxisKind::Delay);

        let spectra = cursors(&session, PlateId::Spectra);
        assert_eq!(spectra.len(), 1);
        assert_eq!(spectra[0].axis, AxisKind::Wavelength);
    }

    #[test]
    fn an_exported_plate_carries_its_caption() {
        let session = session();
        let figure = export_figure(&session, PlateId::Kinetics, &contour_spec(&session));
        assert!(figure.title.contains("Kinetics at"));
        assert_eq!(figure.panels.len(), 1);
        assert_eq!(figure.size.width, 3.25);
    }
}
