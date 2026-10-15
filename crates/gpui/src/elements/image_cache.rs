use crate::{
    AnyElement, AnyEntity, App, AppContext, Asset, AssetLogger, Bounds, Element, ElementId, Entity,
    GlobalElementId, ImageAssetLoader, ImageCacheError, InspectorElementId, IntoElement, LayoutId,
    ParentElement, Pixels, RenderImage, Resource, Style, StyleRefinement, Styled, Task, Window,
    hash,
};

use futures::{FutureExt, future::Shared};
use refineable::Refineable;
use smallvec::SmallVec;
use std::{collections::HashMap, fmt, sync::Arc};

/// Scopes an image cache to the [`img`](crate::img) elements below it.
///
/// Each image in the children uses the cache that `image_cache_provider` returns, instead of the
/// app-wide cache. Pass an `Entity` of an [`ImageCache`] to share one cache, or [`retain_all`] to get
/// a cache that lives with this element's state. The provider is called on every layout and paint.
///
/// Only [`crate::ImageSource::Resource`] sources use the cache. See [`crate::Img::image_cache`].
///
/// # Examples
///
/// ```
/// # use gpui::*;
/// let gallery = image_cache(retain_all("gallery")).children((0..3).map(|ix| {
///     img(format!("https://example.com/photos/{ix}.jpg"))
///         .size(px(120.))
///         .into_any_element()
/// }));
/// ```
pub fn image_cache(image_cache_provider: impl ImageCacheProvider) -> ImageCacheElement {
    ImageCacheElement {
        image_cache_provider: Box::new(image_cache_provider),
        style: StyleRefinement::default(),
        children: SmallVec::default(),
    }
}

/// A type-erased [`ImageCache`], so one [`img`](crate::img) can hold any cache type.
///
/// Built from an `Entity` of an [`ImageCache`] with `From`.
#[derive(Clone)]
pub struct AnyImageCache {
    image_cache: AnyEntity,
    load_fn: fn(
        image_cache: &AnyEntity,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>>,
}

impl<I: ImageCache> From<Entity<I>> for AnyImageCache {
    fn from(image_cache: Entity<I>) -> Self {
        Self {
            image_cache: image_cache.into_any(),
            load_fn: any_image_cache::load::<I>,
        }
    }
}

impl AnyImageCache {
    /// Asks the cache for `resource`. Returns `None` while the image loads, and the result once it is
    /// ready. See [`ImageCache::load`].
    pub fn load(
        &self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        (self.load_fn)(&self.image_cache, resource, window, cx)
    }
}

mod any_image_cache {
    use super::*;

    pub(crate) fn load<I: 'static + ImageCache>(
        image_cache: &AnyEntity,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let image_cache = image_cache.clone().downcast::<I>().unwrap();
        image_cache.update(cx, |image_cache, cx| image_cache.load(resource, window, cx))
    }
}

/// The element that [`image_cache`] returns. It installs the cache for its children during layout
/// and paint, and lays out like a `div`.
pub struct ImageCacheElement {
    image_cache_provider: Box<dyn ImageCacheProvider>,
    style: StyleRefinement,
    children: SmallVec<[AnyElement; 2]>,
}

impl ParentElement for ImageCacheElement {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements)
    }
}

impl Styled for ImageCacheElement {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl IntoElement for ImageCacheElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ImageCacheElement {
    type RequestLayoutState = SmallVec<[LayoutId; 4]>;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let image_cache = self.image_cache_provider.provide(window, cx);
        window.with_image_cache(Some(image_cache), |window| {
            let child_layout_ids = self
                .children
                .iter_mut()
                .map(|child| child.request_layout(window, cx))
                .collect::<SmallVec<_>>();
            let mut style = Style::default();
            style.refine(&self.style);
            let layout_id = window.request_layout(style, child_layout_ids.iter().copied(), cx);
            (layout_id, child_layout_ids)
        })
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        for child in &mut self.children {
            child.prepaint(window, cx);
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let image_cache = self.image_cache_provider.provide(window, cx);
        window.with_image_cache(Some(image_cache), |window| {
            for child in &mut self.children {
                child.paint(window, cx);
            }
        })
    }
}

/// A background load of one image, shared so that every caller waits on the same work.
///
/// Its output is the decoded image or the error.
pub type ImageLoadingTask = Shared<Task<Result<Arc<RenderImage>, ImageCacheError>>>;

/// An entry in a [`RetainAllImageCache`]: a load in progress, or its finished result.
pub enum ImageCacheItem {
    /// The image is still loading. [`ImageCacheItem::get`] moves the item to `Loaded` once the task
    /// finishes.
    Loading(ImageLoadingTask),
    /// The load finished with this result. Errors are kept too, so a failed image is not retried
    /// until its entry is removed.
    Loaded(Result<Arc<RenderImage>, ImageCacheError>),
}

impl std::fmt::Debug for ImageCacheItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status = match self {
            ImageCacheItem::Loading(_) => &"Loading...".to_string(),
            ImageCacheItem::Loaded(render_image) => &format!("{:?}", render_image),
        };
        f.debug_struct("ImageCacheItem")
            .field("status", status)
            .finish()
    }
}

impl ImageCacheItem {
    /// Returns the result if the load has finished, without waiting.
    ///
    /// Once the load has finished, the item is stored as [`ImageCacheItem::Loaded`]. Returns `None`
    /// while the load is still running.
    pub fn get(&mut self) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        match self {
            ImageCacheItem::Loading(task) => {
                let res = task.now_or_never()?;
                *self = ImageCacheItem::Loaded(res.clone());
                Some(res)
            }
            ImageCacheItem::Loaded(res) => Some(res.clone()),
        }
    }
}

/// A store of decoded images that [`img`](crate::img) elements read from.
///
/// Implement it to control how images load and when they are released. [`RetainAllImageCache`]
/// keeps every image until you clear it. Release images you no longer need, so they are removed
/// from all windows.
pub trait ImageCache: 'static {
    /// Returns the image for `resource` if it is ready, or `None` while it loads.
    ///
    /// This runs on every layout and paint of each image that uses the cache, so it must return
    /// quickly. Start the load in the background, return `None`, and notify the current view when
    /// the load finishes, as [`RetainAllImageCache`] does. The element is then drawn again.
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>>;
}

/// Creates the [`AnyImageCache`] for an [`image_cache`] element.
///
/// An `Entity` of an [`ImageCache`] implements it, and so does the provider from [`retain_all`].
pub trait ImageCacheProvider: 'static {
    /// Returns the cache for the children of an [`image_cache`] element.
    ///
    /// Called in both the layout and the paint phase.
    fn provide(&mut self, _window: &mut Window, _cx: &mut App) -> AnyImageCache;
}

impl<T: ImageCache> ImageCacheProvider for Entity<T> {
    fn provide(&mut self, _window: &mut Window, _cx: &mut App) -> AnyImageCache {
        self.clone().into()
    }
}

/// An [`ImageCache`] that keeps every image it loads until you clear it, remove a source, or drop
/// the cache.
///
/// Create one with [`RetainAllImageCache::new`], or let [`retain_all`] create one for an element.
/// Loads run on the background executor, and the view that asked is notified when each finishes.
/// Dropping the cache drops its decoded images from the windows.
pub struct RetainAllImageCache(HashMap<u64, ImageCacheItem>);

impl fmt::Debug for RetainAllImageCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HashMapImageCache")
            .field("num_images", &self.0.len())
            .finish()
    }
}

impl RetainAllImageCache {
    /// Creates a cache as an app entity. Its images are dropped from the windows when the entity is
    /// released.
    ///
    /// Keep the returned `Entity` alive for as long as its images are needed.
    #[inline]
    pub fn new(cx: &mut App) -> Entity<Self> {
        let e = cx.new(|_cx| RetainAllImageCache(HashMap::new()));
        cx.observe_release(&e, |image_cache, cx| {
            for (_, mut item) in std::mem::replace(&mut image_cache.0, HashMap::new()) {
                if let Some(Ok(image)) = item.get() {
                    cx.drop_image(image, None);
                }
            }
        })
        .detach();
        e
    }

    /// Returns the image for `source`, starting its load on the first request.
    ///
    /// Returns `None` while the image loads. After that, it returns the same result on every call,
    /// including an error. Requests for one source share a single load. See [`ImageCache::load`].
    pub fn load(
        &mut self,
        source: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let hash = hash(source);

        if let Some(item) = self.0.get_mut(&hash) {
            return item.get();
        }

        let fut = AssetLogger::<ImageAssetLoader>::load(source.clone(), cx);
        let task = cx.background_executor().spawn(fut).shared();
        self.0.insert(hash, ImageCacheItem::Loading(task.clone()));

        let entity = window.current_view();
        window
            .spawn(cx, {
                async move |cx| {
                    _ = task.await;
                    cx.on_next_frame(move |_, cx| {
                        cx.notify(entity);
                    });
                }
            })
            .detach();

        None
    }

    /// Removes every entry and drops the decoded images from the windows.
    ///
    /// A load still in progress is forgotten, so the next request for its source starts again.
    pub fn clear(&mut self, window: &mut Window, cx: &mut App) {
        for (_, mut item) in std::mem::replace(&mut self.0, HashMap::new()) {
            if let Some(Ok(image)) = item.get() {
                cx.drop_image(image, Some(window));
            }
        }
    }

    /// Removes one source and drops its decoded image from the windows. The next request for it
    /// loads it again.
    pub fn remove(&mut self, source: &Resource, window: &mut Window, cx: &mut App) {
        let hash = hash(source);
        if let Some(mut item) = self.0.remove(&hash)
            && let Some(Ok(image)) = item.get()
        {
            cx.drop_image(image, Some(window));
        }
    }

    /// Returns the number of sources in the cache, including ones still loading.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` if the cache holds no sources, including ones still loading.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl ImageCache for RetainAllImageCache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        RetainAllImageCache::load(self, resource, window, cx)
    }
}

/// Returns a provider for an [`image_cache`] element that keeps one [`RetainAllImageCache`] in
/// the element state under `id`.
///
/// The cache lasts as long as the element state, which is dropped when the element is no longer
/// rendered. Give each gallery its own `id`, so they don't share a cache. For the bigger picture,
/// see the [images guide](crate::_documentation::guides::images_and_assets).
///
/// # Examples
///
/// ```
/// # use gpui::*;
/// let thumbnails = image_cache(retain_all("thumbnails")).child(
///     img("https://example.com/thumb.png").size(px(64.)),
/// );
/// ```
pub fn retain_all(id: impl Into<ElementId>) -> RetainAllImageCacheProvider {
    RetainAllImageCacheProvider { id: id.into() }
}

/// The [`ImageCacheProvider`] that [`retain_all`] returns.
pub struct RetainAllImageCacheProvider {
    id: ElementId,
}

impl ImageCacheProvider for RetainAllImageCacheProvider {
    fn provide(&mut self, window: &mut Window, cx: &mut App) -> AnyImageCache {
        window
            .with_global_id(self.id.clone(), |global_id, window| {
                window.with_element_state::<Entity<RetainAllImageCache>, _>(
                    global_id,
                    |cache, _window| {
                        let mut cache = cache.unwrap_or_else(|| RetainAllImageCache::new(cx));
                        (cache.clone(), cache)
                    },
                )
            })
            .into()
    }
}
