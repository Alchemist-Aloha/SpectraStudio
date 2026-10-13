//! The shell's small vocabulary: labels, read-outs, chips, and the one place a
//! number is typed.
//!
//! Every editable number in the application is an [`InputState`] registered
//! here under a [`FieldId`], and every one of them reports through the same
//! handler, committing on Enter or focus loss, never on a keystroke.
//! One subscription path means typing in a parameter table, a figure
//! size or a wavelength cannot behave differently from each other, and it is why
//! a test can drive the whole interface without a window.

use crate::app::{ink, Workspace};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, px, App, AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement,
    ParentElement, SharedString, Styled, Window,
};
use std::collections::HashMap;

/// Which editable number a field is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldId {
    /// Wavelengths for the linked trace and the overlays.
    Wavelength,
    /// Delays for the linked spectrum and the overlays.
    Delay,
    /// Averaging half-width in nm.
    HalfWidth,
    /// Averaging channel count.
    Channels,
    /// A comparison series' label, scale, offset and shift.
    SeriesLabel(usize),
    SeriesScale(usize),
    SeriesOffset(usize),
    SeriesShift(usize),
    /// A fit parameter's starting value and bounds.
    FitValue(usize),
    FitBounds(usize),
    /// The fitted delay range, written as `low high`.
    FitRange,
    /// Excluded delay intervals, written as `a b, c d`.
    FitExcluded,
    /// The reference delay and signal floor of the automatic estimate.
    FitReferenceDelay,
    /// Figure size and typography.
    FigureWidth,
    FigureHeight,
    FigureFontSize,
    AxisXLabel,
    AxisYLabel,
    /// An axis' explicit limits, written `low high`.
    AxisXRange,
    AxisYRange,
    FigureTickSize,
    LegendFontSize,
    /// Contour colour limits, written `low high`.
    ContourMin,
    ContourMax,
    /// The explicit contour levels, comma separated.
    ContourLevels,
    /// A new annotation's text, position and size.
    AnnotationText,
    AnnotationX,
    AnnotationY,
    LegendLabel(usize),
}

/// The registered fields.
#[derive(Default)]
pub struct Fields {
    map: HashMap<FieldId, Entity<InputState>>,
    committed: HashMap<FieldId, String>,
}

impl Fields {
    pub fn get(&self, id: FieldId) -> Option<&Entity<InputState>> {
        self.map.get(&id)
    }

    pub fn text(&self, id: FieldId, cx: &App) -> String {
        self.map
            .get(&id)
            .map(|state| state.read(cx).value().to_string())
            .unwrap_or_default()
    }

    /// Sets a field's text from the application, so a click on a plate shows up
    /// in the number the user would otherwise type.
    pub fn set(
        &mut self,
        id: FieldId,
        text: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let text = text.into();
        self.committed.insert(id, text.to_string());
        if self.holds(id, text.as_ref(), cx) {
            return;
        }
        if let Some(state) = self.map.get(&id) {
            state.update(cx, |state, cx| state.set_value(text, window, cx));
        }
    }

    /// Whether the field holds this text already, so a programmatic update does
    /// not fight the caret.
    pub fn holds(&self, id: FieldId, text: &str, cx: &App) -> bool {
        self.map
            .get(&id)
            .map(|state| state.read(cx).value().as_ref() == text)
            .unwrap_or(false)
    }

    fn should_commit(&mut self, id: FieldId, event: &InputEvent, text: &str) -> bool {
        if !matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur)
            || self.committed.get(&id).is_some_and(|value| value == text)
        {
            return false;
        }
        // Enter followed by blur must not apply the same edit twice.
        self.committed.insert(id, text.to_string());
        true
    }

    fn insert(&mut self, id: FieldId, state: Entity<InputState>) -> Entity<InputState> {
        self.map.insert(id, state.clone());
        state
    }
}

impl Workspace {
    /// A native disclosure that keeps its state when the inspector re-renders.
    pub fn inspector_section(
        &self,
        key: impl Into<String>,
        title: impl Into<SharedString>,
        default_open: bool,
        content: impl IntoElement,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use gpui_kit::component::accordion::Accordion;
        let key = key.into();
        let open = self
            .inspector_sections
            .get(&key)
            .copied()
            .unwrap_or(default_open);
        Accordion::new(gpui_kit::ElementId::Name(key.clone().into()))
            .bordered(false)
            .small()
            .h_auto()
            .item(|item| {
                item.title(title.into())
                    .open(open)
                    .bg(ink::ground())
                    .title_style(gpui_kit::StyleRefinement::default().text_size(px(11.5)))
                    .content_style(gpui_kit::StyleRefinement::default().px_0())
                    .hover(|style| style.text_color(ink::accent()))
                    .child(content)
            })
            .on_toggle_click(cx.listener(move |workspace, indices: &[usize], _, cx| {
                workspace
                    .inspector_sections
                    .insert(key.clone(), !indices.is_empty());
                cx.notify();
            }))
    }
    /// The field for `id`, created and subscribed the first time it is asked
    /// for, then reused so the caret and the text survive a re-render.
    pub fn field_state(
        &mut self,
        id: FieldId,
        value: impl Into<String>,
        placeholder: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(state) = self.fields.get(id) {
            return state.clone();
        }
        let value: String = value.into();
        self.fields.committed.insert(id, value.clone());
        let state = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        if !value.is_empty() {
            state.update(cx, |state, cx| state.set_value(value, window, cx));
        }
        cx.subscribe_in(
            &state,
            window,
            move |workspace: &mut Workspace, _state, event: &InputEvent, _window, cx| {
                let text = workspace.fields.text(id, cx);
                if workspace.fields.should_commit(id, event, &text) {
                    workspace.on_field_change(id, &text, cx);
                }
            },
        )
        .detach();
        self.fields.insert(id, state)
    }

    /// Puts a coordinate field back in step with the selection, which is what
    /// a click on a plate is: typing and pointing must not disagree.
    pub fn sync_coordinate_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let linked = self.session.trace.as_ref().map(|trace| trace.wavelength.coordinate);
        let wavelengths = self.session.comparison_wavelengths.iter().copied()
            .filter(|value| Some(*value) != linked)
            .chain(linked)
            .map(|value| format!("{value:.4}"))
            .collect::<Vec<_>>()
            .join(", ");
        let delays = if self.session.comparison_delays.is_empty() {
            self.session
                .spectrum
                .as_ref()
                .map(|spectrum| format!("{:.4}", spectrum.delay.coordinate))
                .unwrap_or_default()
        } else {
            self.session
                .comparison_delays
                .iter()
                .map(|value| format!("{value:.4}"))
                .collect::<Vec<String>>()
                .join(", ")
        };
        // Re-renders must preserve a draft; only a changed selection replaces it.
        if !wavelengths.is_empty()
            && self.fields.committed.get(&FieldId::Wavelength) != Some(&wavelengths)
        {
            self.fields
                .set(FieldId::Wavelength, wavelengths, window, cx);
        }
        if !delays.is_empty()
            && self.fields.committed.get(&FieldId::Delay) != Some(&delays)
        {
            self.fields.set(FieldId::Delay, delays, window, cx);
        }
        let _ = cx;
    }
}

/// One line of a panel: a small-caps label over its value.
pub fn field(label: &str, value: impl IntoElement) -> impl IntoElement {
    div()
        .id(gpui_kit::ElementId::Name(format!("field-{label}").into()))
        .flex()
        .flex_col()
        .gap_1()
        .px_3()
        .py_2()
        .border_b_1()
        .border_color(ink::hair())
        .child(small_caps(label))
        .child(value)
}

/// The small-caps label that names a field.
pub fn small_caps(label: &str) -> impl IntoElement {
    div()
        .text_size(px(10.0))
        .text_color(ink::muted())
        .child(label.to_uppercase())
}

/// A section label inside a scrolling column.
pub fn small_caps_in_panel(label: &str) -> impl IntoElement {
    div()
        .px_3()
        .py_2()
        .text_size(px(10.0))
        .text_color(ink::muted())
        .border_b_1()
        .border_color(ink::rule())
        .child(label.to_uppercase())
}

/// A read-out: a number the user cannot edit, in the muted ink.
pub fn readout(text: impl Into<String>) -> impl IntoElement {
    div().text_size(px(11.5)).text_color(ink::muted()).child(text.into())
}

/// A value that is the point of the panel: darker and slightly larger.
pub fn value_text(text: impl Into<String>) -> impl IntoElement {
    div().text_size(px(12.5)).text_color(ink::text()).child(text.into())
}

pub fn em(text: impl Into<String>) -> impl IntoElement {
    div()
        .text_size(px(11.5))
        .text_color(ink::text())
        .font_weight(FontWeight::SEMIBOLD)
        .child(text.into())
}

/// A label on a hairline rule, the shell's one control shape.
pub fn action_id(
    label: &'static str,
    id: gpui_kit::ElementId,
    accent: bool,
    cx: &mut Context<Workspace>,
    run: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
) -> gpui_kit::base::Button {
    let color = if accent { ink::accent() } else { ink::text() };
    let border = if accent { ink::accent() } else { ink::rule() };
    let run = std::rc::Rc::new(run);
    gpui_kit::base::Button::new(id)
        .accessibility_label(label)
        .styles(|styles| styles.disabled(|style| style.text_color(ink::muted()).border_color(ink::hair())))
        .flex_shrink_0()
        .cursor_pointer()
        .h(px(24.0))
        .px_1()
        .text_size(px(11.5))
        .text_color(color)
        .border_b_1()
        .border_color(border)
        .pb(px(1.0))
        .hover(move |style| style.text_color(ink::accent()).bg(ink::ground()))
        .focus(|style| style.border_b_2().border_color(ink::accent()).bg(ink::ground()))
        .child(label)
        .on_click(cx.listener(move |workspace, _event, window, cx| {
            run(workspace, window, cx);
            workspace.touch();
            workspace.sync_studio_fields(window, cx);
            cx.notify();
        }))
}

/// The same control, named by its label.
pub fn action(
    label: &'static str,
    accent: bool,
    cx: &mut Context<Workspace>,
    run: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
) -> impl IntoElement {
    action_id(label, gpui_kit::ElementId::Name(label.into()), accent, cx, run)
}

/// A control that shows which of a set is current: a filled chip when it is.
pub fn choice(
    label: &'static str,
    active: bool,
    cx: &mut Context<Workspace>,
    run: impl Fn(&mut Workspace) + 'static,
) -> gpui_kit::base::Button {
    let run = std::rc::Rc::new(run);
    gpui_kit::base::Button::new(gpui_kit::ElementId::Name(label.into()))
        .accessibility_label(label)
        .selected(active)
        .styles(|styles| styles.disabled(|style| style.text_color(ink::muted()).bg(ink::ground()).border_color(ink::hair())))
        .cursor_pointer()
        .flex_shrink_0()
        .min_h(px(28.0))
        .px_2()
        .py_1()
        .text_size(px(11.0))
        .text_color(if active { ink::paper() } else { ink::text() })
        .bg(if active { ink::accent() } else { ink::paper() })
        .border_1()
        .border_color(if active { ink::accent() } else { ink::rule() })
        .hover(|style| style.border_color(ink::accent()))
        .focus(|style| style.border_2().border_color(ink::accent()))
        .child(label)
        .on_click(cx.listener(move |workspace, _event, window, cx| {
            run(workspace);
            workspace.touch();
            workspace.sync_studio_fields(window, cx);
            cx.notify();
        }))
}

/// A checkbox tied to a `bool` the session holds.
pub fn switch(
    label: &'static str,
    checked: bool,
    cx: &mut Context<Workspace>,
    run: impl Fn(&mut Workspace) + 'static,
) -> impl IntoElement {
    let run = std::rc::Rc::new(run);
    Checkbox::new(gpui_kit::ElementId::Name(label.into()))
        .label(label)
        .checked(checked)
        .small()
        .text_size(px(11.5))
        .on_click(cx.listener(move |workspace, _checked: &bool, _window, cx| {
            run(workspace);
            workspace.touch();
            cx.notify();
        }))
}

/// A compact editable field with a hairline border and native focus feedback.
pub fn number_input(state: &Entity<InputState>, width: f32) -> impl IntoElement {
    div().w(px(width)).child(
        Input::new(state)
            .appearance(true)
            .bordered(true)
            .small()
            .text_size(px(11.5))
            .border_color(ink::rule())
            .text_color(ink::text()),
    )
}

/// A wider field, for a label or a list of coordinates.
pub fn text_input(state: &Entity<InputState>, width: f32) -> impl IntoElement {
    number_input(state, width)
}

// -- parsing ----------------------------------------------------------------

/// One number, or `None` when the text is not one.
pub fn parse_number(text: &str) -> Option<f64> {
    let trimmed = text.trim().replace(',', ".");
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse::<f64>().ok().filter(|value| value.is_finite())
}

/// A list of numbers, separated by commas or spaces.
pub fn parse_numbers(text: &str) -> Vec<f64> {
    text.split([',', ' ', ';', '\t'])
        .filter_map(parse_number)
        .collect()
}

/// A pair written `low high` or `low..high`, and whether it was written at all.
pub fn parse_range(text: &str) -> Option<(f64, f64)> {
    let numbers = parse_numbers(text);
    match numbers.len() {
        0 => None,
        _ => {
            let parts: Vec<&str> = text
                .split([',', ';'])
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .collect();
            let low = numbers[0];
            let high = if numbers.len() > 1 {
                numbers[1]
            } else {
                // A single number is a half-open interval that reaches as far
                // as the axis does.
                let _ = parts;
                return Some((low, f64::INFINITY));
            };
            Some((low.min(high), low.max(high)))
        }
    }
}

/// Bounds written as `low .. high`, either side optional.
pub fn parse_bounds(text: &str) -> (Option<f64>, Option<f64>) {
    let cleaned = text.replace("..", " ");
    let numbers = parse_numbers(&cleaned);
    match numbers.len() {
        0 => (None, None),
        1 => (Some(numbers[0]), None),
        _ => (Some(numbers[0]), Some(numbers[1])),
    }
}

/// Pairs written `a b, c d` — the excluded intervals.
pub fn parse_intervals(text: &str) -> Vec<(f64, f64)> {
    text.split([',', ';', '\n'])
        .flat_map(|chunk| {
            let numbers = parse_numbers(chunk);
            if numbers.len() >= 2 {
                vec![(numbers[0].min(numbers[1]), numbers[0].max(numbers[1]))]
            } else {
                Vec::new()
            }
        })
        .collect()
}

/// The text a range is shown as.
pub fn format_range(range: Option<(f64, f64)>) -> String {
    match range {
        Some((low, high)) if high.is_finite() => format!("{low} {high}"),
        Some((low, _)) => format!("{low}"),
        None => String::new(),
    }
}

pub fn format_intervals(intervals: &[(f64, f64)]) -> String {
    intervals
        .iter()
        .map(|(low, high)| format!("{low:.4} {high:.4}"))
        .collect::<Vec<String>>()
        .join(", ")
}

pub fn format_bounds(low: Option<f64>, high: Option<f64>) -> String {
    match (low, high) {
        (Some(low), Some(high)) => format!("{} .. {}", compact(low), compact(high)),
        (Some(low), None) => compact(low),
        (None, Some(high)) => format!(".. {}", compact(high)),
        (None, None) => String::new(),
    }
}

/// A parameter bound or a starting value, written with the digits it needs and
/// no more: a column of thirteen of them has to stay readable.
pub fn compact(value: f64) -> String {
    if !value.is_finite() {
        return "—".to_string();
    }
    if value == value.trunc() && value.abs() < 1e15 {
        return format!("{value:.0}");
    }
    let magnitude = value.abs();
    let digits = if magnitude >= 100.0 {
        1
    } else if magnitude >= 1.0 {
        2
    } else if magnitude >= 0.01 {
        4
    } else {
        6
    };
    let text = format!("{value:.digits$}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_stays_local_until_enter_or_blur() {
        for id in [FieldId::Wavelength, FieldId::FitValue(0), FieldId::AxisXLabel] {
            let mut fields = Fields::default();
            fields.committed.insert(id, "1".into());
            assert!(!fields.should_commit(id, &InputEvent::Focus, "1"));
            assert!(!fields.should_commit(id, &InputEvent::Change, ""));
            assert!(!fields.should_commit(id, &InputEvent::Change, "2"));
            assert_eq!(fields.committed[&id], "1");
            assert!(fields.should_commit(
                id,
                &InputEvent::PressEnter { secondary: false, shift: false },
                "2",
            ));
            assert!(!fields.should_commit(id, &InputEvent::Blur, "2"));
            assert!(!fields.should_commit(id, &InputEvent::Change, "3"));
            assert!(fields.should_commit(id, &InputEvent::Blur, "3"));
            assert!(!fields.should_commit(id, &InputEvent::Blur, "3"));
            assert!(fields.should_commit(id, &InputEvent::Blur, ""));
        }
    }

    #[test]
    fn programmatic_field_refresh_is_not_a_rounded_user_edit() {
        let mut fields = Fields::default();
        let id = FieldId::AxisXRange;
        fields.committed.insert(id, "0.0000 1.0000".into());
        assert!(!fields.should_commit(id, &InputEvent::Change, "0.0000 1.0000"));
        assert!(!fields.should_commit(id, &InputEvent::Blur, "0.0000 1.0000"));
        assert!(!fields.should_commit(
            id,
            &InputEvent::PressEnter { secondary: false, shift: false },
            "0.0000 1.0000",
        ));
        assert!(fields.should_commit(id, &InputEvent::Blur, "0.0010 1.0000"));
    }


    #[test]
    fn a_number_may_be_written_with_a_comma() {
        assert_eq!(parse_number(" 1,5 "), Some(1.5));
        assert_eq!(parse_number("-0.25"), Some(-0.25));
        assert_eq!(parse_number("  "), None);
        assert_eq!(parse_number("nan"), None);
    }

    #[test]
    fn coordinate_lists_take_commas_or_spaces() {
        assert_eq!(parse_numbers("500, 520 540"), vec![500.0, 520.0, 540.0]);
        assert!(parse_numbers("").is_empty());
    }

    #[test]
    fn an_excluded_interval_is_written_as_two_numbers() {
        assert_eq!(parse_intervals("0 1, 20 30"), vec![(0.0, 1.0), (20.0, 30.0)]);
        // A pair out of order is still an interval.
        assert_eq!(parse_intervals("5 2"), vec![(2.0, 5.0)]);
        assert!(parse_intervals("3").is_empty());
    }

    #[test]
    fn a_bound_is_written_with_the_digits_it_needs() {
        assert_eq!(compact(5000.0), "5000");
        assert_eq!(compact(0.01), "0.01");
        assert_eq!(compact(-0.5), "-0.5");
        assert_eq!(compact(0.1234567), "0.1235");
        assert_eq!(compact(0.0001234), "0.000123");
        assert_eq!(compact(12.3456), "12.35");
        assert_eq!(format_bounds(Some(0.01), Some(5000.0)), "0.01 .. 5000");
        assert_eq!(compact(f64::NAN), "—");
    }

    #[test]
    fn bounds_may_be_half_written() {
        assert_eq!(parse_bounds("-1 .. 1"), (Some(-1.0), Some(1.0)));
        assert_eq!(parse_bounds("0"), (Some(0.0), None));
        assert_eq!(parse_bounds(""), (None, None));
    }

    #[test]
    fn a_range_round_trips_through_its_text() {
        assert_eq!(format_range(Some((-1.0, 10.0))), "-1 10");
        let precise = (0.000000013, 1.23456789);
        assert_eq!(parse_range(&format_range(Some(precise))), Some(precise));
        assert_eq!(format_range(None), "");
        assert_eq!(parse_range("-1 10"), Some((-1.0, 10.0)));
        assert_eq!(format_intervals(&[(0.0, 1.0)]), "0.0000 1.0000");
    }
}
