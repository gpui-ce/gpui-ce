use gpui::{
    AppContext as _, Bounds, Context, HeadlessAppContext, HighlightStyle, Hsla, IntoElement,
    Pixels, PlatformTextSystem, Point, Render, ScaledPixels, Styled as _, StyledText, Window,
    WindowHandle, div, hsla, prelude::*, px, size,
};
use gpui_ce_parley::{ParleyTextSystem, SystemFonts};
use std::{borrow::Cow, sync::Arc};

const IBM_PLEX: &[u8] =
    include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf");
const SCALE_FACTOR: f32 = 1.5;

const INLINE: &str = "inline-translation-root";
const FIRST_BOX: &str = "inline-translation-first-box";
const SECOND_BOX: &str = "inline-translation-second-box";
const NESTED: &str = "inline-translation-nested";

fn first_group_color() -> Hsla {
    hsla(0.02, 0.75, 0.55, 1.0)
}

fn second_group_color() -> Hsla {
    hsla(0.58, 0.75, 0.55, 1.0)
}

fn highlighted(text: &'static str, color: Hsla) -> StyledText {
    StyledText::new(text).with_highlights([(
        0..text.len(),
        HighlightStyle {
            background_color: Some(color),
            ..Default::default()
        },
    )])
}

struct InlineTranslationView {
    width_offset: f32,
}

impl Render for InlineTranslationView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().pl(px(11.)).pt(px(9.)).child(
            div()
                .inline()
                .w(px(160. + self.width_offset))
                .p(px(7.))
                .border_1()
                .text_size(px(17.))
                .line_height(px(23.))
                .text_center()
                .debug_selector(|| INLINE.into())
                .child(highlighted("short line ", first_group_color()))
                .child(
                    div()
                        .w(px(13.))
                        .h(px(16.))
                        .align_middle()
                        .debug_selector(|| FIRST_BOX.into()),
                )
                .child("\n")
                .child(highlighted("secondline ", second_group_color()))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(29.))
                        .h(px(16.))
                        .align_middle()
                        .debug_selector(|| SECOND_BOX.into())
                        .child(div().w(px(5.)).h(px(7.)).debug_selector(|| NESTED.into())),
                ),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Geometry {
    inline: Bounds<Pixels>,
    text_groups: [Bounds<Pixels>; 2],
    first_box: Bounds<Pixels>,
    second_box: Bounds<Pixels>,
    nested: Bounds<Pixels>,
}

impl Geometry {
    fn read(cx: &mut HeadlessAppContext, window: WindowHandle<InlineTranslationView>) -> Self {
        let any_window = window.into();
        let mut debug_bounds = |selector| {
            cx.debug_bounds(any_window, selector)
                .unwrap()
                .unwrap_or_else(|| panic!("missing debug bounds for {selector}"))
        };

        let inline = debug_bounds(INLINE);
        let first_box = debug_bounds(FIRST_BOX);
        let second_box = debug_bounds(SECOND_BOX);
        let nested = debug_bounds(NESTED);
        let text_groups = [
            text_group_bounds(cx, any_window, first_group_color()),
            text_group_bounds(cx, any_window, second_group_color()),
        ];

        Self {
            inline,
            text_groups,
            first_box,
            second_box,
            nested,
        }
    }

    fn participants(self) -> [Bounds<Pixels>; 5] {
        [
            self.text_groups[0],
            self.text_groups[1],
            self.first_box,
            self.second_box,
            self.nested,
        ]
    }

    fn nested_offset(self) -> Point<Pixels> {
        self.nested.origin - self.second_box.origin
    }

    fn assert_valid(self) {
        let epsilon = px(1.);
        assert!(self.text_groups[0].right() <= self.first_box.origin.x + epsilon);
        assert!(self.text_groups[1].right() <= self.second_box.origin.x + epsilon);
        assert!(self.text_groups[0].origin.y < self.text_groups[1].origin.y);

        for participant in self.participants() {
            assert!(participant.origin.x >= self.inline.origin.x);
            assert!(participant.origin.y >= self.inline.origin.y);
            assert!(participant.right() <= self.inline.right());
            assert!(participant.bottom() <= self.inline.bottom());
        }

        assert!(self.nested.origin.x >= self.second_box.origin.x);
        assert!(self.nested.origin.y >= self.second_box.origin.y);
        assert!(self.nested.right() <= self.second_box.right());
        assert!(self.nested.bottom() <= self.second_box.bottom());
    }

    fn assert_moved_as_one_group(self, before: Self, step: usize) {
        let expected_delta = self.text_groups[0].origin - before.text_groups[0].origin;
        for (participant, (actual, previous)) in self
            .participants()
            .into_iter()
            .zip(before.participants())
            .enumerate()
        {
            assert_point_close(
                actual.origin - previous.origin,
                expected_delta,
                &format!("participant {participant} at resize step {step}"),
            );
        }
    }
}

fn text_group_bounds(
    cx: &mut HeadlessAppContext,
    window: gpui::AnyWindowHandle,
    color: Hsla,
) -> Bounds<Pixels> {
    let bounds = cx.solid_quad_bounds(window, color).unwrap();
    assert_eq!(bounds.len(), 1, "expected one rendered quad for {color:?}");
    logical_bounds(bounds[0])
}

fn logical_bounds(bounds: Bounds<ScaledPixels>) -> Bounds<Pixels> {
    bounds.map(|value| px(value.as_f32() / SCALE_FACTOR))
}

fn assert_close(actual: Pixels, expected: Pixels, context: &str) {
    assert!(
        (actual - expected).abs() < px(0.01),
        "{context}: expected {actual:?} to equal {expected:?}"
    );
}

fn assert_point_close(actual: Point<Pixels>, expected: Point<Pixels>, context: &str) {
    assert_close(actual.x, expected.x, context);
    assert_close(actual.y, expected.y, context);
}

#[test]
fn centered_inline_lines_move_as_one_group_during_fractional_resize() {
    let text_system = ParleyTextSystem::new_with_system_font(SystemFonts::Skip, "IBM Plex Sans");
    text_system
        .add_fonts(vec![Cow::Borrowed(IBM_PLEX)])
        .unwrap();
    let mut cx = HeadlessAppContext::new(Arc::new(text_system));
    let window = cx
        .open_window(size(px(340.), px(180.)), |window, cx| {
            window.set_scale_factor(SCALE_FACTOR);
            cx.new(|_| InlineTranslationView { width_offset: 0. })
        })
        .unwrap();

    cx.run_until_parked();
    let initial = Geometry::read(&mut cx, window);
    initial.assert_valid();

    for step in 1..=64 {
        window
            .update(&mut cx, |view, _, cx| {
                view.width_offset = step as f32 / 16.;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();

        let translated = Geometry::read(&mut cx, window);
        translated.assert_valid();
        translated.assert_moved_as_one_group(initial, step);
        assert_point_close(
            translated.nested_offset(),
            initial.nested_offset(),
            &format!("nested offset at resize step {step}"),
        );
    }

    window
        .update(&mut cx, |view, _, cx| {
            view.width_offset = 0.;
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(Geometry::read(&mut cx, window), initial);
}
