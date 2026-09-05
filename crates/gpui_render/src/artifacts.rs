//! WGSL linked at Cargo build time from the Rust-authored shader modules.

use std::marker::PhantomData;

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

/// D3D11 bytecode generated from HLSL at build time.
///
/// Keeping both stages together makes it impossible to pass textual HLSL to a
/// D3D11 creation API by accident.
#[derive(Clone, Copy)]
pub struct Dx11Bytecode {
    pub vertex: &'static [u8],
    pub fragment: &'static [u8],
}

/// The DX11 artifact state for this build host.
///
/// `D3DCompile` exists only on Windows. A Windows-targeting cross-build from a
/// non-Windows host is rejected by the build script, with no runtime-compiler
/// escape hatch.
#[derive(Clone, Copy)]
pub enum Dx11Shader {
    Sm50(Dx11Bytecode),
    NativeWindowsBuildRequired,
}

/// One stage of a GLSL program generated from the downlevel shader dialect.
///
/// No in-tree renderer consumes this source directly. `gpui_wgpu` provides
/// WGSL to WGPU, which owns the final binding assignment for each GL context.
/// These artifacts describe the requested GLSL profile; driver compilation
/// and program linking still belong to a future raw-GL consumer.
pub struct GlslShaderStage {
    pub source: &'static str,
}

/// Desktop core GLSL 3.30 profile marker.
pub enum Glsl330 {}

/// OpenGL ES 3.00 profile marker.
pub enum Gles300 {}

/// A pair of GLSL stages generated from validated WGSL for one profile.
pub struct GlslShader<Profile> {
    pub vertex: GlslShaderStage,
    pub fragment: GlslShaderStage,
    _profile: PhantomData<fn() -> Profile>,
}

impl<Profile> GlslShader<Profile> {
    pub const fn new(vertex: GlslShaderStage, fragment: GlslShaderStage) -> Self {
        Self {
            vertex,
            fragment,
            _profile: PhantomData,
        }
    }
}

pub struct NativeShader {
    pub label: &'static str,
    pub vertex_entry: &'static str,
    pub fragment_entry: &'static str,
    pub dx11: Dx11Shader,
    /// GLSL 3.30 core, using the data-texture downlevel transport.
    pub glsl_330: GlslShader<Glsl330>,
    /// GLSL ES 3.00, using the same downlevel transport.
    pub gles_300: GlslShader<Gles300>,
    pub msl: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/native_shaders.rs"));

#[cfg(test)]
mod tests {
    use super::BASE_DOWNLEVEL_WGSL;

    #[test]
    fn downlevel_background_array_uses_host_element_stride() {
        let decoder = BASE_DOWNLEVEL_WGSL
            .split_once("fn dl_load_Background_impl")
            .and_then(|(_, source)| source.split_once("\n}"))
            .map(|(source, _)| source)
            .expect("generated Background decoder must exist");
        let element_stride = std::mem::size_of::<gpui::LinearColorStop>() / 4;
        let second_element_offset = 7 + element_stride;

        assert!(
            decoder.contains("base + 7u"),
            "Background decoder must load its first color stop at word 7"
        );
        assert!(
            decoder.contains(&format!("base + {second_element_offset}u")),
            "Background decoder must use the host LinearColorStop stride"
        );
        assert!(
            !decoder.contains("base + 17u)), dl_scene_word"),
            "Background decoder must not read padding as the second color stop"
        );
    }
}
