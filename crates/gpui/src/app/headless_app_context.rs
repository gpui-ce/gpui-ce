//! Cross-platform headless app context for tests that need real text shaping.
//!
//! This replaces the macOS-only `HeadlessMetalAppContext` with a platform-neutral
//! implementation backed by `TestPlatform`. Tests supply a real `PlatformTextSystem`
//! to get accurate glyph measurements while keeping everything else deterministic.
//!
//! Optionally, a renderer factory can be provided to enable real GPU rendering
//! and screenshot capture via [`HeadlessAppContext::capture_screenshot`].

use crate::{
    AnyView, AnyWindowHandle, App, AppCell, AssetRegistry, AssetSource, BackgroundExecutor, Bounds,
    Entity, ForegroundExecutor, Hsla, Pixels, PlatformHeadlessRenderer, PlatformTextSystem, Render,
    ScaledPixels, Size, TestDispatcher, TestPlatform, TextSystem, Window, WindowBounds,
    WindowHandle, WindowOptions, app::GpuiMode, context::*,
};
use anyhow::Result;
use image::RgbaImage;
use std::{rc::Rc, sync::Arc, time::Duration};

/// A headless app for tests that need real text layout, without a window on screen.
///
/// Like [`TestAppContext`](crate::TestAppContext), this runs on a deterministic
/// scheduler and a test platform. Unlike it, you supply the text system, so text
/// is measured the way the platform measures it. Windows are hidden and unfocused.
/// Use [`HeadlessAppContext::debug_bounds`] and the `*_bounds` methods to check
/// where things were painted, and [`HeadlessAppContext::capture_screenshot`] to
/// get pixels when a renderer is configured.
///
/// The text system comes from a platform or text crate. For example, the
/// `ParleyTextSystem` in the `gpui_ce_parley` crate measures text with real fonts.
/// This example uses [`TestTextSystem`](crate::TestTextSystem), which is for tests
/// that don't check text metrics:
///
/// ```
/// use std::sync::Arc;
///
/// use gpui::{AppContext, HeadlessAppContext, TestTextSystem};
///
/// let mut cx = HeadlessAppContext::new(Arc::new(TestTextSystem));
/// let count = cx.new(|_| 1usize);
/// assert_eq!(cx.read_entity(&count, |count, _| *count), 1);
/// ```
pub struct HeadlessAppContext {
    /// The underlying app cell.
    pub app: Rc<AppCell>,
    /// The background executor for running async tasks.
    pub background_executor: BackgroundExecutor,
    /// The foreground executor for running tasks on the main thread.
    pub foreground_executor: ForegroundExecutor,
    dispatcher: TestDispatcher,
    text_system: Arc<TextSystem>,
}

impl crate::app_borrow::UpgradeApp for HeadlessAppContext {
    fn upgrade(&self) -> impl crate::app_borrow::BorrowApp {
        self.app.clone()
    }

    fn upgrade_mut(&mut self) -> impl crate::app_borrow::BorrowAppMut {
        self.app.clone()
    }
}

impl HeadlessAppContext {
    /// Creates a headless context that measures text with `platform_text_system`.
    ///
    /// It has no asset source and no renderer, so [`HeadlessAppContext::capture_screenshot`]
    /// returns an error. Use [`HeadlessAppContext::with_platform`] for screenshots.
    pub fn new(platform_text_system: Arc<dyn PlatformTextSystem>) -> Self {
        Self::with_platform(platform_text_system, Arc::new(()), || None)
    }

    /// Creates a headless context with `platform_text_system` and `asset_source`, and no renderer.
    pub fn with_asset_source(
        platform_text_system: Arc<dyn PlatformTextSystem>,
        asset_source: Arc<dyn AssetSource>,
    ) -> Self {
        Self::with_platform(platform_text_system, asset_source, || None)
    }

    /// Creates a headless context with a text system, an asset source and a renderer factory.
    ///
    /// `renderer_factory` creates the renderer that windows draw with. Return `None`
    /// from it to draw without a renderer, which disables screenshots. The scheduler seed
    /// is read from `SEED`, and is `0` when it's unset or isn't a number.
    pub fn with_platform(
        platform_text_system: Arc<dyn PlatformTextSystem>,
        asset_source: Arc<dyn AssetSource>,
        renderer_factory: impl Fn() -> Option<Box<dyn PlatformHeadlessRenderer>> + 'static,
    ) -> Self {
        let seed = std::env::var("SEED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        let dispatcher = TestDispatcher::new(seed);
        let arc_dispatcher = Arc::new(dispatcher.clone());
        let background_executor = BackgroundExecutor::new(arc_dispatcher.clone());
        let foreground_executor = ForegroundExecutor::new(arc_dispatcher);

        let renderer_factory: Box<dyn Fn() -> Option<Box<dyn PlatformHeadlessRenderer>>> =
            Box::new(renderer_factory);
        let platform = TestPlatform::with_platform(
            background_executor.clone(),
            foreground_executor.clone(),
            platform_text_system.clone(),
            Some(renderer_factory),
        );

        let text_system = Arc::new(TextSystem::new(platform_text_system));
        let http_client = crate::http_client::FakeHttpClient::with_404_response();
        let app = App::new_app(platform, AssetRegistry::from(asset_source), http_client);
        app.borrow_mut().mode = GpuiMode::test();

        Self {
            app,
            background_executor,
            foreground_executor,
            dispatcher,
            text_system,
        }
    }

    /// Opens a hidden, unfocused window of `size` at the origin, with a root view from `build_root`.
    ///
    /// # Errors
    ///
    /// Returns an error if the window can't be opened.
    pub fn open_window<V: Render + 'static>(
        &mut self,
        size: Size<Pixels>,
        build_root: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
    ) -> Result<WindowHandle<V>> {
        use crate::{point, px};

        let bounds = Bounds {
            origin: point(px(0.0), px(0.0)),
            size,
        };

        let mut cx = self.app.borrow_mut();
        cx.open_window(
            WindowOptions::new()
                .window_bounds(Some(WindowBounds::Windowed(bounds)))
                .focus(false)
                .show(false),
            build_root,
        )
    }

    /// Runs pending tasks until none can make progress. This doesn't move the simulated clock.
    pub fn run_until_parked(&self) {
        self.dispatcher.run_until_parked();
    }

    /// Moves the simulated clock forward by `duration`, and runs the tasks that become ready.
    pub fn advance_clock(&self, duration: Duration) {
        self.dispatcher.advance_clock(duration);
    }

    /// Lets blocking waits in the scheduler sleep in real time, for tests that wait on real I/O.
    ///
    /// Pair it with [`HeadlessAppContext::forbid_parking`]. See
    /// [`BackgroundExecutor::allow_parking`](crate::BackgroundExecutor::allow_parking).
    pub fn allow_parking(&self) {
        self.dispatcher.allow_parking();
    }

    /// Turns off the parking mode from [`HeadlessAppContext::allow_parking`], returning to deterministic execution.
    pub fn forbid_parking(&self) {
        self.dispatcher.forbid_parking();
    }

    /// Runs `f` with mutable access to the [`App`]. This doesn't run pending tasks.
    pub fn update<R>(&mut self, f: impl FnOnce(&mut App) -> R) -> R {
        let mut app = self.app.borrow_mut();
        f(&mut app)
    }

    /// Runs `f` with the window's root view, the [`Window`] and the [`App`].
    ///
    /// This doesn't paint the window. Methods that read painted output, such as
    /// [`HeadlessAppContext::debug_bounds`], see the last frame that was painted.
    ///
    /// # Errors
    ///
    /// Returns an error if `window` has been closed.
    pub fn update_window<R>(
        &mut self,
        window: AnyWindowHandle,
        f: impl FnOnce(AnyView, &mut Window, &mut App) -> R,
    ) -> Result<R> {
        let mut app = self.app.borrow_mut();
        app.update_window(window, f)
    }

    /// Returns the bounds of the element with `debug_selector(selector)` in the last painted frame, or `None`.
    ///
    /// # Errors
    ///
    /// Returns an error if `window` has been closed.
    pub fn debug_bounds(
        &mut self,
        window: AnyWindowHandle,
        selector: &str,
    ) -> Result<Option<Bounds<Pixels>>> {
        self.update_window(window, |_, window, _| {
            window.rendered_frame.debug_bounds.get(selector).copied()
        })
    }

    /// Returns the device-pixel bounds of each solid quad in `color` in the last painted frame.
    ///
    /// The color must match exactly. Quads with other colors are skipped.
    ///
    /// # Errors
    ///
    /// Returns an error if `window` has been closed.
    pub fn solid_quad_bounds(
        &mut self,
        window: AnyWindowHandle,
        color: Hsla,
    ) -> Result<Vec<Bounds<ScaledPixels>>> {
        let color = color.into();
        self.update_window(window, |_, window, _| {
            window
                .rendered_frame
                .scene
                .quads
                .iter()
                .filter(|quad| quad.background.solid == color)
                .map(|quad| quad.bounds)
                .collect()
        })
    }

    /// Returns the device-pixel bounds of each monochrome glyph painted in `color` in the last painted frame.
    ///
    /// The color must match exactly.
    ///
    /// # Errors
    ///
    /// Returns an error if `window` has been closed.
    pub fn glyph_bounds(
        &mut self,
        window: AnyWindowHandle,
        color: Hsla,
    ) -> Result<Vec<Bounds<ScaledPixels>>> {
        let color = color.into();
        self.update_window(window, |_, window, _| {
            window
                .rendered_frame
                .scene
                .monochrome_sprites
                .iter()
                .filter(|sprite| sprite.color == color)
                .map(|sprite| sprite.bounds)
                .collect()
        })
    }

    /// Returns the device-pixel bounds of each underline painted in `color` in the last painted frame.
    ///
    /// The color must match exactly.
    ///
    /// # Errors
    ///
    /// Returns an error if `window` has been closed.
    pub fn underline_bounds(
        &mut self,
        window: AnyWindowHandle,
        color: Hsla,
    ) -> Result<Vec<Bounds<ScaledPixels>>> {
        let color = color.into();
        self.update_window(window, |_, window, _| {
            window
                .rendered_frame
                .scene
                .underlines
                .iter()
                .filter(|underline| underline.color == color)
                .map(|underline| underline.bounds)
                .collect()
        })
    }

    /// Renders `window`'s last painted frame to an image.
    ///
    /// # Errors
    ///
    /// Returns an error if the context was created without a renderer, which is the
    /// case for [`HeadlessAppContext::new`] and [`HeadlessAppContext::with_asset_source`].
    /// Also returns an error if the window has been closed.
    pub fn capture_screenshot(&mut self, window: AnyWindowHandle) -> Result<RgbaImage> {
        let mut app = self.app.borrow_mut();
        app.update_window(window, |_, window, _| window.render_to_image())?
    }

    /// Returns the text system.
    pub fn text_system(&self) -> &Arc<TextSystem> {
        &self.text_system
    }

    /// Returns the background executor.
    pub fn background_executor(&self) -> &BackgroundExecutor {
        &self.background_executor
    }

    /// Returns the foreground executor.
    pub fn foreground_executor(&self) -> &ForegroundExecutor {
        &self.foreground_executor
    }
}

impl Drop for HeadlessAppContext {
    fn drop(&mut self) {
        // Shut down the app so windows are closed and entity handles are
        // released before the LeakDetector runs.
        self.app.borrow_mut().shutdown();
    }
}

impl AppContext for HeadlessAppContext {}
impl crate::context::ImpliedAppEntity for HeadlessAppContext {}
impl crate::context::ImpliedAppWindow for HeadlessAppContext {}
impl crate::context::ImpliedAppTask for HeadlessAppContext {}
impl crate::context::ImpliedAppGlobals for HeadlessAppContext {}
