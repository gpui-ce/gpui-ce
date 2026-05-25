use gpui::*;

/// A karaoke-style text display that highlights words as they're "sung"
/// This demonstrates smooth animated text gradients for a lyrics highlighting effect
struct KaraokeDemo {
    progress: f32,
    text: SharedString,
}

impl KaraokeDemo {
    fn new(text: impl Into<SharedString>) -> Self {
        Self {
            progress: 0.0,
            text: text.into(),
        }
    }
}

impl Render for KaraokeDemo {
    fn render(&mut self, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .size_full()
            .bg(rgb(0x1a1a1a))
            .gap_8()
            .child(
                div()
                    .text_size(px(48.0))
                    .font_weight(FontWeight::BOLD)
                    .text_gradient_horizontal(
                        linear_color_stop(rgb(0x00d4ff), 0.0),
                        linear_color_stop(rgb(0x666666), self.progress),
                    )
                    .child(self.text.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .w(px(600.0))
                            .h(px(8.0))
                            .bg(rgb(0x333333))
                            .child(
                                div()
                                    .w(relative(self.progress))
                                    .h_full()
                                    .bg(rgb(0x00d4ff)),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(14.0))
                            .text_color(rgb(0x888888))
                            .child(format!("Progress: {:.1}%", self.progress * 100.0)),
                    ),
            )
    }
}

fn main() {
    App::new().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(100.0), px(100.0)),
                    size: size(px(900.0), px(400.0)),
                })),
                ..Default::default()
            },
            |cx| {
                let lyrics = "The quick brown fox jumps over the lazy dog";
                let demo = KaraokeDemo::new(lyrics);

                // Spawn animation task
                cx.spawn(|view, mut cx| async move {
                    loop {
                        // Animate from 0 to 1 over 5 seconds
                        for i in 0..=500 {
                            let progress = i as f32 / 500.0;
                            view.update(&mut cx, |this, cx| {
                                this.progress = progress;
                                cx.notify();
                            })
                            .ok();
                            async_timer::Timer::after(std::time::Duration::from_millis(10))
                                .await;
                        }
                        // Pause at the end
                        async_timer::Timer::after(std::time::Duration::from_secs(1)).await;
                        // Reset and loop
                        view.update(&mut cx, |this, cx| {
                            this.progress = 0.0;
                            cx.notify();
                        })
                        .ok();
                        async_timer::Timer::after(std::time::Duration::from_millis(500))
                            .await;
                    }
                })
                .detach();

                demo
            },
        )
        .unwrap();
    });
}
