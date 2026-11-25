use std::{
    ffi::c_void,
    ptr::{null, null_mut},
    slice::from_raw_parts,
};

use gpui::{RasterizedGlyph, RenderGlyphParams, Result, SUBPIXEL_VARIANTS_X, SUBPIXEL_VARIANTS_Y};
use windows::Win32::{
    Foundation::RECT,
    Graphics::{
        Direct2D::{
            Common::{D2D_RECT_F, D2D_SIZE_U},
            D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
        },
        DirectWrite::{
            DWRITE_GLYPH_IMAGE_DATA, DWRITE_GLYPH_IMAGE_FORMATS, DWRITE_GLYPH_IMAGE_FORMATS_PNG,
            DWRITE_GLYPH_IMAGE_FORMATS_PREMULTIPLIED_B8G8R8A8, IDWriteFontFace4,
        },
        Imaging::{
            CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmap, IWICImagingFactory,
            WICBitmapCacheOnLoad, WICConvertBitmapSource, WICDecodeMetadataCacheOnLoad,
        },
    },
    System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
};

use crate::direct_write::{
    ColorRasterApartment, DirectWriteGlyphRasterizer, DirectWriteRasterError,
    color_bitmap_byte_count, rasterize_color_bitmap, validate_raster_scale_factor,
};

impl DirectWriteGlyphRasterizer {
    pub(super) fn rasterize_bitmap_glyph(
        &self,
        params: &RenderGlyphParams,
    ) -> Result<RasterizedGlyph> {
        self.with_bitmap_glyph(params, |wic, data, bitmap| {
            let destination = bitmap_glyph_rect(params, data)?;
            let ink_bounds = bitmap_ink_bounds(destination)?;

            draw_bitmap_glyph(wic, params, bitmap, destination, ink_bounds)
        })
    }

    pub(super) fn with_bitmap_glyph(
        &self,
        params: &RenderGlyphParams,
        draw: impl FnOnce(
            &IWICImagingFactory,
            &DWRITE_GLYPH_IMAGE_DATA,
            &IWICBitmap,
        ) -> Result<RasterizedGlyph>,
    ) -> Result<RasterizedGlyph> {
        let _apartment = ColorRasterApartment::new()?;
        let face = self.font_face(params.font_id);

        let glyph_id = u16::try_from(params.glyph_id.0).map_err(|_| {
            DirectWriteRasterError::InvalidGlyphId {
                glyph_id: params.glyph_id.0,
                glyph_count: unsafe { face.GetGlyphCount() },
            }
        })?;

        let font_size = params.font_size.as_f32();

        if !font_size.is_finite() || font_size <= 0.0 {
            return Err(DirectWriteRasterError::InvalidFontSize(font_size).into());
        }

        validate_raster_scale_factor(params.scale_factor)?;

        let pixels_per_em = font_size as f64 * params.scale_factor as f64;

        if !pixels_per_em.is_finite() || pixels_per_em <= 0.0 || pixels_per_em > u32::MAX as f64 {
            return Err(DirectWriteRasterError::InvalidBitmapPixelsPerEm(pixels_per_em).into());
        }

        let (image, image_format) =
            load_bitmap_image(face, glyph_id, pixels_per_em.round().max(1.0) as u32)?;

        let wic: IWICImagingFactory =
            unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? };

        let bitmap = decode_bitmap_glyph(&wic, &image.data, image.bytes()?, image_format)?;

        draw(&wic, &image.data, &bitmap)
    }
}

fn load_bitmap_image(
    face: &IDWriteFontFace4,
    glyph_id: u16,
    pixels_per_em: u32,
) -> Result<(GlyphImageData<'_>, DWRITE_GLYPH_IMAGE_FORMATS), DirectWriteRasterError> {
    let glyph_count = unsafe { face.GetGlyphCount() };

    if glyph_id >= glyph_count {
        return Err(DirectWriteRasterError::InvalidGlyphId {
            glyph_id: u32::from(glyph_id),
            glyph_count,
        });
    }

    if pixels_per_em == 0 {
        return Err(DirectWriteRasterError::InvalidBitmapPixelsPerEm(0.0));
    }

    // Search every strike. A bitmap need not exist at the requested pixels per em.
    let formats = unsafe { face.GetGlyphImageFormats(glyph_id, 0, u32::MAX) }
        .map_err(DirectWriteRasterError::QueryBitmapFormats)?;

    for image_format in [
        DWRITE_GLYPH_IMAGE_FORMATS_PNG,
        DWRITE_GLYPH_IMAGE_FORMATS_PREMULTIPLIED_B8G8R8A8,
    ] {
        if (formats & image_format).0 == 0 {
            continue;
        }

        let Some(image) = GlyphImageData::new(face, glyph_id, pixels_per_em, image_format)? else {
            continue;
        };

        return Ok((image, image_format));
    }

    Err(DirectWriteRasterError::MissingBitmapImage {
        glyph_id,
        pixels_per_em,
    })
}

fn decode_bitmap_glyph(
    wic: &IWICImagingFactory,
    data: &DWRITE_GLYPH_IMAGE_DATA,
    bytes: &[u8],
    image_format: DWRITE_GLYPH_IMAGE_FORMATS,
) -> Result<IWICBitmap, DirectWriteRasterError> {
    let width = data.pixelSize.width;
    let height = data.pixelSize.height;

    if width.max(height) > i32::MAX as u32 {
        return Err(DirectWriteRasterError::BitmapDimensionsOverflow(
            data.pixelSize,
        ));
    }

    let byte_count = color_bitmap_byte_count(width as i32, height as i32)?;

    if image_format == DWRITE_GLYPH_IMAGE_FORMATS_PREMULTIPLIED_B8G8R8A8 {
        if bytes.len() != byte_count {
            return Err(DirectWriteRasterError::InvalidBgraBufferSize {
                size: data.pixelSize,
                actual: bytes.len(),
                expected: byte_count,
            });
        }

        return Ok(unsafe {
            wic.CreateBitmapFromMemory(
                width,
                height,
                &GUID_WICPixelFormat32bppPBGRA,
                width * 4,
                bytes,
            )?
        });
    }

    if image_format != DWRITE_GLYPH_IMAGE_FORMATS_PNG {
        return Err(DirectWriteRasterError::UnsupportedBitmapFormat(
            image_format.0,
        ));
    }

    if u32::try_from(bytes.len()).is_err() {
        return Err(DirectWriteRasterError::PngBufferTooLarge(bytes.len()));
    }

    let stream = unsafe { wic.CreateStream()? };
    unsafe { stream.InitializeFromMemory(bytes)? };

    let decoder =
        unsafe { wic.CreateDecoderFromStream(&stream, null(), WICDecodeMetadataCacheOnLoad) }
            .map_err(DirectWriteRasterError::DecodePng)?;

    let frame = unsafe { decoder.GetFrame(0)? };
    let mut decoded = D2D_SIZE_U::default();
    unsafe { frame.GetSize(&mut decoded.width, &mut decoded.height)? };

    if decoded != data.pixelSize {
        return Err(DirectWriteRasterError::PngDimensionsMismatch {
            decoded,
            expected: data.pixelSize,
        });
    }

    let converter = unsafe { WICConvertBitmapSource(&GUID_WICPixelFormat32bppPBGRA, &frame)? };

    // Materialize the pixels before the stream's borrowed input can expire.
    Ok(unsafe { wic.CreateBitmapFromSource(&converter, WICBitmapCacheOnLoad)? })
}

pub(super) fn bitmap_glyph_rect(
    params: &RenderGlyphParams,
    data: &DWRITE_GLYPH_IMAGE_DATA,
) -> Result<D2D_RECT_F, DirectWriteRasterError> {
    let strike_pixels_per_em = data.pixelsPerEm;

    if strike_pixels_per_em == 0 {
        return Err(DirectWriteRasterError::MissingBitmapStrikeSize(
            params.glyph_id.0,
        ));
    }

    let font_size = params.font_size.as_f32();
    let scale_factor = params.scale_factor;
    let scale = f64::from(font_size) * f64::from(scale_factor) / f64::from(strike_pixels_per_em);

    if !scale.is_finite() || scale <= 0.0 {
        return Err(DirectWriteRasterError::InvalidBitmapScale {
            scale,
            font_size,
            scale_factor,
            strike_pixels_per_em,
        });
    }

    let left = params.subpixel_variant.x as f64 / SUBPIXEL_VARIANTS_X as f64
        - data.horizontalLeftOrigin.x as f64 * scale;

    let top = params.subpixel_variant.y as f64 / SUBPIXEL_VARIANTS_Y as f64
        - data.horizontalLeftOrigin.y as f64 * scale;

    Ok(D2D_RECT_F {
        left: left as f32,
        top: top as f32,
        right: (left + data.pixelSize.width as f64 * scale) as f32,
        bottom: (top + data.pixelSize.height as f64 * scale) as f32,
    })
}

fn bitmap_ink_bounds(rect: D2D_RECT_F) -> Result<RECT, DirectWriteRasterError> {
    let edges = [
        rect.left.floor(),
        rect.top.floor(),
        rect.right.ceil(),
        rect.bottom.ceil(),
    ];

    let representable = f64::from(i32::MIN)..=f64::from(i32::MAX);

    for (edge_name, edge) in ["left", "top", "right", "bottom"].into_iter().zip(edges) {
        let edge = f64::from(edge);

        if !representable.contains(&edge) {
            return Err(DirectWriteRasterError::BitmapBoundsOverflow { edge_name, edge });
        }
    }

    Ok(RECT {
        left: edges[0] as i32,
        top: edges[1] as i32,
        right: edges[2] as i32,
        bottom: edges[3] as i32,
    })
}

pub(super) fn draw_bitmap_glyph(
    wic: &IWICImagingFactory,
    params: &RenderGlyphParams,
    bitmap: &IWICBitmap,
    destination: D2D_RECT_F,
    ink_bounds: RECT,
) -> Result<RasterizedGlyph> {
    rasterize_color_bitmap(wic, params.raster_style, ink_bounds, |target| {
        let image = unsafe { target.CreateBitmapFromWicBitmap(bitmap, None)? };

        unsafe {
            target.DrawBitmap(
                &image,
                Some(&destination),
                1.0,
                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                None,
            );
        }

        Ok(())
    })
}

struct GlyphImageData<'a> {
    face: &'a IDWriteFontFace4,
    data: DWRITE_GLYPH_IMAGE_DATA,
    context: Option<*mut c_void>,
}

impl<'a> GlyphImageData<'a> {
    fn new(
        face: &'a IDWriteFontFace4,
        glyph_id: u16,
        pixels_per_em: u32,
        image_format: DWRITE_GLYPH_IMAGE_FORMATS,
    ) -> Result<Option<Self>, DirectWriteRasterError> {
        let mut data = DWRITE_GLYPH_IMAGE_DATA::default();
        let mut context = null_mut();

        unsafe {
            face.GetGlyphImageData(
                glyph_id,
                pixels_per_em,
                image_format,
                &mut data,
                &mut context,
            )
        }
        .map_err(DirectWriteRasterError::GetBitmapImageData)?;

        // Empty results have no mapping. Retain ownership independently of the image metadata.
        let image = Self {
            face,
            data,
            context: (data.imageDataSize != 0).then_some(context),
        };

        if image.data.imageDataSize == 0 {
            return Ok(None);
        }

        Ok(Some(image))
    }

    fn bytes(&self) -> Result<&[u8], DirectWriteRasterError> {
        let byte_count = self.data.imageDataSize;

        if byte_count == 0 || u64::from(byte_count) > isize::MAX as u64 {
            return Err(DirectWriteRasterError::InvalidBitmapBufferSize(byte_count));
        }

        if self.data.imageData.is_null() {
            return Err(DirectWriteRasterError::NullBitmapBuffer(byte_count));
        }

        // The guard borrows the face and releases the mapping only after this slice expires.
        Ok(unsafe { from_raw_parts(self.data.imageData.cast(), byte_count as usize) })
    }
}

impl Drop for GlyphImageData<'_> {
    fn drop(&mut self) {
        if let Some(context) = self.context {
            unsafe { self.face.ReleaseGlyphImageData(context) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context as _;
    use gpui::{
        ForegroundDependency, GlyphId, GlyphRenderMode::Color, Pixels, TextLayoutRequest, TextRun,
        font, px, rgba,
    };
    use std::fmt::{Debug, Display};
    use windows::Win32::{
        Foundation::POINT, Graphics::DirectWrite::DWRITE_GLYPH_IMAGE_FORMATS_SVG,
    };

    use crate::direct_write::{
        new_text_system, read_color_bitmap,
        test_support::{
            bitmap_test_params, oversized_color_bounds, raster_params, test_rasterizer,
        },
    };

    fn assert_error(result: Result<impl Debug, impl Display>, expected: impl Display) {
        assert_eq!(result.unwrap_err().to_string(), expected.to_string());
    }

    #[test]
    fn cbdt_preserves_sequences_and_empty_or_invalid_glyphs() -> Result<()> {
        let system = new_text_system()?;
        let mut params = bitmap_test_params(&*system)?;
        system.rasterize_glyph(&params)?;

        let rasterizer = test_rasterizer(params.font_id, gpui_fonts::NOTO_COLOR_EMOJI.data, 0)?;
        let rasterize_native =
            |params: &RenderGlyphParams| rasterizer.rasterize_bitmap_glyph(params);

        for text in ["👩‍💻", "👩🏽", "🇬🇧", "1️⃣"] {
            let runs = [TextRun {
                len: text.len(),
                font: font("Noto Color Emoji"),
                ..Default::default()
            }];

            let layout = system.layout_text(TextLayoutRequest {
                text,
                font_size: px(24.0),
                runs: &runs,
                options: Default::default(),
            });

            let fragments = &layout.paint_fragments;
            assert_eq!(fragments.len(), 1, "{text}");
            assert_eq!(fragments[0].font_id, params.font_id, "{text}");
            assert_eq!(fragments[0].glyphs.len(), 1, "{text}");
            let glyph_id = fragments[0].glyphs[0].id;
            params = raster_params(&*system, params.font_id, glyph_id, Color, rgba(0xffffffff));

            assert_eq!(
                params.raster_style.foreground_dependency,
                ForegroundDependency::AlphaOnly
            );

            let actual = system.rasterize_glyph(&params)?;
            actual.validate()?;

            assert!(
                actual.pixels.chunks_exact(4).any(|pixel| {
                    pixel[3] > 0 && (pixel[0] != pixel[1] || pixel[1] != pixel[2])
                }),
                "{text}"
            );

            let native = rasterize_native(&params)?;

            assert_eq!(actual.bounds, native.bounds, "{text}");
            assert_eq!(actual.format, native.format, "{text}");
            assert_eq!(actual.pixels, native.pixels, "{text}");
        }

        params.glyph_id = system.glyph_for_char(params.font_id, ' ').unwrap();

        let empty = system.rasterize_glyph(&params)?;
        empty.validate()?;
        assert!(empty.pixels.is_empty());

        let glyph_id = params.glyph_id.0;

        assert_error(
            rasterize_native(&params),
            format_args!(
                "No PNG or premultiplied BGRA for CBDT glyph {glyph_id} at 24 requested pixels per em."
            ),
        );

        for invalid in [u16::MAX as u32, u16::MAX as u32 + 1, u32::MAX] {
            params.glyph_id = GlyphId(invalid);
            assert!(system.rasterize_glyph(&params).is_err());

            assert!(rasterize_native(&params).is_err());
        }

        params.glyph_id = system.glyph_for_char(params.font_id, '😀').unwrap();
        params.font_size = Pixels::ZERO;
        assert!(system.rasterize_glyph(&params)?.pixels.is_empty());

        for invalid in [f32::NAN, f32::INFINITY, -1.0, f32::MAX] {
            params.font_size = px(invalid);

            assert!(system.rasterize_glyph(&params).is_err());
            assert!(rasterize_native(&params).is_err());
        }

        params.font_size = px(24.0);

        for invalid in [f32::NAN, f32::INFINITY, -1.0, 0.0] {
            params.scale_factor = invalid;
            let native = rasterize_native(&params).unwrap_err();
            let error = system.rasterize_glyph(&params).unwrap_err();

            assert_eq!(error.root_cause().to_string(), native.to_string());

            assert_eq!(
                native.to_string(),
                format!(
                    "Invalid raster scale factor {invalid}. The scale factor must be finite and greater than zero."
                )
            );
        }

        params.font_size = px(65536.0);
        params.scale_factor = 65536.0;
        let error = system.rasterize_glyph(&params).unwrap_err();

        assert_eq!(
            error.root_cause().to_string(),
            "Invalid bitmap glyph request of 4294967296 pixels per em. The value must be finite, greater than zero, and at most 4294967295."
        );

        params.font_size = px(24.0);
        params.scale_factor = 1.0;
        assert!(!system.rasterize_glyph(&params)?.pixels.is_empty());

        Ok(())
    }

    #[test]
    fn cbdt_image_data_survives_retrieval_decode_and_target_errors() -> Result<()> {
        let _apartment = ColorRasterApartment::new()?;

        let wic: IWICImagingFactory =
            unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? };

        let system = new_text_system()?;
        let params = bitmap_test_params(&*system)?;
        system.rasterize_glyph(&params)?;
        let rasterizer = test_rasterizer(params.font_id, gpui_fonts::NOTO_COLOR_EMOJI.data, 0)?;
        let face = rasterizer.font_face(params.font_id);
        let glyph_id = params.glyph_id.0 as u16;
        let png = DWRITE_GLYPH_IMAGE_FORMATS_PNG;

        assert!(
            GlyphImageData::new(
                face,
                glyph_id,
                24,
                png | DWRITE_GLYPH_IMAGE_FORMATS_PREMULTIPLIED_B8G8R8A8
            )
            .is_err()
        );

        assert!(GlyphImageData::new(face, glyph_id, 24, DWRITE_GLYPH_IMAGE_FORMATS_SVG)?.is_none());

        // Each failure unwinds the data guard before the next retrieval.
        let load = |length: usize| -> Result<_> {
            let image = GlyphImageData::new(face, glyph_id, 24, png)?
                .context("DirectWrite returned no PNG data for the fixture glyph")?;

            let bytes = image.bytes()?;

            let bitmap =
                decode_bitmap_glyph(&wic, &image.data, &bytes[..length.min(bytes.len())], png)?;

            Ok((image.data, bitmap))
        };

        for iteration in 0..3 {
            assert!(load(16).is_err());
            let (data, bitmap) = load(usize::MAX)?;
            assert!(decode_bitmap_glyph(&wic, &data, b"not a PNG", png).is_err());
            let mut image = GlyphImageData::new(face, glyph_id, 24, png)?.unwrap();
            let mut invalid = data;
            let D2D_SIZE_U { width, height } = data.pixelSize;
            let wrong_width = width + 1;
            invalid.pixelSize.width = wrong_width;

            assert_error(
                decode_bitmap_glyph(&wic, &invalid, image.bytes()?, png),
                format_args!(
                    "Decoded PNG is {width} x {height} pixels. DirectWrite reported {wrong_width} x {height}."
                ),
            );

            for (width, height, reason) in [
                (0, 1, "Both dimensions must be positive"),
                (1, 0, "Both dimensions must be positive"),
                (u32::MAX, 1, "Maximum dimension is 2147483647"),
                (65536, 65536, "exceeding the buffer limit"),
            ] {
                invalid.pixelSize = D2D_SIZE_U { width, height };
                let error = decode_bitmap_glyph(&wic, &invalid, image.bytes()?, png).unwrap_err();
                let message = error.to_string();
                assert!(
                    message.contains(&format!("{width} x {height} pixels")),
                    "{message}"
                );
                assert!(message.contains(reason), "{message}");
            }

            let byte_count = image.data.imageDataSize;
            let max_bytes = isize::MAX;
            image.data.imageData = null();
            assert_error(
                image.bytes(),
                format_args!("Null bitmap glyph data pointer for a {byte_count}-byte buffer."),
            );

            image.data.imageDataSize = 0;
            assert_error(
                image.bytes(),
                format_args!("Bitmap glyph buffer has 0 bytes. Expected 1..={max_bytes} bytes."),
            );
            drop(image);

            invalid = data;
            invalid.pixelsPerEm = 0;

            assert_error(
                bitmap_glyph_rect(&params, &invalid),
                format_args!(
                    "Bitmap glyph {glyph_id} has a strike size of 0 pixels per em. Expected a positive size."
                ),
            );

            for edge in [f32::NAN, f32::INFINITY, i32::MAX as f32] {
                let rect = D2D_RECT_F {
                    right: edge,
                    ..Default::default()
                };
                let edge = f64::from(edge);

                assert_error(
                    bitmap_ink_bounds(rect),
                    format_args!(
                        "Rounded bitmap right edge {edge} must be finite and within -2147483648..=2147483647 device pixels."
                    ),
                );
            }

            let destination = bitmap_glyph_rect(&params, &data)?;
            let bounds = bitmap_ink_bounds(destination)?;
            let failed_bounds = oversized_color_bounds(&wic, params.raster_style)?;
            assert!(draw_bitmap_glyph(&wic, &params, &bitmap, destination, failed_bounds).is_err());
            let actual = draw_bitmap_glyph(&wic, &params, &bitmap, destination, bounds)?;
            actual.validate()?;

            assert!(
                !actual.pixels.is_empty(),
                "glyph data lost after error {iteration}"
            );
        }

        Ok(())
    }

    #[test]
    fn wic_readback_preserves_rows_and_converts_premultiplied_alpha() -> Result<()> {
        let _apartment = ColorRasterApartment::new()?;
        let factory: IWICImagingFactory =
            unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? };
        let source = [
            0, 0, 0, 0, 32, 64, 128, 128, 17, 18, 19, 20, 255, 0, 0, 255, 9, 8, 7, 0, 21, 22, 23,
            24,
        ];
        let bitmap = unsafe {
            factory.CreateBitmapFromMemory(2, 2, &GUID_WICPixelFormat32bppPBGRA, 12, &source)?
        };

        let data = DWRITE_GLYPH_IMAGE_DATA {
            pixelsPerEm: 2,
            pixelSize: D2D_SIZE_U {
                width: 2,
                height: 2,
            },
            horizontalLeftOrigin: POINT { x: 1, y: 2 },
            ..Default::default()
        };

        let bytes = [0, 0, 0, 0, 32, 64, 128, 128, 255, 0, 0, 255, 0, 0, 0, 0];

        let bgra = DWRITE_GLYPH_IMAGE_FORMATS_PREMULTIPLIED_B8G8R8A8;
        let decoded = decode_bitmap_glyph(&factory, &data, &bytes, bgra)?;

        for current in [&bitmap, &decoded] {
            for (alpha, expected_alpha) in [(255, 128), (128, 64), (64, 32)] {
                let pixels = read_color_bitmap(current, 2, 2, alpha)?;

                assert_eq!(
                    pixels,
                    [
                        [0; 4],
                        [64, 128, 255, expected_alpha],
                        [255, 0, 0, alpha],
                        [0; 4]
                    ]
                    .concat()
                );
            }

            assert_eq!(read_color_bitmap(current, 2, 2, 0)?, vec![0; 16]);
        }

        for length in [15, 0] {
            assert_error(
                decode_bitmap_glyph(&factory, &data, &bytes[..length], bgra),
                format_args!(
                    "Premultiplied BGRA glyph of 2 x 2 pixels has {length} bytes. Expected 16 (4 bytes per pixel)."
                ),
            );
        }

        assert_error(
            decode_bitmap_glyph(&factory, &data, &bytes, DWRITE_GLYPH_IMAGE_FORMATS_SVG),
            "Unsupported DirectWrite bitmap glyph format 0x8. Expected PNG or premultiplied BGRA.",
        );

        let system = new_text_system()?;
        let mut params = bitmap_test_params(&*system)?;
        params.font_size = px(4.0);
        params.scale_factor = 0.0;

        assert_error(
            bitmap_glyph_rect(&params, &data),
            "Bitmap glyph scale 0 must be finite and greater than zero. It was computed from font size 4, raster scale factor 0, and strike size 2 pixels per em.",
        );

        params.scale_factor = 1.0;

        let destination = bitmap_glyph_rect(&params, &data)?;
        let bounds = bitmap_ink_bounds(destination)?;

        assert_eq!(
            (bounds.left, bounds.top, bounds.right, bounds.bottom),
            (-2, -4, 2, 0)
        );

        let actual = draw_bitmap_glyph(&factory, &params, &decoded, destination, bounds)?;
        actual.validate()?;

        // Premultiplied interpolation keeps transparent neighbors from darkening the color.
        let edge = &actual.pixels[4..8];
        assert!(edge[3] > 0 && edge[3] < 128);
        assert!(edge[0].abs_diff(64) <= 8);
        assert!(edge[1].abs_diff(128) <= 8);
        assert!(edge[2] >= 248);

        Ok(())
    }
}
