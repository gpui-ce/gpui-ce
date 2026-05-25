use std::time::Duration;

use gpui::*;

/// A full-featured karaoke app with scrolling, fading, and auto text wrapping
/// Using "You Are My Sunshine" - a well-known public domain song
struct KaraokeApp {
    lines: Vec<KaraokeLine>,
    current_line: usize,
    line_progress: f32,
    scroll_offset: f32, // Vertical offset for scrolling animation
    line_max_width: f32, // Maximum width before wrapping
}

struct KaraokeLine {
    text: SharedString,
    wrapped_segments: Vec<SharedString>, // Text split into segments if it wraps
    char_timings: Vec<f32>, // Duration in seconds for each character
    total_duration: f32,
}

impl KaraokeLine {
    fn new(text: impl Into<SharedString>, timing: f32) -> Self {
        let text_string: SharedString = text.into();
        let char_count = text_string.chars().count();

        // Create even timing for each character
        let char_timings = vec![timing / char_count as f32; char_count];
        let total_duration = timing;

        Self {
            text: text_string,
            wrapped_segments: vec![],
            char_timings,
            total_duration,
        }
    }

    /// Get the progress (0.0 to 1.0) through the text given elapsed time
    fn progress_at_time(&self, elapsed: f32) -> f32 {
        if elapsed >= self.total_duration {
            return 1.0;
        }
        (elapsed / self.total_duration).clamp(0.0, 1.0)
    }

    /// Split text into segments if it exceeds max_width
    /// This is a simple character-based split - in production you'd measure actual text width
    fn wrap_text(&mut self, max_width_chars: usize) {
        let text_str = self.text.to_string();
        if text_str.len() <= max_width_chars {
            self.wrapped_segments = vec![self.text.clone()];
            return;
        }

        // Split at word boundaries when possible
        let mut segments = Vec::new();
        let mut current_segment = String::new();

        for word in text_str.split_whitespace() {
            if current_segment.len() + word.len() + 1 > max_width_chars && !current_segment.is_empty() {
                segments.push(current_segment.trim().to_string().into());
                current_segment = word.to_string();
            } else {
                if !current_segment.is_empty() {
                    current_segment.push(' ');
                }
                current_segment.push_str(word);
            }
        }

        if !current_segment.is_empty() {
            segments.push(current_segment.trim().to_string().into());
        }

        self.wrapped_segments = segments;
    }
}

impl KaraokeApp {
    fn new(cx: &mut Context<Self>) -> Self {
        // "You Are My Sunshine" - Public domain lyrics
        let lines = vec![
            // Chorus
            KaraokeLine::new("You are my sunshine, my only sunshine", 3.5),
            KaraokeLine::new("You make me happy when skies are gray", 3.5),
            KaraokeLine::new("You'll never know dear, how much I love you", 3.8),
            KaraokeLine::new("Please don't take my sunshine away", 3.5),

            // Verse 1
            KaraokeLine::new("The other night dear, as I lay sleeping", 3.5),
            KaraokeLine::new("I dreamed I held you in my arms", 3.3),
            KaraokeLine::new("When I awoke dear, I was mistaken", 3.5),
            KaraokeLine::new("So I hung my head and cried", 3.0),

            // Chorus repeat
            KaraokeLine::new("You are my sunshine, my only sunshine", 3.5),
            KaraokeLine::new("You make me happy when skies are gray", 3.5),
            KaraokeLine::new("You'll never know dear, how much I love you", 3.8),
            KaraokeLine::new("Please don't take my sunshine away", 3.5),

            // Verse 2
            KaraokeLine::new("I'll always love you and make you happy", 3.5),
            KaraokeLine::new("If you will only say the same", 3.2),
            KaraokeLine::new("But if you leave me to love another", 3.5),
            KaraokeLine::new("You'll regret it all some day", 3.2),

            // Final chorus
            KaraokeLine::new("You are my sunshine, my only sunshine", 3.5),
            KaraokeLine::new("You make me happy when skies are gray", 3.5),
            KaraokeLine::new("You'll never know dear, how much I love you", 3.8),
            KaraokeLine::new("Please don't take my sunshine away", 4.0),
        ];

        let mut app = Self {
            lines,
            current_line: 0,
            line_progress: 0.0,
            scroll_offset: 0.0,
            line_max_width: 40.0, // Characters
        };

        // Wrap all lines
        for line in &mut app.lines {
            line.wrap_text(40);
        }

        // Start animation
        cx.spawn(async move |this, cx| {
            loop {
                let total_lines = this.update(cx, |this, _| this.lines.len()).ok().unwrap_or(0);

                for line_idx in 0..total_lines {
                    let duration = this.update(cx, |this, _| {
                        this.current_line = line_idx;
                        this.lines[line_idx].total_duration
                    }).ok().unwrap_or(3.0);

                    let steps = (duration * 60.0) as u32; // 60 FPS
                    for i in 0..=steps {
                        let elapsed = (i as f32 / steps as f32) * duration;
                        this.update(cx, |this, cx| {
                            this.line_progress = this.lines[this.current_line].progress_at_time(elapsed);

                            // Smooth scroll animation - target scroll keeps active line near center
                            let target_scroll = this.current_line as f32;
                            this.scroll_offset += (target_scroll - this.scroll_offset) * 0.08;

                            cx.notify();
                        }).ok();
                        Timer::after(Duration::from_millis((1000.0 / 60.0) as u64)).await;
                    }

                    // Brief pause between lines
                    Timer::after(Duration::from_millis(400)).await;
                }

                // Longer pause before restarting
                Timer::after(Duration::from_secs(3)).await;

                this.update(cx, |this, cx| {
                    this.current_line = 0;
                    this.line_progress = 0.0;
                    this.scroll_offset = 0.0;
                    cx.notify();
                }).ok();

                Timer::after(Duration::from_secs(1)).await;
            }
        }).detach();

        app
    }

    /// Calculate visual properties for a line based on its position relative to current line
    fn line_visual_props(&self, line_idx: usize) -> LineVisualProps {
        let relative_position = line_idx as f32 - self.scroll_offset;
        let is_current = line_idx == self.current_line;
        let distance_from_current = (line_idx as i32 - self.current_line as i32).abs() as f32;

        // Lines above current (already sung)
        let is_past = line_idx < self.current_line;
        // Lines below current (not yet sung)
        let _is_future = line_idx > self.current_line;

        // Scale: current=1.0, adjacent=0.85, far=0.7
        let scale = if is_current {
            1.0
        } else if distance_from_current <= 1.0 {
            0.85
        } else {
            (0.7 - (distance_from_current - 1.0) * 0.05).max(0.5)
        };

        // Opacity: current=1.0, adjacent=0.8, far fades out
        let opacity = if is_current {
            1.0
        } else if distance_from_current <= 1.0 {
            0.8
        } else if is_past {
            // Fade out as lines move up and away
            (0.6 - (distance_from_current - 1.0) * 0.15).max(0.0)
        } else {
            // Future lines fade in from below
            (0.4 - (distance_from_current - 1.0) * 0.1).max(0.2)
        };

        // Progress: 1.0 for past, current progress for current, 0.0 for future
        let progress = if is_past {
            1.0
        } else if is_current {
            self.line_progress
        } else {
            0.0
        };

        // Color based on progress through song
        let song_progress = line_idx as f32 / self.lines.len() as f32;
        let color = self.interpolate_color(song_progress);

        LineVisualProps {
            scale,
            opacity,
            progress,
            active_color: color,
            inactive_color: rgb(0x555555),
            relative_position,
        }
    }

    /// Interpolate color through rainbow for visual variety
    fn interpolate_color(&self, t: f32) -> Rgba {
        // Smooth color gradient through the song
        if t < 0.25 {
            // Blue to Cyan
            let blue = rgb(0x0080ff);
            let cyan = rgb(0x00ffff);
            let local_t = t * 4.0;
            Rgba {
                r: blue.r * (1.0 - local_t) + cyan.r * local_t,
                g: blue.g * (1.0 - local_t) + cyan.g * local_t,
                b: blue.b * (1.0 - local_t) + cyan.b * local_t,
                a: 1.0,
            }
        } else if t < 0.5 {
            // Cyan to Yellow
            let cyan = rgb(0x00ffff);
            let yellow = rgb(0xffff00);
            let local_t = (t - 0.25) * 4.0;
            Rgba {
                r: cyan.r * (1.0 - local_t) + yellow.r * local_t,
                g: cyan.g * (1.0 - local_t) + yellow.g * local_t,
                b: cyan.b * (1.0 - local_t) + yellow.b * local_t,
                a: 1.0,
            }
        } else if t < 0.75 {
            // Yellow to Magenta
            let yellow = rgb(0xffff00);
            let magenta = rgb(0xff00ff);
            let local_t = (t - 0.5) * 4.0;
            Rgba {
                r: yellow.r * (1.0 - local_t) + magenta.r * local_t,
                g: yellow.g * (1.0 - local_t) + magenta.g * local_t,
                b: yellow.b * (1.0 - local_t) + magenta.b * local_t,
                a: 1.0,
            }
        } else {
            // Magenta to Blue
            let magenta = rgb(0xff00ff);
            let blue = rgb(0x0080ff);
            let local_t = (t - 0.75) * 4.0;
            Rgba {
                r: magenta.r * (1.0 - local_t) + blue.r * local_t,
                g: magenta.g * (1.0 - local_t) + blue.g * local_t,
                b: magenta.b * (1.0 - local_t) + blue.b * local_t,
                a: 1.0,
            }
        }
    }
}

struct LineVisualProps {
    scale: f32,
    opacity: f32,
    progress: f32,
    active_color: Rgba,
    inactive_color: Rgba,
    relative_position: f32,
}

impl Render for KaraokeApp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let visible_lines = 5; // Show 5 lines at a time
        let center_position = (visible_lines / 2) as f32;

        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .size_full()
            .bg(rgb(0x0a0a0a))
            .overflow_hidden()
            .child(
                // Title bar
                div()
                    .absolute()
                    .top_0()
                    .w_full()
                    .flex()
                    .justify_center()
                    .p_6()
                    .bg(rgba(0x000000aa))
                    .child(
                        div()
                            .text_size(px(28.0))
                            .text_color(rgb(0xffffff))
                            .font_weight(FontWeight::BOLD)
                            .text_gradient_horizontal(
                                linear_color_stop(rgb(0xffd700), 0.0),
                                linear_color_stop(rgb(0xffa500), 1.0),
                            )
                            .child("♫ You Are My Sunshine ♫"),
                    ),
            )
            .child(
                // Main scrolling area
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(16.0))
                    .w_full()
                    .h_full()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(20.0))
                            .children(
                                self.lines.iter().enumerate().map(|(idx, line)| {
                                    let props = self.line_visual_props(idx);

                                    // Only render lines near the current position
                                    if (props.relative_position - center_position).abs() > 4.0 {
                                        return div().into_any_element();
                                    }

                                    let y_offset = (props.relative_position - center_position) * 90.0;

                                    // Render wrapped segments if any, otherwise single line
                                    let segments = if !line.wrapped_segments.is_empty() {
                                        line.wrapped_segments.clone()
                                    } else {
                                        vec![line.text.clone()]
                                    };

                                    div()
                                        .relative()
                                        .top(px(y_offset))
                                        .flex()
                                        .flex_col()
                                        .items_center()
                                        .gap(px(4.0))
                                        .children(segments.into_iter().map(|segment| {
                                            div()
                                                .text_size(px(40.0 * props.scale))
                                                .font_weight(if idx == self.current_line {
                                                    FontWeight::BOLD
                                                } else {
                                                    FontWeight::SEMIBOLD
                                                })
                                                .text_gradient_horizontal(
                                                    linear_color_stop(
                                                        props.active_color,
                                                        props.progress.max(0.01) - 0.01,
                                                    ),
                                                    linear_color_stop(
                                                        props.inactive_color,
                                                        props.progress.max(0.01) + 0.01,
                                                    ),
                                                )
                                                .opacity(props.opacity)
                                                .child(segment)
                                        }))
                                        .into_any_element()
                                }),
                            ),
                    ),
            )
            .child(
                // Progress indicator at bottom
                div()
                    .absolute()
                    .bottom_0()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .p_4()
                    .bg(rgba(0x000000aa))
                    .child(
                        div()
                            .w(px(400.0))
                            .h(px(6.0))
                            .bg(rgb(0x333333))
                            .rounded(px(3.0))
                            .child(
                                div()
                                    .w(relative((self.current_line as f32 + self.line_progress) / self.lines.len() as f32))
                                    .h_full()
                                    .bg(
                                        self.interpolate_color(
                                            (self.current_line as f32 + self.line_progress) / self.lines.len() as f32
                                        )
                                    )
                                    .rounded(px(3.0)),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0x888888))
                            .child(format!(
                                "Line {}/{} • {:.0}%",
                                self.current_line + 1,
                                self.lines.len(),
                                ((self.current_line as f32 + self.line_progress) / self.lines.len() as f32) * 100.0
                            )),
                    ),
            )
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1200.0), px(700.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|cx| KaraokeApp::new(cx)),
        )
        .expect("Failed to open window");
    });
}
