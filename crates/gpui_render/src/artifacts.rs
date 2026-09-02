//! WGSL linked at Cargo build time from the Rust-authored shader modules.

pub const BASE_WGSL: &str = include_str!(concat!(env!("OUT_DIR"), "/gpui_base.wgsl"));
pub const SUBPIXEL_DUAL_SOURCE_WGSL: &str =
    include_str!(concat!(env!("OUT_DIR"), "/gpui_subpixel_dual_source.wgsl"));
/// Downlevel (WebGL2/GLES) dialect: scene arrays travel via an `rgba32uint` data texture.
pub const BASE_DOWNLEVEL_WGSL: &str =
    include_str!(concat!(env!("OUT_DIR"), "/gpui_base_downlevel.wgsl"));

#[derive(Clone, Copy)]
pub struct GeneratedBinding {
    pub binding: u32,
    pub visibility: u32,
    pub kind: GeneratedBindingKind,
}

#[derive(Clone, Copy)]
pub enum GeneratedBindingKind {
    Uniform(u64),
    StorageRead(u64),
    Texture2dFloat,
    FilteringSampler,
    /// Downlevel only: the `rgba32uint` scene-data texture.
    DataTexture,
    /// Downlevel only: the per-draw batch base uniform, bound with a dynamic offset.
    RangeUniform,
}

include!(concat!(env!("OUT_DIR"), "/shader_interface.rs"));

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Dx11ShaderModel {
    Sm50,
}

pub struct Dx11Shader {
    pub source: &'static str,
    pub model: Dx11ShaderModel,
}

pub struct NativeShader {
    pub label: &'static str,
    pub vertex_entry: &'static str,
    pub fragment_entry: &'static str,
    pub dx11: Dx11Shader,
    pub msl: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/native_shaders.rs"));
