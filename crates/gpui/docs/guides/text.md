# Display and style text

A label is usually just a child: `div().child("Saved")` lays out and paints
the string. The container supplies its font and color; the text wraps to the
available width and contributes its measured size to the parent layout.

## Show a string

The direct string child is a string literal (`&'static str`). For dynamic text,
pass an owned `String` or a [`SharedString`] to [`ParentElement::child`]. A
borrowed `&str` with a shorter lifetime cannot be retained as an element, so
copy it into an owned string first:

```rust
# use gpui::*;
let label = div().child("Saved");
let count = div().child(format!("{} files", 3));
```

Plain string children have no accessibility node, which suits a button whose
parent supplies the accessible name. Give standalone text its own node with
[`text!`], keeping the generated ID and its ancestors stable across frames.
One macro invocation always generates the same ID; in a loop, choose an ID for
each item with `text!(id = ..., ...)`. The
[accessibility guide](crate::_documentation::guides::accessibility) explains
how those IDs form the node's identity.

## Set a base style, then override it

Set shared properties on the container and refine them on a child. Here both
lines inherit the report card's small, semibold gray style, while the draft
line adds italics:

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

These [`Styled`] methods work on either the container or the child being
refined:

| Method | Sets |
| --- | --- |
| [`Styled::text_color`] | The color of the text |
| [`Styled::text_xs`] to [`Styled::text_3xl`] | A size from the scale (`text_sm` is `0.875rem`, `text_base` is `1rem`) |
| [`Styled::text_size`] | An exact size, such as `px(15.)` or a `rems` value |
| [`Styled::font_weight`] | A [`FontWeight`] such as `FontWeight::BOLD` |
| [`Styled::italic`] and [`Styled::not_italic`] | The font style |
| [`Styled::font_family`] | A font family by name |
| [`Styled::line_height`] | A [`DefiniteLength`], such as `relative(1.5)` or `px(22.)` |
| [`Styled::letter_spacing`] | Extra spacing in logical pixels |
| [`Styled::text_transform`] | Uppercase or lowercase rendering |

Sizes such as `text_sm` follow the window's root rem size. With the default
16px root, `0.875rem` is 14px; changing the root changes that label too. An
explicit `text_size(rems(...))` follows the same rule. Choose `text_size(px(...))`
when the size should stay fixed as the rem scale changes.

## Decorate and align

[`Styled::underline`] and [`Styled::line_through`] add a 1px line. The link
below also sets a blue decoration color; the price gets a strike-through:

```rust
# use gpui::*;
let row = div()
    .child(div().underline().text_decoration_color(rgb(0x2563eb)).child("Terms"))
    .child(div().line_through().child("Old price"));
```

[`Styled::text_decoration_wavy`] and [`Styled::text_decoration_solid`] change
the underline's shape. Call decoration setters after `underline`: `underline`
replaces the entire underline record, including its color.
[`Styled::text_decoration_none`] clears an underline previously set on the
same element; an inherited underline still applies. If some children must
remain plain, put the underline on the children that need it rather than on
their shared parent.

Use [`Styled::text_left`] and [`Styled::text_right`] for physical edges. The
logical [`Styled::text_start`] and [`Styled::text_end`] swap sides in a
right-to-left block, while [`Styled::text_center`] is direction-independent:

```rust
# use gpui::*;
let heading = div().w_full().text_center().child("Settings");
```

## Wrap, truncate and clamp

A narrow label wraps by default. Choose a line policy and, when text is hidden,
whether it should be marked with an ellipsis:

- [`Styled::truncate`] turns off wrapping, hides overflow and puts an ellipsis
  at the end. Use it for a single line, such as a title in a toolbar.
- [`Styled::whitespace_nowrap`] turns off wrapping without any ellipsis.
  [`Styled::whitespace_normal`] turns wrapping back on.
- [`Styled::line_clamp`] shows at most `n` lines and drops the rest. It hides
  overflow but adds no ellipsis by itself. Add [`Styled::text_ellipsis`] to put
  an ellipsis at the end of the last visible line.

```rust
# use gpui::*;
let title = div().w(px(240.)).truncate().child("A long document title");

let summary = div()
    .w(px(240.))
    .line_clamp(2)
    .text_ellipsis()
    .child("A paragraph long enough to need more than two lines of space.");
```

For paths, [`Styled::text_ellipsis_start`] keeps the end; for names where both
ends matter, [`Styled::text_ellipsis_middle`] keeps both. Other policies go
through [`Styled::text_overflow`] and a [`TextOverflow`] value.

## Mix styles in one paragraph

Keep a paragraph as one [`StyledText`] when only ranges need different styles.
In this example, the first four bytes become bold and bytes 21–28 red. Each
[`HighlightStyle`] supplies only its overrides; the paragraph and ancestor
styles fill in everything else:

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

The ASCII example makes byte offsets look like character counts. They diverge
for other text: `é` occupies two UTF-8 bytes, so the next character starts at
byte 2, not 1. Supply sorted, non-overlapping byte ranges whose endpoints are
Rust `char` boundaries inside the string. Convert character, grapheme or
UTF-16 offsets first. Debug builds check bounds; invalid ordering can produce
a wrong layout or panic even in release builds. Uncovered bytes retain the
inherited style.

Shaping adds another boundary: a base character and combining mark, or several
emoji code points, can form one visual cluster. Caret placement, selection and
hit testing must use the layout's cluster-aware APIs rather than inventing a
byte offset. See [Measure text](#measure-text) for those mappings.

[`StyledText::with_default_highlights`] supplies a base [`TextStyle`],
[`StyledText::with_runs`] supplies all [`TextRun`]s, and
[`StyledText::with_font_family_overrides`] changes a range's family without
constructing runs. The override ranges must be sorted, non-overlapping and on
`char` boundaries. They affect only runs wholly inside a range; the method
does not split a run at either endpoint. A partial override therefore needs
matching run boundaries from highlights or `with_runs`.

## Make ranges clickable

[`InteractiveText`] makes selected [`StyledText`] ranges clickable. Give it an
[`ElementId`], the byte ranges and a listener. A press and release in a range
passes that range's index in the list. Reuse the same list for styling and hit
testing, as the example does:

```rust
# use gpui::*;
# struct Demo;
# impl Render for Demo {
#     fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
let text = "Read the docs or file an issue.";
let links = vec![5..13, 25..30];

let link_style = |_range: &std::ops::Range<usize>| HighlightStyle {
    underline: Some(UnderlineStyle {
        thickness: px(1.),
        ..Default::default()
    }),
    ..HighlightStyle::from(rgb(0x2563eb))
};

let styled = StyledText::new(text).with_highlights(
    links
        .iter()
        .map(|range| (range.clone(), link_style(range))),
);

InteractiveText::new("links", styled).on_click(links, |link_index, _window, _cx| {
    // `link_index` is 0 for "the docs" and 1 for "issue".
    let _ = link_index;
})
#     }
# }
# let mut app = TestApp::new();
# app.open_window(|_, _| Demo);
```

The click callback receives `0` or `1`, not a string offset. Hover and tooltip
callbacks use a different unit: [`InteractiveText::on_hover`] reports the
UTF-8 byte index at the logical start of the cluster under the pointer, or
`None` when the pointer is inside the element but outside the text. It does
not fire when the pointer leaves the element. [`InteractiveText::tooltip`]
passes that byte index to a builder that may return an [`AnyView`].

Tie ranges to the string used to calculate them, and keep the element ID unique
among siblings. If the string changes, stale ranges can put a click target on
different text from the displayed link.

## Load your own fonts

Register bundled font bytes through [`App::text_system`] and
[`TextSystem::add_fonts`] before the first layout requests the family. A
fallback shaped first can enter the layout cache. Pass one byte buffer per
font file:

```rust,no_run
# use gpui::*;
# use std::borrow::Cow;
fn load_fonts(cx: &mut App) -> anyhow::Result<()> {
    let regular = std::fs::read("assets/fonts/Inter-Regular.ttf")?;
    let bold = std::fs::read("assets/fonts/Inter-Bold.ttf")?;
    cx.text_system().add_fonts(vec![Cow::Owned(regular), Cow::Owned(bold)])
}
```

Select the family with [`Styled::font_family`] after loading. Use the family
name from the font's name table, not its filename. A missing face or glyph can
still trigger fallback through the platform text backend.

```rust
# use gpui::*;
let note = div().font_family("Inter").child("Set in the font you loaded");
```

If a file cannot be parsed, `add_fonts` returns an error; later calls can add
other fonts. [`TextSystem::all_font_names`] reports the family names the
platform currently knows.

## Measure text

To reserve width before painting, shape the same text and style that painting
will use. [`Window::text_system`] needs runs covering the string, and
[`Window::text_style`] and [`TextStyle::to_run`] inherit the tree style and use
the string's byte length:

```rust
# use gpui::*;
# struct Demo;
# impl Render for Demo {
#     fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
#         div()
#     }
# }
# let mut app = TestApp::new();
# let mut window = app.open_window(|_, _| Demo);
window.update(|_, window, _cx| {
    let text = "Measure me";
    let style = window.text_style();
    let runs = [style.to_run(text.len())];
    let line = window
        .text_system()
        .shape_line(text.into(), style.font_size.to_pixels(window.rem_size()), &runs);
    assert!(line.width() > px(0.));
});
```

[`WindowTextSystem::shape_line`] returns a [`ShapedLine`]; its
[`ShapedLine::width`] is the width to reserve. It accepts one line and panics in
debug builds on a newline. For a paragraph, use
[`WindowTextSystem::shape_text`] with the wrap width and pass the painted line
height to `ShapedText::size`:

```rust
# use gpui::*;
# struct Demo;
# impl Render for Demo {
#     fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
#         div()
#     }
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
    // One line height for each line the text wrapped onto.
    let height = shaped.size(line_height).height;
    assert_eq!(height, line_height * shaped.line_count());
});
```

Using the same shaping and wrapping rules makes this value suitable for the
current window. Another platform can choose a different fallback face or
rasterizer, so it is layout input rather than a portable pixel snapshot.

[`StyledText::layout`] returns a shared [`TextLayout`]. Clone it while
constructing the element, then query it from an event handler or after the
frame has been drawn: layout and prepaint populate its state, and querying it
before then panics. [`TextLayout::index_for_position`] maps a point to a byte
index; [`TextLayout::position_for_index`] maps an index back to a glyph
position. Together they provide an editor's hit-test result and caret position.

## Bidirectional text

Text containing English and Arabic has two orders to account for. The source
string keeps its logical byte order; the Unicode bidirectional algorithm places
the shaped text in visual order. A pointer follows the visual layout, but hit
testing returns a boundary in the original string. Use that byte index for a
link or style range instead of deriving one from a glyph's visual position.

Set the paragraph direction with [`Styled::rtl`] or [`Styled::ltr`]; it also
decides which physical edge `text_start` and `text_end` use. Use
[`Styled::unicode_bidi`] only for explicit embedding control; ordinary mixed
direction text can use the Unicode algorithm's resolution.

## See also

- [Text example](crate::_documentation::examples::text), which covers the scale,
  weights, decorations, wrapping and a styled paragraph. Run it with
  `cargo run --example text`.
- [Accessibility](crate::_documentation::guides::accessibility) for the `text!`
  macro and IDs.
- [Layout and style](crate::_documentation::concepts::layout_and_style) for how
  styles cascade and how an element's size is decided.
- [Images and assets](crate::_documentation::guides::images_and_assets) for
  icons that sit next to text.

[`text!`]: crate::text
