use crate::{
    App, Bounds, InlineLayout, LineLayout, PaintFragment, Pixels, Point, Result, ScaledPixels,
    SharedString, TextAlign, TextSystem, TransformationMatrix, VisualLine, Window,
    WrappedLineLayout, fill, point, px, size,
};
use derive_more::{Deref, DerefMut};
use std::sync::Arc;

/// A line of text that has been shaped and decorated.
#[derive(Clone, Debug, Deref, DerefMut)]
pub struct ShapedLine {
    #[deref]
    #[deref_mut]
    pub(crate) layout: Arc<LineLayout>,
    /// The text that was shaped for this line.
    pub text: SharedString,
}

impl ShapedLine {
    /// The length of the line in utf-8 bytes.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.layout.len
    }

    /// The width of the shaped line in pixels.
    ///
    /// This is the glyph advance width computed by the text shaping system and is useful for
    /// incrementally advancing a "pen" when painting multiple fragments on the same row.
    pub fn width(&self) -> Pixels {
        self.layout.width
    }

    /// Paint the line of text to the window.
    pub fn paint(
        &self,
        origin: Point<Pixels>,
        line_height: Pixels,
        align: TextAlign,
        align_width: Option<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<()> {
        paint_visual_text(
            origin,
            &self.layout,
            line_height,
            align,
            align_width,
            &self.layout.visual_lines,
            window,
            cx,
        )?;

        Ok(())
    }

    /// Paint the background of the line to the window.
    pub fn paint_background(
        &self,
        origin: Point<Pixels>,
        line_height: Pixels,
        align: TextAlign,
        align_width: Option<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<()> {
        paint_visual_background(
            origin,
            &self.layout,
            line_height,
            align,
            align_width,
            &self.layout.visual_lines,
            window,
            cx,
        )?;

        Ok(())
    }
}

/// A line of text that has been shaped, decorated, and wrapped by the text layout system.
#[derive(Debug, Deref, DerefMut)]
pub struct WrappedLine {
    #[deref]
    #[deref_mut]
    pub(crate) layout: Arc<WrappedLineLayout>,
    /// The text that was shaped for this line.
    pub text: SharedString,
}

impl WrappedLine {
    /// The length of the underlying, unwrapped layout, in utf-8 bytes.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.layout.len()
    }

    /// Paint this line of text to the window.
    pub fn paint(
        &self,
        origin: Point<Pixels>,
        line_height: Pixels,
        align: TextAlign,
        bounds: Option<Bounds<Pixels>>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<()> {
        let align_width = match bounds {
            Some(bounds) => Some(bounds.size.width),
            None => self.layout.wrap_width,
        };

        paint_visual_text(
            origin,
            &self.layout.layout,
            line_height,
            align,
            align_width,
            &self.layout.visual_lines,
            window,
            cx,
        )?;

        Ok(())
    }

    /// Paint the background of line of text to the window.
    pub fn paint_background(
        &self,
        origin: Point<Pixels>,
        line_height: Pixels,
        align: TextAlign,
        bounds: Option<Bounds<Pixels>>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<()> {
        let align_width = match bounds {
            Some(bounds) => Some(bounds.size.width),
            None => self.layout.wrap_width,
        };

        paint_visual_background(
            origin,
            &self.layout.layout,
            line_height,
            align,
            align_width,
            &self.layout.visual_lines,
            window,
            cx,
        )?;

        Ok(())
    }
}

impl InlineLayout {
    /// Paint the text-run backgrounds in this inline layout.
    pub fn paint_background(
        &self,
        origin: Point<Pixels>,
        window: &mut Window,
        context: &mut App,
    ) -> Result<()> {
        paint_inline_layout(self, origin, TextPaintPass::Background, window, context)
    }

    /// Paint the glyphs and foreground decorations in this inline layout.
    pub fn paint(
        &self,
        origin: Point<Pixels>,
        window: &mut Window,
        context: &mut App,
    ) -> Result<()> {
        paint_inline_layout(self, origin, TextPaintPass::Foreground, window, context)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum TextPaintPass {
    Background,
    Foreground,
}

struct FragmentPaintContext<'a> {
    line_origin: Point<Pixels>,
    line_height: Pixels,
    baseline_y: Pixels,
    layout: &'a LineLayout,
    pass: TextPaintPass,
    text_system: &'a TextSystem,
}

fn paint_inline_layout(
    inline: &InlineLayout,
    origin: Point<Pixels>,
    pass: TextPaintPass,
    window: &mut Window,
    context: &mut App,
) -> Result<()> {
    if inline.lines.is_empty() {
        return Ok(());
    }

    let text_system = context.text_system().clone();
    let placement = place_inline_layout(origin, inline.alignment_offset, window);
    window.paint_layer(Bounds::new(placement.origin, inline.size), |window| {
        for (line, visual_line) in inline.lines.iter().zip(&inline.layout.visual_lines) {
            let line_origin = origin + line.origin + placement.delta;
            paint_visual_line(
                &inline.layout,
                visual_line,
                line_origin,
                line.size.height,
                line_origin.y + line.baseline,
                pass,
                &text_system,
                window,
            )?;
        }

        Ok(())
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct InlineLayoutPlacement {
    pub(crate) origin: Point<Pixels>,
    pub(crate) delta: Point<Pixels>,
}

pub(crate) fn place_inline_layout(
    content_origin: Point<Pixels>,
    alignment_offset: Pixels,
    window: &Window,
) -> InlineLayoutPlacement {
    let anchor = content_origin + point(alignment_offset, Pixels::ZERO);
    let placed_anchor = window.pixel_snap_point(anchor);
    let delta = placed_anchor - anchor;
    InlineLayoutPlacement {
        origin: content_origin + delta,
        delta,
    }
}

fn paint_visual_line(
    layout: &LineLayout,
    visual_line: &VisualLine,
    line_origin: Point<Pixels>,
    line_height: Pixels,
    baseline_y: Pixels,
    pass: TextPaintPass,
    text_system: &TextSystem,
    window: &mut Window,
) -> Result<()> {
    let context = FragmentPaintContext {
        line_origin,
        line_height,
        baseline_y,
        layout,
        pass,
        text_system,
    };
    for fragment in &layout.paint_fragments[visual_line.fragment_range.clone()] {
        paint_text_fragment(fragment, &context, window)?;
    }
    Ok(())
}

fn paint_text_fragment(
    fragment: &PaintFragment,
    context: &FragmentPaintContext<'_>,
    window: &mut Window,
) -> Result<()> {
    paint_fragment_decorations_at(
        fragment,
        context.line_origin,
        context.line_height,
        context.baseline_y,
        context.layout,
        window,
        context.pass == TextPaintPass::Foreground,
    );
    if context.pass == TextPaintPass::Background {
        return Ok(());
    }

    let max_glyph_size = context
        .text_system
        .bounding_box(fragment.font_id, fragment.font_size)
        .size;
    for glyph in &fragment.glyphs {
        let cull_origin = point(
            context.line_origin.x + glyph.position.x,
            context.line_origin.y,
        );
        if !glyph_bounds_intersects_content_mask(
            Bounds::new(cull_origin, max_glyph_size),
            window.content_mask().bounds,
            window.current_transform(),
            window.scale_factor(),
        ) {
            continue;
        }

        let glyph_origin = point(
            context.line_origin.x + glyph.position.x,
            context.baseline_y + glyph.position.y,
        );
        if glyph.is_emoji {
            window.paint_emoji(glyph_origin, fragment.font_id, glyph.id, fragment.font_size)?;
        } else {
            window.paint_glyph(
                glyph_origin,
                fragment.font_id,
                glyph.id,
                fragment.font_size,
                fragment.style.color,
            )?;
        }
    }
    Ok(())
}

fn paint_visual_text(
    origin: Point<Pixels>,
    layout: &LineLayout,
    line_height: Pixels,
    align: TextAlign,
    align_width: Option<Pixels>,
    visual_lines: &[VisualLine],
    window: &mut Window,
    cx: &mut App,
) -> Result<()> {
    if visual_lines.is_empty() {
        return Ok(());
    }

    let paint_width = visual_lines
        .iter()
        .map(|line| line.advance_width)
        .fold(Pixels::ZERO, Pixels::max);
    let line_bounds = Bounds::new(
        origin,
        size(paint_width, line_height * visual_lines.len() as f32),
    );
    window.paint_layer(line_bounds, |window| {
        let padding_top = (line_height - layout.ascent - layout.descent) / 2.;
        let text_system = cx.text_system().clone();

        for (line_index, line) in visual_lines.iter().enumerate() {
            let line_origin = point(
                aligned_visual_origin_x(
                    origin.x,
                    align_width.unwrap_or(line.advance_width),
                    line.advance_width,
                    align,
                ),
                origin.y + line_index as f32 * line_height,
            );
            paint_visual_line(
                layout,
                line,
                line_origin,
                line_height,
                line_origin.y + padding_top + layout.ascent,
                TextPaintPass::Foreground,
                &text_system,
                window,
            )?;
        }
        Ok(())
    })
}

fn paint_visual_background(
    origin: Point<Pixels>,
    layout: &LineLayout,
    line_height: Pixels,
    align: TextAlign,
    align_width: Option<Pixels>,
    visual_lines: &[VisualLine],
    window: &mut Window,
    cx: &mut App,
) -> Result<()> {
    if visual_lines.is_empty() {
        return Ok(());
    }
    let paint_width = visual_lines
        .iter()
        .map(|line| line.advance_width)
        .fold(Pixels::ZERO, Pixels::max);
    let line_bounds = Bounds::new(
        origin,
        size(paint_width, line_height * visual_lines.len() as f32),
    );
    window.paint_layer(line_bounds, |window| {
        let padding_top = (line_height - layout.ascent - layout.descent) / 2.;
        let text_system = cx.text_system().clone();
        for (line_index, line) in visual_lines.iter().enumerate() {
            let line_origin = point(
                aligned_visual_origin_x(
                    origin.x,
                    align_width.unwrap_or(line.advance_width),
                    line.advance_width,
                    align,
                ),
                origin.y + line_index as f32 * line_height,
            );
            paint_visual_line(
                layout,
                line,
                line_origin,
                line_height,
                line_origin.y + padding_top + layout.ascent,
                TextPaintPass::Background,
                &text_system,
                window,
            )?;
        }
        Ok(())
    })
}

fn paint_fragment_decorations_at(
    fragment: &PaintFragment,
    line_origin: Point<Pixels>,
    line_height: Pixels,
    baseline_y: Pixels,
    layout: &LineLayout,
    window: &mut Window,
    foreground: bool,
) {
    let range = line_origin.x + fragment.x_range.start..line_origin.x + fragment.x_range.end;
    if foreground {
        if let Some(mut underline) = fragment.style.underline {
            underline.color = Some(underline.color.unwrap_or(fragment.style.color));
            window.paint_underline(
                point(
                    range.start,
                    baseline_y + fragment.underline_offset.unwrap_or(layout.descent * 0.618),
                ),
                range.end - range.start,
                &underline,
            );
        }
        if let Some(mut strike) = fragment.style.strikethrough {
            strike.color = Some(strike.color.unwrap_or(fragment.style.color));
            let strike_y = baseline_y
                + fragment
                    .strikethrough_offset
                    .unwrap_or(-layout.ascent * 0.5);
            window.paint_strikethrough(
                point(range.start, strike_y),
                range.end - range.start,
                &strike,
            );
        }
    } else if let Some(color) = fragment.style.background_color {
        window.paint_quad(fill(
            Bounds::new(
                point(range.start, line_origin.y),
                size(range.end - range.start, line_height),
            ),
            color,
        ));
    }
}

fn aligned_visual_origin_x(
    origin_x: Pixels,
    align_width: Pixels,
    line_width: Pixels,
    align: TextAlign,
) -> Pixels {
    match align {
        TextAlign::Left => origin_x,
        TextAlign::Center => (origin_x * 2.0 + align_width - line_width) / 2.0,
        TextAlign::Right => origin_x + align_width - line_width,
    }
}

fn glyph_bounds_intersects_content_mask(
    glyph_bounds: Bounds<Pixels>,
    content_mask_bounds: Bounds<Pixels>,
    transform: TransformationMatrix,
    scale_factor: f32,
) -> bool {
    let visual_bounds = if transform == TransformationMatrix::unit() {
        glyph_bounds
    } else {
        crate::scene::transform_bounds(glyph_bounds.scale(scale_factor), transform)
            .map(|coordinate: ScaledPixels| px(coordinate.0 / scale_factor))
    };

    visual_bounds.intersects(&content_mask_bounds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translated_glyph_culling_uses_visual_bounds() {
        let glyph_bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(10.0), px(10.0)));
        let content_mask = Bounds::new(point(px(100.0), px(0.0)), size(px(10.0), px(10.0)));
        let transform =
            TransformationMatrix::unit().translate(point(ScaledPixels(100.0), ScaledPixels(0.0)));

        assert!(glyph_bounds_intersects_content_mask(
            glyph_bounds,
            content_mask,
            transform,
            1.0
        ));
    }

    #[test]
    fn scaled_glyph_culling_uses_visual_bounds() {
        let glyph_bounds = Bounds::new(point(px(10.0), px(10.0)), size(px(10.0), px(10.0)));
        let content_mask = Bounds::new(point(px(20.0), px(20.0)), size(px(20.0), px(20.0)));
        let transform = TransformationMatrix::unit().scale(size(2.0, 2.0));

        assert!(glyph_bounds_intersects_content_mask(
            glyph_bounds,
            content_mask,
            transform,
            1.0
        ));
    }
}
