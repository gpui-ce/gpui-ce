use crate::{DevicePixels, Pixels, Result, SharedString, Size, size};
use gpui_util::ResultExt;
use smallvec::SmallVec;

use image::{Delay, EncodableLayout, Frame};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    fmt,
    hash::Hash,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering::SeqCst},
    },
};

/// One way to store a set of assets for gpui to access.
/// Can be provided to [`AssetRegistry`] to load asset binaries on-demand.
/// Alternatively, assets can be provided pre-loaded to the registry while still using this api (though there are newer apis to do so).
///
/// Generally considered deprecated for new usages since [`AssetRegistry`] has more specific
/// support for bundled, pre-loaded, and on-demand asset loading.
pub trait AssetSource: 'static + Send + Sync {
    /// Load the given asset from the source path.
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>>;

    /// List the assets at the given path.
    fn list(&self, path: &str) -> Result<Vec<SharedString>>;
}

impl AssetSource for () {
    fn load(&self, _path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(None)
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(vec![])
    }
}

/// Alias for a Cow byte vec, representing an asset that has been loaded.
pub type AssetData = Cow<'static, [u8]>;
/// A thread-safe callback to load an asset, possibly from disk.
pub type FnAssetLoader = Arc<dyn Fn() -> Option<AssetData> + 'static + Send + Sync>;
/// The entry stored in the [`AssetRegistry`].
pub enum AssetEntry {
    /// The asset is pre-loaded. It may be stored staticlly/embedded, or has already been loaded from disk.
    PreLoaded(AssetData),
    /// The entry should be loaded on demand.
    OnDemand(FnAssetLoader),
}
impl From<AssetData> for AssetEntry {
    fn from(value: AssetData) -> Self {
        Self::PreLoaded(value)
    }
}
impl<F> From<F> for AssetEntry
where
    F: Fn() -> Option<AssetData> + 'static + Send + Sync,
{
    fn from(value: F) -> Self {
        Self::OnDemand(Arc::new(value))
    }
}

/// Caused when multiple assets are registered to the [`AssetRegistry`] with the same path.
#[derive(thiserror::Error, Debug)]
#[error("An asset with the pathkey {0:?} already exists")]
pub struct DuplicateAssetPath(SharedString);

/// Collection of assets known to the [`App`](crate::App).
///
/// Users can provide assets via [`AssetSource`] or directly via `from_iter`, `insert`, `extend_from_source`, or `extend_from_embed`.
#[derive(Default)]
pub struct AssetRegistry {
    entries: BTreeMap<SharedString, AssetEntry>,
}

impl<T: AssetSource + 'static> From<T> for AssetRegistry {
    fn from(value: T) -> Self {
        let shared: Arc<dyn AssetSource> = Arc::new(value);
        Self::from(shared)
    }
}

impl From<Arc<dyn AssetSource>> for AssetRegistry {
    fn from(value: Arc<dyn AssetSource>) -> Self {
        let mut assets = Self::default();
        assets.extend_from_source(value);
        assets
    }
}

impl<KeyType, ValueType> FromIterator<(KeyType, ValueType)> for AssetRegistry
where
    KeyType: AsRef<str>,
    ValueType: Into<AssetEntry>,
{
    fn from_iter<T: IntoIterator<Item = (KeyType, ValueType)>>(iter: T) -> Self {
        iter.into_iter()
            .fold(Self::default(), |mut assets, (key, value)| {
                assets.insert(key.as_ref(), value).log_err();
                assets
            })
    }
}

impl AssetRegistry {
    /// Inserts a single asset into the registry. The asset can be static (embedded/bundled, `&'static [u8]`), pre-loaded ([`Vec<u8>`]), or loaded-on-demand ([`FnAssetLoader`]).
    /// If there is already an entry at the provided key/path, an error is returned.
    pub fn insert(
        &mut self,
        path: impl Into<SharedString>,
        asset: impl Into<AssetEntry>,
    ) -> Result<(), DuplicateAssetPath> {
        let path = path.into();
        if self.entries.contains_key(&path) {
            return Err(DuplicateAssetPath(path));
        }
        self.entries.insert(path, asset.into());
        Ok(())
    }

    /// Returns the asset data for a given path/key. If the asset is static/embedded or pre-loaded,
    /// this will return [`Cow::Borrowed(&'this [u8])`] (the data may be 'static lifetime aka embedded/bundled or simply owned by the registry).
    /// If the asset mapped to the path is load-on-demand, the loader is queried and its result returned as [`Cow::Owned(Vec<u8>)`].
    pub fn load<'this>(&'this self, path: impl AsRef<str>) -> Option<Cow<'this, [u8]>> {
        match self.entries.get(path.as_ref())? {
            AssetEntry::PreLoaded(Cow::Borrowed(data)) => Some(Cow::Borrowed(data)),
            AssetEntry::PreLoaded(Cow::Owned(data)) => Some(Cow::Borrowed(data.as_bytes())),
            AssetEntry::OnDemand(loader) => loader(),
        }
    }

    /// Iterates over the provided paths and inserts the asset returned by `get` at that path.
    /// Errors from insert are logged, but not returned and do not prevent further iteration.
    pub fn extend<KeyType, EntryType>(
        &mut self,
        paths: impl Iterator<Item = KeyType>,
        get: impl Fn(&str) -> Option<EntryType>,
    ) where
        KeyType: AsRef<str>,
        EntryType: Into<AssetEntry>,
    {
        for path in paths {
            let Some(data) = get(path.as_ref()) else {
                continue;
            };
            self.insert(path.as_ref(), data).log_err();
        }
    }

    /// Extends the known assets with all those stored by the source.
    ///
    /// Assets are stored as only loaded-on-demand, they are not preloaded/moved into the registry.
    /// This assumes that since the user is providing a source, they want to own the data.
    pub fn extend_from_source(&mut self, source: impl Into<Arc<dyn AssetSource>>) {
        let source = source.into();
        let Ok(paths) = source.list("") else { return };
        self.extend(paths.into_iter(), |path: &str| {
            use gpui_util::ResultExt;
            let source = source.clone();
            let path = SharedString::from(path);
            // constructing OnDemand for loading assets from shared:Arc only when they are requested
            Some(AssetEntry::from(move || {
                source.load(path.as_str()).log_err().flatten()
            }))
        });
    }

    /// Convenience method provided to easily insert embedded assets via RustEmbed.
    #[cfg(feature = "embedded-assets")]
    pub fn extend_from_embed<T: rust_embed::RustEmbed>(&mut self) {
        self.extend(T::iter(), |path| {
            T::get(path).map(|file| AssetEntry::PreLoaded(file.data))
        });
    }
}

/// A unique identifier for the image cache
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ImageId(pub usize);

#[derive(PartialEq, Eq, Hash, Clone)]
#[expect(missing_docs)]
pub struct RenderImageParams {
    pub image_id: ImageId,
    pub frame_index: usize,
}

/// A cached and processed image, in BGRA format
pub struct RenderImage {
    /// The ID associated with this image
    pub id: ImageId,
    /// The scale factor of this image on render.
    pub(crate) scale_factor: f32,
    data: SmallVec<[Frame; 1]>,
}

impl PartialEq for RenderImage {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for RenderImage {}

impl RenderImage {
    /// Create a new image from the given data.
    pub fn new(data: impl Into<SmallVec<[Frame; 1]>>) -> Self {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

        Self {
            id: ImageId(NEXT_ID.fetch_add(1, SeqCst)),
            scale_factor: 1.0,
            data: data.into(),
        }
    }

    /// Convert this image into a byte slice.
    pub fn as_bytes(&self, frame_index: usize) -> Option<&[u8]> {
        self.data
            .get(frame_index)
            .map(|frame| frame.buffer().as_raw().as_slice())
    }

    /// Get the size of this image, in pixels.
    pub fn size(&self, frame_index: usize) -> Size<DevicePixels> {
        self.data
            .get(frame_index)
            .map(|frame| {
                let (width, height) = frame.buffer().dimensions();
                size(width.into(), height.into())
            })
            .unwrap_or_default()
    }

    /// Get the size of this image, in pixels for display, adjusted for the scale factor.
    pub(crate) fn render_size(&self, frame_index: usize) -> Size<Pixels> {
        self.size(frame_index)
            .map(|v| (v.0 as f32 / self.scale_factor).into())
    }

    /// Get the delay of this frame from the previous
    pub fn delay(&self, frame_index: usize) -> Delay {
        self.data
            .get(frame_index)
            .map(|frame| frame.delay())
            .unwrap_or(Delay::from_numer_denom_ms(100, 1))
    }

    /// Get the number of frames for this image.
    pub fn frame_count(&self) -> usize {
        self.data.len()
    }
}

impl fmt::Debug for RenderImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageData")
            .field("id", &self.id)
            .field("size", &self.data.first().map(|f| f.buffer().dimensions()))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smallvec::SmallVec;

    #[test]
    fn empty_render_image_does_not_panic() {
        let image = RenderImage::new(SmallVec::new());
        assert_eq!(image.frame_count(), 0);
        assert_eq!(image.size(0), Size::default());
        assert_eq!(image.as_bytes(0), None);
        assert_eq!(image.render_size(0), Size::default());
        assert_eq!(image.delay(0), Delay::from_numer_denom_ms(100, 1));
        let _ = format!("{image:?}");
    }

    fn example_assets() -> Vec<(&'static str, AssetData)> {
        vec![
            ("asset0", Cow::Owned(vec![0u8])),
            ("asset1", Cow::Owned(vec![1u8])),
            ("asset2", Cow::Owned(vec![2u8])),
            ("asset3", Cow::Owned(vec![3u8])),
        ]
    }

    fn test_load_from(registry: &AssetRegistry) {
        for (key, value) in example_assets() {
            let loaded = registry.load(key).map(|cow| cow.to_vec());
            assert_eq!(loaded, Some(value.to_vec()));
        }
    }

    struct StubAssetSource;
    impl AssetSource for StubAssetSource {
        fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
            for (key, data) in example_assets() {
                if key == path {
                    return Ok(Some(data));
                }
            }
            unimplemented!()
        }

        fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
            Ok(example_assets()
                .into_iter()
                .map(|(key, _)| key.into())
                .collect())
        }
    }

    #[test]
    fn test_from_assetsource() {
        let assets = AssetRegistry::from(StubAssetSource);
        test_load_from(&assets);
    }

    #[test]
    fn test_from_arc_assetsource() {
        let arc: Arc<dyn AssetSource> = Arc::new(StubAssetSource);
        let assets = AssetRegistry::from(arc);
        test_load_from(&assets);
    }

    #[test]
    fn test_fromiter_assetdata() {
        let assets = AssetRegistry::from_iter(example_assets());
        test_load_from(&assets);
    }

    #[test]
    fn test_fromiter_assetentry() {
        let assets = AssetRegistry::from_iter(
            example_assets()
                .into_iter()
                .map(|(key, value)| (key, AssetEntry::from(value)))
                .chain(std::iter::once((
                    "asset6",
                    AssetEntry::OnDemand(Arc::new(|| Some(AssetData::Owned(vec![6u8])))),
                ))),
        );
        test_load_from(&assets);
        assert_eq!(
            assets.load("asset6").map(|cow| cow.to_vec()),
            Some(vec![6u8])
        );
    }

    #[test]
    fn test_insert() {
        let mut assets = AssetRegistry::default();
        for (key, value) in example_assets() {
            let result = assets.insert(key, value);
            assert!(result.is_ok());
        }
        test_load_from(&assets);
    }

    #[test]
    fn test_extend() {
        let mut assets = AssetRegistry::default();

        let samples = example_assets();
        assets.extend(samples.iter().map(|(key, _)| key), |key| {
            samples
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| AssetEntry::PreLoaded(v.clone()))
        });

        test_load_from(&assets);
    }

    #[test]
    fn test_extend_assetsource() {
        let mut assets = AssetRegistry::default();
        assets.extend_from_source(Arc::new(StubAssetSource) as Arc<dyn AssetSource>);
        test_load_from(&assets);
    }
}
