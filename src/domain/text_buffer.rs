use ropey::Rope;
use std::ops::Range;

/// A text buffer backed by a [`ropey::Rope`].
///
/// # Coordinate model
///
/// All positions exposed by this type are **char** offsets, never byte offsets
/// and never display columns. `char_idx` in the primitive edit methods is a
/// char offset into the whole buffer; `(y, x)` pairs elsewhere are a logical
/// line index plus a char offset within that line (newline excluded).
///
/// # Trailing-newline invariant
///
/// The rope is either empty, or **every** logical line — including the last —
/// is terminated by `'\n'`. Equivalently the rope content equals
/// `if lines.is_empty() { String::new() } else { lines.join("\n") + "\n" }`.
///
/// This is what lets the editor distinguish an *empty document* (0 lines) from
/// a document holding a *single empty line* (1 line): the former is the empty
/// rope, the latter is `"\n"`. `EditorModel` treats those two cases
/// differently, so the distinction must survive round-trips through the buffer.
pub struct TextBuffer {
    rope: Rope,
}

impl Default for TextBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl TextBuffer {
    pub fn new() -> Self {
        Self { rope: Rope::new() }
    }

    // ---- whole document -----------------------------------------------------

    /// Replace the whole buffer from a string, matching `str::lines()` splitting
    /// (so a trailing newline does not produce a final empty line), then store
    /// it under the trailing-newline invariant.
    pub fn set_content(&mut self, content: &str) {
        let lines: Vec<&str> = content.lines().collect();
        self.set_lines_str(&lines);
    }

    /// Reproduce the classic `lines.join("\n")` output: the raw rope content
    /// with the single invariant-terminating `'\n'` stripped.
    pub fn get_content(&self) -> String {
        let s = self.rope.to_string();
        match s.strip_suffix('\n') {
            Some(stripped) => stripped.to_string(),
            None => s,
        }
    }

    // ---- line queries (newline excluded) ------------------------------------

    /// Number of logical lines. `0` for the empty document.
    pub fn line_count(&self) -> usize {
        if self.rope.len_chars() == 0 {
            0
        } else {
            // Under the invariant the rope ends in '\n', so ropey reports one
            // extra (empty) trailing line that is not a logical line.
            self.rope.len_lines() - 1
        }
    }

    #[allow(dead_code)] // part of the frozen API; used by later milestones
    pub fn is_empty(&self) -> bool {
        self.rope.len_chars() == 0
    }

    #[allow(dead_code)] // part of the frozen API; used by later milestones
    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    /// Text of logical line `y`, without its terminating `'\n'`.
    pub fn line_text(&self, y: usize) -> String {
        let mut s = self.rope.line(y).to_string();
        if s.ends_with('\n') {
            s.pop();
        }
        s
    }

    /// Char length of logical line `y`, newline excluded.
    pub fn line_char_len(&self, y: usize) -> usize {
        // `rope.line(y)` includes the terminating '\n' under the invariant.
        self.rope.line(y).len_chars().saturating_sub(1)
    }

    // ---- coordinate conversion ----------------------------------------------

    /// Char index of the first char of logical line `y`.
    pub fn line_to_char(&self, y: usize) -> usize {
        self.rope.line_to_char(y)
    }

    /// Logical line containing char index `idx`.
    pub fn char_to_line(&self, idx: usize) -> usize {
        self.rope.char_to_line(idx)
    }

    /// The raw buffer contents including the invariant-terminating newlines
    /// (one `'\n'` per logical line). Motions scan this char sequence with
    /// newlines acting as blanks.
    pub fn raw_content(&self) -> String {
        self.rope.to_string()
    }

    /// A cheap O(1) copy-on-write clone of the underlying rope. Used to hand a
    /// consistent snapshot of the current text to the background
    /// syntax-highlighting worker without materialising a `String` on the edit
    /// path (the worker turns it into bytes off-thread).
    #[allow(dead_code)] // consumed by the background syntax worker in a later milestone
    pub fn snapshot(&self) -> Rope {
        self.rope.clone()
    }

    // ---- byte coordinates (for tree-sitter / highlight span mapping) --------
    //
    // The cursor layer is char-indexed, but tree-sitter reports byte offsets.
    // These delegate straight to ropey so highlight spans (byte ranges) can be
    // mapped back onto logical lines and columns at render time.

    /// Total length of the buffer in bytes.
    #[allow(dead_code)] // wired up in the highlight-rendering milestone
    pub fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }

    /// Byte index of the first byte of logical line `y`. Used by the renderer
    /// to map highlight spans (byte ranges) onto visible lines.
    pub fn line_to_byte(&self, y: usize) -> usize {
        self.rope.line_to_byte(y)
    }

    /// Logical line containing byte index `byte_idx`.
    #[allow(dead_code)] // wired up in the highlight-rendering milestone
    pub fn byte_to_line(&self, byte_idx: usize) -> usize {
        self.rope.byte_to_line(byte_idx)
    }

    /// Byte index of char index `char_idx`.
    #[allow(dead_code)] // wired up in the highlight-rendering milestone
    pub fn char_to_byte(&self, char_idx: usize) -> usize {
        self.rope.char_to_byte(char_idx)
    }

    /// Whole-buffer char index of cursor position `(y, x)`.
    pub fn cursor_to_char(&self, y: usize, x: usize) -> usize {
        self.rope.line_to_char(y) + x
    }

    #[allow(dead_code)] // part of the frozen API; used by later milestones
    pub fn char_at(&self, char_idx: usize) -> char {
        self.rope.char(char_idx)
    }

    #[allow(dead_code)] // part of the frozen API; used by later milestones
    pub fn slice_text(&self, range: Range<usize>) -> String {
        self.rope.slice(range).to_string()
    }

    // ---- primitive char-indexed edits ---------------------------------------
    //
    // These are intentionally raw: the caller is responsible for preserving the
    // trailing-newline invariant (see `EditorModel`). They are the same
    // primitives used by `Change::apply`/`invert` for undo/redo.

    pub fn insert(&mut self, char_idx: usize, text: &str) {
        self.rope.insert(char_idx, text);
    }

    #[allow(dead_code)] // part of the frozen API; used by unit tests
    pub fn insert_char(&mut self, char_idx: usize, c: char) {
        self.rope.insert_char(char_idx, c);
    }

    pub fn remove(&mut self, range: Range<usize>) {
        self.rope.remove(range);
    }

    // ---- test-setup / rendering helpers -------------------------------------

    /// Append one logical line at the end (1:1 replacement for `lines.push`).
    #[allow(dead_code)] // used by unit tests / later milestones
    pub fn push_line(&mut self, text: &str) {
        let idx = self.rope.len_chars();
        self.rope.insert(idx, text);
        self.rope.insert_char(self.rope.len_chars(), '\n');
    }

    /// Snapshot the logical lines as a `Vec<String>` (assertion/undo helper).
    #[allow(dead_code)] // used by unit tests / later milestones
    pub fn to_lines(&self) -> Vec<String> {
        (0..self.line_count()).map(|y| self.line_text(y)).collect()
    }

    /// Rebuild the buffer from already-split logical lines, preserving the
    /// invariant exactly (used to restore snapshots losslessly, unlike
    /// `set_content` which re-splits and would collapse `[""]` into `[]`).
    #[allow(dead_code)] // part of the frozen API; used by unit tests
    pub fn set_lines(&mut self, lines: &[String]) {
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        self.set_lines_str(&refs);
    }

    fn set_lines_str(&mut self, lines: &[&str]) {
        let text = if lines.is_empty() {
            String::new()
        } else {
            let mut s = lines.join("\n");
            s.push('\n');
            s
        };
        self.rope = Rope::from_str(&text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_document_has_zero_lines() {
        let buf = TextBuffer::new();
        assert_eq!(buf.line_count(), 0);
        assert!(buf.is_empty());
        assert_eq!(buf.to_lines(), Vec::<String>::new());
        assert_eq!(buf.get_content(), "");
    }

    #[test]
    fn single_empty_line_is_distinct_from_empty_document() {
        // [""] must NOT collapse into [].
        let mut buf = TextBuffer::new();
        buf.set_lines(&["".to_string()]);
        assert_eq!(buf.line_count(), 1);
        assert_eq!(buf.line_text(0), "");
        assert_eq!(buf.line_char_len(0), 0);
        assert_eq!(buf.to_lines(), vec!["".to_string()]);
    }

    #[test]
    fn set_content_matches_lines_semantics() {
        let mut buf = TextBuffer::new();
        buf.set_content("line1\nline2");
        assert_eq!(buf.line_count(), 2);
        assert_eq!(buf.line_text(0), "line1");
        assert_eq!(buf.line_text(1), "line2");
        assert_eq!(buf.get_content(), "line1\nline2");
    }

    #[test]
    fn set_content_trailing_newline_does_not_add_empty_line() {
        let mut buf = TextBuffer::new();
        buf.set_content("a\n");
        assert_eq!(buf.line_count(), 1);
        assert_eq!(buf.get_content(), "a");
    }

    #[test]
    fn set_content_empty_yields_empty_document() {
        let mut buf = TextBuffer::new();
        buf.set_content("");
        assert_eq!(buf.line_count(), 0);
    }

    #[test]
    fn trailing_empty_line_preserved() {
        let mut buf = TextBuffer::new();
        buf.set_lines(&["a".to_string(), "".to_string()]);
        assert_eq!(buf.line_count(), 2);
        assert_eq!(buf.line_text(0), "a");
        assert_eq!(buf.line_text(1), "");
        assert_eq!(buf.to_lines(), vec!["a".to_string(), "".to_string()]);
    }

    #[test]
    fn push_line_appends() {
        let mut buf = TextBuffer::new();
        buf.push_line("a");
        buf.push_line("b");
        assert_eq!(buf.line_count(), 2);
        assert_eq!(buf.to_lines(), vec!["a".to_string(), "b".to_string()]);
        assert_eq!(buf.get_content(), "a\nb");
    }

    #[test]
    fn coordinate_conversion() {
        let mut buf = TextBuffer::new();
        buf.set_content("ab\ncd");
        assert_eq!(buf.line_to_char(0), 0);
        assert_eq!(buf.line_to_char(1), 3); // 'a','b','\n'
        assert_eq!(buf.cursor_to_char(1, 1), 4);
        assert_eq!(buf.char_at(4), 'd');
    }

    #[test]
    fn set_lines_roundtrips_through_to_lines() {
        for lines in [
            vec![],
            vec!["".to_string()],
            vec!["a".to_string()],
            vec!["a".to_string(), "b".to_string()],
            vec!["a".to_string(), "".to_string()],
            vec!["".to_string(), "b".to_string()],
        ] {
            let mut buf = TextBuffer::new();
            buf.set_lines(&lines);
            assert_eq!(buf.to_lines(), lines);
        }
    }

    // ---- multibyte / Unicode ------------------------------------------------

    #[test]
    fn multibyte_line_char_lengths_are_char_counts() {
        let mut buf = TextBuffer::new();
        buf.set_content("あいう\nx");
        assert_eq!(buf.line_char_len(0), 3); // 3 chars, not 9 bytes
        assert_eq!(buf.line_char_len(1), 1);
    }

    #[test]
    fn insert_char_into_multibyte_line_no_panic() {
        let mut buf = TextBuffer::new();
        buf.set_content("あい");
        // insert 'x' between the two Japanese chars (char index 1 within line 0)
        let idx = buf.cursor_to_char(0, 1);
        buf.insert_char(idx, 'x');
        assert_eq!(buf.line_text(0), "あxい");
        assert_eq!(buf.line_char_len(0), 3);
    }

    #[test]
    fn remove_multibyte_char() {
        let mut buf = TextBuffer::new();
        buf.set_content("あいう");
        let idx = buf.cursor_to_char(0, 1);
        buf.remove(idx..idx + 1);
        assert_eq!(buf.line_text(0), "あう");
    }

    #[test]
    fn byte_coordinates_multibyte() {
        let mut buf = TextBuffer::new();
        buf.set_content("あい\nx");
        // Stored under the trailing-newline invariant as "あい\nx\n":
        // あ(3) い(3) \n(1) x(1) \n(1) = 9 bytes. "あい\n" = 7, so line 1 @ byte 7.
        assert_eq!(buf.len_bytes(), 9);
        assert_eq!(buf.line_to_byte(0), 0);
        assert_eq!(buf.line_to_byte(1), 7);
        assert_eq!(buf.byte_to_line(7), 1);
        // char 1 is the second 'あい' char -> byte 3.
        assert_eq!(buf.char_to_byte(1), 3);
        // char 3 is 'x' on line 1 -> byte 7.
        assert_eq!(buf.char_to_byte(3), 7);
    }

    #[test]
    fn snapshot_is_independent_copy() {
        let mut buf = TextBuffer::new();
        buf.set_content("abc");
        let snap = buf.snapshot();
        buf.insert(1, "XY");
        // The snapshot reflects the text at snapshot time, not later edits.
        assert_eq!(snap.to_string(), "abc\n");
        assert_eq!(buf.line_text(0), "aXYbc");
    }

    #[test]
    fn emoji_char_indexing() {
        let mut buf = TextBuffer::new();
        buf.set_content("a😀b");
        // '😀' is a single char (one Unicode scalar value) here.
        assert_eq!(buf.line_char_len(0), 3);
        assert_eq!(buf.char_at(buf.cursor_to_char(0, 1)), '😀');
    }
}
