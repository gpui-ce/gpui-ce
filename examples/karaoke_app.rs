use std::time::Duration;

use gpui::*;

/// A polished karaoke app with proper component architecture
/// Using "You Are My Sunshine" - public domain classic
struct KaraokeApp {
    song: Song,
    playback: PlaybackState,
}

struct Song {
    title: SharedString,
    artist: SharedString,
    lines: Vec<LyricLine>,
}

#[derive(Clone)]
struct LyricLine {
    text: SharedString,
    duration: f32,
    color: Rgba,
}

#[derive(Clone)]
struct PlaybackState {
    current_line: usize,
    line_progress: f32,
    scroll_position: f32,
    total_lines: usize,
}

impl PlaybackState {
    fn overall_progress(&self) -> f32 {
        if self.total_lines == 0 {
            return 0.0;
        }
        (self.current_line as f32 + self.line_progress) / self.total_lines as f32
    }

    fn is_active(&self, line_idx: usize) -> bool {
        line_idx == self.current_line
    }

    fn is_past(&self, line_idx: usize) -> bool {
        line_idx < self.current_line
    }
}

impl KaraokeApp {
    fn new(cx: &mut Context<Self>) -> Self {
        // Create the song - lines are pre-split to avoid wrapping issues
        // Each line is kept short enough to fit on screen at large font size
        let lines = vec![
            // Chorus - warm sunny colors
            LyricLine::new("You are my sunshine", 1.8, rgb(0xffd700)),
            LyricLine::new("my only sunshine", 1.7, rgb(0xffcc00)),
            LyricLine::new("You make me happy", 1.8, rgb(0xffb347)),
            LyricLine::new("when skies are gray", 1.7, rgb(0xffa500)),
            LyricLine::new("You'll never know dear", 2.0, rgb(0xff8c42)),
            LyricLine::new("how much I love you", 1.8, rgb(0xff7b35)),
            LyricLine::new("Please don't take", 1.8, rgb(0xff6b35)),
            LyricLine::new("my sunshine away", 1.7, rgb(0xff5a28)),

            // Verse 1 - cool blues
            LyricLine::new("The other night dear", 1.8, rgb(0x5da6ff)),
            LyricLine::new("as I lay sleeping", 1.7, rgb(0x4d96ff)),
            LyricLine::new("I dreamed I held you", 1.8, rgb(0x3d86ff)),
            LyricLine::new("in my arms", 1.5, rgb(0x2d76ff)),
            LyricLine::new("When I awoke dear", 1.8, rgb(0x2d72ff)),
            LyricLine::new("I was mistaken", 1.5, rgb(0x1d68ff)),
            LyricLine::new("So I hung my head", 1.8, rgb(0x1d58d4)),
            LyricLine::new("and cried", 1.2, rgb(0x1d48c4)),

            // Chorus repeat - warm sunny colors
            LyricLine::new("You are my sunshine", 1.8, rgb(0xffd700)),
            LyricLine::new("my only sunshine", 1.7, rgb(0xffcc00)),
            LyricLine::new("You make me happy", 1.8, rgb(0xffb347)),
            LyricLine::new("when skies are gray", 1.7, rgb(0xffa500)),
            LyricLine::new("You'll never know dear", 2.0, rgb(0xff8c42)),
            LyricLine::new("how much I love you", 1.8, rgb(0xff7b35)),
            LyricLine::new("Please don't take", 1.8, rgb(0xff6b35)),
            LyricLine::new("my sunshine away", 1.7, rgb(0xff5a28)),

            // Verse 2 - purples and magentas
            LyricLine::new("I'll always love you", 1.8, rgb(0xda70d6)),
            LyricLine::new("and make you happy", 1.5, rgb(0xca60c6)),
            LyricLine::new("If you will only", 1.6, rgb(0xba55d3)),
            LyricLine::new("say the same", 1.6, rgb(0xaa45c3)),
            LyricLine::new("But if you leave me", 1.8, rgb(0x9a3ab3)),
            LyricLine::new("to love another", 1.7, rgb(0x8a2aa3)),
            LyricLine::new("You'll regret it all", 1.8, rgb(0x7a1f93)),
            LyricLine::new("some day", 1.4, rgb(0x6a0f83)),

            // Final chorus - golden finale
            LyricLine::new("You are my sunshine", 1.8, rgb(0xffd700)),
            LyricLine::new("my only sunshine", 1.7, rgb(0xffcc00)),
            LyricLine::new("You make me happy", 1.8, rgb(0xffb347)),
            LyricLine::new("when skies are gray", 1.7, rgb(0xffa500)),
            LyricLine::new("You'll never know dear", 2.0, rgb(0xff8c42)),
            LyricLine::new("how much I love you", 1.8, rgb(0xff7b35)),
            LyricLine::new("Please don't take", 1.8, rgb(0xff6b35)),
            LyricLine::new("my sunshine away", 2.0, rgb(0xff5a28)),
        ];

        let total_lines = lines.len();
        let song = Song {
            title: "You Are My Sunshine".into(),
            artist: "Public Domain Classic".into(),
            lines,
        };

        let playback = PlaybackState {
            current_line: 0,
            line_progress: 0.0,
            scroll_position: 0.0,
            total_lines,
        };

        let mut app = Self { song, playback };
        app.start_playback(cx);
        app
    }

    fn start_playback(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                let total_lines = this.update(cx, |this, _| this.song.lines.len()).ok().unwrap_or(0);

                for line_idx in 0..total_lines {
                    let duration = this
                        .update(cx, |this, _| {
                            this.playback.current_line = line_idx;
                            this.song.lines[line_idx].duration
                        })
                        .ok()
                        .unwrap_or(2.0);

                    let steps = (duration * 60.0) as u32;
                    for i in 0..=steps {
                        let progress = i as f32 / steps as f32;
                        this.update(cx, |this, cx| {
                            this.playback.line_progress = progress;

                            // Smooth scroll to keep current line in view position 2
                            let target_scroll = (this.playback.current_line as f32).max(2.0) - 2.0;
                            this.playback.scroll_position +=
                                (target_scroll - this.playback.scroll_position) * 0.15;

                            cx.notify();
                        })
                        .ok();

                        Timer::after(Duration::from_millis(16)).await;
                    }

                    // Tiny pause between lines
                    Timer::after(Duration::from_millis(200)).await;
                }

                // Longer pause before restart
                Timer::after(Duration::from_secs(3)).await;

                this.update(cx, |this, cx| {
                    this.playback.current_line = 0;
                    this.playback.line_progress = 0.0;
                    this.playback.scroll_position = 0.0;
                    cx.notify();
                })
                .ok();

                Timer::after(Duration::from_secs(1)).await;
            }
        })
        .detach();
    }
}

impl LyricLine {
    fn new(text: impl Into<SharedString>, duration: f32, color: Rgba) -> Self {
        Self {
            text: text.into(),
            duration,
            color,
        }
    }
}

// ============================================================================
// COMPONENTS
// ============================================================================

/// Single lyric line component with gradient effect
#[derive(IntoElement)]
struct LyricLineComponent {
    line: LyricLine,
    line_idx: usize,
    playback: PlaybackState,
}

impl LyricLineComponent {
    fn new(line: LyricLine, line_idx: usize, playback: PlaybackState) -> Self {
        Self {
            line,
            line_idx,
            playback,
        }
    }

    fn calculate_visual_state(&self) -> LyricVisualState {
        let is_active = self.playback.is_active(self.line_idx);
        let is_past = self.playback.is_past(self.line_idx);
        let visual_position = self.line_idx as f32 - self.playback.scroll_position;
        let distance = (self.line_idx as i32 - self.playback.current_line as i32).abs() as f32;

        // Scale: active is largest, others shrink based on distance
        let scale = if is_active {
            1.3
        } else if distance <= 1.0 {
            1.0
        } else {
            (0.85 - (distance - 1.0) * 0.1).max(0.5)
        };

        // Opacity: active is full, others fade based on distance
        let opacity = if is_active {
            1.0
        } else if distance <= 1.0 {
            0.9
        } else if is_past {
            // Past lines fade out faster
            (0.75 - (distance - 1.0) * 0.2).max(0.0)
        } else {
            // Future lines are dimmer
            (0.5 - (distance - 1.0) * 0.15).max(0.1)
        };

        // Text progress for gradient
        let progress = if is_past {
            1.0
        } else if is_active {
            self.playback.line_progress
        } else {
            0.0
        };

        LyricVisualState {
            scale,
            opacity,
            progress,
            visual_position,
            is_active,
            is_past,
        }
    }
}

struct LyricVisualState {
    scale: f32,
    opacity: f32,
    progress: f32,
    visual_position: f32,
    is_active: bool,
    is_past: bool,
}

impl RenderOnce for LyricLineComponent {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let state = self.calculate_visual_state();
        let y_offset = state.visual_position * px(70.0);

        // Color selection
        let inactive_color = if state.is_past {
            rgb(0x555555)
        } else {
            rgb(0x333333)
        };

        div()
            .absolute()
            .top(y_offset)
            .left_0()
            .right_0()
            .flex()
            .items_center()
            .justify_center()
            .opacity(state.opacity)
            .child(
                div()
                    .text_size(px(52.0 * state.scale))
                    .line_height(relative(1.3))
                    .font_weight(if state.is_active {
                        FontWeight::EXTRA_BOLD
                    } else {
                        FontWeight::SEMIBOLD
                    })
                    .text_gradient_horizontal(
                        linear_color_stop(self.line.color, (state.progress - 0.015).max(0.0)),
                        linear_color_stop(inactive_color, (state.progress + 0.015).min(1.0)),
                    )
                    .child(self.line.text.clone()),
            )
    }
}

/// Elegant title bar
#[derive(IntoElement)]
struct TitleBar {
    title: SharedString,
    artist: SharedString,
}

impl TitleBar {
    fn new(title: SharedString, artist: SharedString) -> Self {
        Self { title, artist }
    }
}

impl RenderOnce for TitleBar {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(90.0))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_1()
            .bg(linear_gradient(
                180.0,
                linear_color_stop(rgba(0x00000099), 0.0),
                linear_color_stop(rgba(0x00000000), 1.0),
            ))
            .child(
                div()
                    .text_size(px(38.0))
                    .font_weight(FontWeight::BOLD)
                    .text_gradient_horizontal(
                        linear_color_stop(rgb(0xffd700), 0.3),
                        linear_color_stop(rgb(0xffaa00), 0.7),
                    )
                    .child(format!("♪ {} ♪", self.title)),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(rgba(0xffffffaa))
                    .child(self.artist),
            )
    }
}

/// Sleek progress bar at bottom
#[derive(IntoElement)]
struct ProgressBar {
    progress: f32,
    current_line: usize,
    total_lines: usize,
}

impl ProgressBar {
    fn new(progress: f32, current_line: usize, total_lines: usize) -> Self {
        Self {
            progress,
            current_line,
            total_lines,
        }
    }

    fn progress_color(&self) -> Rgba {
        if self.progress < 0.25 {
            rgb(0xffd700)
        } else if self.progress < 0.5 {
            rgb(0x5da6ff)
        } else if self.progress < 0.75 {
            rgb(0xda70d6)
        } else {
            rgb(0xffaa00)
        }
    }
}

impl RenderOnce for ProgressBar {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let bar_color = self.progress_color();

        div()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .h(px(75.0))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .bg(linear_gradient(
                0.0,
                linear_color_stop(rgba(0x00000099), 0.0),
                linear_color_stop(rgba(0x00000000), 1.0),
            ))
            .child(
                div()
                    .w(px(600.0))
                    .h(px(6.0))
                    .bg(rgba(0x33333366))
                    .rounded(px(3.0))
                    .overflow_hidden()
                    .child(
                        div()
                            .w(relative(self.progress))
                            .h_full()
                            .bg(bar_color)
                            .rounded(px(3.0))
                            .shadow(vec![BoxShadow {
                                color: Hsla::from(bar_color).opacity(0.6),
                                blur_radius: px(8.0),
                                spread_radius: px(0.0),
                                offset: point(px(0.0), px(0.0)),
                            }]),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgba(0xbbbbbbff))
                            .child(format!("Line {}/{}", self.current_line + 1, self.total_lines)),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgba(0x888888ff))
                            .child("•"),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgba(0xbbbbbbff))
                            .child(format!("{:.0}%", self.progress * 100.0)),
                    ),
            )
    }
}

/// Subtle animated background
#[derive(IntoElement)]
struct AnimatedBackground {
    progress: f32,
}

impl AnimatedBackground {
    fn new(progress: f32) -> Self {
        Self { progress }
    }
}

impl RenderOnce for AnimatedBackground {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        // Subtle color shift through the song
        let (c1, c2) = if self.progress < 0.25 {
            (rgba(0x1a1a14ff), rgba(0x0a0a08ff))
        } else if self.progress < 0.5 {
            (rgba(0x14141aff), rgba(0x08080aff))
        } else if self.progress < 0.75 {
            (rgba(0x1a141aff), rgba(0x0a080aff))
        } else {
            (rgba(0x1a1814ff), rgba(0x0a0a08ff))
        };

        div()
            .absolute()
            .inset_0()
            .bg(linear_gradient(
                145.0,
                linear_color_stop(c1, 0.0),
                linear_color_stop(c2, 1.0),
            ))
    }
}

// ============================================================================
// MAIN RENDER
// ============================================================================

impl Render for KaraokeApp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let playback = self.playback.clone();

        div()
            .flex()
            .size_full()
            .overflow_hidden()
            .child(AnimatedBackground::new(playback.overall_progress()))
            .child(
                div()
                    .absolute()
                    .top(px(90.0))
                    .bottom(px(75.0))
                    .left_0()
                    .right_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .h_full()
                            .overflow_hidden()
                            .child(
                                div()
                                    .relative()
                                    .top(px(180.0))
                                    .w_full()
                                    .children(self.song.lines.iter().enumerate().filter_map(
                                        |(idx, line)| {
                                            let visual_pos = idx as f32 - playback.scroll_position;
                                            if visual_pos < -2.5 || visual_pos > 7.0 {
                                                return None;
                                            }

                                            Some(LyricLineComponent::new(
                                                line.clone(),
                                                idx,
                                                playback.clone(),
                                            ))
                                        },
                                    )),
                            ),
                    ),
            )
            .child(TitleBar::new(
                self.song.title.clone(),
                self.song.artist.clone(),
            ))
            .child(ProgressBar::new(
                playback.overall_progress(),
                playback.current_line,
                playback.total_lines,
            ))
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1400.0), px(800.0)), cx);
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
