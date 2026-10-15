use crate::{
    App, Bounds, ClipboardItem, Context, Entity, InputHandler, Pixels, TextInputConfiguration,
    UTF16Selection, Window,
};
use std::ops::Range;

/// Converts a UTF-8 byte offset into a UTF-16 code-unit offset.
///
/// Platform text input counts in UTF-16 code units, while Rust strings index
/// by UTF-8 bytes. Use this to translate an offset in a `String` into the offset
/// the platform expects. An offset past the end returns the UTF-16 length of
/// `text`. An offset inside a character returns the offset of that character's
/// end.
///
/// # Examples
///
/// ```
/// use gpui::utf8_to_utf16_offset;
///
/// // "A" is 1 byte and 1 code unit. The emoji is 4 bytes and 2 code units.
/// assert_eq!(utf8_to_utf16_offset("A😀B", 5), 3);
/// assert_eq!(utf8_to_utf16_offset("A😀B", 100), 4);
/// ```
pub fn utf8_to_utf16_offset(text: &str, utf8_offset: usize) -> usize {
    text.char_indices()
        .take_while(|(offset, _)| *offset < utf8_offset)
        .map(|(_, character)| character.len_utf16())
        .sum()
}

/// Converts a UTF-16 code-unit offset into a UTF-8 byte offset.
///
/// The reverse of [`utf8_to_utf16_offset`]. Use it on ranges that the platform
/// sends, before slicing a `String`. An offset past the end returns the UTF-8
/// length of `text`. An offset inside a surrogate pair returns the offset of the
/// end of that character.
///
/// # Examples
///
/// ```
/// use gpui::utf16_to_utf8_offset;
///
/// assert_eq!(utf16_to_utf8_offset("A😀B", 3), 5);
/// assert_eq!(utf16_to_utf8_offset("A😀B", 100), 6);
/// ```
pub fn utf16_to_utf8_offset(text: &str, utf16_offset: usize) -> usize {
    let mut consumed_utf16 = 0;
    for (utf8_offset, character) in text.char_indices() {
        if consumed_utf16 >= utf16_offset {
            return utf8_offset;
        }
        consumed_utf16 += character.len_utf16();
    }
    text.len()
}

/// Text input for a view: the methods the platform calls for typed text, IME
/// composition, paste and selection.
///
/// Implement this on a view that edits text, such as a text field. Then wrap the
/// view in an [`ElementInputHandler`] and install it during paint with
/// [`Window::handle_input`](crate::Window::handle_input). The platform's input method sends its requests
/// through that wrapper, and each method runs with the view updated.
///
/// Ranges are in UTF-16 code units, the unit the platform uses, and are
/// relative to the whole document. Convert with [`utf8_to_utf16_offset`] and
/// [`utf16_to_utf8_offset`] when your text is a UTF-8 `String`. Every method has
/// the same meaning as the matching method on [`InputHandler`], which documents
/// the platform side.
///
/// The text guide has a complete editable field: [text input](crate::_documentation::guides::text_input).
///
/// # Examples
///
/// ```
/// use gpui::{
///     Bounds, Context, ElementInputHandler, EntityInputHandler, FocusHandle, IntoElement,
///     Pixels, Point, Render, TestApp, UTF16Selection, Window, canvas, div, prelude::*,
/// };
/// use std::ops::Range;
///
/// struct Field {
///     focus_handle: FocusHandle,
///     text: String,
/// }
///
/// impl EntityInputHandler for Field {
///     fn text_for_range(
///         &mut self,
///         range: Range<usize>,
///         adjusted_range: &mut Option<Range<usize>>,
///         _window: &mut Window,
///         _cx: &mut Context<Self>,
///     ) -> Option<String> {
///         *adjusted_range = Some(range.clone());
///         let start = gpui::utf16_to_utf8_offset(&self.text, range.start);
///         let end = gpui::utf16_to_utf8_offset(&self.text, range.end);
///         Some(self.text[start..end].to_string())
///     }
///
///     fn selected_text_range(
///         &mut self,
///         _ignore_disabled_input: bool,
///         _window: &mut Window,
///         _cx: &mut Context<Self>,
///     ) -> Option<UTF16Selection> {
///         let end = gpui::utf8_to_utf16_offset(&self.text, self.text.len());
///         Some(UTF16Selection { range: end..end, reversed: false })
///     }
///
///     fn marked_text_range(&self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
///         None
///     }
///
///     fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}
///
///     fn replace_text_in_range(
///         &mut self,
///         range: Option<Range<usize>>,
///         text: &str,
///         _window: &mut Window,
///         cx: &mut Context<Self>,
///     ) {
///         let range = range.unwrap_or_else(|| {
///             let end = gpui::utf8_to_utf16_offset(&self.text, self.text.len());
///             end..end
///         });
///         let start = gpui::utf16_to_utf8_offset(&self.text, range.start);
///         let end = gpui::utf16_to_utf8_offset(&self.text, range.end);
///         self.text.replace_range(start..end, text);
///         cx.notify();
///     }
///
///     fn replace_and_mark_text_in_range(
///         &mut self,
///         range: Option<Range<usize>>,
///         new_text: &str,
///         _new_selected_range: Option<Range<usize>>,
///         window: &mut Window,
///         cx: &mut Context<Self>,
///     ) {
///         self.replace_text_in_range(range, new_text, window, cx);
///     }
///
///     fn bounds_for_range(
///         &mut self,
///         _range: Range<usize>,
///         element_bounds: Bounds<Pixels>,
///         _window: &mut Window,
///         _cx: &mut Context<Self>,
///     ) -> Option<Bounds<Pixels>> {
///         Some(element_bounds)
///     }
///
///     fn character_index_for_point(
///         &mut self,
///         _point: Point<Pixels>,
///         _window: &mut Window,
///         _cx: &mut Context<Self>,
///     ) -> Option<usize> {
///         Some(0)
///     }
/// }
///
/// impl Render for Field {
///     fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
///         let view = cx.entity();
///         let focus_handle = self.focus_handle.clone();
///         div().size_full().track_focus(&self.focus_handle).child(
///             canvas(
///                 |_, _, _| {},
///                 move |bounds, _, window, cx| {
///                     window.handle_input(&focus_handle, ElementInputHandler::new(bounds, view), cx);
///                 },
///             )
///             .size_full(),
///         )
///     }
/// }
///
/// let mut app = TestApp::new();
/// let mut window = app.open_window(|window, cx| {
///     let focus_handle = cx.focus_handle();
///     window.focus(&focus_handle, cx);
///     Field { focus_handle, text: String::new() }
/// });
///
/// window.simulate_input("Ada");
/// assert_eq!(window.read(|field, _| field.text.clone()), "Ada");
/// ```
pub trait EntityInputHandler: 'static + Sized {
    /// Returns the text in a UTF-16 range, for the platform to read back.
    ///
    /// The platform uses this to read text around the cursor, for example for
    /// autocorrect or the character picker. If you return text for a different
    /// range than was asked for, such as one snapped to a character boundary,
    /// write the range you returned into `adjusted_range`.
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<String>;

    /// Returns the current selection as a UTF-16 range and direction.
    ///
    /// Returns `None` if there is no selection to report. `ignore_disabled_input`
    /// is passed through from the platform, and each implementation decides what it
    /// means. The bundled text elements ignore it.
    fn selected_text_range(
        &mut self,
        ignore_disabled_input: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<UTF16Selection>;

    /// Returns the UTF-16 range of the text in the current IME composition, or
    /// `None` if no composition is active.
    ///
    /// Composition is the text an input method is still building, such as the
    /// underlined kana before a Japanese conversion is confirmed.
    fn marked_text_range(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Range<usize>>;

    /// Ends the IME composition and keeps its text as ordinary text.
    ///
    /// The platform calls this when it finishes composing. After this call,
    /// [`EntityInputHandler::marked_text_range`] returns `None`.
    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>);

    /// Inserts clipboard contents at the selection, for platforms that deliver
    /// paste as an input event, such as the web.
    ///
    /// The default inserts only the plain-text part of `item`, through
    /// [`EntityInputHandler::replace_text_in_range`] with a `None` range. Override
    /// it to handle rich content.
    fn paste(&mut self, item: ClipboardItem, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = item.text() {
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    /// Replaces a UTF-16 range with typed text, or inserts at a cursor.
    ///
    /// This is how typed characters reach your field. A `None` range means the
    /// platform gave no range. The bundled text elements then replace the marked
    /// text if there is any, and otherwise the selection.
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    );

    /// Replaces a range with IME text and marks that text as composing.
    ///
    /// The platform calls this for each step of an input method's composition.
    /// The marked text is underlined and stays editable until
    /// [`EntityInputHandler::unmark_text`] is called. `new_selected_range` is the
    /// selection within `new_text`, in UTF-16 units, counted from the start of
    /// `new_text`. When it is `None`, the cursor goes to the end of the new text.
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    );

    /// Returns where a UTF-16 range is drawn, so the platform can place its
    /// candidate window and other overlays next to the text.
    ///
    /// `element_bounds` is the bounds of the element that installed this handler.
    /// Return `None` when the text cannot be measured. Returned bounds are in window
    /// coordinates, in logical pixels.
    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>>;

    /// Returns the UTF-16 index of the character at `point`, which the platform uses
    /// to map a screen position to text.
    ///
    /// `point` is in window coordinates, in logical pixels.
    fn character_index_for_point(
        &mut self,
        point: crate::Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize>;

    /// Sets the selection to a UTF-16 range, when the platform moves it.
    ///
    /// Platforms call this for system-driven selection changes, such as a dragged
    /// selection handle or Select All from system UI. It is the reverse of
    /// [`EntityInputHandler::selected_text_range`]. The default does nothing.
    fn set_selected_text_range(
        &mut self,
        _range_utf16: Range<usize>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
    }

    /// Returns the length of the document in UTF-16 code units, or `None` if it is
    /// not known. The default is `None`.
    fn text_length_utf16(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }

    /// Returns whether the field accepts typed text right now.
    ///
    /// Return `false` for a read-only or disabled field. The default is `true`.
    /// For a keystroke with [`KeyDownEvent::prefer_character_input`](crate::KeyDownEvent::prefer_character_input) set, the window
    /// sends the keystroke to text input instead of key bindings only when this
    /// returns `true`.
    fn accepts_text_input(&self, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        true
    }

    /// Returns platform text assistance settings for this field, such as
    /// autocorrect and the action key on the software keyboard.
    ///
    /// The window asks on every frame and forwards the result only when it
    /// changes, so return a cheap value. You can vary it with application state,
    /// such as the cursor position. The default disables text assistance.
    fn text_input_configuration(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> TextInputConfiguration {
        TextInputConfiguration::default()
    }

    /// Returns the UTF-16 range that the platform may read and edit around the
    /// selection, or `None` for no limit.
    ///
    /// Platforms that mirror text into an IME-editable buffer use this range to
    /// keep multi-step edits, such as word deletion and autocorrect, inside the
    /// region you choose. The range should contain the selection. The default is
    /// `None`.
    fn text_input_editable_range(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        None
    }
}

/// Connects an [`EntityInputHandler`] view to the platform's text input.
///
/// This is the usual way to install an [`EntityInputHandler`]. It implements
/// [`InputHandler`] by forwarding each call to the view, with the element's
/// bounds filled in for the methods that need them. Create one during paint and
/// pass it to [`Window::handle_input`](crate::Window::handle_input). The handler is active for the next
/// frame only, so install it again on every paint.
pub struct ElementInputHandler<V> {
    view: Entity<V>,
    element_bounds: Bounds<Pixels>,
}

impl<V: 'static> ElementInputHandler<V> {
    /// Creates a handler that forwards platform text input to `view`.
    ///
    /// Pass the bounds of the element that is painting. They are returned from
    /// [`InputHandler::element_bounds`] and passed to
    /// [`EntityInputHandler::bounds_for_range`]. Call this in
    /// [`Element::paint`][element_paint], then pass the handler to
    /// [`Window::handle_input`](crate::Window::handle_input). The window installs the handler only while the
    /// given focus handle is focused.
    ///
    /// [element_paint]: crate::Element::paint
    pub fn new(element_bounds: Bounds<Pixels>, view: Entity<V>) -> Self {
        ElementInputHandler {
            view,
            element_bounds,
        }
    }
}

impl<V: EntityInputHandler> InputHandler for ElementInputHandler<V> {
    fn selected_text_range(
        &mut self,
        ignore_disabled_input: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<UTF16Selection> {
        self.view.update(cx, |view, cx| {
            view.selected_text_range(ignore_disabled_input, window, cx)
        })
    }

    fn marked_text_range(&mut self, window: &mut Window, cx: &mut App) -> Option<Range<usize>> {
        self.view
            .update(cx, |view, cx| view.marked_text_range(window, cx))
    }

    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<String> {
        self.view.update(cx, |view, cx| {
            view.text_for_range(range_utf16, adjusted_range, window, cx)
        })
    }

    fn replace_text_in_range(
        &mut self,
        replacement_range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.view.update(cx, |view, cx| {
            view.replace_text_in_range(replacement_range, text, window, cx)
        });
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.view.update(cx, |view, cx| {
            view.replace_and_mark_text_in_range(
                range_utf16,
                new_text,
                new_selected_range,
                window,
                cx,
            )
        });
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut App) {
        self.view
            .update(cx, |view, cx| view.unmark_text(window, cx));
    }

    fn paste(&mut self, item: ClipboardItem, window: &mut Window, cx: &mut App) {
        self.view
            .update(cx, |view, cx| view.paste(item, window, cx));
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Bounds<Pixels>> {
        self.view.update(cx, |view, cx| {
            view.bounds_for_range(range_utf16, self.element_bounds, window, cx)
        })
    }

    fn character_index_for_point(
        &mut self,
        point: crate::Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<usize> {
        self.view.update(cx, |view, cx| {
            view.character_index_for_point(point, window, cx)
        })
    }

    fn set_selected_text_range(
        &mut self,
        range_utf16: Range<usize>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.view.update(cx, |view, cx| {
            view.set_selected_text_range(range_utf16, window, cx)
        })
    }

    fn element_bounds(&mut self, _window: &mut Window, _cx: &mut App) -> Option<Bounds<Pixels>> {
        Some(self.element_bounds)
    }

    fn text_length_utf16(&mut self, window: &mut Window, cx: &mut App) -> Option<usize> {
        self.view
            .update(cx, |view, cx| view.text_length_utf16(window, cx))
    }

    fn accepts_text_input(&mut self, window: &mut Window, cx: &mut App) -> bool {
        self.view
            .update(cx, |view, cx| view.accepts_text_input(window, cx))
    }

    fn prefers_ime_for_printable_keys(&mut self, window: &mut Window, cx: &mut App) -> bool {
        self.view
            .update(cx, |view, cx| view.accepts_text_input(window, cx))
    }

    fn text_input_configuration(
        &mut self,
        window: &mut Window,
        cx: &mut App,
    ) -> TextInputConfiguration {
        self.view
            .update(cx, |view, cx| view.text_input_configuration(window, cx))
    }

    fn text_input_editable_range(
        &mut self,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Range<usize>> {
        self.view
            .update(cx, |view, cx| view.text_input_editable_range(window, cx))
    }
}

#[cfg(test)]
mod configuration_tests {
    use super::*;
    use crate::{
        AnyWindowHandle, AppContext as _, FocusHandle, InteractiveElement as _, IntoElement,
        ParentElement as _, Render, Styled as _, TestAppContext, TextInputAction,
        TextInputStateChange, canvas, div,
    };

    #[gpui::test]
    fn text_input_configuration_and_focus_state_are_forwarded_on_change(cx: &mut TestAppContext) {
        let custom = TextInputConfiguration {
            autocorrect: true,
            input_action: TextInputAction::Send,
            ..Default::default()
        };
        let window = cx.add_window({
            let custom = custom.clone();
            move |_, cx| ConfigurationTestView {
                focus_handle: cx.focus_handle(),
                configuration: custom,
            }
        });
        let view = window.root(cx).unwrap();
        let test_window = cx.test_window(window.into());
        let window = AnyWindowHandle::from(window);
        let draw = |cx: &mut TestAppContext| {
            cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
        };

        // Nothing is focused, so the platform learns the default configuration.
        draw(cx);
        assert_eq!(
            test_window.text_input_configurations(),
            vec![TextInputConfiguration::default()]
        );
        assert!(test_window.text_input_state_changes().is_empty());

        // Focusing the view routes its configuration to the platform.
        cx.update_window(window, |_, window, cx| {
            let focus_handle = view.read(cx).focus_handle.clone();
            window.focus(&focus_handle, cx);
        })
        .unwrap();
        draw(cx);
        assert_eq!(
            test_window.text_input_configurations(),
            vec![TextInputConfiguration::default(), custom.clone()]
        );
        assert_eq!(
            test_window.text_input_state_changes(),
            vec![TextInputStateChange::FocusGained]
        );

        // Redrawing without a change forwards nothing.
        draw(cx);
        assert_eq!(test_window.text_input_configurations().len(), 2);
        assert_eq!(test_window.text_input_state_changes().len(), 1);

        // Changing the configuration forwards the new value.
        let updated = TextInputConfiguration {
            suggestions: true,
            ..custom
        };
        view.update(cx, {
            let updated = updated.clone();
            |view, cx| {
                view.configuration = updated;
                cx.notify();
            }
        });
        draw(cx);
        assert_eq!(
            test_window.text_input_configurations().last(),
            Some(&updated)
        );
        assert_eq!(test_window.text_input_configurations().len(), 3);
        assert_eq!(test_window.text_input_state_changes().len(), 1);

        // Losing focus reverts the platform to the default configuration.
        cx.update_window(window, |_, window, cx| window.blur(cx))
            .unwrap();
        draw(cx);
        assert_eq!(
            test_window.text_input_configurations().last(),
            Some(&TextInputConfiguration::default())
        );
        assert_eq!(test_window.text_input_configurations().len(), 4);
        assert_eq!(
            test_window.text_input_state_changes(),
            vec![
                TextInputStateChange::FocusGained,
                TextInputStateChange::FocusLost
            ]
        );
    }

    struct ConfigurationTestView {
        focus_handle: FocusHandle,
        configuration: TextInputConfiguration,
    }

    impl Render for ConfigurationTestView {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let view = cx.entity();
            let focus_handle = self.focus_handle.clone();
            div().size_full().track_focus(&self.focus_handle).child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        window.handle_input(
                            &focus_handle,
                            ElementInputHandler::new(bounds, view),
                            cx,
                        );
                    },
                )
                .size_full(),
            )
        }
    }

    impl EntityInputHandler for ConfigurationTestView {
        fn text_for_range(
            &mut self,
            _range: std::ops::Range<usize>,
            _adjusted_range: &mut Option<std::ops::Range<usize>>,
            _window: &mut Window,
            _cx: &mut Context<Self>,
        ) -> Option<String> {
            None
        }

        fn selected_text_range(
            &mut self,
            _ignore_disabled_input: bool,
            _window: &mut Window,
            _cx: &mut Context<Self>,
        ) -> Option<UTF16Selection> {
            None
        }

        fn marked_text_range(
            &self,
            _window: &mut Window,
            _cx: &mut Context<Self>,
        ) -> Option<std::ops::Range<usize>> {
            None
        }

        fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

        fn replace_text_in_range(
            &mut self,
            _range: Option<std::ops::Range<usize>>,
            _text: &str,
            _window: &mut Window,
            _cx: &mut Context<Self>,
        ) {
        }

        fn replace_and_mark_text_in_range(
            &mut self,
            _range: Option<std::ops::Range<usize>>,
            _new_text: &str,
            _new_selected_range: Option<std::ops::Range<usize>>,
            _window: &mut Window,
            _cx: &mut Context<Self>,
        ) {
        }

        fn bounds_for_range(
            &mut self,
            _range_utf16: std::ops::Range<usize>,
            _element_bounds: Bounds<Pixels>,
            _window: &mut Window,
            _cx: &mut Context<Self>,
        ) -> Option<Bounds<Pixels>> {
            None
        }

        fn character_index_for_point(
            &mut self,
            _point: crate::Point<Pixels>,
            _window: &mut Window,
            _cx: &mut Context<Self>,
        ) -> Option<usize> {
            None
        }

        fn text_input_configuration(
            &mut self,
            _window: &mut Window,
            _cx: &mut Context<Self>,
        ) -> TextInputConfiguration {
            self.configuration.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_offsets_convert_across_unicode_encodings() {
        let cases = [
            ("", vec![(0, 0)]),
            ("plain", vec![(0, 0), (1, 1), (5, 5)]),
            (
                "A😀日本e\u{301}",
                vec![(0, 0), (1, 1), (5, 3), (8, 4), (11, 5), (12, 6), (14, 7)],
            ),
        ];

        for (text, boundaries) in cases {
            for (utf8, utf16) in boundaries {
                assert_eq!(utf8_to_utf16_offset(text, utf8), utf16, "{text:?}");
                assert_eq!(utf16_to_utf8_offset(text, utf16), utf8, "{text:?}");
            }

            assert_eq!(
                utf8_to_utf16_offset(text, usize::MAX),
                text.encode_utf16().count()
            );
            assert_eq!(utf16_to_utf8_offset(text, usize::MAX), text.len());
        }

        let text = "A😀B";
        assert_eq!(utf16_to_utf8_offset(text, 2), 5);
        assert_eq!(utf8_to_utf16_offset(text, 2), 3);
    }
}
