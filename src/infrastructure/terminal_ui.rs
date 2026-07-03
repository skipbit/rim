use crate::domain::editor_model::{EditorMode, EditorModel};
use crate::infrastructure::syntax_worker::{color_for, HlSpan};
use crossterm::{
    cursor, execute, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
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

/// The highlight style covering document byte `byte`, if any. `spans` must be
/// sorted by `start_byte` and non-overlapping (as produced by the highlighter
/// event walk in `syntax_worker`).
fn style_at(spans: &[HlSpan], byte: usize) -> Option<usize> {
    let i = spans.partition_point(|s| s.start_byte <= byte);
    let s = spans.get(i.checked_sub(1)?)?;
    (byte < s.end_byte).then_some(s.style)
}

/// Draw the visible slice of `line` — same horizontal window rule as
/// [`visible_slice`] — but split into runs coloured by their highlight span.
/// `line_start_byte` is the byte offset of the line within the document, so
/// each grapheme's absolute byte can be looked up in `spans`.
fn draw_line_highlighted(
    stdout: &mut io::Stdout,
    line: &str,
    line_start_byte: usize,
    spans: &[HlSpan],
    col_offset: usize,
    width: usize,
) -> io::Result<()> {
    let mut col = 0usize;
    let mut byte_in_line = 0usize;
    let mut current: Option<Color> = None;
    for g in line.graphemes(true) {
        let w = UnicodeWidthStr::width(g);
        let g_start = byte_in_line;
        byte_in_line += g.len();
        if col + w <= col_offset {
            col += w;
            continue;
        }
        if col + w > col_offset + width {
            break;
        }
        let color = style_at(spans, line_start_byte + g_start)
            .map(color_for)
            .unwrap_or(Color::Reset);
        if current != Some(color) {
            queue!(stdout, SetForegroundColor(color))?;
            current = Some(color);
        }
        stdout.write_all(g.as_bytes())?;
        col += w;
    }
    if current.is_some() {
        queue!(stdout, SetForegroundColor(Color::Reset))?;
    }
    Ok(())
}

pub fn draw_editor(
    stdout: &mut io::Stdout,
    editor: &EditorModel,
    status_message: &str,
    spans: &[HlSpan],
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
        if spans.is_empty() {
            // Fast path: no highlights available (worker not ready / disabled).
            let visible = visible_slice(&line, editor.col_offset, cols as usize);
            stdout.write_all(visible.as_bytes())?;
        } else {
            let line_start_byte = editor.buffer.line_to_byte(line_idx);
            draw_line_highlighted(
                stdout,
                &line,
                line_start_byte,
                spans,
                editor.col_offset,
                cols as usize,
            )?;
        }
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
    use super::{style_at, visible_slice, HlSpan};

    fn span(start: usize, end: usize, style: usize) -> HlSpan {
        HlSpan {
            start_byte: start,
            end_byte: end,
            style,
        }
    }

    #[test]
    fn style_at_finds_covering_span() {
        // Two disjoint spans: bytes 0..2 style 10, bytes 5..8 style 16.
        let spans = [span(0, 2, 10), span(5, 8, 16)];
        assert_eq!(style_at(&spans, 0), Some(10));
        assert_eq!(style_at(&spans, 1), Some(10));
        assert_eq!(style_at(&spans, 2), None); // end is exclusive
        assert_eq!(style_at(&spans, 4), None); // gap between spans
        assert_eq!(style_at(&spans, 5), Some(16));
        assert_eq!(style_at(&spans, 7), Some(16));
        assert_eq!(style_at(&spans, 100), None); // past the end
    }

    #[test]
    fn style_at_empty_is_none() {
        assert_eq!(style_at(&[], 0), None);
    }

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
