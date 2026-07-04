//! Application-layer LSP orchestration.
//!
//! Owns the [`ServerSocket`] request handle, drives the document lifecycle
//! (`initialize` → `initialized` → `didOpen` → debounced `didChange` →
//! `shutdown`), and folds server-pushed events back into editor state. This is
//! the LSP analogue of MS3's [`crate::application::syntax::Syntax`] facade: the
//! main loop pumps edits/requests in and applies channel events out, while all
//! the async transport lives in [`crate::infrastructure::lsp_client`].
//!
//! Layering: policy (versioning, staleness, encoding, request dispatch) lives
//! here; the tokio process + async-lsp main loop live in infrastructure; the
//! domain model stays pure.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use async_lsp::lsp_types::{
    CompletionItem, CompletionParams, CompletionResponse, Diagnostic, DiagnosticSeverity,
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DocumentChangeOperation, DocumentChanges, DocumentFormattingParams, FormattingOptions,
    GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents, HoverParams,
    InitializedParams, MarkedString, OneOf, PartialResultParams, Position, PositionEncodingKind,
    ProgressParamsValue, RenameParams, TextDocumentContentChangeEvent, TextDocumentIdentifier,
    TextDocumentItem, TextDocumentPositionParams, TextEdit, Url, VersionedTextDocumentIdentifier,
    WorkDoneProgress, WorkDoneProgressParams, WorkspaceEdit,
};
use async_lsp::{LanguageServer, ServerSocket};
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::Duration;

use crate::application::editor_service::EditorService;
use crate::application::position::{self, PositionEncoding};
use crate::domain::editor_model::EditorModel;
use crate::domain::text_buffer::TextBuffer;
use crate::infrastructure::file_io::FileIO;
use crate::infrastructure::lsp_client::{self, LspEvent};
use crate::infrastructure::terminal_ui::{DiagSeverity, LineDiag};

/// An LSP feature request captured by a synchronous input handler, to be
/// dispatched by the async main loop after the keypress is fully handled.
/// Positions are editor cursor coordinates `(y, x)` (char columns), converted
/// to LSP positions at dispatch time via [`crate::application::position`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)] // variants wired up per feature in later sprints
pub enum LspRequest {
    /// `textDocument/hover` at `(y, x)` (the `K` command).
    Hover { y: usize, x: usize },
    /// `textDocument/definition` at `(y, x)` (the `gd` command).
    Definition { y: usize, x: usize },
    /// `textDocument/completion` at `(y, x)`.
    Completion { y: usize, x: usize },
    /// `textDocument/formatting` for the whole document (`:format`).
    Format,
    /// `textDocument/rename` of the symbol at `(y, x)` to `new_name` (`:rename`).
    Rename {
        y: usize,
        x: usize,
        new_name: String,
    },
}

/// What applying a server event did to the buffer, so the main loop can react
/// (re-sync the LSP document, re-highlight).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ApplyOutcome {
    /// Nothing that needs a re-sync (diagnostics stored, cursor moved, ...).
    Nothing,
    /// The active file changed (cross-file go-to-definition).
    FileSwitched,
    /// The buffer text was edited (format / rename applied).
    Edited,
}

/// Active completion popup state.
struct CompletionState {
    /// All items returned by the server.
    items: Vec<CompletionItem>,
    /// Indices into `items` matching the current prefix.
    filtered: Vec<usize>,
    /// Selected index into `filtered`.
    selected: usize,
    /// Where the completed word starts (the replace region is `anchor..cursor`).
    anchor_y: usize,
    anchor_x: usize,
}

/// Lifecycle state of the language server connection.
enum State {
    /// No server process (never opened a `.rs` file, or the binary is missing).
    Inactive,
    /// Process spawned, `initialize` in flight; edits are buffered.
    Initializing,
    /// Handshake complete; documents are synced and requests can be issued.
    Active,
}

/// The LSP client orchestrator.
pub struct Lsp {
    /// Channel the transport (and our spawned request tasks) push events on.
    event_tx: UnboundedSender<LspEvent>,
    /// Client -> server request handle (cloned per request task).
    server: Option<ServerSocket>,
    state: State,
    /// Negotiated position encoding (defaults to UTF-16 per the LSP spec).
    encoding: PositionEncoding,
    /// Monotonic LSP document version (distinct from `edit_revision`, which
    /// wraps and bumps on undo/redo/load).
    version: i32,
    /// URI of the document currently open in the server, if it is a `.rs` file.
    current_uri: Option<Url>,
    /// Whether an unsynced edit is pending (drives debounced `didChange`).
    dirty: bool,
    /// Latest diagnostics per document, tagged with the version they describe.
    diagnostics: HashMap<Url, (i32, Vec<Diagnostic>)>,
    /// Hover popup content (plain-text lines), shown until the next keypress.
    hover: Option<Vec<String>>,
    /// Active completion popup, if any.
    completion: Option<CompletionState>,
    /// Monotonic id tagging completion requests so stale results are dropped.
    completion_gen: u64,
    /// Title of an in-progress server work item (e.g. indexing), for the status
    /// line; `None` when idle.
    progress: Option<String>,
}

impl Lsp {
    /// Create an inactive orchestrator. The server process is spawned lazily on
    /// the first `.rs` file open (see [`Lsp::on_open`]).
    pub fn new(event_tx: UnboundedSender<LspEvent>) -> Self {
        Self {
            event_tx,
            server: None,
            state: State::Inactive,
            encoding: PositionEncoding::Utf16,
            version: 0,
            current_uri: None,
            dirty: false,
            diagnostics: HashMap::new(),
            hover: None,
            completion: None,
            completion_gen: 0,
            progress: None,
        }
    }

    /// A short status-line message for in-progress server work (e.g. indexing).
    pub fn progress_message(&self) -> Option<String> {
        self.progress.as_ref().map(|t| format!("LSP: {t}"))
    }

    /// React to the active buffer's file path changing (startup, `:e`, or a
    /// cross-file go-to-definition). Spawns + initializes the server on the
    /// first `.rs` file, and otherwise closes the old document and opens the new
    /// one. Non-Rust buffers are left unsynced (the server keeps running).
    pub fn on_open(&mut self, model: &EditorModel) {
        let path = model.get_filepath().cloned();
        let uri = path
            .as_deref()
            .filter(|p| p.ends_with(".rs"))
            .and_then(path_to_uri);

        match self.state {
            State::Inactive => {
                let Some(uri) = uri else { return };
                let root = path.as_deref().map(nearest_cargo_root).unwrap_or_else(cwd);
                // A spawn failure (binary missing) degrades: stay inactive and
                // keep editing without language intelligence.
                if let Ok(server) = lsp_client::spawn(&root, self.event_tx.clone()) {
                    self.server = Some(server.clone());
                    self.current_uri = Some(uri);
                    self.state = State::Initializing;
                    // Drive `initialize` off the main task; the response comes
                    // back as `LspEvent::Initialized`.
                    let tx = self.event_tx.clone();
                    let params = lsp_client::initialize_params(&root);
                    let mut s = server;
                    tokio::spawn(async move {
                        if let Ok(res) = s.initialize(params).await {
                            let _ = tx.send(LspEvent::Initialized(Box::new(res)));
                        }
                    });
                }
            }
            State::Initializing => {
                // `didOpen` fires once initialized; just track the target doc.
                self.current_uri = uri;
            }
            State::Active => {
                if let Some(old) = self.current_uri.take() {
                    if let Some(server) = self.server.as_mut() {
                        let _ = server.did_close(DidCloseTextDocumentParams {
                            text_document: TextDocumentIdentifier { uri: old },
                        });
                    }
                }
                if let Some(uri) = uri {
                    self.current_uri = Some(uri.clone());
                    self.version = 1;
                    self.send_did_open(model, uri);
                }
            }
        }
    }

    /// Mark that the buffer changed; the debounced `dispatch_change` will sync.
    pub fn note_change(&mut self) {
        self.dirty = true;
    }

    /// Send a full-document `didChange` if a change is pending (called when the
    /// shared edit debounce fires).
    pub fn dispatch_change(&mut self, model: &EditorModel) {
        if !matches!(self.state, State::Active) || !self.dirty {
            return;
        }
        let Some(uri) = self.current_uri.clone() else {
            self.dirty = false;
            return;
        };
        self.dirty = false;
        self.version += 1;
        if let Some(server) = self.server.as_mut() {
            let _ = server.did_change(DidChangeTextDocumentParams {
                text_document: VersionedTextDocumentIdentifier {
                    uri,
                    version: self.version,
                },
                content_changes: vec![TextDocumentContentChangeEvent {
                    range: None, // full-document sync
                    range_length: None,
                    text: model.buffer.raw_content(),
                }],
            });
        }
    }

    /// Dispatch an LSP feature request captured by an input handler. Each
    /// request runs on its own `tokio::spawn` task and its result comes back as
    /// an [`LspEvent`], so the main loop never blocks on the round-trip.
    pub fn dispatch_request(&mut self, req: LspRequest, model: &EditorModel) {
        if !matches!(self.state, State::Active) {
            return;
        }
        let (Some(uri), Some(server)) = (self.current_uri.clone(), self.server.clone()) else {
            return;
        };
        let tx = self.event_tx.clone();
        let enc = self.encoding;
        match req {
            LspRequest::Hover { y, x } => {
                let position = position::to_lsp(&model.buffer, enc, y, x);
                let params = HoverParams {
                    text_document_position_params: TextDocumentPositionParams {
                        text_document: TextDocumentIdentifier { uri },
                        position,
                    },
                    work_done_progress_params: WorkDoneProgressParams::default(),
                };
                let mut s = server;
                tokio::spawn(async move {
                    if let Ok(hover) = s.hover(params).await {
                        let _ = tx.send(LspEvent::Hover(hover.map(Box::new)));
                    }
                });
            }
            LspRequest::Definition { y, x } => {
                let position = position::to_lsp(&model.buffer, enc, y, x);
                let params = GotoDefinitionParams {
                    text_document_position_params: TextDocumentPositionParams {
                        text_document: TextDocumentIdentifier { uri },
                        position,
                    },
                    work_done_progress_params: WorkDoneProgressParams::default(),
                    partial_result_params: PartialResultParams::default(),
                };
                let mut s = server;
                tokio::spawn(async move {
                    if let Ok(resp) = s.definition(params).await {
                        let _ = tx.send(LspEvent::Definition(resp));
                    }
                });
            }
            LspRequest::Format => {
                let params = DocumentFormattingParams {
                    text_document: TextDocumentIdentifier { uri },
                    options: FormattingOptions {
                        tab_size: 4,
                        insert_spaces: true,
                        ..FormattingOptions::default()
                    },
                    work_done_progress_params: WorkDoneProgressParams::default(),
                };
                let mut s = server;
                tokio::spawn(async move {
                    if let Ok(edits) = s.formatting(params).await {
                        let _ = tx.send(LspEvent::Format(edits));
                    }
                });
            }
            LspRequest::Rename { y, x, new_name } => {
                let position = position::to_lsp(&model.buffer, enc, y, x);
                let params = RenameParams {
                    text_document_position: TextDocumentPositionParams {
                        text_document: TextDocumentIdentifier { uri },
                        position,
                    },
                    new_name,
                    work_done_progress_params: WorkDoneProgressParams::default(),
                };
                let mut s = server;
                tokio::spawn(async move {
                    if let Ok(edit) = s.rename(params).await {
                        let _ = tx.send(LspEvent::Rename(edit));
                    }
                });
            }
            LspRequest::Completion { y, x } => {
                self.completion_gen += 1;
                let generation = self.completion_gen;
                let position = position::to_lsp(&model.buffer, enc, y, x);
                let params = CompletionParams {
                    text_document_position: TextDocumentPositionParams {
                        text_document: TextDocumentIdentifier { uri },
                        position,
                    },
                    work_done_progress_params: WorkDoneProgressParams::default(),
                    partial_result_params: PartialResultParams::default(),
                    context: None,
                };
                let mut s = server;
                tokio::spawn(async move {
                    if let Ok(resp) = s.completion(params).await {
                        let _ = tx.send(LspEvent::Completion(generation, resp));
                    }
                });
            }
        }
    }

    /// Clear transient overlays (hover popup, later completion menu) — called on
    /// the next keypress so they don't linger.
    pub fn clear_transient(&mut self) {
        self.hover = None;
    }

    /// The current hover popup content (empty when no popup is showing).
    pub fn hover_lines(&self) -> &[String] {
        self.hover.as_deref().unwrap_or(&[])
    }

    // ---- completion --------------------------------------------------------

    /// Whether the completion popup is currently showing.
    pub fn completion_active(&self) -> bool {
        self.completion.is_some()
    }

    /// The filtered completion labels and the selected index, for rendering.
    pub fn completion_view(&self) -> Option<(Vec<String>, usize)> {
        let c = self.completion.as_ref()?;
        let labels = c
            .filtered
            .iter()
            .map(|&i| c.items[i].label.clone())
            .collect();
        Some((labels, c.selected))
    }

    /// Close the completion popup.
    pub fn close_completion(&mut self) {
        self.completion = None;
    }

    /// Move the completion selection (`+1` next, `-1` previous), wrapping.
    pub fn completion_move(&mut self, delta: isize) {
        if let Some(c) = &mut self.completion {
            if !c.filtered.is_empty() {
                let n = c.filtered.len() as isize;
                c.selected = ((c.selected as isize + delta).rem_euclid(n)) as usize;
            }
        }
    }

    /// Store a completion result, filtering by the current word prefix. Stale
    /// results (older generation) are dropped.
    fn apply_completion(
        &mut self,
        generation: u64,
        resp: Option<CompletionResponse>,
        model: &EditorModel,
    ) {
        if generation != self.completion_gen {
            return;
        }
        let items = match resp {
            Some(CompletionResponse::Array(v)) => v,
            Some(CompletionResponse::List(l)) => l.items,
            None => Vec::new(),
        };
        if items.is_empty() {
            self.completion = None;
            return;
        }
        let (prefix, anchor_x) = word_prefix(model, model.cursor_y, model.cursor_x);
        let mut state = CompletionState {
            items,
            filtered: Vec::new(),
            selected: 0,
            anchor_y: model.cursor_y,
            anchor_x,
        };
        filter_completion(&mut state, &prefix);
        self.completion = if state.filtered.is_empty() {
            None
        } else {
            Some(state)
        };
    }

    /// Re-filter the open completion popup against the word now under the cursor
    /// (called after each edit while the popup is active). Closes it if the
    /// cursor left the word or nothing matches.
    pub fn completion_refilter(&mut self, model: &EditorModel) {
        let Some(c) = &mut self.completion else {
            return;
        };
        if model.cursor_y != c.anchor_y || model.cursor_x < c.anchor_x {
            self.completion = None;
            return;
        }
        let line = model.buffer.line_text(c.anchor_y);
        let prefix: String = line
            .chars()
            .skip(c.anchor_x)
            .take(model.cursor_x - c.anchor_x)
            .collect();
        filter_completion(c, &prefix);
        if c.filtered.is_empty() {
            self.completion = None;
        }
    }

    /// Accept the selected completion: replace the word under the cursor with
    /// the item's insert text (one undo step) and place the cursor after it.
    /// Returns `true` if something was inserted.
    pub fn completion_accept<T: FileIO>(&mut self, svc: &mut EditorService<T>) -> bool {
        let Some(c) = self.completion.take() else {
            return false;
        };
        let Some(&item_idx) = c.filtered.get(c.selected) else {
            return false;
        };
        let item = &c.items[item_idx];
        let text = item
            .insert_text
            .clone()
            .unwrap_or_else(|| item.label.clone());
        let start = svc
            .editor_model
            .buffer
            .cursor_to_char(c.anchor_y, c.anchor_x);
        let end = svc
            .editor_model
            .buffer
            .cursor_to_char(svc.editor_model.cursor_y, svc.editor_model.cursor_x);
        let inserted_chars = text.chars().count();
        if svc.editor_model.apply_lsp_edits(vec![(start, end, text)]) {
            // Place the cursor just after the inserted text (single-line items).
            svc.editor_model
                .goto(c.anchor_y, c.anchor_x + inserted_chars);
            true
        } else {
            false
        }
    }

    /// Fold a server event into editor state, returning what it did so the main
    /// loop can re-sync the LSP document / re-highlight as needed.
    pub fn apply<T: FileIO>(
        &mut self,
        event: LspEvent,
        svc: &mut EditorService<T>,
        status: &mut String,
    ) -> ApplyOutcome {
        match event {
            LspEvent::Initialized(res) => {
                self.encoding = match res.capabilities.position_encoding {
                    Some(k) if k == PositionEncodingKind::UTF8 => PositionEncoding::Utf8,
                    _ => PositionEncoding::Utf16,
                };
                self.state = State::Active;
                if let Some(server) = self.server.as_mut() {
                    let _ = server.initialized(InitializedParams {});
                }
                if let Some(uri) = self.current_uri.clone() {
                    self.version = 1;
                    self.send_did_open(&svc.editor_model, uri);
                }
                *status = "LSP: ready".to_string();
                ApplyOutcome::Nothing
            }
            LspEvent::PublishDiagnostics(params) => {
                let incoming = params.version.unwrap_or(self.version);
                let keep = self
                    .diagnostics
                    .get(&params.uri)
                    .is_none_or(|(v, _)| incoming >= *v);
                if keep {
                    self.diagnostics
                        .insert(params.uri, (incoming, params.diagnostics));
                }
                ApplyOutcome::Nothing
            }
            LspEvent::Hover(hover) => {
                self.hover = hover.map(|h| hover_to_lines(&h)).filter(|v| !v.is_empty());
                ApplyOutcome::Nothing
            }
            LspEvent::Definition(resp) => self.apply_definition(resp, svc),
            LspEvent::Format(edits) => self.apply_edits(edits, svc, status),
            LspEvent::Rename(edit) => self.apply_rename(edit, svc, status),
            LspEvent::Completion(generation, resp) => {
                self.apply_completion(generation, resp, &svc.editor_model);
                ApplyOutcome::Nothing
            }
            LspEvent::ShowMessage(params) => {
                *status = format!("LSP: {}", params.message);
                ApplyOutcome::Nothing
            }
            LspEvent::Progress(params) => {
                let ProgressParamsValue::WorkDone(wd) = params.value;
                match wd {
                    WorkDoneProgress::Begin(b) => self.progress = Some(b.title),
                    WorkDoneProgress::End(_) => self.progress = None,
                    WorkDoneProgress::Report(_) => {}
                }
                ApplyOutcome::Nothing
            }
            LspEvent::LogMessage(_) => ApplyOutcome::Nothing,
        }
    }

    /// Navigate to a `textDocument/definition` result: move the cursor, opening
    /// the target file first if it differs.
    fn apply_definition<T: FileIO>(
        &mut self,
        resp: Option<GotoDefinitionResponse>,
        svc: &mut EditorService<T>,
    ) -> ApplyOutcome {
        let Some((uri, pos)) = first_location(resp) else {
            return ApplyOutcome::Nothing;
        };
        // Compare by canonical URI so we don't needlessly reload the current
        // document (which would clear its undo history).
        let current = svc.editor_model.get_filepath().and_then(|p| path_to_uri(p));
        let mut outcome = ApplyOutcome::Nothing;
        if current.as_ref() != Some(&uri) {
            if let Some(path) = uri
                .to_file_path()
                .ok()
                .and_then(|p| p.to_str().map(String::from))
            {
                if svc.open_file(&path).is_ok() {
                    outcome = ApplyOutcome::FileSwitched;
                }
            }
        }
        // Convert in the target buffer's coordinates (now loaded).
        let (y, x) = position::from_lsp(&svc.editor_model.buffer, self.encoding, pos);
        svc.editor_model.goto(y, x);
        outcome
    }

    /// Apply a batch of LSP `TextEdit`s (from formatting/rename) to the current
    /// buffer as a single undo step, then mark the document dirty for the next
    /// `didChange`.
    fn apply_edits<T: FileIO>(
        &mut self,
        edits: Option<Vec<TextEdit>>,
        svc: &mut EditorService<T>,
        status: &mut String,
    ) -> ApplyOutcome {
        let Some(edits) = edits.filter(|e| !e.is_empty()) else {
            *status = "LSP: nothing to change".to_string();
            return ApplyOutcome::Nothing;
        };
        let buffer = &svc.editor_model.buffer;
        // Convert each LSP range to whole-buffer char offsets.
        let char_edits: Vec<(usize, usize, String)> = edits
            .into_iter()
            .map(|e| {
                let (sy, sx) = position::from_lsp(buffer, self.encoding, e.range.start);
                let (ey, ex) = position::from_lsp(buffer, self.encoding, e.range.end);
                (
                    buffer.cursor_to_char(sy, sx),
                    buffer.cursor_to_char(ey, ex),
                    e.new_text,
                )
            })
            .collect();
        if svc.editor_model.apply_lsp_edits(char_edits) {
            self.note_change();
            ApplyOutcome::Edited
        } else {
            ApplyOutcome::Nothing
        }
    }

    /// Apply a rename `WorkspaceEdit`: edits to the current buffer are applied as
    /// one undo step; edits in other files are reported but not applied.
    fn apply_rename<T: FileIO>(
        &mut self,
        edit: Option<WorkspaceEdit>,
        svc: &mut EditorService<T>,
        status: &mut String,
    ) -> ApplyOutcome {
        let Some(edit) = edit else {
            *status = "LSP: rename returned nothing".to_string();
            return ApplyOutcome::Nothing;
        };
        let Some(uri) = self.current_uri.clone() else {
            return ApplyOutcome::Nothing;
        };
        let (mine, others) = workspace_edits_for(edit, &uri);
        if mine.is_empty() {
            *status = if others > 0 {
                format!("LSP: rename only touches other files ({others} edits, not applied)")
            } else {
                "LSP: nothing to rename".to_string()
            };
            return ApplyOutcome::Nothing;
        }
        let outcome = self.apply_edits(Some(mine), svc, status);
        *status = if others > 0 {
            format!("Renamed here; {others} edit(s) in other files not applied")
        } else {
            "Renamed".to_string()
        };
        outcome
    }

    /// Diagnostics for the currently open document (empty if none, or the
    /// active buffer is not a Rust file). Consumed by the renderer.
    #[allow(dead_code)] // wired up in the diagnostics-rendering sprint
    pub fn current_diagnostics(&self) -> &[Diagnostic] {
        match &self.current_uri {
            Some(uri) => self
                .diagnostics
                .get(uri)
                .map_or(&[][..], |(_, d)| d.as_slice()),
            None => &[],
        }
    }

    /// The negotiated position encoding (for converting LSP positions).
    #[allow(dead_code)] // used by the diagnostic-view helpers below
    pub fn encoding(&self) -> PositionEncoding {
        self.encoding
    }

    /// Project the current document's diagnostics onto logical lines (char
    /// columns) for the renderer. Multi-line diagnostics are split per line.
    pub fn line_diagnostics(&self, buffer: &TextBuffer) -> Vec<LineDiag> {
        let mut out = Vec::new();
        for d in self.current_diagnostics() {
            let severity = map_severity(d.severity);
            let (sy, sx) = position::from_lsp(buffer, self.encoding, d.range.start);
            let (ey, ex) = position::from_lsp(buffer, self.encoding, d.range.end);
            for line in sy..=ey.max(sy) {
                let start_col = if line == sy { sx } else { 0 };
                let mut end_col = if line == ey {
                    ex
                } else {
                    buffer.line_char_len(line)
                };
                // Ensure a zero-width range still underlines one column.
                if end_col <= start_col {
                    end_col = start_col + 1;
                }
                out.push(LineDiag {
                    line,
                    start_col,
                    end_col,
                    severity,
                });
            }
        }
        out
    }

    /// The message of the diagnostic under cursor `(y, x)`, if any (shown on the
    /// status line). Falls back to a count summary via [`Lsp::diagnostic_summary`].
    pub fn diagnostic_at(&self, buffer: &TextBuffer, y: usize, x: usize) -> Option<String> {
        for d in self.current_diagnostics() {
            let start = position::from_lsp(buffer, self.encoding, d.range.start);
            let end = position::from_lsp(buffer, self.encoding, d.range.end);
            if start <= (y, x) && (y, x) <= end {
                let sev = map_severity(d.severity);
                let first = d.message.lines().next().unwrap_or(&d.message);
                return Some(format!("{}: {}", severity_label(sev), first));
            }
        }
        None
    }

    /// A short `"N errors, M warnings"` summary of the current document's
    /// diagnostics, or empty when there are none.
    pub fn diagnostic_summary(&self) -> String {
        let (mut errors, mut warnings) = (0usize, 0usize);
        for d in self.current_diagnostics() {
            match map_severity(d.severity) {
                DiagSeverity::Error => errors += 1,
                DiagSeverity::Warning => warnings += 1,
                _ => {}
            }
        }
        match (errors, warnings) {
            (0, 0) => String::new(),
            (e, 0) => format!("{e} error(s)"),
            (0, w) => format!("{w} warning(s)"),
            (e, w) => format!("{e} error(s), {w} warning(s)"),
        }
    }

    /// Best-effort graceful shutdown that never hangs the editor's exit.
    pub async fn shutdown(&mut self) {
        if let Some(mut server) = self.server.take() {
            let _ = tokio::time::timeout(Duration::from_millis(500), server.shutdown(())).await;
            let _ = server.exit(());
        }
        self.state = State::Inactive;
    }

    fn send_did_open(&mut self, model: &EditorModel, uri: Url) {
        self.dirty = false;
        if let Some(server) = self.server.as_mut() {
            let _ = server.did_open(DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri,
                    language_id: "rust".to_string(),
                    version: self.version,
                    // Send the raw rope content (trailing-newline invariant) so
                    // LSP line/char positions map straight onto the buffer.
                    text: model.buffer.raw_content(),
                },
            });
        }
    }
}

/// Nearest ancestor directory of `path` that contains a `Cargo.toml`, so
/// rust-analyzer roots at the crate/workspace rather than a bare file.
fn nearest_cargo_root(path: &str) -> PathBuf {
    let abs = std::fs::canonicalize(Path::new(path)).unwrap_or_else(|_| cwd().join(path));
    let mut dir = abs.parent();
    while let Some(d) = dir {
        if d.join("Cargo.toml").exists() {
            return d.to_path_buf();
        }
        dir = d.parent();
    }
    cwd()
}

fn path_to_uri(path: &str) -> Option<Url> {
    let abs = std::fs::canonicalize(Path::new(path))
        .ok()
        .unwrap_or_else(|| cwd().join(path));
    Url::from_file_path(abs).ok()
}

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Map an LSP diagnostic severity to the renderer's severity (unset defaults to
/// Error so unknown-severity diagnostics stay visible).
fn map_severity(s: Option<DiagnosticSeverity>) -> DiagSeverity {
    match s {
        Some(x) if x == DiagnosticSeverity::WARNING => DiagSeverity::Warning,
        Some(x) if x == DiagnosticSeverity::INFORMATION => DiagSeverity::Info,
        Some(x) if x == DiagnosticSeverity::HINT => DiagSeverity::Hint,
        _ => DiagSeverity::Error,
    }
}

/// The identifier word immediately before cursor `(y, x)` and the char column
/// where it starts. An identifier char is alphanumeric or `_`.
fn word_prefix(model: &EditorModel, y: usize, x: usize) -> (String, usize) {
    let line = model.buffer.line_text(y);
    let chars: Vec<char> = line.chars().collect();
    let mut start = x.min(chars.len());
    while start > 0 && is_ident_char(chars[start - 1]) {
        start -= 1;
    }
    let prefix: String = chars[start..x.min(chars.len())].iter().collect();
    (prefix, start)
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Recompute `state.filtered` as the items whose label starts with `prefix`
/// (case-insensitive), resetting the selection to the top.
fn filter_completion(state: &mut CompletionState, prefix: &str) {
    let pl = prefix.to_lowercase();
    state.filtered = state
        .items
        .iter()
        .enumerate()
        .filter(|(_, it)| it.label.to_lowercase().starts_with(&pl))
        .map(|(i, _)| i)
        .collect();
    state.selected = 0;
}

/// Split a rename `WorkspaceEdit` into `(edits for `uri`, count of edits in
/// other files)`. Handles both the `changes` map and `documentChanges` forms.
fn workspace_edits_for(edit: WorkspaceEdit, uri: &Url) -> (Vec<TextEdit>, usize) {
    let mut mine = Vec::new();
    let mut others = 0usize;
    let take = |edits: Vec<TextEdit>, mine: &mut Vec<TextEdit>, others: &mut usize, same: bool| {
        if same {
            mine.extend(edits);
        } else {
            *others += edits.len();
        }
    };
    if let Some(changes) = edit.changes {
        for (u, edits) in changes {
            take(edits, &mut mine, &mut others, &u == uri);
        }
    }
    if let Some(dc) = edit.document_changes {
        let tds = match dc {
            DocumentChanges::Edits(tds) => tds,
            DocumentChanges::Operations(ops) => ops
                .into_iter()
                .filter_map(|op| match op {
                    DocumentChangeOperation::Edit(td) => Some(td),
                    // Create/rename/delete-file operations are unsupported.
                    DocumentChangeOperation::Op(_) => None,
                })
                .collect(),
        };
        for td in tds {
            let same = &td.text_document.uri == uri;
            let edits: Vec<TextEdit> = td
                .edits
                .into_iter()
                .map(|e| match e {
                    OneOf::Left(te) => te,
                    OneOf::Right(annotated) => annotated.text_edit,
                })
                .collect();
            take(edits, &mut mine, &mut others, same);
        }
    }
    (mine, others)
}

/// The first target of a go-to-definition response as `(uri, position)`.
fn first_location(resp: Option<GotoDefinitionResponse>) -> Option<(Url, Position)> {
    match resp? {
        GotoDefinitionResponse::Scalar(l) => Some((l.uri, l.range.start)),
        GotoDefinitionResponse::Array(v) => v.into_iter().next().map(|l| (l.uri, l.range.start)),
        GotoDefinitionResponse::Link(v) => v
            .into_iter()
            .next()
            .map(|l| (l.target_uri, l.target_selection_range.start)),
    }
}

/// Flatten an LSP hover response to plain-text lines for the popup.
fn hover_to_lines(h: &Hover) -> Vec<String> {
    let text = match &h.contents {
        HoverContents::Markup(m) => m.value.clone(),
        HoverContents::Scalar(s) => marked_string_text(s),
        HoverContents::Array(a) => a
            .iter()
            .map(marked_string_text)
            .collect::<Vec<_>>()
            .join("\n"),
    };
    text.lines()
        .map(|l| l.to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

fn marked_string_text(s: &MarkedString) -> String {
    match s {
        MarkedString::String(s) => s.clone(),
        MarkedString::LanguageString(ls) => ls.value.clone(),
    }
}

fn severity_label(s: DiagSeverity) -> &'static str {
    match s {
        DiagSeverity::Error => "error",
        DiagSeverity::Warning => "warning",
        DiagSeverity::Info => "info",
        DiagSeverity::Hint => "hint",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::file_io::LocalFileIO;
    use async_lsp::lsp_types::{
        Diagnostic, DiagnosticSeverity, Position, PublishDiagnosticsParams, Range,
    };

    fn diag(line: u32, s: u32, e: u32, sev: DiagnosticSeverity) -> Diagnostic {
        Diagnostic {
            range: Range::new(Position::new(line, s), Position::new(line, e)),
            severity: Some(sev),
            message: "boom".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn diagnostics_staleness_and_projection() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut lsp = Lsp::new(tx);
        let uri = Url::parse("file:///t.rs").unwrap();
        lsp.current_uri = Some(uri.clone());
        lsp.encoding = PositionEncoding::Utf16;

        let mut svc = EditorService::new(LocalFileIO);
        let mut status = String::new();

        // Version 2 diagnostics land...
        lsp.apply(
            LspEvent::PublishDiagnostics(PublishDiagnosticsParams {
                uri: uri.clone(),
                diagnostics: vec![diag(1, 4, 7, DiagnosticSeverity::ERROR)],
                version: Some(2),
            }),
            &mut svc,
            &mut status,
        );
        // ...and a stale version-1 update is dropped.
        lsp.apply(
            LspEvent::PublishDiagnostics(PublishDiagnosticsParams {
                uri: uri.clone(),
                diagnostics: vec![],
                version: Some(1),
            }),
            &mut svc,
            &mut status,
        );
        assert_eq!(lsp.current_diagnostics().len(), 1);

        // Projection onto a buffer -> char-column line ranges.
        let mut buffer = TextBuffer::new();
        buffer.set_content("fn main() {}\n  let x = 1;");
        let lines = lsp.line_diagnostics(&buffer);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].line, 1);
        assert_eq!((lines[0].start_col, lines[0].end_col), (4, 7));
        assert_eq!(lines[0].severity, DiagSeverity::Error);

        // Cursor-on-diagnostic message vs. none, and summary.
        assert!(lsp.diagnostic_at(&buffer, 1, 5).is_some());
        assert!(lsp.diagnostic_at(&buffer, 0, 0).is_none());
        assert_eq!(lsp.diagnostic_summary(), "1 error(s)");
    }

    /// Full orchestrator lifecycle against a real `rust-analyzer`, over a tiny
    /// temporary crate so indexing is fast: open a broken file, drive the
    /// handshake to Active, and confirm diagnostics land in the store. Ignored
    /// by default; run with `cargo test -- --ignored lifecycle`.
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "spawns rust-analyzer"]
    async fn lifecycle_diagnostics_flow() {
        let dir = std::env::temp_dir().join(format!("rim_lsp_ms4_{}", std::process::id()));
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname=\"t\"\nversion=\"0.0.0\"\nedition=\"2021\"\n",
        )
        .unwrap();
        // A parse error yields fast native diagnostics (no cargo check needed).
        let main_rs = src.join("main.rs");
        std::fs::write(&main_rs, "fn main() { let x = ; }\n").unwrap();

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut lsp = Lsp::new(tx);
        let mut svc = EditorService::new(LocalFileIO);
        svc.open_file(main_rs.to_str().unwrap()).unwrap();
        lsp.on_open(&svc.editor_model);

        let mut status = String::new();
        let got = tokio::time::timeout(Duration::from_secs(60), async {
            while let Some(ev) = rx.recv().await {
                lsp.apply(ev, &mut svc, &mut status);
                if !lsp.current_diagnostics().is_empty() {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false);

        lsp.shutdown().await;
        let _ = std::fs::remove_dir_all(&dir);
        assert!(got, "expected diagnostics for the broken file");
    }

    /// Hover round-trip: open a tiny crate, drive to Active, and request hover
    /// on `var` until rust-analyzer returns its inferred type.
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "spawns rust-analyzer"]
    async fn hover_flow() {
        let dir = std::env::temp_dir().join(format!("rim_lsp_ms4_hover_{}", std::process::id()));
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname=\"t\"\nversion=\"0.0.0\"\nedition=\"2021\"\n",
        )
        .unwrap();
        let main_rs = src.join("main.rs");
        std::fs::write(&main_rs, "fn main() { let var = 1; }\n").unwrap();

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut lsp = Lsp::new(tx);
        let mut svc = EditorService::new(LocalFileIO);
        svc.open_file(main_rs.to_str().unwrap()).unwrap();
        lsp.on_open(&svc.editor_model);

        let mut status = String::new();
        let got = tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                // Once initialized, keep requesting hover on `var` (col 17)
                // until indexing makes the type available.
                if matches!(lsp.state, State::Active) && lsp.hover_lines().is_empty() {
                    lsp.dispatch_request(LspRequest::Hover { y: 0, x: 17 }, &svc.editor_model);
                }
                let Some(ev) = rx.recv().await else {
                    return false;
                };
                lsp.apply(ev, &mut svc, &mut status);
                if !lsp.hover_lines().is_empty() {
                    return true;
                }
            }
        })
        .await
        .unwrap_or(false);

        let text = lsp.hover_lines().join("\n");
        lsp.shutdown().await;
        let _ = std::fs::remove_dir_all(&dir);
        assert!(got, "expected a hover result for `var`");
        assert!(text.contains("i32"), "hover should show the type: {text}");
    }

    /// Go-to-definition round-trip: `gd` on a call jumps to the function's
    /// definition line.
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "spawns rust-analyzer"]
    async fn goto_definition_flow() {
        let dir = std::env::temp_dir().join(format!("rim_lsp_ms4_gd_{}", std::process::id()));
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname=\"t\"\nversion=\"0.0.0\"\nedition=\"2021\"\n",
        )
        .unwrap();
        let main_rs = src.join("main.rs");
        // `helper` is defined on line 0 and called on line 2.
        std::fs::write(
            &main_rs,
            "fn helper() -> i32 { 1 }\nfn main() {\n    let x = helper();\n}\n",
        )
        .unwrap();

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut lsp = Lsp::new(tx);
        let mut svc = EditorService::new(LocalFileIO);
        svc.open_file(main_rs.to_str().unwrap()).unwrap();
        svc.editor_model.goto(2, 13); // on the `helper` call
        lsp.on_open(&svc.editor_model);

        let mut status = String::new();
        let jumped = tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                // Keep requesting until name resolution jumps us off line 2.
                if matches!(lsp.state, State::Active) && svc.editor_model.cursor_y == 2 {
                    lsp.dispatch_request(LspRequest::Definition { y: 2, x: 13 }, &svc.editor_model);
                }
                let Some(ev) = rx.recv().await else {
                    return false;
                };
                lsp.apply(ev, &mut svc, &mut status);
                if svc.editor_model.cursor_y == 0 {
                    return true;
                }
            }
        })
        .await
        .unwrap_or(false);

        lsp.shutdown().await;
        let _ = std::fs::remove_dir_all(&dir);
        assert!(jumped, "gd should jump to the definition on line 0");
    }

    /// Format round-trip: `:format` reformats the buffer and a single `undo`
    /// reverts the whole change.
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "spawns rust-analyzer + rustfmt"]
    async fn format_flow() {
        let dir = std::env::temp_dir().join(format!("rim_lsp_ms4_fmt_{}", std::process::id()));
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname=\"t\"\nversion=\"0.0.0\"\nedition=\"2021\"\n",
        )
        .unwrap();
        let main_rs = src.join("main.rs");
        std::fs::write(&main_rs, "fn main(){let x=1;let _=x;}\n").unwrap();

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut lsp = Lsp::new(tx);
        let mut svc = EditorService::new(LocalFileIO);
        svc.open_file(main_rs.to_str().unwrap()).unwrap();
        lsp.on_open(&svc.editor_model);
        let original = svc.editor_model.get_content();

        let mut status = String::new();
        let edited = tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                if matches!(lsp.state, State::Active) && svc.editor_model.get_content() == original
                {
                    lsp.dispatch_request(LspRequest::Format, &svc.editor_model);
                }
                let Some(ev) = rx.recv().await else {
                    return false;
                };
                if lsp.apply(ev, &mut svc, &mut status) == ApplyOutcome::Edited {
                    return true;
                }
            }
        })
        .await
        .unwrap_or(false);

        let formatted = svc.editor_model.get_content();
        svc.editor_model.undo();
        let reverted = svc.editor_model.get_content();

        lsp.shutdown().await;
        let _ = std::fs::remove_dir_all(&dir);
        assert!(edited, "format should edit the buffer");
        assert_ne!(formatted, original, "format should change the text");
        assert!(formatted.contains("let x = 1;"), "should be reformatted");
        assert_eq!(reverted, original, "one undo should revert the format");
    }

    /// Rename round-trip: rename a symbol, all in-file occurrences change, and a
    /// single `undo` reverts.
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "spawns rust-analyzer"]
    async fn rename_flow() {
        let dir = std::env::temp_dir().join(format!("rim_lsp_ms4_rename_{}", std::process::id()));
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname=\"t\"\nversion=\"0.0.0\"\nedition=\"2021\"\n",
        )
        .unwrap();
        let main_rs = src.join("main.rs");
        std::fs::write(
            &main_rs,
            "fn old_name() -> i32 { 1 }\nfn main() {\n    let _ = old_name();\n}\n",
        )
        .unwrap();

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut lsp = Lsp::new(tx);
        let mut svc = EditorService::new(LocalFileIO);
        svc.open_file(main_rs.to_str().unwrap()).unwrap();
        svc.editor_model.goto(0, 3); // on the `old_name` definition
        lsp.on_open(&svc.editor_model);
        let original = svc.editor_model.get_content();

        let mut status = String::new();
        let renamed = tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                if matches!(lsp.state, State::Active) && svc.editor_model.get_content() == original
                {
                    lsp.dispatch_request(
                        LspRequest::Rename {
                            y: 0,
                            x: 3,
                            new_name: "renamed".to_string(),
                        },
                        &svc.editor_model,
                    );
                }
                let Some(ev) = rx.recv().await else {
                    return false;
                };
                if lsp.apply(ev, &mut svc, &mut status) == ApplyOutcome::Edited {
                    return true;
                }
            }
        })
        .await
        .unwrap_or(false);

        let after = svc.editor_model.get_content();
        svc.editor_model.undo();
        let reverted = svc.editor_model.get_content();

        lsp.shutdown().await;
        let _ = std::fs::remove_dir_all(&dir);
        assert!(renamed, "rename should edit the buffer");
        assert!(after.contains("fn renamed"), "definition renamed: {after}");
        assert!(after.contains("renamed()"), "call site renamed: {after}");
        assert!(!after.contains("old_name"), "no old name left: {after}");
        assert_eq!(reverted, original, "one undo should revert the rename");
    }

    /// Completion round-trip: request completion after a partial identifier,
    /// confirm the local variable is offered, then accept it.
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "spawns rust-analyzer"]
    async fn completion_flow() {
        let dir = std::env::temp_dir().join(format!("rim_lsp_ms4_cmp_{}", std::process::id()));
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname=\"t\"\nversion=\"0.0.0\"\nedition=\"2021\"\n",
        )
        .unwrap();
        let main_rs = src.join("main.rs");
        // `va` on line 2 should complete to the local `value`.
        std::fs::write(
            &main_rs,
            "fn main() {\n    let value = 1;\n    let _ = va;\n}\n",
        )
        .unwrap();

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut lsp = Lsp::new(tx);
        let mut svc = EditorService::new(LocalFileIO);
        svc.open_file(main_rs.to_str().unwrap()).unwrap();
        svc.editor_model.goto(2, 14); // just after `va`
        lsp.on_open(&svc.editor_model);

        let mut status = String::new();
        // Keep at most one completion request in flight, retrying only after a
        // result arrives, so the generation id doesn't churn and drop results
        // as stale during indexing.
        let has_value = tokio::time::timeout(Duration::from_secs(60), async {
            let mut inflight = false;
            loop {
                if matches!(lsp.state, State::Active) && !inflight && !lsp.completion_active() {
                    lsp.dispatch_request(LspRequest::Completion { y: 2, x: 14 }, &svc.editor_model);
                    inflight = true;
                }
                let Some(ev) = rx.recv().await else {
                    return false;
                };
                let was_completion = matches!(&ev, LspEvent::Completion(..));
                lsp.apply(ev, &mut svc, &mut status);
                if let Some((labels, _)) = lsp.completion_view() {
                    if labels.iter().any(|l| l == "value") {
                        return true;
                    }
                }
                if was_completion {
                    inflight = false;
                }
            }
        })
        .await
        .unwrap_or(false);

        // Navigate to `value` and accept it.
        let mut accepted = false;
        if has_value {
            for _ in 0..64 {
                let (labels, sel) = lsp.completion_view().unwrap();
                if labels[sel] == "value" {
                    lsp.completion_accept(&mut svc);
                    accepted = true;
                    break;
                }
                lsp.completion_move(1);
            }
        }
        let after = svc.editor_model.get_content();

        lsp.shutdown().await;
        let _ = std::fs::remove_dir_all(&dir);
        assert!(has_value, "completion should offer the local `value`");
        assert!(accepted, "should be able to select `value`");
        assert!(
            after.contains("let _ = value;"),
            "accepting inserts the completion: {after}"
        );
    }
}
