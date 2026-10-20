pub use crate::shaders::{
    blur::BlurUniforms,
    common::{
        BlurCompositeClip as ShaderBlurCompositeClip, Bounds as ShaderBounds,
        Corners as ShaderCorners, DownsampleMode,
    },
};
use gpui::{Bounds, Corners, ScaledPixels};
use wgsl_rs::std::{vec2f, vec4f};

/// Scratch textures are allocated at this fraction of the target size. Wider blurs run at a
/// coarser factor inside the top-left part of the same textures.
pub const MIN_DOWNSAMPLE_FACTOR: u32 = 2;
/// Coarsest downsample factor. Blurs that still exceed [`MAX_KERNEL_RADIUS`] at this factor
/// spread their taps over the kernel instead of growing it.
pub const MAX_DOWNSAMPLE_FACTOR: u32 = 16;
/// Largest kernel, in taps per side at the downsampled resolution, that is evaluated exactly.
pub const MAX_KERNEL_RADIUS: u32 = 48;
/// Symmetric tap pairs the shader evaluates at most per pass. Adjacent taps of the exact kernel
/// share one bilinear sample, so this is half of [`MAX_KERNEL_RADIUS`].
pub const MAX_TAP_PAIRS: u32 = MAX_KERNEL_RADIUS / 2;
/// Uniform vectors holding the tap pairs, two `(offset, weight)` pairs per vector. Must match
/// the length of `BlurUniforms::taps`.
pub const KERNEL_VECTORS: usize = (MAX_TAP_PAIRS as usize).div_ceil(2);
pub const GAUSSIAN_CUTOFF_STANDARD_DEVIATIONS: f32 = 3.0;
/// Below this many downsampled texels per standard deviation, the box prefilter and bilinear
/// upsample would start to show, so a blur stays at the finer factor.
const MIN_TEXELS_PER_STANDARD_DEVIATION: u32 = 8;

const _: () = assert!(MAX_KERNEL_RADIUS.is_multiple_of(2));
const _: () = assert!(MAX_DOWNSAMPLE_FACTOR.is_power_of_two());
const _: () = assert!(MIN_DOWNSAMPLE_FACTOR.is_power_of_two() && MIN_DOWNSAMPLE_FACTOR >= 2);

/// Texture dimension for the downsampled blur passes; allocations must agree.
pub fn downsampled_dimension(dimension: u32) -> u32 {
    dimension.div_ceil(MIN_DOWNSAMPLE_FACTOR).max(1)
}

/// Downsample factor for a Gaussian with the given standard deviation in device pixels.
///
/// The factor doubles while the blur keeps at least [`MIN_TEXELS_PER_STANDARD_DEVIATION`]
/// texels per standard deviation. Up to [`MAX_DOWNSAMPLE_FACTOR`], this bounds the kernel to
/// [`MAX_KERNEL_RADIUS`] taps per side, so wide blurs cost about as much as narrow ones.
pub fn downsample_factor(standard_deviation: f32) -> u32 {
    let mut factor = MIN_DOWNSAMPLE_FACTOR;
    while factor < MAX_DOWNSAMPLE_FACTOR
        && standard_deviation >= (2 * factor * MIN_TEXELS_PER_STANDARD_DEVIATION) as f32
    {
        factor *= 2;
    }
    factor
}

#[derive(Clone, Copy)]
pub enum BlurAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy)]
pub enum FilterCompositeClip {
    RoundedBounds,
    ContentShape,
}

#[derive(Clone, Copy)]
pub struct FilterCompositeParameters {
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: Bounds<ScaledPixels>,
    pub corner_radii: Corners<ScaledPixels>,
    pub corner_smoothing: f32,
    pub blur_radius: f32,
    pub opacity: f32,
    pub clip: FilterCompositeClip,
}

/// Normalized, symmetric Gaussian kernel, prepared once per blur and shared by every pixel.
///
/// The shader reads the center weight plus `pair_count` symmetric `(offset, weight)` pairs.
/// When the kernel fits in [`MAX_KERNEL_RADIUS`] taps, each pair merges two adjacent taps into
/// one bilinear sample placed between them. That reproduces the discrete kernel exactly,
/// including at clamp-to-edge boundaries, with half the texture reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlurKernel {
    pub center_weight: f32,
    pub pair_count: u32,
    /// `(offset, weight, offset, weight)`, with offsets in texels from the center.
    pub taps: [[f32; 4]; KERNEL_VECTORS],
    /// Furthest texel from the center that any sample reads, including the bilinear footprint.
    pub reach: u32,
}

impl BlurKernel {
    /// Kernel for a Gaussian with the given standard deviation in texels, cut off at
    /// [`GAUSSIAN_CUTOFF_STANDARD_DEVIATIONS`]. Returns `None` for non-positive deviations.
    pub fn new(standard_deviation: f32) -> Option<Self> {
        if standard_deviation.is_nan() || standard_deviation <= 0.0 {
            return None;
        }
        let radius = (GAUSSIAN_CUTOFF_STANDARD_DEVIATIONS * standard_deviation)
            .ceil()
            .max(1.0);
        let weight = |distance: f32| {
            (-(distance * distance) / (2.0 * standard_deviation * standard_deviation)).exp()
        };

        let mut pairs = [(0.0f32, 0.0f32); MAX_TAP_PAIRS as usize];
        let pair_count;
        if radius <= MAX_KERNEL_RADIUS as f32 {
            let radius = radius as u32;
            pair_count = radius.div_ceil(2);
            for (index, pair) in pairs.iter_mut().take(pair_count as usize).enumerate() {
                let near = 2 * index as u32 + 1;
                let near_weight = weight(near as f32);
                let far_weight = if near < radius {
                    weight((near + 1) as f32)
                } else {
                    0.0
                };
                let combined = near_weight + far_weight;
                let offset = if combined > 0.0 {
                    near as f32 + far_weight / combined
                } else {
                    near as f32
                };
                *pair = (offset, combined);
            }
        } else {
            // Only reachable past the coarsest downsample factor: spread the available taps
            // evenly over the whole support instead of truncating the blur.
            pair_count = MAX_TAP_PAIRS;
            let spacing = radius / MAX_TAP_PAIRS as f32;
            for (index, pair) in pairs.iter_mut().enumerate() {
                let offset = (index + 1) as f32 * spacing;
                *pair = (offset, weight(offset));
            }
        }

        let pairs = &pairs[..pair_count as usize];
        let total = 1.0 + 2.0 * pairs.iter().map(|(_, weight)| weight).sum::<f32>();
        let mut taps = [[0.0; 4]; KERNEL_VECTORS];
        let mut reach = 0.0f32;
        for (index, (offset, weight)) in pairs.iter().enumerate() {
            let lane = (index % 2) * 2;
            taps[index / 2][lane] = *offset;
            taps[index / 2][lane + 1] = weight / total;
            reach = reach.max(*offset);
        }
        Some(Self {
            center_weight: 1.0 / total,
            pair_count,
            taps,
            reach: reach.ceil() as u32 + 1,
        })
    }

    /// The `(offset, weight)` pair at `index`.
    pub fn pair(&self, index: usize) -> (f32, f32) {
        let vector = self.taps[index / 2];
        let lane = (index % 2) * 2;
        (vector[lane], vector[lane + 1])
    }
}

/// A texture sampled by a filter pass, and how target pixels map onto its texels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterSource {
    /// Allocated size of the texture, in texels.
    pub texture_size: [u32; 2],
    /// Texels holding valid content, starting at the origin. Samples clamp to this region as if
    /// it were the whole texture.
    pub extent: [u32; 2],
    /// Source texels per target pixel.
    pub texel_scale: [f32; 2],
}

impl FilterSource {
    /// A full-resolution texture covering the whole target.
    pub fn full_resolution(width: u32, height: u32) -> Self {
        Self {
            texture_size: [width, height],
            extent: [width, height],
            texel_scale: [1.0, 1.0],
        }
    }
}

/// One pass of a blur: its uniforms and the texels it has to write.
#[derive(Clone, Copy)]
pub struct BlurPass {
    pub uniforms: BlurUniforms,
    pub scissor: ScissorRectangle,
}

/// The downsample and separable Gaussian passes of a blur, each limited to the texels that the
/// following pass reads.
///
/// Every pass renders into a scratch texture of [`downsampled_dimension`] size; the passes
/// alternate between two of them, so the vertical result lands in the texture the downsample
/// pass wrote.
#[derive(Clone, Copy)]
pub struct BlurPlan {
    pub downsample_factor: u32,
    pub downsample: BlurPass,
    pub horizontal: BlurPass,
    pub vertical: BlurPass,
    /// How the composite samples the blurred result.
    pub result: FilterSource,
}

impl BlurPlan {
    /// Plans a blur of a full-resolution source of `target_size`, where `blur_radius` is the
    /// Gaussian's standard deviation in device pixels and `region` is where the composite will
    /// sample the result (see [`visible_composite_region`]). Returns `None` when the radius
    /// doesn't blur.
    pub fn new(
        blur_radius: f32,
        region: Bounds<ScaledPixels>,
        target_size: [u32; 2],
    ) -> Option<Self> {
        let factor = downsample_factor(blur_radius);
        let kernel = BlurKernel::new(blur_radius / factor as f32)?;
        let full = target_size.map(|dimension| dimension.max(1));
        let texture_size = full.map(downsampled_dimension);
        let extent = full.map(|dimension| dimension.div_ceil(factor).max(1));
        let texel_scale = [0, 1].map(|axis| extent[axis] as f32 / full[axis] as f32);

        let region_minimum = [region.origin.x.0, region.origin.y.0];
        let region_maximum = [
            region.origin.x.0 + region.size.width.0,
            region.origin.y.0 + region.size.height.0,
        ];
        // Texels that the composite's bilinear samples touch, and those the kernel reaches
        // from them.
        let sampled = [0, 1].map(|axis| {
            clamp_span(
                (region_minimum[axis] * texel_scale[axis]).floor() as i64 - 1,
                (region_maximum[axis] * texel_scale[axis]).ceil() as i64 + 1,
                extent[axis],
            )
        });
        let reached = [0, 1].map(|axis| {
            let [minimum, maximum] = sampled[axis];
            clamp_span(
                minimum as i64 - kernel.reach as i64,
                maximum as i64 + kernel.reach as i64,
                extent[axis],
            )
        });
        let scissor = |x: [u32; 2], y: [u32; 2]| ScissorRectangle {
            x: x[0],
            y: y[0],
            width: x[1] - x[0],
            height: y[1] - y[0],
        };

        let mut downsample = with_source(
            empty_uniforms(DownsampleMode::Box),
            FilterSource::full_resolution(full[0], full[1]),
        );
        downsample.texel_scale = vec2f(1.0 / texel_scale[0], 1.0 / texel_scale[1]);
        downsample.box_taps_per_axis = factor / 2;
        let gaussian = |axis| BlurUniforms {
            direction: match axis {
                BlurAxis::Horizontal => vec2f(1.0, 0.0),
                BlurAxis::Vertical => vec2f(0.0, 1.0),
            },
            center_weight: kernel.center_weight,
            tap_pair_count: kernel.pair_count,
            taps: kernel.taps.map(|[offset0, weight0, offset1, weight1]| {
                vec4f(offset0, weight0, offset1, weight1)
            }),
            ..with_source(
                empty_uniforms(DownsampleMode::Copy),
                FilterSource {
                    texture_size,
                    extent,
                    texel_scale: [1.0, 1.0],
                },
            )
        };

        Some(Self {
            downsample_factor: factor,
            // The horizontal pass reads `reach` texels to either side of what it writes.
            downsample: BlurPass {
                uniforms: downsample,
                scissor: scissor(reached[0], reached[1]),
            },
            // The vertical pass reads `reach` texels above and below what it writes.
            horizontal: BlurPass {
                uniforms: gaussian(BlurAxis::Horizontal),
                scissor: scissor(sampled[0], reached[1]),
            },
            vertical: BlurPass {
                uniforms: gaussian(BlurAxis::Vertical),
                scissor: scissor(sampled[0], sampled[1]),
            },
            result: FilterSource {
                texture_size,
                extent,
                texel_scale,
            },
        })
    }
}

fn clamp_span(minimum: i64, maximum: i64, extent: u32) -> [u32; 2] {
    let minimum = minimum.clamp(0, extent as i64);
    let maximum = maximum.clamp(minimum, extent as i64);
    [minimum as u32, maximum as u32]
}

/// Part of a `target_size` target that a filter composite drawn over `bounds` can change, after
/// clipping to `content_mask`. `None` when nothing is visible.
pub fn visible_composite_region(
    bounds: Bounds<ScaledPixels>,
    content_mask: Bounds<ScaledPixels>,
    target_size: [u32; 2],
) -> Option<Bounds<ScaledPixels>> {
    let target = Bounds::new(
        gpui::point(ScaledPixels(0.0), ScaledPixels(0.0)),
        gpui::size(
            ScaledPixels(target_size[0] as f32),
            ScaledPixels(target_size[1] as f32),
        ),
    );
    let region = bounds.intersect(&content_mask).intersect(&target);
    (region.size.width.0 > 0.0 && region.size.height.0 > 0.0).then_some(region)
}

impl BlurUniforms {
    pub fn copy(size: [f32; 2]) -> Self {
        let mut uniforms = empty_uniforms(DownsampleMode::Copy);
        uniforms.source_size = vec2f(size[0], size[1]);
        uniforms.sample_extent = vec2f(size[0], size[1]);
        uniforms
    }

    pub fn composite(
        bounds: Bounds<ScaledPixels>,
        content_mask: Bounds<ScaledPixels>,
        corner_radii: Corners<ScaledPixels>,
        corner_smoothing: f32,
        opacity: f32,
        clip: FilterCompositeClip,
        source: FilterSource,
    ) -> Self {
        Self {
            bounds: bounds.into(),
            content_mask: content_mask.into(),
            corner_radii: corner_radii.into(),
            corner_smoothing,
            opacity,
            composite_clip: match clip {
                FilterCompositeClip::RoundedBounds => ShaderBlurCompositeClip::RoundedBounds,
                FilterCompositeClip::ContentShape => ShaderBlurCompositeClip::None,
            },
            ..with_source(empty_uniforms(DownsampleMode::Copy), source)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScissorRectangle {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl ScissorRectangle {
    pub fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

fn with_source(mut uniforms: BlurUniforms, source: FilterSource) -> BlurUniforms {
    uniforms.source_size = vec2f(source.texture_size[0] as f32, source.texture_size[1] as f32);
    uniforms.sample_extent = vec2f(source.extent[0] as f32, source.extent[1] as f32);
    uniforms.texel_scale = vec2f(source.texel_scale[0], source.texel_scale[1]);
    uniforms
}

fn empty_uniforms(downsample_mode: DownsampleMode) -> BlurUniforms {
    let bounds = ShaderBounds {
        origin: vec2f(0.0, 0.0),
        size: vec2f(0.0, 0.0),
    };
    BlurUniforms {
        bounds,
        content_mask: bounds,
        corner_radii: ShaderCorners {
            top_left: 0.0,
            top_right: 0.0,
            bottom_right: 0.0,
            bottom_left: 0.0,
        },
        direction: vec2f(0.0, 0.0),
        opacity: 0.0,
        corner_smoothing: 0.0,
        source_size: vec2f(1.0, 1.0),
        sample_extent: vec2f(1.0, 1.0),
        texel_scale: vec2f(1.0, 1.0),
        composite_clip: ShaderBlurCompositeClip::None,
        downsample_mode,
        box_taps_per_axis: 1,
        tap_pair_count: 0,
        center_weight: 1.0,
        padding: 0,
        taps: [vec4f(0.0, 0.0, 0.0, 0.0); KERNEL_VECTORS],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STANDARD_DEVIATIONS: [f32; 12] = [
        0.25, 0.5, 0.75, 1.0, 3.2, 8.0, 11.0, 15.9, 16.0, 16.5, 40.0, 100.0,
    ];

    fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<ScaledPixels> {
        Bounds::new(
            gpui::point(ScaledPixels(x), ScaledPixels(y)),
            gpui::size(ScaledPixels(width), ScaledPixels(height)),
        )
    }

    fn fits_exactly(standard_deviation: f32) -> bool {
        GAUSSIAN_CUTOFF_STANDARD_DEVIATIONS * standard_deviation <= MAX_KERNEL_RADIUS as f32
    }

    /// The discrete kernel the merged pairs must reproduce, from `-radius` to `radius`.
    fn exact_weights(standard_deviation: f32) -> Vec<f32> {
        let radius = (GAUSSIAN_CUTOFF_STANDARD_DEVIATIONS * standard_deviation)
            .ceil()
            .max(1.0) as i32;
        let weights: Vec<f32> = (-radius..=radius)
            .map(|k| (-((k * k) as f32) / (2.0 * standard_deviation * standard_deviation)).exp())
            .collect();
        let total: f32 = weights.iter().sum();
        weights.into_iter().map(|weight| weight / total).collect()
    }

    /// Clamp-to-edge linear filtering of a row of texels, at a position in texel units.
    fn sample_linear(texels: &[f32], position: f32) -> f32 {
        let position = (position - 0.5).clamp(0.0, (texels.len() - 1) as f32);
        let index = position.floor() as usize;
        let next = (index + 1).min(texels.len() - 1);
        texels[index] + (texels[next] - texels[index]) * position.fract()
    }

    fn span(scissor: ScissorRectangle, axis: usize) -> std::ops::Range<i64> {
        let (start, length) = if axis == 0 {
            (scissor.x, scissor.width)
        } else {
            (scissor.y, scissor.height)
        };
        start as i64..(start + length) as i64
    }

    #[test]
    fn downsampled_dimensions_cover_odd_edges() {
        assert_eq!(downsampled_dimension(0), 1);
        assert_eq!(downsampled_dimension(1), 1);
        assert_eq!(downsampled_dimension(2), 1);
        assert_eq!(downsampled_dimension(3), 2);
        assert_eq!(downsampled_dimension(33), 17);
    }

    #[test]
    fn non_positive_deviations_do_not_blur() {
        assert!(BlurKernel::new(0.0).is_none());
        assert!(BlurKernel::new(-1.0).is_none());
        assert!(BlurKernel::new(f32::NAN).is_none());
        assert!(BlurPlan::new(0.0, bounds(0.0, 0.0, 10.0, 10.0), [10, 10]).is_none());
    }

    #[test]
    fn kernel_weights_sum_to_one() {
        for standard_deviation in STANDARD_DEVIATIONS {
            let kernel = BlurKernel::new(standard_deviation).unwrap();
            let total = kernel.center_weight
                + 2.0
                    * (0..kernel.pair_count as usize)
                        .map(|index| kernel.pair(index).1)
                        .sum::<f32>();
            assert!(
                (total - 1.0).abs() < 1e-5,
                "standard deviation {standard_deviation}: weights sum to {total}"
            );
        }
    }

    #[test]
    fn merged_taps_reproduce_the_discrete_kernel_at_edges() {
        let texels: Vec<f32> = (0..64)
            .map(|index| ((index * 73) % 256) as f32 / 255.0)
            .collect();
        for standard_deviation in STANDARD_DEVIATIONS {
            if !fits_exactly(standard_deviation) {
                continue;
            }
            let kernel = BlurKernel::new(standard_deviation).unwrap();
            let weights = exact_weights(standard_deviation);
            let radius = (weights.len() / 2) as i32;
            assert_eq!(kernel.pair_count, (radius as u32).div_ceil(2));
            for center in 0..texels.len() {
                let center = center as f32 + 0.5;
                let expected: f32 = (-radius..=radius)
                    .zip(&weights)
                    .map(|(offset, weight)| sample_linear(&texels, center + offset as f32) * weight)
                    .sum();
                let mut actual = sample_linear(&texels, center) * kernel.center_weight;
                for index in 0..kernel.pair_count as usize {
                    let (offset, weight) = kernel.pair(index);
                    actual += (sample_linear(&texels, center - offset)
                        + sample_linear(&texels, center + offset))
                        * weight;
                }
                assert!(
                    (expected - actual).abs() < 2e-6,
                    "standard deviation {standard_deviation}, center {center}: {expected} vs {actual}"
                );
            }
        }
    }

    #[test]
    fn taps_stay_ordered_and_within_reach() {
        for standard_deviation in STANDARD_DEVIATIONS {
            let kernel = BlurKernel::new(standard_deviation).unwrap();
            let mut previous = 0.0;
            for index in 0..kernel.pair_count as usize {
                let (offset, weight) = kernel.pair(index);
                assert!(offset > previous && weight >= 0.0);
                assert!((offset.ceil() as u32) < kernel.reach);
                previous = offset;
            }
            for index in kernel.pair_count as usize..MAX_TAP_PAIRS as usize {
                assert_eq!(kernel.pair(index), (0.0, 0.0));
            }
        }
    }

    #[test]
    fn oversized_kernels_spread_taps_over_the_full_support() {
        let standard_deviation = 40.0;
        assert!(!fits_exactly(standard_deviation));
        let kernel = BlurKernel::new(standard_deviation).unwrap();
        assert_eq!(kernel.pair_count, MAX_TAP_PAIRS);
        let (last_offset, _) = kernel.pair(MAX_TAP_PAIRS as usize - 1);
        assert_eq!(
            last_offset,
            (GAUSSIAN_CUTOFF_STANDARD_DEVIATIONS * standard_deviation).ceil()
        );
        // The spread taps still describe a Gaussian of (nearly) the requested width; the 3σ
        // cutoff alone narrows it by about 3%.
        let variance = 2.0
            * (0..kernel.pair_count as usize)
                .map(|index| {
                    let (offset, weight) = kernel.pair(index);
                    offset * offset * weight
                })
                .sum::<f32>();
        let expected = standard_deviation * standard_deviation;
        assert!((variance - expected).abs() / expected < 0.05, "{variance}");
    }

    #[test]
    fn downsample_factor_keeps_kernels_exact_and_resolution_fine() {
        assert_eq!(downsample_factor(0.5), 2);
        assert_eq!(downsample_factor(21.0), 2);
        assert_eq!(downsample_factor(31.9), 2);
        assert_eq!(downsample_factor(32.0), 4);
        assert_eq!(downsample_factor(64.0), 8);
        assert_eq!(downsample_factor(128.0), 16);
        assert_eq!(downsample_factor(10_000.0), MAX_DOWNSAMPLE_FACTOR);

        let mut previous = MIN_DOWNSAMPLE_FACTOR;
        for step in 1..=1200 {
            let standard_deviation = step as f32 * 0.25;
            let factor = downsample_factor(standard_deviation);
            assert!(factor >= previous, "factors must not shrink as blurs widen");
            previous = factor;
            let texels = standard_deviation / factor as f32;
            if factor > MIN_DOWNSAMPLE_FACTOR {
                assert!(texels >= MIN_TEXELS_PER_STANDARD_DEVIATION as f32);
            }
            if standard_deviation
                < (MAX_DOWNSAMPLE_FACTOR * 2 * MIN_TEXELS_PER_STANDARD_DEVIATION) as f32
            {
                assert!(fits_exactly(texels), "{standard_deviation}");
            }
        }
    }

    #[test]
    fn narrow_blurs_keep_the_half_resolution_kernel() {
        let plan = BlurPlan::new(4.0, bounds(10.0, 10.0, 20.0, 20.0), [100, 60]).unwrap();
        assert_eq!(plan.downsample_factor, 2);
        // A standard deviation of 2 texels reaches 6 taps per side, merged into 3 pairs.
        assert_eq!(plan.horizontal.uniforms.tap_pair_count, 3);
        assert_eq!(plan.downsample.uniforms.box_taps_per_axis, 1);
        assert_eq!(
            plan.result,
            FilterSource {
                texture_size: [50, 30],
                extent: [50, 30],
                texel_scale: [0.5, 0.5],
            }
        );
    }

    #[test]
    fn wide_blurs_use_part_of_the_half_resolution_scratch() {
        let plan = BlurPlan::new(40.0, bounds(0.0, 0.0, 1001.0, 500.0), [1001, 500]).unwrap();
        assert_eq!(plan.downsample_factor, 4);
        assert_eq!(plan.downsample.uniforms.box_taps_per_axis, 2);
        assert_eq!(plan.result.texture_size, [501, 250]);
        assert_eq!(plan.result.extent, [251, 125]);
        for pass in [plan.downsample, plan.horizontal, plan.vertical] {
            assert!(pass.scissor.x + pass.scissor.width <= 251);
            assert!(pass.scissor.y + pass.scissor.height <= 125);
        }
    }

    #[test]
    fn pass_scissors_cover_every_texel_a_later_pass_reads() {
        for target_size in [[1024, 256], [1279, 799], [17, 13]] {
            for blur_radius in [0.5, 4.0, 8.0, 21.5, 32.0, 100.0, 400.0] {
                for origin in [[0.0, 0.0], [31.25, 17.75], [-20.0, -10.0], [600.0, 200.0]] {
                    let bounds = bounds(origin[0], origin[1], 236.5, 48.5);
                    let Some(region) = visible_composite_region(bounds, bounds, target_size) else {
                        continue;
                    };
                    let plan = BlurPlan::new(blur_radius, region, target_size).unwrap();
                    let reach = BlurKernel::new(blur_radius / plan.downsample_factor as f32)
                        .unwrap()
                        .reach as i64;
                    let extent = plan.result.extent;
                    let inside = |scissor: ScissorRectangle, axis: usize, texel: i64| {
                        span(scissor, axis).contains(&texel.clamp(0, extent[axis] as i64 - 1))
                    };
                    let minimum = [region.origin.x.0, region.origin.y.0];
                    let size = [region.size.width.0, region.size.height.0];
                    for axis in 0..2 {
                        // Composite samples at pixel centers read two texels per axis.
                        let first = minimum[axis].floor() as i64;
                        let last = (minimum[axis] + size[axis]).ceil() as i64;
                        for pixel in first..last {
                            let texel = (pixel as f32 + 0.5) * plan.result.texel_scale[axis] - 0.5;
                            for texel in [texel.floor() as i64, texel.floor() as i64 + 1] {
                                assert!(inside(plan.vertical.scissor, axis, texel));
                            }
                        }
                        // Each Gaussian pass reads along its axis from the previous pass...
                        let (reader, written) = if axis == 0 {
                            (plan.horizontal.scissor, plan.downsample.scissor)
                        } else {
                            (plan.vertical.scissor, plan.horizontal.scissor)
                        };
                        for texel in span(reader, axis) {
                            for offset in -reach..=reach {
                                assert!(inside(written, axis, texel + offset));
                            }
                        }
                        // ...and straight across it.
                        let (reader, written) = if axis == 0 {
                            (plan.vertical.scissor, plan.horizontal.scissor)
                        } else {
                            (plan.horizontal.scissor, plan.downsample.scissor)
                        };
                        for texel in span(reader, axis) {
                            assert!(inside(written, axis, texel));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn invisible_regions_are_skipped() {
        let offscreen = bounds(120.0, 10.0, 20.0, 20.0);
        assert!(visible_composite_region(offscreen, offscreen, [100, 100]).is_none());
        let region = visible_composite_region(
            bounds(40.0, 40.0, 20.0, 20.0),
            bounds(0.0, 0.0, 50.0, 50.0),
            [100, 100],
        )
        .unwrap();
        assert_eq!(region, bounds(40.0, 40.0, 10.0, 10.0));
    }
}
