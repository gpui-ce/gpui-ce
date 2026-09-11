use crate::{DevicePixels, Pixels, Result, SharedString, Size, size};
use smallvec::SmallVec;

use image::{Delay, Frame};
use std::{
    borrow::Cow,
    fmt,
    hash::Hash,
    sync::atomic::{AtomicUsize, Ordering::SeqCst},
};

/// A source of assets for this app to use.
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

macro_rules! impl_asset_source_for_tuples {
    ($(($($source:ident: $index:tt),+)),+ $(,)?) => {
        $(
            impl<$($source: AssetSource),+> AssetSource for ($($source,)+) {
                fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
                    $(
                        if let Some(asset) = self.$index.load(path)? {
                            return Ok(Some(asset));
                        }
                    )+

                    Ok(None)
                }

                fn list(&self, path: &str) -> Result<Vec<SharedString>> {
                    let mut assets = Vec::new();

                    $(
                        assets.extend(self.$index.list(path)?);
                    )+

                    Ok(assets)
                }
            }
        )+
    };
}

impl_asset_source_for_tuples!(
    (T0: 0, T1: 1),
    (T0: 0, T1: 1, T2: 2),
    (T0: 0, T1: 1, T2: 2, T3: 3),
    (T0: 0, T1: 1, T2: 2, T3: 3, T4: 4),
    (T0: 0, T1: 1, T2: 2, T3: 3, T4: 4, T5: 5),
    (T0: 0, T1: 1, T2: 2, T3: 3, T4: 4, T5: 5, T6: 6),
    (T0: 0, T1: 1, T2: 2, T3: 3, T4: 4, T5: 5, T6: 6, T7: 7),
);

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
    use std::sync::{Arc, Mutex};

    struct TestAssetSource<const INDEX: usize> {
        asset: Option<&'static [u8]>,
        files: &'static [&'static str],
        loads: Arc<Mutex<Vec<usize>>>,
    }

    impl<const INDEX: usize> AssetSource for TestAssetSource<INDEX> {
        fn load(&self, _path: &str) -> Result<Option<Cow<'static, [u8]>>> {
            self.loads.lock().unwrap().push(INDEX);
            Ok(self.asset.map(Cow::Borrowed))
        }

        fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
            Ok(self.files.iter().copied().map(SharedString::from).collect())
        }
    }

    #[test]
    fn tuple_asset_sources_list_assets_in_order() {
        let loads = Arc::new(Mutex::new(Vec::new()));

        let sources = (
            TestAssetSource::<0> {
                asset: None,
                files: &["a", "shared"],
                loads: loads.clone(),
            },
            TestAssetSource::<1> {
                asset: None,
                files: &["b", "shared"],
                loads,
            },
        );

        assert_eq!(
            sources.list("").unwrap(),
            vec![
                SharedString::from("a"),
                SharedString::from("shared"),
                SharedString::from("b"),
                SharedString::from("shared"),
            ]
        );
    }

    #[test]
    fn tuple_asset_sources_load_first_match() {
        let loads = Arc::new(Mutex::new(Vec::new()));

        let sources = (
            TestAssetSource::<0> {
                asset: None,
                files: &[],
                loads: loads.clone(),
            },
            TestAssetSource::<1> {
                asset: Some(b"asset"),
                files: &[],
                loads: loads.clone(),
            },
            TestAssetSource::<2> {
                asset: Some(b"shadowed"),
                files: &[],
                loads: loads.clone(),
            },
        );

        assert_eq!(
            sources.load("asset").unwrap().as_deref(),
            Some(b"asset".as_slice())
        );
        assert_eq!(*loads.lock().unwrap(), vec![0, 1]);
    }

    #[test]
    fn tuple_asset_sources_return_none_when_all_sources_miss() {
        let loads = Arc::new(Mutex::new(Vec::new()));

        let sources = (
            TestAssetSource::<0> {
                asset: None,
                files: &[],
                loads: loads.clone(),
            },
            TestAssetSource::<1> {
                asset: None,
                files: &[],
                loads: loads.clone(),
            },
        );

        assert_eq!(sources.load("missing").unwrap(), None);
        assert_eq!(*loads.lock().unwrap(), vec![0, 1]);
    }

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
}
