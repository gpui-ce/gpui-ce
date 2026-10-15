//! A headless app for tests of views, entities and input.
//!
//! [`TestApp`] runs a real [`App`] on a deterministic scheduler, with a test
//! platform in place of the operating system. Open windows with
//! [`TestApp::open_window`], send them input with the `simulate_*` methods on
//! [`TestAppWindow`], and read the results back. Tasks spawned in the app don't
//! run until you call [`TestApp::run_until_parked`] or an `update` method.
//!
//! Use [`TestAppContext`](crate::TestAppContext) instead when a test needs an
//! async function, several apps or randomized scheduling. The
//! [testing guide](crate::_documentation::guides::testing) covers both.
//!
//! # Example
//!
//! ```
//! use gpui::{Context, MouseButton, TestApp, Window, div, point, prelude::*, px};
//!
//! struct Counter {
//!     count: usize,
//! }
//!
//! impl Render for Counter {
//!     fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
//!         div()
//!             .id("increment")
//!             .size(px(100.))
//!             .on_click(cx.listener(|this, _event, _window, cx| {
//!                 this.count += 1;
//!                 cx.notify();
//!             }))
//!             .child(format!("{}", self.count))
//!     }
//! }
//!
//! let mut app = TestApp::new();
//! let mut window = app.open_window(|_, _| Counter { count: 0 });
//!
//! // Paint once, so the click has bounds to hit.
//! window.draw();
//! window.simulate_click(point(px(10.), px(10.)), MouseButton::Left);
//!
//! window.read(|counter, _| assert_eq!(counter.count, 1));
//! ```

use crate::{
    AnyWindowHandle, App, AppCell, AppContext, AssetRegistry, AsyncApp, BackgroundExecutor,
    BorrowAppContext, Bounds, ClipboardItem, Context, Entity, ForegroundExecutor, Global,
    InputEvent, Keystroke, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Platform, PlatformTextSystem, Point, Render, Size, Task, TestDispatcher, TestPlatform,
    TextSystem, Window, WindowBounds, WindowHandle, WindowOptions, app::GpuiMode,
};
use std::{future::Future, rc::Rc, sync::Arc, time::Duration};

/// A headless app for tests.
///
/// `TestApp` runs a real [`App`] on a deterministic scheduler. Its platform keeps
/// the clipboard, opened URLs and prompts in memory, so you can assert on them
/// instead of seeing them on screen. Each `TestApp` is independent: state set in
/// one doesn't appear in another.
///
/// See the [testing guide](crate::_documentation::guides::testing).
pub struct TestApp {
    app: Rc<AppCell>,
    platform: Rc<TestPlatform>,
    background_executor: BackgroundExecutor,
    foreground_executor: ForegroundExecutor,
    #[allow(dead_code)]
    dispatcher: TestDispatcher,
    text_system: Arc<TextSystem>,
}

impl TestApp {
    /// Creates a test app with scheduler seed `0`.
    ///
    /// Text is shaped by the test text system, not by real fonts. Use
    /// [`TestApp::with_text_system`] when a test measures real glyphs.
    pub fn new() -> Self {
        Self::with_seed(0)
    }

    /// Creates a test app whose scheduler orders ready tasks by `seed`.
    ///
    /// The seed decides which ready task runs first. The same seed gives the same
    /// order every time, so a failing seed can be replayed.
    pub fn with_seed(seed: u64) -> Self {
        Self::build(seed, None, Arc::new(()))
    }

    /// Creates a test app that shapes text with `text_system`. Uses seed `0`.
    ///
    /// Pass a real text system to measure glyphs as the app would on a platform.
    pub fn with_text_system(text_system: Arc<dyn PlatformTextSystem>) -> Self {
        Self::build(0, Some(text_system), Arc::new(()))
    }

    /// Creates a test app with the given text system and asset source.
    ///
    /// This is [`TestApp::with_text_system`] plus an [`AssetSource`](crate::AssetSource),
    /// so views can load icons and other assets. Uses seed `0`.
    pub fn with_text_system_and_assets(
        text_system: Arc<dyn PlatformTextSystem>,
        asset_source: Arc<dyn crate::AssetSource>,
    ) -> Self {
        Self::build(0, Some(text_system), asset_source)
    }

    fn build(
        seed: u64,
        platform_text_system: Option<Arc<dyn PlatformTextSystem>>,
        asset_source: Arc<dyn crate::AssetSource>,
    ) -> Self {
        let dispatcher = TestDispatcher::new(seed);
        let arc_dispatcher = Arc::new(dispatcher.clone());
        let background_executor = BackgroundExecutor::new(arc_dispatcher.clone());
        let foreground_executor = ForegroundExecutor::new(arc_dispatcher);
        let platform = match platform_text_system.clone() {
            Some(ts) => TestPlatform::with_text_system(
                background_executor.clone(),
                foreground_executor.clone(),
                ts,
            ),
            None => TestPlatform::new(background_executor.clone(), foreground_executor.clone()),
        };
        let http_client = crate::http_client::FakeHttpClient::with_404_response();
        let text_system = Arc::new(TextSystem::new(
            platform_text_system.unwrap_or_else(|| platform.text_system.clone()),
        ));

        let app = App::new_app(
            platform.clone(),
            AssetRegistry::from(asset_source).into(),
            http_client,
        );
        app.borrow_mut().mode = GpuiMode::test();

        Self {
            app,
            platform,
            background_executor,
            foreground_executor,
            dispatcher,
            text_system,
        }
    }

    /// Runs `f` with mutable access to the [`App`], then runs pending tasks until none can make progress.
    ///
    /// Effects such as observers and subscriptions run when `f` returns, before the
    /// pending tasks run. Timers that haven't expired stay pending; use
    /// [`TestApp::advance_clock`] to fire them.
    ///
    /// # Example
    ///
    /// ```
    /// # use gpui::TestApp;
    /// # use std::{cell::Cell, rc::Rc};
    /// let mut app = TestApp::new();
    /// let finished = Rc::new(Cell::new(false));
    ///
    /// let flag = finished.clone();
    /// app.spawn(|_| async move { flag.set(true) }).detach();
    ///
    /// // Any update runs the task that was spawned above.
    /// app.update(|_| ());
    /// assert!(finished.get());
    /// ```
    pub fn update<R>(&mut self, f: impl FnOnce(&mut App) -> R) -> R {
        let result = {
            let mut app = self.app.borrow_mut();
            app.update(f)
        };
        self.run_until_parked();
        result
    }

    /// Runs `f` with read-only access to the [`App`].
    ///
    /// This doesn't run pending tasks. A task that is still waiting to run doesn't
    /// run before `f` is called.
    pub fn read<R>(&self, f: impl FnOnce(&App) -> R) -> R {
        let app = self.app.borrow();
        f(&app)
    }

    /// Creates an entity from `build` and runs pending tasks until parked.
    ///
    /// This is `cx.new(build)` inside [`TestApp::update`].
    pub fn new_entity<T: 'static>(
        &mut self,
        build: impl FnOnce(&mut Context<T>) -> T,
    ) -> Entity<T> {
        self.update(|cx| cx.new(build))
    }

    /// Updates an entity and runs pending tasks until parked.
    pub fn update_entity<T: 'static, R>(
        &mut self,
        entity: &Entity<T>,
        f: impl FnOnce(&mut T, &mut Context<T>) -> R,
    ) -> R {
        self.update(|cx| entity.update(cx, f))
    }

    /// Reads an entity without running pending tasks.
    pub fn read_entity<T: 'static, R>(
        &self,
        entity: &Entity<T>,
        f: impl FnOnce(&T, &App) -> R,
    ) -> R {
        self.read(|cx| f(entity.read(cx), cx))
    }

    /// Opens a window sized to the test display and returns a [`TestAppWindow`] for its root view.
    ///
    /// The window's bounds are the maximized bounds of the test display. Use
    /// [`TestApp::open_window_with_options`] to choose other bounds.
    ///
    /// # Panics
    ///
    /// Panics if the window can't be opened.
    pub fn open_window<V: Render + 'static>(
        &mut self,
        build_view: impl FnOnce(&mut Window, &mut Context<V>) -> V,
    ) -> TestAppWindow<V> {
        let bounds = self.read(|cx| Bounds::maximized(None, cx));
        let handle = self.update(|cx| {
            cx.open_window(
                WindowOptions::new().window_bounds(Some(WindowBounds::Windowed(bounds))),
                |window, cx| cx.new(|cx| build_view(window, cx)),
            )
            .unwrap()
        });

        TestAppWindow {
            handle,
            app: self.app.clone(),
            platform: self.platform.clone(),
            background_executor: self.background_executor.clone(),
        }
    }

    /// Opens a window with `options` and returns a [`TestAppWindow`] for its root view.
    ///
    /// # Panics
    ///
    /// Panics if the window can't be opened.
    pub fn open_window_with_options<V: Render + 'static>(
        &mut self,
        options: WindowOptions,
        build_view: impl FnOnce(&mut Window, &mut Context<V>) -> V,
    ) -> TestAppWindow<V> {
        let handle = self.update(|cx| {
            cx.open_window(options, |window, cx| cx.new(|cx| build_view(window, cx)))
                .unwrap()
        });

        TestAppWindow {
            handle,
            app: self.app.clone(),
            platform: self.platform.clone(),
            background_executor: self.background_executor.clone(),
        }
    }

    /// Runs tasks until none can make progress.
    ///
    /// This doesn't move the simulated clock. A timer that hasn't expired stays
    /// pending until [`TestApp::advance_clock`] moves time past it.
    ///
    /// # Example
    ///
    /// ```
    /// # use gpui::TestApp;
    /// # use std::{cell::Cell, rc::Rc, time::Duration};
    /// let mut app = TestApp::new();
    /// let fired = Rc::new(Cell::new(false));
    ///
    /// let flag = fired.clone();
    /// app.spawn(|cx| async move {
    ///     cx.background_executor().timer(Duration::from_secs(1)).await;
    ///     flag.set(true);
    /// })
    /// .detach();
    ///
    /// app.run_until_parked();
    /// assert!(!fired.get(), "the timer hasn't expired yet");
    ///
    /// app.advance_clock(Duration::from_secs(1));
    /// app.run_until_parked();
    /// assert!(fired.get());
    /// ```
    pub fn run_until_parked(&self) {
        self.background_executor.run_until_parked();
    }

    /// Moves the simulated clock forward by `duration`.
    ///
    /// Tasks that become ready run during the call, including timers that expire
    /// within the window. Use this to test timeouts and delays, as in the example for
    /// [`TestApp::run_until_parked`].
    pub fn advance_clock(&self, duration: Duration) {
        self.background_executor.advance_clock(duration);
    }

    /// Spawns a future on the foreground executor, given an [`AsyncApp`] for this app.
    ///
    /// The future runs when the app runs tasks: during [`TestApp::run_until_parked`],
    /// [`TestApp::advance_clock`] or the next `update`. Await, store or detach the
    /// returned [`Task`], as in a real app. See [async work](crate::_documentation::guides::async_work).
    pub fn spawn<Fut, R>(&self, f: impl FnOnce(AsyncApp) -> Fut) -> Task<R>
    where
        Fut: Future<Output = R> + 'static,
        R: 'static,
    {
        self.foreground_executor.spawn(f(self.to_async()))
    }

    /// Spawns a `Send` future on the background executor.
    ///
    /// Like [`TestApp::spawn`], the future runs when the app runs tasks.
    pub fn background_spawn<R>(&self, future: impl Future<Output = R> + Send + 'static) -> Task<R>
    where
        R: Send + 'static,
    {
        self.background_executor.spawn(future)
    }

    /// Returns an [`AsyncApp`] for this app.
    ///
    /// The handle holds a weak reference, so it can't keep the app alive.
    pub fn to_async(&self) -> AsyncApp {
        AsyncApp {
            app: Rc::downgrade(&self.app),
            background_executor: self.background_executor.clone(),
            foreground_executor: self.foreground_executor.clone(),
        }
    }

    /// Returns the test background executor.
    pub fn background_executor(&self) -> &BackgroundExecutor {
        &self.background_executor
    }

    /// Returns the test foreground executor.
    pub fn foreground_executor(&self) -> &ForegroundExecutor {
        &self.foreground_executor
    }

    /// Returns the text system the app uses to shape and measure text.
    pub fn text_system(&self) -> &Arc<TextSystem> {
        &self.text_system
    }

    /// Returns whether a global of type `G` is set.
    pub fn has_global<G: Global>(&self) -> bool {
        self.read(|cx| cx.has_global::<G>())
    }

    /// Sets the global of type `G`, replacing any previous value, then runs pending tasks.
    pub fn set_global<G: Global>(&mut self, global: G) {
        self.update(|cx| cx.set_global(global));
    }

    /// Runs `f` with the global of type `G`.
    ///
    /// # Panics
    ///
    /// Panics if no global of type `G` is set. Check with [`TestApp::has_global`] first.
    pub fn read_global<G: Global, R>(&self, f: impl FnOnce(&G, &App) -> R) -> R {
        self.read(|cx| f(cx.global(), cx))
    }

    /// Runs `f` with mutable access to the global of type `G`, then runs pending tasks.
    ///
    /// # Panics
    ///
    /// Panics if no global of type `G` is set.
    pub fn update_global<G: Global, R>(&mut self, f: impl FnOnce(&mut G, &mut App) -> R) -> R {
        self.update(|cx| cx.update_global(f))
    }

    // Platform simulation methods

    /// Writes `item` to the test clipboard.
    ///
    /// [`TestApp::read_from_clipboard`] returns the item until something else is written.
    pub fn write_to_clipboard(&self, item: ClipboardItem) {
        self.platform.write_to_clipboard(item);
    }

    /// Returns the item most recently written to the test clipboard, or `None` if nothing was written.
    pub fn read_from_clipboard(&self) -> Option<ClipboardItem> {
        self.platform.read_from_clipboard()
    }

    /// Returns the URL most recently passed to [`App::open_url`], or `None`.
    ///
    /// Nothing is launched. Only the last URL is kept, so open a second URL and the
    /// first is no longer reported.
    ///
    /// # Example
    ///
    /// ```
    /// # use gpui::TestApp;
    /// let mut app = TestApp::new();
    /// app.update(|cx| cx.open_url("https://example.com"));
    /// assert_eq!(app.opened_url().as_deref(), Some("https://example.com"));
    /// ```
    pub fn opened_url(&self) -> Option<String> {
        self.platform.opened_url.borrow().clone()
    }

    /// Returns whether a [`App::prompt_for_new_path`] request is waiting for an answer.
    ///
    /// Answer it with [`TestApp::simulate_new_path_selection`].
    pub fn did_prompt_for_new_path(&self) -> bool {
        self.platform.did_prompt_for_new_path()
    }

    /// Answers the oldest pending [`App::prompt_for_new_path`] request.
    ///
    /// `select` receives the directory the app asked about and returns the chosen
    /// path, or `None` to cancel the request.
    ///
    /// # Panics
    ///
    /// Panics if no new-path request is pending.
    pub fn simulate_new_path_selection(
        &self,
        select: impl FnOnce(&std::path::Path) -> Option<std::path::PathBuf>,
    ) {
        self.platform.simulate_new_path_selection(select);
    }

    /// Returns whether a prompt from [`Window::prompt`] is waiting for an answer.
    pub fn has_pending_prompt(&self) -> bool {
        self.platform.has_pending_prompt()
    }

    /// Answers the oldest pending prompt by pressing the button labelled `button`.
    ///
    /// The prompt is removed before the label is checked, so a wrong label still
    /// consumes it.
    ///
    /// # Panics
    ///
    /// Panics if no prompt is pending, or if no button has the label `button`.
    pub fn simulate_prompt_answer(&self, button: &str) {
        self.platform.simulate_prompt_answer(button);
    }

    /// Returns the handles of all open windows.
    pub fn windows(&self) -> Vec<AnyWindowHandle> {
        self.read(|cx| cx.windows())
    }
}

impl Default for TestApp {
    fn default() -> Self {
        Self::new()
    }
}

/// A window opened by [`TestApp::open_window`].
///
/// Use it to read and update the root view and to send input to the window.
/// Clones refer to the same window.
///
/// Each method that sends input or updates the view runs pending tasks before it
/// returns, so the next line can assert on the result. The simulated input
/// methods use the window's last painted frame to find elements, so call
/// [`TestAppWindow::draw`] after building or changing the view.
pub struct TestAppWindow<V> {
    handle: WindowHandle<V>,
    app: Rc<AppCell>,
    platform: Rc<TestPlatform>,
    background_executor: BackgroundExecutor,
}

impl<V: 'static + Render> TestAppWindow<V> {
    /// Returns the window's handle, for APIs that take a [`WindowHandle`].
    pub fn handle(&self) -> WindowHandle<V> {
        self.handle
    }

    /// Returns the root view's entity.
    ///
    /// # Panics
    ///
    /// Panics if the window has been closed.
    pub fn root(&self) -> Entity<V> {
        let mut app = self.app.borrow_mut();
        let any_handle: AnyWindowHandle = self.handle.into();
        app.update_window(any_handle, |root_view, _, _| {
            root_view.downcast::<V>().expect("root view type mismatch")
        })
        .expect("window not found")
    }

    /// Runs `f` with the root view, its [`Window`] and a [`Context`], then runs pending tasks until parked.
    ///
    /// # Panics
    ///
    /// Panics if the window has been closed.
    pub fn update<R>(&mut self, f: impl FnOnce(&mut V, &mut Window, &mut Context<V>) -> R) -> R {
        let result = {
            let mut app = self.app.borrow_mut();
            let any_handle: AnyWindowHandle = self.handle.into();
            app.update_window(any_handle, |root_view, window, cx| {
                let view = root_view.downcast::<V>().expect("root view type mismatch");
                view.update(cx, |view, cx| f(view, window, cx))
            })
            .expect("window not found")
        };
        self.background_executor.run_until_parked();
        result
    }

    /// Runs `f` with the root view and the [`App`], without running pending tasks.
    ///
    /// # Panics
    ///
    /// Panics if the window has been closed.
    pub fn read<R>(&self, f: impl FnOnce(&V, &App) -> R) -> R {
        let app = self.app.borrow();
        let view = self
            .app
            .borrow()
            .windows
            .get(self.handle.window_id())
            .and_then(|w| w.as_ref())
            .and_then(|w| w.root.clone())
            .and_then(|r| r.downcast::<V>().ok())
            .expect("window or root view not found");
        f(view.read(&app), &app)
    }

    /// Always returns `None`. The window title isn't available through this type yet.
    ///
    /// Don't use this to check a title. Tests that open windows with
    /// [`TestAppContext::add_window_view`](crate::TestAppContext::add_window_view) can read the title with
    /// [`VisualTestContext::window_title`](crate::VisualTestContext::window_title).
    ///
    /// # Panics
    ///
    /// Panics if the window has been closed.
    ///
    /// # Example
    ///
    /// ```
    /// # use gpui::{Empty, TestApp};
    /// let mut app = TestApp::new();
    /// let window = app.open_window(|_, _| Empty);
    /// assert_eq!(window.title(), None);
    /// ```
    pub fn title(&self) -> Option<String> {
        let app = self.app.borrow();
        app.read_window(&self.handle, |_, _cx| {
            // TODO: expose title through Window API
            None
        })
        .unwrap()
    }

    /// Sends one keystroke, such as `"ctrl-s"` or `"space"`, to the focused element, then runs pending tasks.
    ///
    /// Use the key name `"space"` for the space bar. A space character in the string
    /// isn't a separator here; [`TestAppWindow::simulate_keystrokes`] splits on spaces.
    ///
    /// # Panics
    ///
    /// Panics if `keystroke` isn't a valid keystroke. See [keyboard input](crate::_documentation::guides::keyboard)
    /// for the syntax.
    ///
    /// # Example
    ///
    /// ```
    /// # use gpui::{Context, FocusHandle, KeyDownEvent, Render, TestApp, Window, div, prelude::*};
    /// # struct Keys { focus: FocusHandle, last: Option<String> }
    /// # impl Render for Keys {
    /// #     fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    /// #         div().track_focus(&self.focus).on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
    /// #             this.last = Some(event.keystroke.key.clone());
    /// #             cx.notify();
    /// #         }))
    /// #     }
    /// # }
    /// # let mut app = TestApp::new();
    /// # let mut window = app.open_window(|window, cx| {
    /// #     let focus = cx.focus_handle();
    /// #     window.focus(&focus, cx);
    /// #     Keys { focus, last: None }
    /// # });
    /// window.simulate_keystroke("space");
    /// window.read(|keys, _| assert_eq!(keys.last.as_deref(), Some("space")));
    /// ```
    pub fn simulate_keystroke(&mut self, keystroke: &str) {
        let keystroke = Keystroke::parse(keystroke).unwrap();
        {
            let mut app = self.app.borrow_mut();
            let any_handle: AnyWindowHandle = self.handle.into();
            app.update_window(any_handle, |_, window, cx| {
                window.dispatch_keystroke(keystroke, cx);
            })
            .unwrap();
        }
        self.background_executor.run_until_parked();
    }

    /// Sends each keystroke in a space-separated list, in order, then runs pending tasks after each one.
    ///
    /// Each part is parsed as in [`TestAppWindow::simulate_keystroke`]. A space
    /// character in the list is a separator, so use `simulate_keystroke("space")` to
    /// press the space bar. Two spaces in a row send an empty keystroke.
    ///
    /// # Panics
    ///
    /// Panics if any part isn't a valid keystroke.
    ///
    /// # Example
    ///
    /// ```
    /// # use gpui::{Context, FocusHandle, KeyDownEvent, Render, TestApp, Window, div, prelude::*};
    /// # struct Keys { focus: FocusHandle, last: Option<String> }
    /// # impl Render for Keys {
    /// #     fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    /// #         div().track_focus(&self.focus).on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
    /// #             this.last = Some(event.keystroke.key.clone());
    /// #             cx.notify();
    /// #         }))
    /// #     }
    /// # }
    /// # let mut app = TestApp::new();
    /// # let mut window = app.open_window(|window, cx| {
    /// #     let focus = cx.focus_handle();
    /// #     window.focus(&focus, cx);
    /// #     Keys { focus, last: None }
    /// # });
    /// window.simulate_keystrokes("a b");
    /// window.read(|keys, _| assert_eq!(keys.last.as_deref(), Some("b")));
    /// ```
    pub fn simulate_keystrokes(&mut self, keystrokes: &str) {
        for keystroke in keystrokes.split(' ') {
            self.simulate_keystroke(keystroke);
        }
    }

    /// Types `input` by sending one keystroke per character, then runs pending tasks.
    ///
    /// Each character is sent as a keystroke whose key is that character, so text
    /// inputs receive the text. Uppercase letters are sent with shift held. A space
    /// is sent with the key `" "`, not `"space"`, so a binding for `space` doesn't
    /// match it. Use [`TestAppWindow::simulate_keystroke`] for named keys.
    ///
    /// # Example
    ///
    /// ```
    /// # use gpui::{Context, FocusHandle, KeyDownEvent, Render, TestApp, Window, div, prelude::*};
    /// # struct Keys { focus: FocusHandle, last: Option<String> }
    /// # impl Render for Keys {
    /// #     fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    /// #         div().track_focus(&self.focus).on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
    /// #             this.last = Some(event.keystroke.key.clone());
    /// #             cx.notify();
    /// #         }))
    /// #     }
    /// # }
    /// # let mut app = TestApp::new();
    /// # let mut window = app.open_window(|window, cx| {
    /// #     let focus = cx.focus_handle();
    /// #     window.focus(&focus, cx);
    /// #     Keys { focus, last: None }
    /// # });
    /// window.simulate_input("xy");
    /// window.read(|keys, _| assert_eq!(keys.last.as_deref(), Some("y")));
    /// ```
    pub fn simulate_input(&mut self, input: &str) {
        for char in input.chars() {
            self.simulate_keystroke(&char.to_string());
        }
    }

    /// Moves the pointer to `position` with no button held, then runs pending tasks.
    pub fn simulate_mouse_move(&mut self, position: Point<Pixels>) {
        self.simulate_event(MouseMoveEvent {
            position,
            modifiers: Default::default(),
            pressed_button: None,
        });
    }

    /// Presses `button` at `position`, then runs pending tasks.
    ///
    /// The click count is always `1`, so this never sends a double click.
    pub fn simulate_mouse_down(&mut self, position: Point<Pixels>, button: MouseButton) {
        self.simulate_event(MouseDownEvent {
            position,
            button,
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        });
    }

    /// Releases `button` at `position`, then runs pending tasks.
    ///
    /// The click count is always `1`.
    pub fn simulate_mouse_up(&mut self, position: Point<Pixels>, button: MouseButton) {
        self.simulate_event(MouseUpEvent {
            position,
            button,
            modifiers: Default::default(),
            click_count: 1,
        });
    }

    /// Presses and releases `button` at `position`, then runs pending tasks.
    ///
    /// Both events have a click count of `1`, so calling this twice in a row sends two
    /// single clicks, never a double click. Call [`TestAppWindow::draw`] first, so
    /// the click hits the element you expect. See the example on [`TestApp`].
    pub fn simulate_click(&mut self, position: Point<Pixels>, button: MouseButton) {
        self.simulate_mouse_down(position, button);
        self.simulate_mouse_up(position, button);
    }

    /// Scrolls by `delta`, in pixels, with the pointer at `position`, then runs pending tasks.
    ///
    /// No modifier keys are held.
    pub fn simulate_scroll(&mut self, position: Point<Pixels>, delta: Point<Pixels>) {
        self.simulate_event(crate::ScrollWheelEvent {
            position,
            delta: crate::ScrollDelta::Pixels(delta),
            modifiers: Default::default(),
            touch_phase: crate::TouchPhase::Moved,
        });
    }

    /// Dispatches `event` to the window as platform input, then runs pending tasks.
    ///
    /// Use this for input events that have no dedicated method on [`TestAppWindow`].
    pub fn simulate_event<E: InputEvent>(&mut self, event: E) {
        let platform_input = event.to_platform_input();
        {
            let mut app = self.app.borrow_mut();
            let any_handle: AnyWindowHandle = self.handle.into();
            app.update_window(any_handle, |_, window, cx| {
                window.dispatch_event(platform_input, cx);
            })
            .unwrap();
        }
        self.background_executor.run_until_parked();
    }

    /// Sets the window's size to `size`, then runs pending tasks.
    ///
    /// This runs the window's resize callback, if it has one, and updates the viewport that
    /// `Window::viewport_size` reports. It doesn't paint the window: call
    /// [`TestAppWindow::draw`] to paint at the new size.
    pub fn simulate_resize(&mut self, size: Size<Pixels>) {
        let window_id = self.handle.window_id();
        // The window's resize callback updates the app, so it can't run while the app is borrowed.
        let test_window = {
            let mut app = self.app.borrow_mut();
            match app.windows.get_mut(window_id) {
                Some(Some(window)) => window.platform_window.as_test().cloned(),
                _ => None,
            }
        };
        if let Some(mut test_window) = test_window {
            test_window.simulate_resize(size);
        }
        self.background_executor.run_until_parked();
    }

    /// Makes this the active window, as if the user had clicked it, then runs pending tasks.
    ///
    /// Windows that [`TestApp`] opens start inactive. Focus listeners such as
    /// [`Context::on_focus`] only run in the active window, so activate it before testing them.
    pub fn activate(&mut self) {
        let window_id = self.handle.window_id();
        let test_window = match self.app.borrow_mut().windows.get_mut(window_id) {
            Some(Some(window)) => window.platform_window.as_test().cloned(),
            _ => None,
        };
        self.platform.set_active_window(test_window);
        self.background_executor.run_until_parked();
    }

    /// Paints the window now, so that later input and hit tests use the current layout.
    ///
    /// Call this after building the view and after anything changes its layout, before
    /// simulating a click at a position. It doesn't run pending tasks. See the example
    /// on [`TestApp`].
    pub fn draw(&mut self) {
        let mut app = self.app.borrow_mut();
        let any_handle: AnyWindowHandle = self.handle.into();
        app.update_window(any_handle, |_, window, cx| {
            window.draw(cx).clear(cx);
        })
        .unwrap();
    }
}

impl<V> Clone for TestAppWindow<V> {
    fn clone(&self) -> Self {
        Self {
            handle: self.handle,
            app: self.app.clone(),
            platform: self.platform.clone(),
            background_executor: self.background_executor.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FocusHandle, Focusable, div, prelude::*, px, size};

    struct Counter {
        count: usize,
        focus_handle: FocusHandle,
    }

    impl Counter {
        fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
            let focus_handle = cx.focus_handle();
            Self {
                count: 0,
                focus_handle,
            }
        }

        fn increment(&mut self, _cx: &mut Context<Self>) {
            self.count += 1;
        }
    }

    impl Focusable for Counter {
        fn focus_handle(&self, _cx: &App) -> FocusHandle {
            self.focus_handle.clone()
        }
    }

    impl Render for Counter {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().child(format!("Count: {}", self.count))
        }
    }

    #[test]
    fn test_basic_usage() {
        let mut app = TestApp::new();

        let mut window = app.open_window(Counter::new);

        window.update(|counter, _window, cx| {
            counter.increment(cx);
        });

        window.read(|counter, _| {
            assert_eq!(counter.count, 1);
        });

        drop(window);
        app.update(|cx| cx.shutdown());
    }

    #[test]
    fn test_resize_updates_gpui_viewport() {
        let mut app = TestApp::new();
        let mut window = app.open_window(Counter::new);

        window.simulate_resize(size(px(321.), px(123.)));

        window.update(|_, window, _| {
            assert_eq!(window.viewport_size(), size(px(321.), px(123.)));
        });
    }

    #[test]
    fn test_entity_creation() {
        let mut app = TestApp::new();

        let entity = app.new_entity(|cx| Counter {
            count: 42,
            focus_handle: cx.focus_handle(),
        });

        app.read_entity(&entity, |counter, _| {
            assert_eq!(counter.count, 42);
        });

        app.update_entity(&entity, |counter, _cx| {
            counter.count += 1;
        });

        app.read_entity(&entity, |counter, _| {
            assert_eq!(counter.count, 43);
        });
    }

    #[test]
    fn test_globals() {
        let mut app = TestApp::new();

        struct MyGlobal(String);
        impl Global for MyGlobal {}

        assert!(!app.has_global::<MyGlobal>());

        app.set_global(MyGlobal("hello".into()));

        assert!(app.has_global::<MyGlobal>());

        app.read_global::<MyGlobal, _>(|global, _| {
            assert_eq!(global.0, "hello");
        });

        app.update_global::<MyGlobal, _>(|global, _| {
            global.0 = "world".into();
        });

        app.read_global::<MyGlobal, _>(|global, _| {
            assert_eq!(global.0, "world");
        });
    }
}
