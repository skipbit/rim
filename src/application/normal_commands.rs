use crate::application::editor_service::EditorService;
use crate::domain::editor_model::EditorMode;
use crate::infrastructure::file_io::FileIO;
use crossterm::event::{KeyCode, KeyModifiers};

pub trait NormalCommand<T: FileIO> {
    fn execute(
        &self,
        editor_service: &mut EditorService<T>,
        status_message: &mut String,
        event: &crossterm::event::KeyEvent,
    );
}

pub struct SwitchToInsertMode;

impl<T: FileIO> NormalCommand<T> for SwitchToInsertMode {
    fn execute(
        &self,
        editor_service: &mut EditorService<T>,
        status_message: &mut String,
        _event: &crossterm::event::KeyEvent,
    ) {
        editor_service.set_mode(EditorMode::Insert);
        *status_message = "-- INSERT --".to_string();
    }
}

pub struct MoveCursorLeft;

impl<T: FileIO> NormalCommand<T> for MoveCursorLeft {
    fn execute(
        &self,
        editor_service: &mut EditorService<T>,
        status_message: &mut String,
        _event: &crossterm::event::KeyEvent,
    ) {
        editor_service.move_cursor(KeyCode::Left);
        status_message.clear();
    }
}

pub struct MoveCursorDown;

impl<T: FileIO> NormalCommand<T> for MoveCursorDown {
    fn execute(
        &self,
        editor_service: &mut EditorService<T>,
        status_message: &mut String,
        _event: &crossterm::event::KeyEvent,
    ) {
        editor_service.move_cursor(KeyCode::Down);
        status_message.clear();
    }
}

pub struct MoveCursorUp;

impl<T: FileIO> NormalCommand<T> for MoveCursorUp {
    fn execute(
        &self,
        editor_service: &mut EditorService<T>,
        status_message: &mut String,
        _event: &crossterm::event::KeyEvent,
    ) {
        editor_service.move_cursor(KeyCode::Up);
        status_message.clear();
    }
}

pub struct MoveCursorRight;

impl<T: FileIO> NormalCommand<T> for MoveCursorRight {
    fn execute(
        &self,
        editor_service: &mut EditorService<T>,
        status_message: &mut String,
        _event: &crossterm::event::KeyEvent,
    ) {
        editor_service.move_cursor(KeyCode::Right);
        status_message.clear();
    }
}

pub struct InsertLineBelow;

impl<T: FileIO> NormalCommand<T> for InsertLineBelow {
    fn execute(
        &self,
        editor_service: &mut EditorService<T>,
        status_message: &mut String,
        _event: &crossterm::event::KeyEvent,
    ) {
        editor_service.editor_model.insert_line_below();
        editor_service.set_mode(EditorMode::Insert);
        *status_message = "-- INSERT --".to_string();
    }
}

pub struct InsertLineAbove;

impl<T: FileIO> NormalCommand<T> for InsertLineAbove {
    fn execute(
        &self,
        editor_service: &mut EditorService<T>,
        status_message: &mut String,
        _event: &crossterm::event::KeyEvent,
    ) {
        editor_service.editor_model.insert_line_above();
        editor_service.set_mode(EditorMode::Insert);
        *status_message = "-- INSERT --".to_string();
    }
}

pub struct DeleteCharUnderCursor;

impl<T: FileIO> NormalCommand<T> for DeleteCharUnderCursor {
    fn execute(
        &self,
        editor_service: &mut EditorService<T>,
        status_message: &mut String,
        _event: &crossterm::event::KeyEvent,
    ) {
        editor_service.editor_model.delete_char_under_cursor();
        status_message.clear();
    }
}

pub struct PutLineBelow;

impl<T: FileIO> NormalCommand<T> for PutLineBelow {
    fn execute(
        &self,
        editor_service: &mut EditorService<T>,
        status_message: &mut String,
        _event: &crossterm::event::KeyEvent,
    ) {
        editor_service.editor_model.put_line_below();
        status_message.clear();
    }
}

pub struct Undo;

impl<T: FileIO> NormalCommand<T> for Undo {
    fn execute(
        &self,
        editor_service: &mut EditorService<T>,
        status_message: &mut String,
        _event: &crossterm::event::KeyEvent,
    ) {
        editor_service.editor_model.undo();
        *status_message = "Undo".to_string();
    }
}

pub struct Redo;

impl<T: FileIO> NormalCommand<T> for Redo {
    fn execute(
        &self,
        editor_service: &mut EditorService<T>,
        status_message: &mut String,
        event: &crossterm::event::KeyEvent,
    ) {
        if event.modifiers.contains(KeyModifiers::CONTROL) {
            editor_service.editor_model.redo();
            *status_message = "Redo".to_string();
        }
    }
}

pub struct RepeatLastChange;

impl<T: FileIO> NormalCommand<T> for RepeatLastChange {
    fn execute(
        &self,
        editor_service: &mut EditorService<T>,
        status_message: &mut String,
        _event: &crossterm::event::KeyEvent,
    ) {
        editor_service.editor_model.repeat_last_change();
        status_message.clear();
    }
}

pub struct SwitchToSearchMode;

impl<T: FileIO> NormalCommand<T> for SwitchToSearchMode {
    fn execute(
        &self,
        editor_service: &mut EditorService<T>,
        status_message: &mut String,
        _event: &crossterm::event::KeyEvent,
    ) {
        editor_service.set_mode(EditorMode::Search);
        editor_service.clear_command_buffer();
        *status_message = "/".to_string();
    }
}

pub struct FindNext;

impl<T: FileIO> NormalCommand<T> for FindNext {
    fn execute(
        &self,
        editor_service: &mut EditorService<T>,
        _status_message: &mut String,
        _event: &crossterm::event::KeyEvent,
    ) {
        editor_service.find_next();
    }
}

pub struct FindPrevious;

impl<T: FileIO> NormalCommand<T> for FindPrevious {
    fn execute(
        &self,
        editor_service: &mut EditorService<T>,
        _status_message: &mut String,
        _event: &crossterm::event::KeyEvent,
    ) {
        editor_service.find_previous();
    }
}

pub struct SwitchToCommandMode;

impl<T: FileIO> NormalCommand<T> for SwitchToCommandMode {
    fn execute(
        &self,
        editor_service: &mut EditorService<T>,
        status_message: &mut String,
        _event: &crossterm::event::KeyEvent,
    ) {
        editor_service.set_mode(EditorMode::Command);
        *status_message = ":".to_string();
    }
}

pub struct DKeyHandler;

impl<T: FileIO> NormalCommand<T> for DKeyHandler {
    fn execute(
        &self,
        editor_service: &mut EditorService<T>,
        status_message: &mut String,
        event: &crossterm::event::KeyEvent,
    ) {
        if editor_service.editor_model.d_pressed {
            match event.code {
                KeyCode::Char('d') => {
                    editor_service.editor_model.delete_current_line();
                    status_message.clear();
                }
                KeyCode::Char('y') => {
                    editor_service.yank_current_line();
                    *status_message = "Yanked current line.".to_string();
                }
                _ => {}
            }
            editor_service.editor_model.d_pressed = false;
        } else {
            editor_service.editor_model.d_pressed = true;
            *status_message = "d".to_string();
        }
    }
}

pub struct Quit;

impl<T: FileIO> NormalCommand<T> for Quit {
    fn execute(
        &self,
        _editor_service: &mut EditorService<T>,
        _status_message: &mut String,
        _event: &crossterm::event::KeyEvent,
    ) {
        // This command will be handled directly in main.rs loop to break
    }
}
