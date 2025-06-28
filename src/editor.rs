use crossterm::event::KeyCode;
use std::fs;
use std::io::{self, Error, ErrorKind};

pub struct Editor {
    pub lines: Vec<String>,
    pub cursor_x: usize,
    pub cursor_y: usize,
    pub filepath: Option<String>,
}

impl Editor {
    pub fn new() -> Self {
        Self {
            lines: Vec::new(),
            cursor_x: 0,
            cursor_y: 0,
            filepath: None,
        }
    }

    pub fn open(&mut self, filepath: &str) -> io::Result<()> {
        let content =
            fs::read_to_string(filepath).map_err(|e| Error::new(ErrorKind::NotFound, e))?;
        self.lines = content.lines().map(|s| s.to_string()).collect();
        self.filepath = Some(filepath.to_string());
        Ok(())
    }

    pub fn save(&self) -> io::Result<()> {
        if let Some(path) = &self.filepath {
            let content = self.lines.join("\n");
            fs::write(path, content)
        } else {
            Err(Error::new(ErrorKind::Other, "No file path to save to"))
        }
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_move_cursor_up() {
        let mut editor = Editor::new();
        editor.lines.push("line1".to_string());
        editor.lines.push("line2".to_string());
        editor.cursor_y = 1;
        editor.move_cursor(KeyCode::Up);
        assert_eq!(editor.cursor_y, 0);
    }

    #[test]
    fn test_move_cursor_down() {
        let mut editor = Editor::new();
        editor.lines.push("line1".to_string());
        editor.lines.push("line2".to_string());
        editor.cursor_y = 0;
        editor.move_cursor(KeyCode::Down);
        assert_eq!(editor.cursor_y, 1);
    }

    #[test]
    fn test_move_cursor_left() {
        let mut editor = Editor::new();
        editor.lines.push("line1".to_string());
        editor.cursor_x = 3;
        editor.move_cursor(KeyCode::Left);
        assert_eq!(editor.cursor_x, 2);
    }

    #[test]
    fn test_move_cursor_right() {
        let mut editor = Editor::new();
        editor.lines.push("line1".to_string());
        editor.cursor_x = 0;
        editor.move_cursor(KeyCode::Right);
        assert_eq!(editor.cursor_x, 1);
    }

    #[test]
    fn test_insert_char() {
        let mut editor = Editor::new();
        editor.lines.push("".to_string());
        editor.insert_char('a');
        assert_eq!(editor.lines[0], "a");
        assert_eq!(editor.cursor_x, 1);
    }

    #[test]
    fn test_delete_char() {
        let mut editor = Editor::new();
        editor.lines.push("abc".to_string());
        editor.cursor_x = 3;
        editor.delete_char();
        assert_eq!(editor.lines[0], "ab");
        assert_eq!(editor.cursor_x, 2);
    }

    #[test]
    fn test_delete_char_at_beginning_of_line() {
        let mut editor = Editor::new();
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
}
