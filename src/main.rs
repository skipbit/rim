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

fn run() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    let file_io = LocalFileIO;
    let mut editor_service = EditorService::new(file_io);
    let mut status_message = String::new();

    if args.len() > 1 {
        editor_service.open_file(&args[1])?;
    }

    let mut stdout = io::stdout();
    loop {
        terminal_ui::draw_editor(&mut stdout, &editor_service.editor_model, &status_message)?;

        if event::poll(Duration::from_millis(500))? {
            if let Event::Key(event) = event::read()? {
                match editor_service.editor_model.mode {
                    EditorMode::Normal => match event.code {
                        KeyCode::Char('i') => {
                            editor_service.set_mode(EditorMode::Insert);
                            status_message = "-- INSERT --".to_string();
                        }
                        KeyCode::Char('a') => {
                            editor_service.editor_model.move_cursor_for_append();
                            editor_service.set_mode(EditorMode::Insert);
                            status_message = "-- INSERT --".to_string();
                        }
                        KeyCode::Char('A') => {
                            editor_service
                                .editor_model
                                .move_cursor_for_append_at_line_end();
                            editor_service.set_mode(EditorMode::Insert);
                            status_message = "-- INSERT --".to_string();
                        }
                        KeyCode::Char(':') => {
                            editor_service.set_mode(EditorMode::Command);
                            status_message = ":".to_string();
                        }
                        KeyCode::Char('h') => {
                            editor_service.move_cursor(KeyCode::Left);
                            status_message.clear();
                        }
                        KeyCode::Char('j') => {
                            editor_service.move_cursor(KeyCode::Down);
                            status_message.clear();
                        }
                        KeyCode::Char('k') => {
                            editor_service.move_cursor(KeyCode::Up);
                            status_message.clear();
                        }
                        KeyCode::Char('l') => {
                            editor_service.move_cursor(KeyCode::Right);
                            status_message.clear();
                        }
                        KeyCode::Char('q') => break,
                        _ => {}
                    },
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
                            editor_service.editor_model.clear_command_buffer();
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
                            editor_service.editor_model.clear_command_buffer();
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
                }
            }
        }
    }
    Ok(())
}
