//! The four layouts of the window.
//!
//! `Figure` is the single-dataset shell; `Compare` is §15's multi-object
//! workspace; `Studio` is §17's publication editor; `Fit` is §12's workbench.
//! They share the plate renderer, the caption vocabulary and the editable
//! fields, so a curve behaves the same wherever it is looked at.

use crate::app::{ink, Layout, Workspace};
use crate::studio;
use crate::widgets::{self, FieldId};
use gpui_kit::{
    div, px, AnyElement, Context, ElementId, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window,
};
use ta_core::{AnnotationKind, AxisScale, ExportFormat};

impl Workspace {
    /// The strip that names the layouts, with the one in force filled in.
    pub fn layout_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut tabs: Vec<AnyElement> = Vec::new();
        for layout in Layout::ALL {
            let active = self.layout == layout;
            tabs.push(
                gpui_kit::base::Button::new(ElementId::Name(format!("layout-{}", layout.label()).into()))
                    .accessibility_label(layout.label())
                    .selected(active)
                    .cursor_pointer()
                    .min_h(px(30.0))
                    .hover(|style| style.bg(ink::paper()))
                    .focus(|style| style.underline().bg(ink::hair()))
                    .flex()
                    .flex_col()
                    .px_3()
                    .py_1()
                    .border_b_2()
                    .border_color(if active { ink::accent() } else { ink::paper() })
                    .bg(if active { ink::paper() } else { ink::ground() })
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(if active { ink::accent() } else { ink::text() })
                            .child(layout.label()),
                    )
                    .on_click(cx.listener(move |workspace, _event, _window, cx| {
                        workspace.layout = layout;
                        if layout == Layout::Compare {
                            // The comparison is created on first use rather
                            // than at startup, so a user who never compares
                            // never carries an empty workspace around. It opens
                            // on what is loaded: an empty pair of panes is a
                            // box, not a comparison.
                            workspace.session.comparison_mut();
                            if workspace.session.comparison_series().is_empty() {
                                let _ = workspace.session.add_every_dataset();
                            }
                            workspace.touch_matrix();
                        }
                        if layout == Layout::Studio && workspace.session.studio.is_none() {
                            let plate = workspace.current_plate;
                            workspace.open_studio(plate);
                        }
                        workspace.touch();
                        workspace.sync_studio_fields(_window, cx);
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        div()
            .w_full()
            .flex()
            .items_center()
            .bg(ink::ground())
            .border_b_1()
            .border_color(ink::rule())
            .children(tabs)
            .child(if self.layout == Layout::Figure {
                div().flex().items_center().gap_2().px_2()
                    .child(widgets::switch("Datasets", self.show_index, cx, |workspace| {
                        workspace.show_index = !workspace.show_index;
                    }))
                    .child(widgets::switch("Inspector", self.show_inspector, cx, |workspace| {
                        workspace.show_inspector = !workspace.show_inspector;
                    }))
                    .into_any_element()
            } else {
                div().into_any_element()
            })
            .child(
                div()
                    .flex_1()
                    .flex()
                    .justify_end()
                    .px_4()
                    .pb_1()
                    .text_size(px(11.0))
                    .text_color(ink::muted())
                    .child(self.layout.note()),
            )
    }

    /// §15's comparison layout: a toolbar, two panes, and the series list.
    pub fn compare_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        // The series list is the only part that needs the workspace mutably,
        // so it is built before anything borrows the plates.
        let list = self.compare_series_list(window, cx);
        let toolbar = self.compare_toolbar(cx);
        let left = self.compare_pane(0);
        let right = self.compare_pane(1);
        div()
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(
                                gpui_kit::component::h_resizable("compare-panes")
                                    .child(
                                        gpui_kit::component::resizable_panel()
                                            .child(self.captioned_plate(left, "spectra", &self.compare_caption.1, cx)),
                                    )
                                    .child(
                                        gpui_kit::component::resizable_panel()
                                            .child(self.captioned_plate(right, "kinetics", &self.compare_caption.0, cx)),
                                    ),
                            ),
                    )
                    .child(list),
            )
            .into_any_element()
    }

    /// A plate with the ranked caption the direction asks for, so an exported
    /// comparison and a drawn one say the same thing about themselves.
    fn captioned_plate(
        &self,
        element: impl IntoElement,
        key: &str,
        caption: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let spectra = key == "spectra";
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(ink::paper())
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .min_h(px(60.0))
                    .child(element),
            )
            .child(
                div()
                    .px_2()
                    .py_1()
                    .border_t_1()
                    .border_color(ink::hair())
                    .text_size(px(11.0))
                    .text_color(ink::text())
                    .child(widgets::action_id(
                        "FigureStudio",
                        ElementId::Name(format!("studio-compare-{key}").into()),
                        false,
                        cx,
                        move |workspace, _, _| workspace.open_comparison_studio(spectra),
                    ))
                    .child(format!("[{key}] {caption}")),
            )
    }

    /// §12's workbench: the controls, the parameters, the data and the
    /// residuals, with the fit's own verdict under the plates.
    pub fn fit_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let controls = self.fit_controls(window, cx);
        let table = self.fit_parameter_table(window, cx);
        let data = self.fit_plates.first().map(|plate| {
            plate.element(ElementId::Name("fit-data".into()))
        });
        let residuals = self.fit_plates.get(1).map(|plate| {
            plate.element(ElementId::Name("fit-residuals".into()))
        });
        let caption = self.fit_caption.clone();
        div()
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .w_full()
                    .bg(ink::ground())
                    .border_b_1()
                    .border_color(ink::rule())
                    .child(controls),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .flex_1()
                                    .min_h(px(60.0))
                                    .child(match data {
                                        Some(element) => element.into_any_element(),
                                        None => empty_plate("Select a wavelength to fit."),
                                    }),
                            )
                            .child(
                                div()
                                    .h(px(150.0))
                                    .child(match residuals {
                                        Some(element) => element.into_any_element(),
                                        None => empty_plate("Residuals appear here."),
                                    }),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .py_1()
                                    .border_t_1()
                                    .border_color(ink::hair())
                                    .text_size(px(11.0))
                                    .text_color(ink::text())
                                    .child(caption),
                            ),
                    )
                    .child(
                        div()
                            .w(px(if self.fit_edit_parameters { 560.0 } else { 392.0 }))
                            .flex_shrink_0()
                            .h_full()
                            .flex()
                            .flex_col()
                            .bg(ink::ground())
                            .border_l_1()
                            .border_color(ink::rule())
                            // The parameter table is where a fit is *driven*,
                            // so it shows by default; the results column is
                            // what a reader returns for, so the table can be
                            // folded away to leave the fitted values.
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_3()
                                    .border_b_1()
                                    .border_color(ink::rule())
                                    .child(widgets::small_caps_in_panel("Parameters"))
                                    .child(
                                        div().px_3().py_2().child(widgets::switch(
                                            "edit start values and bounds",
                                            self.fit_edit_parameters,
                                            cx,
                                            |workspace| {
                                                workspace.fit_edit_parameters =
                                                    !workspace.fit_edit_parameters;
                                                workspace.touch();
                                            },
                                        )),
                                    ),
                            )
                            .child(
                                div()
                                    .id(ElementId::Name("fit-parameters".into()))
                                    .flex_1()
                                    .min_h(px(0.0))
                                    .overflow_y_scroll()
                                    .child(table),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// §17's studio: structure on the left, the figure at publication size in
    /// the centre, properties on the right, export along the bottom.
    pub fn studio_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let structure = self.studio_structure(window, cx);
        let properties = self.studio_properties(window, cx);
        let preview = self.studio_plates.first().map(|plate| {
            plate.element(ElementId::Name("studio-preview".into()))
        });
        let export_bar = self.studio_export_bar(cx);
        let caption = self
            .studio_figure()
            .map(|figure| {
                format!(
                    "{} · {:.3} × {:.3} {} · {} panel(s) · {}",
                    figure.title,
                    figure.size.width,
                    figure.size.height,
                    figure.size.unit.symbol(),
                    figure.panels.len(),
                    match figure.export.format {
                        ExportFormat::Svg => "SVG by default".to_string(),
                        ExportFormat::Png => format!("PNG at {} dpi", figure.export.dpi),
                        ExportFormat::Pdf => "PDF".to_string(),
                        ExportFormat::Eps => "EPS".to_string(),
                    }
                )
            })
            .unwrap_or_else(|| "No figure is open in the studio.".to_string());

        div()
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .child(
                        div()
                            .w(px(232.0))
                            .h_full()
                            .flex()
                            .flex_col()
                            .bg(ink::ground())
                            .border_r_1()
                            .border_color(ink::rule())
                            .child(widgets::small_caps_in_panel("Figure"))
                            .child(
                                div()
                                    .id(ElementId::Name("studio-structure".into()))
                                    .flex_1()
                                    .min_h(px(0.0))
                                    .overflow_y_scroll()
                                    .child(structure),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_col()
                            .bg(ink::ground())
                            .p_3()
                            .child(self.preview_zoom_bar(cx))
                            .child(
                                div()
                                    .flex_1()
                                    .min_h(px(60.0))
                                    .w_full()
                                    // A pane clips its plate: on the first
                                    // layout pass a column can be measured
                                    // before the ones beside it, and a figure
                                    // that briefly thinks it is wider than its
                                    // column must not paint over it.
                                    .overflow_hidden()
                                    .child(match preview {
                                        Some(element) => element.into_any_element(),
                                        None => empty_plate("Open FigureStudio from a plate."),
                                    }),
                            )
                            .child(
                                div()
                                    .pt_2()
                                    .text_size(px(11.0))
                                    .text_color(ink::muted())
                                    .child(caption),
                            ),
                    )
                    .child(
                        div()
                            .w(px(292.0))
                            .h_full()
                            .flex()
                            .flex_col()
                            .bg(ink::ground())
                            .border_l_1()
                            .border_color(ink::rule())
                            .child(widgets::small_caps_in_panel("Properties"))
                            .child(
                                div()
                                    .id(ElementId::Name("studio-properties".into()))
                                    .flex_1()
                                    .min_h(px(0.0))
                                    .overflow_y_scroll()
                                    .child(properties),
                            ),
                    ),
            )
            .child(export_bar)
            .into_any_element()
    }

    /// The figure's structure: its panels, and each panel's series in paint
    /// order, because plotting order is a property a user edits by clicking.
    fn studio_structure(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(figure) = self.studio_figure().cloned() else {
            return div().into_any_element();
        };
        let mut rows: Vec<AnyElement> = vec![
            div()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(ink::hair())
                .child(widgets::value_text(figure.title.clone()))
                .into_any_element(),
        ];
        for (panel_index, panel) in figure.panels.iter().enumerate() {
            rows.push(
                div()
                    .px_3()
                    .py_1()
                    .text_size(px(10.5))
                    .text_color(ink::muted())
                    .child(format!(
                        "PANEL {} · {} series",
                        panel_index + 1,
                        panel.series.len()
                    ))
                    .into_any_element(),
            );
            if panel.contour.is_some() {
                rows.push(
                    div()
                        .px_3()
                        .py_1()
                        .text_size(px(11.0))
                        .text_color(ink::text())
                        .child("ΔA contour (matrix + isolines)")
                        .into_any_element(),
                );
            }
            for (series_index, series) in panel.series.iter().enumerate() {
                let id = series.id;
                let label = if series.label.trim().is_empty() {
                    format!("series {}", series_index + 1)
                } else {
                    series.label.clone()
                };
                let legend_state = self.field_state(
                    FieldId::LegendLabel(series_index),
                    panel
                        .legend
                        .labels
                        .get(&id)
                        .cloned()
                        .unwrap_or_else(|| series.label.clone()),
                    "legend label",
                    window,
                    cx,
                );
                rows.push(
                    div()
                        .id(ElementId::Name(format!("structure-{id}").into()))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .px_3()
                        .py_1()
                        .border_b_1()
                        .border_color(ink::hair())
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(studio::swatch(series.style.color, false))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .text_size(px(11.0))
                                        .text_color(ink::text())
                                        .child(format!(
                                            "{}. {}",
                                            series_index + 1,
                                            label
                                        )),
                                )
                                .child(widgets::switch(
                                    "shown",
                                    series.visible,
                                    cx,
                                    move |workspace| {
                                        if let Some(figure) = workspace.session.studio.as_mut() {
                                            if let Some(series) = figure
                                                .panel_mut(0)
                                                .and_then(|panel| {
                                                    panel
                                                        .series
                                                        .iter_mut()
                                                        .find(|candidate| candidate.id == id)
                                                })
                                            {
                                                series.visible = !series.visible;
                                            }
                                        }
                                        workspace.touch();
                                    },
                                ))
                                .child(widgets::action_id(
                                    "↑",
                                    ElementId::Name(format!("studio-up-{id}").into()),
                                    false,
                                    cx,
                                    move |workspace, _, _| workspace.move_series(series_index, -1),
                                ).disabled(series_index == 0))
                                .child(widgets::action_id(
                                    "↓",
                                    ElementId::Name(format!("studio-down-{id}").into()),
                                    false,
                                    cx,
                                    move |workspace, _, _| workspace.move_series(series_index, 1),
                                ).disabled(series_index + 1 == panel.series.len())),
                        )
                        .child(widgets::text_input(&legend_state, 190.0))
                        .into_any_element(),
                );
            }
        }
        div().flex().flex_col().children(rows).into_any_element()
    }

    fn selected_line_controls(
        &mut self,
        panel: &ta_core::PanelModel,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use gpui_kit::component::{checkbox::Checkbox, Sizable};
        self.studio_selection
            .retain(|id| panel.series.iter().any(|series| series.id == *id));
        let selected: Vec<_> = panel
            .series
            .iter()
            .filter(|series| self.studio_selection.contains(&series.id))
            .collect();
        let empty = selected.is_empty();
        let common = |matches: &dyn Fn(&ta_core::PlotSeries) -> bool| {
            !empty && selected.iter().all(|series| matches(series))
        };
        let rows = panel
            .series
            .iter()
            .enumerate()
            .map(|(index, series)| {
                let id = series.id;
                Checkbox::new(ElementId::Name(format!("select-line-{id}").into()))
                    .small()
                    .label(format!("{} · {}", index + 1, short(&series.label)))
                    .checked(self.studio_selection.contains(&id))
                    .on_click(cx.listener(move |workspace, checked: &bool, _, cx| {
                        if *checked {
                            workspace.studio_selection.insert(id);
                            workspace
                                .inspector_sections
                                .insert("studio-line-style".into(), true);
                        } else {
                            workspace.studio_selection.remove(&id);
                        }
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let colors = studio::series_colors()
            .into_iter()
            .map(|color| {
                gpui_kit::base::Button::new(ElementId::Name(
                    format!("selected-color-{}", color.to_hex()).into(),
                ))
                .accessibility_label(format!("Selected lines color {}", color.to_hex()))
                .disabled(empty)
                .size(px(24.0))
                .focus(|style| style.border_2().border_color(ink::accent()))
                .child(studio::swatch(
                    color,
                    common(&|series| series.style.color == color),
                ))
                .on_click(cx.listener(move |workspace, _, _, cx| {
                    workspace.edit_selected_series(|series| series.style.color = color);
                    cx.notify();
                }))
                .into_any_element()
            })
            .collect::<Vec<_>>();
        let controls =
            div()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .child(widgets::small_caps("Colour"))
                .child(div().flex().flex_wrap().gap_1().children(colors))
                .child(widgets::small_caps("Line width · relative adjustment"))
                .child(
                    div()
                        .flex()
                        .gap_3()
                        .child(
                            widgets::action_id(
                                "Thinner",
                                "selected-thinner".into(),
                                false,
                                cx,
                                |workspace, _, _| {
                                    workspace.edit_selected_series(|series| {
                                        series.style.width = (series.style.width - 0.5).max(0.25)
                                    });
                                },
                            )
                            .disabled(empty),
                        )
                        .child(
                            widgets::action_id(
                                "Thicker",
                                "selected-thicker".into(),
                                false,
                                cx,
                                |workspace, _, _| {
                                    workspace.edit_selected_series(|series| {
                                        series.style.width = (series.style.width + 0.5).min(8.0)
                                    });
                                },
                            )
                            .disabled(empty),
                        ),
                )
                .child(widgets::small_caps("Dash"))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .children(studio::dashes().into_iter().map(|(label, dash)| {
                            widgets::choice(
                                label,
                                common(&|series| series.style.dash == dash),
                                cx,
                                move |workspace| {
                                    workspace
                                        .edit_selected_series(|series| series.style.dash = dash);
                                },
                            )
                            .disabled(empty)
                            .into_any_element()
                        })),
                )
                .child(widgets::small_caps("Markers"))
                .child(div().flex().flex_wrap().gap_1().children(
                    studio::markers().into_iter().map(|(label, marker)| {
                        widgets::choice(
                            label,
                            common(&|series| series.style.marker == marker),
                            cx,
                            move |workspace| {
                                for id in workspace.studio_selection.clone() {
                                    workspace.set_series_marker(id, marker);
                                }
                            },
                        )
                        .disabled(empty)
                        .into_any_element()
                    }),
                ))
                .child(widgets::small_caps("Marker size · relative adjustment"))
                .child(
                    div()
                        .flex()
                        .gap_3()
                        .child(
                            widgets::action_id(
                                "Smaller",
                                "selected-smaller".into(),
                                false,
                                cx,
                                |workspace, _, _| {
                                    workspace.edit_selected_series(|series| {
                                        series.style.marker_size =
                                            (series.style.marker_size - 1.0).max(1.0)
                                    });
                                },
                            )
                            .disabled(empty),
                        )
                        .child(
                            widgets::action_id(
                                "Larger",
                                "selected-larger".into(),
                                false,
                                cx,
                                |workspace, _, _| {
                                    workspace.edit_selected_series(|series| {
                                        series.style.marker_size =
                                            (series.style.marker_size + 1.0).min(20.0)
                                    });
                                },
                            )
                            .disabled(empty),
                        ),
                )
                .child(widgets::small_caps("Opacity · relative adjustment"))
                .child(
                    div()
                        .flex()
                        .gap_3()
                        .child(
                            widgets::action_id(
                                "Fainter",
                                "selected-fainter".into(),
                                false,
                                cx,
                                |workspace, _, _| {
                                    workspace.edit_selected_series(|series| {
                                        series.style.opacity = (series.style.opacity - 0.1).max(0.1)
                                    });
                                },
                            )
                            .disabled(empty),
                        )
                        .child(
                            widgets::action_id(
                                "Stronger",
                                "selected-stronger".into(),
                                false,
                                cx,
                                |workspace, _, _| {
                                    workspace.edit_selected_series(|series| {
                                        series.style.opacity = (series.style.opacity + 0.1).min(1.0)
                                    });
                                },
                            )
                            .disabled(empty),
                        ),
                )
                .into_any_element();
        let targets = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .child(widgets::readout(format!(
                "{} of {} lines selected",
                selected.len(),
                panel.series.len()
            )))
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(widgets::action(
                        "Select all",
                        false,
                        cx,
                        |workspace, _, _| {
                            workspace.studio_selection = workspace
                                .session
                                .studio
                                .as_ref()
                                .and_then(|figure| figure.panel(0))
                                .map(|panel| panel.series.iter().map(|series| series.id).collect())
                                .unwrap_or_default();
                            workspace
                                .inspector_sections
                                .insert("studio-line-style".into(), true);
                        },
                    ))
                    .child(widgets::action(
                        "Clear selection",
                        false,
                        cx,
                        |workspace, _, _| {
                            workspace.studio_selection.clear();
                            workspace
                                .inspector_sections
                                .insert("studio-line-style".into(), false);
                        },
                    )),
            )
            .child(
                div()
                    .id("line-selection-list")
                    .flex()
                    .flex_col()
                    .gap_2()
                    .max_h(px(96.0))
                    .overflow_y_scroll()
                    .children(rows),
            )
            .child(widgets::readout(
                "Only the property you change is applied. Other styles stay independent.",
            ));
        div()
            .flex()
            .flex_col()
            .child(targets)
            .child(self.inspector_section(
                "studio-line-style",
                "Style selected lines",
                false,
                controls,
                cx,
            ))
            .into_any_element()
    }

    /// The properties column: size, series style, axes, legend, contour,
    /// annotations — the sections of §17.3 to §17.9.
    fn studio_properties(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(figure) = self.studio_figure().cloned() else {
            return div().into_any_element();
        };
        let panel = figure.panel(0).cloned().unwrap_or_else(|| ta_core::PanelModel::new(""));
        let mut sections: Vec<AnyElement> = Vec::new();

        // -- size (§17.3)
        let width_state = self.field_state(
            FieldId::FigureWidth,
            format!("{:.3}", figure.size.width),
            "3.25",
            window,
            cx,
        );
        let height_state = self.field_state(
            FieldId::FigureHeight,
            format!("{:.3}", figure.size.height),
            "2.5",
            window,
            cx,
        );
        let font_state = self.field_state(
            FieldId::FigureFontSize,
            format!("{:.1}", figure.style.base_font_size),
            "8",
            window,
            cx,
        );
        let tick_state = self.field_state(
            FieldId::FigureTickSize,
            format!("{:.1}", panel.x.tick_label_size),
            "7",
            window,
            cx,
        );
        let legend_font_state = self.field_state(
            FieldId::LegendFontSize,
            format!("{:.1}", panel.legend.font_size),
            "7",
            window,
            cx,
        );
        sections.push(self.inspector_section(
            "studio-canvas", "Canvas & typography", false,
            div()
                .flex()
                .flex_col()
                .child(widgets::field(
                    "Size",
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(widgets::number_input(&width_state, 62.0))
                        .child(widgets::readout("×"))
                        .child(widgets::number_input(&height_state, 62.0))
                        .child(widgets::readout(figure.size.unit.symbol())),
                ))
                .child(widgets::field(
                    "Unit",
                    div()
                        .flex()
                        .gap_1()
                        .children(studio::units().into_iter().map(|(label, unit)| {
                            widgets::choice(label, figure.size.unit == unit, cx, move |workspace| {
                                if let Some(figure) = workspace.session.studio.as_mut() {
                                    // The number follows the unit, so a size
                                    // survives a change of unit unchanged.
                                    let (width, height) = figure.size.in_same_unit(unit);
                                    figure.size = ta_core::FigureSize {
                                        width: (width * 1000.0).round() / 1000.0,
                                        height: (height * 1000.0).round() / 1000.0,
                                        unit,
                                    };
                                }
                                workspace.touch();
                            })
                            .into_any_element()
                        })),
                ))
                .child(widgets::field(
                    "Journal preset",
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .children(studio::SIZE_PRESETS.into_iter().map(|preset| {
                            let label = studio::preset_label(preset);
                            widgets::choice(
                                label,
                                figure.style.preset == preset,
                                cx,
                                move |workspace| {
                                    if let Some(figure) = workspace.session.studio.as_mut() {
                                        if preset == ta_core::JournalPreset::Custom {
                                            figure.style.preset = preset;
                                        } else {
                                            figure.apply_preset(preset);
                                        }
                                    }
                                    workspace.touch();
                                },
                            )
                            .into_any_element()
                        })),
                ))
                .child(widgets::field(
                    "Type size",
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(widgets::number_input(&font_state, 52.0))
                        .child(widgets::readout("pt base")),
                ))
                .child(widgets::field(
                    "Tick and legend type",
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(widgets::number_input(&tick_state, 52.0))
                        .child(widgets::readout("pt ticks ·"))
                        .child(widgets::number_input(&legend_font_state, 52.0))
                        .child(widgets::readout("pt legend")),
                ))
                .into_any_element(),
            cx,
        ).into_any_element());

        // -- figure style (§17.12)
        sections.push(self.inspector_section(
            "studio-presets", "Appearance presets", false,
            widgets::field(
                "Figure style",
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children(studio::style_presets().into_iter().map(|(label, mut style)| {
                        style.preset = figure.style.preset;
                        let active = figure.style == style;
                        widgets::choice(label, active, cx, move |workspace| {
                            workspace.apply_style_preset(label)
                        })
                        .into_any_element()
                    })),
            )
            .into_any_element(),
            cx,
        ).into_any_element());

        // -- series style (§17.4)
        for (index, series) in panel.series.iter().enumerate() {
            let id = series.id;
            let color = series.style.color;
            let style = series.style.clone();
            let kind = series.kind;
            let legend_state = self.field_state(
                FieldId::LegendLabel(index),
                panel
                    .legend
                    .labels
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| series.label.clone()),
                "legend label",
                window,
                cx,
            );
            sections.push(self.inspector_section(
                format!("studio-line-{id}"),
                format!("Line {} · {}", index + 1, short(&series.label)), false,
                div()
                    .flex()
                    .flex_col()
                    .child(widgets::field(
                        "Style and legend label",
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .children(studio::series_colors().into_iter().map(|chosen| {
                                        gpui_kit::base::Button::new(ElementId::Name(
                                            format!("color-{}-{}", id, chosen.to_hex()).into(),
                                        ))
                                            .accessibility_label(format!("Series {} color {}", index + 1, chosen.to_hex()))
                                            .cursor_pointer()
                                            .size(px(20.0))
                                            .focus(|style| style.border_2().border_color(ink::accent()))
                                            .child(studio::swatch(chosen, chosen == color))
                                            .on_click(cx.listener(
                                                move |workspace, _event, _window, cx| {
                                                    workspace.edit_series(id, move |series| {
                                                        series.style.color = chosen;
                                                    });
                                                    cx.notify();
                                                },
                                            ))
                                            .into_any_element()
                                    })),
                            )
                            .child(widgets::text_input(&legend_state, 250.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(widgets::small_caps("line"))
                                    .child(widgets::readout(format!("{:.1} pt", style.width)))
                                    .child(widgets::action_id(
                                        "thicker",
                                        ElementId::Name(format!("thick-{id}").into()),
                                        false,
                                        cx,
                                        move |workspace, _, _| {
                                            workspace.edit_series(id, |series| {
                                                series.style.width =
                                                    (series.style.width + 0.5).min(8.0);
                                            })
                                        },
                                    ).disabled(style.width >= 8.0 || matches!(kind, ta_core::SeriesKind::Markers | ta_core::SeriesKind::Scatter)))
                                    .child(widgets::action_id(
                                        "thinner",
                                        ElementId::Name(format!("thin-{id}").into()),
                                        false,
                                        cx,
                                        move |workspace, _, _| {
                                            workspace.edit_series(id, |series| {
                                                series.style.width =
                                                    (series.style.width - 0.5).max(0.25);
                                            })
                                        },
                                    ).disabled(style.width <= 0.25 || matches!(kind, ta_core::SeriesKind::Markers | ta_core::SeriesKind::Scatter))),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_1()
                                    .children(studio::dashes().into_iter().map(|(label, dash)| {
                                        widgets::choice(
                                            label,
                                            style.dash == dash,
                                            cx,
                                            move |workspace| {
                                                workspace.edit_series(id, move |series| {
                                                    series.style.dash = dash;
                                                })
                                            },
                                        ).disabled(matches!(kind, ta_core::SeriesKind::Markers | ta_core::SeriesKind::Scatter | ta_core::SeriesKind::Bars))
                                        .into_any_element()
                                    })),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_1()
                                    .children(studio::markers().into_iter().map(|(label, marker)| {
                                        widgets::choice(
                                            label,
                                            if matches!(kind, ta_core::SeriesKind::Markers | ta_core::SeriesKind::LineMarkers | ta_core::SeriesKind::Scatter) {
                                                style.marker == marker
                                            } else { marker == ta_core::MarkerShape::None },
                                            cx,
                                            move |workspace| {
                                                workspace.set_series_marker(id, marker)
                                            },
                                        )
                                        .into_any_element()
                                    })),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_1()
                                    .children(studio::kinds().into_iter().map(|(label, chosen)| {
                                        widgets::choice(
                                            label,
                                            kind == chosen,
                                            cx,
                                            move |workspace| {
                                                workspace.set_series_kind(id, chosen)
                                            },
                                        )
                                        .into_any_element()
                                    })),
                            ),
                    ))
                    .into_any_element(),
                cx,
            ).into_any_element());
        }

        // -- axes (§17.5)
        let x_label = self.field_state(
            FieldId::AxisXLabel,
            panel.x.label.clone(),
            "Wavelength",
            window,
            cx,
        );
        let y_label = self.field_state(
            FieldId::AxisYLabel,
            panel.y.label.clone(),
            "ΔOD",
            window,
            cx,
        );
        let x_range = self.field_state(
            FieldId::AxisXRange,
            widgets::format_range(panel.x.range),
            "auto",
            window,
            cx,
        );
        let y_range = self.field_state(
            FieldId::AxisYRange,
            widgets::format_range(panel.y.range),
            "auto",
            window,
            cx,
        );
        sections.push(self.inspector_section(
            "studio-axes", "Axes & grid", false,
            div()
                .flex()
                .flex_col()
                .child(widgets::field(
                    "X axis",
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(widgets::text_input(&x_label, 250.0))
                        .child(widgets::number_input(&x_range, 250.0))
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_1()
                                .children(studio::scale_choices().into_iter().map(
                                    |(label, scale)| {
                                        let selected = same_scale(panel.x.scale, scale);
                                        widgets::choice(label, selected, cx, move |workspace| {
                                            workspace.edit_axis(true, move |axis| {
                                                axis.scale = scale;
                                            })
                                        })
                                        .into_any_element()
                                    },
                                )),
                        ),
                ))
                .child(widgets::field(
                    "Y axis",
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(widgets::text_input(&y_label, 250.0))
                        .child(widgets::number_input(&y_range, 250.0)),
                ))
                .child(widgets::field(
                    "Frame and grid",
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(widgets::switch("frame", figure.style.frame, cx, |workspace| {
                            if let Some(figure) = workspace.session.studio.as_mut() {
                                figure.style.frame = !figure.style.frame;
                            }
                            workspace.touch();
                        }))
                        .child(widgets::switch("grid", figure.style.grid, cx, |workspace| {
                            if let Some(figure) = workspace.session.studio.as_mut() {
                                figure.style.grid = !figure.style.grid;
                                for panel in &mut figure.panels {
                                    panel.x.grid = figure.style.grid;
                                    panel.y.grid = figure.style.grid;
                                }
                            }
                            workspace.touch();
                        }))
                        .child(widgets::switch(
                            "x grid",
                            panel.x.grid,
                            cx,
                            |workspace| {
                                workspace.edit_axis(true, |axis| axis.grid = !axis.grid)
                            },
                        ))
                        .child(widgets::switch(
                            "y grid",
                            panel.y.grid,
                            cx,
                            |workspace| {
                                workspace.edit_axis(false, |axis| axis.grid = !axis.grid)
                            },
                        ))
                        .child(widgets::switch(
                            "mirror x",
                            panel.x.mirror,
                            cx,
                            |workspace| {
                                workspace.edit_axis(true, |axis| axis.mirror = !axis.mirror)
                            },
                        ))
                        .child(widgets::switch(
                            "reversed y",
                            panel.y.reversed,
                            cx,
                            |workspace| {
                                workspace.edit_axis(false, |axis| axis.reversed = !axis.reversed)
                            },
                        )),
                ))
                .into_any_element(),
            cx,
        ).into_any_element());

        // -- legend (§17.6)
        sections.push(self.inspector_section(
            "studio-legend", "Legend", false,
            widgets::field(
                "Legend",
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(widgets::switch(
                        "shown",
                        panel.legend.visible,
                        cx,
                        |workspace| {
                            if let Some(figure) = workspace.session.studio.as_mut() {
                                if let Some(panel) = figure.panel_mut(0) {
                                    panel.legend.visible = !panel.legend.visible;
                                }
                            }
                            workspace.touch();
                        },
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_1()
                            .children(studio::legend_positions().into_iter().map(
                                |(label, position)| {
                                    widgets::choice(
                                        label,
                                        panel.legend.position == position,
                                        cx,
                                        move |workspace| {
                                            if let Some(figure) =
                                                workspace.session.studio.as_mut()
                                            {
                                                if let Some(panel) = figure.panel_mut(0) {
                                                    panel.legend.position = position;
                                                }
                                            }
                                            workspace.touch();
                                        },
                                    )
                                    .into_any_element()
                                },
                            )),
                    )
                    .child(widgets::switch(
                        "border",
                        panel.legend.frame,
                        cx,
                        |workspace| {
                            if let Some(figure) = workspace.session.studio.as_mut() {
                                if let Some(panel) = figure.panel_mut(0) {
                                    panel.legend.frame = !panel.legend.frame;
                                }
                            }
                            workspace.touch();
                        },
                    )),
            )
            .into_any_element(),
            cx,
        ).into_any_element());

        // -- the contour's own controls (§17.9)
        if let Some(contour) = panel.contour.clone() {
            let min_state = self.field_state(
                FieldId::ContourMin,
                format!("{:.4}", contour.limits.min),
                "low",
                window,
                cx,
            );
            let max_state = self.field_state(
                FieldId::ContourMax,
                format!("{:.4}", contour.limits.max),
                "high",
                window,
                cx,
            );
            let levels_state = self.field_state(
                FieldId::ContourLevels,
                contour
                    .levels
                    .iter()
                    .map(|level| format!("{level:.4}"))
                    .collect::<Vec<String>>()
                    .join(", "),
                "auto",
                window,
                cx,
            );
            let colormaps = ["coolwarm", "viridis", "magma", "grayscale"];
            sections.push(self.inspector_section(
                "studio-contour", "Contour & colour scale", false,
                div()
                    .flex()
                    .flex_col()
                    .child(widgets::field(
                        "Colour limits",
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(widgets::number_input(&min_state, 96.0))
                            .child(widgets::readout("…"))
                            .child(widgets::number_input(&max_state, 96.0)),
                    ))
                    .child(widgets::field(
                        "Colormap",
                        div().flex().flex_wrap().gap_1().children(colormaps.map(|name| {
                            let active = contour.colormap.name == name;
                            widgets::choice(name, active, cx, move |workspace| {
                                if let Some(figure) = workspace.session.studio.as_mut() {
                                    if let Some(panel) = figure.panel_mut(0) {
                                        if let Some(contour) = panel.contour.as_mut() {
                                            if let Some(colormap) = ta_core::Colormap::preset(name) {
                                                contour.colormap = colormap;
                                            }
                                        }
                                    }
                                }
                                workspace.touch();
                            })
                            .into_any_element()
                        })),
                    ))
                    .child(widgets::field(
                        "Levels",
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(widgets::number_input(&levels_state, 250.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(widgets::small_caps("count"))
                                    .child(widgets::readout(format!("{}", contour.level_count)))
                                    .child(widgets::action_id(
                                        "more",
                                        ElementId::Name("levels-more".into()),
                                        false,
                                        cx,
                                        |workspace, _, _| {
                                            workspace.edit_contour(|contour| {
                                                contour.level_count =
                                                    (contour.level_count + 2).min(40);
                                                contour.levels.clear();
                                            })
                                        },
                                    ))
                                    .child(widgets::action_id(
                                        "fewer",
                                        ElementId::Name("levels-fewer".into()),
                                        false,
                                        cx,
                                        |workspace, _, _| {
                                            workspace.edit_contour(|contour| {
                                                contour.level_count =
                                                    contour.level_count.saturating_sub(2).max(2);
                                                contour.levels.clear();
                                            })
                                        },
                                    )),
                            ).child(widgets::switch(
                                "isolines",
                                contour.show_lines,
                                cx,
                                |workspace| {
                                    workspace.edit_contour(|contour| {
                                        contour.show_lines = !contour.show_lines
                                    })
                                },
                            )),
                    ))
                    .into_any_element(),
                cx,
            ).into_any_element());
        }

        // -- annotations (§17.7)
        let text_state = self.field_state(
            FieldId::AnnotationText,
            "",
            "annotation",
            window,
            cx,
        );
        let x_state = self.field_state(
            FieldId::AnnotationX,
            "",
            "wavelength",
            window,
            cx,
        );
        let y_state = self.field_state(FieldId::AnnotationY, "", "delay", window, cx);
        let annotations: Vec<AnyElement> = panel
            .annotations
            .iter()
            .enumerate()
            .map(|(index, annotation)| {
                let id = annotation.id;
                div()
                    .id(ElementId::Name(format!("annotation-{id}").into()))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .border_b_1()
                    .border_color(ink::hair())
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_size(px(11.0))
                            .text_color(ink::text())
                            .child(if annotation.text.trim().is_empty() {
                                format!("line at {}", annotation.x.unwrap_or_default())
                            } else {
                                annotation.text.clone()
                            }),
                    )
                    .child(widgets::readout(format!("{index}")))
                    .child(widgets::action_id(
                        "remove",
                        ElementId::Name(format!("annotation-remove-{id}").into()),
                        false,
                        cx,
                        move |workspace, _, _| {
                            if let Some(figure) = workspace.session.studio.as_mut() {
                                if let Some(panel) = figure.panel_mut(0) {
                                    panel.annotations.retain(|candidate| candidate.id != id);
                                }
                            }
                            workspace.touch();
                        },
                    ))
                    .into_any_element()
            })
            .collect();
        sections.push(self.inspector_section(
            "studio-annotations", "Annotations", false,
            div()
                .flex()
                .flex_col()
                .child(widgets::field(
                    "Annotations",
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(widgets::text_input(&text_state, 250.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(widgets::number_input(&x_state, 108.0))
                                .child(widgets::number_input(&y_state, 108.0)),
                        )
                        .child(widgets::action_id(
                            "Add text",
                            ElementId::Name("add-annotation".into()),
                            false,
                            cx,
                            |workspace, _window, cx| workspace.add_annotation(cx),
                        ))
                        .child(widgets::action_id(
                            "Mark the selection",
                            ElementId::Name("mark-selection".into()),
                            false,
                            cx,
                            |workspace, _, _| workspace.mark_selection(),
                        )),
                ))
                .child(div().flex().flex_col().children(annotations))
                .into_any_element(),
            cx,
        ).into_any_element());

        let canvas = sections.remove(0);
        let presets = sections.remove(0);
        let settings = sections.split_off(panel.series.len());
        let individual = div().flex().flex_col().children(sections);
        let lines = self.selected_line_controls(&panel, cx);
        div().flex().flex_col()
            .child(self.inspector_section("studio-lines", "Selected lines", true, lines, cx))
            .child(self.inspector_section("studio-individual", "Individual line settings", false, individual, cx))
            .children(settings)
            .child(canvas)
            .child(presets)
            .into_any_element()
    }

    /// The export row of §17.10 and §18.
    fn studio_export_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(figure) = self.studio_figure().cloned() else {
            return div().into_any_element();
        };
        let format = figure.export.format;
        let dpi = figure.export.dpi;
        let visible_only = self.studio_visible_only;
        div()
            .w_full()
            .flex()
            .flex_wrap()
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .bg(ink::ground())
            .border_t_1()
            .border_color(ink::rule())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(widgets::small_caps("format"))
                    .children(
                        [
                            ("SVG (default)", ExportFormat::Svg),
                            ("PNG", ExportFormat::Png),
                        ]
                        .into_iter()
                        .map(|(label, chosen)| {
                            widgets::choice(label, format == chosen, cx, move |workspace| {
                                if let Some(figure) = workspace.session.studio.as_mut() {
                                    figure.export.format = chosen;
                                }
                                workspace.touch();
                            })
                            .into_any_element()
                        }),
                    ),
            )
            .child(widgets::readout("PDF export not yet supported"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(widgets::small_caps("raster"))
                    .children([300u32, 600, 1200].into_iter().map(|chosen| {
                        widgets::choice(
                            match chosen {
                                300 => "300 dpi",
                                600 => "600 dpi",
                                _ => "1200 dpi",
                            },
                            dpi == chosen,
                            cx,
                            move |workspace| {
                                if let Some(figure) = workspace.session.studio.as_mut() {
                                    figure.export.dpi = chosen;
                                }
                                workspace.touch();
                            },
                        )
                        .into_any_element()
                    })),
            )
            .child(widgets::switch(
                "visible range only",
                visible_only,
                cx,
                |workspace| workspace.studio_visible_only = !workspace.studio_visible_only,
            ))
            .child(
                div()
                    .flex_1()
                    .min_w(px(320.0))
                    .flex()
                    .flex_wrap()
                    .justify_end()
                    .items_center()
                    .gap_4()
                    .child(widgets::action("Export figure", true, cx, |workspace, _, _| {
                        let format = workspace
                            .session
                            .studio
                            .as_ref()
                            .map(|figure| figure.export.format)
                            .unwrap_or(ExportFormat::Svg);
                        workspace.export_studio(format);
                    }))
                    .child(widgets::action("Export data", false, cx, |workspace, _, _| {
                        let visible = workspace.studio_visible_only;
                        workspace.export_studio_data(visible);
                    }))
                    .child(widgets::action("Reopen on plate", false, cx, |workspace, _, _| {
                        workspace.reload_studio();
                    })),
            )
            .into_any_element()
    }

    // -- the studio's editing operations ------------------------------------

    /// Changes only the requested property on the selected curves.
    pub fn edit_selected_series(&mut self, mut change: impl FnMut(&mut ta_core::PlotSeries)) {
        if let Some(panel) = self.session.studio.as_mut().and_then(|figure| figure.panel_mut(0)) {
            for series in &mut panel.series {
                if self.studio_selection.contains(&series.id) {
                    change(series);
                }
            }
        }
        self.touch();
    }

    /// Edits one series of the studio's first panel.
    pub fn edit_series(&mut self, id: ta_core::SeriesId, change: impl FnOnce(&mut ta_core::PlotSeries)) {
        if let Some(figure) = self.session.studio.as_mut() {
            if let Some(panel) = figure.panel_mut(0) {
                if let Some(series) = panel.series.iter_mut().find(|series| series.id == id) {
                    change(series);
                }
            }
        }
        self.touch();
    }

    pub fn edit_axis(&mut self, horizontal: bool, change: impl FnOnce(&mut ta_core::AxisModel)) {
        if let Some(figure) = self.session.studio.as_mut() {
            if let Some(panel) = figure.panel_mut(0) {
                let old_grid = if horizontal { panel.x.grid } else { panel.y.grid };
                change(if horizontal { &mut panel.x } else { &mut panel.y });
                let new_grid = if horizontal { panel.x.grid } else { panel.y.grid };
                if old_grid != new_grid {
                    figure.style.grid = panel.x.grid || panel.y.grid;
                }
            }
        }
        self.touch();
    }

    pub fn edit_contour(&mut self, change: impl FnOnce(&mut ta_core::ContourSpec)) {
        if let Some(figure) = self.session.studio.as_mut() {
            if let Some(panel) = figure.panel_mut(0) {
                if let Some(contour) = panel.contour.as_mut() {
                    change(contour);
                }
            }
        }
        self.touch();
    }

    /// Moves a series one place in paint order.
    pub fn move_series(&mut self, index: usize, delta: isize) {
        if let Some(figure) = self.session.studio.as_mut() {
            if let Some(panel) = figure.panel_mut(0) {
                let count = panel.series.len();
                if count > 1 {
                    let target = (index as isize + delta).clamp(0, count as isize - 1) as usize;
                    if target != index {
                        let series = panel.series.remove(index);
                        panel.series.insert(target, series);
                        for (z, series) in panel.series.iter_mut().enumerate() {
                            series.z = z as i32;
                        }
                    }
                }
            }
        }
        self.touch();
    }

    /// Adds the annotation the studio's fields describe.
    pub fn add_annotation(&mut self, cx: &mut Context<Self>) {
        let (text, x, y) = (
            self.fields.text(FieldId::AnnotationText, cx),
            self.fields.text(FieldId::AnnotationX, cx),
            self.fields.text(FieldId::AnnotationY, cx),
        );
        let Some(x) = widgets::parse_number(&x) else {
            self.session
                .status
                .problem("an annotation needs an x coordinate");
            return;
        };
        let y = widgets::parse_number(&y).unwrap_or(0.0);
        if let Some(figure) = self.session.studio.as_mut() {
            let font_size = figure.style.base_font_size;
            if let Some(panel) = figure.panel_mut(0) {
                panel.annotations.push(ta_core::AnnotationSpec {
                    id: ta_core::AnnotationId::new(),
                    kind: AnnotationKind::Text,
                    text,
                    x: Some(x),
                    y: Some(y),
                    width: None,
                    height: None,
                    axis: None,
                    color: ta_core::Rgba::rgb(16, 21, 38),
                    font_size,
                });
            }
        }
        self.touch();
        cx.notify();
    }

    /// Marks the current selection with a reference line on the figure, which
    /// is what a kinetic trace's paper figure usually carries.
    pub fn mark_selection(&mut self) {
        let on_delay = matches!(self.current_plate, crate::session::PlateId::Kinetics | crate::session::PlateId::Contour);
        let coordinate = if on_delay {
            self.session.spectrum.as_ref().map(|spectrum| spectrum.delay.coordinate)
        } else {
            self.session.trace.as_ref().map(|trace| trace.wavelength.coordinate)
        };
        let Some(value) = coordinate else {
            self.session.status.problem("nothing is selected to mark");
            return;
        };

        if let Some(figure) = self.session.studio.as_mut() {
            if let Some(panel) = figure.panel_mut(0) {
                let mut line = studio::reference_line(
                    if on_delay {
                        ta_core::AxisKind::Delay
                    } else {
                        ta_core::AxisKind::Wavelength
                    },
                    value,
                    &if on_delay {
                        format!("{value:.3} ps")
                    } else {
                        format!("{value:.2} nm")
                    },
                );
                if self.current_plate == crate::session::PlateId::Contour {
                    line.y = line.x.take();
                }
                panel.annotations.push(line);
            }
        }
        self.session
            .status
            .info("a reference line was added to the figure");
        self.touch();
    }
}

/// A plate that has nothing to draw yet, saying so rather than showing a box.
fn empty_plate(message: &str) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(11.5))
        .text_color(ink::muted())
        .child(message.to_string())
        .into_any_element()
}

/// Whether two axis scales are the same choice.
fn same_scale(current: AxisScale, candidate: AxisScale) -> bool {
    match (current, candidate) {
        (AxisScale::Linear, AxisScale::Linear) => true,
        (AxisScale::Logarithmic, AxisScale::Logarithmic) => true,
        (AxisScale::SymmetricLogarithmic { .. }, AxisScale::SymmetricLogarithmic { .. }) => true,
        (AxisScale::SplitLinearLog { .. }, AxisScale::SplitLinearLog { .. }) => true,
        _ => false,
    }
}

/// A caption's opening words, for a list that has one line per series.
fn short(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= 22 {
        return trimmed.to_string();
    }
    trimmed.chars().take(21).collect::<String>() + "…"
}

