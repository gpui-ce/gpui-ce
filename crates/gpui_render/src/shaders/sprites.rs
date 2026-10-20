#[wgsl_rs::wgsl]
pub mod underline {
    use super::super::common::*;
    use wgsl_rs::std::*;

    #[derive(Clone, Copy, Wgsl)]
    pub struct Underline {
        pub order: u32,
        pub padding: u32,
        pub bounds: Bounds,
        pub content_mask: ContentMask,
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
            clip_distances: clip_distances(vertex.viewport_position, underline.content_mask.bounds),
        }
    }

    #[fragment]
    pub fn fragment_underline(input: UnderlineVarying) -> Vec4f {
        if is_clipped(input.clip_distances) {
            return transparent();
        }
        let underline = get!(UNDERLINES)[input.underline_id as usize];
        if !is_enabled(underline.wavy) {
            return blend_color(
                input.color,
                ContentMask::alpha(underline.content_mask, input.position.xy()),
            );
        }
        blend_color(
            input.color,
            wavy_underline_coverage(underline, input.position.xy())
                * ContentMask::alpha(underline.content_mask, input.position.xy()),
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
        pub blur_radius: f32,
        pub bounds: Bounds,
        pub content_mask: ContentMask,
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
        #[location(2)]
        #[interpolate(flat)]
        pub sprite_id: u32,
        #[location(3)]
        pub clip_distances: Vec4f,
    }

    /// Blur taps on each side of the center, per axis, at most.
    pub const MAX_BLUR_TAPS_PER_SIDE: f32 = 6.0;

    pub fn blur_margin(sprite: MonochromeSprite) -> f32 {
        GAUSSIAN_CUTOFF_STANDARD_DEVIATIONS * sprite.blur_radius
    }

    /// Device pixels per tile texel are not always 1: SVG tiles are oversampled.
    pub fn tile_texels_per_pixel(sprite: MonochromeSprite) -> Vec2f {
        let margin = blur_margin(sprite);
        let tile_size = vec2f(
            sprite.tile.bounds.size.x as f32,
            sprite.tile.bounds.size.y as f32,
        );
        let unpadded_size = max(
            sprite.bounds.size - vec2f(2.0 * margin, 2.0 * margin),
            vec2f(MIN_NORMALIZED_WEIGHT, MIN_NORMALIZED_WEIGHT),
        );
        tile_size / unpadded_size
    }

    /// Gaussian-filtered tile coverage. Taps clamp half a texel inside the tile so
    /// filtering never reads a neighboring tile, and fade out beyond it as if the tile
    /// were surrounded by transparent texels.
    pub fn blurred_coverage(sprite: MonochromeSprite, tile_position: Vec2f) -> f32 {
        let dimensions = texture_dimensions(MONOCHROME_TEXTURE);
        let atlas_size = vec2f(dimensions.x as f32, dimensions.y as f32);
        let texel = vec2f(1.0, 1.0) / atlas_size;
        let tile_min = vec2f(
            sprite.tile.bounds.origin.x as f32,
            sprite.tile.bounds.origin.y as f32,
        ) / atlas_size;
        let tile_max = tile_min
            + vec2f(
                sprite.tile.bounds.size.x as f32,
                sprite.tile.bounds.size.y as f32,
            ) / atlas_size;
        let tile_center = (tile_min + tile_max) * 0.5;
        let inset_min = min(tile_min + texel * 0.5, tile_center);
        let inset_max = max(tile_max - texel * 0.5, tile_center);

        let deviation_texels = sprite.blur_radius * tile_texels_per_pixel(sprite);
        let taps_per_side = clamp(
            ceil(GAUSSIAN_CUTOFF_STANDARD_DEVIATIONS * max(deviation_texels.x, deviation_texels.y)),
            1.0,
            MAX_BLUR_TAPS_PER_SIDE,
        );
        // Taps span three standard deviations each way, at most one texel apart for
        // small blurs and half a standard deviation apart for large ones.
        let deviations_per_tap = GAUSSIAN_CUTOFF_STANDARD_DEVIATIONS / taps_per_side;
        let tap_step = deviation_texels * texel * deviations_per_tap;
        let last_tap = taps_per_side as i32;

        let mut coverage = 0.0;
        let mut total_weight = 0.0;
        let mut y = -last_tap;
        while y <= last_tap {
            let mut x = -last_tap;
            while x <= last_tap {
                let offset = vec2f(x as f32, y as f32);
                let deviations = offset * deviations_per_tap;
                let weight = exp(-0.5 * dot(deviations, deviations));
                let position = tile_position + offset * tap_step;
                // Filter as if the tile were surrounded by transparent texels.
                let outside = max(
                    max(inset_min - position, position - inset_max),
                    vec2f(0.0, 0.0),
                );
                let border = saturate(vec2f(1.0, 1.0) - outside / texel);
                coverage += weight
                    * border.x
                    * border.y
                    * texture_sample_level(
                        MONOCHROME_TEXTURE,
                        MONOCHROME_SAMPLER,
                        clamp(position, inset_min, inset_max),
                        0.0,
                    )
                    .x;
                total_weight += weight;
                x += 1;
            }
            y += 1;
        }
        coverage / total_weight
    }

    #[vertex]
    pub fn vertex_monochrome_sprite(
        #[builtin(vertex_index)] vertex_id: u32,
        #[builtin(instance_index)] instance_id: u32,
    ) -> MonochromeSpriteVarying {
        let sprite = get!(MONOCHROME_SPRITES)[instance_id as usize];
        let vertex = transformed_rectangle_vertex(vertex_id, sprite.bounds, sprite.transformation);
        let mut tile_unit_position = vertex.unit_position;
        if sprite.blur_radius > 0.0 {
            // The blur margin maps past the tile, keeping the tile's content at its size.
            let margin = blur_margin(sprite);
            let tile_size = sprite.bounds.size - vec2f(2.0 * margin, 2.0 * margin);
            tile_unit_position = (vertex.unit_position * sprite.bounds.size
                - vec2f(margin, margin))
                / max(
                    tile_size,
                    vec2f(MIN_NORMALIZED_WEIGHT, MIN_NORMALIZED_WEIGHT),
                );
        }
        MonochromeSpriteVarying {
            position: vertex.clip_position,
            tile_position: atlas_texture_coordinates(
                tile_unit_position,
                sprite.tile,
                texture_dimensions(MONOCHROME_TEXTURE),
            ),
            color: hsla_to_rgba(sprite.color),
            sprite_id: instance_id,
            clip_distances: clip_distances(vertex.viewport_position, sprite.content_mask.bounds),
        }
    }

    #[fragment]
    pub fn fragment_monochrome_sprite(input: MonochromeSpriteVarying) -> Vec4f {
        if is_clipped(input.clip_distances) {
            return transparent();
        }
        let sprite = get!(MONOCHROME_SPRITES)[input.sprite_id as usize];
        let mut sample = 0.0;
        if sprite.blur_radius > 0.0 {
            sample = blurred_coverage(sprite, input.tile_position);
        } else {
            sample = texture_sample_level(
                MONOCHROME_TEXTURE,
                MONOCHROME_SAMPLER,
                input.tile_position,
                0.0,
            )
            .x;
        }
        let corrected = apply_contrast_and_gamma_correction(
            sample,
            input.color.rgb(),
            get!(FONT_RASTERIZATION).grayscale_enhanced_contrast,
            get!(FONT_RASTERIZATION).gamma_ratios,
        );
        blend_color(
            input.color,
            corrected * ContentMask::alpha(sprite.content_mask, input.position.xy()),
        )
    }
}

#[wgsl_rs::wgsl]
pub mod polychrome_sprite {
    use super::super::common::*;
    use super::super::corner_smoothing::*;
    use wgsl_rs::std::*;

    #[derive(Clone, Copy, Wgsl)]
    pub struct PolychromeSprite {
        pub order: u32,
        pub grayscale: ShaderBool,
        pub opacity: f32,
        pub corner_smoothing: f32,
        pub bounds: Bounds,
        pub content_mask: ContentMask,
        pub corner_radii: Corners,
        pub tile: AtlasTile,
    }
    storage!(group(1), binding(0), POLYCHROME_SPRITES: RuntimeArray<PolychromeSprite>);
    texture!(group(1), binding(1), POLYCHROME_TEXTURE: Texture2D<f32>);
    sampler!(group(1), binding(2), POLYCHROME_SAMPLER: Sampler);

    #[derive(Clone, Copy, Wgsl)]
    pub struct PolychromeVertexData {
        pub position: Vec4f,
        pub tile_position: Vec2f,
        pub sprite_id: u32,
        pub clip_distances: Vec4f,
    }

    pub fn prepare_polychrome_vertex(
        vertex_id: u32,
        instance_id: u32,
        sprite: PolychromeSprite,
    ) -> PolychromeVertexData {
        let vertex = rectangle_vertex(vertex_id, sprite.bounds);
        PolychromeVertexData {
            position: vertex.clip_position,
            tile_position: atlas_texture_coordinates(
                vertex.unit_position,
                sprite.tile,
                texture_dimensions(POLYCHROME_TEXTURE),
            ),
            sprite_id: instance_id,
            clip_distances: clip_distances(vertex.viewport_position, sprite.content_mask.bounds),
        }
    }

    pub fn polychrome_color(sprite: PolychromeSprite, tile_position: Vec2f) -> Vec4f {
        let sample =
            texture_sample_level(POLYCHROME_TEXTURE, POLYCHROME_SAMPLER, tile_position, 0.0);
        let grayscale = dot(sample.rgb(), LINEAR_RGB_LUMA_WEIGHTS);
        select(
            sample,
            vec4f(grayscale, grayscale, grayscale, sample.w),
            is_enabled(sprite.grayscale),
        )
    }

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
        let vertex = prepare_polychrome_vertex(vertex_id, instance_id, sprite);
        PolychromeSpriteVarying {
            position: vertex.position,
            tile_position: vertex.tile_position,
            sprite_id: vertex.sprite_id,
            clip_distances: vertex.clip_distances,
        }
    }

    #[fragment]
    pub fn fragment_polychrome_sprite(input: PolychromeSpriteVarying) -> Vec4f {
        if is_clipped(input.clip_distances) {
            return transparent();
        }
        let sprite = get!(POLYCHROME_SPRITES)[input.sprite_id as usize];
        let color = polychrome_color(sprite, input.tile_position);
        blend_color(
            color,
            sprite.opacity
                * antialiased_coverage(rounded_rectangle_signed_distance(
                    input.position.xy(),
                    sprite.bounds,
                    sprite.corner_radii,
                ))
                * ContentMask::alpha(sprite.content_mask, input.position.xy()),
        )
    }

    #[derive(Wgsl)]
    pub struct SmoothedPolychromeSpriteVarying {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        pub tile_position: Vec2f,
        #[location(1)]
        #[interpolate(flat)]
        pub sprite_id: u32,
        #[location(2)]
        #[interpolate(flat)]
        pub horizontal_corner_reaches: Vec4f,
        #[location(3)]
        pub clip_distances: Vec4f,
        #[location(4)]
        #[interpolate(flat)]
        pub vertical_corner_reaches: Vec4f,
        #[location(5)]
        #[interpolate(flat)]
        pub smoothing_factors: Vec4f,
        #[location(6)]
        #[interpolate(flat)]
        pub superellipse_power: f32,
    }

    #[vertex]
    pub fn vertex_smoothed_polychrome_sprite(
        #[builtin(vertex_index)] vertex_id: u32,
        #[builtin(instance_index)] instance_id: u32,
    ) -> SmoothedPolychromeSpriteVarying {
        let sprite = get!(POLYCHROME_SPRITES)[instance_id as usize];
        let vertex = prepare_polychrome_vertex(vertex_id, instance_id, sprite);
        let prepared = prepare_corners(
            sprite.bounds.size,
            sprite.corner_radii,
            sprite.corner_smoothing,
            true,
        );

        SmoothedPolychromeSpriteVarying {
            position: vertex.position,
            tile_position: vertex.tile_position,
            sprite_id: vertex.sprite_id,
            horizontal_corner_reaches: prepared.horizontal_reaches,
            clip_distances: vertex.clip_distances,
            vertical_corner_reaches: prepared.vertical_reaches,
            smoothing_factors: prepared.smoothing_factors,
            superellipse_power: prepared.superellipse_power,
        }
    }

    #[fragment]
    pub fn fragment_smoothed_polychrome_sprite(input: SmoothedPolychromeSpriteVarying) -> Vec4f {
        if is_clipped(input.clip_distances) {
            return transparent();
        }
        let sprite = get!(POLYCHROME_SPRITES)[input.sprite_id as usize];
        let color = polychrome_color(sprite, input.tile_position);
        blend_color(
            color,
            sprite.opacity
                * antialiased_coverage(prepared_corner_signed_distance(
                    input.position.xy(),
                    sprite.bounds,
                    sprite.corner_radii,
                    sprite.corner_smoothing,
                    PreparedCorners {
                        horizontal_reaches: input.horizontal_corner_reaches,
                        vertical_reaches: input.vertical_corner_reaches,
                        smoothing_factors: input.smoothing_factors,
                        superellipse_power: input.superellipse_power,
                    },
                ))
                * ContentMask::alpha(sprite.content_mask, input.position.xy()),
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
        pub content_mask: ContentMask,
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
        #[location(2)]
        #[interpolate(flat)]
        pub sprite_id: u32,
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
            sprite_id: instance_id,
            clip_distances: clip_distances(vertex.viewport_position, sprite.content_mask.bounds),
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
        let sprite = get!(SUBPIXEL_SPRITES)[input.sprite_id as usize];
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
        let mask_alpha = ContentMask::alpha(sprite.content_mask, input.position.xy());
        SubpixelSpriteFragmentOutput {
            foreground: vec4f(input.color.x, input.color.y, input.color.z, 1.0),
            alpha: vec4f(
                input.color.w * alpha_corrected.x * mask_alpha,
                input.color.w * alpha_corrected.y * mask_alpha,
                input.color.w * alpha_corrected.z * mask_alpha,
                1.0,
            ),
        }
    }
}
