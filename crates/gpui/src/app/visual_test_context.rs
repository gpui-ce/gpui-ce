use crate::{
    Action, AnyView, AnyWindowHandle, App, AppCell, AppContext, AssetSource, BackgroundExecutor,
    Bounds, ClipboardItem, Context, Entity, EntityId, EventEmitter, ForegroundExecutor, Global,
    InputEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Platform, Point, Render, Result, Size, Task, TestDispatcher, TextSystem,
    VisualTestPlatform, Window, WindowBounds, WindowHandle, WindowOptions, app::GpuiMode,
};
use anyhow::anyhow;
use image::RgbaImage;
use std::{future::Future, rc::Rc, sync::Arc, time::Duration};

/// A test context that renders with the real macOS platform, for screenshot tests.
///
/// **This type is macOS-only.** It exists only on macOS with the `test-support`
/// feature. Its name is `VisualTestAppContext`, not `VisualTestContext`: the
/// latter is the window context for [`TestAppContext`](crate::TestAppContext),
/// which mocks rendering.
///
/// The two differ in these ways:
///
/// - [`TestAppContext`](crate::TestAppContext) runs on `TestPlatform`, which mocks
///   rendering. `VisualTestAppContext` runs on the real macOS platform, so
///   [`VisualTestAppContext::capture_screenshot`] returns the pixels a user would see.
/// - Windows are opened off-screen by [`VisualTestAppContext::open_offscreen_window`],
///   so they don't appear on the display but are still composited.
/// - Input methods take the window as a parameter, not through a
///   `VisualTestContext`. `simulate_keystrokes` splits on whitespace, and
///   `simulate_input` sends a space as the space character.
/// - Tasks are scheduled deterministically, as in `TestAppContext`, and
///   [`VisualTestAppContext::advance_clock`] moves the simulated clock.
///
/// Use `TestAppContext` for behavior tests and this type only when the test must
/// inspect rendered output.
#[derive(Clone)]
pub struct VisualTestAppContext {
    /// The underlying app cell
    pub app: Rc<AppCell>,
    /// The background executor for running async tasks
    pub background_executor: BackgroundExecutor,
    /// The foreground executor for running tasks on the main thread
    pub foreground_executor: ForegroundExecutor,
    /// The test dispatcher for deterministic task scheduling
    dispatcher: TestDispatcher,
    platform: Rc<dyn Platform>,
    text_system: Arc<TextSystem>,
}

impl VisualTestAppContext {
    /// Creates a context on `platform` with the real rendering and deterministic task scheduling.
    ///
    /// The scheduler seed is read from the `SEED` environment variable, and is `0`
    /// when it's unset or isn't a number.
    ///
    /// This uses an empty asset source, so SVG icons don't render. Use
    /// [`VisualTestAppContext::with_asset_source`] to load real assets.
    pub fn new(platform: Rc<dyn Platform>) -> Self {
        Self::with_asset_source(platform, Arc::new(()))
    }

    /// Creates a context on `platform` that loads assets from `asset_source`.
    ///
    /// Pass the application's real asset source so that SVG icons render in screenshots.
    pub fn with_asset_source(
        platform: Rc<dyn Platform>,
        asset_source: Arc<dyn AssetSource>,
    ) -> Self {
        // Use a seeded RNG for deterministic behavior
        let seed = std::env::var("SEED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        // Create a visual test platform that combines real Mac rendering
        // with controllable TestDispatcher for deterministic task scheduling
        let platform = Rc::new(VisualTestPlatform::new(platform, seed));

        // Get the dispatcher and executors from the platform
        let dispatcher = platform.dispatcher().clone();
        let background_executor = platform.background_executor();
        let foreground_executor = platform.foreground_executor();

        let text_system = Arc::new(TextSystem::new(platform.text_system()));

        let http_client = crate::http_client::FakeHttpClient::with_404_response();

        let mut app = App::new_app(
            platform.clone(),
            gpui::AssetRegistry::from(asset_source).into(),
            http_client,
        );
        app.borrow_mut().mode = GpuiMode::test();

        Self {
            app,
            background_executor,
            foreground_executor,
            dispatcher,
            platform,
            text_system,
        }
    }

    /// Opens a window of `size` at the origin `(-10000, -10000)`, off every display.
    ///
    /// The window is shown and composited, so [`VisualTestAppContext::capture_screenshot`]
    /// can capture it. It doesn't take focus.
    ///
    /// # Errors
    ///
    /// Returns an error if the window can't be opened.
    pub fn open_offscreen_window<V: Render + 'static>(
        &mut self,
        size: Size<Pixels>,
        build_root: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
    ) -> Result<WindowHandle<V>> {
        use crate::{point, px};

        let bounds = Bounds {
            origin: point(px(-10000.0), px(-10000.0)),
            size,
        };

        let mut cx = self.app.borrow_mut();
        cx.open_window(
            WindowOptions::new()
                .window_bounds(Some(WindowBounds::Windowed(bounds)))
                .focus(false)
                .show(true),
            build_root,
        )
    }

    /// Opens an off-screen window of 1280 by 800 pixels.
    ///
    /// See [`VisualTestAppContext::open_offscreen_window`].
    pub fn open_offscreen_window_default<V: Render + 'static>(
        &mut self,
        build_root: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
    ) -> Result<WindowHandle<V>> {
        use crate::{px, size};
        self.open_offscreen_window(size(px(1280.0), px(800.0)), build_root)
    }

    /// Returns whether this platform can capture the screen.
    pub fn is_screen_capture_supported(&self) -> bool {
        self.platform.is_screen_capture_supported()
    }

    /// Returns the text system.
    pub fn text_system(&self) -> &Arc<TextSystem> {
        &self.text_system
    }

    /// Returns a clone of the background executor.
    pub fn executor(&self) -> BackgroundExecutor {
        self.background_executor.clone()
    }

    /// Returns a clone of the foreground executor.
    pub fn foreground_executor(&self) -> ForegroundExecutor {
        self.foreground_executor.clone()
    }

    /// Runs pending foreground and background tasks until none can make progress.
    ///
    /// This doesn't move the simulated clock. Use [`VisualTestAppContext::advance_clock`] for that.
    pub fn run_until_parked(&self) {
        self.dispatcher.run_until_parked();
    }

    /// Moves the simulated clock forward by `duration`, and runs the tasks that become ready.
    ///
    /// Use this for time-based behavior such as tooltip delays.
    pub fn advance_clock(&self, duration: Duration) {
        self.dispatcher.advance_clock(duration);
    }

    /// Runs `f` with mutable access to the [`App`]. This doesn't run pending tasks.
    pub fn update<R>(&mut self, f: impl FnOnce(&mut App) -> R) -> R {
        let mut app = self.app.borrow_mut();
        f(&mut app)
    }

    /// Runs `f` with read-only access to the [`App`].
    pub fn read<R>(&self, f: impl FnOnce(&App) -> R) -> R {
        let app = self.app.borrow();
        f(&app)
    }

    /// Runs `f` with the window's root view, the [`Window`] and the [`App`].
    ///
    /// # Errors
    ///
    /// Returns an error if `window` has been closed.
    pub fn update_window<T, F>(&mut self, window: AnyWindowHandle, f: F) -> Result<T>
    where
        F: FnOnce(AnyView, &mut Window, &mut App) -> T,
    {
        let mut lock = self.app.borrow_mut();
        lock.update_window(window, f)
    }

    /// Spawns a future on the foreground executor. It runs when tasks run.
    pub fn spawn<F, R>(&self, f: F) -> Task<R>
    where
        F: Future<Output = R> + 'static,
        R: 'static,
    {
        self.foreground_executor.spawn(f)
    }

    /// Returns whether a global of type `G` is set.
    pub fn has_global<G: Global>(&self) -> bool {
        let app = self.app.borrow();
        app.has_global::<G>()
    }

    /// Runs `f` with the global of type `G`.
    ///
    /// # Panics
    ///
    /// Panics if no global of type `G` is set.
    pub fn read_global<G: Global, R>(&self, f: impl FnOnce(&G, &App) -> R) -> R {
        let app = self.app.borrow();
        f(app.global::<G>(), &app)
    }

    /// Sets the global of type `G`, replacing any previous value.
    pub fn set_global<G: Global>(&mut self, global: G) {
        let mut app = self.app.borrow_mut();
        app.set_global(global);
    }

    /// Runs `f` with mutable access to the global of type `G`.
    ///
    /// # Panics
    ///
    /// Panics if no global of type `G` is set.
    pub fn update_global<G: Global, R>(&mut self, f: impl FnOnce(&mut G, &mut App) -> R) -> R {
        let mut lock = self.app.borrow_mut();
        lock.update(|cx| {
            let mut global = cx.lease_global::<G>();
            let result = f(&mut global, cx);
            cx.end_global_lease(global);
            result
        })
    }

    /// Sends each keystroke in a whitespace-separated list to `window`, then runs pending tasks.
    ///
    /// Unlike [`TestAppContext::simulate_keystrokes`](crate::TestAppContext::simulate_keystrokes),
    /// this splits on any whitespace, so a tab or newline separates keystrokes too.
    ///
    /// # Panics
    ///
    /// Panics if any part isn't a valid keystroke.
    pub fn simulate_keystrokes(&mut self, window: AnyWindowHandle, keystrokes: &str) {
        for keystroke_text in keystrokes.split_whitespace() {
            let keystroke = Keystroke::parse(keystroke_text)
                .unwrap_or_else(|_| panic!("Invalid keystroke: {}", keystroke_text));
            self.dispatch_keystroke(window, keystroke);
        }
        self.run_until_parked();
    }

    /// Dispatches one keystroke to the focused element in `window`.
    ///
    /// If the window is closed, the keystroke is dropped silently. This doesn't run pending tasks.
    pub fn dispatch_keystroke(&mut self, window: AnyWindowHandle, keystroke: Keystroke) {
        self.update_window(window, |_, window, cx| {
            window.dispatch_keystroke(keystroke, cx);
        })
        .ok();
    }

    /// Types `input` into `window`, one keystroke per character, then runs pending tasks.
    ///
    /// Each keystroke carries its character as the key and as the text, so a space
    /// typed this way reaches text inputs as a space.
    pub fn simulate_input(&mut self, window: AnyWindowHandle, input: &str) {
        for char in input.chars() {
            let key = char.to_string();
            let keystroke = Keystroke {
                modifiers: Modifiers::default(),
                key: key.clone(),
                key_char: Some(key),
            };
            self.dispatch_keystroke(window, keystroke);
        }
        self.run_until_parked();
    }

    /// Moves the pointer to `position`, with `button` held if it's `Some`.
    pub fn simulate_mouse_move(
        &mut self,
        window: AnyWindowHandle,
        position: Point<Pixels>,
        button: impl Into<Option<MouseButton>>,
        modifiers: Modifiers,
    ) {
        self.simulate_event(
            window,
            MouseMoveEvent {
                position,
                modifiers,
                pressed_button: button.into(),
            },
        );
    }

    /// Presses `button` at `position`. The click count is always `1`.
    pub fn simulate_mouse_down(
        &mut self,
        window: AnyWindowHandle,
        position: Point<Pixels>,
        button: MouseButton,
        modifiers: Modifiers,
    ) {
        self.simulate_event(
            window,
            MouseDownEvent {
                position,
                modifiers,
                button,
                click_count: 1,
                first_mouse: false,
            },
        );
    }

    /// Releases `button` at `position`. The click count is always `1`.
    pub fn simulate_mouse_up(
        &mut self,
        window: AnyWindowHandle,
        position: Point<Pixels>,
        button: MouseButton,
        modifiers: Modifiers,
    ) {
        self.simulate_event(
            window,
            MouseUpEvent {
                position,
                modifiers,
                button,
                click_count: 1,
            },
        );
    }

    /// Presses and releases the left button at `position`, with click count `1` on both events.
    pub fn simulate_click(
        &mut self,
        window: AnyWindowHandle,
        position: Point<Pixels>,
        modifiers: Modifiers,
    ) {
        self.simulate_mouse_down(window, position, MouseButton::Left, modifiers);
        self.simulate_mouse_up(window, position, MouseButton::Left, modifiers);
    }

    /// Dispatches `event` to `window`, then runs pending tasks.
    ///
    /// If the window is closed, the event is dropped silently.
    pub fn simulate_event<E: InputEvent>(&mut self, window: AnyWindowHandle, event: E) {
        self.update_window(window, |_, window, cx| {
            window.dispatch_event(event.to_platform_input(), cx);
        })
        .ok();
        self.run_until_parked();
    }

    /// Dispatches `action` to the focused element in `window`, then runs pending tasks.
    ///
    /// If the window is closed, the action is dropped silently.
    pub fn dispatch_action(&mut self, window: AnyWindowHandle, action: impl Action) {
        self.update_window(window, |_, window, cx| {
            window.dispatch_action(action.boxed_clone(), cx);
        })
        .ok();
        self.run_until_parked();
    }

    /// Writes `item` to the clipboard.
    pub fn write_to_clipboard(&self, item: ClipboardItem) {
        self.platform.write_to_clipboard(item);
    }

    /// Returns the clipboard's current item, or `None`.
    pub fn read_from_clipboard(&self) -> Option<ClipboardItem> {
        self.platform.read_from_clipboard()
    }

    /// Waits until `predicate` returns `true` for `entity`, or until `timeout` passes.
    ///
    /// The predicate is checked between polls, which are 10 milliseconds apart. The
    /// timeout is measured in real time, not simulated time.
    ///
    /// # Errors
    ///
    /// Returns an error if `timeout` passes before the predicate is true.
    pub async fn wait_for<T: 'static>(
        &mut self,
        entity: &Entity<T>,
        predicate: impl Fn(&T) -> bool,
        timeout: Duration,
    ) -> Result<()> {
        let start = web_time::Instant::now();
        loop {
            {
                let app = self.app.borrow();
                if predicate(entity.read(&app)) {
                    return Ok(());
                }
            }

            if start.elapsed() > timeout {
                return Err(anyhow!("Timed out waiting for condition"));
            }

            self.run_until_parked();
            self.background_executor
                .timer(Duration::from_millis(10))
                .await;
        }
    }

    /// Renders `window` to an image and returns it.
    ///
    /// This renders to a texture and reads the pixels back, so the window doesn't need
    /// to be visible on screen.
    ///
    /// # Errors
    ///
    /// Returns an error if the window has been closed or can't be rendered.
    #[cfg(any(test, feature = "test-support"))]
    pub fn capture_screenshot(&mut self, window: AnyWindowHandle) -> Result<RgbaImage> {
        self.update_window(window, |_, window, _cx| window.render_to_image())?
    }

    /// Waits for about two frames (32 milliseconds of simulated time), then runs pending tasks.
    pub async fn wait_for_animations(&self) {
        self.background_executor
            .timer(Duration::from_millis(32))
            .await;
        self.run_until_parked();
    }
}

impl AppContext for VisualTestAppContext {
    fn new<T: 'static>(&mut self, build_entity: impl FnOnce(&mut Context<T>) -> T) -> Entity<T> {
        let mut app = self.app.borrow_mut();
        app.new(build_entity)
    }

    fn reserve_entity<T: 'static>(&mut self) -> crate::Reservation<T> {
        let mut app = self.app.borrow_mut();
        app.reserve_entity()
    }

    fn insert_entity<T: 'static>(
        &mut self,
        reservation: crate::Reservation<T>,
        build_entity: impl FnOnce(&mut Context<T>) -> T,
    ) -> Entity<T> {
        let mut app = self.app.borrow_mut();
        app.insert_entity(reservation, build_entity)
    }

    fn update_entity<T: 'static, R>(
        &mut self,
        handle: &Entity<T>,
        update: impl FnOnce(&mut T, &mut Context<T>) -> R,
    ) -> R {
        let mut app = self.app.borrow_mut();
        app.update_entity(handle, update)
    }

    fn as_mut<'a, T>(&'a mut self, _: &Entity<T>) -> crate::GpuiBorrow<'a, T>
    where
        T: 'static,
    {
        panic!("Cannot use as_mut with a visual test app context. Try calling update() first")
    }

    fn read_entity<T, R>(&self, handle: &Entity<T>, read: impl FnOnce(&T, &App) -> R) -> R
    where
        T: 'static,
    {
        let app = self.app.borrow();
        app.read_entity(handle, read)
    }

    fn notify(&mut self, entity_id: EntityId) {
        let mut app = self.app.borrow_mut();
        app.notify(entity_id)
    }

    fn emit<EntityType, EventType>(&mut self, entity: &Entity<EntityType>, event: EventType)
    where
        EntityType: EventEmitter<EventType>,
        EventType: 'static,
    {
        let mut app = self.app.borrow_mut();
        app.emit(entity, event)
    }

    fn update_window<T, F>(&mut self, window: AnyWindowHandle, f: F) -> Result<T>
    where
        F: FnOnce(AnyView, &mut Window, &mut App) -> T,
    {
        let mut lock = self.app.borrow_mut();
        lock.update_window(window, f)
    }

    fn with_window<R>(
        &mut self,
        entity_id: EntityId,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> Option<R> {
        let mut lock = self.app.borrow_mut();
        lock.with_window(entity_id, f)
    }

    fn read_window<T, R>(
        &self,
        window: &WindowHandle<T>,
        read: impl FnOnce(Entity<T>, &App) -> R,
    ) -> Result<R>
    where
        T: 'static,
    {
        let app = self.app.borrow();
        app.read_window(window, read)
    }

    fn background_spawn<R>(&self, future: impl Future<Output = R> + Send + 'static) -> Task<R>
    where
        R: Send + 'static,
    {
        self.background_executor.spawn(future)
    }

    fn read_global<G, R>(&self, callback: impl FnOnce(&G, &App) -> R) -> R
    where
        G: Global,
    {
        let app = self.app.borrow();
        callback(app.global::<G>(), &app)
    }

    fn insert_global_entity<E: 'static>(&mut self, entity: Entity<E>) {
        let mut app = self.app.borrow_mut();
        app.insert_global_entity(entity)
    }

    fn remove_global_entity<E: 'static>(&mut self, entity: &Entity<E>) {
        let mut app = self.app.borrow_mut();
        app.remove_global_entity(entity)
    }

    fn global_entities<E: 'static>(&self) -> impl Iterator<Item = Entity<E>> {
        let app = self.app.borrow();
        let iter = app.global_entities();
        // Cloning the iterator here so that lock can be released when function is going out of scope
        iter.collect::<Vec<_>>().into_iter()
    }
}
