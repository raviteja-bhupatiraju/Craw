use craw::lsp_analyzer::LspAnalyzer;
use parking_lot::Mutex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

#[derive(Debug)]
struct Backend {
    client: Client,
    analyzer: Mutex<LspAnalyzer>,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(false),
                    trigger_characters: Some(vec![
                        ".".to_string(),
                        ":".to_string(),
                        ">".to_string(),
                    ]),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "Craw LSP initialized!")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let text = params.text_document.text;
        let diags = {
            let mut analyzer = self.analyzer.lock();
            analyzer.update_document(&uri.to_string(), &text)
        };
        self.client.publish_diagnostics(uri, diags, None).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        if let Some(change) = params.content_changes.into_iter().next() {
            let diags = {
                let mut analyzer = self.analyzer.lock();
                analyzer.update_document(&uri.to_string(), &change.text)
            };
            self.client.publish_diagnostics(uri, diags, None).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri.to_string();
        let mut analyzer = self.analyzer.lock();
        analyzer.remove_document(&uri);
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

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri.to_string();
        let position = params.text_document_position.position;
        let analyzer = self.analyzer.lock();
        Ok(analyzer.find_references(&uri, position.line, position.character))
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = params.text_document_position.text_document.uri.to_string();
        let position = params.text_document_position.position;
        let new_name = params.new_name;
        let analyzer = self.analyzer.lock();
        Ok(analyzer.rename_symbol(&uri, position.line, position.character, new_name))
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
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::build(|client| Backend {
        client,
        analyzer: Mutex::new(LspAnalyzer::new()),
    })
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
        };
        backend_direct
            .analyzer
            .lock()
            .update_document(&uri.to_string(), text);
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

        if let HoverContents::Scalar(MarkedString::LanguageString(lang_string)) = hover_res.contents {
            assert_eq!(lang_string.value, "(fn) my_func");
        } else {
            panic!("Expected HoverContents::Scalar LanguageString");
        }
    }
}
