use crate::domain::text_buffer::TextBuffer;
use crate::domain::transaction::{Change, Transaction};
use crossterm::event::KeyCode;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Char offsets of every grapheme-cluster boundary in `line`, including 0 and
/// the end. Horizontal cursor motion steps between these so a combining mark or
/// ZWJ emoji sequence moves as one visual unit.
fn grapheme_boundaries(line: &str) -> Vec<usize> {
    let mut bounds = vec![0];
    let mut chars = 0;
    for g in line.graphemes(true) {
        chars += g.chars().count();
        bounds.push(chars);
    }
    bounds
}

/// Char offset of the grapheme boundary immediately after `char_idx`.
fn next_grapheme(line: &str, char_idx: usize) -> usize {
    grapheme_boundaries(line)
        .into_iter()
        .find(|&b| b > char_idx)
        .unwrap_or(char_idx)
}

/// Char offset of the grapheme boundary immediately before `char_idx`.
fn prev_grapheme(line: &str, char_idx: usize) -> usize {
    grapheme_boundaries(line)
        .into_iter()
        .rev()
        .find(|&b| b < char_idx)
        .unwrap_or(0)
}

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

pub struct EditorModel {
    pub buffer: TextBuffer,
    pub cursor_x: usize,
    pub cursor_y: usize,
    pub row_offset: usize,
    pub col_offset: usize,
    pub filepath: Option<String>,
    pub mode: EditorMode,
    pub command_buffer: String,
    pub yanked_line: Option<String>,
    pub search_query: Option<String>,
    pub search_matches: Vec<(usize, usize)>,
    pub current_search_match: Option<usize>,
    pub d_pressed: bool,
    undo_stack: Vec<Transaction>,
    redo_stack: Vec<Transaction>,
    coalescing: bool,
    last_change: Option<LastChange>,
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
            yanked_line: None,
            search_query: None,
            search_matches: Vec::new(),
            current_search_match: None,
            d_pressed: false,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            coalescing: false,
            last_change: None,
        }
    }

    /// Set the editor mode. Any mode transition ends the current insert-coalescing
    /// run so a new insert session becomes its own undo step.
    pub fn set_mode(&mut self, mode: EditorMode) {
        self.mode = mode;
        self.coalescing = false;
    }

    pub fn set_content(&mut self, content: &str) {
        self.buffer.set_content(content);
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
                    self.cursor_x = prev_grapheme(&line, self.cursor_x);
                } else if self.cursor_y > 0 {
                    self.cursor_y -= 1;
                    self.cursor_x = self.buffer.line_char_len(self.cursor_y);
                }
            }
            KeyCode::Right if self.cursor_y < self.buffer.line_count() => {
                let line_len = self.buffer.line_char_len(self.cursor_y);
                if self.cursor_x < line_len {
                    let line = self.buffer.line_text(self.cursor_y);
                    self.cursor_x = next_grapheme(&line, self.cursor_x);
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
        change.apply(&mut self.buffer);
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
            change.apply(&mut self.buffer);
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
            change.apply(&mut self.buffer);
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
            change.apply(&mut self.buffer);
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
        change.apply(&mut self.buffer);
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
        change.apply(&mut self.buffer);
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
        change.apply(&mut self.buffer);
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
        change.apply(&mut self.buffer);
        if self.cursor_y >= self.buffer.line_count() && self.cursor_y > 0 {
            self.cursor_y -= 1;
        }
        self.cursor_x = 0;
        self.last_change = Some(LastChange::DeleteCurrentLine);
        self.commit(change, before, (self.cursor_y, self.cursor_x));
    }

    pub fn put_line_below(&mut self) {
        if let Some(yanked_line) = self.yanked_line.clone() {
            let before = (self.cursor_y, self.cursor_x);
            let change = if self.buffer.line_count() == 0 {
                self.cursor_y = 0;
                Change {
                    pos: 0,
                    removed: String::new(),
                    inserted: format!("{}\n", yanked_line),
                }
            } else {
                let idx = self.buffer.line_to_char(self.cursor_y + 1);
                self.cursor_y += 1;
                Change {
                    pos: idx,
                    removed: String::new(),
                    inserted: format!("{}\n", yanked_line),
                }
            };
            change.apply(&mut self.buffer);
            self.cursor_x = 0;
            self.last_change = Some(LastChange::PutLineBelow);
            self.commit(change, before, (self.cursor_y, self.cursor_x));
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
            t.change.invert().apply(&mut self.buffer);
            self.cursor_y = t.cursor_before.0;
            self.cursor_x = t.cursor_before.1;
            self.redo_stack.push(t);
        }
        self.coalescing = false;
    }

    pub fn redo(&mut self) {
        if let Some(t) = self.redo_stack.pop() {
            t.change.apply(&mut self.buffer);
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
        editor.yanked_line = Some("yanked_line".to_string());
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
        editor.yanked_line = Some("".to_string());
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
        editor.yanked_line = Some("yanked".to_string());
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
}
