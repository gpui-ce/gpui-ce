use std::time::Duration;

use gpui::{Context, Entity, EventEmitter, Subscription};
use smallvec::SmallVec;

/// Default interval for caret blinking.
pub const DEFAULT_BLINK_INTERVAL: Duration = Duration::from_millis(500);

/// Events emitted that the [`Caret`] listens to.
pub enum CaretNotify {
    /// The caret should pause blinking in response to a user-action
    PauseBlinking,
}

/// State of an EditableText caret cursor, which supports features like blinking.
/// Blinking is disabled by default.
pub struct Caret {
    /// The frequency at which the caret blinks
    interval: Duration,
    generation: usize,
    /// Whether the caret is presently visible in this frame
    visible: bool,
    /// Whether the caret is currently able to blink
    active: bool,
    /// true when blinking is active but paused for some number of frames
    paused: bool,
    #[allow(dead_code)]
    subscriptions: SmallVec<[Subscription; 2]>,
    /// Tracks whether we were focused on the last update.
    was_focused: bool,
}
impl Default for Caret {
    fn default() -> Self {
        Self {
            interval: Duration::ZERO,
            generation: Default::default(),
            visible: false,
            active: false,
            paused: false,
            subscriptions: SmallVec::new(),
            was_focused: false,
        }
    }
}

impl Caret {
    /// Sets the blinking interval of the caret to the global "default".
    /// The true default of the caret is "do not blink".
    pub fn blink_interval_default(mut self) -> Self {
        self.interval = DEFAULT_BLINK_INTERVAL;
        self
    }

    /// Sets the blinking interval of the caret.
    pub fn blink_interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// Listens for CaretNotify events on an entity (e.g. [`EditableTextState`]).
    pub fn subscribe_to<E>(&mut self, emitter: &Entity<E>, cx: &mut Context<Self>)
    where
        E: EventEmitter<CaretNotify>,
    {
        let handle = cx.subscribe(emitter, |state, _emitter, event, cx| match event {
            CaretNotify::PauseBlinking => {
                if !state.interval.is_zero() {
                    state.pause_blinking(cx);
                    cx.notify();
                }
            }
        });
        self.subscriptions.push(handle);
    }

    /// Processes updates during prepaint and returns whether the caret is currently visible.
    pub(super) fn update_focus(&mut self, is_focused: bool, cx: &mut Context<Self>) -> bool {
        let was_focused = self.was_focused;
        self.was_focused = is_focused;

        match (self.interval.is_zero(), is_focused, was_focused) {
            (true, _, _) => true,
            (false, true, false) => {
                self.enable(cx);
                true
            }
            (false, false, true) => {
                self.disable(cx);
                false
            }
            (false, _, _) => self.visible,
        }
    }

    /// Activates caret blinking.
    ///
    /// While active, the caret will alternate between visible and hidden states at the
    /// configured interval. Has no effect if already active.
    fn enable(&mut self, cx: &mut Context<Self>) {
        if self.active {
            return;
        }

        self.active = true;
        self.visible = false;
        self.paused = false;
        self.spawn_ticker(cx);
    }

    /// Deactivates caret blinking.
    ///
    /// Marks the caret as invisible and pauses blinking indefinitely. `enable` must be called
    /// explicitly to resume visibility and blinking. Call `pause_blinking` instead if you want to
    /// temporarily stop blinking while keeping the caret visible.
    fn disable(&mut self, cx: &mut Context<Self>) {
        self.active = false;
        self.visible = false;
        self.paused = false;
        cx.notify();
    }

    /// Temporarily pauses blinking and leaves the caret visible. Blinking will resume after
    /// the pre-established interval elapses from the time this is called.
    fn pause_blinking(&mut self, cx: &mut Context<Self>) {
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
                    this.spawn_ticker(cx);
                }
            })
        })
        .detach();
    }

    fn spawn_ticker(&mut self, cx: &mut Context<Self>) {
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
                        this.spawn_ticker(cx);
                    }
                });
            }
        })
        .detach();
    }
}
