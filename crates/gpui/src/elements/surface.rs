#[cfg(target_os = "windows")]
use crate::WindowsScreenCaptureFrame;
use crate::{
    App, Bounds, DevicePixels, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, ObjectFit, Pixels, Size, Style, StyleRefinement, Styled, Window,
};
#[cfg(target_os = "macos")]
use core_video::pixel_buffer::CVPixelBuffer;
use refineable::Refineable;

/// A source of a surface's content.
#[derive(Clone)]
pub enum SurfaceSource {
    /// A macOS image buffer from CoreVideo
    #[cfg(target_os = "macos")]
    Surface(CVPixelBuffer),
    /// A GPU texture handle (type-erased to avoid depending on wgpu)
    #[cfg(any(
        target_os = "linux",
        target_os = "freebsd",
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

/// A surface element.
pub struct Surface {
    source: SurfaceSource,
    object_fit: ObjectFit,
    style: StyleRefinement,
}

/// Create a new surface element.
pub fn surface(source: impl Into<SurfaceSource>) -> Surface {
    Surface {
        source: source.into(),
        object_fit: ObjectFit::Contain,
        style: Default::default(),
    }
}

impl Surface {
    /// Set the object fit for the image.
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
        let mut style = Style::default();
        style.refine(&self.style);
        _window.with_element_opacity(style.opacity, |window| {
            let corner_radii = style.corner_radii.to_pixels(window.rem_size());
            window.paint_surface(new_bounds, corner_radii, self.source.clone());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TestAppContext, point, px, size};

    /// A surface inside a rounded frame must carry the frame's curve into the scene. Before
    /// this, `paint_surface` took no radii at all and the texture drew square corners under
    /// the frame, which is visible as four bright wedges wherever a rounded pane holds one.
    #[crate::test]
    fn a_rounded_surface_carries_its_radii_into_the_scene(cx: &mut TestAppContext) {
        let window = cx.add_empty_window();
        window.draw(point(px(0.), px(0.)), size(px(100.), px(100.)), |_, _| {
            surface(SurfaceSource::Unsupported(size(
                DevicePixels(100),
                DevicePixels(100),
            )))
            .size_full()
            .rounded(px(12.))
            .into_any_element()
        });

        let (radii, expected) = window.update(|window, _| {
            (
                window
                    .rendered_frame
                    .scene
                    .surfaces
                    .last()
                    .map(|painted| painted.corner_radii.top_left),
                px(12.).scale(window.scale_factor()),
            )
        });
        assert_eq!(radii, Some(expected));
    }

    /// The radii are clamped to the surface's own bounds the way an image's are, so a radius
    /// larger than the element cannot fold the distance field back on itself.
    #[crate::test]
    fn radii_larger_than_the_surface_are_clamped_to_it(cx: &mut TestAppContext) {
        let window = cx.add_empty_window();
        window.draw(point(px(0.), px(0.)), size(px(100.), px(100.)), |_, _| {
            surface(SurfaceSource::Unsupported(size(
                DevicePixels(100),
                DevicePixels(100),
            )))
            .size_full()
            .rounded(px(400.))
            .into_any_element()
        });

        let (radii, expected) = window.update(|window, _| {
            (
                window
                    .rendered_frame
                    .scene
                    .surfaces
                    .last()
                    .map(|painted| painted.corner_radii.top_left),
                px(50.).scale(window.scale_factor()),
            )
        });
        assert_eq!(radii, Some(expected));
    }

    /// A plain surface still asks for square corners, so nothing that does not opt in changes.
    #[crate::test]
    fn a_plain_surface_still_has_square_corners(cx: &mut TestAppContext) {
        let window = cx.add_empty_window();
        window.draw(point(px(0.), px(0.)), size(px(100.), px(100.)), |_, _| {
            surface(SurfaceSource::Unsupported(size(
                DevicePixels(100),
                DevicePixels(100),
            )))
            .size_full()
            .into_any_element()
        });

        let radii = window.update(|window, _| {
            window
                .rendered_frame
                .scene
                .surfaces
                .last()
                .map(|painted| painted.corner_radii)
        });
        assert_eq!(radii, Some(crate::Corners::default()));
    }
}
