use gpui_kit::*;

struct Shell;

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().child("Spectra Studio")
    }
}

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |_window, cx| cx.new(|_| Shell))
            .expect("failed to open window");
    });
}
