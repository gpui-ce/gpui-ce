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

/// A cross-platform headless app context for tests that need real text shaping.
///
/// Unlike the old `HeadlessMetalAppContext`, this works on any platform. It uses
/// `TestPlatform` for deterministic scheduling and accepts a pluggable
/// `PlatformTextSystem` so tests get real glyph measurements.
///
/// # Usage
///
/// ```ignore
/// let text_system = Arc::new(gpui_parley::ParleyTextSystem::new(
///     gpui_parley::SystemFonts::Load,
/// ));
/// let mut cx = HeadlessAppContext::with_platform(
///     text_system,
///     Arc::new(Assets),
///     || gpui_platform::current_headless_renderer(),
/// );
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
    /// Creates a new headless app context with the given text system.
    pub fn new(platform_text_system: Arc<dyn PlatformTextSystem>) -> Self {
        Self::with_platform(platform_text_system, Arc::new(()), || None)
    }

    /// Creates a new headless app context with a custom text system and asset source.
    pub fn with_asset_source(
        platform_text_system: Arc<dyn PlatformTextSystem>,
        asset_source: Arc<dyn AssetSource>,
    ) -> Self {
        Self::with_platform(platform_text_system, asset_source, || None)
    }

    /// Creates a new headless app context with the given text system, asset source,
    /// and an optional renderer factory for screenshot support.
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
        let app = App::new_app(
            platform,
            AssetRegistry::from(asset_source).into(),
            http_client,
        );
        app.borrow_mut().mode = GpuiMode::test();

        Self {
            app,
            background_executor,
            foreground_executor,
            dispatcher,
            text_system,
        }
    }

    /// Opens a window for headless rendering.
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

    /// Runs all pending tasks until parked.
    pub fn run_until_parked(&self) {
        self.dispatcher.run_until_parked();
    }

    /// Advances the simulated clock.
    pub fn advance_clock(&self, duration: Duration) {
        self.dispatcher.advance_clock(duration);
    }

    /// Enables parking mode, allowing blocking on real I/O (e.g., async asset loading).
    pub fn allow_parking(&self) {
        self.dispatcher.allow_parking();
    }

    /// Disables parking mode, returning to deterministic test execution.
    pub fn forbid_parking(&self) {
        self.dispatcher.forbid_parking();
    }

    /// Updates app state.
    pub fn update<R>(&mut self, f: impl FnOnce(&mut App) -> R) -> R {
        let mut app = self.app.borrow_mut();
        f(&mut app)
    }

    /// Updates a window and calls draw to render.
    pub fn update_window<R>(
        &mut self,
        window: AnyWindowHandle,
        f: impl FnOnce(AnyView, &mut Window, &mut App) -> R,
    ) -> Result<R> {
        let mut app = self.app.borrow_mut();
        app.update_window(window, f)
    }

    /// Returns the most recently rendered bounds for an element's debug selector.
    pub fn debug_bounds(
        &mut self,
        window: AnyWindowHandle,
        selector: &str,
    ) -> Result<Option<Bounds<Pixels>>> {
        self.update_window(window, |_, window, _| {
            window.rendered_frame.debug_bounds.get(selector).copied()
        })
    }

    /// Returns the device-pixel bounds of rendered solid quads with the requested color.
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

    /// Returns the device-pixel bounds of painted monochrome glyphs of this color.
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

    /// Returns the device-pixel bounds of painted underlines of this color.
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

    /// Captures a screenshot from a window.
    ///
    /// Requires that the context was created with a renderer factory that
    /// returns `Some` via [`HeadlessAppContext::with_platform`].
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
