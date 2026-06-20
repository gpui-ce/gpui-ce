use gpui::SharedString;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

pub trait UnicodeTextStorage {
    /// Returns a reference to the utf8 string.
    fn content_utf8(&self) -> &str;

    /// Returns the UTF-16 length of the content.
    fn len_utf16(&self) -> usize;

    fn utf_offset_8to16(&self, pos_uft8: usize) -> usize {
        // Fast path: if offset is 0, return 0
        if pos_uft8 == 0 {
            return 0;
        }

        // Fast path: if offset is at or past end, return cached length
        if pos_uft8 >= self.content_utf8().len() {
            return self.len_utf16();
        }

        let mut count_utf16 = 0;
        for (idx, character) in self.content_utf8().char_indices() {
            if idx >= pos_uft8 {
                break;
            }
            count_utf16 += character.len_utf16();
        }
        count_utf16
    }

    fn utf_offset_16to8(&self, pos_utf16: usize) -> usize {
        // Fast path: if offset is 0, return 0
        if pos_utf16 == 0 {
            return 0;
        }

        let mut count_utf16 = 0;
        for (idx, character) in self.content_utf8().char_indices() {
            if count_utf16 >= pos_utf16 {
                return idx;
            }
            count_utf16 += character.len_utf16();
        }
        self.content_utf8().len()
    }

    fn utf_range_8to16(&self, range_utf8: &Range<usize>) -> Range<usize> {
        self.utf_offset_8to16(range_utf8.start)..self.utf_offset_8to16(range_utf8.end)
    }

    fn utf_range_16to8(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.utf_offset_16to8(range_utf16.start)..self.utf_offset_16to8(range_utf16.end)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        if offset == 0 {
            return 0;
        }

        let text_before = &self.content_utf8()[..offset.min(self.content_utf8().len())];
        text_before
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .next_back()
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        let len_utf8 = self.content_utf8().len();
        if offset >= len_utf8 {
            return len_utf8;
        }

        let text_after = &self.content_utf8()[offset..];
        text_after
            .grapheme_indices(true)
            .nth(1)
            .map(|(i, _)| offset + i)
            .unwrap_or(len_utf8)
    }

    fn previous_word_boundary(&self, offset: usize) -> usize {
        if offset == 0 {
            return 0;
        }

        let text_before = &self.content_utf8()[..offset.min(self.content_utf8().len())];

        let mut last_word_start = 0;
        for (idx, _) in text_before.unicode_word_indices() {
            if idx < offset {
                last_word_start = idx;
            }
        }

        if last_word_start == 0 && offset > 0 {
            let trimmed = text_before.trim_end();
            if trimmed.is_empty() {
                return 0;
            }
            for (idx, _) in trimmed.unicode_word_indices() {
                last_word_start = idx;
            }
        }

        last_word_start
    }

    fn next_word_boundary(&self, offset: usize) -> usize {
        let len_utf8 = self.content_utf8().len();
        if offset >= len_utf8 {
            return len_utf8;
        }

        let text_after = &self.content_utf8()[offset..];

        for (idx, word) in text_after.unicode_word_indices() {
            let word_end = offset + idx + word.len();
            if word_end > offset {
                return word_end;
            }
        }

        len_utf8
    }

    fn word_range_at(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.content_utf8().len());

        for (idx, word) in self.content_utf8().unicode_word_indices() {
            let word_end = idx + word.len();
            if offset >= idx && offset <= word_end {
                return (idx, word_end);
            }
        }

        (offset, offset)
    }
}

impl UnicodeTextStorage for String {
    fn content_utf8(&self) -> &str {
        self.as_str()
    }

    fn len_utf16(&self) -> usize {
        self.len()
    }
}

impl UnicodeTextStorage for SharedString {
    fn content_utf8(&self) -> &str {
        self.as_str()
    }

    fn len_utf16(&self) -> usize {
        self.len()
    }
}
