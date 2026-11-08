use crate::{
    Action, AnyView, AnyWindowHandle, App, AppCell, AppContext, AssetRegistry, AsyncApp,
    AvailableSpace, BackgroundExecutor, BorrowAppContext, Bounds, Capslock, ClipboardItem,
    DrawPhase, Drawable, Element, Empty, EntityId, EventEmitter, ForegroundExecutor, Global,
    InputEvent, Keystroke, Modifiers, ModifiersChangedEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Platform, Point, Render, Result, SharedString, Size,
    SystemNotification, SystemNotificationResponse, Task, TestDispatcher, TestPlatform,
    TestScreenCaptureSource, TestWindow, TextSystem, VisualContext, Window, WindowBounds,
    WindowHandle, WindowOptions, app::GpuiMode, window::ElementArenaScope,
};
use anyhow::{anyhow, bail};
use futures::{Stream, StreamExt, channel::oneshot};

use std::{
    cell::RefCell, future::Future, ops::Deref, path::PathBuf, rc::Rc, sync::Arc, time::Duration,
};

/// The test version of [`App`], passed to `#[gpui::test]` functions.
///
/// `TestAppContext` implements [`AppContext`] and adds helpers for windows, input,
/// prompts and executors. Unlike [`TestApp`](crate::TestApp), its methods don't
/// run pending tasks unless the method says so. Call [`TestAppContext::run_until_parked`]
/// to run them. See the [testing guide](crate::_documentation::guides::testing).
///
/// Create one directly with [`TestAppContext::single`] when you don't need the
/// macro.
#[derive(Clone)]
pub struct TestAppContext {
    #[doc(hidden)]
    pub background_executor: BackgroundExecutor,
    #[doc(hidden)]
    pub foreground_executor: ForegroundExecutor,
    #[doc(hidden)]
    pub dispatcher: TestDispatcher,
    test_platform: Rc<TestPlatform>,
    text_system: Arc<TextSystem>,
    fn_name: Option<&'static str>,
    on_quit: Rc<RefCell<Vec<Box<dyn FnOnce() + 'static>>>>,
    #[doc(hidden)]
    pub app: Rc<AppCell>,
}

impl AppContext for TestAppContext {
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

    fn as_mut<'a, T>(&'a mut self, _: &Entity<T>) -> super::GpuiBorrow<'a, T>
    where
        T: 'static,
    {
        panic!("Cannot use as_mut with a test app context. Try calling update() first")
    }

    fn read_entity<T, R>(&self, handle: &Entity<T>, read: impl FnOnce(&T, &App) -> R) -> R
    where
        T: 'static,
    {
        let app = self.app.borrow();
        app.read_entity(handle, read)
    }

    fn notify(&mut self, entity_id: EntityId) {
        let mut lock = self.app.borrow_mut();
        lock.notify(entity_id)
    }

    fn emit<EntityType, EventType>(&mut self, entity: &Entity<EntityType>, event: EventType)
    where
        EntityType: EventEmitter<EventType>,
        EventType: 'static,
    {
        let mut lock = self.app.borrow_mut();
        lock.emit(entity, event)
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
        app.read_global(callback)
    }

    fn insert_global_entity<T: 'static>(&mut self, entity: Entity<T>) {
        let mut lock = self.app.borrow_mut();
        lock.insert_global_entity(entity)
    }

    fn remove_global_entity<T: 'static>(&mut self, entity: &Entity<T>) {
        let mut lock = self.app.borrow_mut();
        lock.remove_global_entity(entity)
    }

    fn global_entities<T: 'static>(&self) -> impl Iterator<Item = Entity<T>> {
        let lock = self.app.borrow();
        let iter = lock.global_entities();
        // Cloning the iterator here so that lock can be released when function is going out of scope
        iter.collect::<Vec<_>>().into_iter()
    }
}

impl TestAppContext {
    /// Creates a context that runs on `dispatcher`.
    ///
    /// `#[gpui::test]` calls this for you.
    pub fn build(dispatcher: TestDispatcher, fn_name: Option<&'static str>) -> Self {
        let arc_dispatcher = Arc::new(dispatcher.clone());
        let background_executor = BackgroundExecutor::new(arc_dispatcher.clone());
        let foreground_executor = ForegroundExecutor::new(arc_dispatcher);
        let platform = TestPlatform::new(background_executor.clone(), foreground_executor.clone());
        let http_client = crate::http_client::FakeHttpClient::with_404_response();
        let text_system = Arc::new(TextSystem::new(platform.text_system()));

        let app = App::new_app(
            platform.clone(),
            AssetRegistry::default().into(),
            http_client,
        );
        app.borrow_mut().mode = GpuiMode::test();

        Self {
            app,
            background_executor,
            foreground_executor,
            dispatcher,
            test_platform: platform,
            text_system,
            fn_name,
            on_quit: Rc::new(RefCell::new(Vec::default())),
        }
    }

    /// Turns off drawing for this app for the rest of the test.
    pub fn skip_drawing(&mut self) {
        self.app.borrow_mut().mode = GpuiMode::Test { skip_drawing: true };
    }

    /// Creates a context with seed `0` that isn't tied to a test function.
    ///
    /// Use this in doctests and in code that runs outside `#[gpui::test]`.
    pub fn single() -> Self {
        let dispatcher = TestDispatcher::new(0);
        Self::build(dispatcher, None)
    }

    /// Returns the name of the test function that created this context, if known.
    pub fn test_function_name(&self) -> Option<&'static str> {
        self.fn_name
    }

    /// Returns whether a new-path prompt is waiting for [`TestAppContext::simulate_new_path_selection`].
    pub fn did_prompt_for_new_path(&self) -> bool {
        self.test_platform.did_prompt_for_new_path()
    }

    /// Creates another app that shares this context's scheduler, so tasks from both apps interleave deterministically.
    ///
    /// The new app has its own state, windows and globals.
    pub fn new_app(&self) -> TestAppContext {
        Self::build(self.dispatcher.clone(), self.fn_name)
    }

    /// Runs the cleanup registered with [`TestAppContext::on_quit`], then shuts the app down.
    ///
    /// `#[gpui::test]` calls this when the test ends.
    pub fn quit(&self) {
        self.on_quit.borrow_mut().drain(..).for_each(|f| f());
        self.app.borrow_mut().shutdown();
    }

    /// Registers `f` to run when the test ends, before the app shuts down.
    pub fn on_quit(&mut self, f: impl FnOnce() + 'static) {
        self.on_quit.borrow_mut().push(Box::new(f));
    }

    /// Marks every window for redraw on the next effect cycle. Always returns `Ok(())`.
    pub fn refresh(&mut self) -> Result<()> {
        let mut app = self.app.borrow_mut();
        app.refresh_windows();
        Ok(())
    }

    /// Returns a clone of the background executor.
    pub fn executor(&self) -> BackgroundExecutor {
        self.background_executor.clone()
    }

    /// Returns the foreground executor.
    pub fn foreground_executor(&self) -> &ForegroundExecutor {
        &self.foreground_executor
    }

    /// Runs `f` with mutable access to the [`App`].
    ///
    /// Unlike [`TestApp::update`](crate::TestApp::update), this doesn't run pending
    /// tasks. Call [`TestAppContext::run_until_parked`] afterward if you need them.
    ///
    /// # Example
    ///
    /// ```
    /// # use gpui::TestAppContext;
    /// # use std::{cell::Cell, rc::Rc};
    /// let cx = TestAppContext::single();
    /// let finished = Rc::new(Cell::new(false));
    ///
    /// let flag = finished.clone();
    /// cx.foreground_executor().spawn(async move { flag.set(true) }).detach();
    ///
    /// cx.update(|_| ());
    /// assert!(!finished.get(), "update doesn't run pending tasks");
    ///
    /// cx.run_until_parked();
    /// assert!(finished.get());
    /// ```
    pub fn update<R>(&self, f: impl FnOnce(&mut App) -> R) -> R {
        let mut cx = self.app.borrow_mut();
        cx.update(f)
    }

    /// Runs `f` with read-only access to the [`App`].
    pub fn read<R>(&self, f: impl FnOnce(&App) -> R) -> R {
        let cx = self.app.borrow();
        f(&cx)
    }

    /// Opens a window with maximized bounds and returns its handle.
    ///
    /// The window is backed by a test platform window, so input and resize helpers
    /// work on it. This doesn't run pending tasks.
    ///
    /// # Panics
    ///
    /// Panics if the window can't be opened.
    pub fn add_window<F, V>(&mut self, build_window: F) -> WindowHandle<V>
    where
        F: FnOnce(&mut Window, &mut Context<V>) -> V,
        V: 'static + Render,
    {
        let mut cx = self.app.borrow_mut();

        // Some tests rely on the window size matching the bounds of the test display
        let bounds = Bounds::maximized(None, &cx);
        cx.open_window(
            WindowOptions::new().window_bounds(Some(WindowBounds::Windowed(bounds))),
            |window, cx| cx.new(|cx| build_window(window, cx)),
        )
        .unwrap()
    }

    /// Opens a window of `window_size` at the origin and returns its handle.
    ///
    /// Use this when a layout test needs a known size. [`TestAppContext::add_window`]
    /// uses the size of the test display instead. This doesn't run pending tasks.
    ///
    /// # Panics
    ///
    /// Panics if the window can't be opened.
    pub fn open_window<F, V>(
        &mut self,
        window_size: Size<Pixels>,
        build_window: F,
    ) -> WindowHandle<V>
    where
        F: FnOnce(&mut Window, &mut Context<V>) -> V,
        V: 'static + Render,
    {
        let mut cx = self.app.borrow_mut();
        cx.open_window(
            WindowOptions::new().window_bounds(Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: window_size,
            }))),
            |window, cx| cx.new(|cx| build_window(window, cx)),
        )
        .unwrap()
    }

    /// Opens a window with an empty root view and returns a [`VisualTestContext`] for it.
    ///
    /// Runs pending tasks before it returns.
    pub fn add_empty_window(&mut self) -> &mut VisualTestContext {
        let mut cx = self.app.borrow_mut();
        let bounds = Bounds::maximized(None, &cx);
        let window = cx
            .open_window(
                WindowOptions::new().window_bounds(Some(WindowBounds::Windowed(bounds))),
                |_, cx| cx.new(|_| Empty),
            )
            .unwrap();
        drop(cx);
        let cx = VisualTestContext::from_window(*window.deref(), self).into_mut();
        cx.run_until_parked();
        cx
    }

    /// Opens a window whose root view `build_root_view` builds.
    ///
    /// Returns the root view's entity and a [`VisualTestContext`] for the window. Use
    /// the returned context from then on, which is the usual pattern:
    /// `let (view, cx) = cx.add_window_view(...);`. Runs pending tasks before it returns.
    /// See [testing a window](crate::_documentation::guides::testing) for an example.
    pub fn add_window_view<F, V>(
        &mut self,
        build_root_view: F,
    ) -> (Entity<V>, &mut VisualTestContext)
    where
        F: FnOnce(&mut Window, &mut Context<V>) -> V,
        V: 'static + Render,
    {
        let mut cx = self.app.borrow_mut();
        let bounds = Bounds::maximized(None, &cx);
        let window = cx
            .open_window(
                WindowOptions::new().window_bounds(Some(WindowBounds::Windowed(bounds))),
                |window, cx| cx.new(|cx| build_root_view(window, cx)),
            )
            .unwrap();
        drop(cx);
        let view = window.root(self).unwrap();
        let cx = VisualTestContext::from_window(*window.deref(), self).into_mut();
        cx.run_until_parked();

        // it might be nice to try and cleanup these at the end of each test.
        (view, cx)
    }

    /// Returns the text system.
    pub fn text_system(&self) -> &Arc<TextSystem> {
        &self.text_system
    }

    /// Returns the cursor most recently applied to the test platform.
    pub fn cursor_style(&self) -> crate::CursorStyle {
        self.test_platform.cursor_style()
    }

    /// Writes `item` to the test clipboard.
    pub fn write_to_clipboard(&self, item: ClipboardItem) {
        self.test_platform.write_to_clipboard(item)
    }

    /// Returns the item most recently written to the test clipboard, or `None`.
    pub fn read_from_clipboard(&self) -> Option<ClipboardItem> {
        self.test_platform.read_from_clipboard()
    }

    /// Answers the oldest pending new-path prompt.
    ///
    /// `select_path` receives the directory the app asked about and returns the chosen
    /// path, or `None` to cancel.
    ///
    /// # Panics
    ///
    /// Panics if no new-path prompt is pending.
    pub fn simulate_new_path_selection(
        &self,
        select_path: impl FnOnce(&std::path::Path) -> Option<std::path::PathBuf>,
    ) {
        self.test_platform.simulate_new_path_selection(select_path);
    }

    /// Answers the oldest pending `prompt_for_paths` request.
    ///
    /// `select_paths` receives the prompt's options and returns the chosen paths, or
    /// `None` to cancel.
    ///
    /// # Panics
    ///
    /// Panics if no paths prompt is pending. Also panics if `select_paths` returns more
    /// than one path for a prompt that doesn't allow multiple selection.
    pub fn simulate_path_prompt_response(
        &self,
        select_paths: impl FnOnce(&crate::PathPromptOptions) -> Option<Vec<std::path::PathBuf>>,
    ) {
        self.test_platform
            .simulate_path_prompt_response(select_paths);
    }

    /// Returns whether a `prompt_for_paths` request is waiting for [`TestAppContext::simulate_path_prompt_response`].
    pub fn did_prompt_for_paths(&self) -> bool {
        self.test_platform.did_prompt_for_paths()
    }

    /// Presses the button labelled `button` on the oldest pending prompt.
    ///
    /// # Panics
    ///
    /// Panics if no prompt is pending, or if no button on the prompt has the label `button`.
    #[track_caller]
    pub fn simulate_prompt_answer(&self, button: &str) {
        self.test_platform.simulate_prompt_answer(button);
    }

    /// Returns whether a prompt is waiting for an answer.
    pub fn has_pending_prompt(&self) -> bool {
        self.test_platform.has_pending_prompt()
    }

    /// Returns the message and detail of the oldest pending prompt, or `None`.
    ///
    /// The detail is an empty string when the prompt has none.
    pub fn pending_prompt(&self) -> Option<(String, String)> {
        self.test_platform.pending_prompt()
    }

    /// Returns the URL most recently passed to `open_url`, or `None`.
    ///
    /// Only the last URL is kept.
    pub fn opened_url(&self) -> Option<String> {
        self.test_platform.opened_url.borrow().clone()
    }

    /// Returns the identifier and name set with `set_app_identity`, or `None` if none was set.
    pub fn app_identity(&self) -> Option<(SharedString, SharedString)> {
        self.test_platform.app_identity()
    }

    /// Returns every system notification shown during the test, in order.
    ///
    /// A notification shown before the app has an identity is dropped and doesn't
    /// appear here.
    pub fn shown_system_notifications(&self) -> Vec<SystemNotification> {
        self.test_platform.shown_system_notifications()
    }

    /// Returns the notifications that are shown and not yet dismissed.
    ///
    /// A notification with the same tag as an earlier one replaces it.
    pub fn delivered_system_notifications(&self) -> Vec<SystemNotification> {
        self.test_platform.delivered_system_notifications()
    }

    /// Returns the tags of the notifications dismissed during the test, in order.
    pub fn dismissed_system_notifications(&self) -> Vec<SharedString> {
        self.test_platform.dismissed_system_notifications()
    }

    /// Delivers `response` to the response handlers, as if the user had activated the notification.
    pub fn simulate_system_notification_response(&self, response: SystemNotificationResponse) {
        self.test_platform
            .simulate_system_notification_response(response);
    }

    /// Resizes `window_handle` to `size`. See [`VisualTestContext::simulate_resize`].
    pub fn simulate_window_resize(&self, window_handle: AnyWindowHandle, size: Size<Pixels>) {
        self.test_window(window_handle).simulate_resize(size);
    }

    /// Returns a receiver that gets the path and arguments of the next restart request.
    ///
    /// The app doesn't restart in a test. Calling this again replaces the previous
    /// sender, so the previous receiver is cancelled.
    pub fn expect_restart(&self) -> oneshot::Receiver<(Option<PathBuf>, Vec<std::ffi::OsString>)> {
        let (tx, rx) = futures::channel::oneshot::channel();
        self.test_platform.expect_restart.borrow_mut().replace(tx);
        rx
    }

    /// Sets the sources that screen capture queries return.
    pub fn set_screen_capture_sources(&self, sources: Vec<TestScreenCaptureSource>) {
        self.test_platform.set_screen_capture_sources(sources);
    }

    /// Returns the handles of all open windows.
    pub fn windows(&self) -> Vec<AnyWindowHandle> {
        self.app.borrow().windows()
    }

    /// Spawns a future on the foreground executor, with an [`AsyncApp`] for this app.
    ///
    /// The future runs when tasks run, such as during [`TestAppContext::run_until_parked`]
    /// or a `simulate_*` method. A plain `update` doesn't run it.
    #[track_caller]
    pub fn spawn<Fut, R>(&self, f: impl FnOnce(AsyncApp) -> Fut) -> Task<R>
    where
        Fut: Future<Output = R> + 'static,
        R: 'static,
    {
        self.foreground_executor.spawn(f(self.to_async()))
    }

    /// Returns whether a global of type `G` is set.
    pub fn has_global<G: Global>(&self) -> bool {
        let app = self.app.borrow();
        app.has_global::<G>()
    }

    /// Runs `read` with the global of type `G`.
    ///
    /// # Panics
    ///
    /// Panics if no global of type `G` is set. Use [`TestAppContext::try_read_global`] when it might not be.
    pub fn read_global<G: Global, R>(&self, read: impl FnOnce(&G, &App) -> R) -> R {
        let app = self.app.borrow();
        read(app.global(), &app)
    }

    /// Runs `read` with the global of type `G` if it is set, and returns `None` otherwise.
    pub fn try_read_global<G: Global, R>(&self, read: impl FnOnce(&G, &App) -> R) -> Option<R> {
        let lock = self.app.borrow();
        Some(read(lock.try_global()?, &lock))
    }

    /// Sets the global of type `G`, replacing any previous value.
    pub fn set_global<G: Global>(&mut self, global: G) {
        let mut lock = self.app.borrow_mut();
        lock.update(|cx| cx.set_global(global))
    }

    /// Runs `update` with mutable access to the global of type `G`.
    ///
    /// # Panics
    ///
    /// Panics if no global of type `G` is set.
    pub fn update_global<G: Global, R>(&mut self, update: impl FnOnce(&mut G, &mut App) -> R) -> R {
        let mut lock = self.app.borrow_mut();
        lock.update(|cx| cx.update_global(update))
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

    /// Runs pending tasks until none can make progress.
    ///
    /// This doesn't move the simulated clock. Use `executor().advance_clock` for that.
    /// Timers that haven't expired stay pending.
    pub fn run_until_parked(&self) {
        self.dispatcher.run_until_parked();
    }

    /// Dispatches `action` to the focused element in `window`, then runs pending tasks.
    ///
    /// # Panics
    ///
    /// Panics if `window` has been closed.
    pub fn dispatch_action<A>(&mut self, window: AnyWindowHandle, action: A)
    where
        A: Action,
    {
        window
            .update(self, |_, window, cx| {
                window.dispatch_action(action.boxed_clone(), cx)
            })
            .unwrap();

        self.background_executor.run_until_parked()
    }

    /// Sends each keystroke in a space-separated list to `window`, then runs pending tasks.
    ///
    /// Each part is parsed as in [`TestAppWindow::simulate_keystroke`](crate::TestAppWindow::simulate_keystroke).
    /// The space bar is the key `"space"`, which can't appear in this list, because
    /// a space separates keystrokes. Two spaces in a row send an empty keystroke.
    ///
    /// # Panics
    ///
    /// Panics if any part isn't a valid keystroke.
    pub fn simulate_keystrokes(&mut self, window: AnyWindowHandle, keystrokes: &str) {
        for keystroke in keystrokes
            .split(' ')
            .map(Keystroke::parse)
            .map(Result::unwrap)
        {
            self.dispatch_keystroke(window, keystroke);
        }

        self.background_executor.run_until_parked()
    }

    /// Types `input` into `window` by sending one keystroke per character, then runs pending tasks.
    ///
    /// Each character is parsed as a keystroke, so the text reaches text inputs. A
    /// space is sent with the key `" "`, not `"space"`.
    ///
    /// Splitting on an empty string also sends an empty keystroke before the first
    /// character and after the last. Don't rely on those.
    pub fn simulate_input(&mut self, window: AnyWindowHandle, input: &str) {
        for keystroke in input.split("").map(Keystroke::parse).map(Result::unwrap) {
            self.dispatch_keystroke(window, keystroke);
        }

        self.background_executor.run_until_parked()
    }

    /// Dispatches one keystroke to the focused element in `window`.
    ///
    /// This doesn't run pending tasks. [`TestAppContext::simulate_keystrokes`] does.
    pub fn dispatch_keystroke(&mut self, window: AnyWindowHandle, keystroke: Keystroke) {
        self.update_window(window, |_, window, cx| {
            window.dispatch_keystroke(keystroke, cx)
        })
        .unwrap();
    }

    /// Returns the `TestWindow` backing the given handle.
    pub(crate) fn test_window(&self, window: AnyWindowHandle) -> TestWindow {
        self.app
            .borrow_mut()
            .windows
            .get_mut(window.id)
            .unwrap()
            .as_deref_mut()
            .unwrap()
            .platform_window
            .as_test()
            .unwrap()
            .clone()
    }

    /// Returns a stream that yields `()` each time `entity` notifies.
    ///
    /// The stream ends when the entity is released.
    pub fn notifications<T: 'static>(
        &mut self,
        entity: &Entity<T>,
    ) -> impl Stream<Item = ()> + use<T> {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        self.update(|cx| {
            cx.observe(entity, {
                let tx = tx.clone();
                move |_, _| {
                    let _ = tx.unbounded_send(());
                }
            })
            .detach();
            cx.observe_release(entity, move |_, _| tx.close_channel())
                .detach()
        });
        rx
    }

    /// Returns a stream of the events `entity` emits, cloned, in order.
    pub fn events<Evt, T: 'static + EventEmitter<Evt>>(
        &mut self,
        entity: &Entity<T>,
    ) -> futures::channel::mpsc::UnboundedReceiver<Evt>
    where
        Evt: 'static + Clone,
    {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        entity
            .update(self, |_, cx: &mut Context<T>| {
                cx.subscribe(entity, move |_entity, _handle, event, _cx| {
                    let _ = tx.unbounded_send(event.clone());
                })
            })
            .detach();
        rx
    }

    /// Waits until `predicate` returns `true` for `entity`, checking after each notification.
    ///
    /// # Panics
    ///
    /// Panics if the predicate isn't true within 3 seconds of simulated time, or if
    /// the entity is released while waiting.
    pub async fn condition<T: 'static>(
        &mut self,
        entity: &Entity<T>,
        mut predicate: impl FnMut(&mut T, &mut Context<T>) -> bool,
    ) {
        let timer = self.executor().timer(Duration::from_secs(3));
        let mut notifications = self.notifications(entity);

        use futures::FutureExt as _;
        use futures_concurrency::future::Race as _;

        (
            async {
                loop {
                    if entity.update(self, &mut predicate) {
                        return Ok(());
                    }

                    if notifications.next().await.is_none() {
                        bail!("entity dropped")
                    }
                }
            },
            timer.map(|_| Err(anyhow!("condition timed out"))),
        )
            .race()
            .await
            .unwrap();
    }

    /// Sets this app's name.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_name(&mut self, name: &'static str) {
        self.update(|cx| cx.name = Some(name))
    }
}

impl<T: 'static> Entity<T> {
    /// Returns a future that resolves with the next event `self` emits.
    ///
    /// The subscription is made when you call this, not when you await the future, so
    /// an event emitted in between is still received.
    ///
    /// # Panics
    ///
    /// Panics if the entity is released before it emits an event.
    pub fn next_event<Event>(&self, cx: &mut TestAppContext) -> impl Future<Output = Event>
    where
        Event: Send + Clone + 'static,
        T: EventEmitter<Event>,
    {
        let (tx, mut rx) = oneshot::channel();
        let mut tx = Some(tx);
        let subscription = self.update(cx, |_, cx| {
            cx.subscribe(self, move |_, _, event, _| {
                if let Some(tx) = tx.take() {
                    _ = tx.send(event.clone());
                }
            })
        });

        async move {
            let event = rx.await.expect("no event emitted");
            drop(subscription);
            event
        }
    }
}

impl<V: 'static> Entity<V> {
    /// Returns a future that resolves on the entity's next notification.
    ///
    /// This advances the simulated clock by `advance_clock_by` as soon as you call it,
    /// before the future is awaited. Use it when the notification comes from a timer
    /// that the advance will fire.
    ///
    /// # Panics
    ///
    /// Panics if the entity is released while the future waits.
    pub fn next_notification(
        &self,
        advance_clock_by: Duration,
        cx: &TestAppContext,
    ) -> impl Future<Output = ()> {
        use postage::prelude::{Sink as _, Stream as _};

        let (mut tx, mut rx) = postage::mpsc::channel(1);
        let subscription = cx.app.borrow_mut().observe(self, move |_, _| {
            tx.try_send(()).ok();
        });

        cx.executor().advance_clock(advance_clock_by);

        async move {
            rx.recv()
                .await
                .expect("entity dropped while test was waiting for its next notification");
            drop(subscription);
        }
    }
}

impl<V> Entity<V> {
    /// Returns a future that resolves when `predicate` returns `true` for this entity.
    ///
    /// The predicate is checked when the future is first polled, then again after
    /// each notification or event from the entity.
    ///
    /// # Panics
    ///
    /// Panics if the entity is released while the future waits.
    pub fn condition<Evt>(
        &self,
        cx: &TestAppContext,
        mut predicate: impl FnMut(&V, &App) -> bool,
    ) -> impl Future<Output = ()>
    where
        Evt: 'static,
        V: EventEmitter<Evt>,
    {
        use postage::prelude::{Sink as _, Stream as _};

        let (tx, mut rx) = postage::mpsc::channel(1024);

        let mut cx = cx.app.borrow_mut();
        let subscriptions = (
            cx.observe(self, {
                let mut tx = tx.clone();
                move |_, _| {
                    tx.blocking_send(()).ok();
                }
            }),
            cx.subscribe(self, {
                let mut tx = tx;
                move |_, _: &Evt, _| {
                    tx.blocking_send(()).ok();
                }
            }),
        );

        let cx = cx.this.upgrade().unwrap();
        let handle = self.downgrade();

        async move {
            loop {
                {
                    let cx = cx.borrow();
                    let cx = &*cx;
                    if predicate(
                        handle
                            .upgrade()
                            .expect("view dropped with pending condition")
                            .read(cx),
                        cx,
                    ) {
                        break;
                    }
                }

                rx.recv()
                    .await
                    .expect("view dropped with pending condition");
            }
            drop(subscriptions);
        }
    }
}

use derive_more::{Deref, DerefMut};

use super::{Context, Entity};
#[derive(Deref, DerefMut, Clone)]
/// A [`TestAppContext`] bound to one window, the test equivalent of a [`Window`] and [`App`].
///
/// Use it to run window-specific test code. It implements [`VisualContext`] and
/// dereferences to the [`TestAppContext`] it wraps. Clones refer to the same window.
pub struct VisualTestContext {
    #[deref]
    #[deref_mut]
    /// The underlying [`TestAppContext`]. Methods on it are also available through deref.
    pub cx: TestAppContext,
    window: AnyWindowHandle,
}

impl VisualTestContext {
    /// Runs `f` with the window and the app. This doesn't run pending tasks.
    pub fn update<R>(&mut self, f: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        self.cx
            .update_window(self.window, |_, window, cx| f(window, cx))
            .unwrap()
    }

    /// Creates a context for `window` that shares `cx`'s app.
    ///
    /// Shadow the [`TestAppContext`] with the result, which is usually what you want:
    /// `let cx = VisualTestContext::from_window(window, cx);`.
    pub fn from_window(window: AnyWindowHandle, cx: &TestAppContext) -> Self {
        Self {
            cx: cx.clone(),
            window,
        }
    }

    /// Runs pending tasks until none can make progress.
    ///
    /// This runs the same scheduler as [`TestAppContext::run_until_parked`]. It doesn't move the simulated clock.
    pub fn run_until_parked(&self) {
        self.cx.background_executor.run_until_parked();
    }

    /// Dispatches `action` to the focused element in this window, then runs pending tasks.
    pub fn dispatch_action<A>(&mut self, action: A)
    where
        A: Action,
    {
        self.cx.dispatch_action(self.window, action)
    }

    /// Returns the title set with [`Window::set_window_title`], or `None` if none was set.
    pub fn window_title(&mut self) -> Option<String> {
        self.cx.test_window(self.window).0.lock().title.clone()
    }

    /// Returns the document path set with [`Window::set_document_path`], or `None` if none was set.
    pub fn document_path(&mut self) -> Option<std::path::PathBuf> {
        self.cx
            .test_window(self.window)
            .0
            .lock()
            .document_path
            .clone()
    }

    /// Sends a space-separated list of keystrokes to this window, then runs pending tasks.
    ///
    /// This is [`TestAppContext::simulate_keystrokes`] for this window. The same rules
    /// apply: a space separates keystrokes, so use the key name `"space"` only as one
    /// keystroke.
    pub fn simulate_keystrokes(&mut self, keystrokes: &str) {
        self.cx.simulate_keystrokes(self.window, keystrokes)
    }

    /// Types `input` into this window, one keystroke per character, then runs pending tasks.
    ///
    /// See [`TestAppContext::simulate_input`] for how characters are sent.
    pub fn simulate_input(&mut self, input: &str) {
        self.cx.simulate_input(self.window, input)
    }

    /// Moves the pointer to `position`, with `button` held if it's `Some`.
    pub fn simulate_mouse_move(
        &mut self,
        position: Point<Pixels>,
        button: impl Into<Option<MouseButton>>,
        modifiers: Modifiers,
    ) {
        self.simulate_event(MouseMoveEvent {
            position,
            modifiers,
            pressed_button: button.into(),
        })
    }

    /// Presses `button` at `position` with `modifiers` held. The click count is always `1`.
    pub fn simulate_mouse_down(
        &mut self,
        position: Point<Pixels>,
        button: MouseButton,
        modifiers: Modifiers,
    ) {
        self.simulate_event(MouseDownEvent {
            position,
            modifiers,
            button,
            click_count: 1,
            first_mouse: false,
        })
    }

    /// Releases `button` at `position` with `modifiers` held. The click count is always `1`.
    pub fn simulate_mouse_up(
        &mut self,
        position: Point<Pixels>,
        button: MouseButton,
        modifiers: Modifiers,
    ) {
        self.simulate_event(MouseUpEvent {
            position,
            modifiers,
            button,
            click_count: 1,
        })
    }

    /// Presses and releases the left button at `position`, with click count `1` on both events.
    pub fn simulate_click(&mut self, position: Point<Pixels>, modifiers: Modifiers) {
        self.simulate_event(MouseDownEvent {
            position,
            modifiers,
            button: MouseButton::Left,
            click_count: 1,
            first_mouse: false,
        });
        self.simulate_event(MouseUpEvent {
            position,
            modifiers,
            button: MouseButton::Left,
            click_count: 1,
        });
    }

    /// Sends a modifiers-changed event with `modifiers`. Caps lock is reported as off.
    pub fn simulate_modifiers_change(&mut self, modifiers: Modifiers) {
        self.simulate_event(ModifiersChangedEvent {
            modifiers,
            capslock: Capslock { on: false },
        })
    }

    /// Sends a modifiers-changed event with no modifiers and caps lock set to `on`.
    pub fn simulate_capslock_change(&mut self, on: bool) {
        self.simulate_event(ModifiersChangedEvent {
            modifiers: Modifiers::none(),
            capslock: Capslock { on },
        })
    }

    /// Resizes the window to `size`. See [`TestAppContext::simulate_window_resize`].
    ///
    /// This doesn't paint the window. Call [`VisualTestContext::draw`] to paint at the new size.
    pub fn simulate_resize(&self, size: Size<Pixels>) {
        self.simulate_window_resize(self.window, size)
    }

    /// Returns the bounds of the element with `debug_selector(selector)` in the last painted frame, or `None`.
    ///
    /// Draw the view first, so that the frame contains the element. Use this to click
    /// an element without guessing its position.
    pub fn debug_bounds(&mut self, selector: &'static str) -> Option<Bounds<Pixels>> {
        self.update(|window, _| window.rendered_frame.debug_bounds.get(selector).copied())
    }

    /// Lays out and paints `f`'s element as a root at `origin`, within `space`.
    ///
    /// Use this to paint an element on its own, without a root view. It returns the
    /// element's request-layout and prepaint state. The window is refreshed afterward.
    pub fn draw<E>(
        &mut self,
        origin: Point<Pixels>,
        space: impl Into<Size<AvailableSpace>>,
        f: impl FnOnce(&mut Window, &mut App) -> E,
    ) -> (E::RequestLayoutState, E::PrepaintState)
    where
        E: Element,
    {
        self.update(|window, cx| {
            let arena_scope = ElementArenaScope::enter(&cx.element_arena);

            window.invalidator.set_phase(DrawPhase::Prepaint);
            let mut element = Drawable::new(f(window, cx));
            element.layout_as_root(space.into(), window, cx);
            window.with_absolute_element_offset(origin, |window| element.prepaint(window, cx));

            window.invalidator.set_phase(DrawPhase::Paint);
            let (request_layout_state, prepaint_state) = element.paint(window, cx);

            window.invalidator.set_phase(DrawPhase::None);
            window.refresh();

            drop(element);
            arena_scope.exit(&cx.element_arena).clear(cx);

            (request_layout_state, prepaint_state)
        })
    }

    /// Sends `event` to the window as platform input, then runs pending tasks.
    ///
    /// Call [`VisualTestContext::draw`] first, so that hit tests see the current layout.
    pub fn simulate_event<E: InputEvent>(&mut self, event: E) {
        self.test_window(self.window)
            .simulate_input(event.to_platform_input());
        self.background_executor.run_until_parked();
    }

    /// Makes this window inactive, as if the user had switched to another window.
    ///
    /// Does nothing if the window isn't active. Runs pending tasks.
    pub fn deactivate_window(&mut self) {
        if Some(self.window) == self.test_platform.active_window() {
            self.test_platform.set_active_window(None)
        }
        self.background_executor.run_until_parked();
    }

    /// Runs the window's should-close callback and returns its result.
    ///
    /// Returns `false` if the window has no callback. This doesn't remove the window
    /// from the app, even when the callback returns `true`.
    pub fn simulate_close(&mut self) -> bool {
        let handler = self
            .cx
            .update_window(self.window, |_, window, _| {
                window
                    .platform_window
                    .as_test()
                    .unwrap()
                    .0
                    .lock()
                    .should_close_handler
                    .take()
            })
            .unwrap();
        if let Some(mut handler) = handler {
            let should_close = handler();
            self.cx
                .update_window(self.window, |_, window, _| {
                    window.platform_window.on_should_close(handler);
                })
                .unwrap();
            should_close
        } else {
            false
        }
    }

    /// Leaks this context and returns a mutable reference that stays valid until the test ends.
    ///
    /// [`TestAppContext::add_window_view`] and [`TestAppContext::add_empty_window`] use
    /// this to return a context that borrows from the test. The memory is freed when
    /// the test quits.
    pub fn into_mut(self) -> &'static mut Self {
        let ptr = Box::into_raw(Box::new(self));
        // safety: on_quit will be called after the test has finished.
        // the executor will ensure that all tasks related to the test have stopped.
        // so there is no way for cx to be accessed after on_quit is called.
        // todo: This is unsound under stacked borrows (also tree borrows probably?)
        // the mutable reference invalidates `ptr` which is later used in the closure
        let cx = unsafe { &mut *ptr };
        cx.on_quit(move || unsafe {
            drop(Box::from_raw(ptr));
        });
        cx
    }
}

impl AppContext for VisualTestContext {
    fn new<T: 'static>(&mut self, build_entity: impl FnOnce(&mut Context<T>) -> T) -> Entity<T> {
        self.window
            .update(&mut self.cx, |_, _, cx| cx.new(build_entity))
            .expect("window was unexpectedly closed")
    }

    fn reserve_entity<T: 'static>(&mut self) -> crate::Reservation<T> {
        self.cx.reserve_entity()
    }

    fn insert_entity<T: 'static>(
        &mut self,
        reservation: crate::Reservation<T>,
        build_entity: impl FnOnce(&mut Context<T>) -> T,
    ) -> Entity<T> {
        self.window
            .update(&mut self.cx, |_, _, cx| {
                cx.insert_entity(reservation, build_entity)
            })
            .expect("window was unexpectedly closed")
    }

    fn update_entity<T, R>(
        &mut self,
        handle: &Entity<T>,
        update: impl FnOnce(&mut T, &mut Context<T>) -> R,
    ) -> R
    where
        T: 'static,
    {
        self.cx.update_entity(handle, update)
    }

    fn as_mut<'a, T>(&'a mut self, handle: &Entity<T>) -> super::GpuiBorrow<'a, T>
    where
        T: 'static,
    {
        self.cx.as_mut(handle)
    }

    fn read_entity<T, R>(&self, handle: &Entity<T>, read: impl FnOnce(&T, &App) -> R) -> R
    where
        T: 'static,
    {
        self.cx.read_entity(handle, read)
    }

    fn notify(&mut self, entity_id: EntityId) {
        self.cx.notify(entity_id)
    }

    fn emit<EntityType, EventType>(&mut self, entity: &Entity<EntityType>, event: EventType)
    where
        EntityType: EventEmitter<EventType>,
        EventType: 'static,
    {
        self.cx.emit(entity, event)
    }

    fn update_window<T, F>(&mut self, window: AnyWindowHandle, f: F) -> Result<T>
    where
        F: FnOnce(AnyView, &mut Window, &mut App) -> T,
    {
        self.cx.update_window(window, f)
    }

    fn with_window<R>(
        &mut self,
        entity_id: EntityId,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> Option<R> {
        self.cx.with_window(entity_id, f)
    }

    fn read_window<T, R>(
        &self,
        window: &WindowHandle<T>,
        read: impl FnOnce(Entity<T>, &App) -> R,
    ) -> Result<R>
    where
        T: 'static,
    {
        self.cx.read_window(window, read)
    }

    fn background_spawn<R>(&self, future: impl Future<Output = R> + Send + 'static) -> Task<R>
    where
        R: Send + 'static,
    {
        self.cx.background_spawn(future)
    }

    fn read_global<G, R>(&self, callback: impl FnOnce(&G, &App) -> R) -> R
    where
        G: Global,
    {
        self.cx.read_global(callback)
    }

    fn insert_global_entity<T: 'static>(&mut self, entity: Entity<T>) {
        self.cx.insert_global_entity(entity)
    }

    fn remove_global_entity<T: 'static>(&mut self, entity: &Entity<T>) {
        self.cx.remove_global_entity(entity)
    }

    fn global_entities<T: 'static>(&self) -> impl Iterator<Item = Entity<T>> {
        self.cx.global_entities()
    }
}

impl VisualContext for VisualTestContext {
    type Result<T> = T;

    /// Get the underlying window handle underlying this context.
    fn window_handle(&self) -> AnyWindowHandle {
        self.window
    }

    fn new_window_entity<T: 'static>(
        &mut self,
        build_entity: impl FnOnce(&mut Window, &mut Context<T>) -> T,
    ) -> Entity<T> {
        self.window
            .update(&mut self.cx, |_, window, cx| {
                cx.new(|cx| build_entity(window, cx))
            })
            .expect("window was unexpectedly closed")
    }

    fn update_window_entity<V: 'static, R>(
        &mut self,
        view: &Entity<V>,
        update: impl FnOnce(&mut V, &mut Window, &mut Context<V>) -> R,
    ) -> R {
        let view = view.clone();
        self.cx
            .app
            .borrow_mut()
            .with_window(view.entity_id(), |window, app| {
                view.update(app, |v, cx| update(v, window, cx))
            })
            .expect("entity has no current window; use `update` instead of `update_in`")
    }

    fn replace_root_view<V>(
        &mut self,
        build_view: impl FnOnce(&mut Window, &mut Context<V>) -> V,
    ) -> Entity<V>
    where
        V: 'static + Render,
    {
        self.window
            .update(&mut self.cx, |_, window, cx| {
                window.replace_root(cx, build_view)
            })
            .expect("window was unexpectedly closed")
    }

    fn focus<V: crate::Focusable>(&mut self, view: &Entity<V>) {
        self.window
            .update(&mut self.cx, |_, window, cx| {
                view.read(cx).focus_handle(cx).focus(window, cx)
            })
            .expect("window was unexpectedly closed")
    }
}

impl AnyWindowHandle {
    /// Creates an entity with `build_view` in this window.
    ///
    /// # Panics
    ///
    /// Panics if the window has been closed.
    pub fn build_entity<V: Render + 'static>(
        &self,
        cx: &mut TestAppContext,
        build_view: impl FnOnce(&mut Window, &mut Context<V>) -> V,
    ) -> Entity<V> {
        self.update(cx, |_, window, cx| cx.new(|cx| build_view(window, cx)))
            .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        PathPromptOptions, SystemNotification, SystemNotificationAction,
        SystemNotificationResponse, TestAppContext,
    };
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;

    #[gpui::test]
    async fn test_system_notifications_require_identity_and_replace_matching_tags(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            cx.show_system_notification(SystemNotification {
                tag: "thread-1".into(),
                title: "Task started".into(),
                body: "Running tests".into(),
                actions: Vec::new(),
            });
        });
        assert!(cx.shown_system_notifications().is_empty());
        assert!(cx.delivered_system_notifications().is_empty());

        cx.update(|cx| {
            cx.set_app_identity("com.example.tasks", "Tasks");
            cx.show_system_notification(SystemNotification {
                tag: "thread-1".into(),
                title: "Task started".into(),
                body: "Running tests".into(),
                actions: Vec::new(),
            });
            cx.show_system_notification(SystemNotification {
                tag: "thread-1".into(),
                title: "Task finished".into(),
                body: "All tests passed".into(),
                actions: vec![SystemNotificationAction {
                    id: "open".into(),
                    label: "Open".into(),
                }],
            });
        });

        assert_eq!(
            cx.app_identity(),
            Some(("com.example.tasks".into(), "Tasks".into()))
        );
        assert_eq!(cx.shown_system_notifications().len(), 2);
        assert_eq!(
            cx.delivered_system_notifications(),
            [SystemNotification {
                tag: "thread-1".into(),
                title: "Task finished".into(),
                body: "All tests passed".into(),
                actions: vec![SystemNotificationAction {
                    id: "open".into(),
                    label: "Open".into(),
                }],
            }]
        );

        cx.update(|cx| cx.dismiss_system_notification("thread-1"));
        assert!(cx.delivered_system_notifications().is_empty());
        assert_eq!(cx.dismissed_system_notifications(), ["thread-1"]);
    }

    #[gpui::test]
    async fn test_system_notification_body_and_action_responses(cx: &mut TestAppContext) {
        let responses = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            cx.on_system_notification_response({
                let responses = responses.clone();
                move |response, _cx| responses.borrow_mut().push(response)
            });
        });

        cx.simulate_system_notification_response(SystemNotificationResponse {
            tag: "thread-1".into(),
            action_id: None,
        });
        cx.simulate_system_notification_response(SystemNotificationResponse {
            tag: "thread-1".into(),
            action_id: Some("default".into()),
        });

        assert_eq!(
            responses.borrow().as_slice(),
            &[
                SystemNotificationResponse {
                    tag: "thread-1".into(),
                    action_id: None,
                },
                SystemNotificationResponse {
                    tag: "thread-1".into(),
                    action_id: Some("default".into()),
                },
            ]
        );
    }

    #[gpui::test]
    async fn test_system_notification_response_handler_can_be_replaced(cx: &mut TestAppContext) {
        let first_responses = Rc::new(RefCell::new(Vec::new()));
        let second_responses = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            cx.on_system_notification_response({
                let first_responses = first_responses.clone();
                move |response, _cx| first_responses.borrow_mut().push(response)
            });
            cx.on_system_notification_response({
                let second_responses = second_responses.clone();
                move |response, _cx| second_responses.borrow_mut().push(response)
            });
        });

        let response = SystemNotificationResponse {
            tag: "thread-1".into(),
            action_id: None,
        };
        cx.simulate_system_notification_response(response.clone());

        assert!(first_responses.borrow().is_empty());
        assert_eq!(second_responses.borrow().as_slice(), &[response]);
    }

    #[gpui::test]
    async fn test_system_notification_response_handler_can_reenter_app(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_app_identity("com.example.tasks", "Tasks");
            cx.show_system_notification(SystemNotification {
                tag: "thread-1".into(),
                title: "Task finished".into(),
                body: "All tests passed".into(),
                actions: Vec::new(),
            });
            cx.on_system_notification_response(|response, cx| {
                cx.dismiss_system_notification(&response.tag);
            });
        });

        cx.simulate_system_notification_response(SystemNotificationResponse {
            tag: "thread-1".into(),
            action_id: None,
        });

        assert!(cx.delivered_system_notifications().is_empty());
        assert_eq!(cx.dismissed_system_notifications(), ["thread-1"]);
    }

    #[gpui::test]
    async fn test_simulate_path_prompt_response(cx: &mut TestAppContext) {
        assert!(!cx.did_prompt_for_paths());

        let receiver = cx.update(|cx| {
            cx.prompt_for_paths(PathPromptOptions {
                files: false,
                directories: true,
                multiple: true,
                prompt: None,
            })
        });
        assert!(cx.did_prompt_for_paths());

        let selected = vec![PathBuf::from("/a"), PathBuf::from("/b")];
        cx.simulate_path_prompt_response({
            let selected = selected.clone();
            move |options| {
                assert!(options.multiple);
                Some(selected)
            }
        });
        assert!(!cx.did_prompt_for_paths());

        let response = receiver.await.unwrap().unwrap();
        assert_eq!(response, Some(selected));
    }

    #[gpui::test]
    async fn test_simulate_path_prompt_cancellation(cx: &mut TestAppContext) {
        let receiver = cx.update(|cx| {
            cx.prompt_for_paths(PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: None,
            })
        });

        cx.simulate_path_prompt_response(|_options| None);

        let response = receiver.await.unwrap().unwrap();
        assert_eq!(response, None);
    }
}
