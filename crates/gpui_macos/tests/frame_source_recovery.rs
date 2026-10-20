//! Exercises failed display link starts through the real CoreVideo and dispatch
//! source code paths.
//!
//! The display link registry may only be mutated on the main thread, so this test
//! runs without the libtest harness, whose tests run on worker threads. The private
//! module is compiled in directly rather than exported from the crate for tests.

#[cfg(target_os = "macos")]
#[path = "../src/display_link.rs"]
#[allow(dead_code)]
mod display_link;

#[cfg(target_os = "macos")]
static START_ATTEMPTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Replaces CoreVideo's `CVDisplayLinkStart` within this test executable, so every
/// start fails deterministically while links are still created for a real display.
#[cfg(target_os = "macos")]
#[unsafe(export_name = "CVDisplayLinkStart")]
extern "C" fn fail_display_link_start(_display_link: *mut std::ffi::c_void) -> i32 {
    START_ATTEMPTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    const K_CV_RETURN_ERROR: i32 = -6660;
    K_CV_RETURN_ERROR
}

fn main() {
    #[cfg(target_os = "macos")]
    {
        use core_graphics::display::CGMainDisplayID;
        use display_link::WindowFrameSource;
        use std::{ffi::c_void, sync::atomic::Ordering};

        extern "C" fn unused_callback(_: *mut c_void) {}

        let display_id = unsafe { CGMainDisplayID() };
        let mut first = WindowFrameSource::new(std::ptr::null_mut(), unused_callback);
        let mut second = WindowFrameSource::new(std::ptr::null_mut(), unused_callback);

        // Stopping a source that never started is a no-op.
        first.stop();
        assert!(!first.is_running());

        let error = first
            .start(display_id)
            .expect_err("an injected CVDisplayLinkStart failure must propagate");
        if START_ATTEMPTS.load(Ordering::Relaxed) == 0 {
            // Without an active display no link can be created, so the start
            // failed before reaching CVDisplayLinkStart.
            println!("SKIP: no display link could be created for display {display_id}: {error}");
            return;
        }
        assert!(
            !first.is_running(),
            "a source whose start failed must not stay subscribed"
        );

        // The failed start must roll back the display's shared link state; if
        // it stayed marked as running, later subscribers would never start it.
        for source in [&mut second, &mut first] {
            source
                .start(display_id)
                .expect_err("an injected CVDisplayLinkStart failure must propagate");
            assert!(!source.is_running());
        }
        assert_eq!(
            START_ATTEMPTS.load(Ordering::Relaxed),
            3,
            "every start after a failure must try to start the display link again"
        );

        println!("PASS: failed display link starts are rolled back and retried");
    }
}
