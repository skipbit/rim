use crate::domain::editor_model::{EditorMode, EditorModel};
use crate::infrastructure::file_io::FileIO;
use crossterm::event::KeyCode;
use std::io::{self, Error, ErrorKind};

pub enum HandleCommandResult {
    Continue,
    Quit,
}

pub struct EditorService<T: FileIO> {
    pub editor_model: EditorModel,
    file_io: T,
}

impl<T: FileIO> EditorService<T> {
    pub fn new(file_io: T) -> Self {
        Self {
            editor_model: EditorModel::new(),
            file_io,
        }
    }

    pub fn open_file(&mut self, filepath: &str) -> io::Result<()> {
        let content = self.file_io.read_file(filepath)?;
        self.editor_model.set_content(&content);
        self.editor_model.set_filepath(filepath.to_string());
        Ok(())
    }

    pub fn save_file(&mut self, new_filepath: Option<&str>) -> io::Result<()> {
        let path_to_save = if let Some(new_path) = new_filepath {
            self.editor_model.set_filepath(new_path.to_string());
            new_path
        } else if let Some(existing_path) = self.editor_model.get_filepath() {
            existing_path
        } else {
            return Err(Error::new(ErrorKind::Other, "No file path to save to"));
        };

        let content = self.editor_model.get_content();
        self.file_io.write_file(path_to_save, &content)
    }

    pub fn move_cursor(&mut self, key: KeyCode) {
        self.editor_model.move_cursor(key);
    }

    pub fn insert_char(&mut self, c: char) {
        self.editor_model.insert_char(c);
    }

    pub fn delete_char(&mut self) {
        self.editor_model.delete_char();
    }

    pub fn set_mode(&mut self, mode: EditorMode) {
        self.editor_model.set_mode(mode);
    }

    pub fn push_command_char(&mut self, c: char) {
        self.editor_model.push_command_char(c);
    }

    pub fn pop_command_char(&mut self) {
        self.editor_model.pop_command_char();
    }

    pub fn handle_command(&mut self, command: &str) -> io::Result<HandleCommandResult> {
        let parts: Vec<&str> = command.splitn(2, ' ').collect();
        match parts[0] {
            "w" => {
                let filepath = parts.get(1).copied();
                self.save_file(filepath)?;
                Ok(HandleCommandResult::Continue)
            }
            "q" => Ok(HandleCommandResult::Quit),
            _ => Err(Error::new(ErrorKind::InvalidInput, "Unknown command")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    // モックのFileIO実装
    struct MockFileIO {
        read_content: Option<String>,
        read_error: Option<io::Error>,
        written_data: Rc<RefCell<Vec<(String, String)>>>,
        write_error: Option<io::Error>,
    }

    impl MockFileIO {
        fn new() -> Self {
            MockFileIO {
                read_content: None,
                read_error: None,
                written_data: Rc::new(RefCell::new(Vec::new())),
                write_error: None,
            }
        }

        fn set_read_content(&mut self, content: &str) {
            self.read_content = Some(content.to_string());
        }

        fn set_read_error(&mut self, error: io::Error) {
            self.read_error = Some(error);
        }

        fn get_written_data(&self) -> Vec<(String, String)> {
            self.written_data.borrow().clone()
        }

        fn set_write_error(&mut self, error: io::Error) {
            self.write_error = Some(error);
        }
    }

    impl FileIO for MockFileIO {
        fn read_file(&self, _path: &str) -> io::Result<String> {
            if let Some(err) = &self.read_error {
                return Err(io::Error::new(err.kind(), err.to_string()));
            }
            self.read_content
                .clone()
                .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "No content set for mock"))
        }

        fn write_file(&self, path: &str, content: &str) -> io::Result<()> {
            if let Some(err) = &self.write_error {
                return Err(io::Error::new(err.kind(), err.to_string()));
            }
            self.written_data
                .borrow_mut()
                .push((path.to_string(), content.to_string()));
            Ok(())
        }
    }

    #[test]
    fn test_open_file_success() {
        let mut mock_file_io = MockFileIO::new();
        mock_file_io.set_read_content("line1\nline2");
        let mut editor_service = EditorService::new(mock_file_io);

        let result = editor_service.open_file("test.txt");
        assert!(result.is_ok());
        assert_eq!(editor_service.editor_model.lines, vec!["line1", "line2"]);
        assert_eq!(
            editor_service.editor_model.filepath,
            Some("test.txt".to_string())
        );
    }

    #[test]
    fn test_open_file_not_found() {
        let mut mock_file_io = MockFileIO::new();
        mock_file_io.set_read_error(io::Error::new(ErrorKind::NotFound, "File not found"));
        let mut editor_service = EditorService::new(mock_file_io);

        let result = editor_service.open_file("non_existent.txt");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::NotFound);
    }

    #[test]
    fn test_save_file_success() {
        let mock_file_io = MockFileIO::new();
        let mut editor_service = EditorService::new(mock_file_io);
        editor_service
            .editor_model
            .set_filepath("save_test.txt".to_string());
        editor_service.editor_model.set_content("save content");

        let result = editor_service.save_file(None);
        assert!(result.is_ok());

        // MockFileIOのインスタンスを直接保持し、そこからwritten_dataを取得
        let written = editor_service.file_io.get_written_data();
        assert_eq!(written.len(), 1);
        assert_eq!(written[0].0, "save_test.txt");
        assert_eq!(written[0].1, "save content");
    }

    #[test]
    fn test_save_file_no_filepath() {
        let mock_file_io = MockFileIO::new();
        let mut editor_service = EditorService::new(mock_file_io);

        let result = editor_service.save_file(None);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::Other);
    }

    #[test]
    fn test_save_new_file_with_command() {
        let mock_file_io = MockFileIO::new();
        let mut editor_service = EditorService::new(mock_file_io);
        editor_service.editor_model.set_content("new file content");

        let result = editor_service.handle_command("w new_file.txt");
        assert!(result.is_ok());

        let written = editor_service.file_io.get_written_data();
        assert_eq!(written.len(), 1);
        assert_eq!(written[0].0, "new_file.txt");
        assert_eq!(written[0].1, "new file content");
        assert_eq!(
            editor_service.editor_model.get_filepath(),
            Some(&"new_file.txt".to_string())
        );
    }

    #[test]
    fn test_save_file_write_error() {
        let mut mock_file_io = MockFileIO::new();
        mock_file_io.set_write_error(io::Error::new(
            ErrorKind::PermissionDenied,
            "Permission denied",
        ));
        let mut editor_service = EditorService::new(mock_file_io);
        editor_service
            .editor_model
            .set_filepath("error_test.txt".to_string());
        editor_service.editor_model.set_content("error content");

        let result = editor_service.save_file(None);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::PermissionDenied);
    }

    // カーソル移動、文字挿入、削除はEditorModelでテスト済みなので、ここではEditorServiceがそれらを正しく呼び出しているかを確認する簡単なテストに留める
    #[test]
    fn test_move_cursor_delegation() {
        let mock_file_io = MockFileIO::new();
        let mut editor_service = EditorService::new(mock_file_io);
        editor_service.editor_model.lines.push("line1".to_string());
        editor_service.editor_model.lines.push("line2".to_string());
        editor_service.editor_model.cursor_y = 1;

        editor_service.move_cursor(KeyCode::Up);
        assert_eq!(editor_service.editor_model.cursor_y, 0);
    }

    #[test]
    fn test_insert_char_delegation() {
        let mock_file_io = MockFileIO::new();
        let mut editor_service = EditorService::new(mock_file_io);
        editor_service.editor_model.lines.push("".to_string());

        editor_service.insert_char('a');
        assert_eq!(editor_service.editor_model.lines[0], "a");
    }

    #[test]
    fn test_delete_char_delegation() {
        let mock_file_io = MockFileIO::new();
        let mut editor_service = EditorService::new(mock_file_io);
        editor_service.editor_model.lines.push("abc".to_string());
        editor_service.editor_model.cursor_x = 3;

        editor_service.delete_char();
        assert_eq!(editor_service.editor_model.lines[0], "ab");
    }
}
