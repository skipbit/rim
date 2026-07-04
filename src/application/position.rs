//! Conversion between the editor's char-indexed cursor coordinates and LSP
//! protocol positions.
//!
//! LSP `Position.character` counts UTF-16 code units by default; when the
//! `utf-8` position encoding is negotiated (LSP 3.17) it counts UTF-8 bytes
//! within the line. `rim`'s cursor layer is char-indexed, so every LSP position
//! mapping goes through here to keep the two encodings from ever getting mixed
//! up. The UTF-8 path reuses the buffer's existing char<->byte accessors.

use crate::domain::text_buffer::TextBuffer;
use async_lsp::lsp_types::Position;

/// The position encoding negotiated with the language server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // variants wired up in the LSP milestone
pub enum PositionEncoding {
    /// UTF-8 bytes within the line (LSP 3.17 `utf-8`).
    Utf8,
    /// UTF-16 code units within the line (the LSP default).
    Utf16,
}

/// Convert an editor cursor `(y, x)` (char column) to an LSP [`Position`].
#[allow(dead_code)] // wired up in the LSP milestone
pub fn to_lsp(buffer: &TextBuffer, enc: PositionEncoding, y: usize, x: usize) -> Position {
    let character = match enc {
        PositionEncoding::Utf16 => buffer.line_char_to_utf16(y, x),
        PositionEncoding::Utf8 => {
            let abs = buffer.cursor_to_char(y, x);
            buffer
                .char_to_byte(abs)
                .saturating_sub(buffer.line_to_byte(y))
        }
    };
    Position {
        line: y as u32,
        character: character as u32,
    }
}

/// Convert an LSP [`Position`] to an editor cursor `(y, x)` (char column),
/// clamped to the buffer.
#[allow(dead_code)] // wired up in the LSP milestone
pub fn from_lsp(buffer: &TextBuffer, enc: PositionEncoding, pos: Position) -> (usize, usize) {
    let y = (pos.line as usize).min(buffer.len_lines().saturating_sub(1));
    let x = match enc {
        PositionEncoding::Utf16 => buffer.line_utf16_to_char(y, pos.character as usize),
        PositionEncoding::Utf8 => {
            let whole = (buffer.line_to_byte(y) + pos.character as usize).min(buffer.len_bytes());
            buffer
                .byte_to_char(whole)
                .saturating_sub(buffer.line_to_char(y))
                .min(buffer.line_char_len(y))
        }
    };
    (y, x)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(s: &str) -> TextBuffer {
        let mut b = TextBuffer::new();
        b.set_content(s);
        b
    }

    #[test]
    fn utf16_roundtrip_ascii_cjk_emoji() {
        let b = buf("abc\nあい😀\nx");
        for enc in [PositionEncoding::Utf8, PositionEncoding::Utf16] {
            for (y, x) in [(0, 0), (0, 3), (1, 0), (1, 1), (1, 2), (1, 3), (2, 1)] {
                let pos = to_lsp(&b, enc, y, x);
                assert_eq!(from_lsp(&b, enc, pos), (y, x), "{enc:?} ({y},{x})");
            }
        }
    }

    #[test]
    fn utf16_astral_char_is_two_units_utf8_is_four_bytes() {
        let b = buf("😀x");
        // '😀' = 2 UTF-16 code units, 4 UTF-8 bytes.
        assert_eq!(to_lsp(&b, PositionEncoding::Utf16, 0, 1).character, 2);
        assert_eq!(to_lsp(&b, PositionEncoding::Utf8, 0, 1).character, 4);
    }

    #[test]
    fn from_lsp_clamps_out_of_range() {
        let b = buf("ab\ncd");
        // Line beyond the buffer clamps to the last (phantom) line.
        let (y, _) = from_lsp(&b, PositionEncoding::Utf16, Position::new(99, 0));
        assert!(y < b.len_lines());
        // Character beyond the line clamps to the line length.
        assert_eq!(
            from_lsp(&b, PositionEncoding::Utf16, Position::new(0, 99)),
            (0, 2)
        );
        assert_eq!(
            from_lsp(&b, PositionEncoding::Utf8, Position::new(0, 99)),
            (0, 2)
        );
    }
}
