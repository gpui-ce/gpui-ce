#[wgsl_rs::wgsl]
pub mod underline {
    use super::super::common::*;
    use wgsl_rs::std::*;

    #[derive(Clone, Copy, Wgsl)]
    pub struct Underline {
        pub order: u32,
        pub padding: u32,
        pub bounds: Bounds,
        pub content_mask: Bounds,
        pub color: Hsla,
        pub thickness: f32,
        pub wavy: ShaderBool,
    }
    storage!(group(1), binding(0), UNDERLINES: RuntimeArray<Underline>);

    pub fn wavy_underline_coverage(underline: Underline, position: Vec2f) -> f32 {
        let normalized_position =
            (position - underline.bounds.origin) / underline.bounds.size.y - vec2f(0.0, 0.5);
        let angular_frequency =
            PI * UNDERLINE_WAVE_FREQUENCY * underline.thickness / underline.bounds.size.y;
        let amplitude = underline.thickness * UNDERLINE_WAVE_HEIGHT_RATIO / underline.bounds.size.y;
        let wave_height = sin(normalized_position.x * angular_frequency) * amplitude;
        let wave_slope =
            cos(normalized_position.x * angular_frequency) * amplitude * angular_frequency;
        let signed_distance = (normalized_position.y - wave_height)
            / sqrt(1.0 + wave_slope * wave_slope)
            * underline.bounds.size.y;
        let stroke_distance = abs(signed_distance) - underline.thickness * 0.5;
        antialiased_coverage(stroke_distance)
    }

    #[derive(Wgsl)]
    pub struct UnderlineVarying {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        #[interpolate(flat)]
        pub color: Vec4f,
        #[location(1)]
        #[interpolate(flat)]
        pub underline_id: u32,
        #[location(3)]
        pub clip_distances: Vec4f,
    }

    #[vertex]
    pub fn vertex_underline(
        #[builtin(vertex_index)] vertex_id: u32,
        #[builtin(instance_index)] instance_id: u32,
    ) -> UnderlineVarying {
        let underline = get!(UNDERLINES)[instance_id as usize];
        let vertex = rectangle_vertex(vertex_id, underline.bounds);
        UnderlineVarying {
            position: vertex.clip_position,
            color: hsla_to_rgba(underline.color),
            underline_id: instance_id,
            clip_distances: clip_distances(vertex.viewport_position, underline.content_mask),
        }
    }

    #[fragment]
    pub fn fragment_underline(input: UnderlineVarying) -> Vec4f {
        if is_clipped(input.clip_distances) {
            return transparent();
        }
        let underline = get!(UNDERLINES)[input.underline_id as usize];
        if !is_enabled(underline.wavy) {
            return blend_color(input.color, 1.0);
        }
        blend_color(
            input.color,
            wavy_underline_coverage(underline, input.position.xy()),
        )
    }
}

#[wgsl_rs::wgsl]
pub mod monochrome_sprite {
    use super::super::common::*;
    use wgsl_rs::std::*;

    #[derive(Clone, Copy, Wgsl)]
    pub struct MonochromeSprite {
        pub order: u32,
        pub padding: u32,
        pub bounds: Bounds,
        pub content_mask: Bounds,
        pub color: Hsla,
        pub tile: AtlasTile,
        pub transformation: TransformationMatrix,
    }
    storage!(group(1), binding(0), MONOCHROME_SPRITES: RuntimeArray<MonochromeSprite>);
    texture!(group(1), binding(1), MONOCHROME_TEXTURE: Texture2D<f32>);
    sampler!(group(1), binding(2), MONOCHROME_SAMPLER: Sampler);

    #[derive(Wgsl)]
    pub struct MonochromeSpriteVarying {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub tile_position: Vec2f,
        #[location(1)]
        #[interpolate(flat)]
        pub color: Vec4f,
        #[location(3)]
        pub clip_distances: Vec4f,
    }

    #[vertex]
    pub fn vertex_monochrome_sprite(
        #[builtin(vertex_index)] vertex_id: u32,
        #[builtin(instance_index)] instance_id: u32,
    ) -> MonochromeSpriteVarying {
        let sprite = get!(MONOCHROME_SPRITES)[instance_id as usize];
        let vertex = transformed_rectangle_vertex(vertex_id, sprite.bounds, sprite.transformation);
        MonochromeSpriteVarying {
            position: vertex.clip_position,
            tile_position: atlas_texture_coordinates(
                vertex.unit_position,
                sprite.tile,
                texture_dimensions(MONOCHROME_TEXTURE),
            ),
            color: hsla_to_rgba(sprite.color),
            clip_distances: clip_distances(vertex.viewport_position, sprite.content_mask),
        }
    }

    #[fragment]
    pub fn fragment_monochrome_sprite(input: MonochromeSpriteVarying) -> Vec4f {
        if is_clipped(input.clip_distances) {
            return transparent();
        }
        let sample = texture_sample_level(
            MONOCHROME_TEXTURE,
            MONOCHROME_SAMPLER,
            input.tile_position,
            0.0,
        )
        .x;
        let corrected = apply_contrast_and_gamma_correction(
            sample,
            input.color.rgb(),
            get!(FONT_RASTERIZATION).grayscale_enhanced_contrast,
            get!(FONT_RASTERIZATION).gamma_ratios,
        );
        blend_color(input.color, corrected)
    }
}

#[wgsl_rs::wgsl]
pub mod polychrome_sprite {
    use super::super::common::*;
    use wgsl_rs::std::*;

    #[derive(Clone, Copy, Wgsl)]
    pub struct PolychromeSprite {
        pub order: u32,
        pub padding: u32,
        pub grayscale: ShaderBool,
        pub opacity: f32,
        pub bounds: Bounds,
        pub content_mask: Bounds,
        pub corner_radii: Corners,
        pub tile: AtlasTile,
    }
    storage!(group(1), binding(0), POLYCHROME_SPRITES: RuntimeArray<PolychromeSprite>);
    texture!(group(1), binding(1), POLYCHROME_TEXTURE: Texture2D<f32>);
    sampler!(group(1), binding(2), POLYCHROME_SAMPLER: Sampler);

    #[derive(Wgsl)]
    pub struct PolychromeSpriteVarying {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub tile_position: Vec2f,
        #[location(1)]
        #[interpolate(flat)]
        pub sprite_id: u32,
        #[location(3)]
        pub clip_distances: Vec4f,
    }

    #[vertex]
    pub fn vertex_polychrome_sprite(
        #[builtin(vertex_index)] vertex_id: u32,
        #[builtin(instance_index)] instance_id: u32,
    ) -> PolychromeSpriteVarying {
        let sprite = get!(POLYCHROME_SPRITES)[instance_id as usize];
        let vertex = rectangle_vertex(vertex_id, sprite.bounds);
        PolychromeSpriteVarying {
            position: vertex.clip_position,
            tile_position: atlas_texture_coordinates(
                vertex.unit_position,
                sprite.tile,
                texture_dimensions(POLYCHROME_TEXTURE),
            ),
            sprite_id: instance_id,
            clip_distances: clip_distances(vertex.viewport_position, sprite.content_mask),
        }
    }

    #[fragment]
    pub fn fragment_polychrome_sprite(input: PolychromeSpriteVarying) -> Vec4f {
        if is_clipped(input.clip_distances) {
            return transparent();
        }
        let sprite = get!(POLYCHROME_SPRITES)[input.sprite_id as usize];
        let sample = texture_sample_level(
            POLYCHROME_TEXTURE,
            POLYCHROME_SAMPLER,
            input.tile_position,
            0.0,
        );
        let grayscale = dot(sample.rgb(), LINEAR_RGB_LUMA_WEIGHTS);
        let color = select(
            sample,
            vec4f(grayscale, grayscale, grayscale, sample.w),
            is_enabled(sprite.grayscale),
        );
        blend_color(
            color,
            sprite.opacity
                * antialiased_coverage(rounded_rectangle_signed_distance(
                    input.position.xy(),
                    sprite.bounds,
                    sprite.corner_radii,
                )),
        )
    }
}

/// Dual-source LCD glyph pipeline; a separate module keeps the shared instance slot legal.
#[wgsl_rs::wgsl(skip_validation)]
pub mod subpixel_sprite {
    use super::super::common::*;
    use wgsl_rs::std::*;

    #[derive(Clone, Copy, Wgsl)]
    pub struct SubpixelSprite {
        pub order: u32,
        pub padding: u32,
        pub bounds: Bounds,
        pub content_mask: Bounds,
        pub color: Hsla,
        pub tile: AtlasTile,
        pub transformation: TransformationMatrix,
    }
    storage!(group(1), binding(0), SUBPIXEL_SPRITES: RuntimeArray<SubpixelSprite>);
    texture!(group(1), binding(1), SPRITE_TEXTURE: Texture2D<f32>);
    sampler!(group(1), binding(2), SPRITE_SAMPLER: Sampler);

    #[derive(Wgsl)]
    pub struct SubpixelSpriteOutput {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub tile_position: Vec2f,
        #[location(1)]
        #[interpolate(flat)]
        pub color: Vec4f,
        #[location(3)]
        pub clip_distances: Vec4f,
    }
    #[derive(Wgsl)]
    pub struct SubpixelSpriteFragmentOutput {
        #[location(0)]
        #[blend_src(0)]
        pub foreground: Vec4f,
        #[location(0)]
        #[blend_src(1)]
        pub alpha: Vec4f,
    }

    #[vertex]
    pub fn vertex_subpixel_sprite(
        #[builtin(vertex_index)] vertex_id: u32,
        #[builtin(instance_index)] instance_id: u32,
    ) -> SubpixelSpriteOutput {
        let sprite = get!(SUBPIXEL_SPRITES)[instance_id as usize];
        let vertex = transformed_rectangle_vertex(vertex_id, sprite.bounds, sprite.transformation);
        SubpixelSpriteOutput {
            position: vertex.clip_position,
            tile_position: atlas_texture_coordinates(
                vertex.unit_position,
                sprite.tile,
                texture_dimensions(SPRITE_TEXTURE),
            ),
            color: hsla_to_rgba(sprite.color),
            clip_distances: clip_distances(vertex.viewport_position, sprite.content_mask),
        }
    }

    #[fragment]
    pub fn fragment_subpixel_sprite(input: SubpixelSpriteOutput) -> SubpixelSpriteFragmentOutput {
        if is_clipped(input.clip_distances) {
            return SubpixelSpriteFragmentOutput {
                foreground: transparent(),
                alpha: transparent(),
            };
        }
        let sampled =
            texture_sample_level(SPRITE_TEXTURE, SPRITE_SAMPLER, input.tile_position, 0.0).rgb();
        let sample = select(
            sampled,
            sampled.bgr(),
            is_enabled(get!(FONT_RASTERIZATION).uses_blue_green_red_subpixel_order),
        );
        let alpha_corrected = apply_contrast_and_gamma_correction3(
            sample,
            input.color.rgb(),
            get!(FONT_RASTERIZATION).subpixel_enhanced_contrast,
            get!(FONT_RASTERIZATION).gamma_ratios,
        );
        SubpixelSpriteFragmentOutput {
            foreground: vec4f(input.color.x, input.color.y, input.color.z, 1.0),
            alpha: vec4f(
                input.color.w * alpha_corrected.x,
                input.color.w * alpha_corrected.y,
                input.color.w * alpha_corrected.z,
                1.0,
            ),
        }
    }
}
