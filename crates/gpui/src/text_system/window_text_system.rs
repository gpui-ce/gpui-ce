use crate::{
    InlineLayout, InlineLayoutRequest, LineLayout, LineLayoutCache, LineLayoutIndex, Pixels,
    Result, ShapedLine, ShapedText, SharedString, TextLayoutOptions, TextRun, TextSystem,
};
use derive_more::Deref;
use std::{ops::Range, sync::Arc};

/// The text layout subsystem for one window, with a cache of shaped lines.
///
/// Get it with [`Window::text_system`](crate::Window::text_system). It derefs to the shared [`TextSystem`],
/// so you can call font methods on it directly.
///
/// Layout methods such as [`WindowTextSystem::shape_line`] and
/// [`WindowTextSystem::shape_text`] cache their results. A layout is reused when
/// the same text, font size, and shaping settings are requested again. The cache
/// keeps a layout only while the next frame uses it, so a layout that a frame
/// doesn't request is dropped at the end of that frame. Changing only colors or
/// decorations reuses the glyphs and repaints them.
#[derive(Deref)]
pub struct WindowTextSystem {
    line_layout_cache: LineLayoutCache,
    #[deref]
    text_system: Arc<TextSystem>,
}

impl WindowTextSystem {
    /// Creates a window text system with an empty layout cache on top of `text_system`.
    ///
    /// GPUI creates one for each window. You don't need to call this yourself.
    pub fn new(text_system: Arc<TextSystem>) -> Self {
        Self {
            line_layout_cache: LineLayoutCache::new(text_system.platform_text_system.clone()),
            text_system,
        }
    }

    pub(crate) fn layout_index(&self) -> LineLayoutIndex {
        self.line_layout_cache.layout_index()
    }

    pub(crate) fn reuse_layouts(&self, index: Range<LineLayoutIndex>) {
        self.line_layout_cache.reuse_layouts(index)
    }

    pub(crate) fn truncate_layouts(&self, index: LineLayoutIndex) {
        self.line_layout_cache.truncate_layouts(index)
    }

    /// Shapes one line of text, ready to paint.
    ///
    /// Use this for a single line of text that you paint with [`ShapedLine::paint`].
    /// For text that can wrap or has several paragraphs, use [`Self::shape_text`].
    /// `runs` styles the text, and the run lengths must add up to `text.len()`.
    /// Each run's [`TextRun::font`] selects the face used for its bytes.
    ///
    /// The result is cached for the frame, as described on [`WindowTextSystem`].
    ///
    /// # Panics
    ///
    /// In debug builds, panics if `text` contains a `\n`. Release builds don't
    /// check this, and the result for such text is not meaningful. Use
    /// [`Self::shape_text`] for multiple lines.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{EmptyView, TestApp, TextRun, font, hsla, px};
    /// # let mut app = TestApp::new();
    /// # let mut window = app.open_window(|_window, _cx| EmptyView);
    /// window.update(|_view, window, _cx| {
    ///     let run = TextRun {
    ///         len: "Hello".len(),
    ///         font: font("Lilex"),
    ///         color: hsla(0., 0., 0., 1.),
    ///         ..Default::default()
    ///     };
    ///     let line = window.text_system().shape_line("Hello".into(), px(16.), &[run]);
    ///     assert_eq!(line.len(), 5);
    /// });
    /// ```
    pub fn shape_line(
        &self,
        text: SharedString,
        font_size: Pixels,
        runs: &[TextRun],
    ) -> ShapedLine {
        debug_assert!(
            text.find('\n').is_none(),
            "text argument should not contain newlines"
        );

        let layout = self.layout_line(&text, font_size, runs);

        ShapedLine { layout, text }
    }

    /// Shapes text that may span several lines, ready to paint.
    ///
    /// This is the function to use for paragraphs. Each `\n` starts a new line.
    /// When `wrap_width` is `Some`, lines wrap at word boundaries to fit within
    /// that many logical pixels. A word wider than that is not split, so it
    /// overflows. `None` disables soft wrapping, and only `\n` breaks lines.
    /// With a wrap width, `line_clamp = Some(n)` limits soft wrapping: once the
    /// budget is exhausted, the rest of each paragraph occupies an unwrapped line.
    /// Hard paragraph breaks are preserved, so the total can exceed `n`.
    /// `Some(0)` behaves like `Some(1)` and disables soft wrapping; it does not hide text.
    ///
    /// `runs` styles the text. Each run gives the number of UTF-8 bytes it covers,
    /// and the runs must cover the whole string and end on character boundaries.
    /// The `Result` is currently always `Ok`.
    ///
    /// Shaping is cached as described on [`WindowTextSystem`]. To lay out text
    /// inside an element, use [`StyledText`](crate::StyledText) instead. See
    /// [text](crate::_documentation::guides::text) for an overview.
    ///
    /// # Examples
    ///
    /// This example supplies the platform's real text system. [`TestApp::new`](crate::TestApp::new)
    /// uses a simplified test shaper, which does not reproduce paragraph layout.
    ///
    /// ```standalone_crate
    /// # use gpui::{EmptyView, TestApp, TextRun, font, hsla, px};
    /// let text_system = gpui_platform::current_platform(true).text_system();
    /// let mut app = TestApp::with_text_system(text_system);
    /// # let mut window = app.open_window(|_window, _cx| EmptyView);
    /// window.update(|_view, window, _cx| {
    ///     let text = "first line\nsecond line";
    ///     let run = TextRun {
    ///         len: text.len(),
    ///         font: font(".SystemUIFont"),
    ///         color: hsla(0., 0., 0., 1.),
    ///         ..Default::default()
    ///     };
    ///     let shaped = window
    ///         .text_system()
    ///         .shape_text(text, px(16.), &[run.clone()], Some(px(200.)), None)
    ///         .unwrap();
    ///     assert_eq!(shaped.len(), text.len());
    ///
    ///     // A clamp cannot remove the hard break between these paragraphs.
    ///     let zero = window.text_system()
    ///         .shape_text(text, px(16.), &[run.clone()], Some(px(20.)), Some(0))
    ///         .unwrap();
    ///     let one = window.text_system()
    ///         .shape_text(text, px(16.), &[run], Some(px(20.)), Some(1))
    ///         .unwrap();
    ///     assert_eq!(zero.line_count(), 2);
    ///     assert_eq!(one.line_count(), 2);
    /// });
    /// ```
    pub fn shape_text<S: AsRef<str> + Into<SharedString>>(
        &self,
        text: S,
        font_size: Pixels,
        runs: &[TextRun],
        wrap_width: Option<Pixels>,
        line_clamp: Option<usize>,
    ) -> Result<ShapedText> {
        self.shape_text_with_options(
            text,
            font_size,
            runs,
            TextLayoutOptions {
                wrap_width,
                line_clamp,
                ..Default::default()
            },
        )
    }

    /// Shapes a text document with explicit paragraph direction, alignment, and bidi options.
    ///
    /// This is the lower-level form of [`Self::shape_text`]. Use it when text
    /// must align within a width different from its wrap width, or must follow
    /// a specific paragraph direction. The `Result` is currently always `Ok`.
    #[doc(hidden)]
    pub fn shape_text_with_options<S: AsRef<str> + Into<SharedString>>(
        &self,
        text: S,
        font_size: Pixels,
        runs: &[TextRun],
        options: TextLayoutOptions,
    ) -> Result<ShapedText> {
        let text = text.into();
        let layout = self
            .line_layout_cache
            .layout_text_with_options(&text, font_size, runs, options);

        Ok(ShapedText { layout, text })
    }

    /// Lays out text and atomic boxes together, the way an inline element flows in a paragraph.
    ///
    /// Each box is an element that sits between characters, such as an icon or
    /// a badge. Text wraps around the boxes. The result is cached like other
    /// layouts.
    pub fn layout_inline(&self, request: InlineLayoutRequest<'_>) -> Arc<InlineLayout> {
        self.line_layout_cache.layout_inline(request)
    }

    /// Lays out one line of text and returns the backend's [`LineLayout`].
    ///
    /// Most code should call [`Self::shape_line`] instead, which returns a
    /// [`ShapedLine`] that you can paint. Use this only when you need the raw
    /// layout. `text` must not contain newlines, and nothing checks this here.
    pub fn layout_line(&self, text: &str, font_size: Pixels, runs: &[TextRun]) -> Arc<LineLayout> {
        self.line_layout_cache
            .layout_line(&SharedString::new(text), font_size, runs)
    }

    pub(crate) fn finish_frame(&self) {
        self.line_layout_cache.finish_frame()
    }
}
