use crate::domain::editor_model::{EditorMode, EditorModel};
use crate::infrastructure::syntax_worker::{color_for, HlSpan};
use crossterm::{
    cursor, execute, queue,
    style::{
        Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
    },
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

/// Most severe diagnostic covering char column range `[char_start, char_end)`
/// on the line, if any. `line_diags` are `(start_col, end_col, severity)` in
/// char columns.
fn diag_at(
    line_diags: &[(usize, usize, DiagSeverity)],
    char_start: usize,
    char_end: usize,
) -> Option<DiagSeverity> {
    line_diags
        .iter()
        .filter(|(s, e, _)| *s < char_end && char_start < *e)
        .map(|(_, _, sev)| *sev)
        .max()
}

/// Draw the visible slice of `line` — same horizontal window rule as
/// [`visible_slice`] — split into runs coloured by their highlight span and
/// underlined (in the severity colour) where a diagnostic covers them.
/// `line_start_byte` is the byte offset of the line within the document (for
/// `spans` lookup); `line_diags` are this line's diagnostic ranges in char
/// columns.
fn draw_line_highlighted(
    stdout: &mut io::Stdout,
    line: &str,
    line_start_byte: usize,
    spans: &[HlSpan],
    line_diags: &[(usize, usize, DiagSeverity)],
    col_offset: usize,
    width: usize,
) -> io::Result<()> {
    let mut col = 0usize;
    let mut byte_in_line = 0usize;
    let mut char_in_line = 0usize;
    let mut current: Option<Color> = None;
    let mut underlined = false;
    for g in line.graphemes(true) {
        let w = UnicodeWidthStr::width(g);
        let g_start = byte_in_line;
        byte_in_line += g.len();
        let char_start = char_in_line;
        char_in_line += g.chars().count();
        if col + w <= col_offset {
            col += w;
            continue;
        }
        if col + w > col_offset + width {
            break;
        }
        let diag = diag_at(line_diags, char_start, char_in_line);
        // Diagnostic severity colour overrides syntax colour on the offending
        // text; otherwise use the syntax colour.
        let color = match diag {
            Some(sev) => severity_color(sev),
            None => style_at(spans, line_start_byte + g_start)
                .map(color_for)
                .unwrap_or(Color::Reset),
        };
        if current != Some(color) {
            queue!(stdout, SetForegroundColor(color))?;
            current = Some(color);
        }
        let want_underline = diag.is_some();
        if want_underline != underlined {
            queue!(
                stdout,
                SetAttribute(if want_underline {
                    Attribute::Underlined
                } else {
                    Attribute::NoUnderline
                })
            )?;
            underlined = want_underline;
        }
        stdout.write_all(g.as_bytes())?;
        col += w;
    }
    if underlined {
        queue!(stdout, SetAttribute(Attribute::NoUnderline))?;
    }
    if current.is_some() {
        queue!(stdout, SetForegroundColor(Color::Reset))?;
    }
    Ok(())
}

/// Severity of a diagnostic, ordered so `max` selects the most severe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagSeverity {
    Hint,
    Info,
    Warning,
    Error,
}

/// A diagnostic range projected onto a single logical line, in char columns
/// (`start_col..end_col`, end exclusive). Multi-line diagnostics are split into
/// one of these per covered line by the orchestrator.
pub struct LineDiag {
    pub line: usize,
    pub start_col: usize,
    pub end_col: usize,
    pub severity: DiagSeverity,
}

fn severity_color(s: DiagSeverity) -> Color {
    match s {
        DiagSeverity::Error => Color::Red,
        DiagSeverity::Warning => Color::Yellow,
        DiagSeverity::Info => Color::Blue,
        DiagSeverity::Hint => Color::Cyan,
    }
}

fn severity_sign(s: DiagSeverity) -> char {
    match s {
        DiagSeverity::Error => 'E',
        DiagSeverity::Warning => 'W',
        DiagSeverity::Info => 'I',
        DiagSeverity::Hint => 'H',
    }
}

/// Number of digits needed for the largest line number (`line_count`, since
/// numbers are 1-based). At least 1.
fn gutter_digits(line_count: usize) -> usize {
    line_count.max(1).to_string().len()
}

/// Width in columns of the line-number gutter: a 1-column sign slot (for
/// diagnostics), a gap, the right-aligned line number, then a ` │ ` separator.
/// The text area occupies the remaining `cols - gutter_width` columns, and all
/// horizontal cursor/scroll math is offset by this width.
pub fn gutter_width(line_count: usize) -> usize {
    // sign(1) + gap(1) + digits + space(1) + bar(1) + space(1)
    gutter_digits(line_count) + 5
}

/// Draw a bordered popup box holding `lines`, anchored near the cursor at
/// screen `(cursor_row, cursor_col)`. Placed below the cursor when it fits,
/// otherwise above. Content is clipped to the box and the terminal width.
fn draw_popup(
    stdout: &mut io::Stdout,
    lines: &[String],
    cursor_row: u16,
    cursor_col: u16,
    cols: u16,
    text_height: usize,
) -> io::Result<()> {
    if lines.is_empty() || text_height == 0 {
        return Ok(());
    }
    const MAX_LINES: usize = 8;
    let shown = &lines[..lines.len().min(MAX_LINES)];
    let content_w = shown
        .iter()
        .map(|l| UnicodeWidthStr::width(l.as_str()))
        .max()
        .unwrap_or(0);
    let inner_w = content_w.clamp(1, (cols as usize).saturating_sub(2).max(1));
    let box_w = inner_w + 2;
    let box_h = shown.len() + 2;
    // Below the cursor if it fits, else above.
    let cur = cursor_row as usize;
    let top = if cur + 1 + box_h <= text_height {
        cur + 1
    } else {
        cur.saturating_sub(box_h)
    };
    let left = (cursor_col as usize).min((cols as usize).saturating_sub(box_w)) as u16;

    queue!(
        stdout,
        SetBackgroundColor(Color::DarkBlue),
        SetForegroundColor(Color::White),
        cursor::MoveTo(left, top as u16),
    )?;
    stdout.write_all(format!("┌{}┐", "─".repeat(inner_w)).as_bytes())?;
    for (i, line) in shown.iter().enumerate() {
        let vis = visible_slice(line, 0, inner_w);
        let pad = inner_w.saturating_sub(UnicodeWidthStr::width(vis.as_str()));
        queue!(stdout, cursor::MoveTo(left, (top + 1 + i) as u16))?;
        stdout.write_all(format!("│{}{}│", vis, " ".repeat(pad)).as_bytes())?;
    }
    queue!(stdout, cursor::MoveTo(left, (top + 1 + shown.len()) as u16))?;
    stdout.write_all(format!("└{}┘", "─".repeat(inner_w)).as_bytes())?;
    queue!(stdout, ResetColor)?;
    Ok(())
}

/// Draw a completion dropdown of `items` (selected row highlighted) anchored
/// below the cursor at screen `(cursor_row, cursor_col)`.
fn draw_menu(
    stdout: &mut io::Stdout,
    items: &[String],
    selected: usize,
    cursor_row: u16,
    cursor_col: u16,
    cols: u16,
    text_height: usize,
) -> io::Result<()> {
    if items.is_empty() || text_height == 0 {
        return Ok(());
    }
    const MAX_ROWS: usize = 8;
    // Scroll the window so the selection stays visible.
    let start = selected.saturating_sub(MAX_ROWS - 1);
    let shown: Vec<(usize, &String)> = items
        .iter()
        .enumerate()
        .skip(start)
        .take(MAX_ROWS)
        .collect();
    let width = shown
        .iter()
        .map(|(_, l)| UnicodeWidthStr::width(l.as_str()))
        .max()
        .unwrap_or(0)
        .clamp(1, (cols as usize).saturating_sub(1).max(1));
    let cur = cursor_row as usize;
    // Below the cursor if it fits, else above.
    let top = if cur + 1 + shown.len() <= text_height {
        cur + 1
    } else {
        cur.saturating_sub(shown.len())
    };
    let left = (cursor_col as usize).min((cols as usize).saturating_sub(width)) as u16;
    for (row, (idx, label)) in shown.iter().enumerate() {
        let bg = if *idx == selected {
            Color::DarkCyan
        } else {
            Color::DarkGrey
        };
        let vis = visible_slice(label, 0, width);
        let pad = width.saturating_sub(UnicodeWidthStr::width(vis.as_str()));
        queue!(
            stdout,
            cursor::MoveTo(left, (top + row) as u16),
            SetBackgroundColor(bg),
            SetForegroundColor(Color::White),
        )?;
        stdout.write_all(format!("{}{}", vis, " ".repeat(pad)).as_bytes())?;
        queue!(stdout, ResetColor)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn draw_editor(
    stdout: &mut io::Stdout,
    editor: &EditorModel,
    status_message: &str,
    spans: &[HlSpan],
    diagnostics: &[LineDiag],
    diagnostic_msg: &str,
    hover: &[String],
    completion: Option<(&[String], usize)>,
) -> io::Result<()> {
    let (cols, rows) = size()?;
    execute!(
        stdout,
        cursor::Hide,
        Clear(ClearType::All),
        cursor::MoveTo(0, 0)
    )?;

    // Draw the visible window of text (rows - 2 rows, leaving two status lines).
    // A line-number gutter occupies the leftmost `gw` columns; the text area is
    // the remaining `text_width` columns.
    let line_count = editor.buffer.line_count();
    let digits = gutter_digits(line_count);
    let gw = gutter_width(line_count);
    let text_width = (cols as usize).saturating_sub(gw);
    let text_height = (rows as usize).saturating_sub(2);
    for screen_row in 0..text_height {
        let line_idx = editor.row_offset + screen_row;
        if line_idx >= line_count {
            break;
        }
        // This line's diagnostics (char-column ranges) and the most severe one,
        // which drives the gutter sign.
        let line_diags: Vec<(usize, usize, DiagSeverity)> = diagnostics
            .iter()
            .filter(|d| d.line == line_idx)
            .map(|d| (d.start_col, d.end_col, d.severity))
            .collect();
        let sign_sev = line_diags.iter().map(|(_, _, s)| *s).max();

        // Gutter: severity sign slot + gap + right-aligned line number + ` │ `.
        let (sign_char, sign_color) = match sign_sev {
            Some(s) => (severity_sign(s), severity_color(s)),
            None => (' ', Color::DarkGrey),
        };
        queue!(stdout, SetForegroundColor(sign_color))?;
        stdout.write_all(sign_char.to_string().as_bytes())?;
        queue!(stdout, SetForegroundColor(Color::DarkGrey))?;
        stdout.write_all(format!(" {:>digits$} │ ", line_idx + 1).as_bytes())?;
        queue!(stdout, SetForegroundColor(Color::Reset))?;

        let line = editor.buffer.line_text(line_idx);
        if spans.is_empty() && line_diags.is_empty() {
            // Fast path: no highlights and no diagnostics on this line.
            let visible = visible_slice(&line, editor.col_offset, text_width);
            stdout.write_all(visible.as_bytes())?;
        } else {
            let line_start_byte = editor.buffer.line_to_byte(line_idx);
            draw_line_highlighted(
                stdout,
                &line,
                line_start_byte,
                spans,
                &line_diags,
                editor.col_offset,
                text_width,
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
    } else if !status_message.is_empty() {
        format!(" {}", status_message)
    } else {
        // Idle: surface the diagnostic under the cursor (or a summary).
        format!(" {}", diagnostic_msg)
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
    // offsets, shifted right by the gutter width.
    let screen_x = (gw + editor.display_col().saturating_sub(editor.col_offset)) as u16;
    let screen_y = (editor.cursor_y.saturating_sub(editor.row_offset)) as u16;

    // Hover popup / completion menu (drawn over the text, near the cursor).
    draw_popup(stdout, hover, screen_y, screen_x, cols, text_height)?;
    if let Some((items, selected)) = completion {
        draw_menu(
            stdout,
            items,
            selected,
            screen_y,
            screen_x,
            cols,
            text_height,
        )?;
    }

    execute!(stdout, cursor::MoveTo(screen_x, screen_y), cursor::Show)?;

    stdout.flush()
}

#[cfg(test)]
mod tests {
    use super::{gutter_width, style_at, visible_slice, HlSpan};

    #[test]
    fn gutter_width_scales_with_line_count() {
        // sign(1)+gap(1)+digits+space(1)+bar(1)+space(1) = digits + 5.
        assert_eq!(gutter_width(0), 6); // digits=1 ("0".len())
        assert_eq!(gutter_width(1), 6); // digits=1
        assert_eq!(gutter_width(9), 6); // digits=1
        assert_eq!(gutter_width(10), 7); // digits=2
        assert_eq!(gutter_width(999), 8); // digits=3
        assert_eq!(gutter_width(1000), 9); // digits=4
    }

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
