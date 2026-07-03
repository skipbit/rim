mod application;
mod domain;
mod infrastructure;

use application::editor_service::{EditorService, HandleCommandResult};
use application::normal_mode::{NormalMode, NormalResult};
use application::syntax::Syntax;
use domain::editor_model::EditorMode;
use infrastructure::file_io::LocalFileIO;
use infrastructure::terminal_ui;

use crossterm::{
    event::{Event, EventStream, KeyCode, KeyEvent},
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, size, EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use futures::StreamExt;
use std::env;
use std::io;
use tokio::time::{Duration, Instant};

/// How long to wait after the last edit before re-highlighting, so a burst of
/// typing collapses into a single parse.
const HIGHLIGHT_DEBOUNCE: Duration = Duration::from_millis(30);

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

/// Dispatch one key event to the editor. Returns `true` if the editor should
/// quit. Pure synchronous CPU work — it never blocks the async runtime.
fn handle_key(
    event: KeyEvent,
    editor_service: &mut EditorService<LocalFileIO>,
    normal_mode: &mut NormalMode,
    status_message: &mut String,
) -> bool {
    match editor_service.editor_model.mode {
        EditorMode::Normal => match normal_mode.feed(editor_service, &event, status_message) {
            NormalResult::Quit => return true,
            NormalResult::Continue => {}
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
                *status_message = format!(":{}", editor_service.editor_model.command_buffer);
            }
            KeyCode::Backspace => {
                editor_service.pop_command_char();
                *status_message = format!(":{}", editor_service.editor_model.command_buffer);
            }
            KeyCode::Enter => {
                let command = editor_service.editor_model.command_buffer.clone();
                editor_service.clear_command_buffer();
                match editor_service.handle_command(&command) {
                    Ok(HandleCommandResult::Quit) => return true,
                    Ok(HandleCommandResult::Continue) => {
                        *status_message = format!("Command executed: {}", command);
                    }
                    Err(e) => {
                        *status_message = format!("Error: {}", e);
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
                *status_message = format!("/{}", editor_service.editor_model.command_buffer);
            }
            KeyCode::Backspace => {
                editor_service.pop_command_char();
                *status_message = format!("/{}", editor_service.editor_model.command_buffer);
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
    false
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

    // Background syntax highlighting. The worker reports results on `hl_rx`,
    // which is one branch of the select! below.
    let (hl_tx, mut hl_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut syntax = Syntax::spawn(hl_tx);
    if editor_service.editor_model.buffer.line_count() > 0 {
        // Highlight the freshly opened file immediately (no debounce).
        syntax.request_now(
            editor_service.editor_model.edit_revision(),
            editor_service.editor_model.buffer.snapshot(),
        );
    }

    // Async terminal input via crossterm's EventStream (the "event-stream"
    // feature); awaiting it never blocks the runtime.
    let mut reader = EventStream::new();
    // When set, the instant at which a debounced re-highlight should fire.
    let mut deadline: Option<Instant> = None;

    let mut stdout = io::stdout();
    loop {
        // Keep the cursor within the visible text area before rendering.
        let (cols, rows) = size()?;
        let text_height = (rows as usize).saturating_sub(2);
        editor_service
            .editor_model
            .scroll_into_view(text_height, cols as usize);

        // TEMP (Sprint 3): surface highlight state in the status line until the
        // renderer consumes spans (Sprint 4). Confirms the worker is wired.
        let debug_status = format!(
            "{}  [hl:{} rev:{}]",
            status_message,
            syntax.spans().len(),
            syntax.revision()
        );
        terminal_ui::draw_editor(&mut stdout, &editor_service.editor_model, &debug_status)?;

        // A far-future default keeps the timer branch harmless while disabled by
        // its guard; `tick` is a copied Instant so the future borrows no state.
        let tick = deadline.unwrap_or_else(|| Instant::now() + Duration::from_secs(3600));
        tokio::select! {
            maybe_event = reader.next() => {
                match maybe_event {
                    Some(Ok(Event::Key(key))) => {
                        let before = editor_service.editor_model.edit_revision();
                        if handle_key(key, &mut editor_service, &mut normal_mode, &mut status_message) {
                            break;
                        }
                        // If the text changed, (re-)arm the debounce with the newest snapshot.
                        let after = editor_service.editor_model.edit_revision();
                        if after != before {
                            syntax.note_change(after, editor_service.editor_model.buffer.snapshot());
                            deadline = Some(Instant::now() + HIGHLIGHT_DEBOUNCE);
                        }
                    }
                    Some(Ok(_)) => {}   // resize/mouse/paste: just re-render
                    Some(Err(_)) => {}  // transient read error: ignore
                    None => break,      // input stream closed
                }
            }
            Some(highlights) = hl_rx.recv() => {
                syntax.apply(highlights);
            }
            _ = tokio::time::sleep_until(tick), if deadline.is_some() => {
                syntax.dispatch();
                deadline = None;
            }
        }
    }
    Ok(())
}
