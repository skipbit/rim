use crate::domain::text_buffer::TextBuffer;

/// Whether a motion spans whole lines or a character range.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MotionKind {
    Charwise,
    Linewise,
}

/// The resolved destination of a motion from a starting cursor position.
///
/// For a bare motion the cursor moves to `(y, x)`. For an operator (`d`/`c`/`y`)
/// the affected range runs between the start and `(y, x)`; `kind` selects
/// linewise vs charwise and `inclusive` decides whether the target char itself
/// is included (charwise only).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Target {
    pub y: usize,
    pub x: usize,
    pub kind: MotionKind,
    pub inclusive: bool,
}

/// The set of cursor motions understood by normal mode.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    /// `w` / `W`
    WordForward {
        big: bool,
    },
    /// `b` / `B`
    WordBackward {
        big: bool,
    },
    /// `e` / `E`
    WordEnd {
        big: bool,
    },
    /// `0`
    LineStart,
    /// `^`
    FirstNonBlank,
    /// `$`
    LineEnd,
    /// `gg`
    FileStart,
    /// `G`
    FileEnd,
    /// `<count>G` / `<count>gg` — jump to a 1-based line number.
    GotoLine(usize),
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Class {
    Blank,
    Word,
    Punct,
}

fn class(c: char, big: bool) -> Class {
    if c.is_whitespace() {
        Class::Blank
    } else if big {
        // For WORD motions every non-blank char is the same class, so runs are
        // simply whitespace-delimited.
        Class::Word
    } else if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

/// Map a whole-buffer char index back to a `(y, x)` cursor position, clamping
/// `x` to the line length so results never point past a line's end.
fn char_to_yx(buf: &TextBuffer, idx: usize) -> (usize, usize) {
    if buf.line_count() == 0 {
        return (0, 0);
    }
    let len = buf.len_chars();
    if idx >= len {
        let y = buf.line_count() - 1;
        return (y, buf.line_char_len(y));
    }
    let y = buf.char_to_line(idx);
    let x = (idx - buf.line_to_char(y)).min(buf.line_char_len(y));
    (y, x)
}

fn is_empty_line_at(chars: &[char], j: usize) -> bool {
    chars[j] == '\n' && (j == 0 || chars[j - 1] == '\n')
}

fn charwise(buf: &TextBuffer, idx: usize, inclusive: bool) -> Target {
    let (y, x) = char_to_yx(buf, idx);
    Target {
        y,
        x,
        kind: MotionKind::Charwise,
        inclusive,
    }
}

fn word_forward(chars: &[char], mut i: usize, big: bool, count: usize) -> usize {
    let n = chars.len();
    for _ in 0..count {
        if i >= n {
            break;
        }
        let start = class(chars[i], big);
        let mut j = i + 1;
        if start != Class::Blank {
            while j < n && class(chars[j], big) == start {
                j += 1;
            }
        }
        while j < n && class(chars[j], big) == Class::Blank {
            if is_empty_line_at(chars, j) {
                break;
            }
            j += 1;
        }
        i = j;
    }
    i
}

fn word_backward(chars: &[char], mut i: usize, big: bool, count: usize) -> usize {
    for _ in 0..count {
        if i == 0 {
            break;
        }
        let mut j = i - 1;
        while j > 0 && class(chars[j], big) == Class::Blank {
            if is_empty_line_at(chars, j) {
                break;
            }
            j -= 1;
        }
        if class(chars[j], big) == Class::Blank {
            i = j;
        } else {
            let cls = class(chars[j], big);
            while j > 0 && class(chars[j - 1], big) == cls {
                j -= 1;
            }
            i = j;
        }
    }
    i
}

fn word_end(chars: &[char], mut i: usize, big: bool, count: usize) -> usize {
    let n = chars.len();
    for _ in 0..count {
        let mut j = i + 1;
        while j < n && class(chars[j], big) == Class::Blank {
            j += 1;
        }
        if j >= n {
            break;
        }
        let cls = class(chars[j], big);
        while j + 1 < n && class(chars[j + 1], big) == cls {
            j += 1;
        }
        i = j;
    }
    i
}

/// Resolve `motion` from cursor `(y, x)` in `buf`, repeated `count` times where
/// meaningful.
pub fn compute(buf: &TextBuffer, y: usize, x: usize, motion: Motion, count: usize) -> Target {
    let count = count.max(1);
    let line_count = buf.line_count();
    match motion {
        Motion::Left => {
            let nx = x.saturating_sub(count);
            Target {
                y,
                x: nx,
                kind: MotionKind::Charwise,
                inclusive: false,
            }
        }
        Motion::Right => {
            let max_x = buf.line_char_len(y);
            let nx = (x + count).min(max_x);
            Target {
                y,
                x: nx,
                kind: MotionKind::Charwise,
                inclusive: false,
            }
        }
        Motion::Up => {
            let ny = y.saturating_sub(count);
            Target {
                y: ny,
                x,
                kind: MotionKind::Linewise,
                inclusive: false,
            }
        }
        Motion::Down => {
            let ny = (y + count).min(line_count.saturating_sub(1));
            Target {
                y: ny,
                x,
                kind: MotionKind::Linewise,
                inclusive: false,
            }
        }
        Motion::WordForward { big } => {
            let chars: Vec<char> = buf.raw_content().chars().collect();
            let i = buf.cursor_to_char(y, x);
            charwise(buf, word_forward(&chars, i, big, count), false)
        }
        Motion::WordBackward { big } => {
            let chars: Vec<char> = buf.raw_content().chars().collect();
            let i = buf.cursor_to_char(y, x);
            charwise(buf, word_backward(&chars, i, big, count), false)
        }
        Motion::WordEnd { big } => {
            let chars: Vec<char> = buf.raw_content().chars().collect();
            let i = buf.cursor_to_char(y, x);
            charwise(buf, word_end(&chars, i, big, count), true)
        }
        Motion::LineStart => Target {
            y,
            x: 0,
            kind: MotionKind::Charwise,
            inclusive: false,
        },
        Motion::FirstNonBlank => {
            let line = buf.line_text(y);
            let x = line
                .chars()
                .position(|c| !c.is_whitespace())
                .unwrap_or_else(|| buf.line_char_len(y));
            Target {
                y,
                x,
                kind: MotionKind::Charwise,
                inclusive: false,
            }
        }
        Motion::LineEnd => {
            let ny = (y + count - 1).min(line_count.saturating_sub(1));
            let x = buf.line_char_len(ny).saturating_sub(1);
            Target {
                y: ny,
                x,
                kind: MotionKind::Charwise,
                inclusive: true,
            }
        }
        Motion::FileStart => {
            let line = buf.line_text(0);
            let x = line.chars().position(|c| !c.is_whitespace()).unwrap_or(0);
            Target {
                y: 0,
                x,
                kind: MotionKind::Linewise,
                inclusive: false,
            }
        }
        Motion::FileEnd => {
            let ny = line_count.saturating_sub(1);
            let line = buf.line_text(ny);
            let x = line.chars().position(|c| !c.is_whitespace()).unwrap_or(0);
            Target {
                y: ny,
                x,
                kind: MotionKind::Linewise,
                inclusive: false,
            }
        }
        Motion::GotoLine(n) => {
            let ny = n.saturating_sub(1).min(line_count.saturating_sub(1));
            let line = buf.line_text(ny);
            let x = line.chars().position(|c| !c.is_whitespace()).unwrap_or(0);
            Target {
                y: ny,
                x,
                kind: MotionKind::Linewise,
                inclusive: false,
            }
        }
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

    fn word_fwd(s: &str, y: usize, x: usize) -> (usize, usize) {
        let t = compute(&buf(s), y, x, Motion::WordForward { big: false }, 1);
        (t.y, t.x)
    }

    #[test]
    fn w_within_line() {
        assert_eq!(word_fwd("foo bar baz", 0, 0), (0, 4)); // foo -> bar
        assert_eq!(word_fwd("foo bar baz", 0, 4), (0, 8)); // bar -> baz
    }

    #[test]
    fn w_stops_on_punctuation() {
        assert_eq!(word_fwd("foo.bar", 0, 0), (0, 3)); // foo -> '.'
        assert_eq!(word_fwd("foo.bar", 0, 3), (0, 4)); // '.' -> bar
    }

    #[test]
    fn w_crosses_lines() {
        assert_eq!(word_fwd("foo\nbar", 0, 2), (1, 0)); // last of foo -> bar
    }

    #[test]
    fn w_stops_on_empty_line() {
        assert_eq!(word_fwd("a\n\nb", 0, 0), (1, 0)); // 'a' -> empty line
        assert_eq!(word_fwd("a\n\nb", 1, 0), (2, 0)); // empty line -> 'b'
    }

    #[test]
    fn big_word_ignores_punctuation() {
        let t = compute(
            &buf("foo.bar baz"),
            0,
            0,
            Motion::WordForward { big: true },
            1,
        );
        assert_eq!((t.y, t.x), (0, 8)); // whole "foo.bar" is one WORD -> baz
    }

    #[test]
    fn b_within_and_across_lines() {
        let back = |s: &str, y, x| {
            let t = compute(&buf(s), y, x, Motion::WordBackward { big: false }, 1);
            (t.y, t.x)
        };
        assert_eq!(back("foo bar", 0, 4), (0, 0)); // bar -> foo
        assert_eq!(back("foo.bar", 0, 4), (0, 3)); // bar -> '.'
        assert_eq!(back("foo\nbar", 1, 0), (0, 0)); // bar -> foo (cross line)
    }

    #[test]
    fn e_end_of_word() {
        let end = |s: &str, y, x| {
            let t = compute(&buf(s), y, x, Motion::WordEnd { big: false }, 1);
            (t.y, t.x, t.inclusive)
        };
        assert_eq!(end("foo bar", 0, 0), (0, 2, true)); // -> last of foo
        assert_eq!(end("foo bar", 0, 2), (0, 6, true)); // -> last of bar
    }

    #[test]
    fn word_forward_with_count() {
        let t = compute(&buf("a b c d"), 0, 0, Motion::WordForward { big: false }, 3);
        assert_eq!((t.y, t.x), (0, 6)); // a -> d
    }

    #[test]
    fn line_motions() {
        let b = buf("  hello world");
        let ls = compute(&b, 0, 5, Motion::LineStart, 1);
        assert_eq!((ls.y, ls.x), (0, 0));
        let fnb = compute(&b, 0, 5, Motion::FirstNonBlank, 1);
        assert_eq!((fnb.y, fnb.x), (0, 2));
        let le = compute(&b, 0, 0, Motion::LineEnd, 1);
        assert_eq!((le.y, le.x, le.inclusive), (0, 12, true));
    }

    #[test]
    fn file_motions() {
        let b = buf("one\ntwo\nthree");
        let gg = compute(&b, 2, 0, Motion::FileStart, 1);
        assert_eq!((gg.y, gg.x, gg.kind), (0, 0, MotionKind::Linewise));
        let g = compute(&b, 0, 0, Motion::FileEnd, 1);
        assert_eq!((g.y, g.x, g.kind), (2, 0, MotionKind::Linewise));
        let goto = compute(&b, 0, 0, Motion::GotoLine(2), 1);
        assert_eq!((goto.y, goto.x), (1, 0));
    }

    #[test]
    fn multibyte_word_motion() {
        // Japanese chars are alphanumeric -> one word; space then ASCII word.
        let t = compute(
            &buf("あいう abc"),
            0,
            0,
            Motion::WordForward { big: false },
            1,
        );
        assert_eq!((t.y, t.x), (0, 4)); // start of "abc" at char offset 4
    }
}
