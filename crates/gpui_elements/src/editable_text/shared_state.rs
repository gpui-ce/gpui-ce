use crate::editable_text::UnicodeTextStorage;
use gpui::{App, FocusHandle, Focusable, NavigationDirection, Pixels, Point, UTF16Selection};
use std::ops::Range;

pub struct TextInputStateBase {
    storage: Box<dyn UnicodeTextStorage>,

    /// The utf-8 character range that is currently selected by the user.
    /// Valid both when start < end and start > end (which dictates the direction of the selection). Empty when start==end.
    /// The start of this range is always the current position of the caret (input cursor).
    ///
    /// NOTE: because each input has its own selection state, its trivial for users to have multiple selections active across multiple inputs at the same time.
    /// This could be considered undesirable behavior, and could prompt the question of whether there should be a mechanism to clear selection when focus is lost.
    selected_range: Range<usize>,

    /// The utf-8 character range of `storage` which is being composed by IME
    marked_range: Option<Range<usize>>,

    /// True while the user is in the act of highlighting a section of the text (e.g. during mouse pressed & dragging).
    is_selecting: bool,
    /// The last ui location relative to the element that the user clicked. Used to filter when a user clicks multiple times in the same area.
    last_click_position: Option<Point<Pixels>>,
    /// The number of times the user has clicked `last_click_position`. Used to determine which click behavior to trigger, depending on single, double, or triple clicks.
    click_count: usize,

    focus_handle: FocusHandle,
}

impl Focusable for TextInputStateBase {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TextInputStateBase {
    /// Creates a new `Input` with the specified multiline setting.
    /// Cursor blinking is enabled by default.
    pub fn new(storage: impl Into<Box<dyn UnicodeTextStorage>>, cx: &mut App) -> Self {
        Self {
            storage: storage.into(),

            selected_range: 0..0,
            marked_range: None,

            is_selecting: false,
            last_click_position: None,
            click_count: 0,

            focus_handle: cx.focus_handle(),
        }
    }

    pub fn storage(&self) -> &Box<dyn UnicodeTextStorage> {
        &self.storage
    }

    /// Returns the utf-8 character range that is currently selected within the current state of the text.
    /// Internally converts the stored direction-aware range into a canonical range.
    pub fn selected_range(&self) -> Range<usize> {
        self.selected_range.start.min(self.selected_range.end)
            ..self.selected_range.start.max(self.selected_range.end)
    }

    pub fn selection_direction(&self) -> Option<NavigationDirection> {
        match self.selected_range.start.cmp(&self.selected_range.end) {
            std::cmp::Ordering::Less => Some(NavigationDirection::Forward),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(NavigationDirection::Back),
        }
    }

    pub fn caret_pos(&self) -> usize {
        self.selected_range.start
    }
}

impl TextInputStateBase {
    pub fn ime_text_for_range(
        &self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
    ) -> Option<String> {
        let range = self.storage.utf_range_16to8(&range_utf16);
        let storage_len_utf8 = self.storage.content_utf8().len();
        let clamped_range = range.start.min(storage_len_utf8)..range.end.min(storage_len_utf8);
        adjusted_range.replace(self.storage.utf_range_8to16(&clamped_range));
        Some(self.storage.content_utf8()[clamped_range].to_string())
    }

    pub fn ime_selected_text_range(&self, _ignore_disabled_input: bool) -> Option<UTF16Selection> {
        let selection_range = self.selected_range();
        let direction = self.selection_direction();
        Some(UTF16Selection {
            range: self.storage.utf_range_8to16(&selection_range),
            reversed: direction == Some(NavigationDirection::Back),
        })
    }

    pub fn ime_marked_text_range(&self) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.storage.utf_range_8to16(range))
    }

    pub fn ime_unmark_text(&mut self) {
        self.marked_range = None;
    }

    pub fn ime_resolve_range(&self, range_utf16: Option<Range<usize>>) -> Range<usize> {
        // Use a series of fallbacks to pick the range to operate on.
        // Fallback order: IME provided range, active IME marked range, selection
        let range = range_utf16.map(|range_utf16| self.storage.utf_range_16to8(&range_utf16));
        let range = range.or_else(|| self.marked_range.clone());
        let range = range.unwrap_or_else(|| self.selected_range());

        let storage_len_utf8 = self.storage().content_utf8().len();
        range.start.min(storage_len_utf8)..range.end.min(storage_len_utf8)
    }

    pub fn replace_text(&mut self, start: usize, end: usize, new_text: &str) {
        let storage_len_utf8 = self.storage.content_utf8().len();
        let start = start.min(storage_len_utf8);
        let end = end.max(start).min(storage_len_utf8);
        self.storage.replace_range(start..end, new_text);

        let new_caret = start + new_text.len();
        self.selected_range = new_caret..new_caret;
    }

    pub fn replace_text_in_range_bytes(&mut self, range: Range<usize>, mut text_to_insert: &str) {
        // TODO: Apply text sanitization
        // single-line fields should prune \n and \r
        // fields should be able to provide a max_length or other validations on text-input

        let max_length = None::<usize>;

        // Decide the effective new text up front (honouring `max_length`).
        // This avoids the "apply, then truncate" path which would leave the caret past the end.
        if let Some(cap) = max_length {
            let existing_len = self.storage().content_utf8().len() - (range.end - range.start);
            let room = cap.saturating_sub(existing_len);
            text_to_insert = &text_to_insert[..text_to_insert.len().min(room)];
        }

        // TODO: Push history diff
        // self.push_undo_patch(range.clone(), text_to_insert.len());

        self.storage.replace_range(range, text_to_insert);
        self.marked_range = None;

        // TODO: caller emits events
    }

    pub fn ime_mark_text_in_range(&mut self, range: &Range<usize>, text_len: usize) {
        self.marked_range = match text_len {
            0 => None,
            _ => Some(range.start..range.start + text_len),
        };
    }

    pub fn ime_mark_selected_range(
        &mut self,
        range_overwritten: &Range<usize>,
        new_selected_range_utf16: &Option<Range<usize>>,
        text_len: usize,
    ) {
        // NOTE: Differs from yororen-ui
        // https://github.com/MeowLynxSea/yororen-ui/blob/346502ac654b77fdaff3be2d7444fca8783acfc9/crates/yororen-ui-core/src/headless/text_input_core.rs#L359-L371
        self.selected_range = {
            let new_range = new_selected_range_utf16.as_ref();
            let new_range = new_range.map(|range_utf16| self.storage.utf_range_16to8(range_utf16));
            let new_range = new_range.map(|new_range| {
                new_range.start + range_overwritten.start..new_range.end + range_overwritten.start
            });
            new_range.unwrap_or_else(|| {
                range_overwritten.start + text_len..range_overwritten.start + text_len
            })
        };
    }
}
