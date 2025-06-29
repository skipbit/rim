mod application;
mod domain;
mod infrastructure;

use application::editor_service::{EditorService, HandleCommandResult};
use domain::editor_model::EditorMode;
use infrastructure::file_io::LocalFileIO;
use infrastructure::terminal_ui;

use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::env;
use std::io;
use std::time::Duration;

fn main() -> io::Result<()> {
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    enable_raw_mode()?;

    if let Err(e) = run() {
        eprintln!("Error: {:?}", e);
    }

    disable_raw_mode()?;
    execute!(stdout, LeaveAlternateScreen)?;
    Ok(())
}

use application::normal_commands::*;
use std::collections::HashMap;

fn run() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    let file_io = LocalFileIO;
    let mut editor_service = EditorService::new(file_io);
    let mut status_message = String::new();

    if args.len() > 1 {
        editor_service.open_file(&args[1])?;
    }

    let mut normal_commands: HashMap<KeyCode, Box<dyn NormalCommand<LocalFileIO>>> = HashMap::new();
    normal_commands.insert(KeyCode::Char('i'), Box::new(SwitchToInsertMode));
    normal_commands.insert(KeyCode::Char('h'), Box::new(MoveCursorLeft));
    normal_commands.insert(KeyCode::Char('j'), Box::new(MoveCursorDown));
    normal_commands.insert(KeyCode::Char('k'), Box::new(MoveCursorUp));
    normal_commands.insert(KeyCode::Char('l'), Box::new(MoveCursorRight));
    normal_commands.insert(KeyCode::Char('o'), Box::new(InsertLineBelow));
    normal_commands.insert(KeyCode::Char('O'), Box::new(InsertLineAbove));
    normal_commands.insert(KeyCode::Char('x'), Box::new(DeleteCharUnderCursor));
    normal_commands.insert(KeyCode::Char('p'), Box::new(PutLineBelow));
    normal_commands.insert(KeyCode::Char('u'), Box::new(Undo));
    normal_commands.insert(KeyCode::Char('r'), Box::new(Redo));
    normal_commands.insert(KeyCode::Char('.'), Box::new(RepeatLastChange));
    normal_commands.insert(KeyCode::Char('/'), Box::new(SwitchToSearchMode));
    normal_commands.insert(KeyCode::Char('n'), Box::new(FindNext));
    normal_commands.insert(KeyCode::Char('N'), Box::new(FindPrevious));
    normal_commands.insert(KeyCode::Char(':'), Box::new(SwitchToCommandMode));
    normal_commands.insert(KeyCode::Char('d'), Box::new(DKeyHandler));
    normal_commands.insert(KeyCode::Char('q'), Box::new(Quit));

    let mut stdout = io::stdout();
    loop {
        terminal_ui::draw_editor(&mut stdout, &editor_service.editor_model, &status_message)?;

        if event::poll(Duration::from_millis(500))? {
            if let Event::Key(event) = event::read()? {
                match editor_service.editor_model.mode {
                    EditorMode::Normal => {
                        if let Some(command) = normal_commands.get(&event.code) {
                            command.execute(&mut editor_service, &mut status_message, &event);
                            if let KeyCode::Char('q') = event.code {
                                break;
                            }
                        }
                    }
                    EditorMode::Insert => match event.code {
                        KeyCode::Esc => {
                            editor_service.set_mode(EditorMode::Normal);
                            status_message.clear();
                        }
                        KeyCode::Char(c) => {
                            editor_service.insert_char(c);
                            status_message.clear();
                        }
                        KeyCode::Enter => {
                            editor_service.editor_model.insert_newline();
                            status_message.clear();
                        }
                        KeyCode::Backspace => {
                            editor_service.delete_char();
                            status_message.clear();
                        }
                        KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right => {
                            editor_service.move_cursor(event.code);
                            status_message.clear();
                        }
                        _ => {}
                    },
                    EditorMode::Command => match event.code {
                        KeyCode::Esc => {
                            editor_service.set_mode(EditorMode::Normal);
                            editor_service.clear_command_buffer();
                            status_message.clear();
                        }
                        KeyCode::Char(c) => {
                            editor_service.push_command_char(c);
                            status_message =
                                format!(":{}", editor_service.editor_model.command_buffer);
                        }
                        KeyCode::Backspace => {
                            editor_service.pop_command_char();
                            status_message =
                                format!(":{}", editor_service.editor_model.command_buffer);
                        }
                        KeyCode::Enter => {
                            let command = editor_service.editor_model.command_buffer.clone();
                            editor_service.clear_command_buffer();
                            match editor_service.handle_command(&command) {
                                Ok(HandleCommandResult::Quit) => break,
                                Ok(HandleCommandResult::Continue) => {
                                    status_message = format!("Command executed: {}", command);
                                }
                                Err(e) => {
                                    status_message = format!("Error: {}", e);
                                }
                            }
                            editor_service.set_mode(EditorMode::Normal);
                        }
                        _ => {}
                    },
                    EditorMode::Search => match event.code {
                        KeyCode::Esc => {
                            editor_service.set_mode(EditorMode::Normal);
                            editor_service.clear_command_buffer();
                            status_message.clear();
                        }
                        KeyCode::Char(c) => {
                            editor_service.push_command_char(c);
                            status_message =
                                format!("/{}", editor_service.editor_model.command_buffer);
                        }
                        KeyCode::Backspace => {
                            editor_service.pop_command_char();
                            status_message =
                                format!("/{}", editor_service.editor_model.command_buffer);
                        }
                        KeyCode::Enter => {
                            let query = editor_service.editor_model.command_buffer.clone();
                            editor_service.search(&query);
                            editor_service.set_mode(EditorMode::Normal);
                            status_message.clear();
                        }
                        _ => {}
                    },
                }
            }
        }
    }
    Ok(())
}
