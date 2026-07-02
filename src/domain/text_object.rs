use crate::domain::text_buffer::TextBuffer;

/// A text object selectable with `i`/`a` after an operator (e.g. `diw`, `ci"`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextObject {
    /// `iw` / `aw` (or WORD with `big`).
    Word { big: bool },
    /// `i"` / `a"` etc. — a quote-delimited span on the current line.
    Quoted(char),
    /// `i(` / `a(` etc. — a bracket pair (may span lines).
    Pair(char, char),
}

fn class_word(c: char, big: bool) -> u8 {
    if c.is_whitespace() {
        0
    } else if big || c.is_alphanumeric() || c == '_' {
        1
    } else {
        2
    }
}

/// Resolve a text object to a half-open whole-buffer char range `[start, end)`.
/// Returns `None` when there is nothing to select (e.g. no surrounding quotes).
pub fn range(
    buf: &TextBuffer,
    y: usize,
    x: usize,
    obj: TextObject,
    inner: bool,
) -> Option<(usize, usize)> {
    match obj {
        TextObject::Word { big } => word_object(buf, y, x, big, inner),
        TextObject::Quoted(q) => quoted_object(buf, y, x, q, inner),
        TextObject::Pair(open, close) => pair_object(buf, y, x, open, close, inner),
    }
}

fn word_object(
    buf: &TextBuffer,
    y: usize,
    x: usize,
    big: bool,
    inner: bool,
) -> Option<(usize, usize)> {
    let line: Vec<char> = buf.line_text(y).chars().collect();
    if line.is_empty() {
        return None;
    }
    let x = x.min(line.len() - 1);
    let cls = class_word(line[x], big);
    let mut start = x;
    while start > 0 && class_word(line[start - 1], big) == cls {
        start -= 1;
    }
    let mut end = x; // inclusive index of last char in the run
    while end + 1 < line.len() && class_word(line[end + 1], big) == cls {
        end += 1;
    }
    if !inner {
        // `aw`: include trailing whitespace, or leading whitespace if none.
        let after = end + 1;
        if after < line.len() && class_word(line[after], big) == 0 {
            let mut e = after;
            while e + 1 < line.len() && class_word(line[e + 1], big) == 0 {
                e += 1;
            }
            end = e;
        } else {
            while start > 0 && class_word(line[start - 1], big) == 0 {
                start -= 1;
            }
        }
    }
    let base = buf.line_to_char(y);
    Some((base + start, base + end + 1))
}

fn quoted_object(
    buf: &TextBuffer,
    y: usize,
    x: usize,
    q: char,
    inner: bool,
) -> Option<(usize, usize)> {
    let line: Vec<char> = buf.line_text(y).chars().collect();
    let quotes: Vec<usize> = line
        .iter()
        .enumerate()
        .filter(|(_, &c)| c == q)
        .map(|(i, _)| i)
        .collect();
    // Pair consecutive quotes; pick the first pair whose closing quote is at or
    // after the cursor.
    let mut pair = None;
    let mut i = 0;
    while i + 1 < quotes.len() {
        let (p1, p2) = (quotes[i], quotes[i + 1]);
        if x <= p2 {
            pair = Some((p1, p2));
            break;
        }
        i += 2;
    }
    let (p1, p2) = pair?;
    let base = buf.line_to_char(y);
    if inner {
        Some((base + p1 + 1, base + p2))
    } else {
        // `a"`: include the quotes and trailing whitespace if any.
        let mut end = p2 + 1;
        while end < line.len() && line[end].is_whitespace() {
            end += 1;
        }
        Some((base + p1, base + end))
    }
}

fn pair_object(
    buf: &TextBuffer,
    y: usize,
    x: usize,
    open: char,
    close: char,
    inner: bool,
) -> Option<(usize, usize)> {
    let chars: Vec<char> = buf.raw_content().chars().collect();
    let cursor = buf.cursor_to_char(y, x);
    if cursor >= chars.len() {
        return None;
    }
    // Find the enclosing opening bracket (scanning left).
    let open_idx = {
        let mut depth = 0i32;
        let mut idx = None;
        let mut i = cursor as isize;
        while i >= 0 {
            let c = chars[i as usize];
            if c == close && (i as usize) != cursor {
                depth += 1;
            } else if c == open {
                if depth == 0 {
                    idx = Some(i as usize);
                    break;
                }
                depth -= 1;
            }
            i -= 1;
        }
        idx?
    };
    // Find the matching close (scanning right from the open).
    let close_idx = {
        let mut depth = 0i32;
        let mut idx = None;
        let mut i = open_idx;
        while i < chars.len() {
            let c = chars[i];
            if c == open {
                depth += 1;
            } else if c == close {
                depth -= 1;
                if depth == 0 {
                    idx = Some(i);
                    break;
                }
            }
            i += 1;
        }
        idx?
    };
    if inner {
        Some((open_idx + 1, close_idx))
    } else {
        Some((open_idx, close_idx + 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(s: &str) -> TextBuffer {
        let mut b = TextBuffer::new();
        b.set_content(s);
        b
    }

    fn text(b: &TextBuffer, r: (usize, usize)) -> String {
        b.slice_text(r.0..r.1)
    }

    #[test]
    fn inner_word() {
        let b = buf("foo bar baz");
        let r = range(&b, 0, 5, TextObject::Word { big: false }, true).unwrap();
        assert_eq!(text(&b, r), "bar");
    }

    #[test]
    fn a_word_includes_trailing_space() {
        let b = buf("foo bar baz");
        let r = range(&b, 0, 0, TextObject::Word { big: false }, false).unwrap();
        assert_eq!(text(&b, r), "foo ");
    }

    #[test]
    fn inner_and_around_quotes() {
        let b = buf("say \"hello\" now");
        let i = range(&b, 0, 6, TextObject::Quoted('"'), true).unwrap();
        assert_eq!(text(&b, i), "hello");
        let a = range(&b, 0, 6, TextObject::Quoted('"'), false).unwrap();
        assert_eq!(text(&b, a), "\"hello\" ");
    }

    #[test]
    fn inner_and_around_parens() {
        let b = buf("f(a, b)");
        let i = range(&b, 0, 3, TextObject::Pair('(', ')'), true).unwrap();
        assert_eq!(text(&b, i), "a, b");
        let a = range(&b, 0, 3, TextObject::Pair('(', ')'), false).unwrap();
        assert_eq!(text(&b, a), "(a, b)");
    }

    #[test]
    fn nested_parens_pick_inner() {
        let b = buf("(a (b) c)");
        // cursor on 'b' inside the inner pair
        let i = range(&b, 0, 4, TextObject::Pair('(', ')'), true).unwrap();
        assert_eq!(text(&b, i), "b");
    }

    #[test]
    fn pair_across_lines() {
        let b = buf("foo(\n  bar\n)");
        let i = range(&b, 1, 2, TextObject::Pair('(', ')'), true).unwrap();
        assert_eq!(text(&b, i), "\n  bar\n");
    }

    #[test]
    fn no_surrounding_pair_returns_none() {
        let b = buf("no brackets here");
        assert!(range(&b, 0, 3, TextObject::Pair('(', ')'), true).is_none());
    }
}
