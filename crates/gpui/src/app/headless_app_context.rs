//! Cross-platform headless app context for tests that need real text shaping.
//!
//! This replaces the macOS-only `HeadlessMetalAppContext` with a platform-neutral
//! implementation backed by `TestPlatform`. Tests supply a real `PlatformTextSystem`
//! (e.g. `DirectWriteTextSystem` on Windows, `MacTextSystem` on macOS) to get
//! accurate glyph measurements while keeping everything else deterministic.
//!
//! Optionally, a renderer factory can be provided to enable real GPU rendering
//! and screenshot capture via [`HeadlessAppContext::capture_screenshot`].

use crate::{
    AnyView, AnyWindowHandle, App, AppCell, AppContext, AssetSource, BackgroundExecutor, Bounds,
    Context, Entity, EntityId, EventEmitter, ForegroundExecutor, Global, Pixels,
    PlatformHeadlessRenderer, PlatformTextSystem, Render, Reservation, Size, Task, TestDispatcher,
    TestPlatform, TextSystem, Window, WindowBounds, WindowHandle, WindowOptions,
    app::{GpuiBorrow, GpuiMode},
};
use anyhow::Result;
use image::RgbaImage;
use std::{future::Future, rc::Rc, sync::Arc, time::Duration};

/// A cross-platform headless app context for tests that need real text shaping.
///
/// Unlike the old `HeadlessMetalAppContext`, this works on any platform. It uses
/// `TestPlatform` for deterministic scheduling and accepts a pluggable
/// `PlatformTextSystem` so tests get real glyph measurements.
///
/// # Usage
///
/// ```ignore
/// let text_system = Arc::new(gpui_wgpu::CosmicTextSystem::new("fallback"));
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
        let app = App::new_app(platform, asset_source, http_client);
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
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                focus: false,
                show: false,
                ..Default::default()
            },
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

    /// Captures a screenshot from a window.
    ///
    /// Requires that the context was created with a renderer factory that
    /// returns `Some` via [`HeadlessAppContext::with_platform`].
    pub fn capture_screenshot(&mut self, window: AnyWindowHandle) -> Result<RgbaImage> {
        let mut app = self.app.borrow_mut();
        app.update_window(window, |_, window, cx| {
            if let Some(arena_clear_needed) = window.redraw_if_rendered_frame_atlas_is_stale(cx) {
                let image = window.render_to_image();
                arena_clear_needed.clear(cx);
                image
            } else {
                window.render_to_image()
            }
        })?
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

impl AppContext for HeadlessAppContext {
    fn new<T: 'static>(&mut self, build_entity: impl FnOnce(&mut Context<T>) -> T) -> Entity<T> {
        let mut app = self.app.borrow_mut();
        app.new(build_entity)
    }

    fn reserve_entity<T: 'static>(&mut self) -> Reservation<T> {
        let mut app = self.app.borrow_mut();
        app.reserve_entity()
    }

    fn insert_entity<T: 'static>(
        &mut self,
        reservation: Reservation<T>,
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

    fn as_mut<'a, T>(&'a mut self, _: &Entity<T>) -> GpuiBorrow<'a, T>
    where
        T: 'static,
    {
        panic!("Cannot use as_mut with HeadlessAppContext. Call update() instead.")
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
        app.notify(entity_id);
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
        app.read_global(callback)
    }

    fn insert_global_entity<T: 'static>(&mut self, entity: Entity<T>) {
        let mut app = self.app.borrow_mut();
        app.insert_global_entity(entity)
    }

    fn remove_global_entity<T: 'static>(&mut self, entity: &Entity<T>) {
        let mut app = self.app.borrow_mut();
        app.remove_global_entity(entity)
    }

    fn global_entities<T: 'static>(&self) -> impl Iterator<Item = Entity<T>> {
        let app = self.app.borrow();
        let iter = app.global_entities();
        // Cloning the iterator here so that lock can be released when function is going out of scope
        iter.collect::<Vec<_>>().into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AnyView, AtlasKey, AtlasTextureId, AtlasTile, Bounds, DevicePixels, IntoElement,
        NoopTextSystem, ParentElement as _, PlatformAtlas, PrimitiveBatch, RenderImage, Scene,
        StyleRefinement, Styled as _, TileId, div, img, px, size,
    };
    use anyhow::Result;
    use image::{Frame as ImageFrame, ImageBuffer, Rgba, RgbaImage};
    use parking_lot::Mutex;
    use smallvec::SmallVec;
    use std::{
        borrow::Cow,
        collections::{HashMap, HashSet},
    };

    #[derive(Default)]
    struct ResettableAtlas {
        state: Mutex<ResettableAtlasState>,
    }

    #[derive(Default)]
    struct ResettableAtlasState {
        tiles_by_key: HashMap<AtlasKey, AtlasTile>,
        live_textures: HashSet<AtlasTextureId>,
        next_texture_index: u32,
        next_tile_id: u32,
        generation: u64,
    }

    impl ResettableAtlas {
        fn reset_device_resources_for_test(&self) {
            let mut state = self.state.lock();
            state.tiles_by_key.clear();
            state.live_textures.clear();
            state.next_texture_index = 0;
            state.next_tile_id = 0;
            state.generation = state.generation.wrapping_add(1);
        }

        fn has_texture(&self, id: AtlasTextureId) -> bool {
            self.state.lock().live_textures.contains(&id)
        }
    }

    impl PlatformAtlas for ResettableAtlas {
        fn get_or_insert_with<'a>(
            &self,
            key: &AtlasKey,
            build: &mut dyn FnMut() -> Result<Option<(Size<DevicePixels>, Cow<'a, [u8]>)>>,
        ) -> Result<Option<AtlasTile>> {
            if let Some(tile) = self.state.lock().tiles_by_key.get(key).cloned() {
                return Ok(Some(tile));
            }

            let Some((size, _bytes)) = build()? else {
                return Ok(None);
            };

            let mut state = self.state.lock();
            if let Some(tile) = state.tiles_by_key.get(key).cloned() {
                return Ok(Some(tile));
            }

            let texture_id = AtlasTextureId {
                index: state.next_texture_index,
                kind: key.texture_kind(),
            };
            state.next_texture_index += 1;

            let tile = AtlasTile {
                texture_id,
                tile_id: TileId(state.next_tile_id),
                padding: 0,
                bounds: Bounds {
                    origin: Default::default(),
                    size,
                },
            };
            state.next_tile_id += 1;
            state.live_textures.insert(texture_id);
            state.tiles_by_key.insert(key.clone(), tile.clone());
            Ok(Some(tile))
        }

        fn remove(&self, key: &AtlasKey) {
            let mut state = self.state.lock();
            if let Some(tile) = state.tiles_by_key.remove(key) {
                state.live_textures.remove(&tile.texture_id);
                state.generation = state.generation.wrapping_add(1);
            }
        }

        fn generation(&self) -> u64 {
            self.state.lock().generation
        }
    }

    struct CheckingHeadlessRenderer {
        atlas: Arc<ResettableAtlas>,
    }

    impl PlatformHeadlessRenderer for CheckingHeadlessRenderer {
        fn render_scene_to_image(
            &mut self,
            scene: &Scene,
            size: Size<DevicePixels>,
        ) -> Result<RgbaImage> {
            for batch in scene.batches() {
                let texture_id = match batch {
                    PrimitiveBatch::MonochromeSprites { texture_id, .. }
                    | PrimitiveBatch::SubpixelSprites { texture_id, .. }
                    | PrimitiveBatch::PolychromeSprites { texture_id, .. } => texture_id,
                    _ => continue,
                };

                assert!(
                    self.atlas.has_texture(texture_id),
                    "stale atlas texture id after renderer reset: {texture_id:?}",
                );
            }

            Ok(RgbaImage::from_pixel(
                size.width.0.max(1) as u32,
                size.height.0.max(1) as u32,
                Rgba([0, 0, 0, 0]),
            ))
        }

        fn render_scene(&mut self, scene: &Scene, size: Size<DevicePixels>) -> Result<()> {
            self.render_scene_to_image(scene, size).map(|_| ())
        }

        fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
            self.atlas.clone()
        }
    }

    struct ImageRoot {
        image: Arc<RenderImage>,
    }

    struct CachedImageRoot {
        child: Entity<ImageRoot>,
    }

    impl Render for CachedImageRoot {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            AnyView::from(self.child.clone()).cached(StyleRefinement::default().size(px(1.0)))
        }
    }

    impl Render for ImageRoot {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().child(img(self.image.clone()).size(px(1.0)))
        }
    }

    fn test_image() -> Arc<RenderImage> {
        let frame = ImageFrame::new(ImageBuffer::from_pixel(1, 1, Rgba([0, 0, 0, 255])));
        Arc::new(RenderImage::new(SmallVec::from_iter([frame])))
    }

    #[test]
    fn non_forced_present_after_atlas_reset_can_redraw_stale_scene() -> Result<()> {
        let atlas = Arc::new(ResettableAtlas::default());
        let renderer_atlas = atlas.clone();
        let mut cx = HeadlessAppContext::with_platform(
            Arc::new(NoopTextSystem::new()),
            Arc::new(()),
            move || {
                Some(Box::new(CheckingHeadlessRenderer {
                    atlas: renderer_atlas.clone(),
                }))
            },
        );

        let image = test_image();
        let window = cx.open_window(size(px(10.0), px(10.0)), move |_window, cx| {
            cx.new(|_| ImageRoot { image })
        })?;

        cx.capture_screenshot(window.into())?;
        atlas.reset_device_resources_for_test();

        // This models a non-forced Windows paint/present after DirectX device
        // recovery cleared the atlas, but before the forced full redraw happens.
        cx.capture_screenshot(window.into())?;
        Ok(())
    }

    #[test]
    fn non_forced_redraw_after_atlas_reset_must_not_reuse_cached_sprite_scene() -> Result<()> {
        let atlas = Arc::new(ResettableAtlas::default());
        let renderer_atlas = atlas.clone();
        let mut cx = HeadlessAppContext::with_platform(
            Arc::new(NoopTextSystem::new()),
            Arc::new(()),
            move || {
                Some(Box::new(CheckingHeadlessRenderer {
                    atlas: renderer_atlas.clone(),
                }))
            },
        );

        let image = test_image();
        let window = cx.open_window(size(px(10.0), px(10.0)), move |_window, cx| {
            let child = cx.new(|_| ImageRoot { image });
            cx.new(|_| CachedImageRoot { child })
        })?;

        cx.update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear(cx);
            window.render_to_image()
        })??;

        atlas.reset_device_resources_for_test();

        cx.update_window(window.into(), |_, window, cx| {
            window.invalidator.set_dirty(true);
            let arena_clear_needed = window.draw(cx);
            let image = window.render_to_image();
            arena_clear_needed.clear(cx);
            image
        })??;

        Ok(())
    }
}
