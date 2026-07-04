use crate::application::editor_service::{EditorService, HandleCommandResult};
use crate::application::lsp::LspRequest;
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

/// `:format` / `:fmt` — request LSP whole-document formatting. The async result
/// is applied as a single undo step by the orchestrator.
pub struct FormatCommand;

impl<T: FileIO> EditorCommand<T> for FormatCommand {
    fn execute(&self, editor_service: &mut EditorService<T>) -> io::Result<HandleCommandResult> {
        editor_service.request_lsp(LspRequest::Format);
        Ok(HandleCommandResult::Continue)
    }

    fn names(&self) -> Vec<&'static str> {
        vec!["format", "fmt"]
    }
}

/// `:rename <new>` — rename the symbol under the cursor. Edits in the current
/// buffer are applied as one undo step; edits in other files are reported but
/// not applied (single-buffer editor).
pub struct RenameCommand {
    new_name: String,
}

impl RenameCommand {
    pub fn new(new_name: String) -> Self {
        Self { new_name }
    }
}

impl<T: FileIO> EditorCommand<T> for RenameCommand {
    fn execute(&self, editor_service: &mut EditorService<T>) -> io::Result<HandleCommandResult> {
        let (y, x) = (
            editor_service.editor_model.cursor_y,
            editor_service.editor_model.cursor_x,
        );
        editor_service.request_lsp(LspRequest::Rename {
            y,
            x,
            new_name: self.new_name.clone(),
        });
        Ok(HandleCommandResult::Continue)
    }

    fn names(&self) -> Vec<&'static str> {
        vec!["rename"]
    }
}
