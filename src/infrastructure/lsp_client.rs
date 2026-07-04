//! LSP transport.
//!
//! Spawns the language server process (`rust-analyzer`) and runs the
//! [`async_lsp`] main loop as a `tokio` task, bridging **server -> client**
//! messages onto a `tokio` channel consumed by the editor's `select!` loop
//! (mirroring how the syntax worker feeds highlights back in MS3). The returned
//! [`ServerSocket`] is the **client -> server** handle the application-layer
//! orchestrator uses to issue requests (`initialize`, `hover`, ...).
//!
//! Unlike the MS3 syntax worker (a blocking `std::thread`), this subsystem is
//! async and lives *on* the tokio runtime: `async-lsp` is tower-based and the
//! child's stdio is driven cooperatively by the `current_thread` runtime.

use std::future::ready;
use std::ops::ControlFlow;
use std::path::Path;
use std::process::Stdio;

use async_lsp::lsp_types::{
    ClientCapabilities, CompletionResponse, ConfigurationParams, GeneralClientCapabilities,
    GotoDefinitionResponse, Hover, InitializeParams, InitializeResult, LogMessageParams, OneOf,
    PositionEncodingKind, ProgressParams, PublishDiagnosticsParams, RegistrationParams,
    ShowMessageParams, TextEdit, Url, WindowClientCapabilities, WorkDoneProgressCreateParams,
    WorkspaceEdit, WorkspaceFolder,
};
use async_lsp::router::Router;
use async_lsp::{LanguageClient, MainLoop, ResponseError, ServerSocket};
use futures::future::BoxFuture;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

/// A message from the language server delivered to the editor's event loop.
///
/// Server-pushed notifications are forwarded by the [`Router`] below; request
/// *results* (hover, definition, ...) are pushed by the orchestrator's spawned
/// request tasks in later sprints. All variants ride the same channel so the
/// `select!` loop applies them uniformly.
#[derive(Debug)]
#[allow(dead_code)] // result variants added per feature in later sprints
pub enum LspEvent {
    /// The `initialize` response (pushed by the orchestrator's init task).
    Initialized(Box<InitializeResult>),
    /// A `textDocument/hover` result (pushed by a request task).
    Hover(Option<Box<Hover>>),
    /// A `textDocument/definition` result (pushed by a request task).
    Definition(Option<GotoDefinitionResponse>),
    /// A `textDocument/formatting` result (edits to apply as one undo step).
    Format(Option<Vec<TextEdit>>),
    /// A `textDocument/rename` result (a workspace edit).
    Rename(Option<WorkspaceEdit>),
    /// A `textDocument/completion` result tagged with the generation id that
    /// requested it (so stale results are dropped).
    Completion(u64, Option<CompletionResponse>),
    /// `textDocument/publishDiagnostics`.
    PublishDiagnostics(PublishDiagnosticsParams),
    /// `window/showMessage`.
    ShowMessage(ShowMessageParams),
    /// `window/logMessage`.
    LogMessage(LogMessageParams),
    /// `$/progress` (indexing / cache priming, etc.).
    Progress(ProgressParams),
}

/// The client-side router state: forwards server notifications onto `tx` and
/// answers the server->client requests rust-analyzer issues during startup.
struct ClientState {
    tx: UnboundedSender<LspEvent>,
}

impl LanguageClient for ClientState {
    type Error = ResponseError;
    type NotifyResult = ControlFlow<async_lsp::Result<()>>;

    // --- Notifications: forward to the editor loop. ---
    //
    // These MUST be handled: async-lsp routes an unhandled non-`$/`
    // notification to `Break(Err(Routing))`, which would terminate the main
    // loop and silently kill the LSP connection.

    fn publish_diagnostics(&mut self, params: PublishDiagnosticsParams) -> Self::NotifyResult {
        let _ = self.tx.send(LspEvent::PublishDiagnostics(params));
        ControlFlow::Continue(())
    }

    fn show_message(&mut self, params: ShowMessageParams) -> Self::NotifyResult {
        let _ = self.tx.send(LspEvent::ShowMessage(params));
        ControlFlow::Continue(())
    }

    fn log_message(&mut self, params: LogMessageParams) -> Self::NotifyResult {
        let _ = self.tx.send(LspEvent::LogMessage(params));
        ControlFlow::Continue(())
    }

    fn telemetry_event(
        &mut self,
        _params: OneOf<serde_json::Map<String, serde_json::Value>, Vec<serde_json::Value>>,
    ) -> Self::NotifyResult {
        // Ignored, but must be handled so it doesn't break the loop.
        ControlFlow::Continue(())
    }

    fn progress(&mut self, params: ProgressParams) -> Self::NotifyResult {
        let _ = self.tx.send(LspEvent::Progress(params));
        ControlFlow::Continue(())
    }

    // --- Server -> client requests: answer so the server doesn't stall. ---

    fn configuration(
        &mut self,
        params: ConfigurationParams,
    ) -> BoxFuture<'static, Result<Vec<serde_json::Value>, Self::Error>> {
        // We expose no configuration; reply with a null per requested item.
        let nulls = vec![serde_json::Value::Null; params.items.len()];
        Box::pin(ready(Ok(nulls)))
    }

    fn register_capability(
        &mut self,
        _params: RegistrationParams,
    ) -> BoxFuture<'static, Result<(), Self::Error>> {
        // Accept dynamic registrations (e.g. file watchers) as a no-op.
        Box::pin(ready(Ok(())))
    }

    fn work_done_progress_create(
        &mut self,
        _params: WorkDoneProgressCreateParams,
    ) -> BoxFuture<'static, Result<(), Self::Error>> {
        // Accept progress tokens so the server emits `$/progress` updates.
        Box::pin(ready(Ok(())))
    }
}

/// Spawn `rust-analyzer` rooted at `root_dir` and start its async-lsp main loop
/// on the current tokio runtime. Returns the [`ServerSocket`] request handle.
///
/// Returns `Err` if the server binary cannot be spawned (e.g. not on `PATH`),
/// letting the caller **degrade gracefully** — the editor keeps working without
/// language intelligence, exactly like MS3 when a grammar fails to load.
///
/// Must be called from within a tokio runtime (it uses `tokio::spawn`).
pub fn spawn(root_dir: &Path, tx: UnboundedSender<LspEvent>) -> std::io::Result<ServerSocket> {
    let mut child = tokio::process::Command::new("rust-analyzer")
        .current_dir(root_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stdin = child.stdin.take().expect("piped stdin");

    let (mainloop, server) =
        MainLoop::new_client(|_server| Router::from_language_client(ClientState { tx }));

    tokio::spawn(async move {
        // Keep the child bound to the loop's lifetime; `kill_on_drop` reaps it
        // when this task ends (server exit -> stdout EOF -> run_buffered returns,
        // or runtime shutdown at editor quit).
        let _child = child;
        let _ = mainloop
            .run_buffered(stdout.compat(), stdin.compat_write())
            .await;
    });

    Ok(server)
}

/// Build the `initialize` request parameters: negotiate `utf-8` (preferred) then
/// `utf-16` position encoding, advertise work-done progress, and root the server
/// at `root_dir`.
pub fn initialize_params(root_dir: &Path) -> InitializeParams {
    let root_uri = Url::from_file_path(root_dir).ok();
    #[allow(deprecated)] // `root_uri` is deprecated but still honored by servers
    InitializeParams {
        root_uri: root_uri.clone(),
        workspace_folders: root_uri.as_ref().map(|uri| {
            vec![WorkspaceFolder {
                uri: uri.clone(),
                name: "root".into(),
            }]
        }),
        capabilities: ClientCapabilities {
            general: Some(GeneralClientCapabilities {
                position_encodings: Some(vec![
                    PositionEncodingKind::UTF8,
                    PositionEncodingKind::UTF16,
                ]),
                ..GeneralClientCapabilities::default()
            }),
            window: Some(WindowClientCapabilities {
                work_done_progress: Some(true),
                ..WindowClientCapabilities::default()
            }),
            ..ClientCapabilities::default()
        },
        ..InitializeParams::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_lsp::lsp_types::InitializedParams;
    use async_lsp::LanguageServer;

    /// End-to-end handshake against a real `rust-analyzer`. Ignored by default
    /// (needs the binary + network-free Cargo project); run explicitly with
    /// `cargo test --  --ignored lsp_handshake`.
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "spawns rust-analyzer"]
    async fn lsp_handshake() {
        let root = std::env::current_dir().unwrap();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut server = spawn(&root, tx).expect("rust-analyzer should spawn");
        let init = server
            .initialize(initialize_params(&root))
            .await
            .expect("initialize should succeed");
        // rust-analyzer honors our utf-8 preference.
        assert_eq!(
            init.capabilities.position_encoding,
            Some(PositionEncodingKind::UTF8)
        );
        server.initialized(InitializedParams {}).unwrap();
        server.shutdown(()).await.unwrap();
        server.exit(()).unwrap();
    }
}
