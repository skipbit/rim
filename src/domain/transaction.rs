use crate::domain::text_buffer::TextBuffer;

/// A single contiguous edit region: at char index `pos`, `removed` was replaced
/// by `inserted`. A pure insert has `removed` empty; a pure delete has
/// `inserted` empty. Storing both makes the edit trivially invertible.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub pos: usize,
    pub removed: String,
    pub inserted: String,
}

impl Change {
    /// Apply this change to the buffer (remove then insert at `pos`).
    pub fn apply(&self, buf: &mut TextBuffer) {
        if !self.removed.is_empty() {
            buf.remove(self.pos..self.pos + self.removed.chars().count());
        }
        if !self.inserted.is_empty() {
            buf.insert(self.pos, &self.inserted);
        }
    }

    /// The inverse change: swapping `removed` and `inserted` undoes `apply`.
    pub fn invert(&self) -> Change {
        Change {
            pos: self.pos,
            removed: self.inserted.clone(),
            inserted: self.removed.clone(),
        }
    }
}

/// One undo step: a change plus the cursor positions to restore on undo
/// (`cursor_before`) and redo (`cursor_after`). A single `Change` is enough for
/// every current editor operation (even line merges are one `'\n'` removal);
/// this can grow to `Vec<Change>` for multi-region edits without breaking the
/// undo/redo protocol.
pub struct Transaction {
    pub change: Change,
    pub cursor_before: (usize, usize),
    pub cursor_after: (usize, usize),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_then_invert_roundtrips() {
        let mut buf = TextBuffer::new();
        buf.set_content("abc");
        let change = Change {
            pos: 1,
            removed: String::new(),
            inserted: "XY".to_string(),
        };
        change.apply(&mut buf);
        assert_eq!(buf.line_text(0), "aXYbc");
        change.invert().apply(&mut buf);
        assert_eq!(buf.line_text(0), "abc");
    }

    #[test]
    fn delete_change_inverts_to_insert() {
        let mut buf = TextBuffer::new();
        buf.set_content("hello");
        let change = Change {
            pos: 1,
            removed: "ell".to_string(),
            inserted: String::new(),
        };
        change.apply(&mut buf);
        assert_eq!(buf.line_text(0), "ho");
        change.invert().apply(&mut buf);
        assert_eq!(buf.line_text(0), "hello");
    }
}
