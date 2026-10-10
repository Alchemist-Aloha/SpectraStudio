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
    // `--plate <a|b|c|d>` and `--fit`. The flags exist so a layout can be
    // opened directly by a script: driving a native window with a synthetic
    // pointer is not possible on every display server, and a screenshot check
    // that cannot reach a layout checks nothing.
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let mut layout = app::Layout::Figure;
    // `--export` writes the figure and its numbers beside the input and then
    // leaves the studio open, so a scripted run leaves evidence behind.
    let mut plate = session::PlateId::Kinetics;
    let mut fit = false;
    let mut export = false;
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
            other => {
                initial.push(other.to_string());
                index += 1;
            }
        }
    }
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
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
                if layout == app::Layout::Compare {
                    workspace.seed_comparison();
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
