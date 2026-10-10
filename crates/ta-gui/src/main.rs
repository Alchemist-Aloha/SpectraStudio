#![recursion_limit = "512"]

//! Spectra Studio.
//!
//! `session` owns every rule about what is open, what is selected and what each
//! plate shows; `plates` turns that into figure descriptions; `app` renders them
//! in the direction the surface brief records. That split is why the linking
//! between the contour and the linked plates is tested without a window.

mod app;
mod plates;
mod session;

use gpui_kit::*;

fn main() {
    let initial: Vec<String> = std::env::args().skip(1).collect();
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
                cx.new(|_| workspace)
            },
        )
        .expect("failed to open the window");
    });
}
