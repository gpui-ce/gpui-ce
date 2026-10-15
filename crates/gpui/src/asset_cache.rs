use crate::{App, SharedString, SharedUri};
use futures::{Future, TryFutureExt};

use std::fmt::Debug;
use std::hash::{BuildHasher, Hash};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Where an image or other asset is loaded from: a URL, a file path, or a name in the registry.
///
/// Equal resources share one cache entry.
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub enum Resource {
    /// A URL, fetched with the app's HTTP client.
    Uri(SharedUri),
    /// A file on disk, read with [`std::fs::read`]. A relative path is relative to the working
    /// directory.
    Path(Arc<Path>),
    /// A name in the app's [`crate::AssetRegistry`]. Despite the name, this is any registry entry, not
    /// only one from the `embedded-assets` feature.
    Embedded(SharedString),
}

impl From<SharedUri> for Resource {
    fn from(value: SharedUri) -> Self {
        Self::Uri(value)
    }
}

impl From<PathBuf> for Resource {
    fn from(value: PathBuf) -> Self {
        Self::Path(value.into())
    }
}

impl From<Arc<Path>> for Resource {
    fn from(value: Arc<Path>) -> Self {
        Self::Path(value)
    }
}

/// A value that GPUI loads in the background and shares between callers, such as an image.
///
/// Implement it on a type that holds no data, such as an empty enum. Then call
/// [`crate::Window::use_asset`] or [`crate::Window::get_asset`] to load it. The first call for a
/// source starts the load and returns `None`. Later calls share that load and return its output
/// once it is ready. [`App::remove_asset`] discards the app-level entry so the next fetch starts a
/// new load.
pub trait Asset: 'static {
    /// The key that identifies the asset. Equal keys share one cache entry.
    type Source: Clone + Hash + Send;

    /// The result of loading. Callers receive clones of the cached output, so keep it cheap to
    /// clone.
    type Output: Clone + Send;

    /// Starts loading `source`, and returns the future that produces the output.
    ///
    /// The cache calls this on the foreground thread, where `cx` is available. Read any app state
    /// needed to construct the request here; the returned future runs on the background executor.
    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static;
}

/// Wraps an [`Asset`] whose output is a [`Result`], and logs each error as it loads.
///
/// The wrapped loader's result is returned unchanged. [`img`](crate::img) loads images through
/// `AssetLogger`, so a failed load appears in the log.
pub enum AssetLogger<T> {
    #[doc(hidden)]
    _Phantom(PhantomData<T>, &'static dyn crate::seal::Sealed),
}

impl<T, R, E> Asset for AssetLogger<T>
where
    T: Asset<Output = Result<R, E>>,
    R: Clone + Send,
    E: Clone + Send + Debug,
{
    type Source = T::Source;

    type Output = T::Output;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        let load = T::load(source, cx);
        load.inspect_err(|e| log::error!("Failed to load asset: {:?}", e))
    }
}

/// Hashes `data` with a fast, non-cryptographic hash, for use as a cache key within one process.
///
/// The value can change between builds, so don't store it or compare it across runs.
pub fn hash<T: Hash>(data: &T) -> u64 {
    collections::FxBuildHasher.hash_one(data)
}
