use crate::{
    ActiveTooltip, AnyView, App, AppContext, Bounds, DispatchPhase, Element, ElementId,
    GlobalElementId, HighlightStyle, Hitbox, HitboxBehavior, InspectorElementId, IntoElement,
    LayoutId, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, SharedString, Size,
    TextOverflow, TextRun, TextStyle, TextTransform, TooltipId, WhiteSpace, Window, WrappedLine,
    WrappedLineLayout, px, register_tooltip_mouse_handlers, set_tooltip_on_window,
};
use anyhow::Context as _;
use gpui_util::ResultExt;
use smallvec::SmallVec;
use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    mem,
    ops::{Deref, DerefMut, Range},
    rc::Rc,
    sync::Arc,
};
use unicode_segmentation::UnicodeSegmentation;

/// An [`Element`] that renders text.
///
/// In general, [`Text`] objects should be created via the [`text`] macro:
/// ```rust
/// # use gpui::*;
/// # fn render() -> impl IntoElement {
/// div().child(text!("hello"))
/// # }
/// ```
/// ## IDs and Accessibility
///
/// [`Text`] elements have an ID. This ID is primarily used to produce nodes in
/// the accessibility tree, which allows the text to be visible to screen
/// readers and other assistive technologies.
///
/// This ID is stable across frames. If the same text, with the same ID, is
/// present in two consecutive frames, no updates are reported to the screen
/// reader. If the text changes, but the ID stays the same, then the screen
/// reader will be notified that a text node's content has changed. **However**,
/// if the ID changes, then the screen reader will be notified that a node has
/// been removed, and a new node has been added.
///
/// When using the [`text`] macro, each invocation of the macro will get a
/// unique ID, derived from its position in the source code (filename, line, and
/// column). For example:
/// ```rust
/// # use gpui::*;
/// let x = text!("hello");
/// let y = text!("hello");
/// // not equal, because different `text!` invocations produced them
/// assert_ne!(x.id(), y.id());
///
/// fn make_text(s: &str) -> Text { text!(s) }
/// let x = make_text("hello");
/// let y = make_text("hello");
/// // equal, because the same `text!` invocation produced them
/// assert_eq!(x.id(), y.id());
/// ```
/// When the contents of an invocation of [`text`] do not change, this
/// distinction is less relevant (with the caveat that you still need to take
/// care to ensure that duplicate IDs do not appear).
///
/// However, when a [`text`] invocation's argument *does* change, you should
/// consider whether this change should be reported as a node "updating its
/// contents", or an old node being destroyed and a new node being created.
#[derive(Debug, Clone)]
pub struct Text {
    id: Option<ElementId>,
    text: SharedString,
}

pub(crate) struct InlineTextContent {
    pub text: SharedString,
    pub runs: Vec<TextRun>,
}

pub(crate) fn inline_text_content(text: SharedString, text_style: &TextStyle) -> InlineTextContent {
    let text = apply_text_transform_preserving_byte_len(text, text_style.text_transform);
    InlineTextContent {
        runs: vec![text_style.to_run(text.len())],
        text,
    }
}

impl Text {
    /// Create a new [`Text`] element with a specific ID.
    ///
    /// If you want a unique ID to be assigned automatically, use the [`text`]
    /// macro. The docs for [`Text`] have more detail about choosing IDs.
    #[inline]
    pub const fn new(id: ElementId, text: SharedString) -> Self {
        Self { id: Some(id), text }
    }

    /// Create a new [`Text`] element that is inaccessible to screen readers.
    ///
    /// In order for text to be accessible to screen readers, it must have an ID
    /// provided. If you want text to be accessible, either use [`text`] to have
    /// an ID automatically assigned, or use [`Text::new`] to manually assign an
    /// ID.
    ///
    /// This function is intended for use inside custom UI components, where
    /// accessible properties may be set on parent containers.
    #[inline]
    pub const fn new_inaccessible(text: SharedString) -> Self {
        Self { id: None, text }
    }

    /// The ID of this [`Text`] element.
    #[inline]
    pub const fn id(&self) -> Option<&ElementId> {
        self.id.as_ref()
    }

    /// Produce a new [`Text`] with the given `id`.
    pub fn with_id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// The text that this [`Text`] element will display.
    #[inline]
    pub const fn text(&self) -> &SharedString {
        &self.text
    }
}

impl Deref for Text {
    type Target = SharedString;
    fn deref(&self) -> &Self::Target {
        &self.text
    }
}

impl DerefMut for Text {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.text
    }
}

/// Trivial hash function for the location information produced by the [`text`]
/// macro. Not covered by semver guarantees. Performance is not particularly
/// significant because it's only used on small strings in const contexts.
#[doc(hidden)]
pub const fn __hash_text_macro_location_unstable_do_not_use(s: &'static str) -> u64 {
    const BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;

    let bytes = s.as_bytes();
    let mut hash = BASIS;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(PRIME);
        i += 1;
    }
    hash
}

/// Create a new [`Text`] element.
///
/// ```rust
/// # use gpui::*;
/// let a = text!("hello");
/// let b = text!(id = "farewell-message", "hello");
///
/// ```
///
/// Text created with this macro is *accessible*. The macro generates an ID
/// based on the source location. See the docs for [`Text`] for a more in-depth
/// explanation of the significance of the ID of a [`Text`] element.
#[macro_export]
macro_rules! text {
    (id = $id:expr, $text:expr) => {{ $crate::Text::new($id.into(), $text.into()) }};
    ($text:expr) => {{
        const ID: &'static str = concat!(file!(), "/", line!(), ":", column!());
        const HASH: u64 = $crate::__hash_text_macro_location_unstable_do_not_use(ID);
        $crate::Text::new($crate::ElementId::Integer(HASH), $text.into())
    }};
}

impl IntoElement for Text {
    type Element = Self;
    #[inline]
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Text {
    type RequestLayoutState = TextLayout;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        self.id.clone()
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn a11y_role(&self) -> Option<accesskit::Role> {
        if self.id.is_some() {
            Some(accesskit::Role::Label)
        } else {
            None
        }
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        node.set_value(self.text.to_string());
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        <SharedString as Element>::request_layout(&mut self.text, id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        <SharedString as Element>::prepaint(
            &mut self.text,
            id,
            inspector_id,
            bounds,
            request_layout,
            window,
            cx,
        )
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        <SharedString as Element>::paint(
            &mut self.text,
            id,
            inspector_id,
            bounds,
            request_layout,
            prepaint,
            window,
            cx,
        );
    }
}

impl Element for &'static str {
    type RequestLayoutState = TextLayout;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut state = TextLayout::default();
        let layout_id = state.layout(SharedString::from(*self), None, window, cx);
        (layout_id, state)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        text_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        text_layout.prepaint(bounds, self, window)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        text_layout: &mut TextLayout,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        text_layout.paint(self, window, cx)
    }
}

impl IntoElement for &'static str {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl IntoElement for String {
    type Element = SharedString;

    fn into_element(self) -> Self::Element {
        self.into()
    }
}

impl IntoElement for Cow<'static, str> {
    type Element = SharedString;

    fn into_element(self) -> Self::Element {
        self.into()
    }
}

impl Element for SharedString {
    type RequestLayoutState = TextLayout;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut state = TextLayout::default();
        let layout_id = state.layout(self.clone(), None, window, cx);
        (layout_id, state)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        text_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        text_layout.prepaint(bounds, self.as_ref(), window)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        text_layout: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        text_layout.paint(self.as_ref(), window, cx)
    }
}

impl IntoElement for SharedString {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// Renders text with runs of different styles.
///
/// Callers are responsible for setting the correct style for each run.
/// For text with a uniform style, you can usually avoid calling this constructor
/// and just pass text directly.
pub struct StyledText {
    text: SharedString,
    runs: Option<Vec<TextRun>>,
    delayed_highlights: Option<Vec<(Range<usize>, HighlightStyle)>>,
    delayed_font_family_overrides: Option<Vec<(Range<usize>, SharedString)>>,
    layout: TextLayout,
}

impl StyledText {
    /// Construct a new styled text element from the given string.
    pub fn new(text: impl Into<SharedString>) -> Self {
        StyledText {
            text: text.into(),
            runs: None,
            delayed_highlights: None,
            delayed_font_family_overrides: None,
            layout: TextLayout::default(),
        }
    }

    /// Get the layout for this element. This can be used to map indices to pixels and vice versa.
    pub fn layout(&self) -> &TextLayout {
        &self.layout
    }

    /// Set the styling attributes for the given text, as well as
    /// as any ranges of text that have had their style customized.
    pub fn with_default_highlights(
        mut self,
        default_style: &TextStyle,
        highlights: impl IntoIterator<Item = (Range<usize>, HighlightStyle)>,
    ) -> Self {
        debug_assert!(
            self.delayed_highlights.is_none(),
            "Can't use `with_default_highlights` and `with_highlights`"
        );
        let runs = Self::compute_runs(&self.text, default_style, highlights);
        self.with_runs(runs)
    }

    /// Set the styling attributes for the given text, as well as
    /// as any ranges of text that have had their style customized.
    pub fn with_highlights(
        mut self,
        highlights: impl IntoIterator<Item = (Range<usize>, HighlightStyle)>,
    ) -> Self {
        debug_assert!(
            self.runs.is_none(),
            "Can't use `with_highlights` and `with_default_highlights`"
        );
        self.delayed_highlights = Some(
            highlights
                .into_iter()
                .inspect(|(run, _)| {
                    debug_assert!(self.text.is_char_boundary(run.start));
                    debug_assert!(self.text.is_char_boundary(run.end));
                })
                .collect::<Vec<_>>(),
        );
        self
    }

    fn compute_runs(
        text: &str,
        default_style: &TextStyle,
        highlights: impl IntoIterator<Item = (Range<usize>, HighlightStyle)>,
    ) -> Vec<TextRun> {
        let mut runs = Vec::new();
        let mut ix = 0;
        for (range, highlight) in highlights {
            if ix < range.start {
                debug_assert!(text.is_char_boundary(range.start));
                runs.push(default_style.clone().to_run(range.start - ix));
            }
            debug_assert!(text.is_char_boundary(range.end));
            runs.push(
                default_style
                    .clone()
                    .highlight(highlight)
                    .to_run(range.len()),
            );
            ix = range.end;
        }
        if ix < text.len() {
            runs.push(default_style.to_run(text.len() - ix));
        }
        runs
    }

    /// Override the font family for specific byte ranges of the text.
    ///
    /// This is resolved lazily at layout time, so the overrides are applied
    /// on top of the inherited text style from the parent element.
    /// Can be combined with [`with_highlights`](Self::with_highlights).
    ///
    /// The overrides must be sorted by range start and non-overlapping.
    /// Each override range must fall on character boundaries.
    pub fn with_font_family_overrides(
        mut self,
        overrides: impl IntoIterator<Item = (Range<usize>, SharedString)>,
    ) -> Self {
        self.delayed_font_family_overrides = Some(
            overrides
                .into_iter()
                .inspect(|(range, _)| {
                    debug_assert!(self.text.is_char_boundary(range.start));
                    debug_assert!(self.text.is_char_boundary(range.end));
                })
                .collect(),
        );
        self
    }

    fn apply_font_family_overrides(
        runs: &mut [TextRun],
        overrides: &[(Range<usize>, SharedString)],
    ) {
        let mut byte_offset = 0;
        let mut override_idx = 0;
        for run in runs.iter_mut() {
            let run_end = byte_offset + run.len;
            while override_idx < overrides.len() && overrides[override_idx].0.end <= byte_offset {
                override_idx += 1;
            }
            if override_idx < overrides.len() {
                let (ref range, ref family) = overrides[override_idx];
                if byte_offset >= range.start && run_end <= range.end {
                    run.font.family = family.clone();
                }
            }
            byte_offset = run_end;
        }
    }

    fn take_runs(&mut self, default_style: &TextStyle) -> Vec<TextRun> {
        let font_family_overrides = self.delayed_font_family_overrides.take();
        let mut runs = self.runs.take().or_else(|| {
            self.delayed_highlights.take().map(|delayed_highlights| {
                Self::compute_runs(&self.text, default_style, delayed_highlights)
            })
        });

        if let Some(ref overrides) = font_family_overrides {
            let runs = runs.get_or_insert_with(|| vec![default_style.to_run(self.text.len())]);
            Self::apply_font_family_overrides(runs, overrides);
        }

        runs.unwrap_or_else(|| vec![default_style.to_run(self.text.len())])
    }

    pub(crate) fn take_inline_content(&mut self, default_style: &TextStyle) -> InlineTextContent {
        let runs = self.take_runs(default_style);
        let text = apply_text_transform_preserving_byte_len(
            self.text.clone(),
            default_style.text_transform,
        );
        InlineTextContent { text, runs }
    }

    /// Set the text runs for this piece of text.
    pub fn with_runs(mut self, runs: Vec<TextRun>) -> Self {
        let mut text = &*self.text;
        for run in &runs {
            text = text.get(run.len..).unwrap_or_else(|| {
                #[cfg(debug_assertions)]
                panic!("invalid text run. Text: '{text}', run: {run:?}");
                #[cfg(not(debug_assertions))]
                panic!("invalid text run");
            });
        }
        assert!(text.is_empty(), "invalid text run");
        self.runs = Some(runs);
        self
    }
}

impl Element for StyledText {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let runs = self.take_runs(&window.text_style());
        let layout_id = self
            .layout
            .layout(self.text.clone(), Some(runs), window, cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        self.layout.prepaint(bounds, &self.text, window)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.layout.paint(&self.text, window, cx)
    }
}

impl IntoElement for StyledText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// The Layout for TextElement. This can be used to map indices to pixels and vice versa.
#[derive(Default, Clone)]
pub struct TextLayout(Rc<TextLayoutState>);

#[derive(Default)]
struct TextLayoutState {
    layout_id: Cell<Option<LayoutId>>,
    layout: RefCell<Option<TextLayoutInner>>,
}

struct TextLayoutInner {
    len: usize,
    document: Option<WrappedLine>,
    line_height: Pixels,
    wrap_width: Option<Pixels>,
    truncate_width: Option<Pixels>,
    size: Option<Size<Pixels>>,
    bounds: Option<Bounds<Pixels>>,
}

fn apply_text_transform_preserving_byte_len(
    text: SharedString,
    transform: Option<TextTransform>,
) -> SharedString {
    let Some(transform) = transform else {
        return text;
    };
    if matches!(transform, TextTransform::None) {
        return text;
    }

    let mut output = String::with_capacity(text.len());
    match transform {
        TextTransform::Uppercase => {
            for character in text.as_ref().chars() {
                push_case_mapped_character(&mut output, character, CaseMapKind::Upper);
            }
        }
        TextTransform::Lowercase => {
            for character in text.as_ref().chars() {
                push_case_mapped_character(&mut output, character, CaseMapKind::Lower);
            }
        }
        TextTransform::Capitalize => {
            for piece in text.as_ref().split_word_bounds() {
                let mut seen_first_letter = false;
                for character in piece.chars() {
                    if !seen_first_letter && character.is_alphabetic() {
                        push_case_mapped_character(&mut output, character, CaseMapKind::Upper);
                        seen_first_letter = true;
                    } else {
                        output.push(character);
                    }
                }
            }
        }
        TextTransform::None => return text,
    }

    SharedString::from(output)
}

#[derive(Copy, Clone)]
enum CaseMapKind {
    Upper,
    Lower,
}

fn push_case_mapped_character(output: &mut String, character: char, kind: CaseMapKind) {
    let mapped = match kind {
        CaseMapKind::Upper => character.to_uppercase().collect::<String>(),
        CaseMapKind::Lower => character.to_lowercase().collect::<String>(),
    };

    if mapped.len() == character.len_utf8() && mapped.chars().count() == 1 {
        output.push_str(&mapped);
    } else {
        output.push(character);
    }
}

#[cfg(test)]
mod text_transform_tests {
    use super::apply_text_transform_preserving_byte_len;
    use crate::{SharedString, TextTransform};

    #[test]
    fn text_transforms_preserve_bytes_and_spacing() {
        let input = SharedString::from("hello   WORLD\tfoo-bar 123baz déjà vu");
        let uppercase =
            apply_text_transform_preserving_byte_len(input.clone(), Some(TextTransform::Uppercase));
        let lowercase =
            apply_text_transform_preserving_byte_len(input.clone(), Some(TextTransform::Lowercase));
        let capitalize = apply_text_transform_preserving_byte_len(
            input.clone(),
            Some(TextTransform::Capitalize),
        );

        assert_eq!(uppercase.as_ref(), "HELLO   WORLD\tFOO-BAR 123BAZ DÉJÀ VU");
        assert_eq!(lowercase.as_ref(), "hello   world\tfoo-bar 123baz déjà vu");
        assert_eq!(capitalize.as_ref(), "Hello   WORLD\tFoo-Bar 123Baz Déjà Vu");
        assert_eq!(input.len(), uppercase.len());
        assert_eq!(input.len(), lowercase.len());
        assert_eq!(input.len(), capitalize.len());
    }

    #[test]
    fn text_transforms_skip_expanding_unicode_mappings() {
        let input = SharedString::from("straße İSTANBUL");
        let uppercase =
            apply_text_transform_preserving_byte_len(input.clone(), Some(TextTransform::Uppercase));
        let lowercase =
            apply_text_transform_preserving_byte_len(input.clone(), Some(TextTransform::Lowercase));

        assert_eq!(uppercase.as_ref(), "STRAßE İSTANBUL");
        assert_eq!(lowercase.as_ref(), "straße İstanbul");
        assert_eq!(input.len(), uppercase.len());
        assert_eq!(input.len(), lowercase.len());
    }

    #[test]
    fn capitalize_preserves_letters_after_digit_prefix() {
        let input = SharedString::from("123BAZ");
        let output = apply_text_transform_preserving_byte_len(
            input.clone(),
            Some(TextTransform::Capitalize),
        );
        assert_eq!(output.as_ref(), "123BAZ");
        assert_eq!(input.len(), output.len());
    }

    #[test]
    fn capitalize_does_not_fold_remaining_letters() {
        let input = SharedString::from("foo2BAR");
        let output =
            apply_text_transform_preserving_byte_len(input, Some(TextTransform::Capitalize));
        assert_eq!(output.as_ref(), "Foo2BAR");
    }

    #[test]
    fn capitalize_handles_apostrophe_contractions() {
        let input = SharedString::from("don't panic");
        let output =
            apply_text_transform_preserving_byte_len(input, Some(TextTransform::Capitalize));
        assert_eq!(output.as_ref(), "Don't Panic");
    }
}

/// Determines which part of overflowing text is removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TruncateFrom {
    /// Remove text from the start.
    Start,
    /// Remove text from the end.
    End,
    /// Remove text from the middle while retaining both ends.
    Middle,
}

/// Metadata about how text should be truncated. Generated during text layout via `TextLayout::evaluate_overflow`.
pub struct TextLayoutTruncation {
    /// The width that the text can occupy before it is truncated.
    pub width: Option<Pixels>,
    /// The text to affix to the displayed text if truncating (e.g. an ellipsis `...`).
    pub affix: SharedString,
    /// What side of the text will be truncated if it does not fit.
    pub source: TruncateFrom,
}

impl TextLayoutTruncation {
    /// Creates a truncation by using the overflow as the affix, given the provided width.
    fn overflow_width(text_overflow: TextOverflow, width: Option<Pixels>) -> Self {
        match text_overflow {
            TextOverflow::Truncate(s) => TextLayoutTruncation {
                width,
                affix: s,
                source: TruncateFrom::End,
            },
            TextOverflow::TruncateStart(s) => TextLayoutTruncation {
                width,
                affix: s,
                source: TruncateFrom::Start,
            },
            TextOverflow::TruncateMiddle(s) => TextLayoutTruncation {
                width,
                affix: s,
                source: TruncateFrom::Middle,
            },
        }
    }
}

impl TextLayout {
    /// Evaluates the width to wrap the text at.
    pub fn evaluate_wrap_width(
        white_space: &WhiteSpace,
        known_dimensions: Size<Option<Pixels>>,
        available_space: Size<crate::AvailableSpace>,
    ) -> Option<Pixels> {
        use crate::AvailableSpace::*;
        match white_space {
            // Text does not wrap, no max width
            WhiteSpace::Nowrap => None,
            // If the text wraps, return the already calculated width.
            WhiteSpace::Normal => known_dimensions.width.or(match available_space.width {
                // Otherwise if the available space is a concrete value, then that is the width to wrap to.
                Definite(x) => Some(x),
                // If the wrapping is content-based, then there is no wrapping of text.
                MaxContent | MinContent => None,
            }),
        }
    }

    /// Evaluates how truncation should be applied if the text overflows the available space.
    pub fn evaluate_overflow(
        text_style: &TextStyle,
        known_dimensions: Size<Option<Pixels>>,
        available_space: Size<crate::AvailableSpace>,
    ) -> TextLayoutTruncation {
        match text_style.text_overflow.clone() {
            Some(text_overflow) => {
                // Calculate the desired width, prioritizing the calculated dimensions,
                // falling back on calculating a width from the available space and
                // number of lines to clamp to via text style.
                let width = known_dimensions.width.or(match available_space.width {
                    crate::AvailableSpace::Definite(x) => match text_style.line_clamp {
                        Some(max_lines) => Some(x * max_lines),
                        None => Some(x),
                    },
                    _ => None,
                });

                TextLayoutTruncation::overflow_width(text_overflow, width)
            }
            None => TextLayoutTruncation {
                width: None,
                affix: SharedString::default(),
                source: TruncateFrom::End,
            },
        }
    }

    /// Conditionally applies truncation to some text and outputs how the text should be displayed.
    pub fn apply_truncation<'runs>(
        text: SharedString,
        text_style: &TextStyle,
        font_size: Pixels,
        line_height: Pixels,
        wrap_width: Option<Pixels>,
        truncation: &TextLayoutTruncation,
        runs: &'runs [TextRun],
        window: &mut Window,
        cx: &mut App,
    ) -> (SharedString, Cow<'runs, [TextRun]>) {
        let _ = (line_height, cx);
        let Some(truncate_width) = truncation.width else {
            return (text, Cow::Borrowed(runs));
        };
        truncate_to_shaped_layout(
            text,
            font_size,
            wrap_width,
            truncate_width,
            text_style.line_clamp,
            &truncation.affix,
            runs,
            truncation.source,
            window,
        )
    }

    fn layout(
        &self,
        text: SharedString,
        runs: Option<Vec<TextRun>>,
        window: &mut Window,
        _: &mut App,
    ) -> LayoutId {
        let text_style = window.text_style();
        let text = apply_text_transform_preserving_byte_len(text, text_style.text_transform);
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let line_height = window.pixel_snap(
            text_style
                .line_height
                .to_pixels(font_size.into(), window.rem_size()),
        );

        let runs = if let Some(runs) = runs {
            runs
        } else {
            vec![text_style.to_run(text.len())]
        };
        let layout_id = window.request_measured_layout(Default::default(), {
            let element_state = self.clone();

            move |known_dimensions, available_space, window, cx| {
                let wrap_width = Self::evaluate_wrap_width(
                    &text_style.white_space,
                    known_dimensions,
                    available_space,
                );

                let truncation =
                    Self::evaluate_overflow(&text_style, known_dimensions, available_space);
                let truncate_width = truncation.width;

                // Only use cached layout if:
                // 1. We have a cached size
                // 2. wrap_width matches (or both are None)
                // 3. truncate_width is None (if truncate_width is Some, we need to re-layout
                //    because the previous layout may have been computed without truncation)
                // 4. the cached layout was not truncated (a truncated layout answers an
                //    unconstrained probe with the truncated size, which poisons intrinsic
                //    sizing with whatever width some earlier measure pass happened to use)
                if let Some(text_layout) = element_state.0.layout.borrow().as_ref()
                    && let Some(size) = text_layout.size
                    && (wrap_width.is_none() || wrap_width == text_layout.wrap_width)
                    && truncate_width.is_none()
                    && text_layout.truncate_width.is_none()
                {
                    return size;
                }

                let (text, runs) = Self::apply_truncation(
                    text.clone(),
                    &text_style,
                    font_size,
                    line_height,
                    wrap_width,
                    &truncation,
                    &runs,
                    window,
                    cx,
                );
                let len = text.len();

                let Some(document) = window
                    .text_system()
                    .shape_text(
                        text,
                        font_size,
                        &runs,
                        wrap_width,            // Wrap if we know the width.
                        text_style.line_clamp, // Limit the number of lines if line_clamp is set.
                    )
                    .log_err()
                else {
                    element_state
                        .0
                        .layout
                        .borrow_mut()
                        .replace(TextLayoutInner {
                            document: None,
                            len: 0,
                            line_height,
                            wrap_width,
                            truncate_width,
                            size: Some(Size::default()),
                            bounds: None,
                        });
                    return Size::default();
                };

                let size = document.size(line_height);

                element_state
                    .0
                    .layout
                    .borrow_mut()
                    .replace(TextLayoutInner {
                        document: Some(document),
                        len,
                        line_height,
                        wrap_width,
                        truncate_width,
                        size: Some(size),
                        bounds: None,
                    });

                size
            }
        });
        self.0.layout_id.set(Some(layout_id));
        layout_id
    }

    fn prepaint(&self, bounds: Bounds<Pixels>, text: &str, window: &mut Window) {
        let bounds = self
            .0
            .layout_id
            .get()
            .map(|layout_id| window.parent_relative_layout_bounds(layout_id))
            .unwrap_or(bounds);
        let mut element_state = self.0.layout.borrow_mut();
        let element_state = element_state
            .as_mut()
            .with_context(|| format!("measurement has not been performed on {text}"))
            .unwrap();
        element_state.bounds = Some(bounds);
    }

    fn paint(&self, text: &str, window: &mut Window, cx: &mut App) {
        let element_state = self.0.layout.borrow();
        let element_state = element_state
            .as_ref()
            .with_context(|| format!("measurement has not been performed on {text}"))
            .unwrap();
        let bounds = element_state
            .bounds
            .with_context(|| format!("prepaint has not been performed on {text}"))
            .unwrap();

        let line_height = element_state.line_height;
        let text_style = window.text_style();
        if let Some(document) = &element_state.document {
            document
                .paint_background(
                    bounds.origin,
                    line_height,
                    text_style.text_align,
                    Some(bounds),
                    window,
                    cx,
                )
                .log_err();
            document
                .paint(
                    bounds.origin,
                    line_height,
                    text_style.text_align,
                    Some(bounds),
                    window,
                    cx,
                )
                .log_err();
        }
    }

    /// Get the byte index into the input of the pixel position.
    pub fn index_for_position(&self, mut position: Point<Pixels>) -> Result<usize, usize> {
        let element_state = self.0.layout.borrow();
        let element_state = element_state
            .as_ref()
            .expect("measurement has not been performed");
        let bounds = element_state
            .bounds
            .expect("prepaint has not been performed");

        if position.y < bounds.top() {
            return Err(0);
        }

        let line_height = element_state.line_height;
        let Some(document) = &element_state.document else {
            return Err(0);
        };
        document.index_for_position(position - bounds.origin, line_height)
    }

    /// Get the pixel position for the given byte index.
    pub fn position_for_index(&self, index: usize) -> Option<Point<Pixels>> {
        let element_state = self.0.layout.borrow();
        let element_state = element_state
            .as_ref()
            .expect("measurement has not been performed");
        let bounds = element_state
            .bounds
            .expect("prepaint has not been performed");
        let line_height = element_state.line_height;

        let document = element_state.document.as_ref()?;
        Some(bounds.origin + document.position_for_index(index, line_height)?)
    }

    /// Retrieve the layout for the line containing the given byte index.
    pub fn line_layout_for_index(&self, index: usize) -> Option<Arc<WrappedLineLayout>> {
        let element_state = self.0.layout.borrow();
        let element_state = element_state
            .as_ref()
            .expect("measurement has not been performed");
        let document = element_state.document.as_ref()?;
        (index <= document.len()).then(|| document.layout.clone())
    }

    /// Retrieve all line layouts in source order.
    pub fn line_layouts(&self) -> SmallVec<[Arc<WrappedLineLayout>; 1]> {
        self.0
            .layout
            .borrow()
            .as_ref()
            .expect("measurement has not been performed")
            .document
            .iter()
            .map(|document| document.layout.clone())
            .collect()
    }

    /// The bounds of this layout.
    pub fn bounds(&self) -> Bounds<Pixels> {
        self.0.layout.borrow().as_ref().unwrap().bounds.unwrap()
    }

    /// The line height for this layout.
    pub fn line_height(&self) -> Pixels {
        self.0.layout.borrow().as_ref().unwrap().line_height
    }

    /// The UTF-8 length of the underlying text.
    pub fn len(&self) -> usize {
        self.0.layout.borrow().as_ref().unwrap().len
    }

    /// The text for this layout.
    pub fn text(&self) -> String {
        self.0
            .layout
            .borrow()
            .as_ref()
            .unwrap()
            .document
            .as_ref()
            .map_or_else(String::new, |document| document.text.to_string())
    }

    /// The text for this layout (with soft-wraps as newlines)
    pub fn wrapped_text(&self) -> String {
        let mut accumulator = String::new();

        if let Some(document) = &self.0.layout.borrow().as_ref().unwrap().document {
            for visual_line in document.layout.visual_lines() {
                accumulator.push_str(&document.text[visual_line.text_range.clone()]);
                accumulator.push('\n');
            }
        }
        // Remove trailing newline
        accumulator.pop();
        accumulator
    }
}

fn truncate_to_shaped_layout<'a>(
    text: SharedString,
    font_size: Pixels,
    wrap_width: Option<Pixels>,
    truncate_width: Pixels,
    max_lines: Option<usize>,
    affix: &str,
    runs: &'a [TextRun],
    direction: TruncateFrom,
    window: &mut Window,
) -> (SharedString, Cow<'a, [TextRun]>) {
    let fits = |candidate: &str, candidate_runs: &[TextRun], window: &mut Window| {
        let Ok(document) = window.text_system().shape_text(
            SharedString::from(candidate.to_owned()),
            font_size,
            candidate_runs,
            wrap_width,
            None,
        ) else {
            return false;
        };
        let width = wrap_width.unwrap_or(truncate_width);
        max_lines.is_none_or(|max_lines| document.line_count() <= max_lines.max(1))
            && document
                .visual_lines()
                .iter()
                .all(|visual| visual.advance <= width + px(0.01))
    };

    if fits(&text, runs, window) {
        return (text, Cow::Borrowed(runs));
    }

    let mut boundaries = text
        .grapheme_indices(true)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    boundaries.push(text.len());
    let grapheme_count = boundaries.len().saturating_sub(1);
    let candidate =
        |keep| make_truncation_candidate(&text, &boundaries, keep, affix, runs, direction);

    let mut low = 0usize;
    let mut high = grapheme_count.saturating_sub(1);
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        let (candidate_text, candidate_runs) = candidate(middle);
        if fits(&candidate_text, &candidate_runs, window) {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    let (result, result_runs) = candidate(low);
    (result, Cow::Owned(result_runs))
}

fn make_truncation_candidate(
    text: &str,
    boundaries: &[usize],
    keep: usize,
    affix: &str,
    runs: &[TextRun],
    direction: TruncateFrom,
) -> (SharedString, Vec<TextRun>) {
    let grapheme_count = boundaries.len().saturating_sub(1);
    let keep = keep.min(grapheme_count);
    let mut candidate_runs = runs.to_vec();
    match direction {
        TruncateFrom::End => {
            let end = boundaries[keep];
            let prefix = text[..end]
                .trim_end_matches(|ch: char| ch.is_whitespace() || ch.is_ascii_punctuation());
            let result = SharedString::from(format!("{prefix}{affix}"));
            update_runs_after_truncation(&result, affix, &mut candidate_runs, direction);
            (result, candidate_runs)
        }
        TruncateFrom::Start => {
            let start = boundaries[grapheme_count - keep];
            let result = SharedString::from(format!("{affix}{}", &text[start..]));
            update_runs_after_truncation(&result, affix, &mut candidate_runs, direction);
            (result, candidate_runs)
        }
        TruncateFrom::Middle => {
            let front_count = keep.saturating_mul(2).div_ceil(3);
            let back_count = keep - front_count;
            let front_end = boundaries[front_count];
            let back_start = boundaries[grapheme_count - back_count];
            let result = SharedString::from(format!(
                "{}{affix}{}",
                &text[..front_end],
                &text[back_start..]
            ));
            update_runs_after_middle_truncation(affix, &mut candidate_runs, front_end, back_start);
            (result, candidate_runs)
        }
    }
}

fn update_runs_after_truncation(
    result: &str,
    affix: &str,
    runs: &mut Vec<TextRun>,
    direction: TruncateFrom,
) {
    let mut retained = result.len().saturating_sub(affix.len());
    match direction {
        TruncateFrom::Start => {
            for run_index in (0..runs.len()).rev() {
                if runs[run_index].len <= retained {
                    retained -= runs[run_index].len;
                } else {
                    runs[run_index].len = retained + affix.len();
                    runs.drain(..run_index);
                    break;
                }
            }
        }
        TruncateFrom::End => {
            for run_index in 0..runs.len() {
                if runs[run_index].len <= retained {
                    retained -= runs[run_index].len;
                } else {
                    runs[run_index].len = retained + affix.len();
                    runs.truncate(run_index + 1);
                    break;
                }
            }
        }
        TruncateFrom::Middle => unreachable!(),
    }
}

fn update_runs_after_middle_truncation(
    affix: &str,
    runs: &mut Vec<TextRun>,
    front_end: usize,
    back_start: usize,
) {
    let original = mem::take(runs);
    let mut result = Vec::with_capacity(original.len());
    let mut byte_offset = 0usize;
    for run in &original {
        let run_end = byte_offset + run.len;
        if byte_offset < front_end {
            let mut retained = run.clone();
            retained.len = run_end.min(front_end) - byte_offset;
            result.push(retained);
        }
        byte_offset = run_end;
    }
    if let Some(last) = result.last_mut() {
        last.len += affix.len();
    } else if let Some(first) = original.first() {
        let mut affix_run = first.clone();
        affix_run.len = affix.len();
        result.push(affix_run);
    }
    byte_offset = 0;
    for run in &original {
        let run_end = byte_offset + run.len;
        if run_end > back_start {
            let mut retained = run.clone();
            retained.len = run_end - back_start.max(byte_offset);
            result.push(retained);
        }
        byte_offset = run_end;
    }
    *runs = result;
}

#[cfg(test)]
mod truncation_tests {
    use super::*;

    #[test]
    fn truncation_candidates_keep_complete_graphemes_and_cover_output_with_runs() {
        const FAMILY: &str = "👩‍👩‍👧‍👦";
        let text = format!("Ae\u{301}{FAMILY}Z");
        let split = "Ae\u{301}".len();
        let runs = [
            TextRun {
                len: split,
                ..Default::default()
            },
            TextRun {
                len: text.len() - split,
                ..Default::default()
            },
        ];
        let mut boundaries = text
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        boundaries.push(text.len());

        for (direction, keep, expected) in [
            (TruncateFrom::Start, 2, format!("…{FAMILY}Z")),
            (TruncateFrom::End, 2, "Ae\u{301}…".to_owned()),
            (TruncateFrom::Middle, 3, "Ae\u{301}…Z".to_owned()),
        ] {
            let (candidate, candidate_runs) =
                make_truncation_candidate(&text, &boundaries, keep, "…", &runs, direction);
            assert_eq!(candidate.as_ref(), expected, "{direction:?}");
            assert_eq!(
                candidate_runs.iter().map(|run| run.len).sum::<usize>(),
                candidate.len(),
                "style runs must cover {candidate:?} after {direction:?} truncation"
            );
        }
    }
}

/// A text element that can be interacted with.
pub struct InteractiveText {
    element_id: ElementId,
    text: StyledText,
    click_listener:
        Option<Box<dyn Fn(&[Range<usize>], InteractiveTextClickEvent, &mut Window, &mut App)>>,
    hover_listener: Option<Box<dyn Fn(Option<usize>, MouseMoveEvent, &mut Window, &mut App)>>,
    tooltip_builder: Option<Rc<dyn Fn(usize, &mut Window, &mut App) -> Option<AnyView>>>,
    tooltip_id: Option<TooltipId>,
    clickable_ranges: Vec<Range<usize>>,
}

struct InteractiveTextClickEvent {
    mouse_down_index: usize,
    mouse_up_index: usize,
}

#[doc(hidden)]
#[derive(Default)]
pub struct InteractiveTextState {
    mouse_down_index: Rc<Cell<Option<usize>>>,
    hovered_index: Rc<Cell<Option<usize>>>,
    active_tooltip: Rc<RefCell<Option<ActiveTooltip>>>,
}

/// InteractiveTest is a wrapper around StyledText that adds mouse interactions.
impl InteractiveText {
    /// Creates a new InteractiveText from the given text.
    pub fn new(id: impl Into<ElementId>, text: StyledText) -> Self {
        Self {
            element_id: id.into(),
            text,
            click_listener: None,
            hover_listener: None,
            tooltip_builder: None,
            tooltip_id: None,
            clickable_ranges: Vec::new(),
        }
    }

    /// on_click is called when the user clicks on one of the given ranges, passing the index of
    /// the clicked range.
    pub fn on_click(
        mut self,
        ranges: Vec<Range<usize>>,
        listener: impl Fn(usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.click_listener = Some(Box::new(move |ranges, event, window, cx| {
            for (range_ix, range) in ranges.iter().enumerate() {
                if range.contains(&event.mouse_down_index) && range.contains(&event.mouse_up_index)
                {
                    listener(range_ix, window, cx);
                }
            }
        }));
        self.clickable_ranges = ranges;
        self
    }

    /// on_hover is called when the mouse moves over a character within the text, passing the
    /// index of the hovered character, or None if the mouse leaves the text.
    pub fn on_hover(
        mut self,
        listener: impl Fn(Option<usize>, MouseMoveEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.hover_listener = Some(Box::new(listener));
        self
    }

    /// tooltip lets you specify a tooltip for a given character index in the string.
    pub fn tooltip(
        mut self,
        builder: impl Fn(usize, &mut Window, &mut App) -> Option<AnyView> + 'static,
    ) -> Self {
        self.tooltip_builder = Some(Rc::new(builder));
        self
    }
}

impl Element for InteractiveText {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.element_id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn a11y_role(&self) -> Option<accesskit::Role> {
        Some(accesskit::Role::Label)
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        node.set_value(self.text.text.to_string());
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.text.request_layout(None, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Hitbox {
        window.with_optional_element_state::<InteractiveTextState, _>(
            global_id,
            |interactive_state, window| {
                let mut interactive_state = interactive_state
                    .map(|interactive_state| interactive_state.unwrap_or_default());

                if let Some(interactive_state) = interactive_state.as_mut() {
                    if self.tooltip_builder.is_some() {
                        self.tooltip_id =
                            set_tooltip_on_window(&interactive_state.active_tooltip, window);
                    } else {
                        // If there is no longer a tooltip builder, remove the active tooltip.
                        interactive_state.active_tooltip.take();
                    }
                }

                self.text
                    .prepaint(None, inspector_id, bounds, state, window, cx);
                let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
                (hitbox, interactive_state)
            },
        )
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        hitbox: &mut Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        let current_view = window.current_view();
        let text_layout = self.text.layout().clone();
        window.with_element_state::<InteractiveTextState, _>(
            global_id.unwrap(),
            |interactive_state, window| {
                let mut interactive_state = interactive_state.unwrap_or_default();
                if let Some(click_listener) = self.click_listener.take() {
                    let mouse_position = window.mouse_position();
                    if let Ok(ix) = text_layout.index_for_position(mouse_position)
                        && self
                            .clickable_ranges
                            .iter()
                            .any(|range| range.contains(&ix))
                    {
                        window.set_cursor_style(crate::CursorStyle::PointingHand, hitbox)
                    }

                    let text_layout = text_layout.clone();
                    let mouse_down = interactive_state.mouse_down_index.clone();
                    if let Some(mouse_down_index) = mouse_down.get() {
                        let hitbox = hitbox.clone();
                        let clickable_ranges = mem::take(&mut self.clickable_ranges);
                        window.on_mouse_event(
                            move |event: &MouseUpEvent, phase, window: &mut Window, cx| {
                                if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) {
                                    if let Ok(mouse_up_index) =
                                        text_layout.index_for_position(event.position)
                                    {
                                        click_listener(
                                            &clickable_ranges,
                                            InteractiveTextClickEvent {
                                                mouse_down_index,
                                                mouse_up_index,
                                            },
                                            window,
                                            cx,
                                        )
                                    }

                                    mouse_down.take();
                                    window.refresh();
                                }
                            },
                        );
                    } else {
                        let hitbox = hitbox.clone();
                        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, _| {
                            if phase == DispatchPhase::Bubble
                                && hitbox.is_hovered(window)
                                && let Ok(mouse_down_index) =
                                    text_layout.index_for_position(event.position)
                            {
                                mouse_down.set(Some(mouse_down_index));
                                window.refresh();
                            }
                        });
                    }
                }

                window.on_mouse_event({
                    let mut hover_listener = self.hover_listener.take();
                    let hitbox = hitbox.clone();
                    let text_layout = text_layout.clone();
                    let hovered_index = interactive_state.hovered_index.clone();
                    move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) {
                            let current = hovered_index.get();
                            let updated = text_layout.index_for_position(event.position).ok();
                            if current != updated {
                                hovered_index.set(updated);
                                if let Some(hover_listener) = hover_listener.as_ref() {
                                    hover_listener(updated, event.clone(), window, cx);
                                }
                                cx.notify(current_view);
                            }
                        }
                    }
                });

                if let Some(tooltip_builder) = self.tooltip_builder.clone() {
                    let active_tooltip = interactive_state.active_tooltip.clone();
                    let build_tooltip = Rc::new({
                        let tooltip_is_hoverable = false;
                        let text_layout = text_layout.clone();
                        move |window: &mut Window, cx: &mut App| {
                            text_layout
                                .index_for_position(window.mouse_position())
                                .ok()
                                .and_then(|position| tooltip_builder(position, window, cx))
                                .map(|view| (view, tooltip_is_hoverable))
                        }
                    });

                    // Use bounds instead of testing hitbox since this is called during prepaint.
                    let check_is_hovered_during_prepaint = Rc::new({
                        let source_bounds = hitbox.bounds;
                        let text_layout = text_layout.clone();
                        let pending_mouse_down = interactive_state.mouse_down_index.clone();
                        move |window: &Window| {
                            text_layout
                                .index_for_position(window.mouse_position())
                                .is_ok()
                                && source_bounds.contains(&window.mouse_position())
                                && pending_mouse_down.get().is_none()
                        }
                    });

                    let check_is_hovered = Rc::new({
                        let hitbox = hitbox.clone();
                        let text_layout = text_layout.clone();
                        let pending_mouse_down = interactive_state.mouse_down_index.clone();
                        move |window: &Window| {
                            text_layout
                                .index_for_position(window.mouse_position())
                                .is_ok()
                                && hitbox.is_hovered(window)
                                && pending_mouse_down.get().is_none()
                        }
                    });

                    register_tooltip_mouse_handlers(
                        &active_tooltip,
                        self.tooltip_id,
                        build_tooltip,
                        check_is_hovered,
                        check_is_hovered_during_prepaint,
                        None,
                        window,
                    );
                }

                self.text
                    .paint(None, inspector_id, bounds, &mut (), &mut (), window, cx);

                ((), interactive_state)
            },
        );
    }
}

impl IntoElement for InteractiveText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Context, Hsla, Render, ScaledPixels, TestApp, div, hsla, prelude::*};
    use std::collections::HashSet;

    const CONTAINER_COLOR: Hsla = hsla(0.72, 0.45, 0.32, 1.0);
    const TEXT_BACKGROUND_COLOR: Hsla = hsla(0.37, 0.65, 0.42, 1.0);

    struct CenteredTextView {
        extent: f32,
    }

    impl Render for CenteredTextView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(320.0 + self.extent))
                .h(px(160.0 + self.extent))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(118.0))
                        .h(px(31.0))
                        .bg(CONTAINER_COLOR)
                        .text_size(px(14.0))
                        .child(StyledText::new("x").with_highlights([(
                            0..1,
                            HighlightStyle {
                                background_color: Some(TEXT_BACKGROUND_COLOR),
                                ..Default::default()
                            },
                        )])),
                )
        }
    }

    fn only_quad(window: &Window, color: Hsla) -> Bounds<ScaledPixels> {
        let color = color.into();
        let mut bounds = window
            .rendered_frame
            .scene
            .quads
            .iter()
            .filter(|quad| quad.background.solid == color)
            .map(|quad| quad.bounds);
        let result = bounds.next().expect("expected a rendered quad");
        assert!(bounds.next().is_none(), "expected only one rendered quad");
        result
    }

    #[test]
    fn test_into_element_for() {
        use crate::{ParentElement as _, SharedString, div};
        use std::borrow::Cow;

        let _ = div().child("static str");
        let _ = div().child("String".to_string());
        let _ = div().child(Cow::Borrowed("Cow"));
        let _ = div().child(SharedString::from("SharedString"));
    }

    #[test]
    fn text_macro_id() {
        // one call to `text!` = one id
        fn make_text_stable_id(happy: bool) -> Text {
            text!(if happy { "happy" } else { "sad" })
        }

        // two calls to `text!` = two ids
        fn make_text_unstable_id(happy: bool) -> Text {
            if happy { text!("happy") } else { text!("sad") }
        }

        assert_eq!(make_text_stable_id(false).id, make_text_stable_id(true).id);
        assert_ne!(
            make_text_unstable_id(false).id,
            make_text_unstable_id(true).id
        );
    }

    #[test]
    fn accessible_text_keeps_its_unicode_value_and_stable_identity() {
        let first = Text::new("status".into(), "Ready العربية 👩🏽‍💻".into());
        let second = Text::new("status".into(), "Done 日本語 ✅".into());
        assert_eq!(first.id(), second.id());
        assert_eq!(first.a11y_role(), Some(accesskit::Role::Label));

        let mut node = accesskit::Node::new(accesskit::Role::Label);
        second.write_a11y_info(&mut node);
        assert_eq!(node.value(), Some("Done 日本語 ✅"));

        let hidden = Text::new_inaccessible("decorative 👀".into());
        assert_eq!(hidden.a11y_role(), None);
    }

    #[test]
    fn centered_text_keeps_its_device_pixel_offset_when_its_parent_moves() {
        for scale_factor in [1.0, 1.5] {
            let mut app = TestApp::new();
            let mut test_window = app.open_window(|window, _| {
                window.set_scale_factor(scale_factor);
                CenteredTextView { extent: 0.0 }
            });
            test_window.draw();

            let (initial_container, initial_background) = test_window.update(|_, window, _| {
                (
                    only_quad(window, CONTAINER_COLOR),
                    only_quad(window, TEXT_BACKGROUND_COLOR),
                )
            });
            let expected_offset = initial_background.origin - initial_container.origin;
            let mut container_origins = HashSet::from([(
                initial_container.origin.x.as_f32() as i32,
                initial_container.origin.y.as_f32() as i32,
            )]);

            for step in 1..=32 {
                test_window.update(|view, _, cx| {
                    view.extent = step as f32;
                    cx.notify();
                });
                test_window.draw();

                let (container, background) = test_window.update(|_, window, _| {
                    (
                        only_quad(window, CONTAINER_COLOR),
                        only_quad(window, TEXT_BACKGROUND_COLOR),
                    )
                });
                assert_eq!(
                    background.origin - container.origin,
                    expected_offset,
                    "text moved within its parent at scale {scale_factor}, step {step}"
                );
                container_origins.insert((
                    container.origin.x.as_f32() as i32,
                    container.origin.y.as_f32() as i32,
                ));
            }

            assert!(
                container_origins.len() > 8,
                "fixture did not cross enough device pixels at scale {scale_factor}"
            );
        }
    }
}
