/// A unit of text that the caret moves or selects by.
///
/// The variants are `Cluster` (one user-perceived character, such as an emoji
/// with modifiers), `Word`, `VisualLine`, `HardLine` and `Document`. The
/// default storage treats both line variants as the text between `\n`
/// characters, and doesn't wrap lines itself.
pub use gpui::TextBoundary;
use gpui::{NavigationDirection, utf8_to_utf16_offset, utf16_to_utf8_offset};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// Stores the text of an editable text field.
///
/// Implement this trait to store the content in a type other than a
/// [`String`]. For example, a rope suits a large document better, because
/// [`StringStorage`] scans the whole string to compute UTF-16 lengths.
/// Editable text elements use [`StringStorage`] by default.
///
/// Positions are UTF-8 byte offsets, and the UTF-16 conversions exist because
/// the platform's input method reports positions in UTF-16 code units. The
/// default methods provide the navigation and conversions, so an
/// implementation needs only the required methods.
pub trait UnicodeTextStorage {
    /// Returns the version of the content.
    ///
    /// Increase the version on every change, so that the element knows to
    /// lay out the text again. The version wraps around after `u16::MAX`.
    fn version(&self) -> u16;

    /// Returns the content as a UTF-8 string.
    fn content_utf8(&self) -> &str;

    /// Returns the length of the content in UTF-16 code units.
    fn len_utf16(&self) -> usize;

    /// Replaces the text in `range` with `text`.
    ///
    /// `range` is a range of UTF-8 byte offsets. [`StringStorage`] panics if
    /// the range is out of bounds or doesn't start and end on a character
    /// boundary.
    fn replace_range(&mut self, range: Range<usize>, text: &str);

    /// Converts a UTF-8 byte offset to a UTF-16 offset.
    ///
    /// An offset at or past the end of the content returns [`Self::len_utf16`].
    fn utf_offset_8to16(&self, pos_uft8: usize) -> usize {
        // Fast path: if offset is 0, return 0
        if pos_uft8 == 0 {
            return 0;
        }

        // Fast path: if offset is at or past end, return cached length
        if pos_uft8 >= self.content_utf8().len() {
            return self.len_utf16();
        }

        utf8_to_utf16_offset(self.content_utf8(), pos_uft8)
    }

    /// Returns the UTF-8 offset for the UTF-16 offset `pos_utf16`.
    fn utf_offset_16to8(&self, pos_utf16: usize) -> usize {
        // Fast path: if offset is 0, return 0
        if pos_utf16 == 0 {
            return 0;
        }

        utf16_to_utf8_offset(self.content_utf8(), pos_utf16)
    }

    /// Converts a range of UTF-8 offsets into a range of UTF-16 offsets.
    fn utf_range_8to16(&self, range_utf8: &Range<usize>) -> Range<usize> {
        self.utf_offset_8to16(range_utf8.start)..self.utf_offset_8to16(range_utf8.end)
    }

    /// Converts a range of UTF-16 offsets into a range of UTF-8 offsets.
    fn utf_range_16to8(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.utf_offset_16to8(range_utf16.start)..self.utf_offset_16to8(range_utf16.end)
    }

    /// Returns the range between `caret` and the boundary reached by moving in
    /// `direction` by `magnitude`.
    ///
    /// The range is ordered: the start is the earlier offset and the end is the
    /// later one. The range is empty when the caret is already at the boundary.
    fn range_from_caret(
        &self,
        caret: usize,
        direction: NavigationDirection,
        magnitude: TextBoundary,
    ) -> Range<usize> {
        let offset = self.offset_from_caret(caret, direction, magnitude);
        match direction {
            NavigationDirection::Back => offset..caret,
            NavigationDirection::Forward => caret..offset,
        }
    }

    /// Returns the offset that moves `caret` in `direction` to the next `boundary`.
    ///
    /// Moving back from the start, or forward from the end, returns the same
    /// offset. A `boundary` of `Cluster` moves one grapheme cluster, and
    /// `Document` moves to the start or end of the content.
    fn offset_from_caret(
        &self,
        caret: usize,
        direction: NavigationDirection,
        boundary: TextBoundary,
    ) -> usize {
        use NavigationDirection::*;
        use TextBoundary::*;
        match (direction, boundary) {
            (Back, Cluster) => {
                if caret == 0 {
                    return 0;
                }

                let str = self.content_utf8();
                let iter = str[..caret.min(str.len())].grapheme_indices(true);
                iter.map(|(i, _)| i).next_back().unwrap_or(0)
            }
            (Forward, Cluster) => {
                let str = self.content_utf8();
                let len_utf8 = str.len();
                if caret >= len_utf8 {
                    return len_utf8;
                }

                let mut iter = str[caret..].grapheme_indices(true);
                iter.nth(1).map(|(i, _)| caret + i).unwrap_or(len_utf8)
            }
            (Back, Word) => {
                let text = self.content_utf8();
                let prefix = &text[..caret.min(text.len())];

                prefix
                    .unicode_word_indices()
                    .next_back()
                    .map_or(0, |(idx, _word)| idx)
            }
            (Forward, Word) => {
                let text = self.content_utf8();
                let len_utf8 = text.len();

                if caret >= len_utf8 {
                    return len_utf8;
                }

                text[caret..]
                    .unicode_word_indices()
                    .next()
                    .map_or(len_utf8, |(idx, word)| caret + idx + word.len())
            }
            // Returns the UTF-8 byte offset of the first character after the
            // first newline preceding `pos`.
            (Back, VisualLine | HardLine) => {
                let str = self.content_utf8();
                let iter = str[..caret.min(str.len())].rfind('\n');
                iter.map(|pos| pos + 1).unwrap_or(0)
            }
            // Returns the UTF-8 byte offset of the character immediately before
            // the first newline after `pos`.
            (Forward, VisualLine | HardLine) => {
                let str = self.content_utf8();
                let iter = str[caret.min(str.len())..].find('\n');
                iter.map(|pos| caret + pos).unwrap_or(str.len())
            }
            (Back, Document) => 0,
            (Forward, Document) => self.content_utf8().len(),
        }
    }

    /// Returns the range of the word that contains `position`.
    ///
    /// If `position` is not inside a word, returns the empty range at
    /// `position`. `position` is clamped to the content length.
    fn word_range_at(&self, position: usize) -> Range<usize> {
        let offset = position.min(self.content_utf8().len());

        for (idx, word) in self.content_utf8().unicode_word_indices() {
            let word_end = idx + word.len();
            if offset >= idx && offset <= word_end {
                return idx..word_end;
            }
        }

        offset..offset
    }
}

/// The default [`UnicodeTextStorage`], which keeps the content in a [`String`].
///
/// It is simple and fine for a text field. Each call to
/// `UnicodeTextStorage::len_utf16` scans the whole string, so for large
/// documents implement your own storage. Build one from any `Into<String>`
/// value with `StringStorage::from`.
///
/// ```
/// use gpui_ce_elements::editable_text::{StringStorage, UnicodeTextStorage};
///
/// let storage = StringStorage::from("héllo");
/// assert_eq!(storage.content_utf8(), "héllo");
/// assert_eq!(storage.len_utf16(), 5);
/// assert_eq!(storage.version(), 0);
/// ```
#[derive(Clone, Default)]
pub struct StringStorage {
    value: String,
    version: u16,
}
impl<S> From<S> for StringStorage
where
    S: Into<String>,
{
    fn from(value: S) -> Self {
        Self {
            value: value.into(),
            version: u16::default(),
        }
    }
}
impl UnicodeTextStorage for StringStorage {
    fn version(&self) -> u16 {
        self.version
    }

    fn content_utf8(&self) -> &str {
        self.value.as_str()
    }

    fn len_utf16(&self) -> usize {
        self.value.chars().map(|c| c.len_utf16()).sum()
    }

    fn replace_range(&mut self, range: Range<usize>, text: &str) {
        self.value.replace_range(range, &text);
        self.version = self.version.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_navigation_handles_boundaries_and_scripts() {
        for (text, caret, direction, expected) in [
            ("", 0, NavigationDirection::Back, 0),
            ("", 0, NavigationDirection::Forward, 0),
            ("   ", 3, NavigationDirection::Back, 0),
            ("   ", 0, NavigationDirection::Forward, 3),
            ("hello world", 11, NavigationDirection::Back, 6),
            ("hello world", 0, NavigationDirection::Forward, 5),
            ("hello world", 2, NavigationDirection::Forward, 5),
            (", hi!", 5, NavigationDirection::Back, 2),
            (", hi!", 0, NavigationDirection::Forward, 4),
            ("a 日本語 b", 11, NavigationDirection::Back, 8),
            ("a 日本語 b", 2, NavigationDirection::Forward, 5),
            ("hi", 100, NavigationDirection::Back, 0),
            ("hi", 100, NavigationDirection::Forward, 2),
        ] {
            let storage = StringStorage::from(text);

            assert_eq!(
                storage.offset_from_caret(caret, direction, TextBoundary::Word),
                expected,
                "text {text:?}, caret {caret}, direction {direction:?}",
            );
        }
    }
}
