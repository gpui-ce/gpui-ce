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

/// A shaped line that is ready to measure and paint.
///
/// [`WindowTextSystem::shape_line`](crate::WindowTextSystem::shape_line) does
/// the expensive shaping work. Reuse the result to measure with
/// [`ShapedLine::width`] and paint on each frame. Paint backgrounds first with
/// [`ShapedLine::paint_background`] when a row needs to sit behind other content.
///
/// # Examples
///
/// ```
/// # use gpui::{EmptyView, TestApp, TextRun, font, hsla, px};
/// # let mut app = TestApp::new();
/// # let mut window = app.open_window(|_window, _cx| EmptyView);
/// window.update(|_view, window, _cx| {
///     let run = TextRun {
///         len: "Hi".len(),
///         font: font("Lilex"),
///         color: hsla(0., 0., 0., 1.),
///         ..Default::default()
///     };
///     let line = window.text_system().shape_line("Hi".into(), px(16.), &[run]);
///     assert_eq!(line.text.as_ref(), "Hi");
///     assert_eq!(line.len(), 2);
/// });
/// ```
#[derive(Clone, Debug, Deref, DerefMut)]
pub struct ShapedLine {
    #[deref]
    #[deref_mut]
    pub(crate) layout: Arc<LineLayout>,
    /// The text that was shaped for this line.
    pub text: SharedString,
}

impl ShapedLine {
    /// Returns the length of the line in UTF-8 bytes.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.layout.len
    }

    /// Returns the width of the line in logical pixels.
    ///
    /// This is the glyph advance width from the text shaping system. Use it to
    /// advance a "pen" when you paint several lines or fragments on one row.
    pub fn width(&self) -> Pixels {
        self.layout.width
    }

    /// Paints the glyphs, underlines, and strikethroughs of the line.
    ///
    /// `origin` is the top-left corner of the line box, which is `line_height`
    /// tall. The baseline is placed inside that box, centered by the font's
    /// ascent and descent. When `align_width` is `Some`, the line is aligned
    /// within that width using `align`. Otherwise it is aligned in its own width.
    ///
    /// Call this during paint. Background colors are not painted here, so call
    /// [`ShapedLine::paint_background`] first.
    ///
    /// # Errors
    ///
    /// Returns an error only if the window's paint layer fails. A glyph that
    /// fails to paint is logged and skipped, and the call still succeeds.
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
            TextPaintPass::Foreground,
            window,
            cx,
        )
    }

    /// Paints the background color of each run in the line.
    ///
    /// The arguments match [`ShapedLine::paint`]. Call this before `paint`, so
    /// that the glyphs are drawn over the backgrounds.
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
            &self.layout.visual_lines,
            TextPaintPass::Background,
            window,
            cx,
        )
    }
}

/// Shaped text that may contain hard breaks and soft-wrapped lines.
///
/// [`WindowTextSystem::shape_text`](crate::WindowTextSystem::shape_text) returns
/// one visual line for each wrapped or hard-broken line. Paint it with
/// [`ShapedText::paint`], and paint backgrounds first with
/// [`ShapedText::paint_background`]. It derefs to [`ShapedTextLayout`], so the
/// same result supplies measurements such as [`ShapedTextLayout::size`] and
/// [`ShapedTextLayout::line_count`].
///
/// # Examples
///
/// ```
/// # use gpui::{EmptyView, TestApp, TextRun, font, hsla, px};
/// # let mut app = TestApp::new();
/// # let mut window = app.open_window(|_window, _cx| EmptyView);
/// window.update(|_view, window, _cx| {
///     let text = "Hello\nworld";
///     let run = TextRun {
///         len: text.len(),
///         font: font("Lilex"),
///         color: hsla(0., 0., 0., 1.),
///         ..Default::default()
///     };
///     let shaped = window
///         .text_system()
///         .shape_text(text, px(16.), &[run], None, None)
///         .unwrap();
///     assert_eq!(shaped.len(), text.len());
/// });
/// ```
#[derive(Debug, Deref, DerefMut)]
pub struct ShapedText {
    #[deref]
    #[deref_mut]
    pub(crate) layout: Arc<ShapedTextLayout>,
    /// The text that was shaped.
    pub text: SharedString,
}

impl ShapedText {
    /// Returns the length of the text in UTF-8 bytes.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.layout.len()
    }

    /// Paints the glyphs and decorations of every visual line.
    ///
    /// `origin` is the top-left corner of the first line. Each following line
    /// starts `line_height` below the previous one. `bounds` supplies the width
    /// used for alignment; when it is `None`, the layout's wrap width is used.
    /// See [`ShapedLine::paint`] for how the arguments place the text.
    ///
    /// # Errors
    ///
    /// Returns an error only if the window's paint layer fails. A glyph that
    /// fails to paint is logged and skipped.
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
            TextPaintPass::Foreground,
            window,
            cx,
        )
    }

    /// Paints the background colors of every visual line before the foreground.
    /// The arguments have the same meaning as [`ShapedText::paint`].
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
            &self.layout.visual_lines,
            TextPaintPass::Background,
            window,
            cx,
        )
    }
}

impl InlineLayout {
    /// Paints the background color of each text run in the inline layout.
    ///
    /// `origin` is the top-left corner of the layout's box. Call this before
    /// [`InlineLayout::paint`].
    pub fn paint_background(
        &self,
        origin: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<()> {
        paint_inline_layout(self, origin, TextPaintPass::Background, window, cx)
    }

    /// Paints the glyphs and underlines or strikethroughs of the inline layout.
    ///
    /// `origin` is the top-left corner of the layout's box. This method paints
    /// only text. The atomic boxes in [`InlineLayout::boxes`] are not painted here.
    pub fn paint(&self, origin: Point<Pixels>, window: &mut Window, cx: &mut App) -> Result<()> {
        paint_inline_layout(self, origin, TextPaintPass::Foreground, window, cx)
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
    cx: &mut App,
) -> Result<()> {
    if inline.lines.is_empty() {
        return Ok(());
    }

    let text_system = cx.text_system().clone();
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
            );
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
) {
    let context = FragmentPaintContext {
        line_origin,
        line_height,
        baseline_y,
        layout,
        pass,
        text_system,
    };
    for fragment in &layout.paint_fragments[visual_line.paint_fragment_range.clone()] {
        paint_text_fragment(fragment, &context, window);
    }
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
    visual_lines: &[VisualLine],
    pass: TextPaintPass,
    window: &mut Window,
    cx: &mut App,
) -> Result<()> {
    if visual_lines.is_empty() {
        return Ok(());
    }

    let line_bounds = visual_text_bounds(origin, line_height, align, align_width, visual_lines);
    window.paint_layer(line_bounds, |window| {
        let padding_top = (line_height - layout.ascent - layout.descent) / 2.;
        let text_system = cx.text_system().clone();

        for (line_index, line) in visual_lines.iter().enumerate() {
            let line_origin = point(
                visual_line_origin_x(origin.x, align, align_width, line),
                origin.y + line_index as f32 * line_height,
            );
            paint_visual_line(
                layout,
                line,
                line_origin,
                line_height,
                line_origin.y + padding_top + layout.ascent,
                pass,
                &text_system,
                window,
            );
        }

        Ok(())
    })
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
        window.paint_quad(
            PaintQuad::new(Bounds::new(
                point(range.start, line_origin.y),
                size(range.end - range.start, line_height),
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
