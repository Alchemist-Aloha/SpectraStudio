#![recursion_limit = "512"]

//! Spectra Studio.
//!
//! `session` owns every rule about what is open, what is selected and what each
//! plate shows; `plates` turns that into figure descriptions; `app` renders them
//! in the direction the surface brief records. That split is why the linking
//! between the contour and the linked plates is tested without a window.

mod app;
mod compare;
mod fit_panel;
mod plates;
mod resolve;
mod session;
mod studio;
mod views;
mod widgets;

use gpui_kit::*;

fn main() {
    // Arguments: paths to open, then `--layout <figure|compare|studio|fit>`,
    // `--plate <a|b|c|d>`, `--fit`, `--export`, `--figure <single|double>` and
    // `--zoom <percent>`. The flags exist so a state can be reached directly by
    // a script: driving a native window with a synthetic pointer is not possible
    // on every display server, and a screenshot check that cannot reach a state
    // checks nothing.
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let mut layout = app::Layout::Figure;
    // `--export` writes the figure and its numbers beside the input and then
    // leaves the studio open, so a scripted run leaves evidence behind.
    let mut plate = session::PlateId::Kinetics;
    let mut fit = false;
    let mut export = false;
    let mut figure_preset: Option<ta_core::JournalPreset> = None;
    let mut page_zoom: Option<f32> = None;
    let mut initial: Vec<String> = Vec::new();
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--layout" => {
                if let Some(value) = arguments.get(index + 1) {
                    layout = match value.as_str() {
                        "compare" => app::Layout::Compare,
                        "studio" => app::Layout::Studio,
                        "fit" => app::Layout::Fit,
                        _ => app::Layout::Figure,
                    };
                }
                index += 2;
            }
            "--plate" => {
                if let Some(value) = arguments.get(index + 1) {
                    plate = match value.as_str() {
                        "a" => session::PlateId::Contour,
                        "b" => session::PlateId::Kinetics,
                        "c" => session::PlateId::Spectra,
                        _ => session::PlateId::TimeZero,
                    };
                }
                index += 2;
            }
            "--fit" => {
                fit = true;
                index += 1;
            }
            "--export" => {
                export = true;
                index += 1;
            }
            "--zoom" => {
                page_zoom = arguments
                    .get(index + 1)
                    .and_then(|value| value.parse::<f32>().ok())
                    .map(|percent| percent / 100.0);
                index += 2;
            }
            "--figure" => {
                figure_preset = match arguments.get(index + 1).map(String::as_str) {
                    Some("double") => Some(ta_core::JournalPreset::JacsDoubleColumn),
                    Some("single") => Some(ta_core::JournalPreset::JacsSingleColumn),
                    _ => None,
                };
                index += 2;
            }
            other => {
                initial.push(other.to_string());
                index += 1;
            }
        }
    }
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        app::Workspace::bind_shortcuts(cx);
        let initial = initial.clone();
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(60.0), px(40.0)),
                    size(px(1440.0), px(900.0)),
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Spectra Studio".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            cx,
            move |_window, cx| {
                let mut workspace = app::Workspace::new(session::Session::new());
                // A path on the command line opens straight away, which is how
                // the fixture is loaded in development.
                for argument in &initial {
                    workspace.open_path(std::path::Path::new(argument));
                }
                workspace.current_plate = plate;
                if layout == app::Layout::Studio {
                    workspace.open_studio(plate);
                } else {
                    workspace.layout = layout;
                }
                if let Some(preset) = figure_preset {
                    // A scripted check of the preview at another paper size,
                    // which is otherwise one click away in the panel.
                    if workspace.session.studio.is_none() {
                        workspace.open_studio(plate);
                    }
                    if let Some(figure) = workspace.session.studio.as_mut() {
                        figure.apply_preset(preset);
                    }
                    workspace.layout = app::Layout::Studio;
                    workspace.touch();
                }
                if layout == app::Layout::Compare {
                    workspace.seed_comparison();
                }
                if let Some(zoom) = page_zoom {
                    // A scripted check of the page zoom, which is otherwise one
                    // wheel notch or one button away.
                    if workspace.session.studio.is_none() {
                        workspace.open_studio(plate);
                    }
                    workspace.layout = app::Layout::Studio;
                    workspace.pending_page_zoom = Some(zoom);
                    workspace.touch();
                }
                if export {
                    // A scripted check of the export path: the same actions the
                    // export bar runs, without a pointer to click them with.
                    workspace.open_studio(plate);
                    let format = workspace
                        .session
                        .studio
                        .as_ref()
                        .map(|figure| figure.export.format)
                        .unwrap_or(app::ExportFormat::Svg);
                    workspace.export_studio(format);
                    workspace.export_studio_data(true);
                    workspace.export_glotaran();
                    workspace.layout = app::Layout::Studio;
                }
                if fit {
                    workspace.open_fit();
                    match workspace.fit_once() {
                        Ok(result) => {
                            let message = result.stats.message.clone();
                            workspace
                                .session
                                .status
                                .info(format!("startup fit: {message}"));
                        }
                        Err(error) => workspace
                            .session
                            .status
                            .problem(format!("the startup fit failed: {error}")),
                    }
                    workspace.touch();
                }
                cx.new(|_| workspace)
            },
        )
        .expect("failed to open the window");
    });
}
