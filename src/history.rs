//! Undo and redo. Each step keeps a full copy of the lines from before an
//! edit. Consecutive edits of the same kind at the spot where the last one
//! left the cursor share a step, so a typed run of text undoes at once,
//! like an insert-mode run in vim. Any other edit, or moving the cursor,
//! starts a new step.

use std::collections::VecDeque;

use crate::buffer::Pos;

const MAX_STEPS: usize = 1000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditKind {
    Type,
    Backspace,
    Delete,
    MoveLines,
    /// Never merged with anything: Enter, paste, extract, and so on.
    Other,
}

/// Buffer contents plus where the cursor and selection were.
pub struct Snapshot {
    pub lines: Vec<String>,
    pub cursor: Pos,
    pub anchor: Option<Pos>,
    /// Identifies the content, so undoing back to the saved state counts
    /// as unmodified.
    pub state: u64,
}

#[derive(Default)]
pub struct History {
    undo: VecDeque<Snapshot>,
    redo: Vec<Snapshot>,
    /// Kind of the last edit and where it left the cursor.
    last: Option<(EditKind, Pos)>,
}

impl History {
    /// Whether an edit of `kind` with the cursor at `at` joins the current step.
    pub fn continues(&self, kind: EditKind, at: Pos) -> bool {
        kind != EditKind::Other && self.last == Some((kind, at))
    }

    /// Record a finished edit. `before` is `None` when the edit joined the
    /// current step.
    pub fn record(&mut self, before: Option<Snapshot>, kind: EditKind, after: Pos) {
        if let Some(s) = before {
            if self.undo.len() == MAX_STEPS {
                self.undo.pop_front();
            }
            self.undo.push_back(s);
        }
        self.redo.clear();
        self.last = Some((kind, after));
    }

    /// End the current step so the next edit starts a new one.
    pub fn break_step(&mut self) {
        self.last = None;
    }

    /// Swap `current` for the previous state, if there is one.
    pub fn undo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let prev = self.undo.pop_back()?;
        self.redo.push(current);
        self.last = None;
        Some(prev)
    }

    /// Swap `current` for the next state, if there is one.
    pub fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let next = self.redo.pop()?;
        self.undo.push_back(current);
        self.last = None;
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(text: &str, state: u64) -> Snapshot {
        Snapshot {
            lines: vec![text.to_string()],
            cursor: (0, 0),
            anchor: None,
            state,
        }
    }

    #[test]
    fn typing_at_the_cursor_joins_one_step() {
        let mut h = History::default();
        assert!(!h.continues(EditKind::Type, (0, 0)));
        h.record(Some(snap("", 0)), EditKind::Type, (0, 1));
        assert!(h.continues(EditKind::Type, (0, 1)));
        assert!(!h.continues(EditKind::Type, (0, 5)));
        assert!(!h.continues(EditKind::Backspace, (0, 1)));
        assert!(!h.continues(EditKind::Other, (0, 1)));
        h.break_step();
        assert!(!h.continues(EditKind::Type, (0, 1)));
    }

    #[test]
    fn undo_redo_round_trip_and_new_edit_clears_redo() {
        let mut h = History::default();
        h.record(Some(snap("", 0)), EditKind::Other, (0, 1));
        let prev = h.undo(snap("a", 1)).unwrap();
        assert_eq!(prev.lines, vec![""]);
        let next = h.redo(snap("", 0)).unwrap();
        assert_eq!(next.lines, vec!["a"]);
        assert!(h.redo(snap("a", 1)).is_none());

        h.undo(snap("a", 1)).unwrap();
        h.record(Some(snap("", 0)), EditKind::Other, (0, 1));
        assert!(h.redo(snap("b", 2)).is_none());
    }
}
