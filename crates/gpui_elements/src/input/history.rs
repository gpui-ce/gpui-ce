use gpui::NavigationDirection;
use std::{
    ops::Range,
    time::{Duration, Instant},
};

/// Maximum number of history entries to keep.
pub const MAX_HISTORY_LEN: usize = 1000;

/// Default interval for grouping consecutive edits into a single undo entry.
pub const DEFAULT_GROUP_INTERVAL: Duration = Duration::from_millis(300);

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
    pub selected_range: Range<usize>,
    /// The direction of the selection before the edit.
    pub selection_direction: NavigationDirection,
    /// Timestamp for grouping consecutive edits.
    pub timestamp: Instant,
}

impl HistoryEntry {
    /// Apply this patch to undo an edit, returning the reverse patch for redo.
    pub fn apply_undo(&self, content: &mut String) -> HistoryEntry {
        let undo_start = self.range.start;
        let undo_end = (self.range.start + self.new_text_len).min(content.len());

        // Capture what we're about to remove (the "new" text that was inserted)
        let removed_text = content[undo_start..undo_end].to_string();

        // Replace with the old text
        content.replace_range(undo_start..undo_end, &self.old_text);

        // Return reverse patch for redo
        HistoryEntry {
            range: undo_start..undo_start + self.old_text.len(),
            old_text: removed_text,
            new_text_len: self.old_text.len(),
            selected_range: self.selected_range.clone(),
            selection_direction: self.selection_direction,
            timestamp: self.timestamp,
        }
    }

    /// Apply this patch to redo an edit, returning the reverse patch for undo.
    pub fn apply_redo(&self, content: &mut String) -> HistoryEntry {
        // Redo is the same operation as undo - we're reversing the undo
        self.apply_undo(content)
    }
}
