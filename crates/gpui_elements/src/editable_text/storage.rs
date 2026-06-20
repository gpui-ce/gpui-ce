use gpui::{App, NavigationDirection};
use std::{ops::Range, rc::Rc};
use unicode_segmentation::UnicodeSegmentation;

pub enum TextBoundary {
    /// The next utf-8 character in a direction from the caret
    Graphmeme,
    /// The next word in a direction from the caret
    Word,
    /// The start/end of the current line
    Line,
    /// The rest of the text to the start/end of a document
    Document,
}

#[derive(Clone, Default)]
pub(super) struct InitStorage(Option<Rc<dyn Fn(&mut App) -> Box<dyn UnicodeTextStorage>>>);
impl InitStorage {
    pub fn exec(&self, cx: &mut App) -> Box<dyn UnicodeTextStorage> {
        match &self.0 {
            None => Box::new(String::new()),
            Some(init) => (*init)(cx),
        }
    }
}

pub trait UnicodeTextStorage {
    /// Returns a reference to the utf8 string.
    fn content_utf8(&self) -> &str;

    /// Returns the UTF-16 length of the content.
    fn len_utf16(&self) -> usize;

    fn replace_range(&mut self, range: Range<usize>, text: &str);

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

    /// Returns the utf-8 character position of first character after the first new-line preceeding the character at the provided utf-8 character position.
    fn find_line_start(&self, position: usize) -> usize {
        let content = self.content_utf8();
        content[..position.min(content.len())]
            .rfind('\n')
            .map(|pos| pos + 1)
            .unwrap_or(0)
    }

    /// Returns the utf-8 character position of the character immediately before the first new-line character after the character at the provided utf-8 character position.
    fn find_line_end(&self, position: usize) -> usize {
        let content = self.content_utf8();
        content[position.min(content.len())..]
            .find('\n')
            .map(|pos| position + pos)
            .unwrap_or(content.len())
    }

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

    fn offset_from_caret(
        &self,
        caret: usize,
        direction: NavigationDirection,
        magnitude: TextBoundary,
    ) -> usize {
        use NavigationDirection::*;
        use TextBoundary::*;
        match (direction, magnitude) {
            (Back, Graphmeme) => self.previous_boundary(caret),
            (Forward, Graphmeme) => self.next_boundary(caret),
            (Back, Word) => self.previous_word_boundary(caret),
            (Forward, Word) => self.next_word_boundary(caret),
            (Back, Line) => self.find_line_start(caret),
            (Forward, Line) => self.find_line_end(caret),
            (Back, Document) => 0,
            (Forward, Document) => self.content_utf8().len(),
        }
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

    fn replace_range(&mut self, range: Range<usize>, text: &str) {
        self.replace_range(range, &text);
    }
}
