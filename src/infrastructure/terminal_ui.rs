use crate::domain::editor_model::EditorModel;
use crossterm::{
    cursor, execute,
    style::{Color, Print, ResetColor, SetBackgroundColor},
    terminal::{size, Clear, ClearType},
};
use std::io::{self, Write};

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

    // Draw text
    for (i, line) in editor.lines.iter().enumerate() {
        if i >= rows as usize - 1 {
            break;
        }
        stdout.write_all(line.as_bytes())?;
        stdout.write_all(b"\r\n")?;
    }

    // Draw status bar
    let status_bar = format!(
        " {}:{} | {} lines | {}",
        editor.cursor_y + 1,
        editor.cursor_x + 1,
        editor.lines.len(),
        editor.filepath.as_deref().unwrap_or("[No Name]")
    );
    let status_message_line = format!(" {}", status_message);

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

    // Move cursor to position
    execute!(
        stdout,
        cursor::MoveTo(editor.cursor_x as u16, editor.cursor_y as u16),
        cursor::Show
    )?;

    stdout.flush()
}
