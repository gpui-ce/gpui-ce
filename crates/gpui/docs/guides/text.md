# Display and style text

Pass a string to [`ParentElement::child`] and GPUI shapes, wraps, measures, and
paints it using the container's inherited text style. Reach for [`StyledText`]
when byte ranges inside a single paragraph need distinct styling,
[`InteractiveText`] when those ranges should respond to clicks or hover, and
[`WindowTextSystem`] when custom layout or painting code needs glyph metrics
directly.

## Render strings and accessible labels

`IntoElement` is implemented for `&'static str`, `String`, `Cow<'static, str>`,
and [`SharedString`]. Borrowed `&str` slices with shorter lifetimes must be
converted into `String` or `SharedString` because the element tree outlives the
`render` call that builds it:

```rust
# use gpui::*;
let label = div().child("Saved");
let count = div().child(format!("{} files", 3));
```

Bare string children omit an accessibility node, which is right inside a button
or row that already exposes its own accessible label. For standalone copy that
screen readers should announce on its own, use [`text!`] (which derives a stable
compile-time ID from source location, or takes `text!(id = ..., ...)` inside
loops) or [`Text::new`]; [`Text::new_inaccessible`] constructs a `Text` element
without an accessibility node. See the
[accessibility guide](crate::_documentation::guides::accessibility) for details.

## Inherit and override text styles

Text styles cascade down the element tree field by field: a child that sets
`.italic()` keeps the color, size, and weight set on its ancestors:

```rust
# use gpui::*;
let card = div()
    .text_sm()
    .text_color(rgb(0x374151))
    .font_weight(FontWeight::SEMIBOLD)
    .line_height(relative(1.5))
    .child("Quarterly report")
    .child(div().italic().child("Draft, not yet reviewed"));
```

| [`Styled`] method | Controls |
| --- | --- |
| [`text_color`](Styled::text_color) | Foreground glyph color |
| [`text_xs`](Styled::text_xs) .. [`text_3xl`](Styled::text_3xl) | Scaled rem sizes (`text_sm` = `0.875rem`, `text_base` = `1rem`) |
| [`text_size`](Styled::text_size) | Exact size in [`rems`] or [`px`] |
| [`font_weight`](Styled::font_weight), [`italic`](Styled::italic), [`not_italic`](Styled::not_italic) | Weight and posture |
| [`font_family`](Styled::font_family) | Font family name from the font's `name` table |
| [`line_height`](Styled::line_height) | Line box height, e.g. `relative(1.5)` or `px(22.)` |
| [`letter_spacing`](Styled::letter_spacing) | Extra tracking in logical pixels |
| [`text_transform`](Styled::text_transform) | Uppercase, lowercase, or title-case transform |

Scale helpers like `text_sm` store [`rems`], so they respond to
[`Window::set_rem_size`]; pass `text_size(px(...))` when a label must stay at a
fixed pixel size regardless of root scaling.

## Decorations, alignment, wrapping, and truncation

[`Styled::underline`] and [`Styled::line_through`] configure independent `1px`
decorations. Call [`Styled::text_decoration_color`] *after* `underline`, because
`underline()` replaces the entire underline record (including any previously set
color). [`Styled::text_decoration_wavy`] and [`Styled::text_decoration_solid`]
set line shape without setting thickness, so pair them with `underline()` or
`text_decoration_1()`. [`Styled::text_decoration_none`] clears an underline set
on the same element; it does not cancel an underline inherited from an ancestor.

```rust
# use gpui::*;
let terms = div().underline().text_decoration_color(rgb(0x2563eb)).child("Terms");
let heading = div().w_full().text_center().child("Settings");
```

For horizontal alignment, [`Styled::text_left`] and [`Styled::text_right`] pick
physical edges, [`Styled::text_start`] and [`Styled::text_end`] follow the
resolved writing direction, and [`Styled::text_center`] centers each line.

Text wraps at container width by default:

* [`Styled::truncate`] disables wrapping, hides overflow, and appends `…`
  (shorthand for `overflow_hidden().whitespace_nowrap().text_ellipsis()`).
* [`Styled::text_ellipsis_start`] and [`Styled::text_ellipsis_middle`] keep the
  tail or both ends of paths and identifiers when paired with
  `overflow_hidden().whitespace_nowrap()`. [`Styled::text_overflow`] accepts a
  custom [`TextOverflow`] marker.
* [`Styled::line_clamp`] caps soft wrapping at `n` lines per `\n`-delimited
  paragraph; combine it with `text_ellipsis()` to append `…` at the cut:

```rust
# use gpui::*;
let title = div().w(px(240.)).truncate().child("A long document title");

let summary = div()
    .w(px(240.))
    .line_clamp(2)
    .text_ellipsis()
    .child("A paragraph long enough to need more than two lines of space.");
```

## Rich spans and clickable ranges

[`StyledText`] shapes a single string while applying [`HighlightStyle`]
overrides to sorted, non-overlapping **UTF-8 byte ranges** that start and end on
`char` boundaries:

```rust
# use gpui::*;
# struct Demo;
# impl Render for Demo {
#     fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
let message = StyledText::new("Save the file before quitting.").with_highlights([
    (
        0..4,
        HighlightStyle {
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        },
    ),
    (21..29, HighlightStyle::from(rgb(0xdc2626))),
]);

div().text_base().child(message)
#     }
# }
# let mut app = TestApp::new();
# app.open_window(|_, _| Demo);
```

[`StyledText::with_highlights`] resolves runs against the inherited container
style during layout; [`StyledText::with_default_highlights`] resolves against an
explicit [`TextStyle`] immediately, and [`StyledText::with_runs`] accepts raw
[`TextRun`]s whose byte lengths sum to `text.len()`.
[`StyledText::with_font_family_overrides`] swaps the font family on runs fully
contained within sorted byte ranges (include highlight or run boundaries at the
override endpoints so runs split there).

Wrap a [`StyledText`] in [`InteractiveText`] with a sibling-unique [`ElementId`]
to make byte ranges clickable or hoverable:

```rust
# use gpui::*;
# struct Demo;
# impl Render for Demo {
#     fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
let text = "Read the docs or file an issue.";
let links = vec![5..13, 25..30];

let styled = StyledText::new(text).with_highlights(links.iter().map(|range| {
    (
        range.clone(),
        HighlightStyle {
            underline: Some(UnderlineStyle { thickness: px(1.), ..Default::default() }),
            ..HighlightStyle::from(rgb(0x2563eb))
        },
    )
}));

InteractiveText::new("links", styled).on_click(links, |link_index, _window, _cx| {
    // `link_index` is the index into `links` (`0` for "the docs", `1` for "issue").
    let _ = link_index;
})
#     }
# }
# let mut app = TestApp::new();
# app.open_window(|_, _| Demo);
```

Notice the callback units: `on_click` passes the **index into the `ranges`
vec** where both mouse-down and mouse-up landed, whereas
[`InteractiveText::on_hover`] and [`InteractiveText::tooltip`] pass the **UTF-8
byte offset** of the grapheme cluster under the pointer (`Option<usize>` on
hover).

## Custom fonts

Register `.ttf` or `.otf` byte buffers on [`App::text_system`] via
[`TextSystem::add_fonts`] before opening windows, then reference the embedded
family name (from the font's `name` table, not its filename) with
[`Styled::font_family`]:

```rust,no_run
# use gpui::*;
# use std::borrow::Cow;
fn load_fonts(cx: &mut App) -> anyhow::Result<()> {
    let regular = std::fs::read("assets/fonts/Inter-Regular.ttf")?;
    let bold = std::fs::read("assets/fonts/Inter-Bold.ttf")?;
    cx.text_system().add_fonts(vec![Cow::Owned(regular), Cow::Owned(bold)])
}
```

Registering fonts after windows have rendered still works and invalidates cached
layouts, at the cost of one frame drawn with fallback glyphs. On web targets the
font database starts empty apart from `.SystemUIFont`, so bundle and register
every font your UI uses at startup. [`TextSystem::all_font_names`] lists all
resolvable families.

## Measure text and hit-test clusters

To measure text manually, resolve [`Window::text_style`] (which reflects the
ancestor style stack during layout/paint, or defaults outside a frame), build
[`TextRun`]s covering `text.len()` bytes, and call
[`WindowTextSystem::shape_line`] for single-line strings (no `\n`) or
[`WindowTextSystem::shape_text`] for wrapped paragraphs:

```rust
# use gpui::*;
# struct Demo;
# impl Render for Demo {
#     fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement { div() }
# }
# let mut app = TestApp::new();
# let mut window = app.open_window(|_, _| Demo);
window.update(|_, window, _cx| {
    let text = "A paragraph that will wrap onto several lines when it is narrow.";
    let style = window.text_style();
    let font_size = style.font_size.to_pixels(window.rem_size());
    let line_height = window.line_height();
    let runs = [style.to_run(text.len())];
    let shaped = window
        .text_system()
        .shape_text(text, font_size, &runs, Some(px(160.)), None)
        .expect("text shapes");
    assert_eq!(shaped.size(line_height).height, line_height * shaped.line_count());
});
```

Window text layouts are cached by text, font size, run metrics, and wrap width;
color and decoration changes reuse cached glyph positions. For a rendered
[`StyledText`], clone its [`StyledText::layout`] handle and query it in event
handlers after layout/prepaint:

* [`TextLayout::index_for_position`] maps a window-coordinate point to a UTF-8
  byte offset (`Ok(ix)` inside a glyph cluster, `Err(ix)` for the nearest
  offset outside the text bounds).
* [`TextLayout::position_for_index`] maps a UTF-8 byte offset back to the
  caret's top-left window coordinate, snapping mid-cluster bytes to the start of
  their grapheme cluster.

## Bidirectional text

Strings stay in logical UTF-8 byte order while the Unicode bidirectional
algorithm reorders glyphs visually, so hit-testing through [`TextLayout`]
always hands back logical byte offsets ready for slicing or highlight ranges.
[`Styled::rtl`] and [`Styled::ltr`] set inherited paragraph direction;
[`Styled::unicode_bidi`] adds explicit isolate, override, or `Plaintext`
(first-strong per-paragraph) behavior when mixing scripts.

## See also

* [Text example](crate::_documentation::examples::text) — type scale, weights,
  decorations, wrapping, and highlighted runs (`cargo run --example text`).
* [Accept text input](crate::_documentation::guides::text_input) — editable
  fields and UTF-16 platform IME conversion.
* [Layout and style](crate::_documentation::concepts::layout_and_style) — how
  inherited styles and `rems` resolve.

[`text!`]: crate::text
