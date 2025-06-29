use crossterm::event::KeyCode;

pub enum EditorMode {
    Normal,
    Insert,
    Command,
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
    pub lines: Vec<String>,
    pub cursor_x: usize,
    pub cursor_y: usize,
    pub filepath: Option<String>,
    pub mode: EditorMode,
    pub command_buffer: String,
    pub yanked_line: Option<String>,
    history: Vec<(Vec<String>, usize, usize)>,
    history_index: usize,
    last_change: Option<LastChange>,
}

impl EditorModel {
    pub fn new() -> Self {
        Self {
            mode: EditorMode::Normal,
            command_buffer: String::new(),
            lines: Vec::new(),
            cursor_x: 0,
            cursor_y: 0,
            filepath: None,
            yanked_line: None,
            history: Vec::new(),
            history_index: 0,
            last_change: None,
        }
    }

    pub fn set_content(&mut self, content: &str) {
        self.lines = content.lines().map(|s| s.to_string()).collect();
    }

    pub fn get_content(&self) -> String {
        self.lines.join("\n")
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
                if self.cursor_y < self.lines.len().saturating_sub(1) {
                    self.cursor_y += 1;
                }
            }
            KeyCode::Left => {
                if self.cursor_x > 0 {
                    self.cursor_x -= 1;
                } else if self.cursor_y > 0 {
                    self.cursor_y -= 1;
                    self.cursor_x = self.lines[self.cursor_y].len();
                }
            }
            KeyCode::Right => {
                if self.cursor_y < self.lines.len() {
                    let line_len = self.lines[self.cursor_y].len();
                    if self.cursor_x < line_len {
                        self.cursor_x += 1;
                    } else if self.cursor_y < self.lines.len() - 1 {
                        self.cursor_y += 1;
                        self.cursor_x = 0;
                    }
                }
            }
            _ => {}
        }
        // Snap cursor to end of line if it's past the end
        if self.cursor_y < self.lines.len() {
            let line_len = self.lines[self.cursor_y].len();
            if self.cursor_x > line_len {
                self.cursor_x = line_len;
            }
        }
    }

    pub fn insert_char(&mut self, c: char) {
        if self.cursor_y >= self.lines.len() {
            self.lines.push(String::new());
        }
        self.lines[self.cursor_y].insert(self.cursor_x, c);
        self.cursor_x += 1;
        self.last_change = Some(LastChange::InsertChar(c));
        self.save_snapshot();
    }

    pub fn delete_char(&mut self) {
        if self.cursor_y >= self.lines.len() {
            return;
        }
        if self.cursor_x > 0 {
            self.lines[self.cursor_y].remove(self.cursor_x - 1);
            self.cursor_x -= 1;
            self.last_change = Some(LastChange::DeleteChar);
            self.save_snapshot();
        } else if self.cursor_y > 0 {
            let prev_line = self.lines.remove(self.cursor_y);
            self.cursor_y -= 1;
            self.cursor_x = self.lines[self.cursor_y].len();
            self.lines[self.cursor_y].push_str(&prev_line);
            self.last_change = Some(LastChange::DeleteChar);
            self.save_snapshot();
        }
    }

    pub fn delete_char_under_cursor(&mut self) {
        if self.cursor_y >= self.lines.len() {
            return;
        }
        if self.cursor_x < self.lines[self.cursor_y].len() {
            self.lines[self.cursor_y].remove(self.cursor_x);
            self.last_change = Some(LastChange::DeleteCharUnderCursor);
            self.save_snapshot();
        }
    }

    pub fn insert_newline(&mut self) {
        if self.cursor_y >= self.lines.len() {
            self.lines.push(String::new());
        }
        let current_line = &mut self.lines[self.cursor_y];
        let remaining_part = current_line.split_off(self.cursor_x);
        self.lines.insert(self.cursor_y + 1, remaining_part);
        self.cursor_y += 1;
        self.cursor_x = 0;
        self.last_change = Some(LastChange::InsertNewline);
        self.save_snapshot();
    }

    pub fn insert_line_below(&mut self) {
        if self.lines.is_empty() {
            self.lines.push(String::new());
            self.cursor_y = 0;
        } else {
            self.lines.insert(self.cursor_y + 1, String::new());
            self.cursor_y += 1;
        }
        self.cursor_x = 0;
        self.last_change = Some(LastChange::InsertLineBelow);
        self.save_snapshot();
    }

    pub fn insert_line_above(&mut self) {
        self.lines.insert(self.cursor_y, String::new());
        self.cursor_x = 0;
        self.last_change = Some(LastChange::InsertLineAbove);
        self.save_snapshot();
    }

    pub fn delete_current_line(&mut self) {
        if !self.lines.is_empty() {
            self.lines.remove(self.cursor_y);
            if self.cursor_y >= self.lines.len() && self.cursor_y > 0 {
                self.cursor_y -= 1;
            }
            if self.lines.is_empty() {
                self.lines.push(String::new());
            }
            self.cursor_x = 0;
            self.last_change = Some(LastChange::DeleteCurrentLine);
            self.save_snapshot();
        }
    }

    pub fn put_line_below(&mut self) {
        if let Some(yanked_line) = &self.yanked_line {
            if self.lines.is_empty() {
                self.lines.push(yanked_line.clone());
                self.cursor_y = 0;
            } else {
                self.lines.insert(self.cursor_y + 1, yanked_line.clone());
                self.cursor_y += 1;
            }
            self.cursor_x = 0;
            self.last_change = Some(LastChange::PutLineBelow);
            self.save_snapshot();
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

    pub fn save_snapshot(&mut self) {
        // Clear any 'future' history if we're not at the latest state
        self.history.truncate(self.history_index);
        self.history
            .push((self.lines.clone(), self.cursor_x, self.cursor_y));
        self.history_index += 1;
    }

    pub fn undo(&mut self) {
        if self.history_index > 1 {
            self.history_index -= 1;
            let (lines, cursor_x, cursor_y) = &self.history[self.history_index - 1];
            self.lines = lines.clone();
            self.cursor_x = *cursor_x;
            self.cursor_y = *cursor_y;
        }
    }

    pub fn redo(&mut self) {
        if self.history_index < self.history.len() {
            self.history_index += 1;
            let (lines, cursor_x, cursor_y) = &self.history[self.history_index - 1];
            self.lines = lines.clone();
            self.cursor_x = *cursor_x;
            self.cursor_y = *cursor_y;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_move_cursor_up() {
        let mut editor = EditorModel::new();
        editor.lines.push("line1".to_string());
        editor.lines.push("line2".to_string());
        editor.cursor_y = 1;
        editor.move_cursor(KeyCode::Up);
        assert_eq!(editor.cursor_y, 0);
    }

    #[test]
    fn test_move_cursor_down() {
        let mut editor = EditorModel::new();
        editor.lines.push("line1".to_string());
        editor.lines.push("line2".to_string());
        editor.cursor_y = 0;
        editor.move_cursor(KeyCode::Down);
        assert_eq!(editor.cursor_y, 1);
    }

    #[test]
    fn test_move_cursor_left() {
        let mut editor = EditorModel::new();
        editor.lines.push("line1".to_string());
        editor.cursor_x = 3;
        editor.move_cursor(KeyCode::Left);
        assert_eq!(editor.cursor_x, 2);
    }

    #[test]
    fn test_move_cursor_right() {
        let mut editor = EditorModel::new();
        editor.lines.push("line1".to_string());
        editor.cursor_x = 0;
        editor.move_cursor(KeyCode::Right);
        assert_eq!(editor.cursor_x, 1);
    }

    #[test]
    fn test_insert_char() {
        let mut editor = EditorModel::new();
        editor.lines.push("".to_string());
        editor.insert_char('a');
        assert_eq!(editor.lines[0], "a");
        assert_eq!(editor.cursor_x, 1);
    }

    #[test]
    fn test_delete_char() {
        let mut editor = EditorModel::new();
        editor.lines.push("abc".to_string());
        editor.cursor_x = 3;
        editor.delete_char();
        assert_eq!(editor.lines[0], "ab");
        assert_eq!(editor.cursor_x, 2);
    }

    #[test]
    fn test_delete_char_at_beginning_of_line() {
        let mut editor = EditorModel::new();
        editor.lines.push("line1".to_string());
        editor.lines.push("line2".to_string());
        editor.cursor_y = 1;
        editor.cursor_x = 0;
        editor.delete_char();
        assert_eq!(editor.lines.len(), 1);
        assert_eq!(editor.lines[0], "line1line2");
        assert_eq!(editor.cursor_y, 0);
        assert_eq!(editor.cursor_x, 5);
    }

    #[test]
    fn test_insert_newline_middle_of_line() {
        let mut editor = EditorModel::new();
        editor.lines.push("Hello World".to_string());
        editor.cursor_x = 5; // カーソルを 'o' と ' ' の間に設定
        editor.cursor_y = 0;
        editor.insert_newline();
        assert_eq!(editor.lines.len(), 2);
        assert_eq!(editor.lines[0], "Hello");
        assert_eq!(editor.lines[1], " World");
        assert_eq!(editor.cursor_y, 1);
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_insert_newline_end_of_line() {
        let mut editor = EditorModel::new();
        editor.lines.push("Hello World".to_string());
        editor.cursor_x = 11; // カーソルを 'd' の後に設定
        editor.cursor_y = 0;
        editor.insert_newline();
        assert_eq!(editor.lines.len(), 2);
        assert_eq!(editor.lines[0], "Hello World");
        assert_eq!(editor.lines[1], "");
        assert_eq!(editor.cursor_y, 1);
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_insert_newline_empty_line() {
        let mut editor = EditorModel::new();
        editor.lines.push("".to_string());
        editor.cursor_x = 0;
        editor.cursor_y = 0;
        editor.insert_newline();
        assert_eq!(editor.lines.len(), 2);
        assert_eq!(editor.lines[0], "");
        assert_eq!(editor.lines[1], "");
        assert_eq!(editor.cursor_y, 1);
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_insert_line_below() {
        let mut editor = EditorModel::new();
        editor.lines.push("line1".to_string());
        editor.lines.push("line2".to_string());
        editor.cursor_y = 0;
        editor.insert_line_below();
        assert_eq!(editor.lines.len(), 3);
        assert_eq!(editor.lines[1], "");
        assert_eq!(editor.cursor_y, 1);
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_insert_line_above() {
        let mut editor = EditorModel::new();
        editor.lines.push("line1".to_string());
        editor.lines.push("line2".to_string());
        editor.cursor_y = 1;
        editor.insert_line_above();
        assert_eq!(editor.lines.len(), 3);
        assert_eq!(editor.lines[1], "");
        assert_eq!(editor.cursor_y, 1);
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_delete_char_under_cursor() {
        let mut editor = EditorModel::new();
        editor.lines.push("abc".to_string());
        editor.cursor_x = 1;
        editor.delete_char_under_cursor();
        assert_eq!(editor.lines[0], "ac");
        assert_eq!(editor.cursor_x, 1);
    }

    #[test]
    fn test_delete_char_under_cursor_at_end_of_line() {
        let mut editor = EditorModel::new();
        editor.lines.push("abc".to_string());
        editor.cursor_x = 2;
        editor.delete_char_under_cursor();
        assert_eq!(editor.lines[0], "ab");
        assert_eq!(editor.cursor_x, 2);
    }

    #[test]
    fn test_delete_char_under_cursor_empty_line() {
        let mut editor = EditorModel::new();
        editor.lines.push("".to_string());
        editor.cursor_x = 0;
        editor.delete_char_under_cursor();
        assert_eq!(editor.lines[0], "");
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_delete_current_line() {
        let mut editor = EditorModel::new();
        editor.lines.push("line1".to_string());
        editor.lines.push("line2".to_string());
        editor.lines.push("line3".to_string());
        editor.cursor_y = 1;
        editor.delete_current_line();
        assert_eq!(editor.lines.len(), 2);
        assert_eq!(editor.lines[0], "line1");
        assert_eq!(editor.lines[1], "line3");
        assert_eq!(editor.cursor_y, 1);
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_delete_current_line_first_line() {
        let mut editor = EditorModel::new();
        editor.lines.push("line1".to_string());
        editor.lines.push("line2".to_string());
        editor.cursor_y = 0;
        editor.delete_current_line();
        assert_eq!(editor.lines.len(), 1);
        assert_eq!(editor.lines[0], "line2");
        assert_eq!(editor.cursor_y, 0);
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_delete_current_line_last_line() {
        let mut editor = EditorModel::new();
        editor.lines.push("line1".to_string());
        editor.lines.push("line2".to_string());
        editor.cursor_y = 1;
        editor.delete_current_line();
        assert_eq!(editor.lines.len(), 1);
        assert_eq!(editor.lines[0], "line1");
        assert_eq!(editor.cursor_y, 0);
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_delete_current_line_single_line() {
        let mut editor = EditorModel::new();
        editor.lines.push("line1".to_string());
        editor.cursor_y = 0;
        editor.delete_current_line();
        assert_eq!(editor.lines.len(), 1);
        assert_eq!(editor.lines[0], "");
        assert_eq!(editor.cursor_y, 0);
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_put_line_below() {
        let mut editor = EditorModel::new();
        editor.lines.push("line1".to_string());
        editor.lines.push("line2".to_string());
        editor.cursor_y = 0;
        editor.yanked_line = Some("yanked_line".to_string());
        editor.put_line_below();
        assert_eq!(editor.lines.len(), 3);
        assert_eq!(editor.lines[1], "yanked_line");
        assert_eq!(editor.cursor_y, 1);
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_put_line_below_empty_yanked_line() {
        let mut editor = EditorModel::new();
        editor.lines.push("line1".to_string());
        editor.cursor_y = 0;
        editor.yanked_line = Some("".to_string());
        editor.put_line_below();
        assert_eq!(editor.lines.len(), 2);
        assert_eq!(editor.lines[1], "");
        assert_eq!(editor.cursor_y, 1);
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_save_snapshot() {
        let mut editor = EditorModel::new();
        editor.lines.push("line1".to_string());
        editor.save_snapshot();
        assert_eq!(editor.history.len(), 1);
        assert_eq!(editor.history_index, 1);
        assert_eq!(editor.history[0].0, vec!["line1"]);
    }

    #[test]
    fn test_undo() {
        let mut editor = EditorModel::new();
        editor.lines.push("line1".to_string());
        editor.save_snapshot();
        editor.lines.push("line2".to_string());
        editor.save_snapshot();
        editor.undo();
        assert_eq!(editor.lines, vec!["line1"]);
        assert_eq!(editor.history_index, 1);
    }

    #[test]
    fn test_redo() {
        let mut editor = EditorModel::new();
        editor.lines.push("line1".to_string());
        editor.save_snapshot();
        editor.lines.push("line2".to_string());
        editor.save_snapshot();
        editor.undo();
        editor.redo();
        assert_eq!(editor.lines, vec!["line1", "line2"]);
        assert_eq!(editor.history_index, 2);
    }

    #[test]
    fn test_save_snapshot_after_undo() {
        let mut editor = EditorModel::new();
        editor.lines.push("line1".to_string());
        editor.save_snapshot();
        editor.lines.push("line2".to_string());
        editor.save_snapshot();
        editor.undo();
        editor.lines.push("line3".to_string());
        editor.save_snapshot();
        assert_eq!(editor.history.len(), 2);
        assert_eq!(editor.history_index, 2);
        assert_eq!(editor.history[1].0, vec!["line1", "line3"]);
    }

    #[test]
    fn test_repeat_last_change_insert_char() {
        let mut editor = EditorModel::new();
        editor.lines.push("".to_string());
        editor.insert_char('a');
        editor.repeat_last_change();
        assert_eq!(editor.lines[0], "aa");
        assert_eq!(editor.cursor_x, 2);
    }

    #[test]
    fn test_repeat_last_change_delete_char() {
        let mut editor = EditorModel::new();
        editor.lines.push("abc".to_string());
        editor.cursor_x = 3;
        editor.delete_char();
        editor.repeat_last_change();
        assert_eq!(editor.lines[0], "a");
        assert_eq!(editor.cursor_x, 1);
    }

    #[test]
    fn test_repeat_last_change_delete_char_under_cursor() {
        let mut editor = EditorModel::new();
        editor.lines.push("abc".to_string());
        editor.cursor_x = 0;
        editor.delete_char_under_cursor();
        editor.repeat_last_change();
        assert_eq!(editor.lines[0], "c");
        assert_eq!(editor.cursor_x, 0);
    }

    #[test]
    fn test_repeat_last_change_insert_newline() {
        let mut editor = EditorModel::new();
        editor.lines.push("line1".to_string());
        editor.cursor_x = 5;
        editor.insert_newline();
        editor.repeat_last_change();
        assert_eq!(editor.lines.len(), 3);
        assert_eq!(editor.lines[0], "line1");
        assert_eq!(editor.lines[1], "");
        assert_eq!(editor.lines[2], "");
    }

    #[test]
    fn test_repeat_last_change_insert_line_below() {
        let mut editor = EditorModel::new();
        editor.lines.push("line1".to_string());
        editor.insert_line_below();
        editor.repeat_last_change();
        assert_eq!(editor.lines.len(), 3);
        assert_eq!(editor.lines[0], "line1");
        assert_eq!(editor.lines[1], "");
        assert_eq!(editor.lines[2], "");
    }

    #[test]
    fn test_repeat_last_change_insert_line_above() {
        let mut editor = EditorModel::new();
        editor.lines.push("line1".to_string());
        editor.cursor_y = 0;
        editor.insert_line_above();
        editor.repeat_last_change();
        assert_eq!(editor.lines.len(), 3);
        assert_eq!(editor.lines[0], "");
        assert_eq!(editor.lines[1], "");
        assert_eq!(editor.lines[2], "line1");
    }

    #[test]
    fn test_repeat_last_change_delete_current_line() {
        let mut editor = EditorModel::new();
        editor.lines.push("line1".to_string());
        editor.lines.push("line2".to_string());
        editor.delete_current_line();
        editor.repeat_last_change();
        assert_eq!(editor.lines.len(), 1);
        assert_eq!(editor.lines[0], "");
    }

    #[test]
    fn test_repeat_last_change_put_line_below() {
        let mut editor = EditorModel::new();
        editor.lines.push("line1".to_string());
        editor.yanked_line = Some("yanked".to_string());
        editor.put_line_below();
        editor.repeat_last_change();
        assert_eq!(editor.lines.len(), 3);
        assert_eq!(editor.lines[1], "yanked");
        assert_eq!(editor.lines[2], "yanked");
    }

    #[test]
    fn test_insert_line_below_empty_document() {
        let mut editor = EditorModel::new();
        editor.insert_line_below();
        assert_eq!(editor.lines.len(), 1);
        assert_eq!(editor.lines[0], "");
        assert_eq!(editor.cursor_y, 0);
        assert_eq!(editor.cursor_x, 0);
    }
}