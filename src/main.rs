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
    let mut d_pressed = false;
    let mut _y_pressed = false;

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
                            editor_service.set_mode(EditorMode::Insert);
                            status_message.clear();
                            d_pressed = false;
                            _y_pressed = false;
                        }
                        KeyCode::Char('A') => {
                            editor_service.set_mode(EditorMode::Insert);
                            status_message.clear();
                            d_pressed = false;
                            _y_pressed = false;
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
                        KeyCode::Char('o') => {
                            editor_service.editor_model.insert_line_below();
                            editor_service.set_mode(EditorMode::Insert);
                            status_message = "-- INSERT --".to_string();
                        }
                        KeyCode::Char('O') => {
                            editor_service.editor_model.insert_line_above();
                            editor_service.set_mode(EditorMode::Insert);
                            status_message = "-- INSERT --".to_string();
                        }
                        KeyCode::Char('x') => {
                            editor_service.editor_model.delete_char_under_cursor();
                            status_message.clear();
                            d_pressed = false;
                        }
                        KeyCode::Char('d') => {
                            if d_pressed {
                                editor_service.editor_model.delete_current_line();
                                status_message.clear();
                                d_pressed = false;
                            } else {
                                d_pressed = true;
                                status_message = "d".to_string();
                            }
                        }
                        KeyCode::Char('y') => {
                            if d_pressed {
                                editor_service.yank_current_line();
                                status_message = "Yanked current line.".to_string();
                            }
                            d_pressed = false;
                            _y_pressed = false;
                        }
                        KeyCode::Char('p') => {
                            editor_service.editor_model.put_line_below();
                            status_message.clear();
                            d_pressed = false;
                            _y_pressed = false;
                        }
                        KeyCode::Char('u') => {
                            editor_service.editor_model.undo();
                            status_message = "Undo".to_string();
                            d_pressed = false;
                            _y_pressed = false;
                        }
                        KeyCode::Char('r') => {
                            if event.modifiers.contains(event::KeyModifiers::CONTROL) {
                                editor_service.editor_model.redo();
                                status_message = "Redo".to_string();
                            }
                            d_pressed = false;
                            _y_pressed = false;
                        }
                        KeyCode::Char('.') => {
                            editor_service.editor_model.repeat_last_change();
                            status_message.clear();
                            d_pressed = false;
                            _y_pressed = false;
                        }
                        KeyCode::Char('q') => break,
                        KeyCode::Char('/') => {
                            editor_service.set_mode(EditorMode::Search);
                            editor_service.clear_command_buffer();
                            status_message = "/".to_string();
                        }
                        KeyCode::Char('n') => {
                            editor_service.find_next();
                        }
                        KeyCode::Char('N') => {
                            editor_service.find_previous();
                        }
                        _ => {
                            d_pressed = false;
                            _y_pressed = false;
                        }
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
