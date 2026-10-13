//! The multi-object comparison workspace (§15).
//!
//! A comparison holds *references*: a dataset, a version, a coordinate. Two
//! panes — transient spectra on the left, kinetic traces on the right — resolve
//! those references through `resolve`, so a curve that is drawn and a curve that
//! is exported are the same numbers. Nothing here resamples a matrix: traces
//! from datasets with different delay axes keep their own coordinates, and
//! interpolation happens only where a mathematical operation asks for it.

use crate::app::{ink, Workspace};
use crate::plates::{self, PlateContent};
use crate::resolve::{resolve_panel, ResolveOptions, Resolved};
use crate::session::Session;
use crate::widgets::{self, FieldId};
use gpui_kit::{
    div, px, AnyElement, Context, ElementId, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window,
};
use ta_core::{
    kinetic_trace, Alignment, ComparisonWorkspace, DashPattern, MarkerShape, Normalization,
    PlotSeries, Rgba, Result, SeriesId, SeriesSource, SeriesStyle, TaDataset, TraceSelection,
    VersionId,
};

/// Which pane a series is added to, which decides what a click means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeriesTarget {
    /// A kinetic trace at one wavelength.
    Trace,
    /// A transient spectrum at one delay.
    Spectrum,
}

/// The colours a comparison cycles through. The first two are the diverging
/// pair, because the first two curves are the ones a reader compares.
pub fn palette(index: usize) -> Rgba {
    const COLORS: [&str; 6] = [
        "#3b4cc0", "#b40426", "#101526", "#b8791f", "#2f7f6f", "#6b4fa0",
    ];
    Rgba::from_hex(COLORS[index % COLORS.len()]).expect("the palette is literal")
}

impl Session {
    /// The comparison on the bench, creating one the first time it is asked
    /// for — a workspace with nothing in it is still a workspace.
    pub fn comparison(&self) -> Option<&ComparisonWorkspace> {
        self.project.comparisons.get(self.comparison)
    }

    pub fn comparison_mut(&mut self) -> &mut ComparisonWorkspace {
        if self.project.comparisons.is_empty() {
            self.project
                .comparisons
                .push(ComparisonWorkspace::new("Comparison 1"));
            self.comparison = 0;
        }
        let index = self.comparison.min(self.project.comparisons.len() - 1);
        &mut self.project.comparisons[index]
    }

    pub fn comparison_series(&self) -> Vec<PlotSeries> {
        self.comparison()
            .map(|workspace| workspace.series.clone())
            .unwrap_or_default()
    }

    pub fn comparison_options(&self) -> ResolveOptions {
        match self.comparison() {
            Some(workspace) => ResolveOptions {
                normalization: workspace.normalization,
                alignment: workspace.alignment,
                visible_only: true,
            },
            None => ResolveOptions::default(),
        }
    }

    /// Adds the bench selection to the comparison, as a trace or a spectrum.
    pub fn add_to_comparison(&mut self, target: SeriesTarget) -> Result<SeriesId> {
        let dataset = self
            .dataset()
            .ok_or_else(|| ta_core::CoreError::invalid("no dataset is open"))?;
        let version = self.version.unwrap_or(dataset.active_version);
        let (source, label) = match target {
            SeriesTarget::Trace => {
                let trace = self
                    .trace
                    .clone()
                    .ok_or_else(|| ta_core::CoreError::invalid("no wavelength is selected"))?;
                let sample = kinetic_trace(dataset, &trace)?;
                (
                    SeriesSource::Trace(trace.clone()),
                    format!("{} · {}", dataset.name, sample.label),
                )
            }
            SeriesTarget::Spectrum => {
                let spectrum = self
                    .spectrum
                    .clone()
                    .ok_or_else(|| ta_core::CoreError::invalid("no delay is selected"))?;
                let sample = ta_core::transient_spectrum(dataset, &spectrum)?;
                (
                    SeriesSource::Spectrum(spectrum.clone()),
                    format!("{} · {}", dataset.name, sample.label),
                )
            }
        };
        let index = self.comparison().map(|c| c.series.len()).unwrap_or(0);
        let mut series = PlotSeries::new(label, source);
        series.style = SeriesStyle {
            color: palette(index),
            width: 1.5,
            dash: DashPattern::Solid,
            marker: MarkerShape::None,
            marker_size: 4.0,
            opacity: 1.0,
        };
        series.z = index as i32;
        let _ = version;
        let id = series.id;
        self.comparison_mut().series.push(series);
        self.dirty = true;
        Ok(id)
    }

    /// Adds every dataset in the project as a trace at its own strongest
    /// wavelength, which is what "compare what I have" means.
    pub fn add_every_dataset(&mut self) -> Result<usize> {
        let ids: Vec<(ta_core::DatasetId, String, VersionId)> = self
            .project
            .datasets
            .iter()
            .map(|dataset| (dataset.id, dataset.name.clone(), dataset.active_version))
            .collect();
        let mut added = 0usize;
        for (id, name, version) in ids {
            let Ok(dataset) = self.project.dataset(id) else {
                continue;
            };
            // The wavelength with the most signal, so the overlay starts
            // somewhere worth looking.
            let Some(wavelength) = strongest_wavelength(dataset, version) else {
                continue;
            };
            let index = self.comparison().map(|c| c.series.len()).unwrap_or(0);
            let selection = TraceSelection::new(id, version, ta_core::AxisRequest::nearest(wavelength));
            let mut series = PlotSeries::new(
                format!("{name} · {wavelength:.2} nm"),
                SeriesSource::Trace(selection),
            );
            series.style = SeriesStyle {
                color: palette(index),
                width: 1.5,
                ..SeriesStyle::default()
            };
            series.z = index as i32;
            self.comparison_mut().series.push(series);
            added += 1;
        }
        self.dirty = true;
        Ok(added)
    }

    pub fn remove_comparison_series(&mut self, id: SeriesId) {
        let workspace = self.comparison_mut();
        workspace.series.retain(|series| series.id != id);
        self.dirty = true;
    }

    pub fn toggle_comparison_series(&mut self, id: SeriesId) {
        let workspace = self.comparison_mut();
        if let Some(series) = workspace.series.iter_mut().find(|series| series.id == id) {
            series.visible = !series.visible;
        }
        self.dirty = true;
    }

    pub fn comparison_series_at_mut(&mut self, index: usize) -> Option<&mut PlotSeries> {
        self.dirty = true;
        self.comparison_mut().series.get_mut(index)
    }

    /// Moves a series one place earlier or later in paint order.
    pub fn reorder_comparison_series(&mut self, index: usize, delta: isize) {
        let workspace = self.comparison_mut();
        let count = workspace.series.len();
        if count < 2 {
            return;
        }
        let target = (index as isize + delta).clamp(0, count as isize - 1) as usize;
        if target == index {
            return;
        }
        let series = workspace.series.remove(index);
        workspace.series.insert(target, series);
        for (order, series) in workspace.series.iter_mut().enumerate() {
            series.z = order as i32;
        }
        self.dirty = true;
    }

    pub fn set_normalization(&mut self, normalization: Normalization) {
        self.comparison_mut().normalization = normalization;
        self.dirty = true;
    }

    pub fn set_alignment(&mut self, alignment: Alignment) {
        self.comparison_mut().alignment = alignment;
        self.dirty = true;
    }

    /// Starts a new, empty comparison.
    pub fn new_comparison(&mut self) -> usize {
        let name = format!("Comparison {}", self.project.comparisons.len() + 1);
        self.project
            .comparisons
            .push(ComparisonWorkspace::new(name));
        self.comparison = self.project.comparisons.len() - 1;
        self.dirty = true;
        self.comparison
    }

    /// The curves of a pane: spectra on the left, everything else on the right.
    pub fn comparison_pane(&self, spectra: bool) -> Vec<Resolved> {
        let Some(workspace) = self.comparison() else {
            return Vec::new();
        };
        let options = self.comparison_options();
        workspace
            .series
            .iter()
            .enumerate()
            .filter(|(_, series)| {
                matches!(series.source, SeriesSource::Spectrum(_)) == spectra
            })
            .filter_map(|(index, series)| {
                let mut panel = ta_core::PanelModel::new("");
                panel.series.push(series.clone());
                resolve_panel(&self.project, &panel, &options)
                    .into_iter()
                    .next()
                    .map(|mut resolved| {
                        resolved.series = index;
                        resolved
                    })
            })
            .collect()
    }

    /// The caption under a comparison pane, which has to say what is in it.
    pub fn comparison_caption(&self, spectra: bool) -> String {
        let curves = self.comparison_pane(spectra);
        if curves.is_empty() {
            return if spectra {
                "No spectra in this comparison yet.".to_string()
            } else {
                "No kinetics in this comparison yet.".to_string()
            };
        }
        let names: Vec<String> = curves.iter().map(|curve| curve.label.clone()).collect();
        let normalized = curves
            .iter()
            .filter(|curve| curve.normalized)
            .count();
        let mut text = format!(
            "{} {}: {}. ",
            curves.len(),
            if spectra { "spectra" } else { "kinetic traces" },
            names.join("; ")
        );
        if normalized > 0 {
            text.push_str(&format!("{normalized} normalized for display. "));
        }
        let workspace = self.comparison();
        if let Some(workspace) = workspace {
            text.push_str(&match workspace.alignment {
                Alignment::AsMeasured => "Native axes, as measured.",
                Alignment::ToTimeZero => {
                    "Aligned on each dataset's own time-zero correction."
                }
            });
        }
        text
    }

}

/// The wavelength of a dataset that carries the most signal, over a coarse
/// sample of delays. Used only to choose a starting point for a comparison.
fn strongest_wavelength(dataset: &TaDataset, version: VersionId) -> Option<f64> {
    let (wavelength, delay) = dataset.axes(version).ok()?;
    let matrix = dataset.matrix(version).ok()?;
    let row = (0..matrix.nrows()).max_by(|a, b| {
        let score = |row: usize| {
            (0..matrix.ncols())
                .map(|column| matrix[[row, column]].abs())
                .fold(0.0f64, f64::max)
        };
        score(*a).total_cmp(&score(*b))
    })?;
    let _ = delay;
    Some(wavelength.values()[row])
}

// -- the views ---------------------------------------------------------------

/// The comparison toolbar: what is loaded, what goes in, and how it is shown.
impl Workspace {
    pub fn open_comparison_studio(&mut self, spectra: bool) {
        let curves = self.session.comparison_pane(spectra);
        if curves.is_empty() {
            self.session.status.problem("Add a visible trace or spectrum to this comparison first.");
            return;
        }
        let content = pane_content(&self.session, spectra, plates::style());
        let mut figure = ta_core::FigureModel::single_panel(self.session.comparison_caption(spectra));
        figure.panels[0] = content.panel;
        if let Some(comparison) = self.session.comparison() {
            for curve in curves {
                let mut series = comparison.series[curve.series].clone();
                // Bake display transforms into references, never into the measured matrices.
                series.display = curve.transform;
                series.label = curve.label;
                figure.panels[0].series.push(series);
            }
        }
        figure.set_type_size(figure.style.base_font_size);
        self.session.studio = Some(figure);
        self.current_plate = if spectra { crate::session::PlateId::Spectra } else { crate::session::PlateId::Kinetics };
        self.layout = crate::app::Layout::Studio;
        self.session.status.info("FigureStudio opened on the comparison; display transforms preserved.");
        self.touch();
    }

    pub fn compare_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let datasets: Vec<AnyElement> = self
            .session
            .project
            .datasets
            .iter()
            .map(|dataset| {
                let id = dataset.id;
                let active = Some(id) == self.session.opened;
                let label = dataset.name.clone();
                let versions = dataset.versions.len();
                div()
                    .id(ElementId::Name(format!("compare-dataset-{id}").into()))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .border_1()
                    .border_color(if active { ink::accent() } else { ink::rule() })
                    .bg(if active { ink::paper() } else { ink::ground() })
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(if active { ink::accent() } else { ink::text() })
                            .child(format!("{label} · {versions} versions")),
                    )
                    .on_click(cx.listener(move |workspace, _event, _window, cx| {
                        workspace.session.open_bench(id);
                        workspace.touch();
                        workspace.touch_matrix();
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect();

        let normalization = self
            .session
            .comparison()
            .map(|workspace| workspace.normalization)
            .unwrap_or(Normalization::None);
        let alignment = self
            .session
            .comparison()
            .map(|workspace| workspace.alignment)
            .unwrap_or(Alignment::AsMeasured);
        let name = self
            .session
            .comparison()
            .map(|workspace| workspace.name.clone())
            .unwrap_or_else(|| "Comparison".to_string());
        let count = self.session.comparison_series().len();

        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .bg(ink::ground())
            .border_b_1()
            .border_color(ink::rule())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(ink::muted())
                            .child("OBJECTS"),
                    )
                    .children(datasets)
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .justify_end()
                            .items_center()
                            .gap_3()
                            .child(widgets::em(format!("{name} · {count} series")))
                            // The button names what it will add — the bench
                            // dataset's current coordinate — because "the
                            // selection" is only obvious to whoever made it.
                            .child(widgets::readout(format!(
                                "the bench has {}",
                                self.compare_source_label()
                            )))
                            .child(widgets::action_id(
                                if self.compare_target == SeriesTarget::Trace {
                                    "Add its trace to the comparison"
                                } else {
                                    "Add its spectrum to the comparison"
                                },
                                ElementId::Name("compare-add".into()),
                                true,
                                cx,
                                |workspace, _, _| {
                                    let target = workspace.compare_target;
                                    match workspace.session.add_to_comparison(target) {
                                        Ok(_) => {
                                            let count =
                                                workspace.session.comparison_series().len();
                                            workspace.session.status.info(format!(
                                                "the comparison holds {count} series"
                                            ));
                                            workspace.touch();
                                        }
                                        Err(error) => workspace
                                            .session
                                            .status
                                            .problem(format!("{error}")),
                                    }
                                },
                            ))
                            .child(widgets::action(
                                "Add every dataset",
                                false,
                                cx,
                                |workspace, _, _| match workspace.session.add_every_dataset() {
                                    Ok(added) => {
                                        workspace
                                            .session
                                            .status
                                            .info(format!("added {added} traces"));
                                        workspace.touch();
                                    }
                                    Err(error) => {
                                        workspace.session.status.problem(format!("{error}"))
                                    }
                                },
                            ))
                            .child(widgets::action("New comparison", false, cx, |workspace, _, _| {
                                workspace.session.new_comparison();
                                workspace.touch();
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(ink::muted())
                                    .child("ADD"),
                            )
                            .child(widgets::choice(
                                "trace",
                                self.compare_target == SeriesTarget::Trace,
                                cx,
                                |workspace| workspace.compare_target = SeriesTarget::Trace,
                            ))
                            .child(widgets::choice(
                                "spectrum",
                                self.compare_target == SeriesTarget::Spectrum,
                                cx,
                                |workspace| workspace.compare_target = SeriesTarget::Spectrum,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(ink::muted())
                                    .child("NORMALIZE"),
                            )
                            .child(widgets::choice(
                                "none",
                                normalization == Normalization::None,
                                cx,
                                |workspace| workspace.session.set_normalization(Normalization::None),
                            ))
                            .child(widgets::choice(
                                "max |ΔOD|",
                                normalization == Normalization::MaxAbs,
                                cx,
                                |workspace| {
                                    workspace.session.set_normalization(Normalization::MaxAbs)
                                },
                            ))
                            .child(widgets::choice(
                                "at zero",
                                matches!(normalization, Normalization::AtDelay(_)),
                                cx,
                                |workspace| {
                                    workspace.session.set_normalization(Normalization::AtDelay(0.0))
                                },
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(ink::muted())
                                    .child("ALIGN"),
                            )
                            .child(widgets::choice(
                                "as measured",
                                alignment == Alignment::AsMeasured,
                                cx,
                                |workspace| workspace.session.set_alignment(Alignment::AsMeasured),
                            ))
                            .child(widgets::choice(
                                "on time zero",
                                alignment == Alignment::ToTimeZero,
                                cx,
                                |workspace| workspace.session.set_alignment(Alignment::ToTimeZero),
                            )),
                    ),
            )
    }

    /// What the bench would contribute if it were added: the dataset's name and
    /// the coordinate the comparison would take from it.
    pub fn compare_source_label(&self) -> String {
        let Some(dataset) = self.session.dataset() else {
            return "no dataset open".to_string();
        };
        let coordinate = match self.compare_target {
            SeriesTarget::Trace => self
                .session
                .trace
                .as_ref()
                .map(|trace| self.session.wavelength_label(trace.wavelength.coordinate)),
            SeriesTarget::Spectrum => self
                .session
                .spectrum
                .as_ref()
                .map(|spectrum| self.session.delay_label(spectrum.delay.coordinate)),
        };
        match coordinate {
            Some(coordinate) => format!("{} at {coordinate}", dataset.name),
            None => format!("{} and nothing selected", dataset.name),
        }
    }

    /// One comparison pane: the curves, drawn in a plate of their own.
    pub fn compare_pane(&self, index: usize) -> impl IntoElement {
        let plate = &self.compare_plates[index];
        let element = plate.element(ElementId::Name(format!("compare-{index}").into()));
        div().flex().flex_col().size_full().bg(ink::paper()).child(
            div()
                .flex_1()
                .w_full()
                .min_h(px(60.0))
                .child(element),
        )
    }

    /// The series list: the per-curve controls of §15.4.
    pub fn compare_series_list(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let series = self.session.comparison_series();
        let curves = {
            let mut all = self.session.comparison_pane(false);
            all.extend(self.session.comparison_pane(true));
            all
        };
        let mut rows: Vec<AnyElement> = Vec::new();
        for (index, series) in series.iter().enumerate() {
            let id = series.id;
            let curve = curves.iter().find(|curve| curve.series == index);
            let visible = series.visible;
            let kind = match series.source {
                SeriesSource::Trace(_) => "trace",
                SeriesSource::Spectrum(_) => "spectrum",
                SeriesSource::Fit { .. } => "fit",
                SeriesSource::Curve { .. } => "curve",
                _ => "matrix",
            };
            let sampled = curve
                .and_then(|curve| curve.sampled)
                .map(|value| format!("{value:.3}"))
                .unwrap_or_else(|| "—".to_string());
            let range = curve
                .map(|curve| match (curve.points.first(), curve.points.last()) {
                    (Some(first), Some(last)) => {
                        format!("{:.3} … {:.3}", first.1, last.1)
                    }
                    _ => "—".to_string(),
                })
                .unwrap_or_else(|| "—".to_string());

            let label_state = self.field_state(
                FieldId::SeriesLabel(index),
                series.label.clone(),
                "label",
                window,
                cx,
            );
            let scale_state = self.field_state(
                FieldId::SeriesScale(index),
                format!("{:.4}", series.display.scale),
                "1",
                window,
                cx,
            );
            let offset_state = self.field_state(
                FieldId::SeriesOffset(index),
                format!("{:.4}", series.display.offset),
                "0",
                window,
                cx,
            );
            let shift_state = self.field_state(
                FieldId::SeriesShift(index),
                format!("{:.4}", series.display.coordinate_offset),
                "0",
                window,
                cx,
            );
            let color = series.style.color;

            rows.push(
                div()
                    .id(ElementId::Name(format!("series-{id}").into()))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(ink::hair())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(widgets::switch(
                                "shown",
                                visible,
                                cx,
                                move |workspace| {
                                    workspace.session.toggle_comparison_series(id)
                                },
                            ))
                            .child(
                                div()
                                    .w(px(10.0))
                                    .h(px(10.0))
                                    .bg(crate::app::ink::hsla(color)),
                            )
                            .child(div().flex_1().min_w(px(0.0)).child(widgets::text_input(
                                &label_state,
                                168.0,
                            )))
                            .child(widgets::action_id(
                                "↑",
                                ElementId::Name(format!("up-{id}").into()),
                                false,
                                cx,
                                move |workspace, _, _| {
                                    workspace.session.reorder_comparison_series(index, -1)
                                },
                            ))
                            .child(widgets::action_id(
                                "↓",
                                ElementId::Name(format!("down-{id}").into()),
                                false,
                                cx,
                                move |workspace, _, _| {
                                    workspace.session.reorder_comparison_series(index, 1)
                                },
                            ))
                            .child(widgets::action_id(
                                "remove",
                                ElementId::Name(format!("remove-{id}").into()),
                                false,
                                cx,
                                move |workspace, _, _| {
                                    workspace.session.remove_comparison_series(id)
                                },
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .flex_wrap()
                            .child(widgets::readout(format!("{kind} at {sampled}")))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(widgets::small_caps("scale"))
                                    .child(widgets::number_input(&scale_state, 52.0)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(widgets::small_caps("offset"))
                                    .child(widgets::number_input(&offset_state, 52.0)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(widgets::small_caps("shift"))
                                    .child(widgets::number_input(&shift_state, 52.0)),
                            )
                            .child(widgets::readout(format!("{range}"))),
                    )
                    .into_any_element(),
            );
        }

        div()
            .w(px(336.0))
            .h_full()
            .flex()
            .flex_col()
            .bg(ink::ground())
            .border_l_1()
            .border_color(ink::rule())
            .child(
                div()
                    .px_3()
                    .py_2()
                    .text_size(px(10.0))
                    .text_color(ink::muted())
                    .border_b_1()
                    .border_color(ink::rule())
                    .child("SERIES".to_string()),
            )
            .children(rows)
            .into_any_element()
    }
}

/// The panes a comparison workspace draws.
pub fn pane_content(
    session: &Session,
    spectra: bool,
    style: ta_core::PublicationStyle,
) -> PlateContent {
    let curves = session.comparison_pane(spectra);
    let mut panel = ta_core::PanelModel::new("");
    panel.x = if spectra {
        plates::axis("Wavelength", Some("nm"), ta_core::AxisScale::Linear)
    } else {
        plates::axis(
            "Delay",
            Some("ps"),
            ta_core::AxisScale::SplitLinearLog { break_at: 1.0 },
        )
    };
    let normalized = curves.iter().any(|curve| curve.normalized);
    let y_label = if normalized && curves.iter().any(|curve| !curve.normalized) {
        "Signal (partly normalized)"
    } else if normalized {
        "Normalized signal"
    } else {
        "ΔOD"
    };
    panel.y = plates::axis(y_label, None, ta_core::AxisScale::Linear);
    panel.legend.visible = curves.len() > 1;
    let _ = style;

    let mut bounds: Option<(f64, f64)> = None;
    let mut x_bounds: Option<(f64, f64)> = None;
    for curve in &curves {
        for (x, y) in &curve.points {
            bounds = Some(match bounds {
                None => (*y, *y),
                Some((low, high)) => (low.min(*y), high.max(*y)),
            });
            x_bounds = Some(match x_bounds {
                None => (*x, *x),
                Some((low, high)) => (low.min(*x), high.max(*x)),
            });
        }
    }
    if let Some((low, high)) = bounds {
        if high > low {
            let pad = (high - low) * 0.08;
            panel.y.range = Some((low - pad, high + pad));
        }
    }
    if let Some((low, high)) = x_bounds {
        panel.x.range = Some((low, high));
    } else if let (Some(dataset), Some(version)) = (session.dataset(), session.version_id()) {
        // An empty pane is framed at the scale of the dataset it would hold,
        // rather than at a unit box that says nothing about this sample.
        if let Ok((wavelength, delay)) = dataset.axes(version) {
            panel.x.range = Some(if spectra {
                wavelength.range()
            } else {
                delay.range()
            });
        }
    }
    let series = curves.iter().map(Resolved::plotted).collect();
    // A comparison pane's records are the workspace's own series, so a figure
    // made from it refers to them rather than to copies.
    let records = workspace_series(session, spectra);
    PlateContent {
        panel,
        series,
        records,
    }
}

/// The saved records behind one pane, in the order the pane draws them.
fn workspace_series(session: &Session, spectra: bool) -> Vec<PlotSeries> {
    session
        .comparison()
        .map(|workspace| {
            workspace
                .series
                .iter()
                .filter(|series| matches!(series.source, SeriesSource::Spectrum(_)) == spectra)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;
    use ndarray::Array2;
    use ta_core::{
        Axis, AxisKind, AxisUnit, SignalUnit, SourceFormat, SeriesKind, SpectrumSelection,
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

    fn session_with_two_datasets() -> Session {
        let mut session = Session::new();
        session.open_dataset(dataset("sample A", 1.0));
        let second = dataset("sample B", 2.0);
        let id = second.id;
        session.project.add_dataset(second);
        session.open_bench(id);
        session
    }

    #[test]
    fn the_comparison_opens_in_studio_without_losing_normalization_or_dataset_sources() {
        let mut session = session_with_two_datasets();
        session.add_to_comparison(SeriesTarget::Trace).unwrap();
        let first = session.project.datasets[0].id;
        session.open_bench(first);
        session.add_to_comparison(SeriesTarget::Trace).unwrap();
        session.set_normalization(Normalization::MaxAbs);
        let series = &mut session.comparison_mut().series[0];
        series.display.scale = 2.0;
        series.display.offset = 0.25;
        series.display.coordinate_offset = 0.5;
        let expected: Vec<_> = session.comparison_pane(false).iter().map(Resolved::plotted).collect();
        let before = session.project.datasets[0].active_matrix().clone();
        let mut workspace = Workspace::new(session);
        workspace.open_comparison_studio(false);
        let actual = workspace.studio_preview().unwrap();
        assert_eq!(actual.series, expected);
        assert_eq!(actual.panel.y.label, "Normalized signal");
        assert_eq!(workspace.session.project.datasets[0].active_matrix(), &before);
        assert_eq!(workspace.session.studio.as_ref().unwrap().panels[0].series.len(), 2);
        assert_eq!(workspace.layout, crate::app::Layout::Studio);
    }

    #[test]
    fn a_failed_zero_reference_does_not_claim_normalized_units() {
        let mut session = session_with_two_datasets();
        session.add_to_comparison(SeriesTarget::Trace).unwrap();
        session.set_normalization(Normalization::AtDelay(0.0));
        assert!(!session.comparison_pane(false)[0].normalized);
        assert_eq!(pane_content(&session, false, plates::style()).panel.y.label, "ΔOD");
    }

    #[test]
    fn an_empty_comparison_does_not_replace_the_current_figure() {
        let mut workspace = Workspace::new(session_with_two_datasets());
        workspace.open_studio(crate::session::PlateId::Kinetics);
        let before = workspace.session.studio.clone();
        workspace.open_comparison_studio(true);
        assert_eq!(workspace.session.studio, before);
        assert!(workspace.session.status.problem.is_some());
    }

    #[test]
    fn adding_a_trace_then_a_spectrum_fills_the_two_panes() {
        let mut session = session_with_two_datasets();
        session.add_to_comparison(SeriesTarget::Trace).unwrap();
        session.add_to_comparison(SeriesTarget::Spectrum).unwrap();
        assert_eq!(session.comparison_pane(false).len(), 1);
        assert_eq!(session.comparison_pane(true).len(), 1);
        // Both keep their dataset's name, which is the point of a comparison.
        assert!(session.comparison_pane(false)[0].label.contains("sample B"));
    }

    #[test]
    fn a_comparison_over_two_datasets_keeps_native_coordinates() {
        let mut session = session_with_two_datasets();
        session.add_to_comparison(SeriesTarget::Trace).unwrap();
        let first = session
            .project
            .datasets
            .first()
            .map(|dataset| dataset.id)
            .unwrap();
        session.open_bench(first);
        session.add_to_comparison(SeriesTarget::Trace).unwrap();
        let curves = session.comparison_pane(false);
        assert_eq!(curves.len(), 2);
        // Neither was resampled onto the other's grid.
        assert_eq!(curves[0].points[0].0, -1.0);
        assert_eq!(curves[1].points[0].0, -1.0);
        assert_ne!(curves[0].points[0].1, curves[1].points[0].1);
    }

    #[test]
    fn normalization_applies_to_every_curve_of_the_comparison() {
        let mut session = session_with_two_datasets();
        session.add_to_comparison(SeriesTarget::Trace).unwrap();
        session.set_normalization(Normalization::MaxAbs);
        let curves = session.comparison_pane(false);
        for curve in &curves {
            assert!(curve.normalized);
            let peak = curve.points.iter().map(|(_, y)| y.abs()).fold(0.0, f64::max);
            assert!((peak - 1.0).abs() < 1e-12);
        }
        assert!(session.comparison_caption(false).contains("normalized"));
    }

    #[test]
    fn hiding_a_curve_changes_the_pane_and_the_caption_together() {
        let mut session = session_with_two_datasets();
        session.add_to_comparison(SeriesTarget::Trace).unwrap();
        session.add_to_comparison(SeriesTarget::Spectrum).unwrap();
        let id = session.comparison_series()[1].id;
        session.toggle_comparison_series(id);
        assert_eq!(session.comparison_pane(true).len(), 0);
        assert!(session.comparison_caption(true).contains("No spectra"));
        // And it is still in the model, so it comes back.
        assert_eq!(session.comparison_series().len(), 2);
        session.toggle_comparison_series(id);
        assert_eq!(session.comparison_pane(true).len(), 1);
    }

    #[test]
    fn paint_order_is_the_series_order() {
        let mut session = session_with_two_datasets();
        session.add_to_comparison(SeriesTarget::Trace).unwrap();
        session.add_to_comparison(SeriesTarget::Trace).unwrap();
        let second = session.comparison_series()[1].id;
        session.reorder_comparison_series(1, -1);
        assert_eq!(session.comparison_series()[0].id, second);
        assert_eq!(session.comparison_series()[0].z, 0);
    }

    #[test]
    fn adding_every_dataset_adds_one_curve_each_at_a_wavelength_with_signal() {
        let mut session = session_with_two_datasets();
        let added = session.add_every_dataset().unwrap();
        assert_eq!(added, 2);
        let curves = session.comparison_pane(false);
        assert_eq!(curves.len(), 2);
        for curve in &curves {
            let peak = curve.points.iter().map(|(_, y)| y.abs()).fold(0.0, f64::max);
            assert!(peak > 0.0, "a comparison should not start on a flat trace");
        }
    }

    #[test]
    fn the_pane_description_carries_each_curve_with_its_own_style() {
        let mut session = session_with_two_datasets();
        session.add_to_comparison(SeriesTarget::Trace).unwrap();
        session.add_to_comparison(SeriesTarget::Spectrum).unwrap();
        let style = crate::plates::style();
        let kinetics = pane_content(&session, false, style.clone());
        assert_eq!(kinetics.series.len(), 1);
        assert_eq!(kinetics.series[0].style.color, palette(0));
        assert!(kinetics.panel.legend.visible == false);
        let spectra = pane_content(&session, true, style);
        assert_eq!(spectra.series.len(), 1);
        assert_eq!(spectra.panel.x.label, "Wavelength");
        assert_eq!(kinetics.panel.x.label, "Delay");
    }

    #[test]
    fn a_series_that_is_not_in_the_project_resolves_to_nothing_rather_than_an_error() {
        let mut session = session_with_two_datasets();
        session.add_to_comparison(SeriesTarget::Trace).unwrap();
        // The series points at the dataset on the bench, which is the second
        // one; removing it is what leaves a dangling reference.
        let id = session.opened.unwrap();
        session.project.remove_dataset(id).unwrap();
        let curves = session.comparison_pane(false);
        assert!(curves.is_empty());
    }

    #[test]
    fn a_new_comparison_starts_empty_and_becomes_the_bench() {
        let mut session = session_with_two_datasets();
        session.add_to_comparison(SeriesTarget::Trace).unwrap();
        session.new_comparison();
        assert!(session.comparison_series().is_empty());
        assert_eq!(session.project.comparisons.len(), 2);
        assert_eq!(session.comparison, 1);
    }

    #[test]
    fn a_removed_curve_leaves_a_spectrum_selection_untouched() {
        let mut session = session_with_two_datasets();
        let id = session.add_to_comparison(SeriesTarget::Spectrum).unwrap();
        let selection = session.spectrum.clone().unwrap();
        session.remove_comparison_series(id);
        assert!(session.comparison_series().is_empty());
        assert_eq!(session.spectrum.unwrap().delay.coordinate, selection.delay.coordinate);
    }

    #[test]
    fn a_comparison_that_is_exported_says_what_it_holds() {
        let mut session = session_with_two_datasets();
        session.add_to_comparison(SeriesTarget::Trace).unwrap();
        let curves = session.comparison_pane(false);
        assert_eq!(curves.len(), 1);
        assert_eq!(curves[0].kind, SeriesKind::Line);
        // The numbers carry their provenance, which is what makes the exported
        // file self-describing rather than just a table of floats.
        assert!(curves[0].dataset.is_some());
        assert!(curves[0].version.is_some());
        assert!(curves[0].sampled.is_some());
        let _ = SpectrumSelection::new;
    }
}
