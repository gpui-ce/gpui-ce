use crate::{
    App, Bounds, InlineLayout, LineLayout, PaintFragment, PaintQuad, Pixels, Point, Result,
    ShapedTextLayout, SharedString, TextAlign, TextSystem, VisualLine, Window, point, size,
};
use derive_more::{Deref, DerefMut};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

static GLYPH_PAINT_ERRORS: AtomicUsize = AtomicUsize::new(0);
const MAX_LOGGED_GLYPH_PAINT_ERRORS: usize = 20;

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
            TextPaintPass::Foreground,
            window,
            cx,
        )
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
        paint_visual_text(
            origin,
            &self.layout,
            line_height,
            align,
            align_width,
            TextPaintPass::Background,
            window,
            cx,
        )
    }
}

/// Text that has been shaped and decorated by the text layout system.
#[derive(Debug, Deref, DerefMut)]
pub struct ShapedText {
    #[deref]
    #[deref_mut]
    pub(crate) layout: Arc<ShapedTextLayout>,
    /// The text that was shaped.
    pub text: SharedString,
}

impl ShapedText {
    /// The length of the text in UTF-8 bytes.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.layout.len()
    }

    /// Paint this text to the window.
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
            TextPaintPass::Foreground,
            window,
            cx,
        )
    }

    /// Paint the background of this text to the window.
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

        paint_visual_text(
            origin,
            &self.layout.layout,
            line_height,
            align,
            align_width,
            TextPaintPass::Background,
            window,
            cx,
        )
    }
}

impl InlineLayout {
    /// Paint the text-run backgrounds in this inline layout.
    pub fn paint_background(
        &self,
        origin: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<()> {
        paint_inline_layout(self, origin, TextPaintPass::Background, window, cx)
    }

    /// Paint the glyphs and foreground decorations in this inline layout.
    pub fn paint(&self, origin: Point<Pixels>, window: &mut Window, cx: &mut App) -> Result<()> {
        paint_inline_layout(self, origin, TextPaintPass::Foreground, window, cx)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum TextPaintPass {
    Background,
    Foreground,
}

struct LinePaintGeometry {
    origin: Point<Pixels>,
    height: Pixels,
    baseline_y: Pixels,
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
    cx: &mut App,
) -> Result<()> {
    if inline.lines.is_empty() {
        return Ok(());
    }

    let placement = place_inline_layout(origin, inline.alignment_offset, window);

    let lines = inline.lines.iter().map(|line| {
        let line_origin = origin + line.origin + placement.delta;

        LinePaintGeometry {
            origin: line_origin,
            height: line.size.height,
            baseline_y: line_origin.y + line.baseline,
        }
    });

    paint_visual_lines(
        &inline.layout,
        lines,
        Bounds::new(placement.origin, inline.size),
        pass,
        window,
        cx,
    )
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

fn paint_visual_lines(
    layout: &LineLayout,
    lines: impl Iterator<Item = LinePaintGeometry>,
    bounds: Bounds<Pixels>,
    pass: TextPaintPass,
    window: &mut Window,
    cx: &mut App,
) -> Result<()> {
    let text_system = cx.text_system().clone();

    window.paint_layer(bounds, |window| {
        for (visual_line, line) in layout.visual_lines.iter().zip(lines) {
            let context = FragmentPaintContext {
                line_origin: line.origin,
                line_height: line.height,
                baseline_y: line.baseline_y,
                layout,
                pass,
                text_system: &text_system,
            };

            for fragment in &layout.paint_fragments[visual_line.paint_fragment_range.clone()] {
                paint_text_fragment(fragment, &context, window);
            }
        }

        Ok(())
    })
}

fn paint_text_fragment(
    fragment: &PaintFragment,
    context: &FragmentPaintContext<'_>,
    window: &mut Window,
) {
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
        return;
    }

    let max_glyph_size = context
        .text_system
        .bounding_box(fragment.font_id, fragment.font_size)
        .size;
    for glyph in &*fragment.glyphs {
        let cull_origin = point(
            context.line_origin.x + glyph.position.x,
            context.line_origin.y,
        );
        if !Bounds::new(cull_origin, max_glyph_size).intersects(&window.content_mask().bounds) {
            continue;
        }

        let glyph_origin = point(
            context.line_origin.x + glyph.position.x,
            context.baseline_y + glyph.position.y,
        );
        let result = if glyph.is_emoji {
            window.paint_emoji_with_color(
                glyph_origin,
                fragment.font_id,
                glyph.id,
                fragment.font_size,
                fragment.style.color,
            )
        } else {
            window.paint_glyph(
                glyph_origin,
                fragment.font_id,
                glyph.id,
                fragment.font_size,
                fragment.style.color,
            )
        };

        if let Err(error) = result {
            let error_idx = GLYPH_PAINT_ERRORS.fetch_add(1, Ordering::Relaxed);

            if error_idx < MAX_LOGGED_GLYPH_PAINT_ERRORS {
                log::error!(
                    "failed to paint glyph {:?} from font {:?}: {error:#}",
                    glyph.id,
                    fragment.font_id
                );
            } else if error_idx == MAX_LOGGED_GLYPH_PAINT_ERRORS {
                log::error!("suppressing further glyph paint errors");
            }
        }
    }
}

fn paint_visual_text(
    origin: Point<Pixels>,
    layout: &LineLayout,
    line_height: Pixels,
    align: TextAlign,
    align_width: Option<Pixels>,
    pass: TextPaintPass,
    window: &mut Window,
    cx: &mut App,
) -> Result<()> {
    let visual_lines = &layout.visual_lines;

    if visual_lines.is_empty() {
        return Ok(());
    }

    if let Some(lines) = layout.platform_layout.native_lines() {
        let lines = lines.iter().map(|line| {
            let line_origin = origin + line.origin;

            LinePaintGeometry {
                origin: line_origin,
                height: line.size.height,
                baseline_y: line_origin.y + line.baseline,
            }
        });

        return paint_visual_lines(
            layout,
            lines,
            Bounds::new(origin, layout.platform_layout.size()),
            pass,
            window,
            cx,
        );
    }

    let bounds = visual_text_bounds(origin, line_height, align, align_width, visual_lines);
    let padding_top = (line_height - layout.ascent - layout.descent) / 2.;

    let lines = visual_lines.iter().enumerate().map(|(line_idx, line)| {
        let line_origin = point(
            visual_line_origin_x(origin.x, align, align_width, line),
            origin.y + line_idx as f32 * line_height,
        );

        LinePaintGeometry {
            origin: line_origin,
            height: line_height,
            baseline_y: line_origin.y + padding_top + layout.ascent,
        }
    });

    paint_visual_lines(layout, lines, bounds, pass, window, cx)
}

fn visual_text_bounds(
    origin: Point<Pixels>,
    line_height: Pixels,
    align: TextAlign,
    align_width: Option<Pixels>,
    visual_lines: &[VisualLine],
) -> Bounds<Pixels> {
    let mut left = Pixels::MAX;
    let mut right = Pixels::MIN;

    for line in visual_lines {
        let line_left = visual_line_origin_x(origin.x, align, align_width, line);
        left = left.min(line_left);
        right = right.max(line_left + line.advance_width);
    }
    Bounds::new(
        point(left, origin.y),
        size(right - left, line_height * visual_lines.len() as f32),
    )
}

fn visual_line_origin_x(
    origin_x: Pixels,
    align: TextAlign,
    align_width: Option<Pixels>,
    line: &VisualLine,
) -> Pixels {
    line.offset
        + aligned_visual_origin_x(
            origin_x,
            align_width.unwrap_or(line.advance_width),
            line.advance_width,
            align,
            line.direction,
        )
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
    let run_baseline = baseline_y + fragment.baseline_offset;

    if foreground {
        if let Some(mut underline) = fragment.style.underline {
            underline.color = Some(underline.color.unwrap_or(fragment.style.color));
            window.paint_underline(
                point(
                    range.start,
                    run_baseline + fragment.underline_offset.unwrap_or(layout.descent * 0.618),
                ),
                range.end - range.start,
                &underline,
            );
        }
        if let Some(mut strike) = fragment.style.strikethrough {
            strike.color = Some(strike.color.unwrap_or(fragment.style.color));
            let strike_y = run_baseline
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
        let vertical = fragment
            .background_range
            .as_ref()
            .map(|range| baseline_y + range.start..baseline_y + range.end)
            .unwrap_or(line_origin.y..line_origin.y + line_height);
        window.paint_quad(
            PaintQuad::new(Bounds::new(
                point(range.start, vertical.start),
                size(range.end - range.start, vertical.end - vertical.start),
            ))
            .background(color),
        );
    }
}

fn aligned_visual_origin_x(
    origin_x: Pixels,
    align_width: Pixels,
    line_width: Pixels,
    align: TextAlign,
    direction: crate::ResolvedDirection,
) -> Pixels {
    match align {
        TextAlign::Start if direction.is_rtl() => origin_x + align_width - line_width,
        TextAlign::Start | TextAlign::Left => origin_x,
        TextAlign::Center => (origin_x * 2.0 + align_width - line_width) / 2.0,
        TextAlign::End if direction.is_rtl() => origin_x,
        TextAlign::End | TextAlign::Right => origin_x + align_width - line_width,
    }
}
