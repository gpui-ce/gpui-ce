use crate::{
    self as gpui, AbsoluteLength, AlignContent, AlignItems, AlignSelf, BorderStyle, CursorStyle,
    DefiniteLength, Display, Edges, Fill, Filter, FlexDirection, FlexWrap, Font, FontFeatures,
    FontStyle, FontWeight, FontWidth, GridPlacement, GridTemplate, GridTemplateMinSize,
    JustifyContent, LayoutDirection, Length, Pixels, SharedString, StrikethroughStyle,
    StyleRefinement, TextAlign, TextOverflow, TextStyleRefinement, TextTransform, UnderlineStyle,
    UnicodeBidi, VerticalAlign, WhiteSpace, px, relative, rems,
};
pub use gpui_macros::{
    border_style_methods, box_shadow_style_methods, cursor_style_methods, margin_style_methods,
    overflow_style_methods, padding_style_methods, position_style_methods,
    visibility_style_methods,
};
use palette::{Hsla, IntoColor};
use refineable::Refineable;
const ELLIPSIS: SharedString = SharedString::new_static("…");

/// Sets an element's layout, paint and text properties.
///
/// Import the prelude to style an element with method calls. Setters return `Self`, allowing
/// several choices in one expression. This card uses a column flex layout, padding, rounded
/// corners and a shadow:
///
/// ```
/// use gpui::{Display, div, prelude::*, rgb};
///
/// let mut card = div()
///     .flex()
///     .flex_col()
///     .gap_2()
///     .p_4()
///     .rounded_lg()
///     .bg(rgb(0xffffff))
///     .shadow_md();
///
/// assert_eq!(card.style().display, Some(Display::Flex));
/// ```
///
/// Each call writes to the element's [`StyleRefinement`]. Unset properties fall back to
/// [`Style`](crate::Style)'s defaults. Setting the same property again normally replaces its
/// earlier value; an additive setter such as [`Styled::blur`] documents its exception.
/// Interaction styles use refinements too: [`hover`](crate::InteractiveElement::hover) passes
/// an empty refinement to a closure, where these methods describe the changes for that state.
///
/// Setters record a request rather than measuring the element. Layout resolves `size`,
/// `min_size`, `max_size` and percentages within the containing layout; painting then uses
/// the resulting bounds. The [layout and style concept](crate::_documentation::concepts::layout_and_style)
/// explains why a requested size can differ from the final size.
///
/// # Naming scale
///
/// Names generally follow Tailwind CSS. Spacing, sizing and radius suffixes use a quarter-rem
/// scale: `p_4` is 1rem of padding, and `gap_2` is a 0.5rem gap. At the default 16px root size,
/// those lengths are 16px and 8px. The common steps are:
///
/// | Suffix | Length | Pixels at the default rem size |
/// | --- | --- | --- |
/// | `0` | `0` | 0px |
/// | `0p5` | `0.125rem` | 2px |
/// | `1` | `0.25rem` | 4px |
/// | `2` | `0.5rem` | 8px |
/// | `4` | `1rem` | 16px |
/// | `8` | `2rem` | 32px |
/// | `16` | `4rem` | 64px |
/// | `px` | `1px` | 1px |
///
/// A `p` inside a number denotes a decimal point: `p_1p5` is `0.375rem`. Fraction suffixes
/// such as `_1_2` and `_1_3` use a percentage of the containing layout size on that axis.
/// `full` means 100% for a size or the largest radius for a corner. Where allowed, `auto`
/// leaves the choice to layout. Each method lists its available values.
///
/// Font sizes have their own scale, from [`Styled::text_xs`] (`0.75rem`) to
/// [`Styled::text_3xl`] (`1.875rem`). For instance, `text_sm` is `0.875rem`, or 14px at the
/// default root size. [`Window::set_rem_size`](crate::Window::set_rem_size) changes that
/// root, scaling both these fonts and rem-based spacing.
// Avoid expanding the large reflection macro while rust-analyzer indexes this trait.
#[cfg_attr(
    all(any(feature = "inspector", debug_assertions), not(rust_analyzer)),
    gpui_macros::derive_inspector_reflection
)]
pub trait Styled: Sized {
    /// Returns the [`StyleRefinement`] that the methods on this trait write into.
    ///
    /// Use this when you need to set a property that has no dedicated method. Each element stores its
    /// own refinement, so changes here affect only this element. The refinement is merged over
    /// [`Style`](crate::Style)'s defaults when the element is laid out and painted; an unset field does
    /// not clear the corresponding default.
    fn style(&mut self) -> &mut StyleRefinement;

    /// Applies every property set in `refinement` to this element's style.
    ///
    /// Unset properties keep their current values. Nested refinements merge field by field too:
    /// setting a text color does not clear the font weight. Apply shared styles first and specific
    /// overrides afterward so the latter values win.
    fn refine_style(mut self, refinement: &StyleRefinement) -> Self {
        self.style().refine(refinement);

        self
    }

    gpui_macros::style_helpers!();
    gpui_macros::visibility_style_methods!();
    gpui_macros::margin_style_methods!();
    gpui_macros::padding_style_methods!();
    gpui_macros::position_style_methods!();
    gpui_macros::overflow_style_methods!();
    gpui_macros::cursor_style_methods!();
    gpui_macros::border_style_methods!();
    gpui_macros::box_shadow_style_methods!();

    /// Sets the inline direction of this element and its logical descendants.
    ///
    /// The direction is inherited by descendants and is used by logical inline-start/end layout. It
    /// does not rewrite physical `left` and `right` properties. Prefer [`Styled::rtl`] or
    /// [`Styled::ltr`] unless you need to pass a runtime [`LayoutDirection`].
    fn direction(mut self, direction: LayoutDirection) -> Self {
        self.style().direction = Some(direction);

        self
    }

    /// Lays out this element and its logical descendants right to left.
    ///
    /// Use it for right-to-left languages such as Arabic and Hebrew. Descendants inherit this
    /// direction until they set another one; physical left/right properties keep their physical
    /// meaning. See the [layout and style](crate::_documentation::concepts::layout_and_style) page.
    fn rtl(self) -> Self {
        self.direction(LayoutDirection::RightToLeft)
    }

    /// Lays out this element and its logical descendants left to right.
    ///
    /// Use it to restore left-to-right layout inside an element that is set to [`Styled::rtl`].
    /// Elements otherwise inherit their direction, whose resolved default is left to right.
    fn ltr(self) -> Self {
        self.direction(LayoutDirection::LeftToRight)
    }

    /// Sets how this element's text takes part in the Unicode bidirectional algorithm.
    ///
    /// Setting a value marks the property as explicit, so it overrides the default
    /// behavior that GPUI applies otherwise.
    fn unicode_bidi(mut self, unicode_bidi: UnicodeBidi) -> Self {
        let style = self.style();
        style.unicode_bidi = Some(unicode_bidi);
        style.unicode_bidi_explicit = Some(true);

        self
    }

    /// Blurs this element's own content and children, like CSS `filter: blur(<radius>)`.
    ///
    /// The radius is in logical pixels. This isolates the element's subtree, blurs it as
    /// a group, and composites the result back. To blur the content behind the element
    /// instead, use [`Styled::backdrop_blur`].
    ///
    /// This adds to the element's filter chain, so `.blur(8.)` composes with the other filter setters.
    /// A later [`Styled::filter`] call replaces the chain accumulated so far; call `filter` first if
    /// you want to append a blur afterward.
    fn blur(mut self, radius: impl Into<Pixels>) -> Self {
        self.style()
            .filter
            .get_or_insert_with(Vec::new)
            .push(Filter::Blur(radius.into()));
        self
    }

    /// Replaces the list of filters applied to this element's own content, like CSS `filter`.
    ///
    /// Any filters set before this call are discarded. To add one filter to the chain,
    /// use a convenience setter such as [`Styled::blur`].
    fn filter(mut self, filters: impl Into<Vec<Filter>>) -> Self {
        self.style().filter = Some(filters.into());
        self
    }

    /// Blurs the content behind this element, like CSS `backdrop-filter: blur(<radius>)`.
    ///
    /// The radius is in logical pixels. Pair it with a translucent [`Styled::bg`] so the
    /// background tints the blurred content and the result looks like frosted glass.
    ///
    /// This adds to the element's backdrop filter chain. A later [`Styled::backdrop_filter`] call
    /// replaces the chain accumulated so far; call `backdrop_filter` first if you want to append a
    /// blur afterward.
    fn backdrop_blur(mut self, radius: impl Into<Pixels>) -> Self {
        self.style()
            .backdrop_filter
            .get_or_insert_with(Vec::new)
            .push(Filter::Blur(radius.into()));
        self
    }

    /// Replaces the list of filters applied to the content behind this element, like CSS `backdrop-filter`.
    ///
    /// Any backdrop filters set before this call are discarded. To add one filter to the
    /// chain, use a convenience setter such as [`Styled::backdrop_blur`].
    fn backdrop_filter(mut self, filters: impl Into<Vec<Filter>>) -> Self {
        self.style().backdrop_filter = Some(filters.into());
        self
    }

    /// Sets the display type of this element.
    ///
    /// The display type chooses the layout algorithm for the element's children. See
    /// [`Display`] for the values.
    fn display(mut self, display: Display) -> Self {
        self.style().display = Some(display);
        self
    }

    /// Sets the display type of this element to `block`.
    ///
    /// [Docs](https://tailwindcss.com/docs/display)
    fn block(self) -> Self {
        self.display(Display::Block)
    }

    /// Sets the display type of this element to `flex`.
    ///
    /// Its children lay out in a row by default. Use [`Styled::flex_col`] for a column.
    ///
    /// [Docs](https://tailwindcss.com/docs/display)
    fn flex(self) -> Self {
        self.display(Display::Flex)
    }

    /// Sets the display type of this element to `grid`.
    ///
    /// Children are placed in rows and columns. Choose the column count with
    /// [`Styled::grid_cols`].
    ///
    /// [Docs](https://tailwindcss.com/docs/display)
    fn grid(self) -> Self {
        self.display(Display::Grid)
    }

    /// Places this element in its parent's inline formatting context as an inline box.
    ///
    /// Its content participates in the surrounding line layout instead of creating a block box;
    /// whitespace and line breaks still come from the content and its surrounding layout.
    fn inline(self) -> Self {
        self.display(Display::Inline)
    }

    /// Places this element in its parent's paragraph as one atomic box with flex layout inside.
    fn inline_flex(self) -> Self {
        self.display(Display::InlineFlex)
    }

    /// Sets how this element aligns when it is placed in an inline layout.
    ///
    /// Use it for elements that take part in text flow, such as inline boxes inside a
    /// paragraph. For flex and grid alignment, use [`Styled::items_center`] or
    /// [`Styled::self_center`].
    fn vertical_align(mut self, align: VerticalAlign) -> Self {
        self.style().vertical_align = Some(align);
        self
    }

    /// Aligns this inline box with the surrounding text baseline.
    fn align_baseline(self) -> Self {
        self.vertical_align(VerticalAlign::Baseline)
    }

    /// Aligns this inline box's midpoint with the parent baseline plus half its font's x-height.
    ///
    /// This is CSS `vertical-align: middle`. For flex or grid centering, use
    /// [`Styled::self_center`] on the item or [`Styled::items_center`] on its container.
    fn align_middle(self) -> Self {
        self.vertical_align(VerticalAlign::Middle)
    }

    /// Aligns this inline box with the top of its line.
    fn align_top(self) -> Self {
        self.vertical_align(VerticalAlign::Top)
    }

    /// Aligns this inline box with the bottom of its line.
    fn align_bottom(self) -> Self {
        self.vertical_align(VerticalAlign::Bottom)
    }

    /// Sets the display type of this element to `none`, which removes it from layout and painting.
    ///
    /// Its children are removed too. Use [`Styled::invisible`] to keep the space the
    /// element would take.
    ///
    /// [Docs](https://tailwindcss.com/docs/display)
    fn hidden(self) -> Self {
        self.display(Display::None)
    }

    /// Sets the space reserved for the scrollbar, in logical pixels or rems.
    ///
    /// The space affects layout only when the element's overflow is [`Overflow::Scroll`](crate::Overflow::Scroll).
    fn scrollbar_width(mut self, width: impl Into<AbsoluteLength>) -> Self {
        self.style().scrollbar_width = Some(width.into());
        self
    }

    /// Fades the content clipped by this element's overflow to transparent at each edge.
    ///
    /// Each value is the distance over which the content fades as it approaches the
    /// corresponding clipped edge, similar to CSS `mask-image: linear-gradient(...)`.
    /// Use [`Edges`] to set the four edges separately.
    fn overflow_fade(mut self, fade: impl Into<Edges<AbsoluteLength>>) -> Self {
        let fade = fade.into();
        let overflow_fade = &mut self.style().overflow_fade;
        overflow_fade.top = Some(fade.top);
        overflow_fade.right = Some(fade.right);
        overflow_fade.bottom = Some(fade.bottom);
        overflow_fade.left = Some(fade.left);
        self
    }

    /// Fades content at the left and right overflow edges by the same distance.
    ///
    /// This clears any fade set on the top and bottom edges.
    fn overflow_fade_x(self, fade: impl Into<AbsoluteLength>) -> Self {
        self.overflow_fade(Edges::horizontal(fade.into()))
    }

    /// Fades content at the top and bottom overflow edges by the same distance.
    ///
    /// This clears any fade set on the left and right edges.
    fn overflow_fade_y(self, fade: impl Into<AbsoluteLength>) -> Self {
        self.overflow_fade(Edges::vertical(fade.into()))
    }

    /// Sets how this element's text handles whitespace and wrapping.
    fn whitespace(mut self, white_space: WhiteSpace) -> Self {
        self.text_style().white_space = Some(white_space);
        self
    }

    /// Sets the whitespace of this element's text to `normal`, so text wraps at the available width.
    ///
    /// [Docs](https://tailwindcss.com/docs/whitespace#normal)
    fn whitespace_normal(mut self) -> Self {
        self.text_style().white_space = Some(WhiteSpace::Normal);
        self
    }

    /// Sets the whitespace of this element's text to `nowrap`, so text stays on one line.
    ///
    /// [Docs](https://tailwindcss.com/docs/whitespace#nowrap)
    fn whitespace_nowrap(mut self) -> Self {
        self.text_style().white_space = Some(WhiteSpace::Nowrap);
        self
    }

    /// Truncates overflowing text with an ellipsis (…) at the end.
    ///
    /// This sets the overflow mode; pair it with [`Styled::overflow_hidden`] and
    /// [`Styled::whitespace_nowrap`] when you want a single clipped line. [`Styled::truncate`]
    /// applies all three settings.
    ///
    /// [Docs](https://tailwindcss.com/docs/text-overflow#ellipsis)
    fn text_ellipsis(mut self) -> Self {
        self.text_style().text_overflow = Some(TextOverflow::Truncate(ELLIPSIS));
        self
    }

    /// Truncates overflowing text with an ellipsis (…) at the start.
    ///
    /// This sets the overflow mode; pair it with [`Styled::overflow_hidden`] and
    /// [`Styled::whitespace_nowrap`] when you want a single clipped line. Use it for file paths,
    /// where the end matters more than the beginning.
    /// This doesn't exist in Tailwind CSS.
    fn text_ellipsis_start(mut self) -> Self {
        self.text_style().text_overflow = Some(TextOverflow::TruncateStart(ELLIPSIS));
        self
    }

    /// Truncates overflowing text with an ellipsis (…) in the middle.
    ///
    /// The beginning and end of the text stay visible. Pair it with
    /// [`Styled::overflow_hidden`] and [`Styled::whitespace_nowrap`] for a single clipped line.
    /// It is useful for file names.
    /// This doesn't exist in Tailwind CSS.
    fn text_ellipsis_middle(mut self) -> Self {
        self.text_style().text_overflow = Some(TextOverflow::TruncateMiddle(ELLIPSIS));
        self
    }

    /// Sets how this element's text is truncated when it overflows.
    ///
    /// See [`TextOverflow`] for the values.
    fn text_overflow(mut self, overflow: TextOverflow) -> Self {
        self.text_style().text_overflow = Some(overflow);
        self
    }

    /// Sets the horizontal alignment of this element's text.
    fn text_align(mut self, align: TextAlign) -> Self {
        self.text_style().text_align = Some(align);
        self
    }

    /// Aligns text to the start edge of each line, which is the left edge in left-to-right text.
    fn text_start(self) -> Self {
        self.text_align(TextAlign::Start)
    }

    /// Aligns text to the end edge of each line, which is the right edge in left-to-right text.
    fn text_end(self) -> Self {
        self.text_align(TextAlign::End)
    }

    /// Aligns text to the left edge.
    fn text_left(mut self) -> Self {
        self.text_align(TextAlign::Left)
    }

    /// Centers text horizontally.
    fn text_center(mut self) -> Self {
        self.text_align(TextAlign::Center)
    }

    /// Aligns text to the right edge.
    fn text_right(mut self) -> Self {
        self.text_align(TextAlign::Right)
    }

    /// Sets the space between letters in this element and its children, in logical pixels.
    fn letter_spacing(mut self, spacing: impl Into<Pixels>) -> Self {
        self.text_style().letter_spacing = Some(spacing.into());
        self
    }

    /// Sets the case transformation for text in this element and its children.
    fn text_transform(mut self, transform: TextTransform) -> Self {
        self.text_style().text_transform = Some(transform);
        self
    }

    /// Prevents text from wrapping and truncates overflowing text with an ellipsis (…) at the end.
    ///
    /// This sets [`Styled::overflow_hidden`], [`Styled::whitespace_nowrap`] and
    /// [`Styled::text_ellipsis`].
    ///
    /// [Docs](https://tailwindcss.com/docs/text-overflow#truncate)
    fn truncate(mut self) -> Self {
        self.overflow_hidden().whitespace_nowrap().text_ellipsis()
    }

    /// Shows at most `lines` lines of text, then truncates the rest.
    ///
    /// This also sets [`Styled::overflow_hidden`].
    ///
    /// [Docs](https://tailwindcss.com/docs/line-clamp)
    fn line_clamp(mut self, lines: usize) -> Self {
        let mut text_style = self.text_style();
        text_style.line_clamp = Some(lines);
        self.overflow_hidden()
    }

    /// Sets the direction in which flex children are placed.
    ///
    /// See [`FlexDirection`] for the values. [Docs](https://tailwindcss.com/docs/flex-direction)
    fn flex_direction(mut self, direction: FlexDirection) -> Self {
        self.style().flex_direction = Some(direction);
        self
    }

    /// Sets the flex direction to `column`, so children stack from top to bottom.
    ///
    /// [Docs](https://tailwindcss.com/docs/flex-direction#column)
    fn flex_col(mut self) -> Self {
        self.style().flex_direction = Some(FlexDirection::Column);
        self
    }

    /// Sets the flex direction to `column-reverse`, so children stack from bottom to top.
    ///
    /// [Docs](https://tailwindcss.com/docs/flex-direction#column-reverse)
    fn flex_col_reverse(mut self) -> Self {
        self.style().flex_direction = Some(FlexDirection::ColumnReverse);
        self
    }

    /// Sets the flex direction to `row`, so children sit left to right.
    ///
    /// [Docs](https://tailwindcss.com/docs/flex-direction#row)
    fn flex_row(mut self) -> Self {
        self.style().flex_direction = Some(FlexDirection::Row);
        self
    }

    /// Sets the flex direction to `row-reverse`, so children sit right to left.
    ///
    /// [Docs](https://tailwindcss.com/docs/flex-direction#row-reverse)
    fn flex_row_reverse(mut self) -> Self {
        self.style().flex_direction = Some(FlexDirection::RowReverse);
        self
    }

    /// Lets a flex child grow and shrink to fill the space, ignoring its initial size.
    ///
    /// This sets `flex: 1 1 0%`: grow 1, shrink 1, basis 0. Use it on the child that
    /// should take up the remaining space.
    ///
    /// [Docs](https://tailwindcss.com/docs/flex#flex-1)
    fn flex_1(mut self) -> Self {
        self.style().flex_grow = Some(1.);
        self.style().flex_shrink = Some(1.);
        self.style().flex_basis = Some(relative(0.).into());
        self
    }

    /// Lets a flex child grow and shrink, starting from its natural size.
    ///
    /// This sets `flex: 1 1 auto`.
    ///
    /// [Docs](https://tailwindcss.com/docs/flex#auto)
    fn flex_auto(mut self) -> Self {
        self.style().flex_grow = Some(1.);
        self.style().flex_shrink = Some(1.);
        self.style().flex_basis = Some(Length::Auto);
        self
    }

    /// Lets a flex child shrink but not grow, starting from its natural size.
    ///
    /// This sets `flex: 0 1 auto`.
    ///
    /// [Docs](https://tailwindcss.com/docs/flex#initial)
    fn flex_initial(mut self) -> Self {
        self.style().flex_grow = Some(0.);
        self.style().flex_shrink = Some(1.);
        self.style().flex_basis = Some(Length::Auto);
        self
    }

    /// Stops a flex child from growing or shrinking.
    ///
    /// This sets `flex: none`, which keeps the child at its natural size.
    ///
    /// [Docs](https://tailwindcss.com/docs/flex#none)
    fn flex_none(mut self) -> Self {
        self.style().flex_grow = Some(0.);
        self.style().flex_shrink = Some(0.);
        self.style().flex_basis = Some(Length::Auto);
        self
    }

    /// Sets the starting size of a flex child before it grows or shrinks.
    ///
    /// [Docs](https://tailwindcss.com/docs/flex-basis)
    fn flex_basis(mut self, basis: impl Into<Length>) -> Self {
        self.style().flex_basis = Some(basis.into());
        self
    }

    /// Sets the factor by which a flex child grows to fill spare space.
    ///
    /// A child with a factor of 2 takes twice the spare space of a child with 1.
    ///
    /// [Docs](https://tailwindcss.com/docs/flex-grow)
    fn flex_grow(mut self, grow: f32) -> Self {
        self.style().flex_grow = Some(grow);
        self
    }

    /// Stops a flex child from growing.
    ///
    /// [Docs](https://tailwindcss.com/docs/flex-grow#dont-grow)
    fn flex_grow_0(mut self) -> Self {
        self.style().flex_grow = Some(0.);
        self
    }

    /// Lets a flex child grow with a factor of 1.
    ///
    /// [Docs](https://tailwindcss.com/docs/flex-grow#grow-1)
    fn flex_grow_1(mut self) -> Self {
        self.style().flex_grow = Some(1.);
        self
    }

    /// Sets the factor by which a flex child shrinks when space is short.
    ///
    /// [Docs](https://tailwindcss.com/docs/flex-shrink)
    fn flex_shrink(mut self, shrink: f32) -> Self {
        self.style().flex_shrink = Some(shrink);
        self
    }

    /// Prevents this flex child from participating in shrinkage when space is short.
    ///
    /// Other constraints, including the child's minimum size, still affect its final size. This
    /// is the usual fix for a sidebar that gets squeezed by its content area.
    ///
    /// [Docs](https://tailwindcss.com/docs/flex-shrink#dont-shrink)
    fn flex_shrink_0(mut self) -> Self {
        self.style().flex_shrink = Some(0.);
        self
    }

    /// Lets a flex child shrink with a factor of 1.
    ///
    /// [Docs](https://tailwindcss.com/docs/flex-shrink#shrink-1)
    fn flex_shrink_1(mut self) -> Self {
        self.style().flex_shrink = Some(1.);
        self
    }

    /// Lets flex children wrap onto new lines in the normal direction.
    ///
    /// [Docs](https://tailwindcss.com/docs/flex-wrap#wrap-normally)
    fn flex_wrap(mut self) -> Self {
        self.style().flex_wrap = Some(FlexWrap::Wrap);
        self
    }

    /// Lets flex children wrap onto new lines in the reverse direction.
    ///
    /// [Docs](https://tailwindcss.com/docs/flex-wrap#wrap-reversed)
    fn flex_wrap_reverse(mut self) -> Self {
        self.style().flex_wrap = Some(FlexWrap::WrapReverse);
        self
    }

    /// Stops flex children from wrapping. Inflexible children overflow the container instead.
    ///
    /// [Docs](https://tailwindcss.com/docs/flex-wrap#dont-wrap)
    fn flex_nowrap(mut self) -> Self {
        self.style().flex_wrap = Some(FlexWrap::NoWrap);
        self
    }

    /// Aligns flex or grid children to the start of the cross axis.
    ///
    /// [Docs](https://tailwindcss.com/docs/align-items#start)
    fn items_start(mut self) -> Self {
        self.style().align_items = Some(AlignItems::FlexStart);
        self
    }

    /// Aligns flex or grid children to the end of the cross axis.
    ///
    /// [Docs](https://tailwindcss.com/docs/align-items#end)
    fn items_end(mut self) -> Self {
        self.style().align_items = Some(AlignItems::FlexEnd);
        self
    }

    /// Centers flex or grid children along the cross axis.
    ///
    /// [Docs](https://tailwindcss.com/docs/align-items#center)
    fn items_center(mut self) -> Self {
        self.style().align_items = Some(AlignItems::Center);
        self
    }

    /// Aligns flex or grid children on their text baselines along the cross axis.
    ///
    /// [Docs](https://tailwindcss.com/docs/align-items#baseline)
    fn items_baseline(mut self) -> Self {
        self.style().align_items = Some(AlignItems::Baseline);
        self
    }

    /// Stretches flex or grid children to fill the cross axis.
    ///
    /// [Docs](https://tailwindcss.com/docs/align-items#stretch)
    fn items_stretch(mut self) -> Self {
        self.style().align_items = Some(AlignItems::Stretch);
        self
    }

    /// Aligns this flex or grid child to the start of the container's cross axis.
    ///
    /// [Docs](https://tailwindcss.com/docs/align-self#start)
    fn self_start(mut self) -> Self {
        self.style().align_self = Some(AlignSelf::Start);
        self
    }

    /// Aligns this flex or grid child to the end of the container's cross axis.
    ///
    /// [Docs](https://tailwindcss.com/docs/align-self#end)
    fn self_end(mut self) -> Self {
        self.style().align_self = Some(AlignSelf::End);
        self
    }

    /// Aligns this flex child to the flex-relative start of the cross axis.
    ///
    /// [Docs](https://tailwindcss.com/docs/align-self#start)
    fn self_flex_start(mut self) -> Self {
        self.style().align_self = Some(AlignSelf::FlexStart);
        self
    }

    /// Aligns this flex child to the flex-relative end of the cross axis.
    ///
    /// [Docs](https://tailwindcss.com/docs/align-self#end)
    fn self_flex_end(mut self) -> Self {
        self.style().align_self = Some(AlignSelf::FlexEnd);
        self
    }

    /// Centers this flex or grid child along the container's cross axis.
    ///
    /// [Docs](https://tailwindcss.com/docs/align-self#center)
    fn self_center(mut self) -> Self {
        self.style().align_self = Some(AlignSelf::Center);
        self
    }

    /// Aligns this flex or grid child on the container's baseline along the cross axis.
    ///
    /// [Docs](https://tailwindcss.com/docs/align-self#baseline)
    fn self_baseline(mut self) -> Self {
        self.style().align_self = Some(AlignSelf::Baseline);
        self
    }

    /// Stretches this flex or grid child to fill the container's cross axis.
    ///
    /// [Docs](https://tailwindcss.com/docs/align-self#stretch)
    fn self_stretch(mut self) -> Self {
        self.style().align_self = Some(AlignSelf::Stretch);
        self
    }

    /// Packs flex or grid children against the start of the main axis.
    ///
    /// [Docs](https://tailwindcss.com/docs/justify-content#start)
    fn justify_start(mut self) -> Self {
        self.style().justify_content = Some(JustifyContent::Start);
        self
    }

    /// Packs flex or grid children against the end of the main axis.
    ///
    /// [Docs](https://tailwindcss.com/docs/justify-content#end)
    fn justify_end(mut self) -> Self {
        self.style().justify_content = Some(JustifyContent::End);
        self
    }

    /// Packs flex children against the flex-relative start of the main axis.
    fn justify_flex_start(mut self) -> Self {
        self.style().justify_content = Some(JustifyContent::FlexStart);
        self
    }

    /// Packs flex children against the flex-relative end of the main axis.
    fn justify_flex_end(mut self) -> Self {
        self.style().justify_content = Some(JustifyContent::FlexEnd);
        self
    }

    /// Centers flex or grid children along the main axis.
    ///
    /// [Docs](https://tailwindcss.com/docs/justify-content#center)
    fn justify_center(mut self) -> Self {
        self.style().justify_content = Some(JustifyContent::Center);
        self
    }

    /// Spreads children along the main axis with equal space between them.
    ///
    /// The first and last children touch the edges.
    ///
    /// [Docs](https://tailwindcss.com/docs/justify-content#space-between)
    fn justify_between(mut self) -> Self {
        self.style().justify_content = Some(JustifyContent::SpaceBetween);
        self
    }

    /// Spreads children along the main axis with equal space on each side of each child.
    ///
    /// The space at the edges is half the space between children.
    ///
    /// [Docs](https://tailwindcss.com/docs/justify-content#space-around)
    fn justify_around(mut self) -> Self {
        self.style().justify_content = Some(JustifyContent::SpaceAround);
        self
    }

    /// Spreads children along the main axis with equal space between every child and at both edges.
    ///
    /// [Docs](https://tailwindcss.com/docs/justify-content#space-evenly)
    fn justify_evenly(mut self) -> Self {
        self.style().justify_content = Some(JustifyContent::SpaceEvenly);
        self
    }

    /// Clears `align-content`, so lines of children use their default packing.
    ///
    /// [Docs](https://tailwindcss.com/docs/align-content#normal)
    fn content_normal(mut self) -> Self {
        self.style().align_content = None;
        self
    }

    /// Packs lines of children in the center of the cross axis.
    ///
    /// [Docs](https://tailwindcss.com/docs/align-content#center)
    fn content_center(mut self) -> Self {
        self.style().align_content = Some(AlignContent::Center);
        self
    }

    /// Packs lines of children against the start of the cross axis.
    ///
    /// [Docs](https://tailwindcss.com/docs/align-content#start)
    fn content_start(mut self) -> Self {
        self.style().align_content = Some(AlignContent::FlexStart);
        self
    }

    /// Packs lines of children against the end of the cross axis.
    ///
    /// [Docs](https://tailwindcss.com/docs/align-content#end)
    fn content_end(mut self) -> Self {
        self.style().align_content = Some(AlignContent::FlexEnd);
        self
    }

    /// Spreads lines of children along the cross axis with equal space between them.
    ///
    /// [Docs](https://tailwindcss.com/docs/align-content#space-between)
    fn content_between(mut self) -> Self {
        self.style().align_content = Some(AlignContent::SpaceBetween);
        self
    }

    /// Spreads lines of children along the cross axis with equal space on each side of each line.
    ///
    /// [Docs](https://tailwindcss.com/docs/align-content#space-around)
    fn content_around(mut self) -> Self {
        self.style().align_content = Some(AlignContent::SpaceAround);
        self
    }

    /// Spreads lines of children along the cross axis with equal space between every line and at both edges.
    ///
    /// [Docs](https://tailwindcss.com/docs/align-content#space-evenly)
    fn content_evenly(mut self) -> Self {
        self.style().align_content = Some(AlignContent::SpaceEvenly);
        self
    }

    /// Stretches lines of children to fill the cross axis.
    ///
    /// [Docs](https://tailwindcss.com/docs/align-content#stretch)
    fn content_stretch(mut self) -> Self {
        self.style().align_content = Some(AlignContent::Stretch);
        self
    }

    /// Sets the width-to-height ratio of this element.
    ///
    /// A ratio of `2.0` makes the element twice as wide as it is tall. The ratio is used
    /// only when one dimension is not set explicitly.
    ///
    /// [Docs](https://tailwindcss.com/docs/aspect-ratio)
    fn aspect_ratio(mut self, ratio: f32) -> Self {
        self.style().aspect_ratio = Some(ratio);
        self
    }

    /// Sets the aspect ratio to `1.0`, so the element's width and height are equal.
    ///
    /// [Docs](https://tailwindcss.com/docs/aspect-ratio)
    fn aspect_square(mut self) -> Self {
        self.style().aspect_ratio = Some(1.0);
        self
    }

    /// Sets the background of this element.
    ///
    /// Accepts any color, such as [`Hsla`](crate::Hsla) or [`Rgba`](crate::Rgba), or a gradient or pattern as a
    /// [`Background`](crate::Background). See [`Fill`] for the full set of accepted values.
    fn bg<F>(mut self, fill: F) -> Self
    where
        F: Into<Fill>,
        Self: Sized,
    {
        self.style().background = Some(fill.into());
        self
    }

    /// Sets the border style to dashed.
    ///
    /// Use [`Styled::border_dashed_length`] and [`Styled::border_dashed_gap`] to tune the
    /// dashes.
    fn border_dashed(mut self) -> Self {
        self.style().border_style = Some(BorderStyle::Dashed);
        self
    }

    /// Sets the length of each border dash as a multiple of the border width.
    ///
    /// Negative values are clamped to `0.0`. For a 2px border, `3.0` gives 6px dashes.
    fn border_dashed_length(mut self, length_per_border_width: f32) -> Self {
        self.style().border_dashed_length = Some(length_per_border_width.max(0.0));

        self
    }

    /// Sets the gap between border dashes as a multiple of the border width.
    ///
    /// Negative values are clamped to `0.0`. For a 2px border, `2.0` gives 4px gaps.
    fn border_dashed_gap(mut self, gap_per_border_width: f32) -> Self {
        self.style().border_dashed_gap = Some(gap_per_border_width.max(0.0));

        self
    }

    /// Returns the text style refinement configured on this element.
    ///
    /// Use it to set a text property that has no dedicated method. The refinement is inherited
    /// by text descendants when GPUI resolves their styles; a descendant's own value takes
    /// precedence.
    fn text_style(&mut self) -> &mut TextStyleRefinement {
        let style: &mut StyleRefinement = self.style();
        &mut style.text
    }

    /// Sets the text color of this element.
    ///
    /// This value cascades to its children, unless a child sets its own color.
    fn text_color(mut self, color: impl IntoColor<Hsla>) -> Self {
        self.text_style().color = Some(color.into_color());
        self
    }

    /// Sets the font weight of this element and its children.
    ///
    /// This value cascades to its children.
    fn font_weight(mut self, weight: FontWeight) -> Self {
        self.text_style().font_weight = Some(weight);
        self
    }

    /// Sets the font width of this element and its children.
    ///
    /// Accepts a percentage or a [`FontWidth`] preset.
    fn font_width(mut self, width: impl Into<FontWidth>) -> Self {
        self.text_style().font_width = Some(width.into());

        self
    }

    /// Sets the background color behind the text of this element.
    ///
    /// This value cascades to its children.
    fn text_bg(mut self, bg: impl IntoColor<Hsla>) -> Self {
        self.text_style().background_color = Some(bg.into_color());
        self
    }

    /// Sets the font size of this element and its children.
    ///
    /// The size is an absolute length: pass [`px`](crate::px) for logical pixels or
    /// [`rems`](crate::rems) to scale with the window's root font size. This value
    /// cascades to its children.
    fn text_size(mut self, size: impl Into<AbsoluteLength>) -> Self {
        self.text_style().font_size = Some(size.into());
        self
    }

    /// Sets the font size to `0.75rem` (12px with the default 16px root size).
    ///
    /// [Docs](https://tailwindcss.com/docs/font-size#setting-the-font-size)
    fn text_xs(mut self) -> Self {
        self.text_style().font_size = Some(rems(0.75).into());
        self
    }

    /// Sets the font size to `0.875rem` (14px with the default 16px root size).
    ///
    /// [Docs](https://tailwindcss.com/docs/font-size#setting-the-font-size)
    fn text_sm(mut self) -> Self {
        self.text_style().font_size = Some(rems(0.875).into());
        self
    }

    /// Sets the font size to `1rem` (16px with the default 16px root size).
    ///
    /// [Docs](https://tailwindcss.com/docs/font-size#setting-the-font-size)
    fn text_base(mut self) -> Self {
        self.text_style().font_size = Some(rems(1.0).into());
        self
    }

    /// Sets the font size to `1.125rem` (18px with the default 16px root size).
    ///
    /// [Docs](https://tailwindcss.com/docs/font-size#setting-the-font-size)
    fn text_lg(mut self) -> Self {
        self.text_style().font_size = Some(rems(1.125).into());
        self
    }

    /// Sets the font size to `1.25rem` (20px with the default 16px root size).
    ///
    /// [Docs](https://tailwindcss.com/docs/font-size#setting-the-font-size)
    fn text_xl(mut self) -> Self {
        self.text_style().font_size = Some(rems(1.25).into());
        self
    }

    /// Sets the font size to `1.5rem` (24px with the default 16px root size).
    ///
    /// [Docs](https://tailwindcss.com/docs/font-size#setting-the-font-size)
    fn text_2xl(mut self) -> Self {
        self.text_style().font_size = Some(rems(1.5).into());
        self
    }

    /// Sets the font size to `1.875rem` (30px with the default 16px root size).
    ///
    /// [Docs](https://tailwindcss.com/docs/font-size#setting-the-font-size)
    fn text_3xl(mut self) -> Self {
        self.text_style().font_size = Some(rems(1.875).into());
        self
    }

    /// Sets the font style to italic.
    ///
    /// [Docs](https://tailwindcss.com/docs/font-style#italicizing-text)
    fn italic(mut self) -> Self {
        self.text_style().font_style = Some(FontStyle::Italic);
        self
    }

    /// Sets the font style to normal, which is not italic.
    ///
    /// [Docs](https://tailwindcss.com/docs/font-style#displaying-text-normally)
    fn not_italic(mut self) -> Self {
        self.text_style().font_style = Some(FontStyle::Normal);
        self
    }

    /// Underlines the text of this element with a 1px line.
    ///
    /// This replaces any underline style set earlier on this element, including its
    /// color, so call [`Styled::text_decoration_color`] after this method.
    /// An underline set here can cascade to text descendants; [`Styled::text_decoration_none`]
    /// only clears one set on the current element.
    ///
    /// [Docs](https://tailwindcss.com/docs/text-decoration-line#underling-text)
    fn underline(mut self) -> Self {
        let style = self.text_style();
        style.underline = Some(UnderlineStyle {
            thickness: px(1.),
            ..Default::default()
        });
        self
    }

    /// Draws a line through the text of this element with a 1px line.
    ///
    /// This replaces any strikethrough set earlier on this element. It doesn't change the
    /// underline.
    ///
    /// [Docs](https://tailwindcss.com/docs/text-decoration-line#adding-a-line-through-text)
    fn line_through(mut self) -> Self {
        let style = self.text_style();
        style.strikethrough = Some(StrikethroughStyle {
            thickness: px(1.),
            ..Default::default()
        });
        self
    }

    /// Removes an underline set earlier on this element. It can't remove an underline
    /// inherited from a parent, and it leaves any strikethrough alone.
    fn text_decoration_none(mut self) -> Self {
        self.text_style().underline = None;
        self
    }

    /// Sets the color of the text decoration line.
    ///
    /// This sets the underline color, not the strikethrough color. If no underline is set,
    /// it creates one with 0px thickness, which draws nothing until you set a thickness.
    /// Call this after [`Styled::underline`], because `underline` replaces the whole underline
    /// record, including its color.
    fn text_decoration_color(mut self, color: impl IntoColor<Hsla>) -> Self {
        let style = self.text_style();
        let underline = style.underline.get_or_insert_with(Default::default);
        underline.color = Some(color.into_color());
        self
    }

    /// Sets the text decoration style to a solid line.
    ///
    /// If no underline exists yet, this creates one with zero thickness; pair it with
    /// [`Styled::underline`] or a thickness method to make the line visible.
    ///
    /// [Docs](https://tailwindcss.com/docs/text-decoration-style)
    fn text_decoration_solid(mut self) -> Self {
        let style = self.text_style();
        let underline = style.underline.get_or_insert_with(Default::default);
        underline.wavy = false;
        self
    }

    /// Sets the text decoration style to a wavy line.
    ///
    /// If no underline exists yet, this creates one with zero thickness; pair it with
    /// [`Styled::underline`] or a thickness method to make the line visible.
    ///
    /// [Docs](https://tailwindcss.com/docs/text-decoration-style)
    fn text_decoration_wavy(mut self) -> Self {
        let style = self.text_style();
        let underline = style.underline.get_or_insert_with(Default::default);
        underline.wavy = true;
        self
    }

    /// Sets the text decoration thickness to `0px`, which hides the line.
    ///
    /// [Docs](https://tailwindcss.com/docs/text-decoration-thickness)
    fn text_decoration_0(mut self) -> Self {
        let style = self.text_style();
        let underline = style.underline.get_or_insert_with(Default::default);
        underline.thickness = px(0.);
        self
    }

    /// Sets the text decoration thickness to `1px`.
    ///
    /// [Docs](https://tailwindcss.com/docs/text-decoration-thickness)
    fn text_decoration_1(mut self) -> Self {
        let style = self.text_style();
        let underline = style.underline.get_or_insert_with(Default::default);
        underline.thickness = px(1.);
        self
    }

    /// Sets the text decoration thickness to `2px`.
    ///
    /// [Docs](https://tailwindcss.com/docs/text-decoration-thickness)
    fn text_decoration_2(mut self) -> Self {
        let style = self.text_style();
        let underline = style.underline.get_or_insert_with(Default::default);
        underline.thickness = px(2.);
        self
    }

    /// Sets the text decoration thickness to `4px`.
    ///
    /// [Docs](https://tailwindcss.com/docs/text-decoration-thickness)
    fn text_decoration_4(mut self) -> Self {
        let style = self.text_style();
        let underline = style.underline.get_or_insert_with(Default::default);
        underline.thickness = px(4.);
        self
    }

    /// Sets the text decoration thickness to `8px`.
    ///
    /// [Docs](https://tailwindcss.com/docs/text-decoration-thickness)
    fn text_decoration_8(mut self) -> Self {
        let style = self.text_style();
        let underline = style.underline.get_or_insert_with(Default::default);
        underline.thickness = px(8.);
        self
    }

    /// Sets the font family of this element and its children.
    ///
    /// Pass a font family name, such as `"Menlo"`. This value cascades to its children.
    fn font_family(mut self, family_name: impl Into<SharedString>) -> Self {
        self.text_style().font_family = Some(family_name.into());
        self
    }

    /// Sets the OpenType font features of this element and its children.
    fn font_features(mut self, features: FontFeatures) -> Self {
        self.text_style().font_features = Some(features);
        self
    }

    /// Sets the font family, features, fallbacks, weight, width, and style of this element and its children.
    ///
    /// Use [`Styled::font_family`] to change only the family.
    fn font(mut self, font: Font) -> Self {
        let Font {
            family,
            features,
            fallbacks,
            weight,
            width,
            style,
        } = font;

        let text_style = self.text_style();
        text_style.font_family = Some(family);
        text_style.font_features = Some(features);
        text_style.font_weight = Some(weight);
        text_style.font_width = Some(width);
        text_style.font_style = Some(style);
        text_style.font_fallbacks = fallbacks;

        self
    }

    /// Sets the line height of this element and its children.
    ///
    /// Pass an absolute length such as [`px`](crate::px) or [`rems`](crate::rems), or a relative
    /// length such as `relative(1.5)` to make each line 1.5 times the element's font size.
    /// Relative values are resolved against the font size, not the parent's layout size.
    fn line_height(mut self, line_height: impl Into<DefiniteLength>) -> Self {
        self.text_style().line_height = Some(line_height.into());
        self
    }

    /// Sets the opacity of this element and its children, composited together as one group.
    ///
    /// `0.0` is fully transparent and `1.0` is fully opaque.
    fn opacity(mut self, opacity: f32) -> Self {
        self.style().opacity = Some(opacity);
        self
    }

    /// Sets the number of equal-width columns in this grid element.
    ///
    /// Columns share the remaining width equally, with a zero minimum track size. A long child
    /// can therefore overflow or be compressed below its min-content width. Use
    /// [`Styled::grid_cols_min_content`] when each column must accommodate its min-content size.
    fn grid_cols(mut self, cols: u16) -> Self {
        self.style().grid_cols = Some(GridTemplate {
            repeat: cols,
            min_size: GridTemplateMinSize::Zero,
        });
        self
    }

    /// Sets the number of columns, each at least as wide as its content.
    ///
    /// Each track has a min-content minimum and then shares remaining space equally. This keeps
    /// unbreakable or otherwise intrinsic content from being compressed below that minimum.
    fn grid_cols_min_content(mut self, cols: u16) -> Self {
        self.style().grid_cols = Some(GridTemplate {
            repeat: cols,
            min_size: GridTemplateMinSize::MinContent,
        });
        self
    }

    /// Sets the number of columns with max-content sizing as the upper track size.
    ///
    /// Each track can grow to its max-content width instead of sharing all remaining space as a
    /// flexible `1fr` track. The minimum remains zero, so a narrow grid can compress or overflow
    /// content, and unused space need not be distributed across the tracks.
    fn grid_cols_max_content(mut self, cols: u16) -> Self {
        self.style().grid_cols = Some(GridTemplate {
            repeat: cols,
            min_size: GridTemplateMinSize::MaxContent,
        });
        self
    }

    /// Sets the number of equal-height rows in this grid element.
    ///
    /// Rows share the remaining height equally, with a zero minimum track size. A tall child can
    /// therefore overflow or be compressed below its min-content height. Use
    /// [`Styled::grid_rows_min_content`] when each row must accommodate that minimum.
    fn grid_rows(mut self, rows: u16) -> Self {
        self.style().grid_rows = Some(GridTemplate {
            repeat: rows,
            min_size: GridTemplateMinSize::Zero,
        });
        self
    }

    /// Sets the number of rows, each at least as tall as its content.
    ///
    /// Each track has a min-content minimum and then shares remaining space equally. This keeps
    /// content from being compressed below its intrinsic minimum height.
    fn grid_rows_min_content(mut self, rows: u16) -> Self {
        self.style().grid_rows = Some(GridTemplate {
            repeat: rows,
            min_size: GridTemplateMinSize::MinContent,
        });
        self
    }

    /// Sets the number of rows with max-content sizing as the upper track size.
    ///
    /// Each track can grow to its max-content height instead of sharing all remaining space as a
    /// flexible `1fr` track. The minimum remains zero, so a short grid can compress or overflow
    /// content, and unused space need not be distributed across the tracks.
    fn grid_rows_max_content(mut self, rows: u16) -> Self {
        self.style().grid_rows = Some(GridTemplate {
            repeat: rows,
            min_size: GridTemplateMinSize::MaxContent,
        });
        self
    }

    /// Sets the grid column line where this element starts. Lines are numbered from 1;
    /// negative values count backward from the end of the explicit grid.
    fn col_start(mut self, start: i16) -> Self {
        let grid_location = self.style().grid_location_mut();
        grid_location.column.start = GridPlacement::Line(start);
        self
    }

    /// Lets the grid place this element's start column automatically.
    fn col_start_auto(mut self) -> Self {
        let grid_location = self.style().grid_location_mut();
        grid_location.column.start = GridPlacement::Auto;
        self
    }

    /// Sets the grid column line where this element ends. Lines are numbered from 1; negative values count from the end.
    fn col_end(mut self, end: i16) -> Self {
        let grid_location = self.style().grid_location_mut();
        grid_location.column.end = GridPlacement::Line(end);
        self
    }

    /// Lets the grid place this element's end column automatically.
    fn col_end_auto(mut self) -> Self {
        let grid_location = self.style().grid_location_mut();
        grid_location.column.end = GridPlacement::Auto;
        self
    }

    /// Makes this element span `span` grid columns.
    fn col_span(mut self, span: u16) -> Self {
        let grid_location = self.style().grid_location_mut();
        grid_location.column = GridPlacement::Span(span)..GridPlacement::Span(span);
        self
    }

    /// Makes this element span all grid columns.
    fn col_span_full(mut self) -> Self {
        let grid_location = self.style().grid_location_mut();
        grid_location.column = GridPlacement::Line(1)..GridPlacement::Line(-1);
        self
    }

    /// Sets the grid row line where this element starts. Lines are numbered from 1;
    /// negative values count backward from the end of the explicit grid.
    fn row_start(mut self, start: i16) -> Self {
        let grid_location = self.style().grid_location_mut();
        grid_location.row.start = GridPlacement::Line(start);
        self
    }

    /// Lets the grid place this element's start row automatically.
    fn row_start_auto(mut self) -> Self {
        let grid_location = self.style().grid_location_mut();
        grid_location.row.start = GridPlacement::Auto;
        self
    }

    /// Sets the grid row line where this element ends. Lines are numbered from 1; negative values count from the end.
    fn row_end(mut self, end: i16) -> Self {
        let grid_location = self.style().grid_location_mut();
        grid_location.row.end = GridPlacement::Line(end);
        self
    }

    /// Lets the grid place this element's end row automatically.
    fn row_end_auto(mut self) -> Self {
        let grid_location = self.style().grid_location_mut();
        grid_location.row.end = GridPlacement::Auto;
        self
    }

    /// Makes this element span `span` grid rows.
    fn row_span(mut self, span: u16) -> Self {
        let grid_location = self.style().grid_location_mut();
        grid_location.row = GridPlacement::Span(span)..GridPlacement::Span(span);
        self
    }

    /// Makes this element span all grid rows.
    fn row_span_full(mut self) -> Self {
        let grid_location = self.style().grid_location_mut();
        grid_location.row = GridPlacement::Line(1)..GridPlacement::Line(-1);
        self
    }

    /// Draws a debug border around this element.
    ///
    /// This method exists only in debug builds (`debug_assertions`).
    #[cfg(debug_assertions)]
    fn debug(mut self) -> Self {
        self.style().debug = Some(true);
        self
    }

    /// Draws a debug border around this element and every element below it in the tree.
    ///
    /// This method exists only in debug builds (`debug_assertions`).
    #[cfg(debug_assertions)]
    fn debug_below(mut self) -> Self {
        self.style().debug_below = Some(true);
        self
    }

    /// Sets how smooth the rounded corners are, from `0.0` for circular corners to `1.0` for maximum smoothing.
    /// In debug builds, values outside this range trigger an assertion.
    fn rounded_smoothing(mut self, amount: f32) -> Self {
        debug_assert!(
            (0.0..=1.0).contains(&amount),
            "corner smoothing must be between 0 and 1"
        );
        self.style().corner_smoothing = Some(amount);
        self
    }

    /// Sets corner smoothing to `0.6`, which matches iOS-style corners.
    fn rounded_smoothing_ios(mut self) -> Self {
        self.style().corner_smoothing = Some(0.6);
        self
    }
}
