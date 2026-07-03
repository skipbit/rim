use crate::domain::grapheme;
use crate::domain::motion::{self, Motion, MotionKind, Target};
use crate::domain::text_buffer::TextBuffer;
use crate::domain::text_object::{self, TextObject};
use crate::domain::transaction::{Change, Transaction};
use crossterm::event::KeyCode;
use unicode_width::UnicodeWidthStr;

pub enum EditorMode {
    Normal,
    Insert,
    Command,
    Search,
}

pub enum LastChange {
    InsertChar(char),
    DeleteChar,
    DeleteCharUnderCursor,
    InsertNewline,
    InsertLineBelow,
    InsertLineAbove,
    DeleteCurrentLine,
    PutLineBelow,
}

/// A normal-mode operator applied over a motion or text-object range.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Operator {
    Delete,
    Change,
    Yank,
}

/// The unnamed register: text captured by the last delete/change/yank, plus
/// whether it was linewise (pasted onto new lines) or charwise (pasted inline).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Register {
    pub text: String,
    pub linewise: bool,
}

pub struct EditorModel {
    pub buffer: TextBuffer,
    pub cursor_x: usize,
    pub cursor_y: usize,
    pub row_offset: usize,
    pub col_offset: usize,
    pub filepath: Option<String>,
    pub mode: EditorMode,
    pub command_buffer: String,
    pub register: Option<Register>,
    pub search_query: Option<String>,
    pub search_matches: Vec<(usize, usize)>,
    pub current_search_match: Option<usize>,
    undo_stack: Vec<Transaction>,
    redo_stack: Vec<Transaction>,
    coalescing: bool,
    last_change: Option<LastChange>,
    /// Monotonic counter bumped once per buffer edit (including undo/redo and
    /// whole-file loads). The syntax layer compares it before/after handling
    /// input to detect that the text changed and a re-highlight is due.
    edit_revision: u64,
}

impl EditorModel {
    pub fn new() -> Self {
        Self {
            mode: EditorMode::Normal,
            command_buffer: String::new(),
            buffer: TextBuffer::new(),
            cursor_x: 0,
            cursor_y: 0,
            row_offset: 0,
            col_offset: 0,
            filepath: None,
            register: None,
            search_query: None,
            search_matches: Vec::new(),
            current_search_match: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            coalescing: false,
            last_change: None,
            edit_revision: 0,
        }
    }

    /// The current edit revision (see the field docs). Cheap to read every
    /// input cycle.
    #[allow(dead_code)] // consumed by the syntax layer in a later milestone
    pub fn edit_revision(&self) -> u64 {
        self.edit_revision
    }

    /// The single point where a [`Change`] is applied to the buffer. Routing
    /// every edit — inserts, deletes, operators, paste, **and undo/redo**
    /// (which replay inverted changes) — through here guarantees
    /// `edit_revision` bumps exactly once per edit. Any new edit path MUST call
    /// this instead of `change.apply(&mut self.buffer)` directly.
    fn apply_change(&mut self, change: &Change) {
        Change::apply(change, &mut self.buffer);
        self.edit_revision = self.edit_revision.wrapping_add(1);
    }

    /// Set the editor mode. Any mode transition ends the current insert-coalescing
    /// run so a new insert session becomes its own undo step.
    pub fn set_mode(&mut self, mode: EditorMode) {
        self.mode = mode;
        self.coalescing = false;
    }

    pub fn set_content(&mut self, content: &str) {
        self.buffer.set_content(content);
        // Whole-file load bypasses `Change`, so bump the revision here too — the
        // syntax layer re-parses from scratch on the next cycle.
        self.edit_revision = self.edit_revision.wrapping_add(1);
    }

    pub fn get_content(&self) -> String {
        self.buffer.get_content()
    }

    pub fn set_filepath(&mut self, filepath: String) {
        self.filepath = Some(filepath);
    }

    pub fn get_filepath(&self) -> Option<&String> {
        self.filepath.as_ref()
    }

    pub fn move_cursor(&mut self, key: KeyCode) {
        match key {
            KeyCode::Up => {
                self.cursor_y = self.cursor_y.saturating_sub(1);
            }
            KeyCode::Down => {
                if self.cursor_y < self.buffer.line_count().saturating_sub(1) {
                    self.cursor_y += 1;
                }
            }
            KeyCode::Left => {
                if self.cursor_x > 0 {
                    let line = self.buffer.line_text(self.cursor_y);
                    self.cursor_x = grapheme::prev_boundary(&line, self.cursor_x);
                } else if self.cursor_y > 0 {
                    self.cursor_y -= 1;
                    self.cursor_x = self.buffer.line_char_len(self.cursor_y);
                }
            }
            KeyCode::Right if self.cursor_y < self.buffer.line_count() => {
                let line_len = self.buffer.line_char_len(self.cursor_y);
                if self.cursor_x < line_len {
                    let line = self.buffer.line_text(self.cursor_y);
                    self.cursor_x = grapheme::next_boundary(&line, self.cursor_x);
                } else if self.cursor_y < self.buffer.line_count() - 1 {
                    self.cursor_y += 1;
                    self.cursor_x = 0;
                }
            }
            _ => {}
        }
        // Snap cursor to end of line if it's past the end
        if self.cursor_y < self.buffer.line_count() {
            let line_len = self.buffer.line_char_len(self.cursor_y);
            if self.cursor_x > line_len {
                self.cursor_x = line_len;
            }
        }
        // Moving the cursor ends the current insert-coalescing run.
        self.coalescing = false;
    }

    /// Terminal display column of the cursor: the sum of display widths of the
    /// characters left of the cursor on its line (wide CJK = 2, combining = 0).
    pub fn display_col(&self) -> usize {
        let line = self.buffer.line_text(self.cursor_y);
        let prefix: String = line.chars().take(self.cursor_x).collect();
        UnicodeWidthStr::width(prefix.as_str())
    }

    /// Adjust the viewport offsets so the cursor is visible within a text area
    /// of `text_height` rows and `text_width` columns.
    pub fn scroll_into_view(&mut self, text_height: usize, text_width: usize) {
        if text_height == 0 || text_width == 0 {
            return;
        }
        if self.cursor_y < self.row_offset {
            self.row_offset = self.cursor_y;
        } else if self.cursor_y >= self.row_offset + text_height {
            self.row_offset = self.cursor_y + 1 - text_height;
        }
        let dcol = self.display_col();
        if dcol < self.col_offset {
            self.col_offset = dcol;
        } else if dcol >= self.col_offset + text_width {
            self.col_offset = dcol + 1 - text_width;
        }
    }

    pub fn insert_char(&mut self, c: char) {
        let before = (self.cursor_y, self.cursor_x);
        let change = if self.buffer.line_count() == 0 {
            // Empty document: materialize the first line (keeping the invariant).
            Change {
                pos: 0,
                removed: String::new(),
                inserted: format!("{}\n", c),
            }
        } else {
            let idx = self.buffer.cursor_to_char(self.cursor_y, self.cursor_x);
            Change {
                pos: idx,
                removed: String::new(),
                inserted: c.to_string(),
            }
        };
        self.apply_change(&change);
        // In the empty-document case the cursor is (0, 0); either way it advances
        // one char to the right of the inserted character.
        self.cursor_x += 1;
        self.last_change = Some(LastChange::InsertChar(c));
        let after = (self.cursor_y, self.cursor_x);
        self.commit_insert(change, before, after);
    }

    pub fn delete_char(&mut self) {
        if self.cursor_y >= self.buffer.line_count() {
            return;
        }
        let before = (self.cursor_y, self.cursor_x);
        if self.cursor_x > 0 {
            let idx = self.buffer.cursor_to_char(self.cursor_y, self.cursor_x);
            let change = Change {
                pos: idx - 1,
                removed: self.buffer.slice_text(idx - 1..idx),
                inserted: String::new(),
            };
            self.apply_change(&change);
            self.cursor_x -= 1;
            self.last_change = Some(LastChange::DeleteChar);
            self.commit(change, before, (self.cursor_y, self.cursor_x));
        } else if self.cursor_y > 0 {
            // Merge with the previous line by removing its terminating '\n'.
            let prev_len = self.buffer.line_char_len(self.cursor_y - 1);
            let nl_idx = self.buffer.line_to_char(self.cursor_y) - 1;
            let change = Change {
                pos: nl_idx,
                removed: "\n".to_string(),
                inserted: String::new(),
            };
            self.apply_change(&change);
            self.cursor_y -= 1;
            self.cursor_x = prev_len;
            self.last_change = Some(LastChange::DeleteChar);
            self.commit(change, before, (self.cursor_y, self.cursor_x));
        }
    }

    pub fn delete_char_under_cursor(&mut self) {
        if self.cursor_y >= self.buffer.line_count() {
            return;
        }
        if self.cursor_x < self.buffer.line_char_len(self.cursor_y) {
            let before = (self.cursor_y, self.cursor_x);
            let idx = self.buffer.cursor_to_char(self.cursor_y, self.cursor_x);
            let change = Change {
                pos: idx,
                removed: self.buffer.slice_text(idx..idx + 1),
                inserted: String::new(),
            };
            self.apply_change(&change);
            self.last_change = Some(LastChange::DeleteCharUnderCursor);
            self.commit(change, before, (self.cursor_y, self.cursor_x));
        }
    }

    pub fn insert_newline(&mut self) {
        let before = (self.cursor_y, self.cursor_x);
        let change = if self.buffer.line_count() == 0 {
            // Empty document: create the current empty line and the split line.
            Change {
                pos: 0,
                removed: String::new(),
                inserted: "\n\n".to_string(),
            }
        } else {
            let idx = self.buffer.cursor_to_char(self.cursor_y, self.cursor_x);
            Change {
                pos: idx,
                removed: String::new(),
                inserted: "\n".to_string(),
            }
        };
        self.apply_change(&change);
        self.cursor_y += 1;
        self.cursor_x = 0;
        self.last_change = Some(LastChange::InsertNewline);
        self.commit(change, before, (self.cursor_y, self.cursor_x));
    }

    pub fn insert_line_below(&mut self) {
        let before = (self.cursor_y, self.cursor_x);
        let change = if self.buffer.line_count() == 0 {
            self.cursor_y = 0;
            Change {
                pos: 0,
                removed: String::new(),
                inserted: "\n".to_string(),
            }
        } else {
            let idx = self.buffer.line_to_char(self.cursor_y + 1);
            self.cursor_y += 1;
            Change {
                pos: idx,
                removed: String::new(),
                inserted: "\n".to_string(),
            }
        };
        self.apply_change(&change);
        self.cursor_x = 0;
        self.last_change = Some(LastChange::InsertLineBelow);
        self.commit(change, before, (self.cursor_y, self.cursor_x));
    }

    pub fn insert_line_above(&mut self) {
        let before = (self.cursor_y, self.cursor_x);
        let idx = self.buffer.line_to_char(self.cursor_y);
        let change = Change {
            pos: idx,
            removed: String::new(),
            inserted: "\n".to_string(),
        };
        self.apply_change(&change);
        self.cursor_x = 0;
        self.last_change = Some(LastChange::InsertLineAbove);
        self.commit(change, before, (self.cursor_y, self.cursor_x));
    }

    pub fn delete_current_line(&mut self) {
        if self.buffer.line_count() == 0 {
            return;
        }
        let before = (self.cursor_y, self.cursor_x);
        let change = if self.buffer.line_count() == 1 {
            // Only line: delete its content but keep one empty line so the
            // document never collapses to zero lines.
            Change {
                pos: 0,
                removed: self.buffer.line_text(0),
                inserted: String::new(),
            }
        } else {
            let start = self.buffer.line_to_char(self.cursor_y);
            let end = self.buffer.line_to_char(self.cursor_y + 1);
            Change {
                pos: start,
                removed: self.buffer.slice_text(start..end),
                inserted: String::new(),
            }
        };
        self.apply_change(&change);
        if self.cursor_y >= self.buffer.line_count() && self.cursor_y > 0 {
            self.cursor_y -= 1;
        }
        self.cursor_x = 0;
        self.last_change = Some(LastChange::DeleteCurrentLine);
        self.commit(change, before, (self.cursor_y, self.cursor_x));
    }

    pub fn put_line_below(&mut self) {
        self.paste(true, 1);
    }

    /// Delete `count` characters from under the cursor (the `x` command),
    /// filling the unnamed register (charwise) like Vim.
    pub fn delete_under_cursor(&mut self, count: usize) {
        if self.cursor_y >= self.buffer.line_count() {
            return;
        }
        let line_len = self.buffer.line_char_len(self.cursor_y);
        if self.cursor_x >= line_len {
            return;
        }
        let s = self.buffer.cursor_to_char(self.cursor_y, self.cursor_x);
        let end_x = (self.cursor_x + count.max(1)).min(line_len);
        let e = self.buffer.cursor_to_char(self.cursor_y, end_x);
        self.operate_charwise_range(Operator::Delete, s, e);
    }

    /// Paste the unnamed register `count` times. `after` selects `p`
    /// (below/after) vs `P` (above/before). Linewise registers paste onto new
    /// lines; charwise registers paste inline.
    pub fn paste(&mut self, after: bool, count: usize) {
        let Some(reg) = self.register.clone() else {
            return;
        };
        let count = count.max(1);
        let text = reg.text.repeat(count);
        let before = (self.cursor_y, self.cursor_x);
        if reg.linewise {
            let (pos, new_y) = if self.buffer.line_count() == 0 {
                (0, 0)
            } else if after {
                (
                    self.buffer.line_to_char(self.cursor_y + 1),
                    self.cursor_y + 1,
                )
            } else {
                (self.buffer.line_to_char(self.cursor_y), self.cursor_y)
            };
            let change = Change {
                pos,
                removed: String::new(),
                inserted: text,
            };
            self.apply_change(&change);
            self.cursor_y = new_y;
            self.cursor_x = 0;
            self.last_change = Some(LastChange::PutLineBelow);
            self.commit(change, before, (self.cursor_y, self.cursor_x));
        } else {
            // Charwise paste.
            let inserted = if self.buffer.line_count() == 0 {
                format!("{}\n", text)
            } else {
                text.clone()
            };
            let pos = if self.buffer.line_count() == 0 {
                0
            } else {
                let line_len = self.buffer.line_char_len(self.cursor_y);
                let x = if after && line_len > 0 {
                    (self.cursor_x + 1).min(line_len)
                } else {
                    self.cursor_x
                };
                self.buffer.cursor_to_char(self.cursor_y, x)
            };
            let text_len = text.chars().count();
            let change = Change {
                pos,
                removed: String::new(),
                inserted,
            };
            self.apply_change(&change);
            let (cy, cx) = self.char_to_cursor(pos + text_len.saturating_sub(1));
            self.cursor_y = cy;
            self.cursor_x = cx;
            self.last_change = Some(LastChange::PutLineBelow);
            self.commit(change, before, (self.cursor_y, self.cursor_x));
        }
    }

    /// Map a whole-buffer char index to a clamped `(y, x)` cursor position.
    fn char_to_cursor(&self, idx: usize) -> (usize, usize) {
        if self.buffer.line_count() == 0 {
            return (0, 0);
        }
        let len = self.buffer.len_chars();
        if idx >= len {
            let y = self.buffer.line_count() - 1;
            return (y, self.buffer.line_char_len(y));
        }
        let y = self.buffer.char_to_line(idx);
        let x = (idx - self.buffer.line_to_char(y)).min(self.buffer.line_char_len(y));
        (y, x)
    }

    /// Move the cursor by a bare motion (no operator pending).
    pub fn move_by_motion(&mut self, motion: Motion, count: usize) {
        let t = motion::compute(&self.buffer, self.cursor_y, self.cursor_x, motion, count);
        self.cursor_y = t.y.min(self.buffer.line_count().saturating_sub(1));
        self.cursor_x = t.x.min(self.buffer.line_char_len(self.cursor_y));
        self.coalescing = false;
    }

    /// Apply an operator over the range described by `motion`. Returns true when
    /// the caller should switch to insert mode (the change operator).
    pub fn apply_operator(&mut self, op: Operator, motion: Motion, count: usize) -> bool {
        // `cw`/`cW` behaves like `ce`/`cE`: change up to the end of the word.
        let motion = if op == Operator::Change {
            match motion {
                Motion::WordForward { big } => Motion::WordEnd { big },
                other => other,
            }
        } else {
            motion
        };
        let t = motion::compute(&self.buffer, self.cursor_y, self.cursor_x, motion, count);
        self.operate(op, t)
    }

    /// Apply an operator to `count` whole lines from the cursor (dd/cc/yy).
    pub fn operate_current_lines(&mut self, op: Operator, count: usize) -> bool {
        let last = self.buffer.line_count().saturating_sub(1);
        let target_y = (self.cursor_y + count.saturating_sub(1)).min(last);
        let t = Target {
            y: target_y,
            x: 0,
            kind: MotionKind::Linewise,
            inclusive: false,
        };
        self.operate(op, t)
    }

    fn operate(&mut self, op: Operator, t: Target) -> bool {
        if t.kind == MotionKind::Linewise {
            let lo = self.cursor_y.min(t.y);
            let hi = self.cursor_y.max(t.y);
            self.operate_linewise_range(op, lo, hi)
        } else {
            let a = self.buffer.cursor_to_char(self.cursor_y, self.cursor_x);
            let b = self.buffer.cursor_to_char(t.y, t.x);
            let (s, e) = if b >= a {
                // Forward: inclusive extends past the target char.
                let mut e = if t.inclusive { b + 1 } else { b };
                if !t.inclusive && t.y > self.cursor_y && t.x == 0 {
                    // Exclusive motion landing at column 0 of a later line (e.g.
                    // `dw` at end of line): clamp to the end of the cursor's line
                    // so the newline is not deleted / lines are not joined.
                    e = self.buffer.line_to_char(self.cursor_y)
                        + self.buffer.line_char_len(self.cursor_y);
                }
                (a, e)
            } else {
                // Backward: the range starts at the target; inclusive extends
                // past the cursor's own char.
                let e = if t.inclusive { a + 1 } else { a };
                (b, e)
            };
            self.operate_charwise_range(op, s, e)
        }
    }

    /// Apply an operator over an explicit half-open charwise range `[s, e)`
    /// (shared by motions and text objects). Returns true for the change
    /// operator (caller should enter insert mode).
    fn operate_charwise_range(&mut self, op: Operator, s: usize, e: usize) -> bool {
        if s >= e {
            return false;
        }
        let before = (self.cursor_y, self.cursor_x);
        let reg_text = self.buffer.slice_text(s..e);
        self.register = Some(Register {
            text: reg_text.clone(),
            linewise: false,
        });
        match op {
            Operator::Yank => {
                let (cy, cx) = self.char_to_cursor(s);
                self.cursor_y = cy;
                self.cursor_x = cx;
                false
            }
            Operator::Delete | Operator::Change => {
                let change = Change {
                    pos: s,
                    removed: reg_text,
                    inserted: String::new(),
                };
                self.apply_change(&change);
                let (cy, cx) = self.char_to_cursor(s);
                self.cursor_y = cy;
                self.cursor_x = cx;
                self.last_change = Some(LastChange::DeleteChar);
                self.commit(change, before, (self.cursor_y, self.cursor_x));
                op == Operator::Change
            }
        }
    }

    /// Apply an operator over the whole lines `lo..=hi` (linewise). Shared by
    /// linewise motions, `dd`/`cc`/`yy`, and linewise text objects (`ip`/`ap`).
    fn operate_linewise_range(&mut self, op: Operator, lo: usize, hi: usize) -> bool {
        let before = (self.cursor_y, self.cursor_x);
        let last = self.buffer.line_count().saturating_sub(1);
        let lo = lo.min(last);
        let hi = hi.min(last);
        let start = self.buffer.line_to_char(lo);
        let end = self.buffer.line_to_char(hi + 1);
        self.register = Some(Register {
            text: self.buffer.slice_text(start..end),
            linewise: true,
        });
        match op {
            Operator::Yank => {
                self.cursor_y = lo;
                self.cursor_x = 0;
                false
            }
            Operator::Delete => {
                let change = if lo == 0 && hi >= last {
                    // Deleting every line: keep one empty line.
                    Change {
                        pos: 0,
                        removed: self.buffer.get_content(),
                        inserted: String::new(),
                    }
                } else {
                    Change {
                        pos: start,
                        removed: self.buffer.slice_text(start..end),
                        inserted: String::new(),
                    }
                };
                self.apply_change(&change);
                self.cursor_y = lo.min(self.buffer.line_count().saturating_sub(1));
                self.cursor_x = 0;
                self.last_change = Some(LastChange::DeleteCurrentLine);
                self.commit(change, before, (self.cursor_y, self.cursor_x));
                false
            }
            Operator::Change => {
                let change = Change {
                    pos: start,
                    removed: self.buffer.slice_text(start..end),
                    inserted: "\n".to_string(),
                };
                self.apply_change(&change);
                self.cursor_y = lo;
                self.cursor_x = 0;
                self.last_change = Some(LastChange::DeleteCurrentLine);
                self.commit(change, before, (lo, 0));
                true
            }
        }
    }

    /// Apply an operator over a text object (`diw`, `ci"`, `ya(`, `dip` …).
    pub fn apply_operator_textobject(
        &mut self,
        op: Operator,
        obj: TextObject,
        inner: bool,
        count: usize,
    ) -> bool {
        match text_object::range(
            &self.buffer,
            self.cursor_y,
            self.cursor_x,
            obj,
            inner,
            count,
        ) {
            Some((s, e, true)) => {
                // Linewise object (paragraph): convert the char range to lines.
                let lo = self.buffer.char_to_line(s);
                let hi = self.buffer.char_to_line(e.saturating_sub(1));
                self.cursor_y = lo;
                self.operate_linewise_range(op, lo, hi)
            }
            Some((s, e, false)) => self.operate_charwise_range(op, s, e),
            None => false,
        }
    }

    pub fn repeat_last_change(&mut self) {
        if let Some(last_change) = &self.last_change {
            match last_change {
                LastChange::InsertChar(c) => self.insert_char(*c),
                LastChange::DeleteChar => self.delete_char(),
                LastChange::DeleteCharUnderCursor => self.delete_char_under_cursor(),
                LastChange::InsertNewline => self.insert_newline(),
                LastChange::InsertLineBelow => self.insert_line_below(),
                LastChange::InsertLineAbove => self.insert_line_above(),
                LastChange::DeleteCurrentLine => self.delete_current_line(),
                LastChange::PutLineBelow => self.put_line_below(),
            }
        }
    }

    /// Record a non-insert edit as its own undo step, ending any coalescing run.
    fn commit(&mut self, change: Change, before: (usize, usize), after: (usize, usize)) {
        self.redo_stack.clear();
        self.undo_stack.push(Transaction {
            change,
            cursor_before: before,
            cursor_after: after,
        });
        self.coalescing = false;
    }

    /// Record an inserted character, coalescing it onto the previous transaction
    /// when it is a contiguous continuation of an ongoing insert run so that a
    /// word of typing collapses into a single undo step.
    fn commit_insert(&mut self, change: Change, before: (usize, usize), after: (usize, usize)) {
        self.redo_stack.clear();
        if self.coalescing {
            if let Some(top) = self.undo_stack.last_mut() {
                if top.change.removed.is_empty()
                    && change.removed.is_empty()
                    && change.pos == top.change.pos + top.change.inserted.chars().count()
                {
                    top.change.inserted.push_str(&change.inserted);
                    top.cursor_after = after;
                    return;
                }
            }
        }
        self.undo_stack.push(Transaction {
            change,
            cursor_before: before,
            cursor_after: after,
        });
        self.coalescing = true;
    }

    pub fn undo(&mut self) {
        if let Some(t) = self.undo_stack.pop() {
            self.apply_change(&t.change.invert());
            self.cursor_y = t.cursor_before.0;
            self.cursor_x = t.cursor_before.1;
            self.redo_stack.push(t);
        }
        self.coalescing = false;
    }

    pub fn redo(&mut self) {
        if let Some(t) = self.redo_stack.pop() {
            self.apply_change(&t.change);
            self.cursor_y = t.cursor_after.0;
            self.cursor_x = t.cursor_after.1;
            self.undo_stack.push(t);
        }
        self.coalescing = false;
    }

    pub fn search(&mut self, query: &str) {
        self.search_query = Some(query.to_string());
        self.search_matches.clear();
        self.current_search_match = None;

        if query.is_empty() {
            return;
        }

        for y in 0..self.buffer.line_count() {
            let line = self.buffer.line_text(y);
            for (byte_idx, _) in line.match_indices(query) {
                // Convert the byte offset from match_indices into a char offset
                // so match positions share the cursor's char coordinate.
                let char_x = line[..byte_idx].chars().count();
                self.search_matches.push((y, char_x));
            }
        }

        if !self.search_matches.is_empty() {
            let initial_match_index = self
                .search_matches
                .iter()
                .position(|&(y, x)| y >= self.cursor_y && x >= self.cursor_x)
                .unwrap_or(0);
            self.current_search_match = Some(initial_match_index);
            let (y, x) = self.search_matches[initial_match_index];
            self.cursor_y = y;
            self.cursor_x = x;
        }
    }

    pub fn find_next(&mut self) {
        if let Some(current_match) = self.current_search_match {
            if self.search_matches.is_empty() {
                return;
            }
            let next_match_index = (current_match + 1) % self.search_matches.len();
            self.current_search_match = Some(next_match_index);
            let (y, x) = self.search_matches[next_match_index];
            self.cursor_y = y;
            self.cursor_x = x;
        }
    }

    pub fn find_previous(&mut self) {
        if let Some(current_match) = self.current_search_match {
            if self.search_matches.is_empty() {
                return;
            }
            let prev_match_index = if current_match == 0 {
                self.search_matches.len() - 1
            } else {
                current_match - 1
            };
            self.current_search_match = Some(prev_match_index);
            let (y, x) = self.search_matches[prev_match_index];
            self.cursor_y = y;
            self.cursor_x = x;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_move_cursor_up() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("line1");
        editor.buffer.push_line("line2");
        editor.cursor_y = 1;
        editor.move_cursor(KeyCode::Up);
        assert_eq!(editor.cursor_y, 0);
    }

    #[test]
    fn test_move_cursor_down() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("line1");
        editor.buffer.push_line("line2");
        editor.cursor_y = 0;
        editor.move_cursor(KeyCode::Down);
        assert_eq!(editor.cursor_y, 1);
    }

    #[test]
    fn test_move_cursor_left() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("line1");
        editor.cursor_x = 3;
        editor.move_cursor(KeyCode::Left);
        assert_eq!(editor.cursor_x, 2);
    }

    #[test]
    fn test_scroll_down_brings_cursor_into_view() {
        let mut editor = EditorModel::new();
        for i in 0..20 {
            editor.buffer.push_line(&format!("line{i}"));
        }
        editor.cursor_y = 15;
        editor.scroll_into_view(10, 80);
        assert!(editor.row_offset <= 15 && 15 < editor.row_offset + 10);
        assert_eq!(editor.row_offset, 6); // 15 + 1 - 10
    }

    #[test]
    fn test_scroll_up_brings_cursor_into_view() {
        let mut editor = EditorModel::new();
        for i in 0..20 {
            editor.buffer.push_line(&format!("line{i}"));
        }
        editor.row_offset = 10;
        editor.cursor_y = 3;
        editor.scroll_into_view(10, 80);
        assert_eq!(editor.row_offset, 3);
    }

    #[test]
    fn test_no_scroll_when_cursor_visible() {
        let mut editor = EditorModel::new();
        for i in 0..20 {
            editor.buffer.push_line(&format!("line{i}"));
        }
        editor.row_offset = 5;
        editor.cursor_y = 8;
        editor.scroll_into_view(10, 80);
        assert_eq!(editor.row_offset, 5);
    }

    #[test]
    fn test_display_col_wide_chars() {
        let mut editor = EditorModel::new();
        editor.buffer.set_content("あい"); // each full-width CJK = 2 columns
        editor.cursor_y = 0;
        editor.cursor_x = 2;
        assert_eq!(editor.display_col(), 4);
    }

    #[test]
    fn test_display_col_combining_mark_is_zero_width() {
        let mut editor = EditorModel::new();
        editor.buffer.set_content("a\u{310}b");
        editor.cursor_y = 0;
        editor.cursor_x = 2; // after 'a' + combining mark
        assert_eq!(editor.display_col(), 1);
    }

    #[test]
    fn test_move_cursor_grapheme_combining_mark() {
        // "a̐b" = 'a' + U+0310 combining mark + 'b': the first grapheme spans 2 chars.
        let mut editor = EditorModel::new();
        editor.buffer.set_content("a\u{310}b");
        editor.cursor_y = 0;
        editor.cursor_x = 0;
        editor.move_cursor(KeyCode::Right);
        assert_eq!(editor.cursor_x, 2); // skipped the whole "a̐" grapheme
        editor.move_cursor(KeyCode::Right);
        assert_eq!(editor.cursor_x, 3);
        editor.move_cursor(KeyCode::Left);
        assert_eq!(editor.cursor_x, 2);
        editor.move_cursor(KeyCode::Left);
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_move_cursor_grapheme_emoji_modifier() {
        // "👍🏽" = thumbs-up + skin-tone modifier = one grapheme (2 scalar values).
        let mut editor = EditorModel::new();
        editor.buffer.set_content("👍🏽!");
        editor.cursor_y = 0;
        editor.cursor_x = 0;
        editor.move_cursor(KeyCode::Right);
        assert_eq!(editor.cursor_x, 2); // both scalars skipped as one unit
        editor.move_cursor(KeyCode::Right);
        assert_eq!(editor.cursor_x, 3);
    }

    #[test]
    fn test_insert_char_multibyte_no_panic() {
        // With the old byte-indexed cursor this panicked (cursor_x=1 is not a
        // byte boundary of "あい"). Now cursor_x is a char offset.
        let mut editor = EditorModel::new();
        editor.buffer.set_content("あい");
        editor.cursor_y = 0;
        editor.cursor_x = 1;
        editor.insert_char('x');
        assert_eq!(editor.buffer.line_text(0), "あxい");
        assert_eq!(editor.cursor_x, 2);
    }

    #[test]
    fn test_delete_char_multibyte() {
        let mut editor = EditorModel::new();
        editor.buffer.set_content("あい");
        editor.cursor_y = 0;
        editor.cursor_x = 2;
        editor.delete_char();
        assert_eq!(editor.buffer.line_text(0), "あ");
        assert_eq!(editor.cursor_x, 1);
    }

    #[test]
    fn test_search_multibyte_char_offsets() {
        let mut editor = EditorModel::new();
        editor.buffer.set_content("あいうabc");
        editor.search("abc");
        // match is at char offset 3, not byte offset 9
        assert_eq!(editor.search_matches, vec![(0, 3)]);
        assert_eq!(editor.cursor_x, 3);
    }

    #[test]
    fn test_move_cursor_right() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("line1");
        editor.cursor_x = 0;
        editor.move_cursor(KeyCode::Right);
        assert_eq!(editor.cursor_x, 1);
    }

    #[test]
    fn test_insert_char() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("");
        editor.insert_char('a');
        assert_eq!(editor.buffer.line_text(0), "a");
        assert_eq!(editor.cursor_x, 1);
    }

    #[test]
    fn test_delete_char() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("abc");
        editor.cursor_x = 3;
        editor.delete_char();
        assert_eq!(editor.buffer.line_text(0), "ab");
        assert_eq!(editor.cursor_x, 2);
    }

    #[test]
    fn test_delete_char_at_beginning_of_line() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("line1");
        editor.buffer.push_line("line2");
        editor.cursor_y = 1;
        editor.cursor_x = 0;
        editor.delete_char();
        assert_eq!(editor.buffer.line_count(), 1);
        assert_eq!(editor.buffer.line_text(0), "line1line2");
        assert_eq!(editor.cursor_y, 0);
        assert_eq!(editor.cursor_x, 5);
    }

    #[test]
    fn test_insert_newline_middle_of_line() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("Hello World");
        editor.cursor_x = 5; // カーソルを 'o' と ' ' の間に設定
        editor.cursor_y = 0;
        editor.insert_newline();
        assert_eq!(editor.buffer.line_count(), 2);
        assert_eq!(editor.buffer.line_text(0), "Hello");
        assert_eq!(editor.buffer.line_text(1), " World");
        assert_eq!(editor.cursor_y, 1);
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_insert_newline_end_of_line() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("Hello World");
        editor.cursor_x = 11; // カーソルを 'd' の後に設定
        editor.cursor_y = 0;
        editor.insert_newline();
        assert_eq!(editor.buffer.line_count(), 2);
        assert_eq!(editor.buffer.line_text(0), "Hello World");
        assert_eq!(editor.buffer.line_text(1), "");
        assert_eq!(editor.cursor_y, 1);
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_insert_newline_empty_line() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("");
        editor.cursor_x = 0;
        editor.cursor_y = 0;
        editor.insert_newline();
        assert_eq!(editor.buffer.line_count(), 2);
        assert_eq!(editor.buffer.line_text(0), "");
        assert_eq!(editor.buffer.line_text(1), "");
        assert_eq!(editor.cursor_y, 1);
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_insert_line_below() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("line1");
        editor.buffer.push_line("line2");
        editor.cursor_y = 0;
        editor.insert_line_below();
        assert_eq!(editor.buffer.line_count(), 3);
        assert_eq!(editor.buffer.line_text(1), "");
        assert_eq!(editor.cursor_y, 1);
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_insert_line_above() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("line1");
        editor.buffer.push_line("line2");
        editor.cursor_y = 1;
        editor.insert_line_above();
        assert_eq!(editor.buffer.line_count(), 3);
        assert_eq!(editor.buffer.line_text(1), "");
        assert_eq!(editor.cursor_y, 1);
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_delete_char_under_cursor() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("abc");
        editor.cursor_x = 1;
        editor.delete_char_under_cursor();
        assert_eq!(editor.buffer.line_text(0), "ac");
        assert_eq!(editor.cursor_x, 1);
    }

    #[test]
    fn test_delete_char_under_cursor_at_end_of_line() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("abc");
        editor.cursor_x = 2;
        editor.delete_char_under_cursor();
        assert_eq!(editor.buffer.line_text(0), "ab");
        assert_eq!(editor.cursor_x, 2);
    }

    #[test]
    fn test_delete_char_under_cursor_empty_line() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("");
        editor.cursor_x = 0;
        editor.delete_char_under_cursor();
        assert_eq!(editor.buffer.line_text(0), "");
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_delete_current_line() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("line1");
        editor.buffer.push_line("line2");
        editor.buffer.push_line("line3");
        editor.cursor_y = 1;
        editor.delete_current_line();
        assert_eq!(editor.buffer.line_count(), 2);
        assert_eq!(editor.buffer.line_text(0), "line1");
        assert_eq!(editor.buffer.line_text(1), "line3");
        assert_eq!(editor.cursor_y, 1);
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_delete_current_line_first_line() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("line1");
        editor.buffer.push_line("line2");
        editor.cursor_y = 0;
        editor.delete_current_line();
        assert_eq!(editor.buffer.line_count(), 1);
        assert_eq!(editor.buffer.line_text(0), "line2");
        assert_eq!(editor.cursor_y, 0);
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_delete_current_line_last_line() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("line1");
        editor.buffer.push_line("line2");
        editor.cursor_y = 1;
        editor.delete_current_line();
        assert_eq!(editor.buffer.line_count(), 1);
        assert_eq!(editor.buffer.line_text(0), "line1");
        assert_eq!(editor.cursor_y, 0);
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_delete_current_line_single_line() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("line1");
        editor.cursor_y = 0;
        editor.delete_current_line();
        assert_eq!(editor.buffer.line_count(), 1);
        assert_eq!(editor.buffer.line_text(0), "");
        assert_eq!(editor.cursor_y, 0);
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_put_line_below() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("line1");
        editor.buffer.push_line("line2");
        editor.cursor_y = 0;
        editor.register = Some(Register {
            text: "yanked_line\n".to_string(),
            linewise: true,
        });
        editor.put_line_below();
        assert_eq!(editor.buffer.line_count(), 3);
        assert_eq!(editor.buffer.line_text(1), "yanked_line");
        assert_eq!(editor.cursor_y, 1);
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_put_line_below_empty_yanked_line() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("line1");
        editor.cursor_y = 0;
        editor.register = Some(Register {
            text: "\n".to_string(),
            linewise: true,
        });
        editor.put_line_below();
        assert_eq!(editor.buffer.line_count(), 2);
        assert_eq!(editor.buffer.line_text(1), "");
        assert_eq!(editor.cursor_y, 1);
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_undo_reverts_edit_and_restores_cursor() {
        let mut editor = EditorModel::new();
        editor.buffer.set_content("ab");
        editor.cursor_y = 0;
        editor.cursor_x = 2;
        editor.insert_char('c');
        assert_eq!(editor.buffer.line_text(0), "abc");
        editor.undo();
        assert_eq!(editor.buffer.line_text(0), "ab");
        assert_eq!(editor.cursor_x, 2);
    }

    #[test]
    fn test_redo_reapplies_edit() {
        let mut editor = EditorModel::new();
        editor.buffer.set_content("ab");
        editor.cursor_y = 0;
        editor.cursor_x = 2;
        editor.insert_char('c');
        editor.undo();
        editor.redo();
        assert_eq!(editor.buffer.line_text(0), "abc");
        assert_eq!(editor.cursor_x, 3);
    }

    #[test]
    fn test_undo_coalesces_consecutive_inserts() {
        // Typing a word is a single undo step.
        let mut editor = EditorModel::new();
        editor.buffer.set_content("x");
        editor.cursor_y = 0;
        editor.cursor_x = 1;
        for c in "hello".chars() {
            editor.insert_char(c);
        }
        assert_eq!(editor.buffer.line_text(0), "xhello");
        editor.undo();
        assert_eq!(editor.buffer.line_text(0), "x");
        assert_eq!(editor.cursor_x, 1);
    }

    #[test]
    fn test_cursor_move_breaks_coalescing() {
        // A cursor move between inserts splits them into separate undo steps.
        let mut editor = EditorModel::new();
        editor.buffer.set_content("z");
        editor.cursor_y = 0;
        editor.cursor_x = 1;
        editor.insert_char('a'); // "za"
        editor.move_cursor(KeyCode::Left);
        editor.move_cursor(KeyCode::Right);
        editor.insert_char('b'); // "zab"
        assert_eq!(editor.buffer.line_text(0), "zab");
        editor.undo();
        assert_eq!(editor.buffer.line_text(0), "za");
        editor.undo();
        assert_eq!(editor.buffer.line_text(0), "z");
    }

    #[test]
    fn test_new_edit_clears_redo_stack() {
        let mut editor = EditorModel::new();
        editor.buffer.set_content("z");
        editor.cursor_y = 0;
        editor.cursor_x = 1;
        editor.insert_char('a'); // "za"
        editor.undo(); // "z"
        editor.insert_char('b'); // "zb", redo history discarded
        editor.redo(); // no-op
        assert_eq!(editor.buffer.line_text(0), "zb");
    }

    #[test]
    fn test_delete_current_line_is_undoable() {
        let mut editor = EditorModel::new();
        editor.buffer.set_content("line1\nline2");
        editor.cursor_y = 0;
        editor.delete_current_line();
        assert_eq!(editor.buffer.to_lines(), vec!["line2"]);
        editor.undo();
        assert_eq!(editor.buffer.to_lines(), vec!["line1", "line2"]);
    }

    #[test]
    fn test_repeat_last_change_insert_char() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("");
        editor.insert_char('a');
        editor.repeat_last_change();
        assert_eq!(editor.buffer.line_text(0), "aa");
        assert_eq!(editor.cursor_x, 2);
    }

    #[test]
    fn test_repeat_last_change_delete_char() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("abc");
        editor.cursor_x = 3;
        editor.delete_char();
        editor.repeat_last_change();
        assert_eq!(editor.buffer.line_text(0), "a");
        assert_eq!(editor.cursor_x, 1);
    }

    #[test]
    fn test_repeat_last_change_delete_char_under_cursor() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("abc");
        editor.cursor_x = 0;
        editor.delete_char_under_cursor();
        editor.repeat_last_change();
        assert_eq!(editor.buffer.line_text(0), "c");
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_repeat_last_change_insert_newline() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("line1");
        editor.cursor_x = 5;
        editor.insert_newline();
        editor.repeat_last_change();
        assert_eq!(editor.buffer.line_count(), 3);
        assert_eq!(editor.buffer.line_text(0), "line1");
        assert_eq!(editor.buffer.line_text(1), "");
        assert_eq!(editor.buffer.line_text(2), "");
    }

    #[test]
    fn test_repeat_last_change_insert_line_below() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("line1");
        editor.insert_line_below();
        editor.repeat_last_change();
        assert_eq!(editor.buffer.line_count(), 3);
        assert_eq!(editor.buffer.line_text(0), "line1");
        assert_eq!(editor.buffer.line_text(1), "");
        assert_eq!(editor.buffer.line_text(2), "");
    }

    #[test]
    fn test_repeat_last_change_insert_line_above() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("line1");
        editor.cursor_y = 0;
        editor.insert_line_above();
        editor.repeat_last_change();
        assert_eq!(editor.buffer.line_count(), 3);
        assert_eq!(editor.buffer.line_text(0), "");
        assert_eq!(editor.buffer.line_text(1), "");
        assert_eq!(editor.buffer.line_text(2), "line1");
    }

    #[test]
    fn test_repeat_last_change_delete_current_line() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("line1");
        editor.buffer.push_line("line2");
        editor.delete_current_line();
        editor.repeat_last_change();
        assert_eq!(editor.buffer.line_count(), 1);
        assert_eq!(editor.buffer.line_text(0), "");
    }

    #[test]
    fn test_repeat_last_change_put_line_below() {
        let mut editor = EditorModel::new();
        editor.buffer.push_line("line1");
        editor.register = Some(Register {
            text: "yanked\n".to_string(),
            linewise: true,
        });
        editor.put_line_below();
        editor.repeat_last_change();
        assert_eq!(editor.buffer.line_count(), 3);
        assert_eq!(editor.buffer.line_text(1), "yanked");
        assert_eq!(editor.buffer.line_text(2), "yanked");
    }

    #[test]
    fn test_insert_line_below_empty_document() {
        let mut editor = EditorModel::new();
        editor.insert_line_below();
        assert_eq!(editor.buffer.line_count(), 1);
        assert_eq!(editor.buffer.line_text(0), "");
        assert_eq!(editor.cursor_y, 0);
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_search() {
        let mut editor = EditorModel::new();
        editor.buffer.set_content("hello world\nworld hello");
        editor.search("world");
        assert_eq!(editor.search_matches, vec![(0, 6), (1, 0)]);
        assert_eq!(editor.cursor_y, 0);
        assert_eq!(editor.cursor_x, 6);
    }

    #[test]
    fn test_find_next() {
        let mut editor = EditorModel::new();
        editor.buffer.set_content("a b c\nd e f");
        editor.search(" ");
        editor.find_next();
        assert_eq!(editor.cursor_y, 0);
        assert_eq!(editor.cursor_x, 3);
        editor.find_next();
        assert_eq!(editor.cursor_y, 1);
        assert_eq!(editor.cursor_x, 1);
    }

    #[test]
    fn test_find_previous() {
        let mut editor = EditorModel::new();
        editor.buffer.set_content("a b c\nd e f");
        editor.search(" ");
        editor.find_previous();
        assert_eq!(editor.cursor_y, 1);
        assert_eq!(editor.cursor_x, 3);
        editor.find_previous();
        assert_eq!(editor.cursor_y, 1);
        assert_eq!(editor.cursor_x, 1);
    }

    // ---- MS2: operators × motions ------------------------------------------

    fn model(s: &str) -> EditorModel {
        let mut e = EditorModel::new();
        e.buffer.set_content(s);
        e
    }

    #[test]
    fn test_hl_motion_steps_by_grapheme() {
        // Normal-mode h/l must move by grapheme cluster (matching MS1 insert-mode
        // behaviour), not by scalar char.
        let mut e = model("a\u{310}b"); // "a̐b": first grapheme is 2 chars
        e.cursor_y = 0;
        e.cursor_x = 0;
        e.move_by_motion(Motion::Right, 1);
        assert_eq!(e.cursor_x, 2); // skipped the combining mark with its base
        e.move_by_motion(Motion::Left, 1);
        assert_eq!(e.cursor_x, 0);
    }

    #[test]
    fn test_dw_deletes_word_and_trailing_space() {
        let mut e = model("foo bar");
        e.apply_operator(Operator::Delete, Motion::WordForward { big: false }, 1);
        assert_eq!(e.buffer.line_text(0), "bar");
        assert_eq!((e.cursor_y, e.cursor_x), (0, 0));
    }

    #[test]
    fn test_de_deletes_to_word_end_inclusive() {
        let mut e = model("foo bar");
        e.apply_operator(Operator::Delete, Motion::WordEnd { big: false }, 1);
        assert_eq!(e.buffer.line_text(0), " bar");
    }

    #[test]
    fn test_dw_at_line_end_does_not_join() {
        let mut e = model("foo\nbar");
        e.cursor_y = 0;
        e.cursor_x = 2; // last char of "foo"
        e.apply_operator(Operator::Delete, Motion::WordForward { big: false }, 1);
        assert_eq!(
            e.buffer.to_lines(),
            vec!["fo".to_string(), "bar".to_string()]
        );
    }

    #[test]
    fn test_d_dollar_deletes_to_end_of_line() {
        let mut e = model("hello world");
        e.cursor_x = 6;
        e.apply_operator(Operator::Delete, Motion::LineEnd, 1);
        assert_eq!(e.buffer.line_text(0), "hello ");
    }

    #[test]
    fn test_dd_deletes_current_line() {
        let mut e = model("a\nb\nc");
        e.cursor_y = 1;
        e.operate_current_lines(Operator::Delete, 1);
        assert_eq!(e.buffer.to_lines(), vec!["a".to_string(), "c".to_string()]);
        assert_eq!(e.cursor_y, 1);
    }

    #[test]
    fn test_count_dd_deletes_multiple_lines() {
        let mut e = model("a\nb\nc\nd");
        e.cursor_y = 0;
        e.operate_current_lines(Operator::Delete, 2);
        assert_eq!(e.buffer.to_lines(), vec!["c".to_string(), "d".to_string()]);
    }

    #[test]
    fn test_dj_deletes_two_lines_linewise() {
        let mut e = model("a\nb\nc");
        e.cursor_y = 0;
        e.apply_operator(Operator::Delete, Motion::Down, 1);
        assert_eq!(e.buffer.to_lines(), vec!["c".to_string()]);
    }

    #[test]
    fn test_dd_all_lines_leaves_one_empty() {
        let mut e = model("only");
        e.operate_current_lines(Operator::Delete, 1);
        assert_eq!(e.buffer.to_lines(), vec!["".to_string()]);
    }

    #[test]
    fn test_cw_changes_to_word_end_and_enters_insert() {
        let mut e = model("foo bar");
        let enter = e.apply_operator(Operator::Change, Motion::WordForward { big: false }, 1);
        assert!(enter); // change enters insert mode
        assert_eq!(e.buffer.line_text(0), " bar"); // "foo" removed, trailing space kept
        assert_eq!((e.cursor_y, e.cursor_x), (0, 0));
    }

    #[test]
    fn test_cc_clears_line_keeps_it() {
        let mut e = model("foo\nbar");
        e.cursor_y = 0;
        let enter = e.operate_current_lines(Operator::Change, 1);
        assert!(enter);
        assert_eq!(e.buffer.to_lines(), vec!["".to_string(), "bar".to_string()]);
        assert_eq!((e.cursor_y, e.cursor_x), (0, 0));
    }

    #[test]
    fn test_operator_is_single_undo_step() {
        let mut e = model("foo bar");
        e.apply_operator(Operator::Delete, Motion::WordForward { big: false }, 1);
        assert_eq!(e.buffer.line_text(0), "bar");
        e.undo();
        assert_eq!(e.buffer.line_text(0), "foo bar");
    }

    #[test]
    fn test_yank_word_then_charwise_paste() {
        let mut e = model("foo bar");
        e.apply_operator(Operator::Yank, Motion::WordForward { big: false }, 1);
        // register holds "foo " (charwise); cursor returns to start of yank
        assert_eq!((e.cursor_y, e.cursor_x), (0, 0));
        e.paste(true, 1); // paste after the char under cursor
        assert_eq!(e.buffer.line_text(0), "ffoo oo bar");
    }

    #[test]
    fn test_yy_then_linewise_paste() {
        let mut e = model("line1\nline2");
        e.cursor_y = 0;
        e.operate_current_lines(Operator::Yank, 1);
        e.paste(true, 1);
        assert_eq!(
            e.buffer.to_lines(),
            vec![
                "line1".to_string(),
                "line1".to_string(),
                "line2".to_string()
            ]
        );
        assert_eq!(e.cursor_y, 1);
    }

    #[test]
    fn test_x_fills_register_and_honors_count() {
        let mut e = model("abcdef");
        e.cursor_x = 1;
        e.delete_under_cursor(3); // delete "bcd"
        assert_eq!(e.buffer.line_text(0), "aef");
        assert_eq!(
            e.register,
            Some(Register {
                text: "bcd".to_string(),
                linewise: false
            })
        );
        // x then p re-inserts the deleted text (Vim behaviour).
        e.paste(true, 1);
        assert_eq!(e.buffer.line_text(0), "aebcdf");
    }

    #[test]
    fn test_x_clamps_to_line_end() {
        let mut e = model("ab");
        e.cursor_x = 1;
        e.delete_under_cursor(5); // only "b" remains to delete
        assert_eq!(e.buffer.line_text(0), "a");
    }

    #[test]
    fn test_charwise_paste_count() {
        let mut e = model("ab");
        e.register = Some(Register {
            text: "X".to_string(),
            linewise: false,
        });
        e.cursor_x = 0;
        e.paste(true, 3); // paste "XXX" after 'a'
        assert_eq!(e.buffer.line_text(0), "aXXXb");
    }

    #[test]
    fn test_charwise_paste_before() {
        let mut e = model("abc");
        e.register = Some(Register {
            text: "X".to_string(),
            linewise: false,
        });
        e.cursor_x = 1; // on 'b'
        e.paste(false, 1); // P inserts before cursor
        assert_eq!(e.buffer.line_text(0), "aXbc");
        assert_eq!(e.cursor_x, 1);
    }
}
