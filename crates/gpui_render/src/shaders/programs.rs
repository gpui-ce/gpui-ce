/// How a primitive's [`BackgroundTag::Shader`](super::common::BackgroundTag) side is
/// colored. Ordinary pipelines never draw shader backgrounds, so this stub paints
/// nothing; shader pipelines link [`paint_glue`] in its place.
#[wgsl_rs::wgsl]
pub mod paint_hook {
    use wgsl_rs::std::*;

    pub fn shader_paint(_origin: Vec2f, _size: Vec2f, _params: u32, _position: Vec2f) -> Vec4f {
        vec4f(0.0, 0.0, 0.0, 0.0)
    }
}
