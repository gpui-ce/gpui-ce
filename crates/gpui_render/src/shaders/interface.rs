/// Shader-declared data that can be uploaded without serialization.
///
/// # Safety
/// Implementors need a stable C-compatible layout, no padding, matching the WGSL declaration.
pub unsafe trait BufferData: Sized {
    const WGSL_TYPE: &'static str;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorageAbi {
    pub wgsl_type: &'static str,
    pub rust_stride: usize,
}

pub const fn storage_abi<T: BufferData>() -> StorageAbi {
    StorageAbi {
        wgsl_type: T::WGSL_TYPE,
        rust_stride: std::mem::size_of::<T>(),
    }
}

pub fn bytes_of<T: BufferData>(value: &T) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(
            std::ptr::from_ref(value).cast::<u8>(),
            std::mem::size_of::<T>(),
        )
    }
}

pub fn slice_as_bytes<T: BufferData>(values: &[T]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataLayout {
    Instances,
    TexturedInstances,
    Surface,
    Blur,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VertexCount {
    Rectangle,
    FullscreenTriangle,
    Dynamic,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrimitiveTopology {
    TriangleList,
    TriangleStrip,
}

impl VertexCount {
    pub const fn fixed(self) -> Option<u32> {
        match self {
            Self::Rectangle => Some(RECTANGLE_VERTEX_COUNT),
            Self::FullscreenTriangle => Some(FULLSCREEN_TRIANGLE_VERTEX_COUNT),
            Self::Dynamic => None,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Pipeline {
    pub label: &'static str,
    pub vertex_entry: &'static str,
    pub fragment_entry: &'static str,
    pub topology: PrimitiveTopology,
    pub data_layout: DataLayout,
    pub vertex_count: VertexCount,
}

macro_rules! define_pipelines {
        ($($name:ident: $label:literal, $vertex:ident, $fragment:ident, $topology:ident, $layout:ident, $vertices:ident;)*) => {
            $(
                pub const $name: Pipeline = Pipeline {
                    label: $label,
                    vertex_entry: stringify!($vertex),
                    fragment_entry: stringify!($fragment),
                    topology: PrimitiveTopology::$topology,
                    data_layout: DataLayout::$layout,
                    vertex_count: VertexCount::$vertices,
                };
            )*

            pub const ALL: &[Pipeline] = &[$($name),*];
        };
    }

define_pipelines! {
    QUADS: "quads", vertex_quad, fragment_quad, TriangleStrip, Instances, Rectangle;
    SHADOWS: "shadows", vertex_shadow, fragment_shadow, TriangleStrip, Instances, Rectangle;
    PATH_RASTERIZATION: "path_rasterization", vertex_path_rasterization, fragment_path_rasterization, TriangleList, Instances, Dynamic;
    PATHS: "paths", vertex_path, fragment_path, TriangleStrip, TexturedInstances, Rectangle;
    UNDERLINES: "underlines", vertex_underline, fragment_underline, TriangleStrip, Instances, Rectangle;
    MONOCHROME_SPRITES: "monochrome_sprites", vertex_monochrome_sprite, fragment_monochrome_sprite, TriangleStrip, TexturedInstances, Rectangle;
    SUBPIXEL_SPRITES: "subpixel_sprites", vertex_subpixel_sprite, fragment_subpixel_sprite, TriangleStrip, TexturedInstances, Rectangle;
    POLYCHROME_SPRITES: "polychrome_sprites", vertex_polychrome_sprite, fragment_polychrome_sprite, TriangleStrip, TexturedInstances, Rectangle;
    SURFACES: "surfaces", vertex_surface, fragment_surface, TriangleStrip, Surface, Rectangle;
    BLUR_DOWNSAMPLE: "blur_downsample", vertex_blur_fullscreen, fragment_blur_downsample, TriangleList, Blur, FullscreenTriangle;
    BLUR: "blur", vertex_blur_fullscreen, fragment_blur, TriangleList, Blur, FullscreenTriangle;
    BLUR_COMPOSITE: "blur_composite", vertex_blur_composite, fragment_blur_composite, TriangleStrip, Blur, Rectangle;
}

pub const GLOBAL_BIND_GROUP: u32 = 0;
pub const DATA_BIND_GROUP: u32 = 1;
pub const GLOBAL_UNIFORMS_BINDING: u32 = 0;
pub const FONT_RASTERIZATION_BINDING: u32 = 1;
pub const DATA_BUFFER_BINDING: u32 = 0;
pub const PRIMARY_TEXTURE_BINDING: u32 = 1;
pub const SECONDARY_TEXTURE_BINDING: u32 = 2;
pub const PRIMARY_SAMPLER_BINDING: u32 = 2;
pub const SURFACE_SAMPLER_BINDING: u32 = 3;
pub const RECTANGLE_VERTEX_COUNT: u32 = 4;
pub const FULLSCREEN_TRIANGLE_VERTEX_COUNT: u32 = 3;

macro_rules! buffer_data {
        ($($rust:ty => $wgsl:literal),* $(,)?) => {
            $(
                unsafe impl BufferData for $rust {
                    const WGSL_TYPE: &'static str = $wgsl;
                }
            )*
        };
    }

buffer_data! {
    super::common::GlobalUniforms => "GlobalUniforms",
    super::common::FontRasterizationUniforms => "FontRasterizationUniforms",
    super::surface::SurfaceUniforms => "SurfaceUniforms",
    super::blur::BlurUniforms => "BlurUniforms",
    gpui::Quad => "Quad",
    gpui::Shadow => "Shadow",
    gpui::Underline => "Underline",
    gpui::MonochromeSprite => "MonochromeSprite",
    gpui::SubpixelSprite => "SubpixelSprite",
    gpui::PolychromeSprite => "PolychromeSprite",
}

pub const SCENE_STORAGE_ABI: &[StorageAbi] = &[
    storage_abi::<gpui::Quad>(),
    storage_abi::<gpui::Shadow>(),
    storage_abi::<gpui::Underline>(),
    storage_abi::<gpui::MonochromeSprite>(),
    storage_abi::<gpui::SubpixelSprite>(),
    storage_abi::<gpui::PolychromeSprite>(),
];
