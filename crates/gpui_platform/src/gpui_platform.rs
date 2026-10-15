//! Selects GPUI's platform for the current operating system.
//!
//! Call [`application`] to get a [`gpui::Application`] for an interactive app,
//! or [`headless`] for tests and tools that don't need a display. This crate
//! picks the macOS, Windows, Linux or web platform for you, so your code needs
//! no `#[cfg]` gates for each OS. It also re-exports [`Platform`], the trait
//! that each platform implements.
//!
//! ```no_run
//! # extern crate gpui_ce_platform as gpui_platform;
//! use gpui::{App, Context, IntoElement, Render, Window, WindowOptions, div, prelude::*};
//!
//! struct HelloWorld;
//!
//! impl Render for HelloWorld {
//!     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
//!         div().child("Hello, world!")
//!     }
//! }
//!
//! fn main() {
//!     gpui_platform::application().run(|cx: &mut App| {
//!         cx.open_window(WindowOptions::default(), |_window, cx| cx.new(|_| HelloWorld))
//!             .unwrap();
//!         cx.activate(true);
//!     });
//! }
//! ```

pub use gpui::Platform;

use std::rc::Rc;

/// Returns a [`gpui::BackgroundExecutor`] from a new headless platform.
///
/// Use it where no `App` exists, such as in a tool or a test helper. Each call
/// creates a platform to get the executor, so keep the returned value instead of
/// calling this in a loop. Inside an app, use `background_executor` on the
/// context instead.
///
/// ```no_run
/// # extern crate gpui_ce_platform as gpui_platform;
/// let executor = gpui_platform::background_executor();
/// executor.spawn(async { println!("running in the background") }).detach();
/// ```
pub fn background_executor() -> gpui::BackgroundExecutor {
    current_platform(true).background_executor()
}

/// Returns an [`gpui::Application`] for an interactive app on the current platform.
///
/// On macOS, Windows and Linux this connects to the window system. On the web,
/// it calls `application_with_web_backend` with `WebBackendPreference::Auto`, which
/// tries WebGPU and then WebGL2. If neither backend can initialize, `run` reports the
/// platform startup failure instead of opening a window. Call
/// [`gpui::Application::run`] on the result to start the event loop.
///
/// ```no_run
/// # extern crate gpui_ce_platform as gpui_platform;
/// gpui_platform::application().run(|cx| {
///     cx.quit();
/// });
/// ```
pub fn application() -> gpui::Application {
    #[cfg(target_family = "wasm")]
    {
        application_with_web_backend(gpui_web::WebBackendPreference::Auto)
    }

    #[cfg(not(target_family = "wasm"))]
    gpui::Application::with_platform(current_platform(false))
}

/// Returns an [`gpui::Application`] that doesn't connect to a display.
///
/// Use it for tests, command-line tools and servers that render without showing
/// a window. Use [`application`] for an app with a visible UI.
///
/// ```no_run
/// # extern crate gpui_ce_platform as gpui_platform;
/// gpui_platform::headless().run(|cx| {
///     cx.quit();
/// });
/// ```
pub fn headless() -> gpui::Application {
    gpui::Application::with_platform(current_platform(true))
}

/// The rendering backend that the web platform uses.
///
/// `Auto` (the default) prefers browser WebGPU and falls back to WebGL2. `WebGpu`
/// and `WebGl` force one backend. Pass it to `application_with_web_backend`.
#[cfg(target_family = "wasm")]
pub use gpui_web::WebBackendPreference;

/// Returns an [`gpui::Application`] for the web, using the given rendering backend.
///
/// The application allows background work to run on Web Workers when `gpui_web`
/// is built with multithreading support. Use `WebBackendPreference::Auto` for the
/// default choice, or force `WebGpu` or `WebGl` to pick a backend.
///
/// # Panics
///
/// Panics if the code isn't running in a browser window, because the platform
/// needs a `window` object.
#[cfg(target_family = "wasm")]
pub fn application_with_web_backend(backend_preference: WebBackendPreference) -> gpui::Application {
    let platform = Rc::new(gpui_web::WebPlatform::new_with_backend(
        true,
        backend_preference,
    ));
    let http_client = std::sync::Arc::new(platform.fetch_http_client());
    gpui::Application::with_platform(platform).with_http_client(http_client)
}

/// Returns a web [`gpui::Application`] that runs all of its work on the main thread.
///
/// Unlike [`application`] on the web, this doesn't allow background work to run
/// on Web Workers. It uses the automatic rendering backend, so it is suitable for
/// deployments that want a single-threaded dispatcher. Build without shared-memory
/// WebAssembly linker flags when the deployment cannot provide cross-origin isolation;
/// this function cannot change the memory mode of an already-built binary.
///
/// # Panics
///
/// Panics if the code isn't running in a browser window.
#[cfg(target_family = "wasm")]
pub fn single_threaded_web() -> gpui::Application {
    let platform = Rc::new(gpui_web::WebPlatform::new(false));
    let http_client = std::sync::Arc::new(platform.fetch_http_client());
    gpui::Application::with_platform(platform).with_http_client(http_client)
}

/// Sets up panic reporting and logging for the web platform.
///
/// Panics are printed to the browser console with their messages. Log output
/// goes to the console too. Call this once at the start of your
/// `#[wasm_bindgen]` entry point, before you create an application.
#[cfg(target_family = "wasm")]
pub fn web_init() {
    console_error_panic_hook::set_once();
    gpui_web::init_logging();
}

/// Returns the default [`Platform`] for the current operating system.
///
/// `headless` decides whether the platform connects to the window system.
/// `false` is for an interactive app, and `true` is for tests and tools that
/// do not need a display. On the web it has no effect. Each call creates a new platform.
/// Most code should use [`application`] or [`headless`] instead.
///
/// # Panics
///
/// Panics on Windows if the platform can't be initialized.
pub fn current_platform(headless: bool) -> Rc<dyn Platform> {
    #[cfg(target_os = "macos")]
    {
        Rc::new(gpui_macos::MacPlatform::new(headless))
    }

    #[cfg(target_os = "windows")]
    {
        Rc::new(
            gpui_windows::WindowsPlatform::new(headless)
                .expect("failed to initialize Windows platform"),
        )
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        gpui_linux::current_platform(headless)
    }

    #[cfg(target_family = "wasm")]
    {
        let _ = headless;
        Rc::new(gpui_web::WebPlatform::new(true))
    }
}

/// Returns a [`gpui::PlatformHeadlessRenderer`] for drawing without a window.
///
/// Returns `None` on platforms that have no headless renderer. It is available
/// only with the `bench-support` or `test-support` feature. On macOS it returns a
/// Metal renderer.
#[cfg(any(feature = "bench-support", feature = "test-support"))]
pub fn current_headless_renderer() -> Option<Box<dyn gpui::PlatformHeadlessRenderer>> {
    #[cfg(target_os = "macos")]
    {
        Some(
            Box::new(gpui_apple::metal_renderer::MetalHeadlessRenderer::new())
                as Box<dyn gpui::PlatformHeadlessRenderer>,
        )
    }

    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

#[cfg(all(test, feature = "test-support", target_os = "macos"))]
mod tests {
    use super::*;
    use gpui::{AppContext, Empty, VisualTestAppContext};
    use std::cell::RefCell;
    use std::time::Duration;

    // Note: All VisualTestAppContext tests are ignored by default because they require
    // the macOS main thread. Standard Rust tests run on worker threads, which causes
    // SIGABRT when interacting with macOS AppKit/Cocoa APIs.
    //
    // To run these tests, use:
    // cargo test -p gpui visual_test_context -- --ignored --test-threads=1

    #[test]
    #[ignore] // Requires macOS main thread
    fn test_foreground_tasks_run_with_run_until_parked() {
        let mut cx = VisualTestAppContext::new(current_platform(false));

        let task_ran = Rc::new(RefCell::new(false));

        // Spawn a foreground task via the App's spawn method
        // This should use our TestDispatcher, not the MacDispatcher
        {
            let task_ran = task_ran.clone();
            cx.update(|cx| {
                cx.spawn(async move |_| {
                    *task_ran.borrow_mut() = true;
                })
                .detach();
            });
        }

        // The task should not have run yet
        assert!(!*task_ran.borrow());

        // Run until parked should execute the foreground task
        cx.run_until_parked();

        // Now the task should have run
        assert!(*task_ran.borrow());
    }

    #[test]
    #[ignore] // Requires macOS main thread
    fn test_advance_clock_triggers_delayed_tasks() {
        let mut cx = VisualTestAppContext::new(current_platform(false));

        let task_ran = Rc::new(RefCell::new(false));

        // Spawn a task that waits for a timer
        {
            let task_ran = task_ran.clone();
            let executor = cx.background_executor.clone();
            cx.update(|cx| {
                cx.spawn(async move |_| {
                    executor.timer(Duration::from_millis(500)).await;
                    *task_ran.borrow_mut() = true;
                })
                .detach();
            });
        }

        // Run until parked - the task should be waiting on the timer
        cx.run_until_parked();
        assert!(!*task_ran.borrow());

        // Advance clock past the timer duration
        cx.advance_clock(Duration::from_millis(600));

        // Now the task should have completed
        assert!(*task_ran.borrow());
    }

    #[test]
    #[ignore] // Requires macOS main thread - window creation fails on test threads
    fn test_window_spawn_uses_test_dispatcher() {
        let mut cx = VisualTestAppContext::new(current_platform(false));

        let task_ran = Rc::new(RefCell::new(false));

        let window = cx
            .open_offscreen_window_default(|_, cx| cx.new(|_| Empty))
            .expect("Failed to open window");

        // Spawn a task via window.spawn - this is the critical test case
        // for tooltip behavior, as tooltips use window.spawn for delayed show
        {
            let task_ran = task_ran.clone();
            cx.update_window(window.into(), |_, window, cx| {
                window
                    .spawn(cx, async move |_| {
                        *task_ran.borrow_mut() = true;
                    })
                    .detach();
            })
            .ok();
        }

        // The task should not have run yet
        assert!(!*task_ran.borrow());

        // Run until parked should execute the foreground task spawned via window
        cx.run_until_parked();

        // Now the task should have run
        assert!(*task_ran.borrow());
    }
}
