use crate::domain::editor_model::EditorModel;
use crate::infrastructure::file_io::FileIO;
use crossterm::event::KeyCode;
use std::io::{self, Error, ErrorKind};

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

    pub fn save_file(&self) -> io::Result<()> {
        if let Some(path) = self.editor_model.get_filepath() {
            let content = self.editor_model.get_content();
            self.file_io.write_file(path, &content)
        } else {
            Err(Error::new(ErrorKind::Other, "No file path to save to"))
        }
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
}
