use gpui::CaretSelection;
use std::{
    ops::Range,
    time::{Duration, Instant},
};

/// The undo stack limit enforced when [`EditableTextHistory::record`] adds an entry.
/// Recording drops the oldest entry when the stack is already at the limit.
pub const MAX_HISTORY_LEN: usize = 1000;

/// The default time window in which consecutive edits are grouped into one undo entry.
pub const DEFAULT_GROUP_INTERVAL: Duration = Duration::from_millis(300);

// TODO: Should history attach directly to storage? It is currently per field and
// operates on both the storage and the selection.

/// The undo and redo stacks of one editable text field.
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

/// One recorded edit, stored as a patch rather than a copy of the content.
///
/// The entry holds only the text that the edit replaced, so it can reverse the edit.
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

/// Selects the undo stack or the redo stack.
#[derive(Clone, Debug)]
pub enum HistoryKind {
    /// The stack of edits that can be undone.
    Undo,
    /// The stack of edits that were undone and can be redone.
    Redo,
}

impl EditableTextHistory {
    /// Sets how long after an edit's first change later edits still join it.
    pub fn set_grouping_interval(&mut self, interval: Duration) {
        self.grouping_interval = interval;
    }

    /// Records an edit so that it can be undone.
    ///
    /// The edit replaced `old_text` at `range` with `new_text_len` bytes. A
    /// change that starts at the end of the latest entry extends it if the time
    /// since that entry's first change is less than the grouping interval.
    /// Other changes start a new entry, which clears the redo stack.
    pub fn record(
        &mut self,
        range: Range<usize>,
        old_text: &str,
        new_text_len: usize,
        selected_range: CaretSelection,
    ) {
        let now = Instant::now();

        // Check if we should group with the last entry
        if let Some(last) = self.undo_stack.last_mut()
            && now.duration_since(last.timestamp) < self.grouping_interval
        {
            // The change was triggered within group interval timing.
            // Try to extend the existing patch (which is a mutation).
            // If extending succeeds, then we can early-out. Otherwise the mutation is non-contiguous.
            if last.extend(&range, new_text_len) {
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

        // New edit invalidates redo stack
        self.redo_stack.clear();
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

    /// Returns `true` if the given stack has at least one entry.
    pub fn has_next(&self, kind: HistoryKind) -> bool {
        !self.stack(kind).is_empty()
    }

    /// Pushes `entry` onto the given stack.
    ///
    /// Use [`Self::record`] to enforce the undo limit and clear the redo stack for a new edit.
    pub fn push(&mut self, kind: HistoryKind, entry: HistoryEntry) {
        self.stack_mut(kind).push(entry);
    }

    /// Removes and returns the newest entry of the given stack, or `None` if it is empty.
    pub fn take(&mut self, kind: HistoryKind) -> Option<HistoryEntry> {
        self.stack_mut(kind).pop()
    }
}

impl HistoryEntry {
    fn extend(&mut self, range: &Range<usize>, new_text_len: usize) -> bool {
        // NOTE: Could be more robust. Currently only supports human-written extensions from start towards end.

        // ranges must be contiguous in order to integrate/extend
        if self.range.end != range.start {
            return false;
        }

        self.range.end = range.start + new_text_len;
        self.new_text_len += new_text_len;

        true
    }

    /// Returns the byte range that this entry covers in the current content.
    ///
    /// The end is clamped to `max_len`. The caller must ensure the start is in
    /// bounds and both endpoints are UTF-8 character boundaries before slicing.
    pub fn char_range(&self, max_len: usize) -> Range<usize> {
        let undo_start = self.range.start;
        let undo_end = (self.range.start + self.new_text_len).min(max_len);
        undo_start..undo_end
    }

    /// Returns the entry that reverses this one, which is how an undone edit
    /// goes onto the redo stack.
    ///
    /// `prev_text_at_range` is the text that currently occupies the entry's
    /// range, and it becomes the inverted entry's `old_text`.
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
