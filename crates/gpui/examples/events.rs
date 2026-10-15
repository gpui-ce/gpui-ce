//! Entities that talk to each other through a shared model, notifications and events.
//!
//! A `Palette` model holds the current color. A controls view changes it and calls
//! `cx.notify()`, and a preview view `observe`s it to redraw. When the user saves,
//! the model emits a `PaletteEvent`, which a history view `subscribe`s to. The
//! root view composes the three views as children.
//!
//! Run it with `cargo run --example events`.

use gpui::{
    App, Bounds, Context, Entity, EventEmitter, Hsla, QuitMode, Render, Subscription, Window,
    WindowBounds, WindowOptions, colors::Colors, div, hsla, prelude::*, px, size,
};

struct Palette {
    hue: f32,
    lightness: f32,
}

impl Palette {
    fn color(&self) -> Hsla {
        hsla(self.hue, 0.7, self.lightness, 1.)
    }
}

enum PaletteEvent {
    Saved(Hsla),
}

impl EventEmitter<PaletteEvent> for Palette {}

/// Edits the palette. It holds a handle to the model, but no subscription: it only writes.
struct Controls {
    palette: Entity<Palette>,
}

impl Render for Controls {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);

        // Edits that change the model notify its observers. Saving changes nothing, so it emits an
        // event for subscribers instead.
        let edits: [(&str, fn(&mut Palette, &mut Context<Palette>)); 3] = [
            ("Lighter", |palette, cx| {
                palette.lightness = (palette.lightness + 0.1).min(0.9);
                cx.notify();
            }),
            ("Darker", |palette, cx| {
                palette.lightness = (palette.lightness - 0.1).max(0.1);
                cx.notify();
            }),
            ("Save", |palette, cx| {
                cx.emit(PaletteEvent::Saved(palette.color()))
            }),
        ];

        let swatches = (0..6_usize).map(|index| {
            let hue = index as f32 / 6.;
            let palette = self.palette.clone();
            div()
                .id(("swatch", index))
                .size(px(36.))
                .rounded_md()
                .bg(hsla(hue, 0.7, 0.5, 1.))
                .cursor_pointer()
                .on_click(move |_event, _window, cx| {
                    palette.update(cx, |palette, cx| {
                        palette.hue = hue;
                        cx.notify();
                    })
                })
        });

        let buttons = edits.into_iter().map(|(label, edit)| {
            let palette = self.palette.clone();
            div()
                .id(label)
                .px_4()
                .py_2()
                .rounded_lg()
                .bg(colors.selected)
                .text_color(colors.selected_text)
                .cursor_pointer()
                .hover(|style| style.opacity(0.9))
                .child(label)
                .on_click(move |_event, _window, cx| palette.update(cx, edit))
        });

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().flex().flex_wrap().gap_2().children(swatches))
            .child(div().flex().gap_2().children(buttons))
    }
}

struct Preview {
    palette: Entity<Palette>,
    _observation: Subscription,
}

impl Preview {
    fn new(palette: &Entity<Palette>, cx: &mut Context<Self>) -> Self {
        Self {
            palette: palette.clone(),
            _observation: cx.observe(palette, |_, _, cx| cx.notify()),
        }
    }
}

impl Render for Preview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);
        let palette = self.palette.read(cx);

        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().h(px(96.)).rounded_lg().bg(palette.color()))
            .child(div().text_sm().text_color(colors.disabled).child(format!(
                "Hue {:.0}°, lightness {:.0}%",
                palette.hue * 360.,
                palette.lightness * 100.,
            )))
    }
}

struct History {
    saved: Vec<Hsla>,
    _subscription: Subscription,
}

impl History {
    fn new(palette: &Entity<Palette>, cx: &mut Context<Self>) -> Self {
        Self {
            saved: Vec::new(),
            _subscription: cx.subscribe(palette, |history, _palette, event, cx| {
                let PaletteEvent::Saved(color) = event;
                history.saved.push(*color);
                cx.notify();
            }),
        }
    }
}

impl Render for History {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);

        let saved = if self.saved.is_empty() {
            div()
                .text_sm()
                .text_color(colors.disabled)
                .child("Nothing saved yet.")
        } else {
            div().flex().flex_wrap().gap_2().children(
                self.saved
                    .iter()
                    .map(|color| div().size(px(36.)).rounded_md().bg(*color)),
            )
        };

        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_sm().text_color(colors.disabled).child("Saved"))
            .child(saved)
    }
}

struct Showcase {
    preview: Entity<Preview>,
    controls: Entity<Controls>,
    history: Entity<History>,
}

impl Showcase {
    fn new(cx: &mut Context<Self>) -> Self {
        let palette = cx.new(|_| Palette {
            hue: 0.6,
            lightness: 0.5,
        });
        Self {
            preview: cx.new(|cx| Preview::new(&palette, cx)),
            controls: cx.new(|_| Controls {
                palette: palette.clone(),
            }),
            history: cx.new(|cx| History::new(&palette, cx)),
        }
    }
}

impl Render for Showcase {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);

        div()
            .id("showcase")
            .size_full()
            .flex()
            .flex_col()
            .gap_6()
            .p_6()
            .overflow_y_scroll()
            .bg(colors.background)
            .text_color(colors.text)
            .child(div().text_xl().child("Palette"))
            .child(self.preview.clone())
            .child(self.controls.clone())
            .child(self.history.clone())
    }
}

fn main() {
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(run);
}

/// Opens the example window. The browser player and the example tests call this too.
pub fn run(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(420.), px(620.)), cx);
    cx.open_window(
        WindowOptions::default().window_bounds(WindowBounds::Windowed(bounds)),
        |_window, cx| cx.new(Showcase::new),
    )
    .unwrap();
    cx.activate(true);
}
