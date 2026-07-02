use unicode_segmentation::UnicodeSegmentation;

/// Char offsets of every grapheme-cluster boundary in `line`, including 0 and
/// the end. Horizontal cursor motion steps between these so a combining mark or
/// ZWJ emoji sequence moves as one visual unit.
fn boundaries(line: &str) -> Vec<usize> {
    let mut bounds = vec![0];
    let mut chars = 0;
    for g in line.graphemes(true) {
        chars += g.chars().count();
        bounds.push(chars);
    }
    bounds
}

/// Char offset of the grapheme boundary immediately after `char_idx`.
pub fn next_boundary(line: &str, char_idx: usize) -> usize {
    boundaries(line)
        .into_iter()
        .find(|&b| b > char_idx)
        .unwrap_or(char_idx)
}

/// Char offset of the grapheme boundary immediately before `char_idx`.
pub fn prev_boundary(line: &str, char_idx: usize) -> usize {
    boundaries(line)
        .into_iter()
        .rev()
        .find(|&b| b < char_idx)
        .unwrap_or(0)
}

/// Step `count` grapheme boundaries to the right of `char_idx`.
pub fn next_n(line: &str, char_idx: usize, count: usize) -> usize {
    let mut x = char_idx;
    for _ in 0..count {
        let nx = next_boundary(line, x);
        if nx == x {
            break;
        }
        x = nx;
    }
    x
}

/// Step `count` grapheme boundaries to the left of `char_idx`.
pub fn prev_n(line: &str, char_idx: usize, count: usize) -> usize {
    let mut x = char_idx;
    for _ in 0..count {
        let px = prev_boundary(line, x);
        if px == x {
            break;
        }
        x = px;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_boundaries() {
        assert_eq!(next_boundary("abc", 0), 1);
        assert_eq!(prev_boundary("abc", 2), 1);
        assert_eq!(next_n("abc", 0, 2), 2);
        assert_eq!(prev_n("abc", 3, 2), 1);
    }

    #[test]
    fn combining_mark_is_one_step() {
        // "a̐b" = 'a' + U+0310 + 'b'; the first grapheme spans 2 chars.
        assert_eq!(next_boundary("a\u{310}b", 0), 2);
        assert_eq!(prev_boundary("a\u{310}b", 2), 0);
    }
}
