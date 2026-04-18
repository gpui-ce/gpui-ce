use std::cell::Cell;
use std::time::{Duration, Instant};

/// Detects whether a window is actively being resized by watching the stream
/// of [`winit::event::WindowEvent::Resized`] events.
///
/// **Primary strategy (macOS):** poll `[NSEvent pressedMouseButtons]` — a
/// permission-free AppKit class method that returns the live bitmask of held
/// mouse buttons. While bit 0 (left button) is set after a resize event, the
/// user is still dragging the resize handle. The moment it clears the resize
/// is done. No timer needed, no mouse events required.
///
/// **Fallback strategy (all platforms):** idle-threshold timer. If no
/// `Resized` event arrives for [`IDLE_THRESHOLD`] the resize is considered
/// finished. Used on Linux/Windows and as the macOS fallback for keyboard or
/// programmatic resizes where no mouse button is held.
pub struct ResizeDetector {
    /// Set when a resize event has been seen and we haven't confirmed the end yet.
    active: Cell<bool>,
    /// Deadline for the timer-based fallback.
    deadline: Cell<Option<Instant>>,
}

/// Fallback idle period after the last `Resized` event on non-macOS platforms.
const IDLE_THRESHOLD: Duration = Duration::from_millis(150);

impl ResizeDetector {
    pub fn new() -> Self {
        Self {
            active: Cell::new(false),
            deadline: Cell::new(None),
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

        // On macOS: ask the OS directly whether the left mouse button is still
        // held. [NSEvent pressedMouseButtons] needs no special permissions.
        #[cfg(target_os = "macos")]
        {
            if left_mouse_button_pressed() {
                // Still dragging — stay active, reset deadline so the fallback
                // doesn't fire while the button is held.
                self.deadline.set(Some(Instant::now() + IDLE_THRESHOLD));
                return true;
            }
            // Button released: resize is over. Fall through to clear state.
            self.active.set(false);
            self.deadline.set(None);
            return false;
        }

        // Non-macOS: timer fallback.
        #[cfg(not(target_os = "macos"))]
        {
            match self.deadline.get() {
                Some(deadline) if Instant::now() < deadline => true,
                _ => {
                    self.active.set(false);
                    self.deadline.set(None);
                    false
                }
            }
        }
    }
}

impl Default for ResizeDetector {
    fn default() -> Self {
        Self::new()
    }
}

/// Poll whether the left mouse button is currently held, using the AppKit
/// `[NSEvent pressedMouseButtons]` class method. Requires no special
/// permissions. Bit 0 of the return value is the left button.
#[cfg(target_os = "macos")]
fn left_mouse_button_pressed() -> bool {
    use std::ffi::c_ulong;

    // We talk directly to the ObjC runtime rather than pulling in a full
    // AppKit binding crate. objc_msgSend is always available on macOS.
    #[link(name = "objc", kind = "dylib")]
    unsafe extern "C" {
        fn objc_getClass(name: *const u8) -> *mut std::ffi::c_void;
        fn sel_registerName(name: *const u8) -> *mut std::ffi::c_void;
        // integer-returning class method — safe to call as a plain fn
        fn objc_msgSend(receiver: *mut std::ffi::c_void, sel: *mut std::ffi::c_void, ...) -> c_ulong;
    }

    let pressed: c_ulong = unsafe {
        let cls = objc_getClass(b"NSEvent\0".as_ptr());
        let sel = sel_registerName(b"pressedMouseButtons\0".as_ptr());
        objc_msgSend(cls, sel)
    };

    // Bit 0 = left button
    pressed & 1 != 0
}

