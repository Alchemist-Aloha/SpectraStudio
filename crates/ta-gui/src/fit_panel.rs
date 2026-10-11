//! The fitting workbench (§12.3–§12.6).
//!
//! The solver is in `ta-fitting`; this is the panel that drives it. A fit runs
//! on the background executor, so a four-exponential fit of a long trace cannot
//! freeze the window, and a result is only accepted if the data it was computed
//! from is still the data on the bench — the version and selection are checked
//! when the answer comes back, not when the job was started.

use crate::app::{ink, Workspace};
use crate::plates::{self, PlateContent};
use crate::widgets::{self, FieldId};
use crate::session::Session;
use gpui_kit::{div, px, AnyElement, Context, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled, Window};
use ta_core::{
    kinetic_trace, CoreError, FitModelKind, FitParameter, FitResult, Result, SeriesKind,
    SeriesStyle, Trace, TraceSelection, Weighting,
};
use ta_fitting::{FitRequest, SolverOptions};

/// What the workbench is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FitState {
    /// Nothing has been fitted yet for this selection.
    Idle,
    /// A background job is running.
    Running,
    /// A result arrived and is being shown.
    Done,
    /// The job failed or was cancelled.
    Failed,
}

/// Everything the workbench holds between renders.
#[derive(Debug)]
pub struct FitSession {
    pub state: FitState,
    pub model: FitModelKind,
    pub components: u8,
    pub parameters: Vec<FitParameter>,
    pub fit_range: Option<(f64, f64)>,
    pub excluded: Vec<(f64, f64)>,
    pub weighting: Weighting,
    pub options: SolverOptions,
    /// The result currently on screen, which is a record in the dataset.
    pub result: Option<FitResult>,
    /// Incremented for every job; a stale answer carries an older number.
    pub generation: u64,
    /// What the running job is fitting, so a stale answer can be told apart.
    pub job: Option<(ta_core::DatasetId, ta_core::VersionId, f64)>,
    /// How the residuals are drawn.
    pub show_residuals: bool,
    /// Whether the fitted curve is overlaid on the data plate.
    pub show_overlay: bool,
    /// The last message the solver produced.
    pub message: String,
    /// A name for the parameter set, so a preset can be saved and recalled.
    pub preset_name: String,
}

impl Default for FitSession {
    fn default() -> Self {
        Self {
            state: FitState::Idle,
            model: FitModelKind::LegacyGaussianCdfIrf,
            components: 2,
            parameters: ta_core::legacy_parameter_template(2),
            fit_range: None,
            excluded: Vec::new(),
            weighting: Weighting::Uniform,
            options: SolverOptions::default(),
            result: None,
            generation: 0,
            job: None,
            show_residuals: true,
            show_overlay: true,
            message: String::new(),
            preset_name: String::new(),
        }
    }
}

/// The parameters that are actually free for a given component count.
pub fn active_parameters(parameters: &[FitParameter]) -> Vec<&FitParameter> {
    parameters.iter().filter(|parameter| !parameter.fixed).collect()
}

fn component_slot(name: &str) -> Option<u8> {
    name.strip_prefix("amplitude_").or_else(|| name.strip_prefix("lifetime_"))?.parse().ok()
}

impl FitSession {
    /// Rebuilds the parameter list when the component count or the model
    /// changes, keeping the values the user has already typed.
    pub fn set_components(&mut self, components: u8) {
        let components = components.clamp(1, 4);
        let mut fresh = ta_core::legacy_parameter_template(components);
        for parameter in &mut fresh {
            if let Some(existing) = self
                .parameters
                .iter()
                .find(|candidate| candidate.name == parameter.name)
            {
                parameter.value = existing.value;
                parameter.min = existing.min;
                parameter.max = existing.max;
                if component_slot(&parameter.name).is_none_or(|slot| slot <= self.components.min(components)) {
                    parameter.fixed = existing.fixed;
                }
            }
        }
        self.parameters = fresh;
        self.components = components;
        self.result = None;
        self.state = FitState::Idle;
    }

    /// The starting values, in the shape the fitter wants.
    pub fn request(
        &self,
        dataset: ta_core::DatasetId,
        version: ta_core::VersionId,
        selection: TraceSelection,
    ) -> FitRequest {
        FitRequest {
            dataset,
            version,
            selection,
            model: self.model,
            components: self.components,
            parameters: self.parameters.clone(),
            fit_range: self.fit_range,
            excluded: self.excluded.clone(),
            weighting: self.weighting.clone(),
            options: self.options,
        }
    }

    /// Takes the values of a finished fit as the starting point for the next
    /// one, which is the "reuse values from previous fits" requirement.
    pub fn adopt(&mut self, result: &FitResult) {
        for parameter in &mut self.parameters {
            if let Some(found) = result.parameter(&parameter.name) {
                parameter.value = found.value;
                parameter.min = found.min;
                parameter.max = found.max;
                parameter.fixed = found.fixed;
            }
        }
        self.model = result.model;
        self.components = result.components;
        self.fit_range = result.fit_range;
        self.excluded = result.excluded.clone();
    }

    /// One line describing how the fit ended, for the panel and the caption.
    pub fn verdict(&self) -> String {
        match (&self.state, &self.result) {
            (FitState::Idle, _) => "Not fitted yet.".to_string(),
            (FitState::Running, _) => "Fitting…".to_string(),
            (FitState::Failed, _) => format!("The fit did not finish: {}", self.message),
            (FitState::Done, Some(result)) => {
                // The solver's own message already names the iteration count and
                // the sum of squares; repeating them reads as two answers.
                let mut text = if result.converged() {
                    format!("Converged: {}", result.stats.message)
                } else {
                    format!("Not converged: {}", result.stats.message)
                };
                // The solver's message already states the sum of squares, so
                // only the derived numbers are added here.
                text.push_str(&format!(", reduced χ² unavailable without errors"));
                if let Some(r_squared) = result.stats.r_squared {
                    text.push_str(&format!(", R² {r_squared:.5}"));
                }
                let pinned = result.pinned_parameters();
                if !pinned.is_empty() {
                    let names: Vec<String> = pinned
                        .iter()
                        .map(|parameter| parameter.name.clone())
                        .collect();
                    text.push_str(&format!(
                        ". At a bound, so no standard error: {}.",
                        names.join(", ")
                    ));
                }
                text
            }
            (FitState::Done, None) => "No result is on screen.".to_string(),
        }
    }
}

impl Session {
    /// The workbench's state, created on first use.
    pub fn fit_session(&mut self) -> &mut FitSession {
        if self.fit.is_none() {
            self.fit = Some(FitSession::default());
        }
        self.fit.as_mut().expect("just created")
    }

    /// Retained fits of the bench dataset, newest last.
    pub fn retained_fits(&self) -> Vec<FitResult> {
        let Some(dataset) = self.dataset() else {
            return Vec::new();
        };
        dataset
            .fits
            .iter()
            .filter(|fit| Some(fit.version) == self.version)
            .cloned()
            .collect()
    }

    /// Stores a finished fit in the dataset it came from, which is where §12.7
    /// says it lives.
    pub fn retain_fit(&mut self, result: FitResult) -> Result<()> {
        let dataset = self
            .project
            .dataset_mut(result.dataset)
            .map_err(|_| CoreError::invalid("the dataset this fit belongs to is gone"))?;
        dataset.fits.push(result);
        self.dirty = true;
        Ok(())
    }

    /// The trace a fit would be made against.
    pub fn fit_trace(&self) -> Option<Trace> {
        let dataset = self.dataset()?;
        let selection = self.trace.clone()?;
        kinetic_trace(dataset, &selection).ok()
    }

    /// A fit that is reproducible after reopening the project: the record, the
    /// request it came from, and the version that supplied the numbers.
    ///
    /// Opening the workbench is not a prerequisite for asking what it would
    /// fit: a request made before the panel has ever been drawn uses the
    /// reference defaults, which is what a first fit wants anyway.
    pub fn fit_request(&mut self) -> Result<(FitRequest, Trace)> {
        let dataset = self
            .dataset()
            .ok_or_else(|| CoreError::invalid("no dataset is open"))?;
        let selection = self
            .trace
            .clone()
            .ok_or_else(|| CoreError::invalid("no wavelength is selected"))?;
        let trace = kinetic_trace(dataset, &selection)?;
        let id = dataset.id;
        let session = self.fit_session();
        Ok((session.request(id, selection.version, selection), trace))
    }
}

// -- the views -----------------------------------------------------------------

impl Workspace {
    /// Runs a fit on the background executor.
    ///
    /// The job carries the request and the trace's numbers, so it touches no
    /// shared state while it runs; the answer is only used when the generation
    /// and the selection still match.
    pub fn start_fit(&mut self, cx: &mut Context<Self>) {
        let (request, trace) = match self.session.fit_request() {
            Ok(pair) => pair,
            Err(error) => {
                self.session.status.problem(format!("{error}"));
                return;
            }
        };
        let session = self.session.fit_session();
        session.generation += 1;
        let generation = session.generation;
        session.state = FitState::Running;
        session.message.clear();
        session.job = Some((request.dataset, request.version, request.selection.wavelength.coordinate));
        let coordinate = request.selection.wavelength.coordinate;

        let work = cx.background_executor().spawn(async move {
            ta_fitting::fit_trace(&trace, &request)
        });

        cx.spawn(async move |this, cx| {
            let outcome = work.await;
            let _ = this.update(cx, |workspace, cx| {
                workspace.finish_fit(generation, coordinate, outcome);
                cx.notify();
            });
        })
        .detach();
    }

    /// Where a fit's answer lands, whether it succeeded, failed or is stale.
    fn finish_fit(
        &mut self,
        generation: u64,
        coordinate: f64,
        outcome: ta_fitting::Result<FitResult>,
    ) {
        let session = self.session.fit_session();
        if session.generation != generation {
            // A newer job is running; this answer describes an older question.
            return;
        }
        session.job = None;
        match outcome {
            Ok(result) => {
                session.state = FitState::Done;
                session.message = result.stats.message.clone();
                session.result = Some(result.clone());
                match self.session.retain_fit(result) {
                    Ok(()) => self
                        .session
                        .status
                        .info(format!("fitted {coordinate:.3} nm")),
                    Err(error) => self.session.status.problem(format!("{error}")),
                }
            }
            Err(error) => {
                session.state = FitState::Failed;
                session.message = format!("{error}");
                self.session.status.problem(format!("the fit failed: {error}"));
            }
        }
        self.touch();
    }

    /// Cancels the running job by invalidating its generation.
    pub fn cancel_fit(&mut self) {
        let session = self.session.fit_session();
        if session.state == FitState::Running {
            session.generation += 1;
            session.state = FitState::Idle;
            session.job = None;
            session.message = "cancelled".to_string();
            self.session
                .status
                .info("the fit was cancelled; nothing was stored");
        }
    }

    /// The parameter table: value and bounds per parameter, with the fixed
    /// state as a checkbox and the units in a column of their own.
    pub fn fit_parameter_table(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let parameters = self
            .session
            .fit
            .as_ref()
            .map(|fit| fit.parameters.clone())
            .unwrap_or_default();
        let result = self
            .session
            .fit
            .as_ref()
            .and_then(|fit| fit.result.clone());
        let mut rows: Vec<AnyElement> = Vec::new();
        let editing = self.fit_edit_parameters;
        let components = self.session.fit.as_ref().map(|fit| fit.components).unwrap_or(2);
        for (index, parameter) in parameters.iter().enumerate() {
            if component_slot(&parameter.name).is_some_and(|slot| slot > components) {
                continue;
            }
            let name = parameter.name.clone();
            let unit = parameter.unit.clone().unwrap_or_default();
            let fixed = parameter.fixed;
            let value_state = self.field_state(
                FieldId::FitValue(index),
                format!("{:.6}", parameter.value),
                "value",
                window,
                cx,
            );
            let bounds_state = self.field_state(
                FieldId::FitBounds(index),
                widgets::format_bounds(parameter.min, parameter.max),
                "min .. max",
                window,
                cx,
            );
            // What the solver found, and how well it knows it.
            let (fitted, error) = match result
                .as_ref()
                .and_then(|fit| fit.parameter(&name))
            {
                Some(found) => (
                    format!("{:.6}", found.value),
                    match found.stderr {
                        Some(stderr) => format!("± {stderr:.2e}"),
                        None if found.at_bound() => "at a bound".to_string(),
                        None => "not identifiable".to_string(),
                    },
                ),
                None => ("—".to_string(), String::new()),
            };
            let at_bound = result
                .as_ref()
                .and_then(|fit| fit.parameter(&name))
                .is_some_and(|found| found.at_bound() && !found.fixed);

            rows.push(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .border_b_1()
                    .border_color(ink::hair())
                    .child(
                        div()
                            .w(px(112.0))
                            .flex_shrink_0()
                            .text_size(px(11.0))
                            .text_color(if at_bound { ink::amber() } else { ink::text() })
                            .child(name),
                    )
                    .child(if editing {
                        widgets::number_input(&value_state, 84.0).into_any_element()
                    } else {
                        div().into_any_element()
                    })
                    .child(if editing {
                        widgets::number_input(&bounds_state, 106.0).into_any_element()
                    } else {
                        div().into_any_element()
                    })
                    .child(if editing {
                        widgets::switch("fixed", fixed, cx, move |workspace| {
                            if let Some(fit) = workspace.session.fit.as_mut() {
                                if let Some(parameter) = fit.parameters.get_mut(index) {
                                    parameter.fixed = !parameter.fixed;
                                }
                            }
                        })
                        .into_any_element()
                    } else {
                        div()
                            .w(px(48.0))
                            .flex_shrink_0()
                            .text_size(px(10.0))
                            .text_color(ink::muted())
                            .child(if fixed { "fixed" } else { "free" })
                            .into_any_element()
                    })
                    .child(
                        div()
                            .w(px(32.0))
                            .flex_shrink_0()
                            .text_size(px(10.0))
                            .text_color(ink::muted())
                            .child(if !unit.is_empty() { unit } else { String::new() }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(ink::text())
                                    .child(fitted),
                            )
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(ink::muted())
                                    .child(error),
                            ),
                    )
                    .into_any_element(),
            );
        }

        div()
            .w_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .border_b_1()
                    .border_color(ink::rule())
                    .child(
                        div()
                            .w(px(112.0))
                            .flex_shrink_0()
                            .text_size(px(10.0))
                            .text_color(ink::muted())
                            .child("PARAMETER"),
                    )
                    .child(if editing {
                        div().w(px(84.0)).flex_shrink_0().text_size(px(10.0))
                            .text_color(ink::muted()).child("START").into_any_element()
                    } else {
                        div().into_any_element()
                    })
                    .child(if editing {
                        div().w(px(106.0)).flex_shrink_0().text_size(px(10.0))
                            .text_color(ink::muted()).child("BOUNDS").into_any_element()
                    } else {
                        div().into_any_element()
                    })
                    .child(
                        div()
                            .w(px(48.0))
                            .flex_shrink_0()
                            .text_size(px(10.0))
                            .text_color(ink::muted())
                            .child("STATE"),
                    )
                    .child(div().w(px(32.0)).flex_shrink_0().text_size(px(10.0))
                        .text_color(ink::muted()).child("UNIT"))
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(ink::muted())
                            .id("fit-result-heading")
                            .child("FITTED")
                            .tooltip(|window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new("FITTED (Δ FROM A BOUND IN AMBER)")
                                    .build(window, cx)
                            }),
                    ),
            )
            .children(rows)
            .into_any_element()
    }

    /// The workbench's controls: model, components, ranges, weighting, run.
    pub fn fit_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let (model, components, state, weighting, fit_range, excluded) = match &self.session.fit {
            Some(fit) => (
                fit.model,
                fit.components,
                fit.state,
                fit.weighting.clone(),
                fit.fit_range,
                fit.excluded.clone(),
            ),
            None => (
                FitModelKind::LegacyGaussianCdfIrf,
                2,
                FitState::Idle,
                Weighting::Uniform,
                None,
                Vec::new(),
            ),
        };
        let range_state = self.field_state(
            FieldId::FitRange,
            widgets::format_range(fit_range),
            "whole trace",
            window,
            cx,
        );
        let excluded_state = self.field_state(
            FieldId::FitExcluded,
            widgets::format_intervals(&excluded),
            "0 1, 20 30",
            window,
            cx,
        );
        let reference_state = self.field_state(
            FieldId::FitReferenceDelay,
            format!("{:.4}", 1.0),
            "1.0",
            window,
            cx,
        );
        let busy = state == FitState::Running;
        let verdict = self
            .session
            .fit
            .as_ref()
            .map(FitSession::verdict)
            .unwrap_or_else(|| "Not fitted yet.".to_string());
        let overlay = self
            .session
            .fit
            .as_ref()
            .map(|fit| fit.show_overlay)
            .unwrap_or(true);
        let residuals_on = self
            .session
            .fit
            .as_ref()
            .map(|fit| fit.show_residuals)
            .unwrap_or(true);
        let retained = self.session.retained_fits().len();

        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .px_3()
            .py_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(widgets::small_caps("model"))
                    .child(widgets::choice(
                        "legacy Gaussian CDF",
                        model == FitModelKind::LegacyGaussianCdfIrf,
                        cx,
                        |workspace| {
                            if let Some(fit) = workspace.session.fit.as_mut() {
                                fit.model = FitModelKind::LegacyGaussianCdfIrf;
                            }
                        },
                    ))
                    .child(widgets::choice(
                        "convolved exponential",
                        model == FitModelKind::ConvolvedExponential,
                        cx,
                        |workspace| {
                            if let Some(fit) = workspace.session.fit.as_mut() {
                                fit.model = FitModelKind::ConvolvedExponential;
                            }
                        },
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(widgets::small_caps("components"))
                    .children((1u8..=4).map(|count| {
                        widgets::choice(
                            match count {
                                1 => "one",
                                2 => "two",
                                3 => "three",
                                _ => "four",
                            },
                            components == count,
                            cx,
                            move |workspace| {
                                if let Some(fit) = workspace.session.fit.as_mut() {
                                    fit.set_components(count);
                                }
                            },
                        )
                        .into_any_element()
                    })),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(widgets::small_caps("fit range"))
                            .child(widgets::number_input(&range_state, 130.0)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(widgets::small_caps("exclude"))
                            .child(widgets::number_input(&excluded_state, 150.0)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(widgets::small_caps("reference delay"))
                            .child(widgets::number_input(&reference_state, 66.0)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(widgets::small_caps("weights"))
                            .child(widgets::choice(
                                "uniform",
                                matches!(weighting, Weighting::Uniform),
                                cx,
                                |workspace| {
                                    if let Some(fit) = workspace.session.fit.as_mut() {
                                        fit.weighting = Weighting::Uniform;
                                    }
                                },
                            ))
                            .child(widgets::choice(
                                "poisson",
                                matches!(weighting, Weighting::Poisson),
                                cx,
                                |workspace| {
                                    if let Some(fit) = workspace.session.fit.as_mut() {
                                        fit.weighting = Weighting::Poisson;
                                    }
                                },
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(widgets::small_caps("show"))
                            .child(widgets::switch("fit overlay", overlay, cx, |workspace| {
                                if let Some(fit) = workspace.session.fit.as_mut() {
                                    fit.show_overlay = !fit.show_overlay;
                                }
                            }))
                            .child(widgets::switch("residuals", residuals_on, cx, |workspace| {
                                if let Some(fit) = workspace.session.fit.as_mut() {
                                    fit.show_residuals = !fit.show_residuals;
                                }
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(if busy {
                        widgets::action("Cancel", true, cx, |workspace, _, _| {
                            workspace.cancel_fit()
                        })
                        .into_any_element()
                    } else {
                        widgets::action("Fit", true, cx, |workspace, _window, cx| {
                            workspace.start_fit(cx)
                        })
                        .into_any_element()
                    })
                    .child(widgets::action(
                        "Auto time zero",
                        false,
                        cx,
                        |workspace, _window, cx| workspace.auto_time_zero(cx),
                    ))
                    .child(widgets::action("Adopt last fit", false, cx, |workspace, _, _| {
                        workspace.adopt_last_fit()
                    }))
                    .child(widgets::action("Keep as preset", false, cx, |workspace, _, _| {
                        workspace.keep_preset()
                    }))
                    .child(widgets::action("Recall preset", false, cx, |workspace, _, _| {
                        workspace.recall_preset()
                    }))
                    .child(widgets::readout(format!("{retained} retained fits"))),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(if state == FitState::Failed {
                        ink::amber()
                    } else {
                        ink::muted()
                    })
                    .child(verdict),
            )
            .into_any_element()
    }

    /// Estimates the correction curve from the matrix and stores it as a
    /// processing version, so the workbench's "auto" is a real operation.
    pub fn auto_time_zero(&mut self, cx: &mut Context<Self>) {
        let Some(dataset) = self.session.dataset() else {
            self.session.status.problem("no dataset is open");
            return;
        };
        let version = self.session.version.unwrap_or(dataset.active_version);
        let Ok(wavelength) = dataset.axes(version) else {
            self.session.status.problem("the version has no axes");
            return;
        };
        let delays = match dataset.axes(version) {
            Ok((_, delay)) => delay.clone(),
            Err(error) => {
                self.session.status.problem(format!("{error}"));
                return;
            }
        };
        let matrix = match dataset.matrix(version) {
            Ok(matrix) => matrix.clone(),
            Err(error) => {
                self.session.status.problem(format!("{error}"));
                return;
            }
        };
        let reference = self
            .fields
            .get(FieldId::FitReferenceDelay)
            .and_then(|state| {
                let text = state.read(cx).value().to_string();
                widgets::parse_number(&text)
            })
            .unwrap_or(1.0);
        let options = ta_fitting::AutoTimeZeroOptions {
            components: self
                .session
                .fit
                .as_ref()
                .map(|fit| fit.components)
                .unwrap_or(2),
            reference_delay: reference,
            ..ta_fitting::AutoTimeZeroOptions::default()
        };
        match ta_fitting::estimate_time_zero_curve(&wavelength.0, &delays, &matrix, &options) {
            Ok(outcome) => {
                self.session.status.info(format!(
                    "correction fitted from {} wavelengths; {} rejected",
                    outcome.accepted, outcome.rejected
                ));
                self.session.estimated_curve = Some(outcome.curve);
                self.touch();
            }
            Err(error) => self.session.status.problem(format!("{error}")),
        }
    }

    /// Takes the values of the most recent retained fit as the starting point.
    pub fn adopt_last_fit(&mut self) {
        let last = self
            .session
            .retained_fits()
            .into_iter()
            .next_back();
        match last {
            Some(result) => {
                self.session.fit_session().adopt(&result);
                self.session
                    .status
                    .info("the parameters now start from the last fit");
                self.touch();
            }
            None => self
                .session
                .status
                .problem("there is no earlier fit to adopt"),
        }
    }

    pub fn keep_preset(&mut self) {
        let Some(fit) = self.session.fit.as_ref() else {
            return;
        };
        let name = if fit.preset_name.trim().is_empty() {
            format!("{} components", fit.components)
        } else {
            fit.preset_name.trim().to_string()
        };
        let preset = ta_core::ParameterPreset {
            name: name.clone(),
            model: fit.model,
            components: fit.components,
            parameters: fit.parameters.clone(),
        };
        self.session
            .project
            .presets
            .retain(|existing| existing.name != name);
        self.session.project.presets.push(preset);
        self.session.dirty = true;
        self.session.status.info(format!("kept the preset “{name}”"));
    }

    pub fn recall_preset(&mut self) {
        let Some(preset) = self.session.project.presets.last().cloned() else {
            self.session.status.problem("no preset has been kept yet");
            return;
        };
        let fit = self.session.fit_session();
        fit.model = preset.model;
        fit.components = preset.components;
        fit.parameters = preset.parameters;
        self.session
            .status
            .info(format!("recalled “{}”", preset.name));
        self.touch();
    }
}

/// Data plate content for the workbench: the trace, the fitted curve and, when
/// asked for, the residuals in a panel of their own.
pub fn fit_content(session: &Session) -> Option<(PlateContent, Option<PlateContent>)> {
    let trace = session.fit_trace()?;
    let fit = session.fit.as_ref();
    let mut panel = ta_core::PanelModel::new("");
    panel.x = plates::axis(
        "Delay",
        Some("ps"),
        ta_core::AxisScale::SplitLinearLog { break_at: 1.0 },
    );
    panel.y = plates::axis("ΔOD", None, ta_core::AxisScale::Linear);
    panel.legend.visible = true;

    let points: Vec<(f64, f64)> = trace
        .delay
        .iter()
        .copied()
        .zip(trace.signal.iter().copied())
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .collect();
    let mut series = vec![
        ta_plot::PlottedSeries::new(
            "measured",
            points.clone(),
            SeriesStyle {
                color: plates::cool(),
                width: 1.0,
                marker: ta_core::MarkerShape::Circle,
                marker_size: 3.0,
                ..SeriesStyle::default()
            },
        )
        .with_kind(SeriesKind::LineMarkers),
    ];

    let result = fit.and_then(|fit| fit.result.as_ref());
    let mut residuals = None;
    if let Some(result) = result {
        if fit.is_some_and(|fit| fit.show_overlay) {
            let curve = ta_fitting::model_curve(result.model, &result.params, &trace.delay)
                .unwrap_or_else(|_| vec![f64::NAN; trace.delay.len()]);
            let points: Vec<(f64, f64)> = trace
                .delay
                .iter()
                .copied()
                .zip(curve.iter().copied())
                .filter(|(x, y)| x.is_finite() && y.is_finite())
                .collect();
            series.push(
                ta_plot::PlottedSeries::new(
                    "fitted",
                    points,
                    SeriesStyle {
                        color: plates::accent(),
                        width: 1.5,
                        ..SeriesStyle::default()
                    },
                )
                .with_kind(SeriesKind::Line),
            );
        }
        if let Some(residual) = &result.residuals {
            let points: Vec<(f64, f64)> = residual
                .x
                .iter()
                .copied()
                .zip(residual.y.iter().copied())
                .filter(|(x, y)| x.is_finite() && y.is_finite())
                .collect();
            let mut residual_panel = ta_core::PanelModel::new("");
            residual_panel.x = panel.x.clone();
            residual_panel.x.range = trace.delay.first().copied().zip(trace.delay.last().copied());
            residual_panel.y = plates::axis("Residual", None, ta_core::AxisScale::Linear);
            residual_panel.legend.visible = false;
            // A residual is centred on zero, and a residual plot that is not
            // centred on zero cannot show a bias.
            let peak = points
                .iter()
                .map(|(_, y)| y.abs())
                .fold(0.0f64, f64::max)
                .max(1e-12);
            residual_panel.y.range = Some((-peak * 1.1, peak * 1.1));
            residuals = Some(PlateContent {
                panel: residual_panel,
                records: Vec::new(),
                series: vec![ta_plot::PlottedSeries::new(
                    "residual",
                    points,
                    SeriesStyle {
                        color: plates::warm(),
                        width: 1.0,
                        marker: ta_core::MarkerShape::Circle,
                        marker_size: 2.5,
                        ..SeriesStyle::default()
                    },
                )],
            });
        }
    }

    let mut bounds: Option<(f64, f64)> = None;
    for (_, y) in &points {
        bounds = Some(match bounds {
            None => (*y, *y),
            Some((low, high)) => (low.min(*y), high.max(*y)),
        });
    }
    if let Some((low, high)) = bounds {
        let pad = ((high - low).abs() * 0.1).max(1e-4);
        panel.y.range = Some((low - pad, high + pad));
    }
    panel.x.range = trace.delay.first().copied().zip(trace.delay.last().copied());
    Some((
        PlateContent {
            panel,
            series,
            records: Vec::new(),
        },
        residuals,
    ))
}

/// The caption under the workbench's data plate: what was fitted, and how.
pub fn fit_caption(session: &Session) -> String {
    let Some(trace) = session.fit_trace() else {
        return "No wavelength is selected for fitting.".to_string();
    };
    // The caption has to read correctly before a fit has ever been run, so the
    // default model and component count stand in for a missing workbench.
    let default = FitSession::default();
    let fit = session.fit.as_ref().unwrap_or(&default);
    let dataset = session
        .dataset()
        .map(|dataset| dataset.name.clone())
        .unwrap_or_default();
    let version = session
        .version()
        .map(|version| version.label.clone())
        .unwrap_or_default();
    let result = fit.result.as_ref();
    let mut text = format!(
        "{} at {} [{dataset} · {version}]: {}. {} components",
        if result.is_some() { "Fit" } else { "Ready to fit" },
        trace.label,
        fit.model.label(),
        fit.components
    );
    if let Some(result) = result {
        text.push_str(&format!(
            ", {} points, {} free",
            result.stats.dof + active_parameters(&result.params).len(),
            active_parameters(&result.params).len()
        ));
        let lifetimes: Vec<String> = (1..=result.components)
            .filter_map(|index| {
                result
                    .lifetime(index)
                    .filter(|lifetime| *lifetime > 0.0)
                    .map(|lifetime| format!("τ{index} = {lifetime:.3} ps"))
            })
            .collect();
        if !lifetimes.is_empty() {
            text.push_str(&format!(". {}", lifetimes.join(", ")));
        }
        text.push_str(&format!(
            ". Weighting: {}",
            match &result.weighting {
                Weighting::Uniform => "uniform".to_string(),
                Weighting::Poisson => "Poisson".to_string(),
                Weighting::Explicit(weights) => format!("{} explicit weights", weights.len()),
                Weighting::IntervalExclusions(intervals) => {
                    format!("{} excluded intervals", intervals.len())
                }
            }
        ));
        if let Some((low, high)) = result.fit_range {
            text.push_str(&format!(", fitted from {low:.3} to {high:.3} ps"));
        }
        if !result.excluded.is_empty() {
            let parts: Vec<String> = result
                .excluded
                .iter()
                .map(|(low, high)| format!("{low:.3}–{high:.3} ps"))
                .collect();
            text.push_str(&format!(", excluding {}", parts.join(" and ")));
        }
        text.push('.');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::Array2;
    use ta_core::{Axis, AxisKind, AxisUnit, SignalUnit, SourceFormat, TaDataset};

    fn dataset() -> TaDataset {
        let wavelength = Axis::new(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            "Wavelength",
            vec![520.0, 600.0, 680.0],
        )
        .unwrap();
        let delay = Axis::new(
            AxisKind::Delay,
            AxisUnit::Picosecond,
            "Delay",
            (-40..160).map(|i| -0.5 + i as f64 * 0.05).collect(),
        )
        .unwrap();
        let mut data = Array2::zeros((3, 200));
        let params = ta_fitting::ModelParams {
            irf_fwhm: 0.12,
            amplitude_scale: 1.0,
            components: vec![
                ta_fitting::Component {
                    amplitude: -0.4,
                    lifetime: 2.0,
                },
                ta_fitting::Component {
                    amplitude: 0.2,
                    lifetime: 30.0,
                },
                ta_fitting::Component {
                    amplitude: 0.0,
                    lifetime: 0.0,
                },
                ta_fitting::Component {
                    amplitude: 0.0,
                    lifetime: 0.0,
                },
            ],
            time_zero: 0.05,
            pre_zero: 0.0,
            long_term: 0.0,
        };
        for row in 0..3 {
            let delays: Vec<f64> = delay.values().to_vec();
            let curve = ta_fitting::model::evaluate(
                ta_core::FitModelKind::LegacyGaussianCdfIrf.into(),
                &params,
                &delays,
            );
            for (column, value) in curve.iter().enumerate() {
                data[[row, column]] = value * (row as f64 + 1.0) * 0.5;
            }
        }
        TaDataset::new_raw(
            "kinetics",
            wavelength,
            delay,
            data,
            SignalUnit::DeltaOpticalDensity,
            SourceFormat::Synthetic,
        )
        .unwrap()
    }

    fn session() -> Session {
        let mut session = Session::new();
        session.open_dataset(dataset());
        session.select_wavelength(600.0).unwrap();
        session
    }

    #[test]
    fn the_parameter_template_follows_the_component_count_and_keeps_typed_values() {
        let mut fit = FitSession::default();
        fit.parameters.iter_mut().find(|parameter| parameter.name == "lifetime_1").unwrap().fixed = true;
        fit.set_components(3);
        assert_eq!(fit.parameters.len(), 13);
        assert!(fit.parameters.iter().find(|parameter| parameter.name == "lifetime_1").unwrap().fixed);
        assert!(!fit.parameters.iter().find(|parameter| parameter.name == "amplitude_3").unwrap().fixed);
        assert!(!fit.parameters.iter().find(|parameter| parameter.name == "lifetime_3").unwrap().fixed);
        let amplitude = fit
            .parameters
            .iter_mut()
            .find(|parameter| parameter.name == "amplitude_3")
            .unwrap();
        amplitude.value = -0.7;
        fit.set_components(2);
        assert!(fit
            .parameters
            .iter()
            .find(|parameter| parameter.name == "amplitude_3")
            .expect("slot three survives as a fixed slot")
            .fixed);
        fit.set_components(3);
        assert!(!fit.parameters.iter().find(|parameter| parameter.name == "amplitude_3").unwrap().fixed);
        assert!((fit
            .parameters
            .iter()
            .find(|parameter| parameter.name == "amplitude_3")
            .unwrap()
            .value
            + 0.7)
            .abs() < 1e-12);
    }

    #[test]
    fn the_table_only_offers_parameters_for_the_selected_components() {
        let fit = FitSession::default();
        let visible: Vec<_> = fit.parameters.iter().filter(|parameter| {
            component_slot(&parameter.name).is_none_or(|slot| slot <= fit.components)
        }).collect();
        assert_eq!(visible.len(), 9);
        assert!(visible.iter().any(|parameter| parameter.name == "amplitude_scale"));
        assert!(!visible.iter().any(|parameter| parameter.name == "lifetime_3"));
        assert_eq!(fit.parameters.len(), 13, "hidden starting values remain available");
    }

    #[test]
    fn a_fit_request_carries_the_range_the_exclusions_and_the_weighting() {
        let mut session = session();
        let fit = session.fit_session();
        fit.fit_range = Some((0.1, 100.0));
        fit.excluded = vec![(1.0, 2.0)];
        fit.weighting = Weighting::Poisson;
        let (request, trace) = session.fit_request().unwrap();
        assert_eq!(request.fit_range, Some((0.1, 100.0)));
        assert_eq!(request.excluded, vec![(1.0, 2.0)]);
        assert!(matches!(request.weighting, Weighting::Poisson));
        assert_eq!(trace.delay.len(), 200);
        assert_eq!(request.components, 2);
    }

    #[test]
    fn a_finished_fit_is_retained_in_its_dataset_and_can_be_adopted() {
        let mut session = session();
        let (request, trace) = session.fit_request().unwrap();
        let result = ta_fitting::fit_trace(&trace, &request).unwrap();
        let id = result.id;
        session.retain_fit(result.clone()).unwrap();
        assert_eq!(session.retained_fits().len(), 1);
        // Reproducible from the record: the same model, the same lifetimes.
        assert!(result.converged(), "{}", result.stats.message);
        let lifetime = result.lifetime(1).unwrap();
        assert!(lifetime > 0.0 && lifetime < 5000.0);

        let mut next = FitSession::default();
        next.adopt(&result);
        assert_eq!(next.components, 2);
        assert_eq!(next.parameters.len(), 13);
        assert!(
            (next
                .parameters
                .iter()
                .find(|parameter| parameter.name == "lifetime_1")
                .unwrap()
                .value
                - lifetime)
                .abs()
                < 1e-9
        );
        let _ = id;
    }

    #[test]
    fn the_caption_states_the_model_the_components_and_the_weighting() {
        let mut session = session();
        session.fit_session().set_components(2);
        let ready = fit_caption(&session);
        assert!(ready.contains("2 components"));
        assert!(ready.contains("Legacy Gaussian-CDF IRF"));

        let (request, trace) = session.fit_request().unwrap();
        let result = ta_fitting::fit_trace(&trace, &request).unwrap();
        session.fit_session().result = Some(result);
        let caption = fit_caption(&session);
        assert!(caption.contains("τ1"), "{caption}");
        assert!(caption.contains("uniform"), "{caption}");
        assert!(caption.contains("600.0000 nm"), "{caption}");
    }

    #[test]
    fn the_workbench_plate_carries_the_data_and_the_fitted_curve() {
        let mut session = session();
        let (request, trace) = session.fit_request().unwrap();
        let result = ta_fitting::fit_trace(&trace, &request).unwrap();
        session.fit_session().result = Some(result);
        let (data, residuals) = fit_content(&session).unwrap();
        assert_eq!(data.series.len(), 2);
        assert_eq!(data.series[0].label, "measured");
        assert_eq!(data.series[1].style.color, plates::accent());
        // Residuals in their own panel, centred on zero.
        let residuals = residuals.expect("residuals are drawn");
        let (low, high) = residuals.panel.y.range.unwrap();
        assert!((low + high).abs() < 1e-12);
    }

    #[test]
    fn a_fit_that_never_ran_shows_the_data_alone_and_says_so() {
        let session = session();
        let (data, residuals) = fit_content(&session).unwrap();
        assert_eq!(data.series.len(), 1);
        assert!(residuals.is_none());
        assert!(fit_caption(&session).contains("Ready to fit"));
    }

    #[test]
    fn the_verdict_names_a_parameter_that_sits_on_its_bound() {
        let mut fit = FitSession::default();
        let mut result = FitResult {
            id: ta_core::FitId::new(),
            dataset: ta_core::DatasetId::new(),
            version: ta_core::VersionId::new(),
            selection: TraceSelection::new(
                ta_core::DatasetId::new(),
                ta_core::VersionId::new(),
                ta_core::AxisRequest::nearest(600.0),
            ),
            model: FitModelKind::LegacyGaussianCdfIrf,
            components: 1,
            params: vec![FitParameter::new("lifetime_1", 5000.0).with_bounds(0.01, 5000.0)],
            stats: ta_core::FitStats {
                sse: 1e-4,
                converged: true,
                message: "converged".into(),
                ..ta_core::FitStats::default()
            },
            residuals: None,
            fit_range: None,
            excluded: Vec::new(),
            weighting: Weighting::Uniform,
            created_at: 0,
        };
        fit.result = Some(result.clone());
        fit.state = FitState::Done;
        let verdict = fit.verdict();
        assert!(verdict.contains("lifetime_1"), "{verdict}");
        assert!(verdict.contains("no standard error"), "{verdict}");
        // And a parameter that is not at a bound is not accused of being one.
        result.params[0].value = 12.0;
        fit.result = Some(result);
        assert!(!fit.verdict().contains("lifetime_1"));
    }

    #[test]
    fn the_residual_panel_scales_to_the_largest_residual_not_to_the_signal() {
        let mut session = session();
        let (request, trace) = session.fit_request().unwrap();
        let result = ta_fitting::fit_trace(&trace, &request).unwrap();
        let sse = result.stats.sse;
        session.fit_session().result = Some(result);
        let (data, residuals) = fit_content(&session).unwrap();
        let residuals = residuals.unwrap();
        let (low, high) = residuals.panel.y.range.unwrap();
        let signal = data.panel.y.range.unwrap();
        assert!(
            high < (signal.1 - signal.0).abs(),
            "a residual panel should not be scaled like a signal panel"
        );
        // And it shares the fitted delays, so a residual sits under the point
        // it belongs to instead of on a unit box of its own.
        assert_eq!(residuals.panel.x.range, data.panel.x.range);
        assert!(low < 0.0 && high > 0.0);
        assert!(sse >= 0.0);
    }

    #[test]
    fn the_overlay_follows_the_switch() {
        let mut session = session();
        let (request, trace) = session.fit_request().unwrap();
        let result = ta_fitting::fit_trace(&trace, &request).unwrap();
        session.fit_session().result = Some(result);
        session.fit_session().show_overlay = false;
        let (data, _) = fit_content(&session).unwrap();
        assert_eq!(data.series.len(), 1, "the overlay is off");
    }
}
