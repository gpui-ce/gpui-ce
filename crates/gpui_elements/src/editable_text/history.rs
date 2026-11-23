use gpui::CaretSelection;
use std::{ops::Range, time::Duration};
use web_time::Instant;

/// Maximum number of history entries to keep.
pub const MAX_HISTORY_LEN: usize = 1000;

/// Default interval for grouping consecutive edits into a single undo entry.
pub const DEFAULT_GROUP_INTERVAL: Duration = Duration::from_millis(300);

// TODO: Should history get attached directly to storage? currently its per text field and operate both on storage and selection
pub struct EditableTextHistory {
    /// The maximum duration between changes to `content` that can be grouped together as a single entry in the history log.
    grouping_interval: Duration,
    /// Stack of previous states for undo.
    undo_stack: Vec<HistoryEntry>,
    /// Stack of undone states for redo.
    redo_stack: Vec<HistoryEntry>,
}
impl Default for EditableTextHistory {
    fn default() -> Self {
        Self {
            grouping_interval: DEFAULT_GROUP_INTERVAL,
            undo_stack: Vec::with_capacity(MAX_HISTORY_LEN),
            redo_stack: Vec::with_capacity(MAX_HISTORY_LEN),
        }
    }
}

/// A patch-based history entry for memory-efficient undo/redo operations.
/// Instead of storing the full content, we store only the change needed to reverse the edit.
#[derive(Clone, Debug)]
pub struct HistoryEntry {
    /// The byte range that was modified (after the edit, for undo; before the edit, for redo).
    pub range: Range<usize>,
    /// The text that was replaced (to restore on undo).
    pub old_text: String,
    /// The length of the new text that replaced old_text (to know how much to remove on undo).
    pub new_text_len: usize,
    /// The selection range before the edit.
    pub selected_range: CaretSelection,
    /// Timestamp for grouping consecutive edits.
    pub timestamp: Instant,
}

#[derive(Clone, Debug)]
pub enum HistoryKind {
    Undo,
    Redo,
}

impl EditableTextHistory {
    pub fn set_grouping_interval(&mut self, interval: Duration) {
        self.grouping_interval = interval;
    }

    pub fn record(
        &mut self,
        range: Range<usize>,
        old_text: &str,
        new_text_len: usize,
        selected_range: CaretSelection,
    ) {
        let now = Instant::now();

        // Any new edit invalidates the redo stack, including one that joins an earlier group.
        self.redo_stack.clear();

        // Check if we should group with the last entry
        if let Some(last) = self.undo_stack.last_mut()
            && now.duration_since(last.timestamp) < self.grouping_interval
        {
            // The change was triggered within group interval timing.
            // Try to extend the existing patch (which is a mutation).
            // If extending successeds, then we can early-out. Otherwise the mutation is non-contiguous.
            if last.extend(&range, old_text, new_text_len) {
                return;
            }
        }

        // Limit history size
        if self.undo_stack.len() >= MAX_HISTORY_LEN {
            self.undo_stack.remove(0);
        }

        self.push(
            HistoryKind::Undo,
            HistoryEntry {
                range: range.start..range.start + new_text_len,
                old_text: old_text.to_string(),
                new_text_len,
                selected_range,
                timestamp: now,
            },
        );
    }

    fn stack(&self, kind: HistoryKind) -> &Vec<HistoryEntry> {
        // NOTE: Could be an internal map
        match kind {
            HistoryKind::Undo => &self.undo_stack,
            HistoryKind::Redo => &self.redo_stack,
        }
    }

    fn stack_mut(&mut self, kind: HistoryKind) -> &mut Vec<HistoryEntry> {
        match kind {
            HistoryKind::Undo => &mut self.undo_stack,
            HistoryKind::Redo => &mut self.redo_stack,
        }
    }

    pub fn has_next(&self, kind: HistoryKind) -> bool {
        !self.stack(kind).is_empty()
    }

    pub fn push(&mut self, kind: HistoryKind, entry: HistoryEntry) {
        self.stack_mut(kind).push(entry);
    }

    pub fn take(&mut self, kind: HistoryKind) -> Option<HistoryEntry> {
        self.stack_mut(kind).pop()
    }
}

impl HistoryEntry {
    fn extend(&mut self, range: &Range<usize>, old_text: &str, new_text_len: usize) -> bool {
        // NOTE: Could be more robust. Currently only supports human-written extensions from start towards end.

        // ranges must be contiguous in order to integrate/extend
        if self.range.end != range.start {
            return false;
        }

        self.range.end = range.start + new_text_len;
        self.new_text_len += new_text_len;
        self.old_text.push_str(old_text);

        true
    }

    pub fn char_range(&self, max_len: usize) -> Range<usize> {
        let undo_start = self.range.start;
        let undo_end = (self.range.start + self.new_text_len).min(max_len);
        undo_start..undo_end
    }

    pub fn as_inverted(self, prev_text_at_range: String) -> Self {
        HistoryEntry {
            range: self.range.start..self.range.start + self.old_text.len(),
            old_text: prev_text_at_range,
            new_text_len: self.old_text.len(),
            selected_range: self.selected_range,
            timestamp: self.timestamp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record_edit(
        history: &mut EditableTextHistory,
        range: Range<usize>,
        old_text: &str,
        new_len: usize,
    ) {
        history.record(range, old_text, new_len, 0.into());
    }

    #[test]
    fn adjacent_replacements_preserve_all_replaced_text() {
        let mut history = EditableTextHistory::default();
        // "ab" becomes "xy" through two replacements that the grouping interval merges.
        record_edit(&mut history, 0..1, "a", 1);
        record_edit(&mut history, 1..2, "b", 1);

        let entry = history.take(HistoryKind::Undo).unwrap();
        assert!(history.take(HistoryKind::Undo).is_none());
        assert_eq!(entry.old_text, "ab");
        assert_eq!(entry.char_range(usize::MAX), 0..2);
    }

    #[test]
    fn consecutive_forward_deletions_preserve_all_removed_text() {
        let mut history = EditableTextHistory::default();
        // Deleting "a" and then "b" forward from the same offset.
        record_edit(&mut history, 0..1, "a", 0);
        record_edit(&mut history, 0..1, "b", 0);

        let entry = history.take(HistoryKind::Undo).unwrap();
        assert!(history.take(HistoryKind::Undo).is_none());
        assert_eq!(entry.old_text, "ab");
    }

    #[test]
    fn an_edit_that_joins_an_earlier_group_also_invalidates_redo() {
        let mut history = EditableTextHistory::default();
        record_edit(&mut history, 0..1, "a", 1);
        record_edit(&mut history, 5..6, "f", 1);
        let undone = history.take(HistoryKind::Undo).unwrap();
        history.push(HistoryKind::Redo, undone);
        assert!(history.has_next(HistoryKind::Redo));

        // Contiguous with the first entry, so it joins that group instead of pushing a new one.
        record_edit(&mut history, 1..2, "b", 1);
        assert!(!history.has_next(HistoryKind::Redo));
    }
}
