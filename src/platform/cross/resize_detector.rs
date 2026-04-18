use std::cell::Cell;
use std::time::{Duration, Instant};
use device_query::{DeviceQuery, DeviceState, MouseButton};

/// Detects whether a window is actively being resized by watching the stream
/// of [`winit::event::WindowEvent::Resized`] events.
///
/// **Primary strategy:** poll global mouse button state via `device_query`.
/// While the left button is held after a resize event started, the user is
/// still dragging. The moment it releases the resize is done instantly.
/// No special permissions required on any platform.
///
/// **Fallback:** idle-threshold timer for keyboard/programmatic resizes where
/// no mouse button is held (e.g. resize via keyboard shortcut, snap, tiling WM).
pub struct ResizeDetector {
    /// Set when a resize event has been seen and we haven't confirmed the end yet.
    active: Cell<bool>,
    /// Deadline for the timer-based fallback (programmatic / non-mouse resizes).
    deadline: Cell<Option<Instant>>,
    /// Cached device_query state — cheap to call, avoids re-initializing each poll.
    device_state: DeviceState,
}

/// Fallback idle period used when the resize wasn't driven by a held mouse button
/// (e.g. keyboard shortcut, tiling WM, programmatic resize).
const IDLE_THRESHOLD: Duration = Duration::from_millis(150);

impl ResizeDetector {
    pub fn new() -> Self {
        Self {
            active: Cell::new(false),
            deadline: Cell::new(None),
            device_state: DeviceState::new(),
        }
    }

    /// Call on every `WindowEvent::Resized` from the winit event loop.
    pub fn on_resize_event(&self) {
        self.active.set(true);
        self.deadline.set(Some(Instant::now() + IDLE_THRESHOLD));
    }

    /// Returns `true` while the resize is in progress.
    pub fn is_resizing(&self) -> bool {
        if !self.active.get() {
            return false;
        }

        // Poll the real global left-button state. If it's held the user is
        // still dragging the resize handle; keep deferring and push the
        // fallback deadline out so it doesn't fire during the drag.
        let mouse = self.device_state.get_mouse();
        let left_held = mouse.button_pressed.contains(&MouseButton::Left);

        if left_held {
            self.deadline.set(Some(Instant::now() + IDLE_THRESHOLD));
            return true;
        }

        // Left button is up. The user finished a mouse-driven resize; done immediately.
        // Also covers the timer path: if the button was never down (keyboard/programmatic)
        // we fall through to the deadline check below.
        //
        // Check deadline in case this is a non-mouse resize that still needs the grace period.
        match self.deadline.get() {
            Some(deadline) if Instant::now() < deadline => {
                // Could be a split-second between event and button release detection;
                // keep active until the deadline just to be safe.
                true
            }
            _ => {
                self.active.set(false);
                self.deadline.set(None);
                false
            }
        }
    }
}

impl Default for ResizeDetector {
    fn default() -> Self {
        Self::new()
    }
}

