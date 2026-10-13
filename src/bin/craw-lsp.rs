use craw::lsp_analyzer::{LspAnalyzer, ServerConfig};
use craw::semantic::Workspace;
use parking_lot::Mutex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

#[derive(Debug)]
struct Backend {
    client: Client,
    analyzer: Mutex<LspAnalyzer>,
    workspace: Mutex<Workspace>,
}

/// Parses the `craw` settings object VS Code sends (nested per the dotted
/// setting names, e.g. `craw.diagnostics.unresolved` becomes
/// `{"diagnostics": {"unresolved": ...}}`) into a [`ServerConfig`]. Missing
/// fields keep their default (everything on). Accepts both the bare
/// section (`initializationOptions`) and the `{"craw": {...}}` wrapper
/// vscode-languageclient uses for `workspace/didChangeConfiguration`.
fn parse_server_config(value: &serde_json::Value) -> ServerConfig {
    let value = value.get("craw").unwrap_or(value);
    let mut config = ServerConfig::default();
    if let Some(unresolved) = value
        .pointer("/diagnostics/unresolved")
        .and_then(|v| v.as_bool())
    {
        config.unresolved = unresolved;
    }
    if let Some(unused) = value
        .pointer("/diagnostics/unused")
        .and_then(|v| v.as_bool())
    {
        config.unused = unused;
    }
    let flag = |path: &str| value.pointer(path).and_then(|v| v.as_bool());
    if let Some(b) = flag("/inlayHints/parameterNames/enable") {
        config.inlay.parameter_names = b;
    }
    if let Some(b) = flag("/inlayHints/typeHints/enable") {
        config.inlay.types = b;
    }
    if let Some(b) = flag("/inlayHints/returnTypeHints/enable") {
        config.inlay.return_types = b;
    }
    config
}

/// Transpiles `text` to Rust in a child process (this same binary, run
/// with `--transpile-stdin`), so a transpiler panic — which aborts the
/// process in release builds — can never take the language server down.
///
/// With the document's `path`, its imports resolve relative to it, and
/// every other open buffer in `open` (path, unsaved text) is linked in
/// place of that file's on-disk contents.
fn transpile_in_child(
    text: &str,
    path: Option<&std::path::Path>,
    open: Vec<(std::path::PathBuf, String)>,
) -> std::result::Result<String, String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(exe);
    let payload = match path {
        Some(p) => {
            cmd.arg("--transpile-json");
            let mut files: serde_json::Map<String, serde_json::Value> = open
                .into_iter()
                .map(|(p, t)| (p.to_string_lossy().into_owned(), t.into()))
                .collect();
            files.insert(p.to_string_lossy().into_owned(), text.into());
            serde_json::json!({ "entry": p.to_string_lossy(), "files": files }).to_string()
        }
        None => {
            cmd.arg("--transpile-stdin");
            text.to_string()
        }
    };
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to start transpiler: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("no stdin")?
        .write_all(payload.as_bytes())
        .map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() {
            format!("transpiler exited with {}", out.status)
        } else {
            err
        })
    }
}

/// `--transpile-stdin [PATH]`: reads Craw source on stdin, writes Rust on
/// stdout, or `line:col: message` errors on stderr with exit code 1. With
/// `PATH`, the source is treated as that file's (unsaved) content and its
/// imports are linked from disk; without, it must be self-contained.
fn transpile_stdin(path: Option<std::path::PathBuf>) -> i32 {
    use std::io::Read;
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return 2;
    }
    let entry = path.unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap_or_default()
            .join("<stdin>.craw")
    });
    transpile_with(entry, vec![], input)
}

/// `--transpile-json`: stdin is `{"entry": PATH, "files": {PATH: TEXT}}`
/// — the entry file and every open (unsaved) buffer, which override
/// those files' on-disk text while linking.
fn transpile_json() -> i32 {
    use std::io::Read;
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return 2;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&input) else {
        eprintln!("invalid --transpile-json payload");
        return 2;
    };
    let Some(entry) = v["entry"].as_str().map(std::path::PathBuf::from) else {
        eprintln!("--transpile-json payload has no entry");
        return 2;
    };
    let mut files: Vec<(std::path::PathBuf, String)> = v["files"]
        .as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(k, t)| Some((std::path::PathBuf::from(k), t.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let text = files
        .iter()
        .position(|(p, _)| *p == entry)
        .map(|i| files.remove(i).1)
        .unwrap_or_default();
    transpile_with(entry, files, text)
}

fn transpile_with(
    entry: std::path::PathBuf,
    others: Vec<(std::path::PathBuf, String)>,
    input: String,
) -> i32 {
    let mut loader = craw::modules::FsLoader::default();
    for (p, t) in others {
        loader.overrides.insert(craw::modules::normalize(&p), t);
    }
    loader
        .overrides
        .insert(craw::modules::normalize(&entry), input);
    match craw::modules::parse_program(&entry, &[], &loader) {
        Ok((ast, _)) => {
            print!("{}", craw::transpiler::transpile(&ast));
            0
        }
        Err(errors) => {
            for e in &errors {
                // The document itself: `line:col`; another module: its path.
                if craw::modules::normalize(&e.file) == craw::modules::normalize(&entry) {
                    eprintln!("{}:{}: {}", e.line + 1, e.character + 1, e.message);
                } else {
                    eprintln!("{}:{}:{}: {}", e.file.display(), e.line + 1, e.character + 1, e.message);
                }
            }
            1
        }
    }
}

fn doc_uri(params: &serde_json::Value) -> Option<String> {
    params
        .pointer("/textDocument/uri")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

impl Backend {
    /// `craw/syntaxTree` `{textDocument}` → the document's AST as text.
    async fn syntax_tree(&self, params: serde_json::Value) -> Result<String> {
        let uri = doc_uri(&params)
            .ok_or_else(|| tower_lsp::jsonrpc::Error::invalid_params("missing textDocument.uri"))?;
        self.analyzer
            .lock()
            .syntax_tree(&uri)
            .ok_or_else(|| tower_lsp::jsonrpc::Error::invalid_params("document is not open"))
    }

    /// `craw/transpile` `{textDocument}` → `{rust}` for the document's
    /// current (possibly unsaved) text, or an error listing syntax errors.
    async fn transpile(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        let uri = doc_uri(&params)
            .ok_or_else(|| tower_lsp::jsonrpc::Error::invalid_params("missing textDocument.uri"))?;
        let (text, open) = {
            let analyzer = self.analyzer.lock();
            let text = analyzer
                .text(&uri)
                .ok_or_else(|| tower_lsp::jsonrpc::Error::invalid_params("document is not open"))?;
            // Every other open buffer, so unsaved edits to imported
            // modules show up in the generated Rust too.
            let open: Vec<(std::path::PathBuf, String)> = analyzer
                .open_uris()
                .into_iter()
                .filter(|u| *u != uri)
                .filter_map(|u| {
                    let p = Url::parse(&u).ok()?.to_file_path().ok()?;
                    Some((p, analyzer.text(&u)?))
                })
                .collect();
            (text, open)
        };
        let path = Url::parse(&uri).ok().and_then(|u| u.to_file_path().ok());
        let result =
            tokio::task::spawn_blocking(move || transpile_in_child(&text, path.as_deref(), open))
            .await
            .map_err(|_| tower_lsp::jsonrpc::Error::internal_error())?;
        match result {
            Ok(rust) => Ok(serde_json::json!({ "rust": rust })),
            Err(message) => Err(tower_lsp::jsonrpc::Error {
                code: tower_lsp::jsonrpc::ErrorCode::ServerError(-32803),
                message: message.into(),
                data: None,
            }),
        }
    }
}

impl Backend {
    /// Republishes diagnostics for every open document except `skip`: an
    /// edit to one module can fix or break the imports of another.
    async fn republish_others(&self, skip: Option<&Url>) {
        let batch: Vec<(Url, Vec<Diagnostic>)> = {
            let analyzer = self.analyzer.lock();
            analyzer
                .open_uris()
                .into_iter()
                .filter_map(|u| {
                    let url = Url::parse(&u).ok()?;
                    if Some(&url) == skip {
                        return None;
                    }
                    Some((url, analyzer.diagnostics_for(&u)?))
                })
                .collect()
        };
        for (url, diags) in batch {
            self.client.publish_diagnostics(url, diags, None).await;
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        {
            let mut workspace = self.workspace.lock();
            let mut roots = Vec::new();
            #[allow(deprecated)]
            let folders: Vec<Url> = match params.workspace_folders {
                Some(f) if !f.is_empty() => f.into_iter().map(|f| f.uri).collect(),
                _ => params.root_uri.into_iter().collect(),
            };
            for uri in folders {
                if let Ok(path) = uri.to_file_path() {
                    workspace.add_root(&path);
                    roots.push(path);
                }
            }
            // Workspace folders are module search roots too.
            self.analyzer.lock().set_roots(roots);
        }
        if let Some(options) = &params.initialization_options {
            self.analyzer
                .lock()
                .set_config(parse_server_config(options));
        }
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::INCREMENTAL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                document_highlight_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: Default::default(),
                })),
                document_symbol_provider: Some(OneOf::Left(true)),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(true),
                    trigger_characters: Some(vec![
                        ".".to_string(),
                        ":".to_string(),
                        ">".to_string(),
                    ]),
                    ..Default::default()
                }),
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec!["(".to_string(), ",".to_string()]),
                    retrigger_characters: Some(vec![",".to_string()]),
                    work_done_progress_options: Default::default(),
                }),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            legend: craw::ide::semantic_tokens::legend(),
                            range: Some(true),
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                            work_done_progress_options: Default::default(),
                        },
                    ),
                ),
                folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
                selection_range_provider: Some(SelectionRangeProviderCapability::Simple(true)),
                inlay_hint_provider: Some(OneOf::Left(true)),
                code_action_provider: Some(CodeActionProviderCapability::Options(
                    CodeActionOptions {
                        code_action_kinds: Some(vec![
                            CodeActionKind::QUICKFIX,
                            CodeActionKind::REFACTOR_EXTRACT,
                            CodeActionKind::REFACTOR_INLINE,
                            CodeActionKind::REFACTOR_REWRITE,
                        ]),
                        work_done_progress_options: Default::default(),
                        resolve_provider: None,
                    },
                )),
                document_formatting_provider: Some(OneOf::Left(true)),
                document_range_formatting_provider: Some(OneOf::Left(true)),
                call_hierarchy_provider: Some(CallHierarchyServerCapability::Simple(true)),
                type_definition_provider: Some(TypeDefinitionProviderCapability::Simple(true)),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "craw-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "Craw LSP initialized!")
            .await;
        let watchers = vec![FileSystemWatcher {
            glob_pattern: GlobPattern::String("**/*.craw".to_string()),
            kind: None,
        }];
        let registration = Registration {
            id: "craw-workspace-watch".to_string(),
            method: "workspace/didChangeWatchedFiles".to_string(),
            register_options: serde_json::to_value(DidChangeWatchedFilesRegistrationOptions {
                watchers,
            })
            .ok(),
        };
        let _ = self.client.register_capability(vec![registration]).await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let text = params.text_document.text;
        let diags = {
            let mut analyzer = self.analyzer.lock();
            analyzer.update_document(uri.as_str(), &text)
        };
        self.workspace.lock().update_open(uri.clone(), &text);
        self.client.publish_diagnostics(uri.clone(), diags, None).await;
        self.republish_others(Some(&uri)).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let mut diags = Vec::new();
        {
            let mut analyzer = self.analyzer.lock();
            // Each change's range is resolved against the text *after*
            // every earlier change in this same notification has already
            // been applied, so they must be applied one at a time, in
            // order, rather than only looking at the last one.
            for change in params.content_changes {
                diags = analyzer.apply_change(uri.as_str(), change.range, &change.text);
            }
        }
        if let Some(text) = self
            .analyzer
            .lock()
            .get_document(uri.as_str())
            .map(|d| d.model.text().to_string())
        {
            self.workspace.lock().update_open(uri.clone(), &text);
        }
        self.client.publish_diagnostics(uri.clone(), diags, None).await;
        self.republish_others(Some(&uri)).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.analyzer.lock().remove_document(uri.as_str());
        // The workspace index still holds the editor's (possibly unsaved)
        // text; fall back to what's on disk, or drop it if the file is gone.
        let on_disk = uri
            .to_file_path()
            .ok()
            .and_then(|path| std::fs::read_to_string(path).ok());
        {
            let mut workspace = self.workspace.lock();
            match on_disk {
                Some(text) => workspace.update_open(uri.clone(), &text),
                None => workspace.remove(&uri),
            }
        }
        // Importers now see the file's on-disk text again.
        self.republish_others(Some(&uri)).await;
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        let config = parse_server_config(&params.settings);
        let uris: Vec<String> = {
            let mut analyzer = self.analyzer.lock();
            analyzer.set_config(config);
            analyzer.open_uris()
        };
        for uri in uris {
            let Some(diags) = self.analyzer.lock().diagnostics_for(&uri) else {
                continue;
            };
            if let Ok(url) = Url::parse(&uri) {
                self.client.publish_diagnostics(url, diags, None).await;
            }
        }
        // Inlay hint settings may have changed; clients that don't
        // support refresh just return an error, which is harmless.
        let _ = self.client.inlay_hint_refresh().await;
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        {
            let mut analyzer = self.analyzer.lock();
            for change in &params.changes {
                if let Ok(path) = change.uri.to_file_path() {
                    analyzer.file_changed(&path);
                }
            }
        }
        {
            let mut workspace = self.workspace.lock();
            for change in params.changes {
                match change.typ {
                    FileChangeType::DELETED => workspace.remove(&change.uri),
                    _ => {
                        if let Ok(path) = change.uri.to_file_path()
                            && let Ok(text) = std::fs::read_to_string(&path)
                        {
                            workspace.update_open(change.uri, &text);
                        }
                    }
                }
            }
        }
        // Modules appeared, disappeared or changed on disk.
        self.republish_others(None).await;
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<Vec<SymbolInformation>>> {
        let workspace = self.workspace.lock();
        let hits = workspace.query_symbols(&params.query);
        #[allow(deprecated)]
        let symbols = hits
            .into_iter()
            .map(|h| SymbolInformation {
                name: h.name,
                kind: craw::lsp_analyzer::lsp_symbol_kind(h.kind),
                tags: None,
                deprecated: None,
                location: Location {
                    uri: h.uri,
                    range: h.name_range,
                },
                container_name: None,
            })
            .collect();
        Ok(Some(symbols))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .to_string();
        let position = params.text_document_position_params.position;
        let analyzer = self.analyzer.lock();
        Ok(analyzer.get_hover_info(&uri, position.line, position.character))
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .to_string();
        let position = params.text_document_position_params.position;
        let analyzer = self.analyzer.lock();
        Ok(analyzer.get_signature_help(&uri, position.line, position.character))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .to_string();
        let position = params.text_document_position_params.position;
        let analyzer = self.analyzer.lock();
        Ok(analyzer
            .get_definition(&uri, position.line, position.character)
            .map(GotoDefinitionResponse::Scalar))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri.to_string();
        let position = params.text_document_position.position;
        let analyzer = self.analyzer.lock();
        let items = analyzer.get_completions(&uri, position.line, position.character);
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn completion_resolve(&self, item: CompletionItem) -> Result<CompletionItem> {
        let analyzer = self.analyzer.lock();
        Ok(analyzer.resolve_completion(item))
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri.to_string();
        let position = params.text_document_position.position;
        let analyzer = self.analyzer.lock();
        Ok(analyzer.find_references(&uri, position.line, position.character))
    }

    async fn document_highlight(
        &self,
        params: DocumentHighlightParams,
    ) -> Result<Option<Vec<DocumentHighlight>>> {
        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .to_string();
        let position = params.text_document_position_params.position;
        let analyzer = self.analyzer.lock();
        Ok(analyzer.document_highlight(&uri, position.line, position.character))
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        let uri = params.text_document.uri.to_string();
        let position = params.position;
        let analyzer = self.analyzer.lock();
        Ok(analyzer
            .prepare_rename(&uri, position.line, position.character)
            .map(
                |(range, placeholder)| PrepareRenameResponse::RangeWithPlaceholder {
                    range,
                    placeholder,
                },
            ))
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = params.text_document_position.text_document.uri.to_string();
        let position = params.text_document_position.position;
        let new_name = params.new_name;
        let analyzer = self.analyzer.lock();
        analyzer
            .rename_symbol(&uri, position.line, position.character, new_name)
            .map_err(tower_lsp::jsonrpc::Error::invalid_params)
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let uri = params.text_document.uri.to_string();
        Ok(self
            .analyzer
            .lock()
            .semantic_tokens(&uri, None)
            .map(|data| {
                SemanticTokensResult::Tokens(SemanticTokens {
                    result_id: None,
                    data,
                })
            }))
    }

    async fn semantic_tokens_range(
        &self,
        params: SemanticTokensRangeParams,
    ) -> Result<Option<SemanticTokensRangeResult>> {
        let uri = params.text_document.uri.to_string();
        Ok(self
            .analyzer
            .lock()
            .semantic_tokens(&uri, Some(params.range))
            .map(|data| {
                SemanticTokensRangeResult::Tokens(SemanticTokens {
                    result_id: None,
                    data,
                })
            }))
    }

    async fn folding_range(&self, params: FoldingRangeParams) -> Result<Option<Vec<FoldingRange>>> {
        Ok(self
            .analyzer
            .lock()
            .folding_ranges(params.text_document.uri.as_str()))
    }

    async fn selection_range(
        &self,
        params: SelectionRangeParams,
    ) -> Result<Option<Vec<SelectionRange>>> {
        Ok(self
            .analyzer
            .lock()
            .selection_ranges(params.text_document.uri.as_str(), &params.positions))
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        Ok(self
            .analyzer
            .lock()
            .inlay_hints(params.text_document.uri.as_str(), params.range))
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let actions = self
            .analyzer
            .lock()
            .code_actions(params.text_document.uri.as_str(), params.range);
        let only = params.context.only;
        Ok(actions.map(|actions| {
            actions
                .into_iter()
                .filter(|a| {
                    only.as_ref().is_none_or(|kinds| {
                        a.kind.as_ref().is_some_and(|k| {
                            kinds.iter().any(|want| {
                                k.as_str() == want.as_str()
                                    || k.as_str().starts_with(&format!("{}.", want.as_str()))
                            })
                        })
                    })
                })
                .map(CodeActionOrCommand::CodeAction)
                .collect()
        }))
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        Ok(self
            .analyzer
            .lock()
            .format(params.text_document.uri.as_str(), None))
    }

    async fn range_formatting(
        &self,
        params: DocumentRangeFormattingParams,
    ) -> Result<Option<Vec<TextEdit>>> {
        Ok(self
            .analyzer
            .lock()
            .format(params.text_document.uri.as_str(), Some(params.range)))
    }

    async fn prepare_call_hierarchy(
        &self,
        params: CallHierarchyPrepareParams,
    ) -> Result<Option<Vec<CallHierarchyItem>>> {
        let p = params.text_document_position_params;
        Ok(self.analyzer.lock().prepare_call_hierarchy(
            p.text_document.uri.as_str(),
            p.position.line,
            p.position.character,
        ))
    }

    async fn incoming_calls(
        &self,
        params: CallHierarchyIncomingCallsParams,
    ) -> Result<Option<Vec<CallHierarchyIncomingCall>>> {
        Ok(self.analyzer.lock().incoming_calls(&params.item))
    }

    async fn outgoing_calls(
        &self,
        params: CallHierarchyOutgoingCallsParams,
    ) -> Result<Option<Vec<CallHierarchyOutgoingCall>>> {
        Ok(self.analyzer.lock().outgoing_calls(&params.item))
    }

    async fn goto_type_definition(
        &self,
        params: request::GotoTypeDefinitionParams,
    ) -> Result<Option<request::GotoTypeDefinitionResponse>> {
        let p = params.text_document_position_params;
        Ok(self
            .analyzer
            .lock()
            .type_definition(
                p.text_document.uri.as_str(),
                p.position.line,
                p.position.character,
            )
            .map(GotoDefinitionResponse::Scalar))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri.to_string();
        let analyzer = self.analyzer.lock();
        Ok(analyzer
            .get_document_symbols(&uri)
            .map(DocumentSymbolResponse::Nested))
    }
}

#[tokio::main]
async fn main() {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("craw-lsp {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--transpile-json") {
        std::process::exit(transpile_json());
    }
    if let Some(i) = args.iter().position(|a| a == "--transpile-stdin") {
        let path = args.get(i + 1).map(std::path::PathBuf::from);
        std::process::exit(transpile_stdin(path));
    }
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::build(|client| Backend {
        client,
        analyzer: Mutex::new(LspAnalyzer::new()),
        workspace: Mutex::new(Workspace::new()),
    })
    .custom_method("craw/syntaxTree", Backend::syntax_tree)
    .custom_method("craw/transpile", Backend::transpile)
    .finish();
    Server::new(stdin, stdout, socket).serve(service).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;

    async fn setup_backend(text: &str) -> Backend {
        let (service, _) = LspService::build(|client| Backend {
            client,
            analyzer: Mutex::new(LspAnalyzer::new()),
            workspace: Mutex::new(Workspace::new()),
        })
        .finish();

        let backend = service.inner();
        let uri = Url::parse("file:///test.craw").unwrap();

        backend
            .did_open(DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: uri.clone(),
                    language_id: "craw".to_string(),
                    version: 1,
                    text: text.to_string(),
                },
            })
            .await;

        let backend_direct = Backend {
            client: service.inner().client.clone(),
            analyzer: Mutex::new(LspAnalyzer::new()),
            workspace: Mutex::new(Workspace::new()),
        };
        backend_direct
            .analyzer
            .lock()
            .update_document(uri.as_str(), text);
        backend_direct
    }

    #[tokio::test]
    async fn test_completion_params() {
        let text = "def my_func(a, b):\n    return a\n";
        let backend = setup_backend(text).await;

        let completions = backend
            .completion(CompletionParams {
                text_document_position: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier {
                        uri: Url::parse("file:///test.craw").unwrap(),
                    },
                    position: Position {
                        line: 1,
                        character: 10,
                    },
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
                context: None,
            })
            .await
            .unwrap()
            .unwrap();

        if let CompletionResponse::Array(items) = completions {
            let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
            assert!(labels.contains(&"a"), "Should contain parameter 'a'");
            assert!(labels.contains(&"b"), "Should contain parameter 'b'");
        } else {
            panic!("Expected CompletionResponse::Array");
        }
    }

    #[tokio::test]
    async fn test_completion_rust_passthrough() {
        let text = "rust:\n    fn my_rust_fn() {}\n\n";
        let backend = setup_backend(text).await;

        let completions = backend
            .completion(CompletionParams {
                text_document_position: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier {
                        uri: Url::parse("file:///test.craw").unwrap(),
                    },
                    position: Position {
                        line: 2,
                        character: 0,
                    },
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
                context: None,
            })
            .await
            .unwrap()
            .unwrap();

        if let CompletionResponse::Array(items) = completions {
            let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
            assert!(
                labels.contains(&"my_rust_fn"),
                "Should contain rust function"
            );
        } else {
            panic!("Expected CompletionResponse::Array");
        }
    }

    #[tokio::test]
    async fn test_completion_native_import() {
        let text = "from std.math import math\n";
        let backend = setup_backend(text).await;

        let completions = backend
            .completion(CompletionParams {
                text_document_position: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier {
                        uri: Url::parse("file:///test.craw").unwrap(),
                    },
                    position: Position {
                        line: 1,
                        character: 0,
                    },
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
                context: None,
            })
            .await
            .unwrap()
            .unwrap();

        if let CompletionResponse::Array(items) = completions {
            let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
            eprintln!("Labels: {:?}", labels);
            // In our current simple logic, it extracts idents from the statement
            assert!(
                labels.contains(&"math"),
                "Should contain imported name 'math'"
            );
        } else {
            panic!("Expected CompletionResponse::Array");
        }
    }

    #[tokio::test]
    async fn test_hover_kind() {
        let text = "def my_func(a, b):\n    return a\n";
        let backend = setup_backend(text).await;

        let hover_res = backend
            .hover(HoverParams {
                text_document_position_params: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier {
                        uri: Url::parse("file:///test.craw").unwrap(),
                    },
                    position: Position {
                        line: 0,
                        character: 4,
                    },
                },
                work_done_progress_params: Default::default(),
            })
            .await
            .unwrap()
            .unwrap();

        if let HoverContents::Markup(m) = hover_res.contents {
            assert_eq!(m.value, "```craw\ndef my_func(a, b)\n```");
        } else {
            panic!("Expected HoverContents::Markup");
        }
    }

    #[test]
    fn parse_server_config_reads_nested_diagnostics_settings() {
        let cfg = parse_server_config(&serde_json::json!({
            "diagnostics": { "unresolved": false, "unused": true }
        }));
        assert!(!cfg.unresolved);
        assert!(cfg.unused);
    }

    #[test]
    fn parse_server_config_reads_settings_wrapped_in_the_craw_section() {
        let cfg = parse_server_config(&serde_json::json!({
            "craw": { "diagnostics": { "unresolved": false, "unused": false } }
        }));
        assert!(!cfg.unresolved);
        assert!(!cfg.unused);
    }

    #[test]
    fn parse_server_config_defaults_missing_fields_to_on() {
        let cfg = parse_server_config(&serde_json::json!({}));
        assert!(cfg.unresolved);
        assert!(cfg.unused);
    }

    #[tokio::test]
    async fn did_change_configuration_filters_republished_diagnostics() {
        let backend = setup_backend("x = y\n").await;
        let uri = "file:///test.craw".to_string();
        assert!(
            backend
                .analyzer
                .lock()
                .diagnostics_for(&uri)
                .unwrap()
                .iter()
                .any(|d| d.code.as_ref().is_some_and(|c| matches!(c,
                    tower_lsp::lsp_types::NumberOrString::String(s) if s == "E0100")))
        );

        backend
            .did_change_configuration(DidChangeConfigurationParams {
                settings: serde_json::json!({ "diagnostics": { "unresolved": false, "unused": true } }),
            })
            .await;

        assert!(
            !backend
                .analyzer
                .lock()
                .diagnostics_for(&uri)
                .unwrap()
                .iter()
                .any(|d| d.code.as_ref().is_some_and(|c| matches!(c,
                    tower_lsp::lsp_types::NumberOrString::String(s) if s == "E0100")))
        );
    }
}
