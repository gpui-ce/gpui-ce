use gpui::{
    App, Bounds, Context, Font, FontStyle, FontWeight, Render, Window, WindowBounds, WindowOptions,
    div, font, prelude::*, px, rgb, size,
};
use std::borrow::Cow;

const IBM_PLEX_REGULAR: &[u8] =
    include_bytes!("../../../../assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf");
const IBM_PLEX_ITALIC: &[u8] =
    include_bytes!("../../../../assets/fonts/ibm-plex-sans/IBMPlexSans-Italic.ttf");
const IBM_PLEX_SEMIBOLD_ITALIC: &[u8] =
    include_bytes!("../../../../assets/fonts/ibm-plex-sans/IBMPlexSans-SemiBoldItalic.ttf");
const LILEX_REGULAR: &[u8] = include_bytes!("../../../../assets/fonts/lilex/Lilex-Regular.ttf");

#[derive(Clone)]
struct Check {
    name: &'static str,
    passed: bool,
    detail: String,
}

impl Check {
    fn new(name: &'static str, passed: bool, detail: impl Into<String>) -> Self {
        Self {
            name,
            passed,
            detail: detail.into(),
        }
    }
}

struct FontReport {
    checks: Vec<Check>,
}

impl Render for FontReport {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let passed = self.checks.iter().filter(|check| check.passed).count();
        let total = self.checks.len();

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .p_6()
            .bg(rgb(0x111318))
            .text_color(rgb(0xe6e8ee))
            .child(
                div()
                    .text_2xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Font backend diagnostics"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x9ca3af))
                    .child(format!("{passed} of {total} checks passed")),
            )
            .children(self.checks.iter().map(|check| {
                let status_color = if check.passed {
                    rgb(0x4ade80)
                } else {
                    rgb(0xf87171)
                };
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_3()
                    .rounded_md()
                    .bg(rgb(0x1b1f27))
                    .border_1()
                    .border_color(rgb(0x303641))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                div()
                                    .text_color(status_color)
                                    .font_weight(FontWeight::BOLD)
                                    .child(if check.passed { "PASS" } else { "FAIL" }),
                            )
                            .child(check.name),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0xaeb4bf))
                            .child(check.detail.clone()),
                    )
            }))
            .child(
                div()
                    .mt_2()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .font_family("IBM Plex Sans")
                            .child("IBM Plex Sans regular: Hamburgefonts 0123456789"),
                    )
                    .child(
                        div()
                            .font_family("IBM Plex Sans")
                            .italic()
                            .child("IBM Plex Sans italic: Hamburgefonts 0123456789"),
                    )
                    .child(
                        div()
                            .font_family("IBM Plex Sans")
                            .font_weight(FontWeight::SEMIBOLD)
                            .italic()
                            .child("IBM Plex Sans semibold italic: Hamburgefonts 0123456789"),
                    )
                    .child(
                        div()
                            .font_family("Lilex")
                            .child("Lilex regular: fn main() { println!(\"hello\"); }"),
                    ),
            )
    }
}

fn selected_font(family: &str, weight: FontWeight, style: FontStyle) -> Font {
    let mut selected = font(family);
    selected.weight = weight;
    selected.style = style;
    selected
}

fn main() {
    gpui_platform::application().run(move |cx: &mut App| {
        let text_system = cx.text_system();
        text_system
            .add_fonts(vec![
                Cow::Borrowed(IBM_PLEX_REGULAR),
                Cow::Borrowed(IBM_PLEX_ITALIC),
                Cow::Borrowed(IBM_PLEX_SEMIBOLD_ITALIC),
                Cow::Borrowed(LILEX_REGULAR),
            ])
            .expect("failed to register the example fonts");

        let names = text_system.all_font_names();
        let regular = text_system.resolve_font(&selected_font(
            "IBM Plex Sans",
            FontWeight::NORMAL,
            FontStyle::Normal,
        ));
        let italic = text_system.resolve_font(&selected_font(
            "IBM Plex Sans",
            FontWeight::NORMAL,
            FontStyle::Italic,
        ));
        let semibold_italic = text_system.resolve_font(&selected_font(
            "IBM Plex Sans",
            FontWeight::SEMIBOLD,
            FontStyle::Italic,
        ));
        let lilex = text_system.resolve_font(&selected_font(
            "Lilex",
            FontWeight::NORMAL,
            FontStyle::Normal,
        ));

        let checks = vec![
            Check::new(
                "registered family enumeration",
                names.iter().any(|name| name == "IBM Plex Sans")
                    && names.iter().any(|name| name == "Lilex"),
                "IBM Plex Sans and Lilex appear in TextSystem::all_font_names",
            ),
            Check::new(
                "stable family ordering",
                names.windows(2).all(|pair| pair[0] <= pair[1]),
                format!("the backend returned {} sorted family names", names.len()),
            ),
            Check::new(
                "style selection",
                regular != italic,
                format!("regular {regular:?}, italic {italic:?}"),
            ),
            Check::new(
                "combined weight and style selection",
                semibold_italic != regular && semibold_italic != italic,
                format!("semibold italic {semibold_italic:?}"),
            ),
            Check::new(
                "separate family selection",
                lilex != regular,
                format!("IBM Plex Sans {regular:?}, Lilex {lilex:?}"),
            ),
        ];

        let bounds = Bounds::centered(None, size(px(760.0), px(680.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |_, cx| {
                cx.new(|_| FontReport {
                    checks: checks.clone(),
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
