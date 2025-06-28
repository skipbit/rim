use std::io::{self, Write};
use crossterm::{terminal::{Clear, ClearType}, cursor, execute};

pub fn draw_editor_rows(stdout: &mut io::Stdout, lines: &[String], cursor_x: usize, cursor_y: usize) -> io::Result<()> {
    execute!(
        stdout,
        Clear(ClearType::All),
        cursor::MoveTo(0, 0)
    )?;

    for (i, line) in lines.iter().enumerate() {
        stdout.write_all(line.as_bytes())?;
        if i < lines.len() - 1 {
            stdout.write_all(b"\r\n")?;
        }
    }

    execute!(stdout, cursor::MoveTo(cursor_x as u16, cursor_y as u16))?;

    stdout.flush()
}
