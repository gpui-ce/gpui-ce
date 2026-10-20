#[wgsl_rs::wgsl]
pub mod surface {
    use super::super::common::*;
    use wgsl_rs::std::*;

    #[repr(C)]
    #[derive(Clone, Copy, Wgsl)]
    pub struct SurfaceUniforms {
        pub bounds: Bounds,
        pub content_mask: ContentMask,
        pub color_format: SurfaceColorFormat,
        pub opacity: f32,
        pub padding0: u32,
        pub padding1: u32,
        pub padding2: u32,
        pub padding3: u32,
        pub padding4: u32,
        pub padding5: u32,
    }
    uniform!(group(1), binding(0), SURFACE_LOCALS: SurfaceUniforms);
    texture!(group(1), binding(1), SURFACE_TEXTURE: Texture2D<f32>);
    texture!(group(1), binding(2), SURFACE_CHROMA_TEXTURE: Texture2D<f32>);
    sampler!(group(1), binding(3), SURFACE_SAMPLER: Sampler);

    pub const YCBCR_TO_LINEAR_RGB: Mat4x4f = mat4x4f(
        vec4f(1.0000, 1.0000, 1.0000, 0.0),
        vec4f(0.0000, -0.3441, 1.7720, 0.0),
        vec4f(1.4020, -0.7141, 0.0000, 0.0),
        vec4f(-0.7010, 0.5291, -0.8860, 1.0),
    );

    pub fn sample_yuv_surface(texture_position: Vec2f) -> Vec4f {
        let luma = texture_sample_level(SURFACE_TEXTURE, SURFACE_SAMPLER, texture_position, 0.0).x;
        let chroma = texture_sample_level(
            SURFACE_CHROMA_TEXTURE,
            SURFACE_SAMPLER,
            texture_position,
            0.0,
        )
        .xy();
        YCBCR_TO_LINEAR_RGB * vec4f(luma, chroma.x, chroma.y, 1.0)
    }

    #[derive(Wgsl)]
    pub struct SurfaceVarying {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub texture_position: Vec2f,
        #[location(3)]
        pub clip_distances: Vec4f,
    }

    #[vertex]
    pub fn vertex_surface(#[builtin(vertex_index)] vertex_id: u32) -> SurfaceVarying {
        let vertex = rectangle_vertex(vertex_id, get!(SURFACE_LOCALS).bounds);
        SurfaceVarying {
            position: vertex.clip_position,
            texture_position: vertex.unit_position,
            clip_distances: clip_distances(
                vertex.viewport_position,
                get!(SURFACE_LOCALS).content_mask.bounds,
            ),
        }
    }

    #[fragment]
    pub fn fragment_surface(input: SurfaceVarying) -> Vec4f {
        if is_clipped(input.clip_distances) {
            return transparent();
        }
        let locals = get!(SURFACE_LOCALS);
        let fade = ContentMask::alpha(locals.content_mask, input.position.xy());
        if locals.color_format == SurfaceColorFormat::Yuv {
            return sample_yuv_surface(input.texture_position) * locals.opacity * fade;
        }
        texture_sample_level(
            SURFACE_TEXTURE,
            SURFACE_SAMPLER,
            input.texture_position,
            0.0,
        ) * locals.opacity
            * fade
    }
}

#[wgsl_rs::wgsl]
pub mod blur {
    use super::super::common::*;
    use super::super::corner_smoothing::*;
    use wgsl_rs::std::*;

    #[repr(C)]
    #[derive(Clone, Copy, Wgsl)]
    pub struct BlurUniforms {
        pub bounds: Bounds,
        pub content_mask: Bounds,
        pub corner_radii: Corners,
        /// Gaussian passes: the axis to blur along, in texels.
        pub direction: Vec2f,
        pub opacity: f32,
        pub corner_smoothing: f32,
        /// Allocated size of the sampled texture, in texels.
        pub source_size: Vec2f,
        /// Texels of the sampled texture holding valid content; samples clamp to them.
        pub sample_extent: Vec2f,
        /// Sampled texels per target pixel.
        pub texel_scale: Vec2f,
        pub composite_clip: BlurCompositeClip,
        pub downsample_mode: DownsampleMode,
        /// Box downsampling: bilinear taps per axis, each averaging a 2x2 block.
        pub box_taps_per_axis: u32,
        /// Gaussian passes: symmetric `(offset, weight)` pairs in `taps`, beside the center.
        pub tap_pair_count: u32,
        pub center_weight: f32,
        pub padding: u32,
        /// Two `(offset, weight)` pairs per vector; see `gpui_render::blur::BlurKernel`.
        pub taps: [Vec4f; 12],
    }
    uniform!(group(1), binding(0), BLUR_LOCALS: BlurUniforms);
    texture!(group(1), binding(1), BLUR_TEXTURE: Texture2D<f32>);
    sampler!(group(1), binding(2), BLUR_SAMPLER: Sampler);

    #[derive(Clone, Copy, Wgsl)]
    pub struct BlurCompositeVertexData {
        pub position: Vec4f,
        pub texture_coordinates: Vec2f,
        pub clip_distances: Vec4f,
    }

    pub fn prepare_blur_composite_vertex(vertex_id: u32) -> BlurCompositeVertexData {
        let vertex = rectangle_vertex(vertex_id, get!(BLUR_LOCALS).bounds);
        BlurCompositeVertexData {
            position: vertex.clip_position,
            texture_coordinates: vertex.unit_position,
            clip_distances: clip_distances(
                vertex.viewport_position,
                get!(BLUR_LOCALS).content_mask,
            ),
        }
    }

    pub fn blur_composite_color(position: Vec2f, coverage: f32) -> Vec4f {
        let blurred = sample_blur_texels(position * get!(BLUR_LOCALS).texel_scale);
        let factor = coverage * get!(BLUR_LOCALS).opacity;
        vec4f(
            blurred.x * factor,
            blurred.y * factor,
            blurred.z * factor,
            blurred.w * factor,
        )
    }

    #[derive(Wgsl)]
    pub struct BlurVarying {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub texture_coordinates: Vec2f,
        #[location(3)]
        pub clip_distances: Vec4f,
    }

    #[vertex]
    pub fn vertex_blur_fullscreen(#[builtin(vertex_index)] vertex_id: u32) -> BlurVarying {
        let vertex = fullscreen_vertex(vertex_id);
        BlurVarying {
            position: vertex.clip_position,
            texture_coordinates: vertex.texture_coordinates,
            clip_distances: unclipped_distances(),
        }
    }

    /// Samples the blur texture at a position in texels, clamped to the valid extent so a
    /// partially used texture behaves like clamp-to-edge sampling of just that extent.
    pub fn sample_blur_texels(position: Vec2f) -> Vec4f {
        let clamped = clamp(
            position,
            vec2f(0.5, 0.5),
            get!(BLUR_LOCALS).sample_extent - 0.5,
        );
        texture_sample_level(
            BLUR_TEXTURE,
            BLUR_SAMPLER,
            clamped / get!(BLUR_LOCALS).source_size,
            0.0,
        )
    }

    #[fragment]
    pub fn fragment_blur_downsample(input: BlurVarying) -> Vec4f {
        if get!(BLUR_LOCALS).downsample_mode == DownsampleMode::Box {
            // Average the block of source texels under this texel. Each bilinear tap lands on
            // the shared corner of a 2x2 block, so a factor-wide block takes (factor / 2)^2.
            let taps = get!(BLUR_LOCALS).box_taps_per_axis;
            let block = get!(BLUR_LOCALS).texel_scale;
            let spacing = block / taps as f32;
            let first = floor(input.position.xy()) * block + spacing * 0.5;
            let mut color = vec4f(0.0, 0.0, 0.0, 0.0);
            let mut row = 0u32;
            while row < taps {
                let mut column = 0u32;
                while column < taps {
                    color = color
                        + sample_blur_texels(first + vec2f(column as f32, row as f32) * spacing);
                    column += 1u32;
                }
                row += 1u32;
            }
            return color / (taps * taps) as f32;
        }
        texture_sample_level(BLUR_TEXTURE, BLUR_SAMPLER, input.texture_coordinates, 0.0)
    }

    /// One pass of a separable Gaussian, with weights and merged tap offsets prepared on the
    /// CPU. Reads and writes share a resolution, so `position` is also the source position.
    pub fn gaussian_blur(position: Vec2f) -> Vec4f {
        let direction = get!(BLUR_LOCALS).direction;
        let mut color = sample_blur_texels(position) * get!(BLUR_LOCALS).center_weight;
        let mut pair = 0u32;
        while pair < get!(BLUR_LOCALS).tap_pair_count {
            let taps = get!(BLUR_LOCALS).taps[(pair / 2u32) as usize];
            let second = (pair % 2u32) == 1u32;
            let offset = direction * select(taps.x, taps.z, second);
            let weight = select(taps.y, taps.w, second);
            color = color
                + (sample_blur_texels(position - offset) + sample_blur_texels(position + offset))
                    * weight;
            pair += 1u32;
        }
        color
    }

    #[fragment]
    pub fn fragment_blur(input: BlurVarying) -> Vec4f {
        gaussian_blur(input.position.xy())
    }

    #[vertex]
    pub fn vertex_blur_composite(#[builtin(vertex_index)] vertex_id: u32) -> BlurVarying {
        let vertex = prepare_blur_composite_vertex(vertex_id);
        BlurVarying {
            position: vertex.position,
            texture_coordinates: vertex.texture_coordinates,
            clip_distances: vertex.clip_distances,
        }
    }

    #[fragment]
    pub fn fragment_blur_composite(input: BlurVarying) -> Vec4f {
        if is_clipped(input.clip_distances) {
            return transparent();
        }
        let coverage = select(
            1.0,
            antialiased_coverage(rounded_rectangle_signed_distance(
                input.position.xy(),
                get!(BLUR_LOCALS).bounds,
                get!(BLUR_LOCALS).corner_radii,
            )),
            get!(BLUR_LOCALS).composite_clip == BlurCompositeClip::RoundedBounds,
        );
        blur_composite_color(input.position.xy(), coverage)
    }

    #[derive(Wgsl)]
    pub struct SmoothedBlurVarying {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub texture_coordinates: Vec2f,
        #[location(1)]
        #[interpolate(flat)]
        pub horizontal_corner_reaches: Vec4f,
        #[location(2)]
        #[interpolate(flat)]
        pub vertical_corner_reaches: Vec4f,
        #[location(3)]
        pub clip_distances: Vec4f,
        #[location(4)]
        #[interpolate(flat)]
        pub smoothing_factors: Vec4f,
        #[location(5)]
        #[interpolate(flat)]
        pub superellipse_power: f32,
    }

    #[vertex]
    pub fn vertex_smoothed_blur_composite(
        #[builtin(vertex_index)] vertex_id: u32,
    ) -> SmoothedBlurVarying {
        let vertex = prepare_blur_composite_vertex(vertex_id);
        let prepared = prepare_corners(
            get!(BLUR_LOCALS).bounds.size,
            get!(BLUR_LOCALS).corner_radii,
            get!(BLUR_LOCALS).corner_smoothing,
            true,
        );
        SmoothedBlurVarying {
            position: vertex.position,
            texture_coordinates: vertex.texture_coordinates,
            horizontal_corner_reaches: prepared.horizontal_reaches,
            vertical_corner_reaches: prepared.vertical_reaches,
            clip_distances: vertex.clip_distances,
            smoothing_factors: prepared.smoothing_factors,
            superellipse_power: prepared.superellipse_power,
        }
    }

    #[fragment]
    pub fn fragment_smoothed_blur_composite(input: SmoothedBlurVarying) -> Vec4f {
        if is_clipped(input.clip_distances) {
            return transparent();
        }
        let coverage = select(
            1.0,
            antialiased_coverage(prepared_corner_signed_distance(
                input.position.xy(),
                get!(BLUR_LOCALS).bounds,
                get!(BLUR_LOCALS).corner_radii,
                get!(BLUR_LOCALS).corner_smoothing,
                PreparedCorners {
                    horizontal_reaches: input.horizontal_corner_reaches,
                    vertical_reaches: input.vertical_corner_reaches,
                    smoothing_factors: input.smoothing_factors,
                    superellipse_power: input.superellipse_power,
                },
            )),
            get!(BLUR_LOCALS).composite_clip == BlurCompositeClip::RoundedBounds,
        );
        blur_composite_color(input.position.xy(), coverage)
    }
}
