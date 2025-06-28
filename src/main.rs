mod application;
mod domain;
mod infrastructure;

use application::editor_service::EditorService;
use infrastructure::file_io::LocalFileIO;
use infrastructure::terminal_ui;

use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
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
                match (event.code, event.modifiers) {
                    (KeyCode::Char('q'), KeyModifiers::CONTROL) => break,
                    (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
                        if editor_service.save_file().is_ok() {
                            status_message = "File saved successfully!".to_string();
                        } else {
                            status_message = "Error saving file!".to_string();
                        }
                    }
                    (KeyCode::Char(c), _) => {
                        editor_service.insert_char(c);
                        status_message.clear();
                    }
                    (KeyCode::Backspace, _) => {
                        editor_service.delete_char();
                        status_message.clear();
                    }
                    (KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right, _) => {
                        editor_service.move_cursor(event.code);
                        status_message.clear();
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}
