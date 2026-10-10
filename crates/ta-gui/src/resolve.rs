//! Reading saved series definitions back into drawable curves.
//!
//! A comparison workspace and a figure hold *references* — a dataset, a version,
//! a coordinate — never copies of a matrix. This is where a reference becomes
//! numbers: the same module serves the comparison panes, the figure preview and
//! the data export, so what a user sees and what they export cannot drift apart.

use ta_core::{
    kinetic_trace, transient_spectrum, Alignment, Normalization, PanelModel, PlotSeries, Project,
    SeriesKind, SeriesSource, SeriesTransform,
};
use ta_plot::PlottedSeries;

/// One resolved curve, with what the caption needs to name it.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub series: usize,
    pub label: String,
    /// The transform that was applied, after normalization had its turn.
    pub transform: SeriesTransform,
    pub normalized: bool,
    pub points: Vec<(f64, f64)>,
    pub style: ta_core::SeriesStyle,
    pub kind: SeriesKind,
    pub z: i32,
    /// The requested coordinate, and the coordinate actually sampled.
    pub requested: Option<f64>,
    pub sampled: Option<f64>,
    /// The dataset and version the numbers came from.
    pub dataset: Option<ta_core::DatasetId>,
    pub version: Option<ta_core::VersionId>,
}

impl Resolved {
    pub fn plotted(&self) -> PlottedSeries {
        PlottedSeries::new(self.label.clone(), self.points.clone(), self.style.clone())
            .with_kind(self.kind)
    }
}

/// How a panel's series are turned into curves.
#[derive(Clone, Copy, Debug)]
pub struct ResolveOptions {
    pub normalization: Normalization,
    pub alignment: Alignment,
    /// Drop the series the user has hidden.
    pub visible_only: bool,
}

impl Default for ResolveOptions {
    fn default() -> Self {
        Self {
            normalization: Normalization::None,
            alignment: Alignment::AsMeasured,
            visible_only: false,
        }
    }
}

/// Every curve of a panel, in paint order.
pub fn resolve_panel(
    project: &Project,
    panel: &PanelModel,
    options: &ResolveOptions,
) -> Vec<Resolved> {
    let mut resolved: Vec<Resolved> = panel
        .series
        .iter()
        .enumerate()
        .filter(|(_, series)| !options.visible_only || series.visible)
        .filter_map(|(index, series)| resolve_one(project, series, index, panel, options))
        .collect();
    resolved.sort_by_key(|curve| curve.z);
    resolved
}

/// One series as numbers, or `None` when it is not a curve this panel can draw.
pub fn resolve_one(
    project: &Project,
    series: &PlotSeries,
    index: usize,
    panel: &PanelModel,
    options: &ResolveOptions,
) -> Option<Resolved> {
    let mut resolved = match &series.source {
        SeriesSource::Trace(selection) => {
            let dataset = project.dataset(selection.dataset).ok()?;
            let trace = kinetic_trace(dataset, selection).ok()?;
            let mut points: Vec<(f64, f64)> = Vec::new();
            for (x, y) in trace.delay.iter().copied().zip(trace.signal.iter().copied()) {
                if x.is_finite() && y.is_finite() {
                    points.push((x, y));
                }
            }
            // A trace can be aligned on the delay axis; a spectrum cannot,
            // because its correction varies along the axis it is plotted on.
            // The alignment shift and the user's own shift both apply, and they
            // are different things: one comes from the physics, the other from
            // the display.
            let aligned = match options.alignment {
                Alignment::AsMeasured => 0.0,
                Alignment::ToTimeZero => {
                    time_zero_offset(dataset, selection.version, trace.sampled.coordinate)
                }
            };
            Resolved {
                series: index,
                label: default_label(dataset.name.clone(), &trace.label),
                transform: SeriesTransform {
                    coordinate_offset: aligned + series.display.coordinate_offset,
                    ..series.display
                },
                normalized: false,
                points,
                style: series.style.clone(),
                kind: series.kind,
                z: series.z,
                requested: Some(selection.wavelength.coordinate),
                sampled: Some(trace.sampled.coordinate),
                dataset: Some(selection.dataset),
                version: Some(selection.version),
            }
        }
        SeriesSource::Spectrum(selection) => {
            let dataset = project.dataset(selection.dataset).ok()?;
            let spectrum = transient_spectrum(dataset, selection).ok()?;
            let points: Vec<(f64, f64)> = spectrum
                .wavelength
                .iter()
                .copied()
                .zip(spectrum.signal.iter().copied())
                .filter(|(x, y)| x.is_finite() && y.is_finite())
                .collect();
            Resolved {
                series: index,
                label: default_label(dataset.name.clone(), &spectrum.label),
                transform: series.display,
                normalized: false,
                points,
                style: series.style.clone(),
                kind: series.kind,
                z: series.z,
                requested: Some(selection.delay.coordinate),
                sampled: Some(spectrum.sampled.coordinate),
                dataset: Some(selection.dataset),
                version: Some(selection.version),
            }
        }
        SeriesSource::Curve {
            x,
            y,
            x_label,
            y_label,
            ..
        } => {
            let points: Vec<(f64, f64)> = x
                .iter()
                .copied()
                .zip(y.iter().copied())
                .filter(|(x, y)| x.is_finite() && y.is_finite())
                .collect();
            Resolved {
                series: index,
                label: if series.label.is_empty() {
                    format!("{y_label} against {x_label}")
                } else {
                    series.label.clone()
                },
                transform: series.display,
                normalized: false,
                points,
                style: series.style.clone(),
                kind: series.kind,
                z: series.z,
                requested: None,
                sampled: None,
                dataset: None,
                version: None,
            }
        }
        SeriesSource::Fit {
            dataset: source,
            version,
            selection,
            fit,
        } => {
            let dataset = project.dataset(*source).ok()?;
            let record = dataset.fit(*fit)?;
            let trace = kinetic_trace(dataset, selection).ok()?;
            let params = ta_fitting::ModelParams::from_parameters(&record.params).ok()?;
            let curve = ta_fitting::model::evaluate(record.model.into(), &params, &trace.delay);
            let points: Vec<(f64, f64)> = trace
                .delay
                .iter()
                .copied()
                .zip(curve.iter().copied())
                .filter(|(x, y)| x.is_finite() && y.is_finite())
                .collect();
            let aligned = match options.alignment {
                Alignment::AsMeasured => 0.0,
                Alignment::ToTimeZero => {
                    time_zero_offset(dataset, *version, trace.sampled.coordinate)
                }
            };
            Resolved {
                series: index,
                label: if series.label.is_empty() {
                    format!("{} fit", record.model.label())
                } else {
                    series.label.clone()
                },
                transform: SeriesTransform {
                    coordinate_offset: aligned + series.display.coordinate_offset,
                    ..series.display
                },
                normalized: false,
                points,
                style: series.style.clone(),
                kind: series.kind,
                z: series.z,
                requested: Some(selection.wavelength.coordinate),
                sampled: Some(trace.sampled.coordinate),
                dataset: Some(*source),
                version: Some(*version),
            }
        }
        // A matrix is drawn by the contour plate, not as a curve.
        SeriesSource::Heatmap { .. }
        | SeriesSource::Contour { .. }
        | SeriesSource::ContourLines { .. } => return None,
    };

    // The legend's own label wins over the generated one.
    if let Some(label) = panel.legend.labels.get(&series.id) {
        if !label.is_empty() {
            resolved.label = label.clone();
        }
    }
    if !series.label.is_empty() {
        resolved.label = series.label.clone();
    }

    if !resolved.points.is_empty() {
        // The factor folds into the transform, so the numbers are moved exactly
        // once and the legend can report what was done to them.
        let (normalized, factor) = normalization_factor(&resolved.points, options.normalization);
        if normalized {
            resolved.normalized = true;
            resolved.transform.scale *= factor;
        }
    }
    let transform = resolved.transform;
    for (x, y) in &mut resolved.points {
        *x += transform.coordinate_offset;
        *y = *y * transform.scale + transform.offset;
    }
    // A shift can reorder the points on a descending axis; comparisons draw
    // whatever was measured, in the order it was measured, so the sort is by
    // coordinate and only when the axis descends.
    if resolved.points.len() > 1 && resolved.points[0].0 > resolved.points[1].0 {
        resolved.points.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    Some(resolved)
}

/// The factor a normalization asks for, and whether it applies.
fn normalization_factor(points: &[(f64, f64)], normalization: Normalization) -> (bool, f64) {
    match normalization {
        Normalization::None => (false, 1.0),
        Normalization::MaxAbs => {
            let peak = points.iter().map(|(_, y)| y.abs()).fold(0.0f64, f64::max);
            if peak > 0.0 && peak.is_finite() {
                (true, 1.0 / peak)
            } else {
                (false, 1.0)
            }
        }
        Normalization::AtDelay(delay) => {
            let value = interpolate(points, delay);
            match value {
                Some(value) if value.abs() > f64::MIN_POSITIVE && value.is_finite() => {
                    (true, 1.0 / value)
                }
                // A reference point that is missing or zero cannot normalize
                // anything, and dividing by it would invent a curve.
                _ => (false, 1.0),
            }
        }
    }
}

/// The signal at a delay, linearly interpolated between the measured points.
pub fn interpolate(points: &[(f64, f64)], x: f64) -> Option<f64> {
    if points.is_empty() || !x.is_finite() {
        return None;
    }
    if x < points[0].0 || x > points[points.len() - 1].0 {
        return None;
    }
    for pair in points.windows(2) {
        let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
        if x >= x0 && x <= x1 {
            if (x1 - x0).abs() < f64::MIN_POSITIVE {
                return Some(y0);
            }
            let t = (x - x0) / (x1 - x0);
            return Some(y0 + (y1 - y0) * t);
        }
    }
    None
}

/// The time-zero offset a dataset's own correction curve gives at a wavelength.
fn time_zero_offset(
    dataset: &ta_core::TaDataset,
    version: ta_core::VersionId,
    wavelength: f64,
) -> f64 {
    dataset
        .versions
        .iter()
        .filter(|candidate| candidate.inputs.contains(&version) || candidate.id == version)
        .filter_map(|candidate| match &candidate.op {
            Some(ta_core::ProcessingOp::TimeZero(correction)) => Some(correction.curve.clone()),
            _ => None,
        })
        .next_back()
        .and_then(|curve| ta_processing::offset_at(&curve, wavelength).ok())
        .unwrap_or(0.0)
}

/// A label that names the dataset as well as the coordinate, because a
/// comparison overlay is exactly where two datasets look alike.
fn default_label(dataset: String, coordinate: &str) -> String {
    format!("{dataset} · {coordinate}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::Array2;
    use ta_core::{
        Axis, AxisKind, AxisRequest, AxisUnit, DatasetId, PanelModel, SignalUnit, SourceFormat,
        SpectrumSelection, TaDataset, TraceSelection,
    };

    fn dataset(name: &str, scale: f64) -> TaDataset {
        let wavelength = Axis::new(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            "Wavelength",
            vec![500.0, 600.0, 700.0],
        )
        .unwrap();
        let delay = Axis::new(
            AxisKind::Delay,
            AxisUnit::Picosecond,
            "Delay",
            vec![-1.0, 0.0, 1.0, 10.0],
        )
        .unwrap();
        let mut data = Array2::zeros((3, 4));
        for row in 0..3 {
            for column in 0..4 {
                data[[row, column]] = scale * (row as f64 + 1.0) * (column as f64 - 1.0) * 0.01;
            }
        }
        TaDataset::new_raw(
            name,
            wavelength,
            delay,
            data,
            SignalUnit::DeltaOpticalDensity,
            SourceFormat::Synthetic,
        )
        .unwrap()
    }

    fn project() -> (Project, DatasetId, DatasetId) {
        let mut project = Project::new("test");
        let first = project.add_dataset(dataset("sample A", 1.0));
        let second = project.add_dataset(dataset("sample B", 2.0));
        (project, first, second)
    }

    fn trace_series(project: &Project, id: DatasetId, wavelength: f64) -> PlotSeries {
        let dataset = project.dataset(id).unwrap();
        PlotSeries::new(
            "",
            SeriesSource::Trace(TraceSelection::new(
                id,
                dataset.active_version,
                AxisRequest::nearest(wavelength),
            )),
        )
    }

    #[test]
    fn a_trace_resolves_to_the_measured_delays_and_signals() {
        let (project, first, _) = project();
        let series = trace_series(&project, first, 600.0);
        let mut panel = PanelModel::new("");
        panel.series.push(series);

        let resolved = resolve_panel(&project, &panel, &ResolveOptions::default());
        assert_eq!(resolved.len(), 1);
        let curve = &resolved[0];
        assert_eq!(curve.points.len(), 4);
        // 600 nm is the second row: 2 × (delay index − 1) × 0.01.
        assert_eq!(curve.points[0], (-1.0, -0.02));
        assert_eq!(curve.points[2], (1.0, 0.02));
        assert!((curve.points[3].1 - 0.04).abs() < 1e-12);
        // The label names the dataset, which is what a comparison needs.
        assert!(curve.label.contains("sample A"));
        assert!(curve.label.contains("600.00"));
    }

    #[test]
    fn two_datasets_keep_their_own_coordinates_when_overlaid() {
        let (project, first, second) = project();
        let mut panel = PanelModel::new("");
        panel.series.push(trace_series(&project, first, 600.0));
        panel.series.push(trace_series(&project, second, 700.0));
        let resolved = resolve_panel(&project, &panel, &ResolveOptions::default());
        assert_eq!(resolved.len(), 2);
        // Same delay axis, different amplitudes: nothing was resampled onto a
        // common grid, which §15.4 requires.
        assert_eq!(resolved[0].points[0].0, resolved[1].points[0].0);
        // Sample B is the same shape at twice the amplitude, and its third row
        // is three times the first: six times A's value at the same delay.
        assert!((resolved[1].points[3].1 / resolved[0].points[3].1 - 3.0).abs() < 1e-9);
    }

    #[test]
    fn normalizing_to_the_positive_maximum_scales_each_curve_itself() {
        let (project, first, second) = project();
        let mut panel = PanelModel::new("");
        panel.series.push(trace_series(&project, first, 600.0));
        panel.series.push(trace_series(&project, second, 700.0));
        let options = ResolveOptions {
            normalization: Normalization::MaxAbs,
            ..ResolveOptions::default()
        };
        let resolved = resolve_panel(&project, &panel, &options);
        for curve in &resolved {
            let peak = curve.points.iter().map(|(_, y)| y.abs()).fold(0.0, f64::max);
            assert!((peak - 1.0).abs() < 1e-12, "peak was {peak}");
            assert!(curve.normalized);
        }
    }

    #[test]
    fn normalizing_at_a_delay_divides_by_the_value_there() {
        let (project, first, _) = project();
        let mut panel = PanelModel::new("");
        panel.series.push(trace_series(&project, first, 600.0));
        let options = ResolveOptions {
            normalization: Normalization::AtDelay(1.0),
            ..ResolveOptions::default()
        };
        let resolved = resolve_panel(&project, &panel, &options);
        let curve = &resolved[0];
        let at_one = curve
            .points
            .iter()
            .find(|(x, _)| (*x - 1.0).abs() < 1e-12)
            .map(|(_, y)| *y)
            .unwrap();
        assert!((at_one - 1.0).abs() < 1e-12);
    }

    #[test]
    fn a_reference_point_of_zero_leaves_the_curve_alone_instead_of_dividing_by_it() {
        let (project, first, _) = project();
        // The 500 nm trace is zero at -1 ps and at 0 ps.
        let mut panel = PanelModel::new("");
        panel.series.push(trace_series(&project, first, 500.0));
        let options = ResolveOptions {
            normalization: Normalization::AtDelay(0.0),
            ..ResolveOptions::default()
        };
        let resolved = resolve_panel(&project, &panel, &options);
        assert!(!resolved[0].normalized);
        assert!(resolved[0].points.iter().all(|(_, y)| y.is_finite()));
    }

    #[test]
    fn a_display_transform_scales_and_offsets_without_touching_the_data() {
        let (project, first, _) = project();
        let mut series = trace_series(&project, first, 600.0);
        series.display = SeriesTransform {
            scale: 10.0,
            offset: 1.0,
            coordinate_offset: 0.5,
        };
        let mut panel = PanelModel::new("");
        panel.series.push(series);
        let resolved = resolve_panel(&project, &panel, &ResolveOptions::default());
        let curve = &resolved[0];
        assert_eq!(curve.points[0].0, -0.5);
        // -0.02 at -1 ps, scaled by ten and lifted by one.
        assert!((curve.points[0].1 - 0.8).abs() < 1e-12);
        // 0.04 at 10 ps becomes 1.4.
        assert!((curve.points[3].1 - 1.4).abs() < 1e-12);
        assert!((curve.transform.describe().contains("×10.0000")));
    }

    #[test]
    fn hiding_a_series_removes_it_from_the_drawing_but_not_from_the_model() {
        let (project, first, second) = project();
        let mut panel = PanelModel::new("");
        panel.series.push(trace_series(&project, first, 600.0));
        let mut hidden = trace_series(&project, second, 700.0);
        hidden.visible = false;
        panel.series.push(hidden);
        let options = ResolveOptions {
            visible_only: true,
            ..ResolveOptions::default()
        };
        assert_eq!(resolve_panel(&project, &panel, &options).len(), 1);
        // The model still holds both, so hiding is not deletion.
        assert_eq!(panel.series.len(), 2);
    }

    #[test]
    fn a_legend_label_override_names_the_curve() {
        let (project, first, _) = project();
        let mut panel = PanelModel::new("");
        let series = trace_series(&project, first, 600.0);
        let id = series.id;
        panel.series.push(series);
        panel.legend.labels.insert(id, "10 ps after injection".to_string());
        let resolved = resolve_panel(&project, &panel, &ResolveOptions::default());
        assert_eq!(resolved[0].label, "10 ps after injection");
    }

    #[test]
    fn a_spreadsheet_curve_resolves_without_a_dataset() {
        let (project, _, _) = project();
        let mut panel = PanelModel::new("");
        panel.series.push(PlotSeries::new(
            "imported",
            SeriesSource::Curve {
                x: vec![0.0, 1.0],
                y: vec![2.0, 3.0],
                x_label: "delay".into(),
                y_label: "ΔOD".into(),
                unit: SignalUnit::DeltaOpticalDensity,
            },
        ));
        let resolved = resolve_panel(&project, &panel, &ResolveOptions::default());
        assert_eq!(resolved[0].label, "imported");
        assert_eq!(resolved[0].points, vec![(0.0, 2.0), (1.0, 3.0)]);
    }

    #[test]
    fn a_series_whose_dataset_is_gone_is_dropped_rather_than_drawn_at_zero() {
        let (mut project, first, _) = project();
        let mut panel = PanelModel::new("");
        panel.series.push(trace_series(&project, first, 600.0));
        project.remove_dataset(first).unwrap();
        assert!(resolve_panel(&project, &panel, &ResolveOptions::default()).is_empty());
    }

    #[test]
    fn a_spectrum_resolves_along_the_wavelength_axis() {
        let (project, first, _) = project();
        let dataset = project.dataset(first).unwrap();
        let mut panel = PanelModel::new("");
        panel.series.push(PlotSeries::new(
            "",
            SeriesSource::Spectrum(SpectrumSelection::new(
                first,
                dataset.active_version,
                AxisRequest::nearest(1.0),
            )),
        ));
        let resolved = resolve_panel(&project, &panel, &ResolveOptions::default());
        assert_eq!(resolved[0].points.len(), 3);
        assert_eq!(resolved[0].points[0].0, 500.0);
    }

    #[test]
    fn the_fitted_curve_of_a_retained_fit_resolves() {
        let (mut project, first, _) = project();
        // Fit the 600 nm trace with one exponential, then reference the record.
        let dataset = project.dataset(first).unwrap();
        let version = dataset.active_version;
        let selection = TraceSelection::new(first, version, AxisRequest::nearest(600.0));
        let trace = kinetic_trace(dataset, &selection).unwrap();
        let request = ta_fitting::FitRequest {
            dataset: first,
            version,
            selection: selection.clone(),
            model: ta_core::FitModelKind::LegacyGaussianCdfIrf,
            components: 1,
            parameters: ta_core::legacy_parameter_template(1),
            fit_range: None,
            excluded: Vec::new(),
            weighting: ta_core::Weighting::Uniform,
            options: ta_fitting::SolverOptions {
                max_iterations: 40,
                ..Default::default()
            },
        };
        let record = ta_fitting::fit_trace(&trace, &request).unwrap();
        let id = record.id;
        project.dataset_mut(first).unwrap().fits.push(record);

        let mut panel = PanelModel::new("");
        panel.series.push(PlotSeries::new(
            "",
            SeriesSource::Fit {
                dataset: first,
                version,
                selection,
                fit: id,
            },
        ));
        let resolved = resolve_panel(&project, &panel, &ResolveOptions::default());
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].points.len(), 4);
        assert!(resolved[0].label.contains("fit"));
    }
}
