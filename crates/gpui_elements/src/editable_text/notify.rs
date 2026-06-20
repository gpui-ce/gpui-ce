use std::{ops::Range, time::Instant};

use crate::editable_text::UnicodeTextStorage;

pub struct TextChanged;

pub struct TextHistoryPushed {
    pub timestamp: Instant,
    pub modified_range: Range<usize>,
    pub text_payload: String,
    pub new_length: usize,
    pub selected_range: Range<usize>,
}
impl TextHistoryPushed {
    pub fn new(
        range: Range<usize>,
        new_length: usize,
        storage: impl UnicodeTextStorage,
        selected_range: Range<usize>,
    ) -> Self {
        let timestamp = Instant::now();
        let modified_range = range.start..range.start + new_length;
        // NOTE: not performant to allocate a new text payload if the event doesnt
        // need to be logged (based on timestamp). Should consider a more robust way to access
        // the storage only if it absolutely needs to be cloned from.
        let text_payload = storage.content_utf8()[range].to_string();
        Self {
            timestamp,
            modified_range,
            text_payload,
            new_length,
            selected_range,
        }
    }

    pub fn convert_to_redo(self, content: &str) -> Self {
        let undo_start = self.modified_range.start;
        let undo_end = (self.modified_range.start + self.new_length).min(content.len());
        let text_payload = content[undo_start..undo_end].to_string();
        let new_length = self.text_payload.len();
        Self {
            timestamp: self.timestamp,
            modified_range: undo_start..undo_start + self.text_payload.len(),
            text_payload,
            new_length,
            selected_range: self.selected_range,
        }
    }
}
