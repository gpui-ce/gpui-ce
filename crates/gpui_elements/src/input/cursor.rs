use gpui::Context;
use std::time::Duration;

/// Manages the blinking state of a text cursor.
///
/// The cursor blinks at a configurable interval when enabled. Blinking can be
/// temporarily paused (e.g., during typing) to provide immediate visual feedback.
pub struct CursorBlink {
    interval: Duration,
    generation: usize,
    visible: bool,
    active: bool,
    paused: bool,
}

impl CursorBlink {
    /// Creates a new cursor blink manager with the given interval.
    ///
    /// The cursor starts in a disabled state with visibility set to true.
    pub fn new(interval: Duration, _cx: &mut Context<Self>) -> Self {
        Self {
            interval,
            generation: 0,
            visible: true,
            active: false,
            paused: false,
        }
    }

    /// Returns whether the cursor should currently be rendered.
    pub fn visible(&self) -> bool {
        self.visible
    }

    /// Returns whether blinking is currently active.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Activates cursor blinking.
    ///
    /// When activated, the cursor will alternate between visible and hidden
    /// states at the configured interval. Has no effect if already active.
    pub fn enable(&mut self, cx: &mut Context<Self>) {
        if self.active {
            return;
        }

        self.active = true;
        self.visible = false;
        self.paused = false;
        self.tick(cx);
    }

    /// Deactivates cursor blinking.
    ///
    /// The cursor visibility is set to false when disabled. Call
    /// `pause_blinking` instead if you want to temporarily stop blinking
    /// while keeping the cursor visible.
    pub fn disable(&mut self, cx: &mut Context<Self>) {
        self.active = false;
        self.visible = false;
        self.paused = false;
        cx.notify();
    }

    /// Temporarily pauses blinking and shows the cursor.
    ///
    /// This is useful during user input to provide immediate feedback.
    /// Blinking resumes automatically after the blink interval elapses.
    pub fn pause_blinking(&mut self, cx: &mut Context<Self>) {
        if !self.visible {
            self.visible = true;
            cx.notify();
        }

        self.paused = true;
        self.generation = self.generation.wrapping_add(1);

        let generation = self.generation;
        let interval = self.interval;

        cx.spawn(async move |this, cx| {
            async_io::Timer::after(interval).await;
            this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.paused = false;
                    this.tick(cx);
                }
            })
        })
        .detach();
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        if !self.active || self.paused {
            return;
        }

        self.visible = !self.visible;
        cx.notify();

        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let interval = self.interval;

        cx.spawn(async move |this, cx| {
            async_io::Timer::after(interval).await;
            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| {
                    if this.generation == generation {
                        this.tick(cx);
                    }
                });
            }
        })
        .detach();
    }
}
