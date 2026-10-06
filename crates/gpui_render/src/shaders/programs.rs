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

/// The renderer side of a shader paint: it builds the [`Fragment`], reads the
/// paint's parameter block, samples the backdrop, and calls the linked
/// program's `paint_program`.
///
/// A parameter block starts with `[scale_factor, opacity, 0, 0]`, followed by
/// the program's parameter slots.
#[wgsl_rs::wgsl(skip_validation)]
pub mod paint_glue {
    use super::super::common::*;
    use gpui::shader::prelude::*;
    use wgsl_rs::std::*;

    storage!(group(2), binding(0), PAINT_PARAMS: RuntimeArray<Vec4f>);
    texture!(group(2), binding(1), BACKDROP: Texture2D<f32>);
    sampler!(group(2), binding(2), BACKDROP_SAMPLER: Sampler);

    pub fn paint_param(index: u32) -> Vec4f {
        get!(PAINT_PARAMS)[index as usize]
    }

    pub fn paint_backdrop(fragment: Fragment, uv: Vec2f) -> Vec4f {
        let pixel = fragment.origin + uv * fragment.size * fragment.scale;
        let sample_uv = clamp(
            pixel / get!(GLOBALS).viewport_size,
            vec2f(0.0, 0.0),
            vec2f(1.0, 1.0),
        );
        texture_sample_level(BACKDROP, BACKDROP_SAMPLER, sample_uv, 0.0)
    }

    pub fn shader_paint(origin: Vec2f, size: Vec2f, params: u32, position: Vec2f) -> Vec4f {
        let header = paint_param(params);
        let scale = header.x;
        let fragment = Fragment {
            uv: (position - origin) / size,
            position: (position - origin) / scale,
            size: size / scale,
            origin,
            scale,
        };
        let color = Color::fade(paint_program(fragment, params + 1u32), header.y);
        Color::unpremultiply(color)
    }

    /// Provided by the linked program.
    #[wgsl_ignore]
    pub fn paint_program(_fragment: Fragment, _base: u32) -> Vec4f {
        vec4f(0.0, 0.0, 0.0, 0.0)
    }
}

/// The renderer side of a custom pipeline: instance slots, the draw header,
/// the clip-space transform, clipping, and blending.
///
/// A draw header is three slots: `[scale_factor, opacity, origin]` (the
/// device-pixel origin geometry is relative to), the content mask's
/// device-pixel bounds, and its fade distances. Every instance ends with a
/// slot holding its header's index, bit-cast to `f32`.
#[wgsl_rs::wgsl]
pub mod pipeline_glue {
    use super::super::common::*;
    use wgsl_rs::std::*;

    storage!(group(1), binding(0), PIPELINE_DATA: RuntimeArray<Vec4f>);

    pub fn pipeline_slot(index: u32) -> Vec4f {
        get!(PIPELINE_DATA)[index as usize]
    }

    pub fn pipeline_header(index: u32) -> u32 {
        bitcast_u32(pipeline_slot(index).x)
    }

    pub fn pipeline_clip(position: Vec2f, header: u32) -> Vec4f {
        let draw = pipeline_slot(header);
        viewport_to_clip_position(draw.zw() + position * draw.x)
    }

    pub fn pipeline_position(clip: Vec4f, header: u32) -> Vec2f {
        let draw = pipeline_slot(header);
        (clip.xy() - draw.zw()) / draw.x
    }

    pub fn pipeline_output(color: Vec4f, clip: Vec4f, header: u32) -> Vec4f {
        let bounds = pipeline_slot(header + 1u32);
        let fade = pipeline_slot(header + 2u32);
        let mask = ContentMask {
            bounds: Bounds {
                origin: bounds.xy(),
                size: bounds.zw(),
            },
            fade_out: Edges {
                top: fade.x,
                right: fade.y,
                bottom: fade.z,
                left: fade.w,
            },
        };
        let position = clip.xy();
        let far = mask.bounds.origin + mask.bounds.size;
        let inside = position.x >= mask.bounds.origin.x
            && position.y >= mask.bounds.origin.y
            && position.x <= far.x
            && position.y <= far.y;
        let coverage = select(0.0, ContentMask::alpha(mask, position), inside);
        blend_color(color, coverage * pipeline_slot(header).y)
    }
}
