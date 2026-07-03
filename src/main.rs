mod application;
mod domain;
mod infrastructure;

use application::editor_service::{EditorService, HandleCommandResult};
use application::normal_mode::{NormalMode, NormalResult};
use domain::editor_model::EditorMode;
use infrastructure::file_io::LocalFileIO;
use infrastructure::terminal_ui;

use crossterm::{
    event::{Event, EventStream, KeyCode},
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, size, EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use futures::StreamExt;
use std::env;
use std::io;

#[tokio::main(flavor = "current_thread")]
async fn main() -> io::Result<()> {
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    enable_raw_mode()?;

    // Restore the terminal on panic before the default hook prints, so a panic
    // mid-run cannot leave the terminal in raw/alternate-screen mode. The async
    // loop has more early-exit paths than the old blocking one, so this matters.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        default_hook(info);
    }));

    let result = run().await;

    disable_raw_mode()?;
    execute!(stdout, LeaveAlternateScreen)?;
    if let Err(e) = result {
        eprintln!("Error: {:?}", e);
    }
    Ok(())
}

async fn run() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    let file_io = LocalFileIO;
    let mut editor_service = EditorService::new(file_io);
    let mut status_message = String::new();
    let mut normal_mode = NormalMode::new();

    if args.len() > 1 {
        editor_service.open_file(&args[1])?;
    }

    // Async terminal input. crossterm's EventStream (the "event-stream" feature)
    // yields events as a futures Stream; awaiting it never blocks the runtime,
    // leaving room for background work in later sprints.
    let mut reader = EventStream::new();

    let mut stdout = io::stdout();
    loop {
        // Keep the cursor within the visible text area before rendering.
        let (cols, rows) = size()?;
        let text_height = (rows as usize).saturating_sub(2);
        editor_service
            .editor_model
            .scroll_into_view(text_height, cols as usize);

        terminal_ui::draw_editor(&mut stdout, &editor_service.editor_model, &status_message)?;

        // Await the next terminal event. `None` => the stream closed (input
        // gone) so we exit; a transient read error is ignored and we re-render.
        let event = match reader.next().await {
            Some(Ok(event)) => event,
            Some(Err(_)) => continue,
            None => break,
        };

        // Non-key events (resize, mouse, paste) fall through and simply trigger
        // a re-render on the next iteration, which re-queries the terminal size.
        if let Event::Key(event) = event {
            match editor_service.editor_model.mode {
                EditorMode::Normal => {
                    match normal_mode.feed(&mut editor_service, &event, &mut status_message) {
                        NormalResult::Quit => break,
                        NormalResult::Continue => {}
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
                        status_message = format!(":{}", editor_service.editor_model.command_buffer);
                    }
                    KeyCode::Backspace => {
                        editor_service.pop_command_char();
                        status_message = format!(":{}", editor_service.editor_model.command_buffer);
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
                        status_message = format!("/{}", editor_service.editor_model.command_buffer);
                    }
                    KeyCode::Backspace => {
                        editor_service.pop_command_char();
                        status_message = format!("/{}", editor_service.editor_model.command_buffer);
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
    Ok(())
}
