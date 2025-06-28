use crate::application::editor_service::{EditorService, HandleCommandResult};
use crate::infrastructure::file_io::FileIO;
use std::io;

pub trait EditorCommand<T: FileIO> {
    fn execute(&self, editor_service: &mut EditorService<T>) -> io::Result<HandleCommandResult>;
    fn names(&self) -> Vec<&'static str>;
}

pub struct WriteCommand {
    filepath: Option<String>,
}

impl WriteCommand {
    pub fn new(filepath: Option<String>) -> Self {
        Self { filepath }
    }
}

impl<T: FileIO> EditorCommand<T> for WriteCommand {
    fn execute(&self, editor_service: &mut EditorService<T>) -> io::Result<HandleCommandResult> {
        editor_service.save_file(self.filepath.as_deref())?;
        Ok(HandleCommandResult::Continue)
    }

    fn names(&self) -> Vec<&'static str> {
        vec!["w", "write"]
    }
}

pub struct QuitCommand;

impl<T: FileIO> EditorCommand<T> for QuitCommand {
    fn execute(&self, _editor_service: &mut EditorService<T>) -> io::Result<HandleCommandResult> {
        Ok(HandleCommandResult::Quit)
    }

    fn names(&self) -> Vec<&'static str> {
        vec!["q", "quit"]
    }
}

pub struct EditCommand {
    filepath: String,
}

impl EditCommand {
    pub fn new(filepath: String) -> Self {
        Self { filepath }
    }
}

impl<T: FileIO> EditorCommand<T> for EditCommand {
    fn execute(&self, editor_service: &mut EditorService<T>) -> io::Result<HandleCommandResult> {
        editor_service.open_file(&self.filepath)?;
        Ok(HandleCommandResult::Continue)
    }

    fn names(&self) -> Vec<&'static str> {
        vec!["e", "edit"]
    }
}
