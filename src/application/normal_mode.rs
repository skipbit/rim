use crate::application::editor_service::EditorService;
use crate::application::lsp::LspRequest;
use crate::domain::editor_model::{EditorMode, Operator};
use crate::domain::motion::Motion;
use crate::domain::text_object::TextObject;
use crate::infrastructure::file_io::FileIO;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Result of feeding one key to normal mode.
pub enum NormalResult {
    Continue,
    Quit,
}

/// Normal-mode input interpreter implementing Vim's compositional grammar:
/// an optional count, an optional operator (`d`/`c`/`y`) with its own optional
/// count, and a motion or text object. State accumulates across keystrokes
/// until a complete command is recognised, then resets.
#[derive(Default)]
pub struct NormalMode {
    count: Option<usize>,
    operator: Option<Operator>,
    op_count: Option<usize>,
    pending_g: bool,
    /// Set to `Some(inner)` after an operator + `i`/`a`, awaiting the object key.
    pending_object: Option<bool>,
    /// Set to `Some((till, forward))` after `f`/`t`/`F`/`T`, awaiting the target.
    pending_find: Option<(bool, bool)>,
    /// The last completed `f`/`t`/`F`/`T` as `(target, till, forward)`, for
    /// `;` (repeat) and `,` (repeat reversed). Persists across commands.
    last_find: Option<(char, bool, bool)>,
}

impl NormalMode {
    pub fn new() -> Self {
        Self::default()
    }

    fn reset(&mut self) {
        self.count = None;
        self.operator = None;
        self.op_count = None;
        self.pending_g = false;
        self.pending_object = None;
        self.pending_find = None;
    }

    /// Combined repeat count: a count before the operator multiplies a count
    /// after it (Vim semantics: `2d3w` deletes six words).
    fn effective_count(&self) -> usize {
        self.count.unwrap_or(1) * self.op_count.unwrap_or(1)
    }

    fn count_in_progress(&self) -> bool {
        if self.operator.is_some() {
            self.op_count.is_some()
        } else {
            self.count.is_some()
        }
    }

    fn push_digit(&mut self, d: usize) {
        if self.operator.is_some() {
            self.op_count = Some(self.op_count.unwrap_or(0) * 10 + d);
        } else {
            self.count = Some(self.count.unwrap_or(0) * 10 + d);
        }
    }

    fn run_motion<T: FileIO>(
        &mut self,
        svc: &mut EditorService<T>,
        motion: Motion,
        status: &mut String,
    ) {
        let count = self.effective_count();
        if let Some(op) = self.operator {
            let enter_insert = svc.editor_model.apply_operator(op, motion, count);
            if enter_insert {
                svc.set_mode(EditorMode::Insert);
                *status = "-- INSERT --".to_string();
            } else {
                status.clear();
            }
        } else {
            svc.editor_model.move_by_motion(motion, count);
            status.clear();
        }
        self.reset();
    }

    fn handle_operator<T: FileIO>(
        &mut self,
        svc: &mut EditorService<T>,
        op: Operator,
        status: &mut String,
    ) {
        if self.operator == Some(op) {
            // Doubled operator: dd / cc / yy operate on whole lines.
            let count = self.effective_count();
            let enter_insert = svc.editor_model.operate_current_lines(op, count);
            if enter_insert {
                svc.set_mode(EditorMode::Insert);
                *status = "-- INSERT --".to_string();
            } else {
                status.clear();
            }
            self.reset();
        } else if self.operator.is_some() {
            // A different operator after one already pending cancels.
            self.reset();
            status.clear();
        } else {
            self.operator = Some(op);
        }
    }

    fn enter_insert<T: FileIO>(&mut self, svc: &mut EditorService<T>, status: &mut String) {
        svc.set_mode(EditorMode::Insert);
        *status = "-- INSERT --".to_string();
    }

    pub fn feed<T: FileIO>(
        &mut self,
        svc: &mut EditorService<T>,
        ev: &KeyEvent,
        status: &mut String,
    ) -> NormalResult {
        // Second key of a `g`-prefixed command.
        if self.pending_g {
            self.pending_g = false;
            match ev.code {
                KeyCode::Char('g') => {
                    let motion = if self.count_in_progress() {
                        Motion::GotoLine(self.effective_count())
                    } else {
                        Motion::FileStart
                    };
                    self.run_motion(svc, motion, status);
                }
                KeyCode::Char('e') => {
                    self.run_motion(svc, Motion::WordPrevEnd { big: false }, status)
                }
                KeyCode::Char('E') => {
                    self.run_motion(svc, Motion::WordPrevEnd { big: true }, status)
                }
                // `gd`: go to definition. Record the jump origin first.
                KeyCode::Char('d') => {
                    svc.editor_model.push_jump();
                    let (y, x) = (svc.editor_model.cursor_y, svc.editor_model.cursor_x);
                    svc.request_lsp(LspRequest::Definition { y, x });
                    status.clear();
                }
                _ => {
                    self.reset();
                    status.clear();
                }
            }
            return NormalResult::Continue;
        }

        // Object key of an `i`/`a` text object following an operator.
        if let Some(inner) = self.pending_object.take() {
            let obj = match ev.code {
                KeyCode::Char('w') => Some(TextObject::Word { big: false }),
                KeyCode::Char('W') => Some(TextObject::Word { big: true }),
                KeyCode::Char('"') => Some(TextObject::Quoted('"')),
                KeyCode::Char('\'') => Some(TextObject::Quoted('\'')),
                KeyCode::Char('`') => Some(TextObject::Quoted('`')),
                KeyCode::Char('(') | KeyCode::Char(')') | KeyCode::Char('b') => {
                    Some(TextObject::Pair('(', ')'))
                }
                KeyCode::Char('{') | KeyCode::Char('}') | KeyCode::Char('B') => {
                    Some(TextObject::Pair('{', '}'))
                }
                KeyCode::Char('[') | KeyCode::Char(']') => Some(TextObject::Pair('[', ']')),
                KeyCode::Char('p') => Some(TextObject::Paragraph),
                _ => None,
            };
            if let (Some(obj), Some(op)) = (obj, self.operator) {
                let count = self.effective_count();
                let enter = svc
                    .editor_model
                    .apply_operator_textobject(op, obj, inner, count);
                if enter {
                    self.enter_insert(svc, status);
                } else {
                    status.clear();
                }
            }
            self.reset();
            return NormalResult::Continue;
        }

        // Target character of an `f`/`t`/`F`/`T` search.
        if let Some((till, forward)) = self.pending_find.take() {
            if let KeyCode::Char(target) = ev.code {
                self.last_find = Some((target, till, forward));
                self.run_motion(
                    svc,
                    Motion::Find {
                        target,
                        till,
                        forward,
                    },
                    status,
                );
            } else {
                self.reset();
                status.clear();
            }
            return NormalResult::Continue;
        }

        match ev.code {
            KeyCode::Esc => {
                self.reset();
                status.clear();
            }

            // `i`/`a` begin a text object only when an operator is pending.
            KeyCode::Char('i') if self.operator.is_some() => self.pending_object = Some(true),
            KeyCode::Char('a') if self.operator.is_some() => self.pending_object = Some(false),

            // Counts. '0' is a digit only while a count is being built, else it
            // is the line-start motion.
            KeyCode::Char('0') if self.count_in_progress() => self.push_digit(0),
            KeyCode::Char(c @ '1'..='9') => self.push_digit(c.to_digit(10).unwrap() as usize),

            // Operators.
            KeyCode::Char('d') => self.handle_operator(svc, Operator::Delete, status),
            KeyCode::Char('c') => self.handle_operator(svc, Operator::Change, status),
            KeyCode::Char('y') => self.handle_operator(svc, Operator::Yank, status),

            // Motions (work bare or as an operator's range).
            KeyCode::Char('h') | KeyCode::Left => self.run_motion(svc, Motion::Left, status),
            KeyCode::Char('l') | KeyCode::Right => self.run_motion(svc, Motion::Right, status),
            KeyCode::Char('j') | KeyCode::Down => self.run_motion(svc, Motion::Down, status),
            KeyCode::Char('k') | KeyCode::Up => self.run_motion(svc, Motion::Up, status),
            KeyCode::Char('w') => self.run_motion(svc, Motion::WordForward { big: false }, status),
            KeyCode::Char('W') => self.run_motion(svc, Motion::WordForward { big: true }, status),
            KeyCode::Char('b') => self.run_motion(svc, Motion::WordBackward { big: false }, status),
            KeyCode::Char('B') => self.run_motion(svc, Motion::WordBackward { big: true }, status),
            KeyCode::Char('e') => self.run_motion(svc, Motion::WordEnd { big: false }, status),
            KeyCode::Char('E') => self.run_motion(svc, Motion::WordEnd { big: true }, status),
            KeyCode::Char('0') => self.run_motion(svc, Motion::LineStart, status),
            KeyCode::Char('^') => self.run_motion(svc, Motion::FirstNonBlank, status),
            KeyCode::Char('$') => self.run_motion(svc, Motion::LineEnd, status),
            KeyCode::Char('G') => {
                let motion = if self.count_in_progress() {
                    Motion::GotoLine(self.effective_count())
                } else {
                    Motion::FileEnd
                };
                self.run_motion(svc, motion, status);
            }
            KeyCode::Char('g') => self.pending_g = true,

            // Character-search motions await their target character.
            KeyCode::Char('f') => self.pending_find = Some((false, true)),
            KeyCode::Char('t') => self.pending_find = Some((true, true)),
            KeyCode::Char('F') => self.pending_find = Some((false, false)),
            KeyCode::Char('T') => self.pending_find = Some((true, false)),
            KeyCode::Char('%') => self.run_motion(svc, Motion::MatchPair, status),
            KeyCode::Char(';') => match self.last_find {
                Some((target, till, forward)) => self.run_motion(
                    svc,
                    Motion::Find {
                        target,
                        till,
                        forward,
                    },
                    status,
                ),
                None => {
                    self.reset();
                    status.clear();
                }
            },
            KeyCode::Char(',') => match self.last_find {
                Some((target, till, forward)) => self.run_motion(
                    svc,
                    Motion::Find {
                        target,
                        till,
                        forward: !forward,
                    },
                    status,
                ),
                None => {
                    self.reset();
                    status.clear();
                }
            },

            // From here on, a pending operator with a non-motion key cancels.
            _ if self.operator.is_some() => {
                self.reset();
                status.clear();
            }

            // Jump list: Ctrl-o back, Ctrl-i forward (guarded before the bare
            // `o`/`i` insert-entry arms below).
            KeyCode::Char('o') if ev.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some((path, y, x)) = svc.editor_model.jump_back() {
                    jump_to(svc, path, y, x);
                }
                self.reset();
                status.clear();
            }
            KeyCode::Char('i') if ev.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some((path, y, x)) = svc.editor_model.jump_forward() {
                    jump_to(svc, path, y, x);
                }
                self.reset();
                status.clear();
            }

            // Operator shortcuts (no pending operator here).
            KeyCode::Char('D') => {
                svc.editor_model
                    .apply_operator(Operator::Delete, Motion::LineEnd, 1);
                self.reset();
                status.clear();
            }
            KeyCode::Char('C') => {
                let enter = svc
                    .editor_model
                    .apply_operator(Operator::Change, Motion::LineEnd, 1);
                if enter {
                    self.enter_insert(svc, status);
                }
                self.reset();
            }
            KeyCode::Char('Y') => {
                let count = self.effective_count();
                svc.editor_model
                    .operate_current_lines(Operator::Yank, count);
                self.reset();
                status.clear();
            }

            // Insert-entry actions.
            KeyCode::Char('i') => {
                self.enter_insert(svc, status);
                self.reset();
            }
            KeyCode::Char('a') => {
                let m = &mut svc.editor_model;
                if m.buffer.line_count() > 0 {
                    let len = m.buffer.line_char_len(m.cursor_y);
                    if m.cursor_x < len {
                        m.cursor_x += 1;
                    }
                }
                self.enter_insert(svc, status);
                self.reset();
            }
            KeyCode::Char('A') => {
                let m = &mut svc.editor_model;
                if m.buffer.line_count() > 0 {
                    m.cursor_x = m.buffer.line_char_len(m.cursor_y);
                }
                self.enter_insert(svc, status);
                self.reset();
            }
            KeyCode::Char('I') => {
                svc.editor_model.move_by_motion(Motion::FirstNonBlank, 1);
                self.enter_insert(svc, status);
                self.reset();
            }
            KeyCode::Char('o') => {
                svc.editor_model.insert_line_below();
                self.enter_insert(svc, status);
                self.reset();
            }
            KeyCode::Char('O') => {
                svc.editor_model.insert_line_above();
                self.enter_insert(svc, status);
                self.reset();
            }

            // Editing actions.
            KeyCode::Char('x') => {
                svc.editor_model.delete_under_cursor(self.effective_count());
                self.reset();
                status.clear();
            }
            KeyCode::Char('p') => {
                svc.editor_model.paste(true, self.effective_count());
                self.reset();
                status.clear();
            }
            KeyCode::Char('P') => {
                svc.editor_model.paste(false, self.effective_count());
                self.reset();
                status.clear();
            }
            KeyCode::Char('u') => {
                svc.editor_model.undo();
                self.reset();
                *status = "Undo".to_string();
            }
            KeyCode::Char('r') if ev.modifiers.contains(KeyModifiers::CONTROL) => {
                svc.editor_model.redo();
                self.reset();
                *status = "Redo".to_string();
            }
            KeyCode::Char('.') => {
                svc.editor_model.repeat_last_change();
                self.reset();
                status.clear();
            }

            // LSP: hover for the symbol under the cursor.
            KeyCode::Char('K') => {
                let (y, x) = (svc.editor_model.cursor_y, svc.editor_model.cursor_x);
                svc.request_lsp(LspRequest::Hover { y, x });
                self.reset();
                status.clear();
            }

            // Mode switches.
            KeyCode::Char('/') => {
                svc.set_mode(EditorMode::Search);
                svc.clear_command_buffer();
                self.reset();
                *status = "/".to_string();
            }
            KeyCode::Char('n') => {
                svc.find_next();
                self.reset();
            }
            KeyCode::Char('N') => {
                svc.find_previous();
                self.reset();
            }
            KeyCode::Char(':') => {
                svc.set_mode(EditorMode::Command);
                self.reset();
                *status = ":".to_string();
            }
            KeyCode::Char('q') => {
                self.reset();
                return NormalResult::Quit;
            }

            _ => {}
        }

        NormalResult::Continue
    }
}

/// Move to a jump-list location, opening its file first if it differs from the
/// current buffer. (Reloading the same file would clear undo history, so we
/// only open when the path actually changes; the main loop notices the file
/// switch and re-syncs the LSP / re-highlights.)
fn jump_to<T: FileIO>(svc: &mut EditorService<T>, path: Option<String>, y: usize, x: usize) {
    if let Some(p) = path {
        if Some(&p) != svc.editor_model.get_filepath() {
            let _ = svc.open_file(&p);
        }
    }
    svc.editor_model.goto(y, x);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::editor_model::EditorMode;
    use std::io;

    struct NoFile;
    impl FileIO for NoFile {
        fn read_file(&self, _path: &str) -> io::Result<String> {
            Ok(String::new())
        }
        fn write_file(&self, _path: &str, _content: &str) -> io::Result<()> {
            Ok(())
        }
    }

    fn service(content: &str) -> EditorService<NoFile> {
        let mut svc = EditorService::new(NoFile);
        svc.editor_model.set_content(content);
        svc
    }

    /// Feed a sequence of plain character keys.
    fn press(nm: &mut NormalMode, svc: &mut EditorService<NoFile>, keys: &str) {
        let mut status = String::new();
        for c in keys.chars() {
            let ev = KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
            nm.feed(svc, &ev, &mut status);
        }
    }

    #[test]
    fn dw_via_keystrokes() {
        let mut nm = NormalMode::new();
        let mut svc = service("foo bar baz");
        press(&mut nm, &mut svc, "dw");
        assert_eq!(svc.editor_model.buffer.line_text(0), "bar baz");
    }

    #[test]
    fn count_word_motion() {
        let mut nm = NormalMode::new();
        let mut svc = service("a b c d e");
        press(&mut nm, &mut svc, "3w"); // a -> d
        assert_eq!(svc.editor_model.cursor_x, 6);
    }

    #[test]
    fn count_dd_deletes_lines() {
        let mut nm = NormalMode::new();
        let mut svc = service("a\nb\nc\nd");
        press(&mut nm, &mut svc, "2dd");
        assert_eq!(
            svc.editor_model.buffer.to_lines(),
            vec!["c".to_string(), "d".to_string()]
        );
    }

    #[test]
    fn operator_count_multiplies() {
        // 2d2w == delete 4 words.
        let mut nm = NormalMode::new();
        let mut svc = service("one two three four five");
        press(&mut nm, &mut svc, "2d2w");
        assert_eq!(svc.editor_model.buffer.line_text(0), "five");
    }

    #[test]
    fn yy_then_p_pastes_line() {
        let mut nm = NormalMode::new();
        let mut svc = service("line1\nline2");
        press(&mut nm, &mut svc, "yyp");
        assert_eq!(
            svc.editor_model.buffer.to_lines(),
            vec![
                "line1".to_string(),
                "line1".to_string(),
                "line2".to_string()
            ]
        );
    }

    #[test]
    fn dollar_moves_to_line_end() {
        let mut nm = NormalMode::new();
        let mut svc = service("hello");
        press(&mut nm, &mut svc, "$");
        assert_eq!(svc.editor_model.cursor_x, 4);
    }

    #[test]
    fn gg_and_uppercase_g_jump() {
        let mut nm = NormalMode::new();
        let mut svc = service("one\ntwo\nthree");
        press(&mut nm, &mut svc, "G");
        assert_eq!(svc.editor_model.cursor_y, 2);
        press(&mut nm, &mut svc, "gg");
        assert_eq!(svc.editor_model.cursor_y, 0);
    }

    #[test]
    fn change_word_enters_insert_mode() {
        let mut nm = NormalMode::new();
        let mut svc = service("foo bar");
        press(&mut nm, &mut svc, "cw");
        assert!(matches!(svc.editor_model.mode, EditorMode::Insert));
        assert_eq!(svc.editor_model.buffer.line_text(0), " bar");
    }

    #[test]
    fn x_deletes_char_under_cursor() {
        let mut nm = NormalMode::new();
        let mut svc = service("abc");
        press(&mut nm, &mut svc, "x");
        assert_eq!(svc.editor_model.buffer.line_text(0), "bc");
    }

    #[test]
    fn i_enters_insert_mode() {
        let mut nm = NormalMode::new();
        let mut svc = service("abc");
        press(&mut nm, &mut svc, "i");
        assert!(matches!(svc.editor_model.mode, EditorMode::Insert));
    }

    #[test]
    fn esc_cancels_pending_operator() {
        let mut nm = NormalMode::new();
        let mut svc = service("foo bar");
        // 'd' then Esc should cancel; a following 'w' is a bare motion.
        let mut status = String::new();
        nm.feed(
            &mut svc,
            &KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
            &mut status,
        );
        nm.feed(
            &mut svc,
            &KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &mut status,
        );
        press(&mut nm, &mut svc, "w");
        assert_eq!(svc.editor_model.buffer.line_text(0), "foo bar"); // nothing deleted
        assert_eq!(svc.editor_model.cursor_x, 4); // 'w' moved to "bar"
    }

    #[test]
    fn diw_deletes_inner_word() {
        let mut nm = NormalMode::new();
        let mut svc = service("foo bar baz");
        svc.editor_model.cursor_x = 5; // on "bar"
        press(&mut nm, &mut svc, "diw");
        assert_eq!(svc.editor_model.buffer.line_text(0), "foo  baz");
    }

    #[test]
    fn daw_deletes_a_word_with_space() {
        let mut nm = NormalMode::new();
        let mut svc = service("foo bar baz");
        svc.editor_model.cursor_x = 4; // on "bar"
        press(&mut nm, &mut svc, "daw");
        assert_eq!(svc.editor_model.buffer.line_text(0), "foo baz");
    }

    #[test]
    fn ci_quote_changes_inside_quotes() {
        let mut nm = NormalMode::new();
        let mut svc = service("say \"hello\" now");
        svc.editor_model.cursor_x = 6; // inside the quotes
        press(&mut nm, &mut svc, "ci\"");
        assert_eq!(svc.editor_model.buffer.line_text(0), "say \"\" now");
        assert!(matches!(svc.editor_model.mode, EditorMode::Insert));
    }

    #[test]
    fn da_paren_deletes_around_parens() {
        let mut nm = NormalMode::new();
        let mut svc = service("f(a, b)g");
        svc.editor_model.cursor_x = 3; // inside the parens
        press(&mut nm, &mut svc, "da(");
        assert_eq!(svc.editor_model.buffer.line_text(0), "fg");
    }

    #[test]
    fn df_deletes_through_char() {
        let mut nm = NormalMode::new();
        let mut svc = service("hello world");
        press(&mut nm, &mut svc, "df "); // delete through the space
        assert_eq!(svc.editor_model.buffer.line_text(0), "world");
    }

    #[test]
    fn dt_deletes_up_to_char() {
        let mut nm = NormalMode::new();
        let mut svc = service("hello world");
        press(&mut nm, &mut svc, "dtw"); // delete up to (not incl) 'w'
        assert_eq!(svc.editor_model.buffer.line_text(0), "world");
    }

    #[test]
    fn ge_via_g_prefix() {
        let mut nm = NormalMode::new();
        let mut svc = service("foo bar");
        svc.editor_model.cursor_x = 5; // in "bar"
        press(&mut nm, &mut svc, "ge");
        assert_eq!(svc.editor_model.cursor_x, 2); // end of "foo"
    }

    #[test]
    fn semicolon_repeats_find_comma_reverses() {
        let mut nm = NormalMode::new();
        let mut svc = service("a.b.c.d");
        press(&mut nm, &mut svc, "f."); // -> first '.'
        assert_eq!(svc.editor_model.cursor_x, 1);
        press(&mut nm, &mut svc, ";"); // -> next '.'
        assert_eq!(svc.editor_model.cursor_x, 3);
        press(&mut nm, &mut svc, ";"); // -> next '.'
        assert_eq!(svc.editor_model.cursor_x, 5);
        press(&mut nm, &mut svc, ","); // reverse -> previous '.'
        assert_eq!(svc.editor_model.cursor_x, 3);
    }

    #[test]
    fn percent_jumps_to_match() {
        let mut nm = NormalMode::new();
        let mut svc = service("a(bcd)e");
        svc.editor_model.cursor_x = 1; // on '('
        press(&mut nm, &mut svc, "%");
        assert_eq!(svc.editor_model.cursor_x, 5); // on ')'
    }

    #[test]
    fn d_percent_deletes_bracket_span() {
        let mut nm = NormalMode::new();
        let mut svc = service("a(bcd)e");
        svc.editor_model.cursor_x = 1; // on '('
        press(&mut nm, &mut svc, "d%");
        assert_eq!(svc.editor_model.buffer.line_text(0), "ae");
    }

    #[test]
    fn dip_deletes_paragraph() {
        let mut nm = NormalMode::new();
        let mut svc = service("a\nb\n\nc\nd");
        svc.editor_model.cursor_y = 0;
        press(&mut nm, &mut svc, "dip");
        // First paragraph (a,b) deleted, leaving the blank line then c,d.
        assert_eq!(
            svc.editor_model.buffer.to_lines(),
            vec!["".to_string(), "c".to_string(), "d".to_string()]
        );
    }

    #[test]
    fn dap_deletes_paragraph_and_trailing_blank() {
        let mut nm = NormalMode::new();
        let mut svc = service("one\ntwo\n\nrest");
        svc.editor_model.cursor_y = 0;
        press(&mut nm, &mut svc, "dap"); // delete paragraph + following blank line
        assert_eq!(svc.editor_model.buffer.to_lines(), vec!["rest".to_string()]);
    }

    #[test]
    fn yap_yanks_paragraph_linewise() {
        let mut nm = NormalMode::new();
        let mut svc = service("one\ntwo\n\nrest");
        svc.editor_model.cursor_y = 0;
        press(&mut nm, &mut svc, "yap");
        assert_eq!(
            svc.editor_model.register,
            Some(crate::domain::editor_model::Register {
                text: "one\ntwo\n\n".to_string(),
                linewise: true,
            })
        );
    }

    #[test]
    fn count_inner_word_object() {
        let mut nm = NormalMode::new();
        let mut svc = service("foo bar baz");
        press(&mut nm, &mut svc, "d3iw"); // word + ws + word
        assert_eq!(svc.editor_model.buffer.line_text(0), " baz");
    }

    #[test]
    fn quit_key_returns_quit() {
        let mut nm = NormalMode::new();
        let mut svc = service("abc");
        let mut status = String::new();
        let r = nm.feed(
            &mut svc,
            &KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            &mut status,
        );
        assert!(matches!(r, NormalResult::Quit));
    }
}
