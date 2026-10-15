#[cfg(target_os = "windows")]
use crate::WindowsScreenCaptureFrame;
use crate::{
    App, Bounds, DevicePixels, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, ObjectFit, Pixels, Size, Style, StyleRefinement, Styled, Window,
};
#[cfg(target_os = "macos")]
use core_video::pixel_buffer::CVPixelBuffer;
use refineable::Refineable;

/// The frame that a [`Surface`] shows.
///
/// Each variant exists only on the platforms that can show it. On macOS, pass a `CVPixelBuffer`.
/// On Windows, pass a Windows Graphics Capture frame. On Linux and FreeBSD, or with the
/// `custom-gpu` feature, pass a GPU texture.
#[derive(Clone)]
pub enum SurfaceSource {
    /// A macOS image buffer from CoreVideo.
    #[cfg(target_os = "macos")]
    Surface(CVPixelBuffer),
    /// A GPU texture, with its size in device pixels.
    #[cfg(any(
        target_os = "linux",
        target_os = "freebsd",
        all(target_os = "macos", feature = "custom-gpu"),
        all(target_family = "wasm", feature = "custom-gpu")
    ))]
    Texture {
        /// The GPU texture, type-erased (expected to be `Arc<wgpu::Texture>`)
        #[cfg(not(target_family = "wasm"))]
        texture: std::sync::Arc<dyn std::any::Any + Send + Sync>,
        /// The GPU texture, type-erased (expected to be `Arc<wgpu::Texture>`).
        ///
        /// WGPU handles are intentionally thread-local in browser builds.
        #[cfg(target_family = "wasm")]
        texture: std::sync::Arc<dyn std::any::Any>,
        /// Dimensions of the texture in device pixels
        size: Size<DevicePixels>,
    },
    /// A native Windows Graphics Capture texture.
    #[cfg(target_os = "windows")]
    WindowsCapture(WindowsScreenCaptureFrame),
    /// A placeholder for platforms that cannot import native surfaces.
    #[doc(hidden)]
    Unsupported(Size<DevicePixels>),
}

impl std::fmt::Debug for SurfaceSource {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            #[cfg(target_os = "macos")]
            SurfaceSource::Surface(ref buf) => _f.debug_tuple("Surface").field(buf).finish(),
            #[cfg(any(
                target_os = "linux",
                target_os = "freebsd",
                all(target_os = "macos", feature = "custom-gpu"),
                all(target_family = "wasm", feature = "custom-gpu")
            ))]
            SurfaceSource::Texture { size, .. } => _f
                .debug_struct("Texture")
                .field("size", &size)
                .finish_non_exhaustive(),
            #[cfg(target_os = "windows")]
            SurfaceSource::WindowsCapture(ref frame) => frame.fmt(_f),
            SurfaceSource::Unsupported(size) => _f.debug_tuple("Unsupported").field(&size).finish(),
        }
    }
}

impl SurfaceSource {
    fn size(&self) -> Size<DevicePixels> {
        match self {
            #[cfg(target_os = "macos")]
            SurfaceSource::Surface(buffer) => {
                crate::size(buffer.get_width().into(), buffer.get_height().into())
            }
            #[cfg(any(
                target_os = "linux",
                target_os = "freebsd",
                all(target_os = "macos", feature = "custom-gpu"),
                all(target_family = "wasm", feature = "custom-gpu")
            ))]
            SurfaceSource::Texture { size, .. } => *size,
            #[cfg(target_os = "windows")]
            SurfaceSource::WindowsCapture(frame) => frame.size(),
            SurfaceSource::Unsupported(size) => *size,
        }
    }
}

#[cfg(target_os = "macos")]
impl From<CVPixelBuffer> for SurfaceSource {
    fn from(value: CVPixelBuffer) -> Self {
        SurfaceSource::Surface(value)
    }
}

#[cfg(target_os = "windows")]
impl From<WindowsScreenCaptureFrame> for SurfaceSource {
    fn from(value: WindowsScreenCaptureFrame) -> Self {
        SurfaceSource::WindowsCapture(value)
    }
}

#[cfg(all(target_os = "windows", feature = "screen-capture"))]
impl From<crate::ScreenCaptureFrame> for SurfaceSource {
    fn from(value: crate::ScreenCaptureFrame) -> Self {
        SurfaceSource::WindowsCapture(value.0)
    }
}

/// An element that shows a platform video or capture frame, such as a camera feed.
///
/// Create one with [`surface`]. The frame is scaled into the element's bounds with [`ObjectFit`],
/// and the element's opacity applies to it. Corner radii are not applied yet.
pub struct Surface {
    source: SurfaceSource,
    object_fit: ObjectFit,
    style: StyleRefinement,
}

/// Creates a [`Surface`] that shows `source`.
///
/// The frame is scaled with [`ObjectFit::Contain`] unless you change it with [`Surface::object_fit`].
/// Size the element with [`Styled`] methods, such as `.size_full()`, because the frame doesn't
/// give it a size.
pub fn surface(source: impl Into<SurfaceSource>) -> Surface {
    Surface {
        source: source.into(),
        object_fit: ObjectFit::Contain,
        style: Default::default(),
    }
}

impl Surface {
    /// Sets how the frame is scaled to the element's bounds.
    pub fn object_fit(mut self, object_fit: ObjectFit) -> Self {
        self.object_fit = object_fit;
        self
    }
}

impl Element for Surface {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.refine(&self.style);
        let layout_id = window.request_layout(style, [], cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        _window: &mut Window,
        _: &mut App,
    ) {
        let new_bounds = self.object_fit.get_bounds(_bounds, self.source.size());
        // TODO: Add support for corner_radii.
        let mut style = Style::default();
        style.refine(&self.style);
        _window.with_element_opacity(style.opacity, |window| {
            window.paint_surface(new_bounds, self.source.clone());
        });
    }
}

impl IntoElement for Surface {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for Surface {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
