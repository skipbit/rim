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
    /// `ip` / `ap` — a paragraph (run of non-blank or blank lines). Linewise.
    Paragraph,
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

/// Resolve a text object to a half-open whole-buffer char range and whether it
/// is linewise. Returns `None` when there is nothing to select.
pub fn range(
    buf: &TextBuffer,
    y: usize,
    x: usize,
    obj: TextObject,
    inner: bool,
    count: usize,
) -> Option<(usize, usize, bool)> {
    let count = count.max(1);
    match obj {
        TextObject::Word { big } => {
            word_object(buf, y, x, big, inner, count).map(|(s, e)| (s, e, false))
        }
        TextObject::Quoted(q) => quoted_object(buf, y, x, q, inner).map(|(s, e)| (s, e, false)),
        TextObject::Pair(open, close) => {
            pair_object(buf, y, x, open, close, inner, count).map(|(s, e)| (s, e, false))
        }
        TextObject::Paragraph => paragraph_object(buf, y, inner, count).map(|(s, e)| (s, e, true)),
    }
}

/// Contiguous same-class runs of a line, as `(class, start_x, end_x)` (end
/// exclusive).
fn segments(line: &[char], big: bool) -> Vec<(u8, usize, usize)> {
    let mut segs = Vec::new();
    let mut i = 0;
    while i < line.len() {
        let c = class_word(line[i], big);
        let start = i;
        while i < line.len() && class_word(line[i], big) == c {
            i += 1;
        }
        segs.push((c, start, i));
    }
    segs
}

fn word_object(
    buf: &TextBuffer,
    y: usize,
    x: usize,
    big: bool,
    inner: bool,
    count: usize,
) -> Option<(usize, usize)> {
    let line: Vec<char> = buf.line_text(y).chars().collect();
    if line.is_empty() {
        return None;
    }
    let x = x.min(line.len() - 1);
    let segs = segments(&line, big);
    let idx = segs.iter().position(|&(_, s, e)| x >= s && x < e)?;
    let n = segs.len();
    let base = buf.line_to_char(y);

    if inner {
        // `count` consecutive runs starting at the cursor's run.
        let end = (idx + count - 1).min(n - 1);
        Some((base + segs[idx].1, base + segs[end].2))
    } else {
        // `aw`: `count` words plus their separating whitespace (trailing, or a
        // leading blank when there is no trailing one).
        let mut words = 0;
        let mut j = idx;
        loop {
            if segs[j].0 != 0 {
                words += 1;
            }
            if (words >= count && segs[j].0 != 0) || j + 1 >= n {
                break;
            }
            j += 1;
        }
        let end = j;
        let mut lo = segs[idx].1;
        let mut hi = segs[end].2;
        if end + 1 < n && segs[end + 1].0 == 0 {
            hi = segs[end + 1].2;
        } else if idx > 0 && segs[idx - 1].0 == 0 {
            lo = segs[idx - 1].1;
        }
        Some((base + lo, base + hi))
    }
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

/// Find the bracket pair enclosing char index `cursor`.
fn enclosing_pair(
    chars: &[char],
    cursor: usize,
    open: char,
    close: char,
) -> Option<(usize, usize)> {
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
    let close_idx = {
        let mut depth = 0i32;
        let mut idx = None;
        for (i, &c) in chars.iter().enumerate().skip(open_idx) {
            if c == open {
                depth += 1;
            } else if c == close {
                depth -= 1;
                if depth == 0 {
                    idx = Some(i);
                    break;
                }
            }
        }
        idx?
    };
    Some((open_idx, close_idx))
}

fn pair_object(
    buf: &TextBuffer,
    y: usize,
    x: usize,
    open: char,
    close: char,
    inner: bool,
    count: usize,
) -> Option<(usize, usize)> {
    let chars: Vec<char> = buf.raw_content().chars().collect();
    let mut cursor = buf.cursor_to_char(y, x);
    if cursor >= chars.len() {
        return None;
    }
    let mut found = None;
    for _ in 0..count {
        let (o, c) = enclosing_pair(&chars, cursor, open, close)?;
        found = Some((o, c));
        if o == 0 {
            break;
        }
        cursor = o - 1; // search outward for the next enclosing pair
    }
    let (o, c) = found?;
    if inner {
        Some((o + 1, c))
    } else {
        Some((o, c + 1))
    }
}

fn paragraph_object(
    buf: &TextBuffer,
    y: usize,
    inner: bool,
    count: usize,
) -> Option<(usize, usize)> {
    let n = buf.line_count();
    if n == 0 {
        return None;
    }
    let is_blank = |i: usize| buf.line_char_len(i) == 0;
    let start_blank = is_blank(y);

    // Extend up/down over lines of the same blank/non-blank kind.
    let mut lo = y;
    while lo > 0 && is_blank(lo - 1) == start_blank {
        lo -= 1;
    }
    let mut hi = y;
    while hi + 1 < n && is_blank(hi + 1) == start_blank {
        hi += 1;
    }

    // count > 1: absorb further alternating blocks downward.
    let mut remaining = count - 1;
    while remaining > 0 && hi + 1 < n {
        let kind = is_blank(hi + 1);
        hi += 1;
        while hi + 1 < n && is_blank(hi + 1) == kind {
            hi += 1;
        }
        remaining -= 1;
    }

    if !inner && !start_blank {
        // `ap`: include trailing blank lines, or leading blanks if none follow.
        if hi + 1 < n && is_blank(hi + 1) {
            while hi + 1 < n && is_blank(hi + 1) {
                hi += 1;
            }
        } else {
            while lo > 0 && is_blank(lo - 1) {
                lo -= 1;
            }
        }
    }

    let s = buf.line_to_char(lo);
    let e = buf.line_to_char(hi + 1);
    Some((s, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(s: &str) -> TextBuffer {
        let mut b = TextBuffer::new();
        b.set_content(s);
        b
    }

    fn text(b: &TextBuffer, r: (usize, usize, bool)) -> String {
        b.slice_text(r.0..r.1)
    }

    #[test]
    fn inner_word() {
        let b = buf("foo bar baz");
        let r = range(&b, 0, 5, TextObject::Word { big: false }, true, 1).unwrap();
        assert_eq!(text(&b, r), "bar");
    }

    #[test]
    fn a_word_includes_trailing_space() {
        let b = buf("foo bar baz");
        let r = range(&b, 0, 0, TextObject::Word { big: false }, false, 1).unwrap();
        assert_eq!(text(&b, r), "foo ");
    }

    #[test]
    fn inner_word_count() {
        let b = buf("foo bar baz");
        // 2iw = word + following whitespace.
        let r = range(&b, 0, 0, TextObject::Word { big: false }, true, 2).unwrap();
        assert_eq!(text(&b, r), "foo ");
        // 3iw = word + ws + word.
        let r3 = range(&b, 0, 0, TextObject::Word { big: false }, true, 3).unwrap();
        assert_eq!(text(&b, r3), "foo bar");
    }

    #[test]
    fn a_word_count() {
        let b = buf("foo bar baz");
        let r = range(&b, 0, 0, TextObject::Word { big: false }, false, 2).unwrap();
        assert_eq!(text(&b, r), "foo bar ");
    }

    #[test]
    fn inner_and_around_quotes() {
        let b = buf("say \"hello\" now");
        let i = range(&b, 0, 6, TextObject::Quoted('"'), true, 1).unwrap();
        assert_eq!(text(&b, i), "hello");
        let a = range(&b, 0, 6, TextObject::Quoted('"'), false, 1).unwrap();
        assert_eq!(text(&b, a), "\"hello\" ");
    }

    #[test]
    fn inner_and_around_parens() {
        let b = buf("f(a, b)");
        let i = range(&b, 0, 3, TextObject::Pair('(', ')'), true, 1).unwrap();
        assert_eq!(text(&b, i), "a, b");
        let a = range(&b, 0, 3, TextObject::Pair('(', ')'), false, 1).unwrap();
        assert_eq!(text(&b, a), "(a, b)");
    }

    #[test]
    fn nested_parens_pick_inner_then_outer_by_count() {
        let b = buf("(a (b) c)");
        let i = range(&b, 0, 4, TextObject::Pair('(', ')'), true, 1).unwrap();
        assert_eq!(text(&b, i), "b");
        // 2i( selects the outer pair.
        let o = range(&b, 0, 4, TextObject::Pair('(', ')'), true, 2).unwrap();
        assert_eq!(text(&b, o), "a (b) c");
    }

    #[test]
    fn pair_across_lines() {
        let b = buf("foo(\n  bar\n)");
        let i = range(&b, 1, 2, TextObject::Pair('(', ')'), true, 1).unwrap();
        assert_eq!(text(&b, i), "\n  bar\n");
    }

    #[test]
    fn no_surrounding_pair_returns_none() {
        let b = buf("no brackets here");
        assert!(range(&b, 0, 3, TextObject::Pair('(', ')'), true, 1).is_none());
    }

    #[test]
    fn inner_paragraph_is_linewise() {
        let b = buf("a\nb\n\nc\nd");
        let r = range(&b, 0, 0, TextObject::Paragraph, true, 1).unwrap();
        assert_eq!(r.2, true); // linewise
        assert_eq!(text(&b, r), "a\nb\n"); // first paragraph block, incl. newlines
    }

    #[test]
    fn around_paragraph_includes_trailing_blanks() {
        let b = buf("a\nb\n\n\nc");
        let r = range(&b, 0, 0, TextObject::Paragraph, false, 1).unwrap();
        assert_eq!(text(&b, r), "a\nb\n\n\n"); // paragraph + following blank lines
    }
}
