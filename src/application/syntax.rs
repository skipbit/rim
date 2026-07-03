//! Syntax-highlighting orchestration (application layer).
//!
//! `Syntax` is the main-task handle to the background highlight worker
//! ([`crate::infrastructure::syntax_worker`]). It keeps the domain pure: the
//! editor model exposes only an `edit_revision` counter and a cheap rope
//! `snapshot`, and this type turns "the text changed" into debounced parse
//! requests, then holds the latest returned spans for the renderer.
//!
//! Flow: the event loop compares `edit_revision` before/after each key; on a
//! change it calls [`Syntax::note_change`] and arms a debounce timer. When the
//! timer fires it calls [`Syntax::dispatch`], sending the newest snapshot to
//! the worker. Worker results come back over a tokio channel and are installed
//! via [`Syntax::apply`]; [`Syntax::spans`] feeds the renderer.

use crate::infrastructure::syntax_worker::{self, Highlights, HlSpan, ParseRequest};
use ropey::Rope;
use tokio::sync::mpsc::UnboundedSender;

pub struct Syntax {
    req_tx: std::sync::mpsc::Sender<ParseRequest>,
    /// The newest snapshot awaiting dispatch (replaced on every edit during a
    /// debounce window, so only the latest is ever parsed).
    pending: Option<ParseRequest>,
    /// The most recent highlights installed, shown until newer ones arrive.
    latest: Option<Highlights>,
}

impl Syntax {
    /// Start the worker thread and return a handle. `out_tx` is the channel the
    /// worker reports [`Highlights`] on; the event loop owns the receiver.
    pub fn spawn(out_tx: UnboundedSender<Highlights>) -> Self {
        let (req_tx, req_rx) = std::sync::mpsc::channel();
        syntax_worker::spawn(req_rx, out_tx);
        Self {
            req_tx,
            pending: None,
            latest: None,
        }
    }

    /// Record a snapshot to be re-highlighted after the debounce window. Cheap:
    /// `rope` is an O(1) copy-on-write clone.
    pub fn note_change(&mut self, revision: u64, rope: Rope) {
        self.pending = Some(ParseRequest { revision, rope });
    }

    /// Send the pending snapshot to the worker (called when the debounce timer
    /// fires). No-op if nothing is pending.
    pub fn dispatch(&mut self) {
        if let Some(req) = self.pending.take() {
            let _ = self.req_tx.send(req);
        }
    }

    /// Request an immediate highlight, bypassing debounce — used for the first
    /// parse when a file is opened.
    pub fn request_now(&mut self, revision: u64, rope: Rope) {
        let _ = self.req_tx.send(ParseRequest { revision, rope });
    }

    /// Install a worker result, keeping the newest by revision. Older results
    /// arriving late (out of order) are dropped, and the previous spans stay on
    /// screen until a newer set lands (no flash of unstyled text).
    pub fn apply(&mut self, highlights: Highlights) {
        let is_newer = self
            .latest
            .as_ref()
            .is_none_or(|cur| highlights.revision >= cur.revision);
        if is_newer {
            self.latest = Some(highlights);
        }
    }

    /// The spans to render (empty until the first result arrives).
    pub fn spans(&self) -> &[HlSpan] {
        self.latest.as_ref().map_or(&[], |h| h.spans.as_slice())
    }

    /// Revision of the currently shown highlights (0 if none yet).
    pub fn revision(&self) -> u64 {
        self.latest.as_ref().map_or(0, |h| h.revision)
    }
}
