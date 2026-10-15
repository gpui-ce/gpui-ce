use std::{
    fs,
    hash::{Hash, Hasher},
    path::Path,
    sync::Arc,
};

use crate::{
    App, Asset, Bounds, Element, GlobalElementId, Hitbox, InspectorElementId, InteractiveElement,
    Interactivity, IntoElement, LayoutId, Pixels, Point, Radians, SharedString, Size,
    StyleRefinement, Styled, TransformationMatrix, Window, point, px, radians, size,
};
use gpui_util::ResultExt;

/// An element that draws an SVG as a single-color mask, filled with the text color.
///
/// Create one with [`svg`]. The SVG's alpha channel becomes the shape, and its own colors are
/// dropped. That suits icons that follow the theme. To keep an SVG's colors, use [`crate::img`] instead.
///
/// Nothing is drawn until the element has a source: [`Svg::path`], [`Svg::external_path`] or
/// [`Svg::data`].
#[derive(derive_setters::Setters)]
#[setters(generate = false)]
pub struct Svg {
    interactivity: Interactivity,
    /// Rotates, scales or moves the rendered SVG around the center of the element.
    ///
    /// The transformation changes only what is drawn. The layout and the hitbox stay the same, so
    /// the element still takes its full box and receives clicks there. See [`Transformation`].
    #[setters(generate, rename = "with_transformation", strip_option)]
    transformation: Option<Transformation>,
    /// Sets the SVG to load from the app's [`crate::AssetRegistry`], by `path`, such as `"icons/search.svg"`.
    ///
    /// The registry is read when the icon is first drawn at a given size, and each rasterized size
    /// is then cached. If the registry has no such path, nothing is drawn.
    ///
    /// When several sources are set, [`Svg::data`] wins, then [`Svg::external_path`], then this
    /// one.
    #[setters(generate, strip_option, into)]
    path: Option<SharedString>,
    /// Sets the SVG to load from a file on disk, relative to the working directory.
    ///
    /// The file is read once, in the background, and cached under `path`. Nothing is drawn until it
    /// loads. A read error is logged, and nothing is drawn. The cache keeps the entry, so later
    /// changes to the file are not picked up until the entry is removed.
    ///
    /// When several sources are set, [`Svg::data`] wins over this one.
    #[setters(generate, strip_option, into)]
    external_path: Option<SharedString>,
    data: Option<Arc<[u8]>>,
    data_path: Option<SharedString>,
}

/// Creates an [`Svg`] element with no source yet.
///
/// Give it a source with [`Svg::path`], [`Svg::external_path`] or [`Svg::data`]. The icon is filled
/// with the text color, so set that with [`Styled::text_color`].
///
/// # Examples
///
/// ```
/// # use gpui::*;
/// const STAR: &[u8] = b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 8 8\"><rect width=\"8\" height=\"8\"/></svg>";
///
/// let star = svg().data(STAR).size(px(16.)).text_color(rgb(0xf59e0b));
/// ```
#[track_caller]
pub fn svg() -> Svg {
    Svg {
        interactivity: Interactivity::new(),
        transformation: None,
        path: None,
        external_path: None,
        data: None,
        data_path: None,
    }
}

impl Svg {
    /// Sets the SVG from bytes that are already in memory, such as one included with `include_bytes!`.
    ///
    /// The bytes are copied into the element. The rasterized icon is cached under a hash of the
    /// bytes, so the same data is rasterized once. Calling this again replaces the bytes. When
    /// several sources are set, this one wins.
    pub fn data(mut self, data: &[u8]) -> Self {
        // Generate a unique deterministic path based on the data hash for caching
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        data.hash(&mut hasher);
        let hash = hasher.finish();
        let path = SharedString::from(format!("__binary_svg__{}", hash));
        self.data = Some(Arc::from(data));
        self.data_path = Some(path);
        self
    }
}

impl Element for Svg {
    type RequestLayoutState = ();
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<crate::ElementId> {
        self.interactivity.element_id.clone()
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.interactivity.source_location()
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let layout_id = self.interactivity.request_layout(
            global_id,
            inspector_id,
            window,
            cx,
            |style, window, cx| window.request_layout(style, None, cx),
        );
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Hitbox> {
        self.interactivity.prepaint(
            global_id,
            inspector_id,
            bounds,
            bounds.size,
            window,
            cx,
            |_, _, hitbox, _, _| hitbox,
        )
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        hitbox: &mut Option<Hitbox>,
        window: &mut Window,
        cx: &mut App,
    ) where
        Self: Sized,
    {
        self.interactivity.paint(
            global_id,
            inspector_id,
            bounds,
            hitbox.as_ref(),
            window,
            cx,
            |style, window, cx| {
                let transformation = self
                    .transformation
                    .as_ref()
                    .map(|transformation| {
                        transformation.into_matrix(bounds.center(), window.scale_factor())
                    })
                    .unwrap_or_default();

                if let Some((data, path)) = self.data.as_ref().zip(self.data_path.as_ref()) {
                    if let Some(color) = style.text.color {
                        window
                            .paint_svg(
                                bounds,
                                path.clone(),
                                Some(&**data),
                                transformation,
                                color,
                                cx,
                            )
                            .log_err();
                    }
                } else if let Some((path, color)) =
                    self.external_path.as_ref().zip(style.text.color)
                {
                    let Some(bytes) = window
                        .use_asset::<SvgAsset>(path, cx)
                        .and_then(|asset| asset.log_err())
                    else {
                        return;
                    };

                    window
                        .paint_svg(
                            bounds,
                            path.clone(),
                            Some(&bytes),
                            transformation,
                            color,
                            cx,
                        )
                        .log_err();
                } else if let Some((path, color)) = self.path.as_ref().zip(style.text.color) {
                    window
                        .paint_svg(bounds, path.clone(), None, transformation, color, cx)
                        .log_err();
                }
            },
        )
    }
}

impl IntoElement for Svg {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for Svg {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.interactivity.base_style
    }
}

impl InteractiveElement for Svg {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}

/// A rotation, scale and translation applied to an [`Svg`] when it is drawn.
///
/// Build one with [`Transformation::scale`], [`Transformation::rotate`] or
/// [`Transformation::translate`], then chain the `with_*` methods to set the other parts. The
/// transformation turns around the center of the element's bounds. Translation is in logical
/// pixels, and rotation is in radians.
///
/// # Examples
///
/// ```
/// # use gpui::*;
/// let rotated = Transformation::rotate(radians(std::f32::consts::FRAC_PI_4));
/// let shrunk = Transformation::scale(size(0.5, 0.5));
/// assert_eq!(shrunk, Transformation::default().with_scaling(size(0.5, 0.5)));
///
/// let icon = svg()
///     .path("icons/chevron.svg")
///     .size(px(16.))
///     .with_transformation(rotated);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, derive_setters::Setters)]
#[setters(generate = false)]
pub struct Transformation {
    /// Returns this transformation with its scale set to `scale`.
    #[setters(generate, rename = "with_scaling")]
    scale: Size<f32>,
    /// Returns this transformation with its translation set to `translate`, in logical pixels.
    #[setters(generate, rename = "with_translation")]
    translate: Point<Pixels>,
    /// Returns this transformation with its rotation set to `rotate`.
    #[setters(generate, rename = "with_rotation", into)]
    rotate: Radians,
}

impl Default for Transformation {
    fn default() -> Self {
        Self {
            scale: size(1.0, 1.0),
            translate: point(px(0.0), px(0.0)),
            rotate: radians(0.0),
        }
    }
}

impl Transformation {
    /// Creates a transformation that scales the SVG by `scale` on each axis. `1.0` keeps the size.
    pub fn scale(scale: Size<f32>) -> Self {
        Self {
            scale,
            translate: point(px(0.0), px(0.0)),
            rotate: radians(0.0),
        }
    }

    /// Creates a transformation that moves the SVG by `translate`, in logical pixels.
    pub fn translate(translate: Point<Pixels>) -> Self {
        Self {
            scale: size(1.0, 1.0),
            translate,
            rotate: radians(0.0),
        }
    }

    /// Creates a transformation that rotates the SVG around its center by `rotate`.
    pub fn rotate(rotate: impl Into<Radians>) -> Self {
        let rotate = rotate.into();
        Self {
            scale: size(1.0, 1.0),
            translate: point(px(0.0), px(0.0)),
            rotate,
        }
    }

    fn into_matrix(self, center: Point<Pixels>, scale_factor: f32) -> TransformationMatrix {
        //Note: if you read this as a sequence of matrix multiplications, start from the bottom
        TransformationMatrix::unit()
            .translate(center.scale(scale_factor) + self.translate.scale(scale_factor))
            .rotate(self.rotate)
            .scale(self.scale)
            .translate(center.scale(-scale_factor))
    }
}

enum SvgAsset {}

impl Asset for SvgAsset {
    type Source = SharedString;
    type Output = Result<Arc<[u8]>, Arc<std::io::Error>>;

    fn load(
        source: Self::Source,
        _cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        async move {
            let bytes = fs::read(Path::new(source.as_ref())).map_err(|e| Arc::new(e))?;
            let bytes = Arc::from(bytes);
            Ok(bytes)
        }
    }
}
