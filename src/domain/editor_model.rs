use crossterm::event::KeyCode;

pub enum EditorMode {
    Normal,
    Insert,
    Command,
}

pub struct EditorModel {
    pub lines: Vec<String>,
    pub cursor_x: usize,
    pub cursor_y: usize,
    pub filepath: Option<String>,
    pub mode: EditorMode,
    pub command_buffer: String,
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
    }

    pub fn delete_char(&mut self) {
        if self.cursor_y >= self.lines.len() {
            return;
        }
        if self.cursor_x > 0 {
            self.lines[self.cursor_y].remove(self.cursor_x - 1);
            self.cursor_x -= 1;
        } else if self.cursor_y > 0 {
            let prev_line = self.lines.remove(self.cursor_y);
            self.cursor_y -= 1;
            self.cursor_x = self.lines[self.cursor_y].len();
            self.lines[self.cursor_y].push_str(&prev_line);
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
    }

    pub fn move_cursor_for_append(&mut self) {
        if self.cursor_y < self.lines.len() {
            let line_len = self.lines[self.cursor_y].len();
            if self.cursor_x < line_len {
                self.cursor_x += 1;
            } else if self.cursor_y < self.lines.len().saturating_sub(1) {
                // If at end of line, move to next line and beginning
                self.cursor_y += 1;
                self.cursor_x = 0;
            }
        }
    }

    pub fn move_cursor_for_append_at_line_end(&mut self) {
        if self.cursor_y < self.lines.len() {
            self.cursor_x = self.lines[self.cursor_y].len();
        }
    }

    pub fn set_mode(&mut self, mode: EditorMode) {
        self.mode = mode;
    }

    #[allow(dead_code)]
    pub fn get_mode(&self) -> &EditorMode {
        &self.mode
    }

    pub fn push_command_char(&mut self, c: char) {
        self.command_buffer.push(c);
    }

    pub fn pop_command_char(&mut self) {
        self.command_buffer.pop();
    }

    pub fn clear_command_buffer(&mut self) {
        self.command_buffer.clear();
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
    fn test_move_cursor_for_append_middle_of_line() {
        let mut editor = EditorModel::new();
        editor.lines.push("Hello World".to_string());
        editor.cursor_x = 5;
        editor.cursor_y = 0;
        editor.move_cursor_for_append();
        assert_eq!(editor.cursor_x, 6);
        assert_eq!(editor.cursor_y, 0);
    }

    #[test]
    fn test_move_cursor_for_append_end_of_line() {
        let mut editor = EditorModel::new();
        editor.lines.push("Hello World".to_string());
        editor.cursor_x = 11;
        editor.cursor_y = 0;
        editor.move_cursor_for_append();
        assert_eq!(editor.cursor_x, 11);
        assert_eq!(editor.cursor_y, 0);
    }

    #[test]
    fn test_move_cursor_for_append_at_line_end_middle_of_line() {
        let mut editor = EditorModel::new();
        editor.lines.push("Hello World".to_string());
        editor.cursor_x = 5;
        editor.cursor_y = 0;
        editor.move_cursor_for_append_at_line_end();
        assert_eq!(editor.cursor_x, 11);
        assert_eq!(editor.cursor_y, 0);
    }

    #[test]
    fn test_move_cursor_for_append_at_line_end_end_of_line() {
        let mut editor = EditorModel::new();
        editor.lines.push("Hello World".to_string());
        editor.cursor_x = 11;
        editor.cursor_y = 0;
        editor.move_cursor_for_append_at_line_end();
        assert_eq!(editor.cursor_x, 11);
        assert_eq!(editor.cursor_y, 0);
    }
}
