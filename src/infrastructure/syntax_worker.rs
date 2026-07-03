//! Background syntax-highlighting engine (tree-sitter).
//!
//! This module owns the tree-sitter integration and keeps it off the edit
//! path. The pure engine — [`build_config`] + [`highlight_source`] — turns a
//! byte slice into a flat list of [`HlSpan`]s and is unit-tested synchronously.
//! [`spawn`] runs that engine on a dedicated thread: it receives
//! [`ParseRequest`]s (a cheap rope snapshot + revision), re-highlights, and
//! reports [`Highlights`] back over a tokio channel. Wiring into the async
//! event loop happens in the application/`main` layers.
//!
//! Note (deliberate MS3 scope): `tree-sitter-highlight` re-parses the whole
//! `source` on every call — its public API exposes no `old_tree` reuse — so we
//! re-highlight the entire document per (debounced) change and clip spans to
//! the viewport at render time. True incremental parsing is a later migration
//! to the raw `Query` API.

use crossterm::style::Color;
use ropey::Rope;
use tree_sitter_highlight::{Highlight, HighlightConfiguration, HighlightEvent, Highlighter};

/// A contiguous run of source bytes that should be drawn in one highlight
/// style. `style` indexes [`HIGHLIGHT_NAMES`] (resolve to a colour with
/// [`color_for`]). Byte offsets are into the snapshot that produced them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HlSpan {
    pub start_byte: usize,
    pub end_byte: usize,
    pub style: usize,
}

/// A request to (re-)highlight a document snapshot.
#[derive(Clone)]
pub struct ParseRequest {
    pub revision: u64,
    pub rope: Rope,
}

/// The result of highlighting a snapshot, tagged with the revision it was based
/// on so the main loop can drop stale results.
#[derive(Clone, Debug)]
pub struct Highlights {
    pub revision: u64,
    pub spans: Vec<HlSpan>,
}

/// Capture names we recognise, in a fixed order. `configure` maps grammar
/// captures onto these indices (longest-prefix match), and highlight events
/// carry the index, so this order defines the `style` values in [`HlSpan`].
/// These mirror the captures used by `tree-sitter-rust`'s `highlights.scm`.
pub const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",
    "comment",
    "comment.documentation",
    "constant",
    "constant.builtin",
    "constructor",
    "escape",
    "function",
    "function.macro",
    "function.method",
    "keyword",
    "label",
    "operator",
    "property",
    "punctuation.bracket",
    "punctuation.delimiter",
    "string",
    "type",
    "type.builtin",
    "variable.builtin",
    "variable.parameter",
];

/// Map a `style` index (into [`HIGHLIGHT_NAMES`]) to a terminal colour. Matches
/// on the top-level capture category so related captures share a colour; an
/// out-of-range or uncategorised style falls back to the terminal default.
pub fn color_for(style: usize) -> Color {
    let name = HIGHLIGHT_NAMES.get(style).copied().unwrap_or("");
    match name.split('.').next().unwrap_or("") {
        "keyword" => Color::Magenta,
        "function" | "constructor" => Color::Blue,
        "type" => Color::Cyan,
        "string" | "escape" => Color::Green,
        "comment" => Color::DarkGrey,
        "constant" | "attribute" | "label" => Color::Yellow,
        "operator" | "punctuation" => Color::Grey,
        _ => Color::Reset, // variables, properties, anything else: default fg
    }
}

/// Build the highlight configuration for Rust. Returns `Err` (rather than
/// panicking) on an invalid grammar/query or an ABI mismatch, so the caller can
/// degrade to plain, uncoloured rendering.
pub fn build_config() -> Result<HighlightConfiguration, tree_sitter::QueryError> {
    let mut config = HighlightConfiguration::new(
        tree_sitter_rust::LANGUAGE.into(),
        "rust",
        tree_sitter_rust::HIGHLIGHTS_QUERY,
        tree_sitter_rust::INJECTIONS_QUERY,
        "", // tree-sitter-rust ships no locals query
    )?;
    config.configure(HIGHLIGHT_NAMES);
    Ok(config)
}

/// Highlight `source` and flatten the event stream into styled spans. Walks the
/// `HighlightStart`/`HighlightEnd` stack and emits one [`HlSpan`] per `Source`
/// range that is under an active highlight (unstyled ranges are skipped to keep
/// the list small). On a highlighter error the spans gathered so far are
/// returned — never a panic.
pub fn highlight_source(
    highlighter: &mut Highlighter,
    config: &HighlightConfiguration,
    source: &[u8],
) -> Vec<HlSpan> {
    let mut spans = Vec::new();
    let mut stack: Vec<usize> = Vec::new();

    let events = match highlighter.highlight(config, source, None, |_| None) {
        Ok(events) => events,
        Err(_) => return spans,
    };

    for event in events {
        match event {
            Ok(HighlightEvent::HighlightStart(Highlight(idx))) => stack.push(idx),
            Ok(HighlightEvent::HighlightEnd) => {
                stack.pop();
            }
            Ok(HighlightEvent::Source { start, end }) => {
                if let Some(&style) = stack.last() {
                    if start < end {
                        spans.push(HlSpan {
                            start_byte: start,
                            end_byte: end,
                            style,
                        });
                    }
                }
            }
            Err(_) => break,
        }
    }
    spans
}

/// Spawn the highlight worker on a dedicated OS thread. It owns the
/// `Highlighter` + `HighlightConfiguration` (both `Send` but `!Sync`, so they
/// must not be shared), receives [`ParseRequest`]s, coalesces to the newest
/// pending request, re-highlights the whole snapshot, and sends [`Highlights`]
/// back. The thread exits when `req_rx` is closed (all senders dropped) or the
/// grammar fails to load (highlighting silently disabled).
pub fn spawn(
    req_rx: std::sync::mpsc::Receiver<ParseRequest>,
    out_tx: tokio::sync::mpsc::UnboundedSender<Highlights>,
) {
    std::thread::spawn(move || {
        let config = match build_config() {
            Ok(config) => config,
            Err(_) => return, // ABI/query failure -> no highlighting, editor still works
        };
        let mut highlighter = Highlighter::new();

        while let Ok(mut req) = req_rx.recv() {
            // Coalesce: if the user typed faster than we parse, skip to the
            // newest queued snapshot instead of processing every keystroke.
            while let Ok(newer) = req_rx.try_recv() {
                req = newer;
            }
            let text = req.rope.to_string();
            let spans = highlight_source(&mut highlighter, &config, text.as_bytes());
            // Receiver gone (editor quitting) -> stop.
            if out_tx
                .send(Highlights {
                    revision: req.revision,
                    spans,
                })
                .is_err()
            {
                break;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style_of(name: &str) -> usize {
        HIGHLIGHT_NAMES.iter().position(|n| *n == name).unwrap()
    }

    #[test]
    fn config_builds_for_rust() {
        assert!(build_config().is_ok());
    }

    #[test]
    fn highlights_keyword_string_and_comment() {
        let config = build_config().unwrap();
        let mut hl = Highlighter::new();
        let src = "// hi\nfn main() { let s = \"x\"; }";
        let spans = highlight_source(&mut hl, &config, src.as_bytes());
        assert!(!spans.is_empty(), "expected some highlight spans");

        // Every span is a non-empty, in-bounds byte range.
        for s in &spans {
            assert!(s.start_byte < s.end_byte);
            assert!(s.end_byte <= src.len());
        }

        // The styles present should include keyword (`fn`/`let`), string, comment.
        let styles: Vec<usize> = spans.iter().map(|s| s.style).collect();
        assert!(styles.contains(&style_of("keyword")), "keyword missing");
        assert!(styles.contains(&style_of("string")), "string missing");
        assert!(styles.contains(&style_of("comment")), "comment missing");
    }

    #[test]
    fn keyword_span_covers_fn() {
        let config = build_config().unwrap();
        let mut hl = Highlighter::new();
        let src = "fn main() {}";
        let spans = highlight_source(&mut hl, &config, src.as_bytes());
        // Find a keyword span at byte 0..2 ("fn").
        let kw = style_of("keyword");
        assert!(
            spans
                .iter()
                .any(|s| s.style == kw && s.start_byte == 0 && s.end_byte == 2),
            "expected a keyword span covering `fn`, got {spans:?}"
        );
    }

    #[test]
    fn color_mapping_categories() {
        assert_eq!(color_for(style_of("keyword")), Color::Magenta);
        assert_eq!(color_for(style_of("string")), Color::Green);
        assert_eq!(color_for(style_of("comment")), Color::DarkGrey);
        assert_eq!(color_for(style_of("function")), Color::Blue);
        assert_eq!(color_for(style_of("type")), Color::Cyan);
        // Out-of-range falls back to the terminal default.
        assert_eq!(color_for(9999), Color::Reset);
    }
}
