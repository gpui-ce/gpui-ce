use crate::{
    AnyElement, AnyImageCache, App, AppContext, Asset, AssetLogger, Bounds, DefiniteLength,
    Element, ElementId, Entity, GlobalElementId, Hitbox, Image, ImageCache, InspectorElementId,
    InteractiveElement, Interactivity, IntoElement, LayoutId, Length, ObjectFit, Pixels,
    RenderImage, Resource, SharedString, SharedUri, StyleRefinement, Styled, Task, Window,
    decode_static_image, decode_static_image_from_decoder, px,
};
use anyhow::Result;

use futures::Future;
use gpui_util::ResultExt;
use image::{
    AnimationDecoder, ImageError, ImageFormat, Rgba,
    codecs::{gif::GifDecoder, webp::WebPDecoder},
};
use scheduler::Instant;
use smallvec::SmallVec;
use std::{
    fs,
    io::{self, Cursor},
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
    time::Duration,
};
use thiserror::Error;

use super::{Stateful, StatefulInteractiveElement};

/// How long an image may take to load before the [`StyledImage::with_loading`] placeholder appears.
///
/// An image that finishes within this delay never shows the placeholder, so fast loads don't flash.
pub const LOADING_DELAY: Duration = Duration::from_millis(200);

/// The asset loader that [`img`] uses for [`ImageSource::Resource`] sources: URLs, file paths and
/// [`crate::AssetRegistry`] entries.
///
/// The other [`ImageSource`] variants don't use it. Use `App::remove_asset::<ImgResourceLoader>`
/// with a [`Resource`] to evict one entry.
pub type ImgResourceLoader = AssetLogger<ImageAssetLoader>;

/// Where an [`img`] element gets its pixels.
///
/// You rarely build this yourself. Each argument to [`img`] converts into one variant: a string,
/// a [`PathBuf`](std::path::PathBuf) or a [`SharedUri`] becomes [`ImageSource::Resource`], an
/// `Arc<Image>` becomes [`ImageSource::Image`], and an `Arc<RenderImage>` becomes
/// [`ImageSource::Render`].
///
/// # How a string is read
///
/// A `&str`, `String` or `SharedString` is a URL if the `url` crate parses it, and an asset path
/// otherwise. Any scheme counts, so `"https://example.com/a.png"` is a URL, and so are
/// `"c:/logo.png"` (a Windows drive path) and `"data:..."`. URLs go to the app's HTTP client, which
/// normally rejects schemes it doesn't handle. `"images/logo.png"` has no scheme, so it is looked
/// up in the [`crate::AssetRegistry`].
///
/// To skip that check, convert a [`PathBuf`](std::path::PathBuf) or a [`SharedUri`] instead. A
/// `PathBuf` always reads the file system, and a `SharedUri` always fetches.
#[derive(Clone)]
pub enum ImageSource {
    /// Load the image from a [`Resource`]: a URL, a file path or a name in the [`crate::AssetRegistry`].
    ///
    /// The bytes load in the background and are cached by source.
    Resource(Resource),
    /// Pixels that are already decoded. Nothing is loaded, and the frames are drawn as they are.
    Render(Arc<RenderImage>),
    /// Encoded bytes in an [`Image`], such as a PNG or an SVG. The image is decoded in the
    /// background and cached under the [`Image`]'s ID, which is a hash of its bytes.
    Image(Arc<Image>),
    /// A function that the element calls on every layout and paint to get the image.
    ///
    /// Return `None` while the image is not ready, `Some(Ok(..))` with the image, or
    /// `Some(Err(..))` to show the fallback. gpui doesn't cache the result, so the function must be
    /// fast or cache its own result.
    Custom(Arc<dyn Fn(&mut Window, &mut App) -> Option<Result<Arc<RenderImage>, ImageCacheError>>>),
}

fn is_uri(uri: &str) -> bool {
    url::Url::from_str(uri).is_ok()
}

impl From<SharedUri> for ImageSource {
    fn from(value: SharedUri) -> Self {
        Self::Resource(Resource::Uri(value))
    }
}

impl<'a> From<&'a str> for ImageSource {
    fn from(s: &'a str) -> Self {
        if is_uri(s) {
            Self::Resource(Resource::Uri(s.to_string().into()))
        } else {
            Self::Resource(Resource::Embedded(s.to_string().into()))
        }
    }
}

impl From<String> for ImageSource {
    fn from(s: String) -> Self {
        if is_uri(&s) {
            Self::Resource(Resource::Uri(s.into()))
        } else {
            Self::Resource(Resource::Embedded(s.into()))
        }
    }
}

impl From<SharedString> for ImageSource {
    fn from(s: SharedString) -> Self {
        s.as_ref().into()
    }
}

impl From<&Path> for ImageSource {
    fn from(value: &Path) -> Self {
        Self::Resource(value.to_path_buf().into())
    }
}

impl From<Arc<Path>> for ImageSource {
    fn from(value: Arc<Path>) -> Self {
        Self::Resource(value.into())
    }
}

impl From<PathBuf> for ImageSource {
    fn from(value: PathBuf) -> Self {
        Self::Resource(value.into())
    }
}

impl From<Arc<RenderImage>> for ImageSource {
    fn from(value: Arc<RenderImage>) -> Self {
        Self::Render(value)
    }
}

impl From<Arc<Image>> for ImageSource {
    fn from(value: Arc<Image>) -> Self {
        Self::Image(value)
    }
}

impl<F> From<F> for ImageSource
where
    F: Fn(&mut Window, &mut App) -> Option<Result<Arc<RenderImage>, ImageCacheError>> + 'static,
{
    fn from(value: F) -> Self {
        Self::Custom(Arc::new(value))
    }
}

/// The settings that the [`StyledImage`] methods set on an image element.
///
/// Reach it through [`StyledImage::image_style`]. You don't create one directly.
pub struct ImageStyle {
    grayscale: bool,
    object_fit: ObjectFit,
    loading: Option<Box<dyn Fn() -> AnyElement>>,
    fallback: Option<Box<dyn Fn() -> AnyElement>>,
}

impl Default for ImageStyle {
    fn default() -> Self {
        Self {
            grayscale: false,
            object_fit: ObjectFit::Contain,
            loading: None,
            fallback: None,
        }
    }
}

/// Styling methods for image elements, implemented by [`Img`] and `Stateful<Img>`.
///
/// Each method returns `self`, so calls chain after [`img`].
pub trait StyledImage: Sized {
    /// Returns the [`ImageStyle`] that the methods on this trait write to.
    fn image_style(&mut self) -> &mut ImageStyle;

    /// Draws the image in grayscale when `grayscale` is `true`. The default is `false`.
    fn grayscale(mut self, grayscale: bool) -> Self {
        self.image_style().grayscale = grayscale;
        self
    }

    /// Sets how the image fills its box. The default is [`ObjectFit::Contain`].
    ///
    /// The fit matters only when the box's aspect ratio differs from the image's, so set both a
    /// width and a height. With no size set, the box is the image's natural size, and every fit
    /// looks the same.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::*;
    /// let avatar = img("images/avatar.png")
    ///     .size(px(48.))
    ///     .rounded_full()
    ///     .object_fit(ObjectFit::Cover);
    /// ```
    fn object_fit(mut self, object_fit: ObjectFit) -> Self {
        self.image_style().object_fit = object_fit;
        self
    }

    /// Sets the closure that builds the element to show when the image fails to load.
    ///
    /// Failures include a missing asset, a non-success HTTP status, bytes that don't decode, and a
    /// URL that the HTTP client can't fetch. The closure runs again on each layout where the
    /// fallback is shown, so keep it cheap.
    fn with_fallback(mut self, fallback: impl Fn() -> AnyElement + 'static) -> Self {
        self.image_style().fallback = Some(Box::new(fallback));
        self
    }

    /// Sets the closure that builds a placeholder to show while the image loads.
    ///
    /// The placeholder appears only if loading takes longer than [`LOADING_DELAY`], so a quick load
    /// never shows it. The placeholder and the fallback are children of the image element, so the
    /// element's own size and style still apply.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::*;
    /// let photo = img("https://example.com/photo.jpg")
    ///     .size(px(240.))
    ///     .with_loading(|| {
    ///         div()
    ///             .size(px(240.))
    ///             .bg(rgb(0xe5e7eb))
    ///             .into_any_element()
    ///     })
    ///     .with_fallback(|| {
    ///         div()
    ///             .size(px(240.))
    ///             .flex()
    ///             .items_center()
    ///             .justify_center()
    ///             .child("Image unavailable")
    ///             .into_any_element()
    ///     });
    /// ```
    fn with_loading(mut self, loading: impl Fn() -> AnyElement + 'static) -> Self {
        self.image_style().loading = Some(Box::new(loading));
        self
    }
}

impl StyledImage for Img {
    fn image_style(&mut self) -> &mut ImageStyle {
        &mut self.style
    }
}

impl StyledImage for Stateful<Img> {
    fn image_style(&mut self) -> &mut ImageStyle {
        &mut self.element.style
    }
}

/// An image element, made by [`img`].
///
/// It is a styled, interactive element. Style it with [`Styled`] and [`StyledImage`], and attach
/// event handlers with [`InteractiveElement`].
pub struct Img {
    interactivity: Interactivity,
    source: ImageSource,
    style: ImageStyle,
    image_cache: Option<AnyImageCache>,
}

/// Creates an image element that draws `source`.
///
/// Building the element does no work. The image starts loading the first time the element is laid
/// out, on a background thread. Until it loads, the element has no size unless you set one, and it
/// draws nothing unless you set a [`StyledImage::with_loading`] placeholder. Once the image loads,
/// the element takes the image's natural size. If you set only a width or only a height, the other
/// dimension follows the aspect ratio.
///
/// `source` is anything that converts into an [`ImageSource`]. That type explains how a string is
/// read.
///
/// Loaded images are cached, so a second `img` with the same source reuses the decoded image. See
/// the [images guide](crate::_documentation::guides::images_and_assets) for the cache rules, and
/// [`crate::image_cache`] to scope them to a group of images.
///
/// # Examples
///
/// ```
/// # use gpui::*;
/// let remote = img("https://example.com/avatar.png");
/// let bundled = img("images/logo.png");
/// let local = img(std::path::PathBuf::from("assets/banner.jpg"));
/// ```
#[track_caller]
pub fn img(source: impl Into<ImageSource>) -> Img {
    Img {
        interactivity: Interactivity::new(),
        source: source.into(),
        style: ImageStyle::default(),
        image_cache: None,
    }
}

impl Img {
    /// Returns the file extensions that `img` can decode, in lowercase and without the dot.
    ///
    /// The list is the extensions of `image::ImageFormat` plus `svg`. gpui decides whether a file
    /// loads by reading its bytes, not its extension, so the list is a guide.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::*;
    /// assert!(Img::extensions().contains(&"png"));
    /// assert!(Img::extensions().contains(&"svg"));
    /// ```
    pub fn extensions() -> &'static [&'static str] {
        // This is the list in [image::ImageFormat::from_extension] + `svg`
        &[
            "avif", "jpg", "jpeg", "png", "gif", "webp", "tif", "tiff", "tga", "dds", "bmp", "ico",
            "hdr", "exr", "pbm", "pam", "ppm", "pgm", "ff", "farbfeld", "qoi", "svg",
        ]
    }

    /// Uses `image_cache` for this image, instead of the cache from an ancestor.
    ///
    /// Without this call, the image uses the cache of the nearest ancestor [`crate::image_cache`] element,
    /// or the app-wide asset cache if there is none. Only [`ImageSource::Resource`] sources go
    /// through an image cache. The other variants ignore it.
    #[inline]
    pub fn image_cache<I: ImageCache>(self, image_cache: &Entity<I>) -> Self {
        Self {
            image_cache: Some(image_cache.clone().into()),
            ..self
        }
    }
}

impl Deref for Stateful<Img> {
    type Target = Img;

    fn deref(&self) -> &Self::Target {
        &self.element
    }
}

impl DerefMut for Stateful<Img> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.element
    }
}

/// Frames shorter than this are treated as this long, so zero-delay frames can't stall playback.
const MIN_FRAME_DELAY: Duration = Duration::from_millis(1);

/// Steps an animation forward by `elapsed`, returning the new frame index and the time already
/// spent in that frame. Advances through every frame covered by `elapsed`, including multiple
/// frames when one update spans several frame intervals.
fn advance_animation(
    mut frame_index: usize,
    elapsed: Duration,
    frame_count: usize,
    frame_delay: impl Fn(usize) -> Duration,
) -> (usize, Duration) {
    if frame_count == 0 {
        return (0, elapsed);
    }

    let delay = |index| frame_delay(index).max(MIN_FRAME_DELAY);
    frame_index %= frame_count;

    // Whole cycles end on the same frame, so skip them rather than stepping through each.
    let cycle_nanos = (0..frame_count)
        .map(|index| delay(index).as_nanos())
        .sum::<u128>();
    let remainder_nanos = elapsed.as_nanos() % cycle_nanos;
    let mut remaining = Duration::new(
        (remainder_nanos / 1_000_000_000) as u64,
        (remainder_nanos % 1_000_000_000) as u32,
    );

    while remaining >= delay(frame_index) {
        remaining -= delay(frame_index);
        frame_index = (frame_index + 1) % frame_count;
    }
    (frame_index, remaining)
}

/// The image state between frames
struct ImgState {
    frame_index: usize,
    last_frame_time: Option<Instant>,
    started_loading: Option<(Instant, Task<()>)>,
}

/// The per-frame state that an [`Img`] passes from layout to paint.
///
/// It holds the frame of an animated image to draw, and the placeholder or fallback element when
/// one is showing.
pub struct ImgLayoutState {
    frame_index: usize,
    replacement: Option<AnyElement>,
}

impl Element for Img {
    type RequestLayoutState = ImgLayoutState;
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<ElementId> {
        self.interactivity.element_id.clone()
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        self.interactivity.source_location()
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut layout_state = ImgLayoutState {
            frame_index: 0,
            replacement: None,
        };

        window.with_optional_element_state(global_id, |state, window| {
            let mut state = state.map(|state| {
                state.unwrap_or(ImgState {
                    frame_index: 0,
                    last_frame_time: None,
                    started_loading: None,
                })
            });

            let mut frame_index = state.as_ref().map(|state| state.frame_index).unwrap_or(0);

            let layout_id = self.interactivity.request_layout(
                global_id,
                inspector_id,
                window,
                cx,
                |mut style, window, cx| {
                    let mut replacement_id = None;

                    match self.source.use_data(
                        self.image_cache
                            .clone()
                            .or_else(|| window.image_cache_stack.last().cloned()),
                        window,
                        cx,
                    ) {
                        Some(Ok(data)) => {
                            let frame_count = data.frame_count();
                            let max_frame_index = frame_count.saturating_sub(1);

                            if let Some(state) = &mut state {
                                state.frame_index = state.frame_index.min(max_frame_index);
                                if frame_count > 1 && !cx.reduce_motion() {
                                    if window.is_window_active() {
                                        let current_time = cx.background_executor().now();
                                        if let Some(last_frame_time) = state.last_frame_time {
                                            let elapsed = current_time - last_frame_time;
                                            let (frame_index, remainder) = advance_animation(
                                                state.frame_index,
                                                elapsed,
                                                frame_count,
                                                |index| Duration::from(data.delay(index)),
                                            );
                                            state.frame_index = frame_index;
                                            state.last_frame_time = Some(current_time - remainder);
                                        } else {
                                            state.last_frame_time = Some(current_time);
                                        }
                                    } else {
                                        state.last_frame_time = None;
                                    }
                                } else {
                                    state.last_frame_time = None;
                                }
                                state.started_loading = None;
                                frame_index = state.frame_index;
                            }

                            let image_size = data.render_size(frame_index);

                            if style.aspect_ratio.is_none() {
                                style.aspect_ratio = Some(image_size.width / image_size.height);
                            }

                            if let Length::Auto = style.size.width {
                                style.size.width = match style.size.height {
                                    Length::Definite(DefiniteLength::Absolute(abs_length)) => {
                                        let height_px = abs_length.to_pixels(window.rem_size());
                                        Length::Definite(
                                            px(image_size.width.0 * height_px.0
                                                / image_size.height.0)
                                            .into(),
                                        )
                                    }
                                    _ => Length::Definite(image_size.width.into()),
                                };
                            }

                            if let Length::Auto = style.size.height {
                                style.size.height = match style.size.width {
                                    Length::Definite(DefiniteLength::Absolute(abs_length)) => {
                                        let width_px = abs_length.to_pixels(window.rem_size());
                                        Length::Definite(
                                            px(image_size.height.0 * width_px.0
                                                / image_size.width.0)
                                            .into(),
                                        )
                                    }
                                    _ => Length::Definite(image_size.height.into()),
                                };
                            }

                            if global_id.is_some()
                                && data.frame_count() > 1
                                && window.is_window_active()
                                && !cx.reduce_motion()
                            {
                                window.request_animation_frame();
                            }
                        }
                        Some(_err) => {
                            if let Some(fallback) = self.style.fallback.as_ref() {
                                let mut element = fallback();
                                replacement_id = Some(element.request_layout(window, cx));
                                layout_state.replacement = Some(element);
                            }
                            if let Some(state) = &mut state {
                                state.started_loading = None;
                            }
                        }
                        None => {
                            if let Some(state) = &mut state {
                                if let Some((started_loading, _)) = state.started_loading {
                                    if cx.background_executor().now() - started_loading
                                        > LOADING_DELAY
                                        && let Some(loading) = self.style.loading.as_ref()
                                    {
                                        let mut element = loading();
                                        replacement_id = Some(element.request_layout(window, cx));
                                        layout_state.replacement = Some(element);
                                    }
                                } else {
                                    let current_view = window.current_view();
                                    let task = window.spawn(cx, async move |cx| {
                                        cx.background_executor().timer(LOADING_DELAY).await;
                                        cx.update(move |_, cx| {
                                            cx.notify(current_view);
                                        })
                                        .ok();
                                    });
                                    state.started_loading =
                                        Some((cx.background_executor().now(), task));
                                }
                            }
                        }
                    }

                    window.request_layout(style, replacement_id, cx)
                },
            );

            layout_state.frame_index = frame_index;

            ((layout_id, layout_state), state)
        })
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.interactivity.prepaint(
            global_id,
            inspector_id,
            bounds,
            bounds.size,
            window,
            cx,
            |_, _, hitbox, window, cx| {
                if let Some(replacement) = &mut request_layout.replacement {
                    replacement.prepaint(window, cx);
                }

                hitbox
            },
        )
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout_state: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let source = self.source.clone();
        self.interactivity.paint(
            global_id,
            inspector_id,
            bounds,
            hitbox.as_ref(),
            window,
            cx,
            |style, window, cx| {
                if let Some(Ok(data)) = source.use_data(
                    self.image_cache
                        .clone()
                        .or_else(|| window.image_cache_stack.last().cloned()),
                    window,
                    cx,
                ) {
                    if data.frame_count() == 0 {
                        return;
                    }
                    let new_bounds = self
                        .style
                        .object_fit
                        .get_bounds(bounds, data.size(layout_state.frame_index));
                    let corner_radii = style.corner_radii.to_pixels(window.rem_size());
                    window
                        .paint_image(
                            bounds,
                            new_bounds,
                            corner_radii,
                            style.corner_smoothing.unwrap_or_default(),
                            data,
                            layout_state.frame_index,
                            self.style.grayscale,
                        )
                        .log_err();
                } else if let Some(replacement) = &mut layout_state.replacement {
                    replacement.paint(window, cx);
                }
            },
        )
    }
}

impl Styled for Img {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.interactivity.base_style
    }
}

impl InteractiveElement for Img {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}

impl IntoElement for Img {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl StatefulInteractiveElement for Img {}

impl ImageSource {
    pub(crate) fn use_data(
        &self,
        cache: Option<AnyImageCache>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        match self {
            ImageSource::Resource(resource) => {
                if let Some(cache) = cache {
                    cache.load(resource, window, cx)
                } else {
                    window.use_asset::<ImgResourceLoader>(resource, cx)
                }
            }
            ImageSource::Custom(loading_fn) => loading_fn(window, cx),
            ImageSource::Render(data) => Some(Ok(data.to_owned())),
            ImageSource::Image(data) => window.use_asset::<AssetLogger<ImageDecoder>>(data, cx),
        }
    }

    pub(crate) fn get_data(
        &self,
        cache: Option<AnyImageCache>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        match self {
            ImageSource::Resource(resource) => {
                if let Some(cache) = cache {
                    cache.load(resource, window, cx)
                } else {
                    window.get_asset::<ImgResourceLoader>(resource, cx)
                }
            }
            ImageSource::Custom(loading_fn) => loading_fn(window, cx),
            ImageSource::Render(data) => Some(Ok(data.to_owned())),
            ImageSource::Image(data) => window.get_asset::<AssetLogger<ImageDecoder>>(data, cx),
        }
    }

    /// Evicts this source from the app's asset cache, so the next `img` that uses it loads it again.
    ///
    /// Only [`ImageSource::Resource`] and [`ImageSource::Image`] are in that cache, so the other
    /// variants do nothing. Images held by an [`crate::image_cache`] element are not affected.
    pub fn remove_asset(&self, cx: &mut App) {
        match self {
            ImageSource::Resource(resource) => {
                cx.remove_asset::<ImgResourceLoader>(resource);
            }
            ImageSource::Custom(_) | ImageSource::Render(_) => {}
            ImageSource::Image(data) => cx.remove_asset::<AssetLogger<ImageDecoder>>(data),
        }
    }

    /// Returns `true` if this source is loading or loaded in the app's asset cache. Starts no load.
    ///
    /// Available with the `test-support` feature. Use it in tests to check that an image was
    /// requested or evicted.
    #[cfg(any(test, feature = "test-support"))]
    pub fn is_asset_cached(&self, cx: &App) -> bool {
        match self {
            ImageSource::Resource(resource) => cx.has_asset::<ImgResourceLoader>(resource),
            ImageSource::Custom(_) | ImageSource::Render(_) => false,
            ImageSource::Image(data) => cx.has_asset::<AssetLogger<ImageDecoder>>(data),
        }
    }
}

#[derive(Clone)]
enum ImageDecoder {}

impl Asset for ImageDecoder {
    type Source = Arc<Image>;
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        let renderer = cx.svg_renderer();
        async move { source.to_image_data(renderer).map_err(Into::into) }
    }
}

/// Loads and decodes a [`Resource`] into a [`RenderImage`]. [`img`] uses it, through
/// [`ImgResourceLoader`], for URLs, file paths and registry entries.
///
/// The bytes come from the source. URLs go through the HTTP client and must return a success
/// status. Paths are read from disk. Registry names are read from the [`crate::AssetRegistry`]. The format
/// is then guessed from the bytes. Anything the `image` crate recognizes is decoded, and every
/// frame that decodes is kept for GIF and animated WebP. Anything else is parsed as SVG.
#[derive(Clone)]
pub enum ImageAssetLoader {}

impl Asset for ImageAssetLoader {
    type Source = Resource;
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        let client = cx.http_client();
        // TODO: Can we make SVGs always rescale?
        // let scale_factor = cx.scale_factor();
        let svg_renderer = cx.svg_renderer();
        let asset_registry = cx.assets().clone();
        async move {
            let bytes = match source.clone() {
                Resource::Path(uri) => fs::read(uri.as_ref())?,
                Resource::Uri(uri) => {
                    use anyhow::Context as _;

                    let response = client
                        .get(uri.as_ref(), true)
                        .await
                        .with_context(|| format!("loading image asset from {uri:?}"))?;
                    if !response.status.is_success() {
                        let mut error_body = String::from_utf8_lossy(&response.body).into_owned();
                        let first_line = error_body.lines().next().unwrap_or("").trim_end();
                        error_body.truncate(first_line.len());
                        return Err(ImageCacheError::BadStatus {
                            uri,
                            status: response.status,
                            body: error_body,
                        });
                    }
                    response.body
                }
                Resource::Embedded(path) => {
                    let data = asset_registry.load(&path);
                    if let Some(data) = data {
                        data.to_vec()
                    } else {
                        return Err(ImageCacheError::Asset(
                            format!("Embedded resource not found: {}", path).into(),
                        ));
                    }
                }
            };

            if let Ok(format) = image::guess_format(&bytes) {
                let data = match format {
                    ImageFormat::Gif => {
                        let decoder = GifDecoder::new(Cursor::new(&bytes))?;
                        let mut frames = SmallVec::new();

                        for frame in decoder.into_frames() {
                            match frame {
                                Ok(mut frame) => {
                                    // Convert from RGBA to BGRA.
                                    for pixel in frame.buffer_mut().chunks_exact_mut(4) {
                                        pixel.swap(0, 2);
                                    }
                                    frames.push(frame);
                                }
                                Err(err) => {
                                    log::debug!(
                                        "Skipping GIF frame in {source:?} due to decode error: {err}"
                                    );
                                }
                            }
                        }

                        if frames.is_empty() {
                            return Err(ImageCacheError::Other(Arc::new(anyhow::anyhow!(
                                "GIF could not be decoded: all frames failed ({source:?})"
                            ))));
                        }

                        frames
                    }
                    ImageFormat::WebP => {
                        let mut decoder = WebPDecoder::new(Cursor::new(&bytes))?;

                        if decoder.has_animation() {
                            let _ = decoder.set_background_color(Rgba([0, 0, 0, 0]));
                            let mut frames = SmallVec::new();

                            for frame in decoder.into_frames() {
                                match frame {
                                    Ok(mut frame) => {
                                        // Convert from RGBA to BGRA.
                                        for pixel in frame.buffer_mut().chunks_exact_mut(4) {
                                            pixel.swap(0, 2);
                                        }
                                        frames.push(frame);
                                    }
                                    Err(err) => {
                                        log::debug!(
                                            "Skipping WebP frame in {source:?} due to decode error: {err}"
                                        );
                                    }
                                }
                            }

                            if frames.is_empty() {
                                return Err(ImageCacheError::Other(Arc::new(anyhow::anyhow!(
                                    "WebP could not be decoded: all frames failed ({source:?})"
                                ))));
                            }

                            frames
                        } else {
                            decode_static_image_from_decoder(decoder)?
                        }
                    }
                    _ => decode_static_image(&bytes, format)?,
                };

                Ok(Arc::new(RenderImage::new(data)))
            } else {
                svg_renderer
                    .render_single_frame(&bytes, 1.0)
                    .map_err(Into::into)
            }
        }
    }
}

/// Why an image failed to load or decode.
///
/// [`StyledImage::with_fallback`] is shown when you get one of these. The cache keeps the error
/// with the image, so every element that shows the image sees the same error.
#[derive(Debug, Error, Clone)]
pub enum ImageCacheError {
    /// A general failure, such as a failed HTTP request, or a GIF or WebP whose frames all failed
    /// to decode.
    #[error("error: {0}")]
    Other(#[from] Arc<anyhow::Error>),
    /// A file could not be read, such as a [`Resource::Path`] that doesn't exist.
    #[error("IO error: {0}")]
    Io(Arc<std::io::Error>),
    /// The HTTP response had a status other than success.
    #[error("unexpected http status for {uri}: {status}, body: {body}")]
    BadStatus {
        /// The URI that was requested.
        uri: SharedUri,
        /// The HTTP status code.
        status: http::StatusCode,
        /// The first line of the response body.
        body: String,
    },
    /// A name was not found in the [`crate::AssetRegistry`].
    #[error("asset error: {0}")]
    Asset(SharedString),
    /// The `image` crate could not decode the bytes.
    #[error("image error: {0}")]
    Image(Arc<ImageError>),
    /// The SVG could not be parsed or rendered.
    #[error("svg error: {0}")]
    Usvg(Arc<usvg::Error>),
}

impl From<anyhow::Error> for ImageCacheError {
    fn from(value: anyhow::Error) -> Self {
        Self::Other(Arc::new(value))
    }
}

impl From<io::Error> for ImageCacheError {
    fn from(value: io::Error) -> Self {
        Self::Io(Arc::new(value))
    }
}

impl From<usvg::Error> for ImageCacheError {
    fn from(value: usvg::Error) -> Self {
        Self::Usvg(Arc::new(value))
    }
}

impl From<image::ImageError> for ImageCacheError {
    fn from(value: image::ImageError) -> Self {
        Self::Image(Arc::new(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Context, ParentElement as _, Render, TestAppContext, Window, canvas, div, point, px, size,
    };
    use image::{Delay, Frame, ImageBuffer, Rgba};

    const TEST_IMG_ID: &str = "test-img";

    fn test_image(frame_count: usize) -> Arc<RenderImage> {
        let frame = Frame::new(ImageBuffer::from_pixel(1, 1, Rgba([0, 0, 0, 0])));
        Arc::new(RenderImage::new(SmallVec::from_iter(
            (0..frame_count).map(|_| frame.clone()),
        )))
    }

    fn test_image_with_size(width: u32, height: u32) -> Arc<RenderImage> {
        let frame = Frame::new(ImageBuffer::from_pixel(width, height, Rgba([0, 0, 0, 0])));
        Arc::new(RenderImage::new(SmallVec::from_elem(frame, 1)))
    }

    /// An animation whose frames are 1, 2 and 3 pixels wide, lasting 100, 200 and 300 ms.
    fn animated_image() -> Arc<RenderImage> {
        let frames = [(1, 100), (2, 200), (3, 300)].map(|(width, delay_ms)| {
            Frame::from_parts(
                ImageBuffer::from_pixel(width, 1, Rgba([255, 0, 0, 255])),
                0,
                0,
                Delay::from_numer_denom_ms(delay_ms, 1),
            )
        });
        Arc::new(RenderImage::new(SmallVec::from_iter(frames)))
    }

    struct AnimatedImageView(Arc<RenderImage>);

    impl Render for AnimatedImageView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            img(ImageSource::Render(self.0.clone()))
                .id(TEST_IMG_ID)
                .size_full()
        }
    }

    #[test]
    fn advance_animation_steps_through_every_covered_frame() {
        let delays = [100, 200, 300].map(Duration::from_millis);
        let advance = |frame, elapsed_ms| {
            advance_animation(frame, Duration::from_millis(elapsed_ms), 3, |ix| delays[ix])
        };

        assert_eq!(advance(0, 0), (0, Duration::ZERO));
        assert_eq!(advance(0, 99), (0, Duration::from_millis(99)));
        assert_eq!(advance(0, 340), (2, Duration::from_millis(40)));
        assert_eq!(advance(2, 375), (0, Duration::from_millis(75)));
        // A full cycle plus 150 ms: back to the first frame, 50 ms into the second.
        assert_eq!(advance(0, 750), (1, Duration::from_millis(50)));
    }

    #[test]
    fn advance_animation_skips_whole_cycles_and_bounds_zero_delay_frames() {
        let delays = [
            Duration::ZERO,
            Duration::from_millis(2),
            Duration::from_millis(3),
        ];
        // Zero delays count as 1 ms, so a cycle lasts 6 ms.
        let elapsed = Duration::from_millis(4);
        let expected = advance_animation(1, elapsed, 3, |ix| delays[ix]);
        assert_eq!(
            advance_animation(1, elapsed + Duration::from_millis(6) * 500_000, 3, |ix| {
                delays[ix]
            }),
            expected
        );
        assert_eq!(
            advance_animation(0, Duration::from_millis(1), 2, |_| Duration::ZERO),
            (1, Duration::ZERO)
        );
    }

    #[gpui::test]
    fn animated_image_follows_the_executor_clock(cx: &mut TestAppContext) {
        let (_, window) = cx.add_window_view(|_, _| AnimatedImageView(animated_image()));
        window.update(|window, _| window.activate());
        let clock = window.cx.background_executor.clone();
        let frame_width_after = |window: &mut crate::VisualTestContext, elapsed_ms: u64| {
            clock.advance_clock(Duration::from_millis(elapsed_ms));
            window.update(|window, cx| {
                window.simulate_next_frame(cx);
                window.draw(cx).clear(cx);
                window
                    .rendered_frame
                    .scene
                    .polychrome_sprites
                    .last()
                    .expect("animated image should paint a sprite")
                    .tile
                    .bounds
                    .size
                    .width
                    .0
            })
        };

        for (elapsed_ms, expected_width) in [(0, 1), (99, 1), (1, 2), (1_100, 1), (99, 1), (1, 2)] {
            assert_eq!(
                frame_width_after(window, elapsed_ms),
                expected_width,
                "after {elapsed_ms} ms"
            );
        }
    }

    /// Overwrites the cached `frame_index` of the sibling `img` during paint.
    fn seed_frame_index(frame_index: usize) -> impl IntoElement {
        canvas(
            |_, _, _| (),
            move |_, _, window, _| {
                window.with_global_id(TEST_IMG_ID.into(), |id, window| {
                    window.with_element_state::<ImgState, _>(id, |state, _| {
                        let mut state = state.expect("img state should be initialized");
                        state.frame_index = frame_index;
                        ((), state)
                    });
                });
            },
        )
    }

    #[gpui::test]
    fn zero_frame_image_does_not_panic_on_paint(cx: &mut TestAppContext) {
        cx.add_empty_window()
            .draw(point(px(0.), px(0.)), size(px(100.), px(100.)), |_, _| {
                img(ImageSource::Render(test_image(0))).into_any_element()
            });
    }

    #[gpui::test]
    fn image_object_fit_cover_crops_to_element_bounds(cx: &mut TestAppContext) {
        let window = cx.add_empty_window();
        let image = test_image_with_size(200, 100);
        window.draw(point(px(0.), px(0.)), size(px(100.), px(100.)), |_, _| {
            img(ImageSource::Render(image.clone()))
                .size_full()
                .object_fit(ObjectFit::Fill)
                .into_any_element()
        });
        let full_tile_bounds = window.update(|window, _| {
            window
                .rendered_frame
                .scene
                .polychrome_sprites
                .last()
                .expect("fill image should paint a sprite")
                .tile
                .bounds
        });

        window.draw(point(px(10.), px(20.)), size(px(100.), px(100.)), |_, _| {
            img(ImageSource::Render(image))
                .size_full()
                .object_fit(ObjectFit::Cover)
                .into_any_element()
        });

        let (rendered_bounds, rendered_tile_bounds, scale_factor) = window.update(|window, _| {
            let sprite = window
                .rendered_frame
                .scene
                .polychrome_sprites
                .last()
                .expect("cover image should paint a sprite");
            (sprite.bounds, sprite.tile.bounds, window.scale_factor())
        });
        assert_eq!(
            rendered_bounds,
            Bounds {
                origin: point(px(10.).scale(scale_factor), px(20.).scale(scale_factor)),
                size: size(px(100.).scale(scale_factor), px(100.).scale(scale_factor)),
            }
        );
        assert_eq!(
            (
                rendered_tile_bounds.origin.x.0 - full_tile_bounds.origin.x.0,
                rendered_tile_bounds.origin.y.0 - full_tile_bounds.origin.y.0,
                rendered_tile_bounds.size.width.0,
                rendered_tile_bounds.size.height.0,
            ),
            (50, 0, 100, 100),
        );
    }

    #[gpui::test]
    fn explicit_aspect_ratio_is_not_overridden_by_intrinsic_ratio(cx: &mut TestAppContext) {
        let window = cx.add_empty_window();

        // A portrait image in a square container
        window.draw(point(px(0.), px(0.)), size(px(100.), px(100.)), |_, _| {
            div()
                .size(px(100.))
                .overflow_hidden()
                .child(
                    img(ImageSource::Render(test_image_with_size(100, 200)))
                        .size_full()
                        .aspect_square()
                        .object_fit(ObjectFit::Contain),
                )
                .into_any_element()
        });

        let (rendered_bounds, scale_factor) = window.update(|window, _| {
            let sprite = window
                .rendered_frame
                .scene
                .polychrome_sprites
                .last()
                .expect("contained image should paint a sprite");
            (sprite.bounds, window.scale_factor())
        });

        // The element stays 100x100, so the image is letterboxed horizontally
        assert_eq!(
            rendered_bounds,
            Bounds {
                origin: point(px(25.).scale(scale_factor), px(0.).scale(scale_factor)),
                size: size(px(50.).scale(scale_factor), px(100.).scale(scale_factor)),
            }
        );
    }

    #[gpui::test]
    fn image_object_fit_cover_clamps_corner_radii_to_visible_bounds(cx: &mut TestAppContext) {
        let window = cx.add_empty_window();
        window.draw(point(px(0.), px(0.)), size(px(100.), px(100.)), |_, _| {
            img(ImageSource::Render(test_image_with_size(200, 100)))
                .size_full()
                .rounded(px(100.))
                .object_fit(ObjectFit::Cover)
                .into_any_element()
        });

        let (corner_radius, expected_corner_radius) = window.update(|window, _| {
            (
                window
                    .rendered_frame
                    .scene
                    .polychrome_sprites
                    .last()
                    .map(|sprite| sprite.corner_radii.top_left),
                px(50.).scale(window.scale_factor()),
            )
        });
        assert_eq!(corner_radius, Some(expected_corner_radius));
    }

    #[gpui::test]
    fn stale_frame_index_is_clamped_when_image_changes(cx: &mut TestAppContext) {
        let window = cx.add_empty_window();

        // Assert that a cached frame_index from a previous multi-frame image
        // does not cause an out-of-bounds panic when the image is replaced
        // with one that has fewer frames.
        window.draw(point(px(0.), px(0.)), size(px(100.), px(100.)), |_, _| {
            div()
                .child(img(ImageSource::Render(test_image(5))).id(TEST_IMG_ID))
                .child(seed_frame_index(4))
                .into_any_element()
        });
        window.draw(point(px(0.), px(0.)), size(px(100.), px(100.)), |_, _| {
            img(ImageSource::Render(test_image(1)))
                .id(TEST_IMG_ID)
                .into_any_element()
        });
    }
}
