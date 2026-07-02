use crate::domain::editor_model::{EditorMode, EditorModel};
use crossterm::{
    cursor, execute,
    style::{Color, Print, ResetColor, SetBackgroundColor},
    terminal::{size, Clear, ClearType},
};
use std::io::{self, Write};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Return the portion of `line` visible within the horizontal window
/// `[col_offset, col_offset + width)`, measured in display columns. Graphemes
/// straddling either edge are dropped whole (a simplification acceptable for
/// this milestone).
fn visible_slice(line: &str, col_offset: usize, width: usize) -> String {
    let mut out = String::new();
    let mut col = 0usize;
    for g in line.graphemes(true) {
        let w = UnicodeWidthStr::width(g);
        if col + w <= col_offset {
            col += w;
            continue;
        }
        if col + w > col_offset + width {
            break;
        }
        out.push_str(g);
        col += w;
    }
    out
}

pub fn draw_editor(
    stdout: &mut io::Stdout,
    editor: &EditorModel,
    status_message: &str,
) -> io::Result<()> {
    let (cols, rows) = size()?;
    execute!(
        stdout,
        cursor::Hide,
        Clear(ClearType::All),
        cursor::MoveTo(0, 0)
    )?;

    // Draw the visible window of text (rows - 2 rows, leaving two status lines).
    let text_height = (rows as usize).saturating_sub(2);
    for screen_row in 0..text_height {
        let line_idx = editor.row_offset + screen_row;
        if line_idx >= editor.buffer.line_count() {
            break;
        }
        let line = editor.buffer.line_text(line_idx);
        let visible = visible_slice(&line, editor.col_offset, cols as usize);
        stdout.write_all(visible.as_bytes())?;
        stdout.write_all(b"\r\n")?;
    }

    // Draw status bar
    let mode_indicator = match editor.mode {
        EditorMode::Normal => "NORMAL",
        EditorMode::Insert => "INSERT",
        EditorMode::Command => "COMMAND",
        EditorMode::Search => "SEARCH",
    };

    let status_bar = format!(
        " {}:{} | {} lines | {} | {}",
        editor.cursor_y + 1,
        editor.cursor_x + 1,
        editor.buffer.line_count(),
        editor.filepath.as_deref().unwrap_or("[No Name]"),
        mode_indicator
    );

    let status_message_line = if let EditorMode::Command = editor.mode {
        format!(":{}", editor.command_buffer)
    } else {
        format!(" {}", status_message)
    };

    execute!(
        stdout,
        cursor::MoveTo(0, rows - 2),
        SetBackgroundColor(Color::DarkGrey),
        Print(format!("{:<width$}", status_bar, width = cols as usize)),
        ResetColor,
        cursor::MoveTo(0, rows - 1),
        SetBackgroundColor(Color::DarkGrey),
        Print(format!(
            "{:<width$}",
            status_message_line,
            width = cols as usize
        )),
        ResetColor
    )?;

    // Move cursor to its on-screen position, using the display column (so wide
    // and combining characters place the cursor correctly) minus the viewport
    // offsets.
    let screen_x = editor.display_col().saturating_sub(editor.col_offset) as u16;
    let screen_y = (editor.cursor_y.saturating_sub(editor.row_offset)) as u16;
    execute!(stdout, cursor::MoveTo(screen_x, screen_y), cursor::Show)?;

    stdout.flush()
}

#[cfg(test)]
mod tests {
    use super::visible_slice;

    #[test]
    fn ascii_window() {
        assert_eq!(visible_slice("hello", 0, 3), "hel");
        assert_eq!(visible_slice("hello", 2, 3), "llo");
        assert_eq!(visible_slice("hello", 0, 10), "hello");
    }

    #[test]
    fn wide_chars_drop_at_edges() {
        // Each full-width char is 2 columns; a width-3 window fits only the first.
        assert_eq!(visible_slice("あいう", 0, 3), "あ");
        assert_eq!(visible_slice("あいう", 0, 4), "あい");
        // Skipping the first full-width column-pair by offset.
        assert_eq!(visible_slice("あいう", 2, 2), "い");
    }

    #[test]
    fn combining_mark_stays_with_base() {
        assert_eq!(visible_slice("a\u{310}b", 0, 2), "a\u{310}b");
    }
}
