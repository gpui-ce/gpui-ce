use anyhow::{Context as _, Result, anyhow, ensure};
use core_foundation::{
    array::{CFArray, CFArrayRef},
    base::{CFIndex, CFType, TCFType},
    data::{CFData, CFDataRef},
    dictionary::CFDictionary,
    number::CFNumber,
    string::CFString,
};
use core_graphics::{
    base::{CGFloat, kCGImageAlphaPremultipliedLast},
    color_space::CGColorSpace,
    context::{CGContext, CGTextDrawingMode},
    display::CGPoint,
    geometry::CGAffineTransform,
};
use core_text::{
    font,
    font_descriptor::{self, CTFontDescriptor, kCTFontOrientationDefault},
};
use gpui::{
    Bounds, DevicePixels, GlyphRenderMode, PreparedRasterStyle, RasterColorEffect,
    RasterStyleRequest, RasterizedGlyph, RasterizedGlyphFormat, RenderGlyphParams, Rgba8,
    SUBPIXEL_VARIANTS_X, SUBPIXEL_VARIANTS_Y, TextRenderingMode, point, size,
};
use gpui_parley::{GlyphRasterizer, RasterFace};
use objc2::rc::autoreleasepool;
use std::{collections::HashMap, f64::consts::PI, sync::OnceLock};

#[allow(non_upper_case_globals)]
const kCGImageAlphaOnly: u32 = 7;

/// CoreText and CoreGraphics rasterization for the exact face selected by Parley.
pub(crate) struct MacGlyphRasterizer {
    faces: HashMap<gpui::FontId, NativeFace>,
}

struct NativeFace {
    descriptor: CTFontDescriptor,
    // CoreText may defer reading tables from descriptors created from in-memory data until a
    // sized CTFont first draws. Keep the descriptor's source alive for the full cached-face
    // lifetime, as the pre-Parley backend did through its retained CGFont.
    _source_data: SendCFData,
}

/// An immutable Core Foundation data object retained by the serialized macOS rasterizer.
struct SendCFData {
    _data: CFData,
}

// SAFETY: CFData is immutable, and MacGlyphRasterizer only accesses native faces while its
// enclosing mutex is held. The value is retained solely to extend the source data's lifetime.
unsafe impl Send for SendCFData {}

impl MacGlyphRasterizer {
    pub(crate) fn new() -> Self {
        Self {
            faces: HashMap::default(),
        }
    }

    fn native_face(&mut self, face: &RasterFace<'_>) -> Result<&NativeFace> {
        match self.faces.entry(face.font_id) {
            std::collections::hash_map::Entry::Occupied(entry) => Ok(entry.into_mut()),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let native = autoreleasepool(|_| NativeFace::new(face)).with_context(|| {
                    format!(
                        "CoreText could not create FontId {:?}, face index {}, variations {:?}",
                        face.font_id, face.face_index, face.variations
                    )
                })?;
                Ok(entry.insert(native))
            }
        }
    }

    fn rasterize_inner(
        &mut self,
        face: &RasterFace<'_>,
        params: &RenderGlyphParams,
    ) -> Result<RasterizedGlyph> {
        let native = self.native_face(face)?;
        let font_size = f64::from(params.font_size);
        let scale_factor = f64::from(params.scale_factor);
        ensure!(
            font_size.is_finite() && font_size >= 0.0,
            "invalid font size"
        );
        ensure!(
            scale_factor.is_finite() && scale_factor > 0.0,
            "invalid raster scale factor"
        );
        let font = font::new_from_descriptor(&native.descriptor, font_size);
        let glyph: u16 = params
            .glyph_id
            .0
            .try_into()
            .context("CoreText glyph IDs are 16-bit")?;

        let skew = face
            .synthesis
            .skew_degrees
            .map_or(0.0, |degrees| f64::from(degrees) * PI / 180.0)
            .tan();
        let text_matrix = CGAffineTransform::new(1.0, 0.0, skew, 1.0, 0.0, 0.0);
        let glyph_rect = font
            .get_bounding_rects_for_glyphs(kCTFontOrientationDefault, &[glyph])
            .apply_transform(&text_matrix);
        if glyph_rect.is_empty() || glyph_rect.size.width <= 0.0 || glyph_rect.size.height <= 0.0 {
            return Ok(RasterizedGlyph::empty(format_for_mode(
                params.raster_style.mode,
            )));
        }

        let embolden = if face.synthesis.embolden {
            font_size / 48.0
        } else {
            0.0
        };
        let padding = (embolden * scale_factor).ceil() + 1.0;
        let left = (glyph_rect.origin.x * scale_factor - padding).floor();
        let mut right =
            ((glyph_rect.origin.x + glyph_rect.size.width) * scale_factor + padding).ceil();
        let top =
            (-(glyph_rect.origin.y + glyph_rect.size.height) * scale_factor - padding).floor();
        let bottom = (-glyph_rect.origin.y * scale_factor + padding).ceil();
        if params.subpixel_variant.x > 0 {
            right += 1.0;
        }
        let width = (right - left) as i32;
        let height = (bottom - top) as i32;
        if width <= 0 || height <= 0 {
            return Ok(RasterizedGlyph::empty(format_for_mode(
                params.raster_style.mode,
            )));
        }

        let format = format_for_mode(params.raster_style.mode);
        let bytes_per_pixel = if format == RasterizedGlyphFormat::BgraColor {
            4
        } else {
            1
        };
        let mut pixels = vec![0; width as usize * height as usize * bytes_per_pixel];
        {
            let color_space = if bytes_per_pixel == 4 {
                CGColorSpace::create_device_rgb()
            } else {
                CGColorSpace::create_device_gray()
            };
            let context = CGContext::create_bitmap_context(
                Some(pixels.as_mut_ptr().cast()),
                width as usize,
                height as usize,
                8,
                width as usize * bytes_per_pixel,
                &color_space,
                if bytes_per_pixel == 4 {
                    kCGImageAlphaPremultipliedLast
                } else {
                    kCGImageAlphaOnly
                },
            );
            configure_context(
                &context,
                params.raster_style,
                face.synthesis.embolden,
                embolden,
                text_matrix,
            );
            context.translate(-left, top + f64::from(height));
            context.scale(scale_factor, scale_factor);
            let offset = CGPoint::new(
                f64::from(params.subpixel_variant.x)
                    / f64::from(SUBPIXEL_VARIANTS_X)
                    / scale_factor,
                f64::from(params.subpixel_variant.y)
                    / f64::from(SUBPIXEL_VARIANTS_Y)
                    / scale_factor,
            );
            font.draw_glyphs(&[glyph], &[offset], context);
        }

        if format == RasterizedGlyphFormat::BgraColor {
            for pixel in pixels.chunks_exact_mut(4) {
                gpui::swap_rgba_pa_to_bgra(pixel);
            }
        }

        Ok(RasterizedGlyph {
            bounds: Bounds {
                origin: point(DevicePixels(left as i32), DevicePixels(top as i32)),
                size: size(DevicePixels(width), DevicePixels(height)),
            },
            size: size(DevicePixels(width), DevicePixels(height)),
            format,
            pixels,
        })
    }
}

impl GlyphRasterizer for MacGlyphRasterizer {
    fn prepare_style(&self, request: RasterStyleRequest) -> PreparedRasterStyle {
        if request.requested_mode == GlyphRenderMode::Color {
            return PreparedRasterStyle {
                mode: GlyphRenderMode::Color,
                color_effect: RasterColorEffect::Preblend(request.scene_color.into()),
            };
        }

        let color_effect = if font_smoothing_allowed_by_user() {
            let color = request.scene_color;
            let luminance = 0.2126 * color.red + 0.7152 * color.green + 0.0722 * color.blue;
            let dilation = ((4.0 * luminance) + 0.5).floor().clamp(0.0, 4.0) as u8;
            RasterColorEffect::Dilation(dilation)
        } else {
            RasterColorEffect::Dilation(0)
        };
        PreparedRasterStyle {
            mode: GlyphRenderMode::Grayscale,
            color_effect,
        }
    }

    fn rasterize(
        &mut self,
        face: RasterFace<'_>,
        params: &RenderGlyphParams,
    ) -> Result<RasterizedGlyph> {
        autoreleasepool(|_| self.rasterize_inner(&face, params))
    }

    fn recommended_mode(&self) -> TextRenderingMode {
        TextRenderingMode::Grayscale
    }
}

impl NativeFace {
    fn new(face: &RasterFace<'_>) -> Result<Self> {
        let data = CFData::from_buffer(face.data);
        let descriptors_ref =
            unsafe { CTFontManagerCreateFontDescriptorsFromData(data.as_concrete_TypeRef()) };
        ensure!(
            !descriptors_ref.is_null(),
            "CoreText rejected the supplied font bytes"
        );
        let descriptors: CFArray<CTFontDescriptor> =
            unsafe { CFArray::wrap_under_create_rule(descriptors_ref) };
        let descriptor = descriptors.get(face.face_index as CFIndex).ok_or_else(|| {
            anyhow!(
                "collection contains {} faces, requested {}",
                descriptors.len(),
                face.face_index
            )
        })?;
        let mut descriptor =
            unsafe { CTFontDescriptor::wrap_under_get_rule(descriptor.as_concrete_TypeRef()) };

        if !face.variations.is_empty() {
            let variations = face
                .variations
                .iter()
                .map(|variation| {
                    let tag = u32::from_be_bytes(variation.tag.to_be_bytes());
                    (
                        CFNumber::from(i64::from(tag)),
                        CFNumber::from(f64::from(variation.value)),
                    )
                })
                .collect::<Vec<_>>();
            let variations = CFDictionary::from_CFType_pairs(&variations);
            let variation_key = unsafe {
                CFString::wrap_under_get_rule(font_descriptor::kCTFontVariationAttribute)
            };
            let variation_value = unsafe { CFType::wrap_under_get_rule(variations.as_CFTypeRef()) };
            let attributes =
                CFDictionary::from_CFType_pairs(&[(variation_key, variation_value)]).into_untyped();
            descriptor = descriptor
                .create_copy_with_attributes(attributes)
                .map_err(|()| anyhow!("CoreText rejected the variation coordinates"))?;
        }

        Ok(Self {
            descriptor,
            _source_data: SendCFData { _data: data },
        })
    }
}

fn configure_context(
    context: &CGContext,
    style: PreparedRasterStyle,
    embolden: bool,
    embolden_amount: CGFloat,
    text_matrix: CGAffineTransform,
) {
    context.set_text_drawing_mode(if embolden {
        CGTextDrawingMode::CGTextFillStroke
    } else {
        CGTextDrawingMode::CGTextFill
    });
    context.set_text_matrix(&text_matrix);
    context.set_allows_antialiasing(true);
    context.set_should_antialias(true);
    context.set_allows_font_subpixel_positioning(true);
    context.set_should_subpixel_position_fonts(true);
    context.set_allows_font_subpixel_quantization(false);
    context.set_should_subpixel_quantize_fonts(false);
    context.set_line_width(embolden_amount * 2.0);

    match style.color_effect {
        RasterColorEffect::Dilation(level) => {
            let luminance = f64::from(level) * 0.25;
            context.set_should_smooth_fonts(level > 0);
            context.set_gray_fill_color(luminance, 1.0);
            context.set_rgb_stroke_color(luminance, luminance, luminance, 1.0);
        }
        RasterColorEffect::Preblend(Rgba8 {
            red,
            green,
            blue,
            alpha,
        }) => {
            let [red, green, blue, alpha] = [red, green, blue, alpha].map(|c| f64::from(c) / 255.0);
            context.set_rgb_fill_color(red, green, blue, alpha);
            context.set_rgb_stroke_color(red, green, blue, alpha);
        }
        RasterColorEffect::Independent => {
            context.set_gray_fill_color(0.0, 1.0);
            context.set_rgb_stroke_color(0.0, 0.0, 0.0, 1.0);
        }
    }
}

fn format_for_mode(mode: GlyphRenderMode) -> RasterizedGlyphFormat {
    match mode {
        GlyphRenderMode::Color => RasterizedGlyphFormat::BgraColor,
        GlyphRenderMode::Grayscale | GlyphRenderMode::Subpixel => RasterizedGlyphFormat::AlphaMask,
    }
}

fn font_smoothing_allowed_by_user() -> bool {
    static ALLOWED: OnceLock<bool> = OnceLock::new();
    *ALLOWED.get_or_init(|| {
        use core_foundation_sys::preferences::{
            CFPreferencesCopyAppValue, kCFPreferencesCurrentApplication,
        };

        let key = CFString::new("AppleFontSmoothing");
        let value_ref = unsafe {
            CFPreferencesCopyAppValue(key.as_concrete_TypeRef(), kCFPreferencesCurrentApplication)
        };
        if value_ref.is_null() {
            return true;
        }
        let value = unsafe { CFType::wrap_under_create_rule(value_ref) };
        value
            .downcast_into::<CFNumber>()
            .and_then(|number| number.to_i64())
            != Some(0)
    })
}

#[link(name = "CoreText", kind = "framework")]
unsafe extern "C" {
    fn CTFontManagerCreateFontDescriptorsFromData(data: CFDataRef) -> CFArrayRef;
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        GlyphId, GlyphRenderMode, PlatformTextSystem, RasterColorEffect, RasterizedGlyphFormat,
        font, point, px, rgba,
    };
    use gpui_parley::{ParleyTextSystem, SystemFonts};
    use std::borrow::Cow;

    const SOURCE_SERIF: &[u8] =
        include_bytes!("../../../assets/fonts/source-serif-4/SourceSerif4[opsz,wght].ttf");

    #[test]
    fn in_memory_variable_font_renders_stably_across_glyphs_and_sizes() {
        let system = ParleyTextSystem::new_with_rasterizer(
            SystemFonts::Skip,
            "Source Serif 4",
            MacGlyphRasterizer::new(),
        );
        system.add_fonts(vec![Cow::Borrowed(SOURCE_SERIF)]).unwrap();
        let font_id = system.font_id(&font("Source Serif 4")).unwrap();
        let render_pass = || {
            "Ag&"
                .chars()
                .enumerate()
                .map(|(index, character)| {
                    let step = index as u8;
                    let glyph = system
                        .rasterize_glyph(&RenderGlyphParams {
                            font_id,
                            glyph_id: system.glyph_for_char(font_id, character).unwrap(),
                            font_size: px(12.0 * f32::from(step + 1)),
                            subpixel_variant: point(step, step),
                            scale_factor: 1.0 + f32::from(step) * 0.5,
                            raster_style: PreparedRasterStyle {
                                mode: GlyphRenderMode::Grayscale,
                                color_effect: RasterColorEffect::Dilation(step * 2),
                            },
                        })
                        .unwrap();
                    glyph.validate().unwrap();
                    assert!(
                        glyph.pixels.iter().any(|&coverage| coverage != 0),
                        "'{character}' produced an empty coverage mask"
                    );
                    glyph
                })
                .collect::<Vec<_>>()
        };

        let first_pass = render_pass();
        let second_pass = render_pass();
        for (character, (expected, actual)) in
            "Ag&".chars().zip(first_pass.iter().zip(&second_pass))
        {
            assert_eq!(
                actual.bounds, expected.bounds,
                "bounds changed for '{character}'"
            );
            assert_eq!(
                actual.pixels, expected.pixels,
                "pixels changed for '{character}'"
            );
        }
    }

    #[test]
    fn core_text_obeys_platform_style_mask_color_baseline_and_empty_glyph_behavior() {
        let system = ParleyTextSystem::new_with_rasterizer(
            SystemFonts::Skip,
            "Source Serif 4",
            MacGlyphRasterizer::new(),
        );
        system.add_fonts(vec![Cow::Borrowed(SOURCE_SERIF)]).unwrap();
        let font_id = system
            .font_id(&font("Source Serif 4").bold().italic())
            .unwrap();

        let render_style = |glyph_id: GlyphId, raster_style, variant| {
            system
                .rasterize_glyph(&RenderGlyphParams {
                    font_id,
                    glyph_id,
                    font_size: px(24.0),
                    subpixel_variant: variant,
                    scale_factor: 2.0,
                    raster_style,
                })
                .unwrap()
        };
        let render = |glyph_id: GlyphId, mode, color, variant| {
            render_style(
                glyph_id,
                system.prepare_raster_style(RasterStyleRequest {
                    scene_color: color,
                    requested_mode: mode,
                }),
                variant,
            )
        };

        let letter = system.glyph_for_char(font_id, 'A').unwrap();
        let normalized_subpixel = system.prepare_raster_style(RasterStyleRequest {
            scene_color: rgba(0x303030ff),
            requested_mode: GlyphRenderMode::Subpixel,
        });
        assert_eq!(normalized_subpixel.mode, GlyphRenderMode::Grayscale);

        let light_style = system.prepare_raster_style(RasterStyleRequest {
            scene_color: rgba(0xffffffff),
            requested_mode: GlyphRenderMode::Grayscale,
        });
        assert_eq!(
            light_style.color_effect,
            RasterColorEffect::Dilation(if font_smoothing_allowed_by_user() {
                4
            } else {
                0
            })
        );

        let undilated = render_style(
            letter,
            PreparedRasterStyle {
                mode: GlyphRenderMode::Grayscale,
                color_effect: RasterColorEffect::Dilation(0),
            },
            point(0, 0),
        );
        let dilated = render_style(
            letter,
            PreparedRasterStyle {
                mode: GlyphRenderMode::Grayscale,
                color_effect: RasterColorEffect::Dilation(4),
            },
            point(0, 0),
        );
        assert_ne!(undilated.pixels, dilated.pixels);

        let shifted = render_style(
            letter,
            PreparedRasterStyle {
                mode: GlyphRenderMode::Grayscale,
                color_effect: RasterColorEffect::Dilation(0),
            },
            point(SUBPIXEL_VARIANTS_X - 1, 0),
        );
        assert_eq!(shifted.bounds.origin, undilated.bounds.origin);
        assert_eq!(shifted.size.height, undilated.size.height);
        assert_eq!(shifted.size.width.0, undilated.size.width.0 + 1);

        let mask = render(
            letter,
            GlyphRenderMode::Grayscale,
            rgba(0x303030ff),
            point(3, 0),
        );
        assert_eq!(mask.format, RasterizedGlyphFormat::AlphaMask);
        assert_eq!(mask.bounds.size, mask.size);
        assert!(mask.bounds.origin.y.0 < 0);
        assert!(mask.size.width.0 > 0 && mask.size.height.0 > 0);
        mask.validate().unwrap();

        let color = render(
            letter,
            GlyphRenderMode::Color,
            rgba(0xe02010ff),
            point(1, 0),
        );
        assert_eq!(color.format, RasterizedGlyphFormat::BgraColor);
        color.validate().unwrap();
        let colored_pixel = color
            .pixels
            .chunks_exact(4)
            .find(|pixel| pixel[3] > 128)
            .expect("colored glyph pixel");
        assert!(colored_pixel[2] > colored_pixel[0], "{colored_pixel:?}");

        let space = system.glyph_for_char(font_id, ' ').unwrap();
        let empty = render(
            space,
            GlyphRenderMode::Grayscale,
            rgba(0x000000ff),
            point(0, 0),
        );
        assert_eq!(empty.size, gpui::Size::default());
        assert!(empty.pixels.is_empty());

        let emoji_system = ParleyTextSystem::new_with_rasterizer(
            SystemFonts::Load,
            ".AppleSystemUIFont",
            MacGlyphRasterizer::new(),
        );
        let emoji_font = emoji_system
            .font_id(&font("Apple Color Emoji"))
            .expect("Apple Color Emoji is available on macOS");
        let emoji = emoji_system
            .rasterize_glyph(&RenderGlyphParams {
                font_id: emoji_font,
                glyph_id: emoji_system.glyph_for_char(emoji_font, '😀').unwrap(),
                font_size: px(24.0),
                subpixel_variant: point(2, 0),
                scale_factor: 2.0,
                raster_style: emoji_system.prepare_raster_style(RasterStyleRequest {
                    scene_color: rgba(0xffffffff),
                    requested_mode: GlyphRenderMode::Color,
                }),
            })
            .unwrap();
        assert_eq!(emoji.format, RasterizedGlyphFormat::BgraColor);
        emoji.validate().unwrap();
        assert!(emoji.pixels.chunks_exact(4).any(|pixel| {
            pixel[3] > 128
                && (pixel[0].abs_diff(pixel[1]) > 20
                    || pixel[1].abs_diff(pixel[2]) > 20
                    || pixel[0].abs_diff(pixel[2]) > 20)
        }));
    }
}
