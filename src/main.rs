mod application;
mod domain;
mod infrastructure;

use application::editor_service::{EditorService, HandleCommandResult};
use application::lsp::{ApplyOutcome, Lsp, LspRequest};
use application::normal_mode::{NormalMode, NormalResult};
use application::syntax::Syntax;
use domain::editor_model::EditorMode;
use infrastructure::file_io::LocalFileIO;
use infrastructure::terminal_ui;

use crossterm::{
    event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers},
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

/// Plain insert-mode key handling (no completion popup active).
fn insert_default_key(
    event: KeyEvent,
    editor_service: &mut EditorService<LocalFileIO>,
    status_message: &mut String,
) {
    match event.code {
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
    }
}

/// Dispatch one key event to the editor. Returns `true` if the editor should
/// quit. Pure synchronous CPU work — it never blocks the async runtime.
fn handle_key(
    event: KeyEvent,
    editor_service: &mut EditorService<LocalFileIO>,
    normal_mode: &mut NormalMode,
    status_message: &mut String,
    lsp: &mut Lsp,
) -> bool {
    let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
    match editor_service.editor_model.mode {
        EditorMode::Normal => match normal_mode.feed(editor_service, &event, status_message) {
            NormalResult::Quit => return true,
            NormalResult::Continue => {}
        },
        EditorMode::Insert if lsp.completion_active() => match event.code {
            KeyCode::Esc => lsp.close_completion(),
            KeyCode::Char('n') if ctrl => lsp.completion_move(1),
            KeyCode::Char('p') if ctrl => lsp.completion_move(-1),
            KeyCode::Down => lsp.completion_move(1),
            KeyCode::Up => lsp.completion_move(-1),
            KeyCode::Enter | KeyCode::Tab => {
                lsp.completion_accept(editor_service);
            }
            KeyCode::Char(c) if !ctrl && (c.is_alphanumeric() || c == '_') => {
                editor_service.insert_char(c);
                lsp.completion_refilter(&editor_service.editor_model);
            }
            KeyCode::Backspace => {
                editor_service.delete_char();
                lsp.completion_refilter(&editor_service.editor_model);
            }
            // Any other key dismisses the popup and is handled normally.
            _ => {
                lsp.close_completion();
                insert_default_key(event, editor_service, status_message);
            }
        },
        EditorMode::Insert => match event.code {
            // Trigger completion: Ctrl-n or Ctrl-Space.
            KeyCode::Char('n') if ctrl => {
                let (y, x) = (
                    editor_service.editor_model.cursor_y,
                    editor_service.editor_model.cursor_x,
                );
                editor_service.request_lsp(LspRequest::Completion { y, x });
            }
            KeyCode::Char(' ') if ctrl => {
                let (y, x) = (
                    editor_service.editor_model.cursor_y,
                    editor_service.editor_model.cursor_x,
                );
                editor_service.request_lsp(LspRequest::Completion { y, x });
            }
            _ => insert_default_key(event, editor_service, status_message),
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

    // Background LSP client. Server->client messages (and our request results)
    // arrive on `lsp_rx`, a fourth branch of the select! below. The language
    // server is spawned lazily on the first `.rs` file open.
    let (lsp_tx, mut lsp_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut lsp = Lsp::new(lsp_tx);
    lsp.on_open(&editor_service.editor_model);

    // Async terminal input via crossterm's EventStream (the "event-stream"
    // feature); awaiting it never blocks the runtime.
    let mut reader = EventStream::new();
    // When set, the instant at which a debounced re-highlight should fire.
    let mut deadline: Option<Instant> = None;

    let mut stdout = io::stdout();
    loop {
        // Keep the cursor within the visible text area before rendering. The
        // line-number gutter narrows the horizontal text area.
        let (cols, rows) = size()?;
        let text_height = (rows as usize).saturating_sub(2);
        let gutter = terminal_ui::gutter_width(editor_service.editor_model.buffer.line_count());
        let text_width = (cols as usize).saturating_sub(gutter);
        editor_service
            .editor_model
            .scroll_into_view(text_height, text_width);

        // Project LSP diagnostics onto the buffer for rendering, and the
        // message of the diagnostic under the cursor (or a count summary) for
        // the idle status line.
        let model = &editor_service.editor_model;
        let diagnostics = lsp.line_diagnostics(&model.buffer);
        let diagnostic_msg = lsp
            .diagnostic_at(&model.buffer, model.cursor_y, model.cursor_x)
            .or_else(|| lsp.progress_message())
            .unwrap_or_else(|| lsp.diagnostic_summary());
        let completion = lsp.completion_view();

        terminal_ui::draw_editor(
            &mut stdout,
            &editor_service.editor_model,
            &status_message,
            syntax.spans(),
            &diagnostics,
            &diagnostic_msg,
            lsp.hover_lines(),
            completion
                .as_ref()
                .map(|(items, sel)| (items.as_slice(), *sel)),
        )?;

        // A far-future default keeps the timer branch harmless while disabled by
        // its guard; `tick` is a copied Instant so the future borrows no state.
        let tick = deadline.unwrap_or_else(|| Instant::now() + Duration::from_secs(3600));
        tokio::select! {
            maybe_event = reader.next() => {
                match maybe_event {
                    Some(Ok(Event::Key(key))) => {
                        // Any keypress dismisses the hover popup (completion has
                        // its own lifecycle).
                        lsp.clear_transient();
                        let before = editor_service.editor_model.edit_revision();
                        let before_path = editor_service.editor_model.filepath.clone();
                        if handle_key(key, &mut editor_service, &mut normal_mode, &mut status_message, &mut lsp) {
                            break;
                        }
                        let after = editor_service.editor_model.edit_revision();
                        let after_path = editor_service.editor_model.filepath.clone();
                        if after_path != before_path {
                            // File switched (:e / cross-file gd): re-open in the
                            // LSP and re-highlight the whole new buffer. Handled
                            // before the didChange path so the `set_content`
                            // revision bump is subsumed into the open, not sent
                            // as a spurious change against the new document.
                            lsp.on_open(&editor_service.editor_model);
                            if editor_service.editor_model.buffer.line_count() > 0 {
                                syntax.request_now(after, editor_service.editor_model.buffer.snapshot());
                            }
                            deadline = None;
                        } else if after != before {
                            // Text changed: (re-)arm the shared edit debounce for
                            // both re-highlight and didChange.
                            syntax.note_change(after, editor_service.editor_model.buffer.snapshot());
                            lsp.note_change();
                            deadline = Some(Instant::now() + HIGHLIGHT_DEBOUNCE);
                        }
                        // Fire any LSP feature request the keypress recorded.
                        if let Some(req) = editor_service.take_pending_lsp() {
                            lsp.dispatch_request(req, &editor_service.editor_model);
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
            Some(event) = lsp_rx.recv() => {
                match lsp.apply(event, &mut editor_service, &mut status_message) {
                    // Cross-file go-to-definition: re-open in the LSP and
                    // re-highlight the whole new buffer.
                    ApplyOutcome::FileSwitched => {
                        lsp.on_open(&editor_service.editor_model);
                        if editor_service.editor_model.buffer.line_count() > 0 {
                            syntax.request_now(
                                editor_service.editor_model.edit_revision(),
                                editor_service.editor_model.buffer.snapshot(),
                            );
                        }
                    }
                    // Format/rename edited the buffer: re-highlight now and arm
                    // the debounced didChange.
                    ApplyOutcome::Edited => {
                        syntax.request_now(
                            editor_service.editor_model.edit_revision(),
                            editor_service.editor_model.buffer.snapshot(),
                        );
                        deadline = Some(Instant::now() + HIGHLIGHT_DEBOUNCE);
                    }
                    ApplyOutcome::Nothing => {}
                }
            }
            _ = tokio::time::sleep_until(tick), if deadline.is_some() => {
                syntax.dispatch();
                lsp.dispatch_change(&editor_service.editor_model);
                deadline = None;
            }
        }
    }

    // Best-effort graceful LSP shutdown (timeout-guarded) before the terminal
    // is restored by `main`.
    lsp.shutdown().await;
    Ok(())
}
