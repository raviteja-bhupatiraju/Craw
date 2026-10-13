//! Bridges the LSP wire protocol (UTF-16 line/column positions, `Url`
//! strings) to [`crate::semantic::SemanticModel`], which does all the real
//! work. Every position-based lookup here follows the same shape: convert
//! the LSP position to a byte offset via the document's `LineIndex`, find
//! the [`Reference`](crate::semantic::Reference) whose span contains it,
//! and read off its resolved `Symbol`.

use crate::semantic::{
    RefKind, Reference, Resolution, SemanticModel, Symbol, SymbolId, SymbolKind,
};
use crate::text::Pos;
use std::collections::HashMap;
use tower_lsp::lsp_types::{
    Command, CompletionItem, CompletionItemKind, Diagnostic, DocumentHighlight,
    DocumentHighlightKind, DocumentSymbol, Documentation, Hover, HoverContents, InsertTextFormat,
    Location, MarkupContent, MarkupKind, Range, TextEdit, Url, WorkspaceEdit,
};

/// `command` + `commitCharacters` shared by every completion item for a
/// callable (a builtin function or a UFCS method): accepting it should
/// immediately show signature help, and typing `(` or `.` should commit it.
fn callable_extras() -> (Option<Command>, Option<Vec<String>>) {
    (
        Some(Command {
            title: "Trigger Parameter Hints".to_string(),
            command: "editor.action.triggerParameterHints".to_string(),
            arguments: None,
        }),
        Some(vec!["(".to_string(), ".".to_string()]),
    )
}

#[derive(Debug)]
pub struct DocumentState {
    /// Shared so cross-file lookups can hand out other documents' models.
    pub model: std::sync::Arc<SemanticModel>,
}

/// User-configurable toggles for semantic diagnostics (`craw.diagnostics.*`
/// in the extension). Filtering happens after the model is built, not
/// during `SemanticModel::build`, so the model itself stays independent of
/// LSP-only presentation settings.
#[derive(Debug, Clone, Copy)]
pub struct ServerConfig {
    pub unresolved: bool,
    pub unused: bool,
    /// `craw.inlayHints.*`.
    pub inlay: crate::ide::inlay_hints::HintConfig,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            unresolved: true,
            unused: true,
            inlay: Default::default(),
        }
    }
}

#[derive(Debug)]
pub struct LspAnalyzer {
    pub(crate) documents: HashMap<String, DocumentState>,
    config: ServerConfig,
    pub(crate) project: crate::lsp_modules::Project,
}

impl Default for LspAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl LspAnalyzer {
    pub fn new() -> Self {
        Self {
            documents: HashMap::new(),
            config: ServerConfig::default(),
            project: Default::default(),
        }
    }

    pub fn set_config(&mut self, config: ServerConfig) {
        self.config = config;
    }

    fn diagnostic_allowed(&self, code: &Diagnostic) -> bool {
        match code.code.as_ref() {
            Some(tower_lsp::lsp_types::NumberOrString::String(s)) if s == "E0100" => {
                self.config.unresolved
            }
            Some(tower_lsp::lsp_types::NumberOrString::String(s)) if s == "W0200" => {
                self.config.unused
            }
            _ => true,
        }
    }

    /// Recomputes the filtered diagnostic set for an already-open document
    /// from its current model, without reparsing — used when only
    /// `ServerConfig` changed (e.g. a `craw.diagnostics.*` setting).
    pub fn diagnostics_for(&self, uri: &str) -> Option<Vec<Diagnostic>> {
        let model = &self.documents.get(uri)?.model;
        Some(self.build_diagnostics(uri, model))
    }

    fn build_diagnostics(&self, uri: &str, model: &SemanticModel) -> Vec<Diagnostic> {
        let mut diags: Vec<Diagnostic> = model
            .syntax_errors()
            .iter()
            .map(|e| Diagnostic {
                range: model.line_index().range(e.span),
                severity: Some(tower_lsp::lsp_types::DiagnosticSeverity::ERROR),
                code: Some(tower_lsp::lsp_types::NumberOrString::String(
                    "E0001".to_string(),
                )),
                source: Some("craw".to_string()),
                message: e.message.clone(),
                ..Default::default()
            })
            .collect();
        diags.extend(
            model
                .semantic_diagnostics()
                .iter()
                .filter(|d| self.diagnostic_allowed(d))
                .cloned(),
        );
        // Names a `from m import *` provides look unresolved to a single
        // file's analysis.
        let starred = self.star_provided(uri, model);
        if !starred.is_empty() {
            let text = model.text();
            diags.retain(|d| {
                let is_unresolved = d.code
                    == Some(tower_lsp::lsp_types::NumberOrString::String("E0100".into()));
                let name = model
                    .line_index()
                    .offset(d.range.start.into())
                    .zip(model.line_index().offset(d.range.end.into()))
                    .and_then(|(a, b)| text.get(a..b));
                !(is_unresolved && name.is_some_and(|n| starred.contains(n)))
            });
        }
        diags.extend(self.import_diagnostics(uri, model));
        diags
    }

    /// Rebuilds the document's semantic model from scratch and returns its
    /// combined syntax + semantic diagnostics. The new model always
    /// replaces the old one, even when the new text fails to parse (with
    /// error recovery, some AST always survives) — a stale model is never
    /// kept around to serve positions against text that's no longer there.
    pub fn update_document(&mut self, uri: &str, text: &str) -> Vec<Diagnostic> {
        let model = std::sync::Arc::new(SemanticModel::build(text));
        self.documents.insert(
            uri.to_string(),
            DocumentState {
                model: model.clone(),
            },
        );
        self.build_diagnostics(uri, &model)
    }

    /// Applies one `textDocument/didChange` content change to the
    /// document's current text and rebuilds its model, exactly like
    /// [`Self::update_document`] with the patched text. `range: None`
    /// means the change is a full-document replacement (the
    /// `TextDocumentSyncKind::FULL` shape); `Some(range)` replaces just
    /// that (UTF-16) range with `text`, computed against this document's
    /// *current* content — so when a client sends several changes for one
    /// notification, each must be applied with its own call, in order,
    /// before the next one's range is resolved.
    pub fn apply_change(
        &mut self,
        uri: &str,
        range: Option<tower_lsp::lsp_types::Range>,
        text: &str,
    ) -> Vec<Diagnostic> {
        let new_text = match range {
            None => text.to_string(),
            Some(range) => {
                let current = self
                    .documents
                    .get(uri)
                    .map(|d| d.model.text().to_string())
                    .unwrap_or_default();
                let line_index = crate::text::LineIndex::new(&current);
                let start = line_index
                    .offset(Pos::from(range.start))
                    .unwrap_or(current.len());
                let end = line_index
                    .offset(Pos::from(range.end))
                    .unwrap_or(current.len());
                let mut patched = String::with_capacity(current.len() + text.len());
                patched.push_str(&current[..start]);
                patched.push_str(text);
                patched.push_str(&current[end..]);
                patched
            }
        };
        self.update_document(uri, &new_text)
    }

    pub fn get_document(&self, uri: &str) -> Option<&DocumentState> {
        self.documents.get(uri)
    }

    /// Every currently open document's URI, for republishing diagnostics
    /// across all of them when `ServerConfig` changes.
    pub fn open_uris(&self) -> Vec<String> {
        self.documents.keys().cloned().collect()
    }

    pub fn remove_document(&mut self, uri: &str) {
        self.documents.remove(uri);
    }

    fn offset(model: &SemanticModel, line: u32, col: u32) -> Option<usize> {
        model.line_index().offset(Pos {
            line,
            character: col,
        })
    }

    fn reference_at(model: &SemanticModel, offset: usize) -> Option<&Reference> {
        model
            .references()
            .iter()
            .find(|r| r.span.start <= offset && offset < r.span.end)
    }

    fn symbol_at(model: &SemanticModel, line: u32, col: u32) -> Option<(SymbolId, &Symbol)> {
        let offset = Self::offset(model, line, col)?;
        let r = Self::reference_at(model, offset)?;
        let Resolution::Symbol(id) = r.target else {
            return None;
        };
        Some((id, model.symbol(id)))
    }

    /// Signature help for the call the cursor is inside: walks backward
    /// from the cursor, tracking bracket depth, to the innermost unclosed
    /// `(`; if the token right before it is an identifier that resolves
    /// to a `def`/method/`data` constructor with a known signature (or,
    /// failing that, a builtin), returns its rendered signature and which
    /// parameter the cursor is currently in (a `*args` vararg absorbs
    /// every argument from its own position onward, so the active
    /// parameter never advances past it).
    pub fn get_signature_help(
        &self,
        uri: &str,
        line: u32,
        col: u32,
    ) -> Option<tower_lsp::lsp_types::SignatureHelp> {
        use crate::lexer::Token;
        let model = &self.documents.get(uri)?.model;
        let offset = Self::offset(model, line, col)?;
        let tokens = model.tokens();
        let spans = model.spans();

        let lparen = Self::find_enclosing_call(tokens, spans, offset)?;
        if lparen == 0 {
            return None;
        }
        let name_idx = lparen - 1;
        let Token::Ident(name) = &tokens[name_idx] else {
            return None;
        };

        // Resolve the callee: prefer whatever the identifier before `(`
        // resolves to at that position (a def, method, or data
        // constructor); fall back to a builtin.
        // Resolved directly by name + scope, deliberately *not* via
        // `model.references()`: a call still being typed (no closing `)`
        // yet, the common case signature help exists for) fails to parse
        // and gets blanked by error recovery, so no Reference is ever
        // built for its callee even though the original tokens located
        // above (`find_enclosing_call`) are unaffected.
        let callee_scope = model.scope_at(spans[name_idx].start);
        let local = model.lookup(name, callee_scope).map(|id| model.symbol(id));
        // An imported function (or `module.f`): its signature lives in the
        // file that defines it.
        let cross = self.cross_target(uri, model, name_idx);
        let resolved = match &cross {
            Some(t) => t.symbol.map(|id| t.model.symbol(id)).or(local),
            None => local,
        };

        let (sig_label, param_names, has_vararg, documentation) = if let Some(sym) = resolved
            && let Some(sig) = &sym.signature
        {
            let mut labels: Vec<String> = sig
                .params
                .iter()
                .map(|p| match &p.default {
                    Some(d) => format!("{}={}", p.name, d),
                    None => p.name.clone(),
                })
                .collect();
            let mut names: Vec<String> = sig.params.iter().map(|p| p.name.clone()).collect();
            if let Some(v) = &sig.vararg {
                labels.push(format!("*{v}"));
                names.push(v.clone());
            }
            (
                render_signature(&sym.name, sig),
                names.into_iter().zip(labels).collect::<Vec<_>>(),
                sig.vararg.is_some(),
                sym.docs.clone(),
            )
        } else {
            let doc = crate::semantic::builtin_doc(name)?;
            let params = builtin_params(doc.signature);
            let has_vararg = params.last().is_some_and(|p| p.starts_with('*'));
            let pairs = params
                .into_iter()
                .map(|p| {
                    let n = p
                        .trim_start_matches('*')
                        .split('=')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    (n, p)
                })
                .collect();
            (
                doc.signature.to_string(),
                pairs,
                has_vararg,
                Some(doc.markdown.to_string()),
            )
        };

        let comma_count = count_commas_before_cursor(tokens, spans, lparen, offset);
        // A `name=` keyword argument selects its parameter by name.
        let keyword = current_keyword_argument(tokens, spans, lparen, offset);
        let total_slots = param_names.len();
        let active_parameter =
            match keyword.and_then(|k| param_names.iter().position(|(n, _)| *n == k)) {
                Some(i) => i,
                None if total_slots == 0 => 0,
                None if has_vararg => comma_count.min(total_slots - 1),
                None => comma_count.min(total_slots),
            };
        // Past the last parameter of a function without a vararg there is
        // no parameter to highlight; clients treat an out-of-range index
        // as "none".
        let parameters = param_names
            .into_iter()
            .map(|(_, label)| tower_lsp::lsp_types::ParameterInformation {
                label: tower_lsp::lsp_types::ParameterLabel::Simple(label),
                documentation: None,
            })
            .collect();

        Some(tower_lsp::lsp_types::SignatureHelp {
            signatures: vec![tower_lsp::lsp_types::SignatureInformation {
                label: sig_label,
                documentation: documentation.map(|d| {
                    Documentation::MarkupContent(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: d,
                    })
                }),
                parameters: Some(parameters),
                active_parameter: Some(active_parameter as u32),
            }],
            active_signature: Some(0),
            active_parameter: Some(active_parameter as u32),
        })
    }

    /// The index of the `(` most recently opened and not yet closed
    /// before `offset`, found by scanning tokens backward from the
    /// cursor and tracking bracket depth (an already-closed nested call
    /// increments the depth so its own `(` is skipped over, not mistaken
    /// for the enclosing one).
    fn find_enclosing_call(
        tokens: &[crate::lexer::Token],
        spans: &[crate::text::Span],
        offset: usize,
    ) -> Option<usize> {
        use crate::lexer::Token;
        let mut depth = 0i32;
        for i in (0..tokens.len()).rev() {
            // `>=`: a `)` starting exactly at the cursor (the auto-closed
            // bracket in `f(|)`) is after the cursor, not a closed call.
            if spans[i].start >= offset {
                continue;
            }
            match &tokens[i] {
                Token::RParen => depth += 1,
                Token::LParen => {
                    if depth == 0 {
                        return Some(i);
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
        None
    }

    pub fn get_completions(&self, uri: &str, line: u32, col: u32) -> Vec<CompletionItem> {
        let Some(model) = self.documents.get(uri).map(|d| &d.model) else {
            return Vec::new();
        };
        let Some(offset) = Self::offset(model, line, col) else {
            return Vec::new();
        };
        if Self::in_inline_rust(model, offset) {
            return Vec::new();
        }
        if let Some(items) = self.module_completions(uri, model, offset) {
            return items;
        }

        match Self::completion_context(model, offset) {
            CompletionContext::Member(receiver_tok) => {
                Self::member_completions(model, receiver_tok)
            }
            CompletionContext::Import => import_completions(),
            CompletionContext::StatementStart => {
                let mut items = local_completions(model, offset);
                items.extend(keyword_completions());
                items.extend(snippet_completions());
                items
            }
            CompletionContext::Expression => {
                let mut items = local_completions(model, offset);
                items.extend(builtin_completions());
                let taken: std::collections::HashSet<String> =
                    items.iter().map(|i| i.label.clone()).collect();
                items.extend(
                    expression_keyword_completions()
                        .into_iter()
                        .filter(|k| !taken.contains(&k.label)),
                );
                items
            }
        }
    }

    /// Whether `offset` falls inside a `rust:`/`®(...)`-style passthrough
    /// block's captured Rust source — a single opaque `String` token the
    /// lexer never breaks into Craw identifiers, so there is nothing
    /// meaningful to complete inside one.
    fn in_inline_rust(model: &SemanticModel, offset: usize) -> bool {
        let tokens = model.tokens();
        let spans = model.spans();
        let text = model.text();
        tokens.iter().enumerate().any(|(i, t)| {
            if !matches!(t, crate::lexer::Token::String(_))
                || !matches!(
                    tokens.get(i + 1),
                    Some(crate::lexer::Token::PassthroughStart)
                )
            {
                return false;
            }
            let span = spans[i];
            // A `rust:` block's captured content always runs through any
            // *blank lines* after it too, up to the next less-indented,
            // non-blank line — so the raw span alone would also cover an
            // empty line right after a block, where completions should
            // work normally. Trim complete trailing blank lines, but
            // never the content's own last (possibly newline-less) line,
            // however much trailing whitespace it has: that's exactly
            // where an in-progress edit's cursor can legitimately sit.
            let content = &text[span.start..span.end];
            let mut remaining = content;
            while let Some((rest, last_line)) = remaining.rsplit_once('\n') {
                if last_line.trim().is_empty() {
                    remaining = rest;
                } else {
                    break;
                }
            }
            let trimmed_end = span.start + remaining.len();
            span.start <= offset && offset <= trimmed_end
        })
    }

    /// The token index whose span ends at or before `offset` — i.e. the
    /// last complete token before the cursor (or the partial token
    /// currently being typed, if the cursor sits at its own end, which is
    /// the same index either way since a partial identifier is still a
    /// complete `Ident` token on its own).
    fn token_before(model: &SemanticModel, offset: usize) -> Option<usize> {
        use crate::lexer::Token;
        model
            .tokens()
            .iter()
            .zip(model.spans().iter())
            .enumerate()
            .filter(|(_, (t, s))| {
                // `Eof` is a pure end-of-stream marker that would
                // otherwise always "win" a tie for the largest
                // `end <= offset`, since nothing can come after it.
                if matches!(t, Token::Eof) {
                    return false;
                }
                // A zero-width marker (`Newline`, `Indent`, `Dedent`) sits
                // *at* one exact position rather than covering a range.
                // The cursor only counts as having passed it once it's
                // strictly beyond that position — sitting exactly at it
                // (e.g. right after typing `receiver.` with nothing else
                // on the line: the real `Dot` token and the line's own
                // `Newline` both end at that same offset) must still favor
                // whatever real token precedes it. Once the cursor moves
                // past that position (has actually crossed the line
                // break, or the indent), the marker is fair game again —
                // and is exactly what a statement-start position should
                // resolve to, even if some real token earlier on the
                // previous line ends at that identical offset too (a line
                // ending in a real newline right at end-of-text ties a
                // trailing `Number`/`Ident` against that `Newline` this
                // same way; `Newline` must still win there).
                if s.start == s.end {
                    s.end < offset
                } else {
                    s.end <= offset
                }
            })
            .max_by_key(|(_, (_, s))| s.end)
            .map(|(i, _)| i)
    }

    /// The token that actually establishes the completion context at
    /// `offset` — skipping back over a partial word currently being
    /// typed, if there is one, since that word is a filter prefix, not
    /// context evidence. Without this, `token_before` itself (an `Ident`
    /// ending exactly at the cursor) would always win, and *any* typed
    /// character — the overwhelmingly common case, not the exception —
    /// would flip every position into [`CompletionContext::Expression`]:
    /// `receiver.na` would lose the `Dot` that makes it a member access,
    /// and a fresh statement's first typed letter would lose the
    /// `Newline`/`Indent` that makes it a statement start.
    pub(crate) fn context_token_before(model: &SemanticModel, offset: usize) -> Option<usize> {
        use crate::lexer::Token;
        let i = Self::token_before(model, offset)?;
        match &model.tokens()[i] {
            Token::Ident(_) if model.spans()[i].end == offset => {
                if i == 0 {
                    None
                } else {
                    Some(i - 1)
                }
            }
            _ => Some(i),
        }
    }

    fn completion_context(model: &SemanticModel, offset: usize) -> CompletionContext {
        use crate::lexer::Token;
        match Self::context_token_before(model, offset) {
            None => CompletionContext::StatementStart,
            Some(i) => match &model.tokens()[i] {
                Token::Dot | Token::DotDot => CompletionContext::Member(i.wrapping_sub(1)),
                Token::From | Token::Import => CompletionContext::Import,
                Token::Newline
                | Token::Indent
                | Token::Dedent
                | Token::Colon
                | Token::Semicolon => CompletionContext::StatementStart,
                _ => CompletionContext::Expression,
            },
        }
    }

    /// Field/method completions for `receiver.`. Tries to *infer* the
    /// receiver's type first (see [`infer_receiver_type`]): a known
    /// `data`/`class` constructor narrows to that type's own fields; a
    /// built-in type (`String`, `List`, ...) narrows to just the methods
    /// tagged for it. When inference can't determine a type — the common
    /// case in a dynamically-typed program — falls back to every field
    /// known anywhere in the file plus every built-in instance method,
    /// since a string, list, or dict is exactly as plausible a receiver
    /// as any `data`/`struct` instance. UFCS-callable one-arg free
    /// functions are offered in every case: `craw_ufcs_call` accepts them
    /// for any receiver type, not just a specific one.
    fn member_completions(model: &SemanticModel, receiver_tok: usize) -> Vec<CompletionItem> {
        let receiver_name = model.tokens().get(receiver_tok).and_then(|t| match t {
            crate::lexer::Token::Ident(name) => Some(name.as_str()),
            _ => None,
        });
        let receiver_at = model.spans().get(receiver_tok).map_or(0, |s| s.start);
        let inferred =
            receiver_name.and_then(|name| infer_receiver_type(model, name, receiver_at, 0));

        let mut items = Vec::new();
        match inferred {
            Some(InferredReceiver::Custom(ctor)) => {
                for field in model.children(ctor.id) {
                    items.push(field_completion_item(field));
                }
            }
            Some(InferredReceiver::Named(type_name)) => {
                for m in crate::semantic::builtin_methods() {
                    if m.types.contains(&type_name) {
                        items.push(builtin_method_completion_item(m));
                    }
                }
            }
            None => {
                let mut seen = std::collections::HashSet::new();
                for sym in model.symbols() {
                    if sym.kind == SymbolKind::Field && seen.insert(sym.name.clone()) {
                        items.push(field_completion_item(sym));
                    }
                }
                for m in crate::semantic::builtin_methods() {
                    items.push(builtin_method_completion_item(m));
                }
            }
        }
        items.extend(ufcs_candidates(model).into_iter().map(|f| {
            let (command, commit_characters) = callable_extras();
            CompletionItem {
                label: f.name.clone(),
                kind: Some(CompletionItemKind::METHOD),
                detail: f.signature.as_ref().map(|s| render_signature(&f.name, s)),
                sort_text: Some(format!("1{}", f.name)),
                command,
                commit_characters,
                ..Default::default()
            }
        }));
        items
    }

    /// Hover for a keyword token under the cursor.
    fn keyword_hover(model: &SemanticModel, offset: usize) -> Option<Hover> {
        let (i, _) = model
            .tokens()
            .iter()
            .zip(model.spans())
            .enumerate()
            .find(|(_, (t, s))| {
                s.start <= offset && offset < s.end && crate::ide::keyword_docs::is_keyword_token(t)
            })?;
        let span = model.spans()[i];
        let word = crate::ide::token_text(model, i);
        let value = crate::ide::keyword_docs::keyword_hover(word)?;
        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value,
            }),
            range: Some(model.line_index().range(span)),
        })
    }

    pub fn get_definition(&self, uri: &str, line: u32, col: u32) -> Option<Location> {
        let model = &self.documents.get(uri)?.model;
        // An imported name, `module.name`, or a name in an import
        // statement: its definition in the other file.
        let offset = Self::offset(model, line, col)?;
        if let Some(tok) = crate::lsp_modules::ident_token_at(model, offset)
            && let Some(loc) = self.cross_target(uri, model, tok).and_then(|t| t.location())
        {
            return Some(loc);
        }
        let (_, sym) = Self::symbol_at(model, line, col)?;
        Some(Location {
            uri: Url::parse(uri).ok()?,
            range: model.line_index().range(sym.name_span),
        })
    }

    /// Hover markdown: a fenced signature (or, for a plain binder, just its
    /// name) followed by its doc comment or a short kind/scope description.
    /// Builtins and built-in types render their static documentation the
    /// same way, so hover looks the same whether a name is user-defined or
    /// not.
    pub fn get_hover_info(&self, uri: &str, line: u32, col: u32) -> Option<Hover> {
        let model = &self.documents.get(uri)?.model;
        let offset = Self::offset(model, line, col)?;
        if let Some(tok) = crate::lsp_modules::ident_token_at(model, offset)
            && let Some(t) = self.cross_target(uri, model, tok)
        {
            let range = model.line_index().range(model.spans()[tok]);
            return Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: self.cross_hover(&t),
                }),
                range: Some(range),
            });
        }
        let Some(r) = Self::reference_at(model, offset) else {
            return Self::keyword_hover(model, offset);
        };
        let (value, range) = match &r.target {
            Resolution::Symbol(id) => {
                let sym = model.symbol(*id);
                (
                    render_symbol_hover(model, sym, r.span.start),
                    model.line_index().range(sym.name_span),
                )
            }
            Resolution::Field(ids) => {
                let value = match ids.as_slice() {
                    [id] => {
                        let field = model.symbol(*id);
                        let owner = field
                            .parent
                            .map(|p| model.symbol(p).name.clone())
                            .unwrap_or_default();
                        format!("```craw\nfield {} of {owner}\n```", field.name)
                    }
                    _ => format!("```craw\n{}\n```\n\n(field)", r.name),
                };
                (value, model.line_index().range(r.span))
            }
            Resolution::Builtin(name) => {
                let doc = crate::semantic::builtin_doc(name);
                let header = doc.map(|d| d.signature).unwrap_or(name);
                (
                    render_doc_hover(header, doc.map(|d| d.markdown)),
                    model.line_index().range(r.span),
                )
            }
            Resolution::BuiltinType(name) => (
                render_doc_hover(name, crate::semantic::builtin_type_doc(name)),
                model.line_index().range(r.span),
            ),
            Resolution::Unresolved => return None,
        };
        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value,
            }),
            range: Some(range),
        })
    }

    pub fn find_references(&self, uri: &str, line: u32, col: u32) -> Option<Vec<Location>> {
        let model = &self.documents.get(uri)?.model;
        let offset = Self::offset(model, line, col)?;
        if let Some(target) = self.project_target_at(uri, offset) {
            let locations = self.project_occurrences(&target, false);
            if !locations.is_empty() {
                return Some(locations);
            }
        }
        let (id, _) = Self::symbol_at(model, line, col)?;
        let url = Url::parse(uri).ok()?;
        Some(
            model
                .references_to(id)
                .into_iter()
                .map(|r| Location {
                    uri: url.clone(),
                    range: model.line_index().range(r.span),
                })
                .collect(),
        )
    }

    /// Occurrences of the symbol under the cursor within this document,
    /// tagged `WRITE` for its definition and any assignment target,
    /// `READ` for every other use — for the editor's "highlight all
    /// occurrences of the selected symbol" feature.
    pub fn document_highlight(
        &self,
        uri: &str,
        line: u32,
        col: u32,
    ) -> Option<Vec<DocumentHighlight>> {
        let model = &self.documents.get(uri)?.model;
        let (id, _) = Self::symbol_at(model, line, col)?;
        Some(
            model
                .references_to(id)
                .into_iter()
                .map(|r| DocumentHighlight {
                    range: model.line_index().range(r.span),
                    kind: Some(match r.kind {
                        RefKind::Definition | RefKind::Write => DocumentHighlightKind::WRITE,
                        RefKind::Read => DocumentHighlightKind::READ,
                    }),
                })
                .collect(),
        )
    }

    /// `textDocument/prepareRename`: whether the position under the cursor
    /// can be renamed at all, before the user is asked for a new name.
    /// Keywords, builtins, unresolved names, and field/attribute
    /// references are not user-defined symbols this analyzer can safely
    /// rewrite everywhere, so they refuse here rather than in `rename`.
    pub fn prepare_rename(&self, uri: &str, line: u32, col: u32) -> Option<(Range, String)> {
        let model = &self.documents.get(uri)?.model;
        let offset = Self::offset(model, line, col)?;
        match self.rename_scope(uri, offset) {
            crate::lsp_modules::RenameScope::Refuse => None,
            crate::lsp_modules::RenameScope::Project(_) => {
                let tok = crate::lsp_modules::ident_token_at(model, offset)?;
                let span = model.spans()[tok];
                Some((
                    model.line_index().range(span),
                    crate::ide::token_text(model, tok).to_string(),
                ))
            }
            crate::lsp_modules::RenameScope::Local => {
                let (_, sym) = Self::symbol_at(model, line, col)?;
                Some((model.line_index().range(sym.name_span), sym.name.clone()))
            }
        }
    }

    /// Renames the symbol under the cursor everywhere it's referenced in
    /// this document. `Err` names *why* the rename was refused (an
    /// invalid identifier, or a name that collides with a keyword) so the
    /// client can show it, rather than silently doing nothing; `Ok(None)`
    /// means there's simply nothing renameable at that position.
    pub fn rename_symbol(
        &self,
        uri: &str,
        line: u32,
        col: u32,
        new_name: String,
    ) -> Result<Option<WorkspaceEdit>, String> {
        if !is_valid_identifier(&new_name) {
            return Err(format!("`{new_name}` is not a valid identifier"));
        }
        if KEYWORDS.contains(&new_name.as_str()) {
            return Err(format!("`{new_name}` is a keyword"));
        }
        let Some(model) = self.documents.get(uri).map(|d| &d.model) else {
            return Ok(None);
        };
        let Some(offset) = Self::offset(model, line, col) else {
            return Ok(None);
        };
        match self.rename_scope(uri, offset) {
            crate::lsp_modules::RenameScope::Refuse => {
                return Err("this name can't be renamed (rename the module's file instead)".to_string());
            }
            crate::lsp_modules::RenameScope::Project(target) => {
                let changes = self.project_rename(&target, &new_name);
                return Ok((!changes.is_empty()).then_some(WorkspaceEdit {
                    changes: Some(changes),
                    document_changes: None,
                    change_annotations: None,
                }));
            }
            crate::lsp_modules::RenameScope::Local => {}
        }
        let Some((id, _)) = Self::symbol_at(model, line, col) else {
            return Ok(None);
        };
        let url = Url::parse(uri).ok();
        let refs: Vec<Location> = model
            .references_to(id)
            .into_iter()
            .filter_map(|r| {
                Some(Location {
                    uri: url.clone()?,
                    range: model.line_index().range(r.span),
                })
            })
            .collect();
        let edits: Vec<TextEdit> = refs
            .into_iter()
            .map(|r| TextEdit {
                range: r.range,
                new_text: new_name.clone(),
            })
            .collect();
        let mut changes = HashMap::new();
        let Ok(url) = Url::parse(uri) else {
            return Ok(None);
        };
        changes.insert(url, edits);
        Ok(Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        }))
    }

    /// Fills in a completion item's documentation from the `data` field
    /// stashed at build time (`completionItem/resolve`), rather than
    /// rendering markdown for every item up front — most never get
    /// resolved, since the user usually only looks at the ones the
    /// completion popup shows before picking one.
    pub fn resolve_completion(&self, mut item: CompletionItem) -> CompletionItem {
        if item.documentation.is_some() {
            return item;
        }
        let Some(data) = item.data.as_ref() else {
            return item;
        };
        if let Some(builtin) = data.get("builtin").and_then(|v| v.as_str())
            && let Some(doc) = crate::semantic::builtin_doc(builtin)
        {
            item.documentation = Some(Documentation::String(doc.markdown.to_string()));
        } else if let Some(name) = data.get("builtin_method").and_then(|v| v.as_str())
            && let Some(doc) = crate::semantic::builtin_method_doc(name)
        {
            item.documentation = Some(Documentation::String(doc.markdown.to_string()));
        }
        item
    }

    fn model(&self, uri: &str) -> Option<&SemanticModel> {
        self.documents.get(uri).map(|d| &*d.model)
    }

    fn span_of(model: &SemanticModel, range: Range) -> Option<crate::text::Span> {
        let li = model.line_index();
        let start = li.offset(range.start.into())?;
        let end = li.offset(range.end.into()).unwrap_or(model.text().len());
        Some(crate::text::Span::new(start, end.max(start)))
    }

    /// `textDocument/semanticTokens/full` (`range: None`) and `/range`.
    pub fn semantic_tokens(
        &self,
        uri: &str,
        range: Option<Range>,
    ) -> Option<Vec<tower_lsp::lsp_types::SemanticToken>> {
        let model = self.model(uri)?;
        let span = match range {
            Some(r) => Some(Self::span_of(model, r)?),
            None => None,
        };
        Some(crate::ide::semantic_tokens::encode(
            model,
            span,
            &self.cross_file(uri, model),
        ))
    }

    pub fn folding_ranges(&self, uri: &str) -> Option<Vec<tower_lsp::lsp_types::FoldingRange>> {
        Some(crate::ide::folding::folding_ranges(self.model(uri)?))
    }

    pub fn selection_ranges(
        &self,
        uri: &str,
        positions: &[tower_lsp::lsp_types::Position],
    ) -> Option<Vec<tower_lsp::lsp_types::SelectionRange>> {
        let model = self.model(uri)?;
        positions
            .iter()
            .map(|p| {
                let offset = model.line_index().offset((*p).into())?;
                Some(crate::ide::selection::selection_range(model, offset))
            })
            .collect()
    }

    pub fn inlay_hints(
        &self,
        uri: &str,
        range: Range,
    ) -> Option<Vec<tower_lsp::lsp_types::InlayHint>> {
        let model = self.model(uri)?;
        let span = Self::span_of(model, range)?;
        Some(crate::ide::inlay_hints::inlay_hints(
            model,
            span,
            self.config.inlay,
            &self.cross_file(uri, model),
        ))
    }

    /// Code actions for `range`. Quick fixes are computed from the
    /// document's *current* diagnostics overlapping the range (rather
    /// than the client's copy, which may lag an edit behind).
    pub fn code_actions(
        &self,
        uri: &str,
        range: Range,
    ) -> Option<Vec<tower_lsp::lsp_types::CodeAction>> {
        let model = self.model(uri)?;
        let url = Url::parse(uri).ok()?;
        let span = Self::span_of(model, range)?;
        let diags: Vec<Diagnostic> = self
            .build_diagnostics(uri, model)
            .into_iter()
            .filter(|d| {
                let li = model.line_index();
                match (
                    li.offset(d.range.start.into()),
                    li.offset(d.range.end.into()),
                ) {
                    (Some(a), Some(b)) => crate::ide::overlaps(crate::text::Span::new(a, b), span),
                    _ => false,
                }
            })
            .collect();
        let mut actions = crate::ide::code_actions::code_actions(model, &url, span, &diags);
        actions.extend(self.native_import_actions(uri, model, &diags));
        Some(actions)
    }

    /// `textDocument/formatting` (`range: None`) and `/rangeFormatting`
    /// (whole lines touched by `range`).
    pub fn format(&self, uri: &str, range: Option<Range>) -> Option<Vec<TextEdit>> {
        let model = self.model(uri)?;
        let lines = range.map(|r| {
            let end = if r.end.character == 0 && r.end.line > r.start.line {
                r.end.line - 1
            } else {
                r.end.line
            };
            (r.start.line, end)
        });
        Some(crate::ide::formatting::format_edits(model, lines))
    }

    pub fn prepare_call_hierarchy(
        &self,
        uri: &str,
        line: u32,
        col: u32,
    ) -> Option<Vec<tower_lsp::lsp_types::CallHierarchyItem>> {
        let model = self.model(uri)?;
        let offset = Self::offset(model, line, col)?;
        let url = Url::parse(uri).ok()?;
        crate::ide::call_hierarchy::prepare(model, &url, offset).map(|i| vec![i])
    }

    pub fn incoming_calls(
        &self,
        item: &tower_lsp::lsp_types::CallHierarchyItem,
    ) -> Option<Vec<tower_lsp::lsp_types::CallHierarchyIncomingCall>> {
        let model = self.model(item.uri.as_str())?;
        let id = crate::ide::call_hierarchy::resolve_item(model, item)?;
        Some(crate::ide::call_hierarchy::incoming(model, &item.uri, id))
    }

    pub fn outgoing_calls(
        &self,
        item: &tower_lsp::lsp_types::CallHierarchyItem,
    ) -> Option<Vec<tower_lsp::lsp_types::CallHierarchyOutgoingCall>> {
        let model = self.model(item.uri.as_str())?;
        let id = crate::ide::call_hierarchy::resolve_item(model, item)?;
        Some(crate::ide::call_hierarchy::outgoing(model, &item.uri, id))
    }

    /// `textDocument/typeDefinition`: for a name whose type is inferable
    /// as a user `data`/`class`, that type's definition.
    pub fn type_definition(&self, uri: &str, line: u32, col: u32) -> Option<Location> {
        let model = self.model(uri)?;
        let offset = Self::offset(model, line, col)?;
        let r = Self::reference_at(model, offset)?;
        let ty = match &r.target {
            Resolution::Symbol(id) => {
                let sym = model.symbol(*id);
                match sym.kind {
                    SymbolKind::DataClass | SymbolKind::Class | SymbolKind::Struct => Some(sym),
                    _ => match infer_receiver_type(model, &r.name, r.span.start, 0)? {
                        InferredReceiver::Custom(t) => Some(t),
                        InferredReceiver::Named(_) => None,
                    },
                }
            }
            _ => None,
        }?;
        Some(Location {
            uri: Url::parse(uri).ok()?,
            range: model.line_index().range(ty.name_span),
        })
    }

    /// `craw/syntaxTree`: the parsed (error-recovered) AST, for debugging
    /// the parser from the editor.
    pub fn syntax_tree(&self, uri: &str) -> Option<String> {
        let model = self.model(uri)?;
        let mut out = format!("{:#?}", model.ast());
        if !model.syntax_errors().is_empty() {
            out.push_str("\n\n// syntax errors:\n");
            for e in model.syntax_errors() {
                let p = model.line_index().position(e.span.start);
                out.push_str(&format!(
                    "// {}:{}: {}\n",
                    p.line + 1,
                    p.character + 1,
                    e.message
                ));
            }
        }
        Some(out)
    }

    /// The document's current text (for `craw/transpile`).
    pub fn text(&self, uri: &str) -> Option<String> {
        Some(self.model(uri)?.text().to_string())
    }

    /// Whether the document parsed without syntax errors.
    pub fn has_syntax_errors(&self, uri: &str) -> Option<bool> {
        Some(!self.model(uri)?.syntax_errors().is_empty())
    }

    pub fn get_document_symbols(&self, uri: &str) -> Option<Vec<DocumentSymbol>> {
        let model = &self.documents.get(uri)?.model;
        let module = model.module_scope();
        let mut out: Vec<DocumentSymbol> = model
            .symbols()
            .iter()
            .filter(|s| s.parent.is_none() && s.scope == module)
            .map(|s| to_document_symbol(model, s))
            .collect();
        out.sort_by_key(|s| (s.range.start.line, s.range.start.character));
        Some(out)
    }
}

/// The keyword a symbol's own definition would start with, for symbols that
/// carry a [`crate::semantic::Signature`] (functions, generators, methods,
/// data classes) — `None` for everything else, which renders as a plain
/// name instead of a signature.
pub(crate) fn hover_prefix(kind: SymbolKind) -> Option<&'static str> {
    match kind {
        SymbolKind::Function | SymbolKind::Method => Some("def"),
        SymbolKind::Generator => Some("gen"),
        SymbolKind::DataClass => Some("data"),
        _ => None,
    }
}

pub(crate) fn signature_header(
    prefix: Option<&str>,
    name: &str,
    sig: &crate::semantic::Signature,
) -> String {
    let mut s = String::new();
    if let Some(p) = prefix {
        s.push_str(p);
        s.push(' ');
    }
    s.push_str(&render_signature(name, sig));
    if let Some(rt) = &sig.return_type {
        s.push_str(" -> ");
        s.push_str(rt);
    }
    s
}

/// A short parenthetical for a symbol with no signature: where a plain
/// variable lives, or which type a field belongs to.
fn describe_symbol(model: &SemanticModel, sym: &Symbol) -> String {
    match sym.kind {
        SymbolKind::Variable if sym.scope == model.module_scope() => {
            "variable, module scope".to_string()
        }
        SymbolKind::Field => match sym.parent.map(|p| model.symbol(p).name.clone()) {
            Some(owner) => format!("field of {owner}"),
            None => "field".to_string(),
        },
        other => kind_label(other).to_string(),
    }
}

pub(crate) fn render_symbol_hover(model: &SemanticModel, sym: &Symbol, at: usize) -> String {
    match &sym.signature {
        Some(sig) => {
            let header = signature_header(hover_prefix(sym.kind), &sym.name, sig);
            let mut s = format!("```craw\n{header}\n```");
            if let Some(docs) = &sym.docs {
                s.push_str(&format!("\n\n---\n{docs}"));
            }
            s
        }
        None => {
            // A binder's type: its annotation, else whatever inference can
            // prove from its assignments, seen from the hovered use.
            let ty = match sym.kind {
                SymbolKind::Variable | SymbolKind::Parameter => {
                    sym.declared_type.clone().or_else(|| {
                        infer_receiver_type(model, &sym.name, at, 0).map(crate::ide::type_name)
                    })
                }
                _ => None,
            };
            let mut s = match ty {
                Some(t) => format!("```craw\n{}: {t}\n```", sym.name),
                None => format!("```craw\n{}\n```", sym.name),
            };
            match &sym.docs {
                Some(docs) => s.push_str(&format!("\n\n---\n{docs}")),
                None => s.push_str(&format!("\n\n({})", describe_symbol(model, sym))),
            }
            s
        }
    }
}

fn render_doc_hover(header: &str, docs: Option<&str>) -> String {
    let mut s = format!("```craw\n{header}\n```");
    if let Some(d) = docs {
        s.push_str(&format!("\n\n---\n{d}"));
    }
    s
}

fn kind_label(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function => "fn",
        SymbolKind::Generator => "gen",
        SymbolKind::Method => "method",
        SymbolKind::Parameter => "parameter",
        SymbolKind::Variable => "variable",
        SymbolKind::DataClass => "data",
        SymbolKind::Field => "field",
        SymbolKind::Struct => "struct",
        SymbolKind::Trait => "trait",
        SymbolKind::Impl => "impl",
        SymbolKind::Class => "class",
        SymbolKind::Template => "template",
        SymbolKind::Macro => "macro",
        SymbolKind::Operator => "operator",
        SymbolKind::Import => "import",
        SymbolKind::Builtin => "builtin",
        SymbolKind::BuiltinType => "type",
    }
}

/// Maps a semantic-model [`SymbolKind`] to the closest LSP `SymbolKind`,
/// for document- and workspace-symbol responses.
pub fn lsp_symbol_kind(kind: SymbolKind) -> tower_lsp::lsp_types::SymbolKind {
    use tower_lsp::lsp_types::SymbolKind as K;
    match kind {
        SymbolKind::Function | SymbolKind::Generator | SymbolKind::Template | SymbolKind::Macro => {
            K::FUNCTION
        }
        SymbolKind::Method => K::METHOD,
        SymbolKind::Parameter | SymbolKind::Variable | SymbolKind::Import => K::VARIABLE,
        SymbolKind::DataClass | SymbolKind::Class => K::CLASS,
        SymbolKind::Field => K::FIELD,
        SymbolKind::Struct => K::STRUCT,
        SymbolKind::Trait => K::INTERFACE,
        SymbolKind::Impl => K::NAMESPACE,
        SymbolKind::Operator => K::OPERATOR,
        SymbolKind::Builtin | SymbolKind::BuiltinType => K::CONSTANT,
    }
}

#[allow(deprecated)]
fn to_document_symbol(model: &SemanticModel, sym: &Symbol) -> DocumentSymbol {
    let children: Vec<DocumentSymbol> = model
        .children(sym.id)
        .into_iter()
        .map(|c| to_document_symbol(model, c))
        .collect();
    DocumentSymbol {
        name: sym.name.clone(),
        detail: sym.signature.as_ref().map(|s| {
            s.params
                .iter()
                .map(|p| p.name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        }),
        kind: lsp_symbol_kind(sym.kind),
        tags: None,
        deprecated: None,
        range: model.line_index().range(sym.full_span),
        selection_range: model.line_index().range(sym.name_span),
        children: Some(children).filter(|c| !c.is_empty()),
    }
}

fn field_completion_item(field: &Symbol) -> CompletionItem {
    CompletionItem {
        label: field.name.clone(),
        kind: Some(CompletionItemKind::FIELD),
        sort_text: Some(format!("0{}", field.name)),
        ..Default::default()
    }
}

fn builtin_method_completion_item(m: &crate::semantic::BuiltinMethod) -> CompletionItem {
    let (command, commit_characters) = callable_extras();
    CompletionItem {
        label: m.doc.name.to_string(),
        kind: Some(CompletionItemKind::METHOD),
        detail: Some(m.doc.signature.to_string()),
        data: Some(serde_json::json!({ "builtin_method": m.doc.name })),
        sort_text: Some(format!("1{}", m.doc.name)),
        command,
        commit_characters,
        ..Default::default()
    }
}

enum CompletionContext {
    /// `receiver.` — the index is the receiver's own identifier token
    /// (`i - 1` from the `.`), if there is one.
    Member(usize),
    Import,
    StatementStart,
    Expression,
}

/// Every name visible by plain lookup at `offset`, walking outward through
/// enclosing scopes (innermost binding wins on a name collision), as
/// `VARIABLE`-kind completions sorted ahead of builtins and keywords.
fn local_completions(model: &SemanticModel, offset: usize) -> Vec<CompletionItem> {
    let scope = model.scope_at(offset);
    let mut visible: HashMap<String, &Symbol> = HashMap::new();
    let mut current = Some(scope);
    while let Some(id) = current {
        let s = &model.scopes()[id.0 as usize];
        for (name, sym_id) in &s.bindings {
            visible
                .entry(name.clone())
                .or_insert_with(|| model.symbol(*sym_id));
        }
        current = s.parent;
    }
    let mut entries: Vec<_> = visible.into_iter().collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
        .into_iter()
        .map(|(name, sym)| {
            let callable = sym.signature.is_some();
            let (command, commit_characters) = if callable {
                callable_extras()
            } else {
                (None, None)
            };
            let detail = match &sym.signature {
                Some(sig) => Some(signature_header(hover_prefix(sym.kind), &sym.name, sig)),
                None => sym.declared_type.clone().or_else(|| {
                    matches!(sym.kind, SymbolKind::Variable)
                        .then(|| infer_receiver_type(model, &sym.name, offset, 0))
                        .flatten()
                        .map(crate::ide::type_name)
                }),
            };
            CompletionItem {
                sort_text: Some(format!("0{name}")),
                kind: Some(completion_kind(sym.kind)),
                detail,
                documentation: sym.docs.clone().map(|d| {
                    Documentation::MarkupContent(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: d,
                    })
                }),
                label: name,
                command,
                commit_characters,
                ..Default::default()
            }
        })
        .collect()
}

/// The completion icon for a symbol kind.
pub(crate) fn completion_kind(kind: SymbolKind) -> CompletionItemKind {
    match kind {
        SymbolKind::Function | SymbolKind::Generator | SymbolKind::Builtin => {
            CompletionItemKind::FUNCTION
        }
        SymbolKind::Method => CompletionItemKind::METHOD,
        SymbolKind::Parameter | SymbolKind::Variable => CompletionItemKind::VARIABLE,
        SymbolKind::DataClass | SymbolKind::Class => CompletionItemKind::CLASS,
        SymbolKind::Field => CompletionItemKind::FIELD,
        SymbolKind::Struct => CompletionItemKind::STRUCT,
        SymbolKind::Trait => CompletionItemKind::INTERFACE,
        SymbolKind::Impl => CompletionItemKind::MODULE,
        SymbolKind::Template | SymbolKind::Macro => CompletionItemKind::SNIPPET,
        SymbolKind::Operator => CompletionItemKind::OPERATOR,
        SymbolKind::Import => CompletionItemKind::MODULE,
        SymbolKind::BuiltinType => CompletionItemKind::TYPE_PARAMETER,
    }
}

/// Keywords that can begin or continue an *expression* (as opposed to a
/// statement), offered in expression position after locals and builtins.
const EXPRESSION_KEYWORDS: &[&str] = &[
    "not", "and", "or", "is", "in", "if", "then", "else", "for", "lambda", "True", "False", "None",
];

fn expression_keyword_completions() -> Vec<CompletionItem> {
    EXPRESSION_KEYWORDS
        .iter()
        .map(|kw| CompletionItem {
            label: kw.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            sort_text: Some(format!("2{kw}")),
            ..Default::default()
        })
        .collect()
}

fn builtin_completions() -> Vec<CompletionItem> {
    crate::transpiler::BUILTINS
        .iter()
        .filter_map(|(name, _)| crate::semantic::builtin_doc(name))
        .map(|doc| {
            let (command, commit_characters) = callable_extras();
            CompletionItem {
                label: doc.name.to_string(),
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some(doc.signature.to_string()),
                data: Some(serde_json::json!({ "builtin": doc.name })),
                sort_text: Some(format!("1{}", doc.name)),
                command,
                commit_characters,
                ..Default::default()
            }
        })
        .collect()
}

fn keyword_completions() -> Vec<CompletionItem> {
    KEYWORDS
        .iter()
        .map(|kw| CompletionItem {
            label: kw.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            sort_text: Some(format!("2{kw}")),
            ..Default::default()
        })
        .collect()
}

/// `(label, snippet body, one-line description)`. Bodies use standard LSP
/// snippet syntax (`$1`, `${1:placeholder}`, final stop `$0`).
const SNIPPETS: &[(&str, &str, &str)] = &[
    (
        "def",
        "def ${1:name}(${2:args}):\n    $0",
        "Function definition",
    ),
    (
        "gen",
        "gen ${1:name}(${2:args}):\n    $0",
        "Generator definition",
    ),
    ("data", "data ${1:Name}(${2:fields})", "Data class"),
    (
        "class",
        "class ${1:Name}(${2:args}):\n    $0",
        "Class definition",
    ),
    (
        "match",
        "match ${1:expr}:\n    case ${2:pattern}:\n        $0",
        "Pattern match",
    ),
    ("case", "case ${1:pattern}:\n    $0", "Match arm"),
    ("for", "for ${1:item} in ${2:iterable}:\n    $0", "For loop"),
    ("while", "while ${1:cond}:\n    $0", "While loop"),
    ("if", "if ${1:cond}:\n    $0", "If statement"),
    (
        "template",
        "template ${1:name}(${2:args}):\n    $0",
        "Template definition",
    ),
    (
        "macro",
        "macro ${1:name}(${2:args}):\n    $0",
        "Macro definition",
    ),
    ("rust", "rust:\n    $0", "Inline Rust block"),
    ("lambda", "${1:x} => $0", "Lambda"),
    ("where", "${1:expr} where:\n    $0", "Where clause"),
];

fn snippet_completions() -> Vec<CompletionItem> {
    SNIPPETS
        .iter()
        .map(|(label, body, doc)| CompletionItem {
            label: label.to_string(),
            kind: Some(CompletionItemKind::SNIPPET),
            insert_text: Some(body.to_string()),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            documentation: Some(tower_lsp::lsp_types::Documentation::String(doc.to_string())),
            sort_text: Some(format!("3{label}")),
            ..Default::default()
        })
        .collect()
}

/// Standard-library module paths recognized after `from`/`import`. There
/// is no real module registry to introspect yet, so this is a fixed list
/// of the paths the language reference and samples actually use.
pub(crate) const IMPORT_PATHS: &[&str] = &["std.math", "std.collections", "std.thread", "std.fs"];

fn import_completions() -> Vec<CompletionItem> {
    IMPORT_PATHS
        .iter()
        .map(|path| CompletionItem {
            label: path.to_string(),
            kind: Some(CompletionItemKind::MODULE),
            ..Default::default()
        })
        .collect()
}

/// A concrete type inferred for a receiver expression, well enough to
/// narrow member completion: either one of Craw's own built-in value
/// kinds (a tag matching [`crate::semantic::BuiltinMethod::types`]), or a
/// user `data`/`class` symbol (whose own fields/methods already apply).
#[derive(Clone, Copy)]
pub(crate) enum InferredReceiver<'a> {
    Named(&'static str),
    Custom(&'a Symbol),
}

/// Builtins whose return type is knowable regardless of their arguments
/// — deliberately conservative: `sum`/`product`/`abs` depend on their
/// input's type, so they're left out rather than guessed at.
const BUILTIN_RETURN_TYPES: &[(&str, &str)] = &[
    ("str", "String"),
    ("list", "List"),
    ("range", "List"),
    ("len", "Int"),
    ("count", "Int"),
];

/// The built-in type-tag names a return-type annotation or inference
/// result can name (matching [`crate::semantic::BuiltinMethod::types`]).
const NAMED_TYPES: &[&str] = &[
    "Int", "Float", "String", "Bool", "List", "Dict", "Set", "Tuple",
];

/// Best-effort type inference over the subset of expression shapes
/// completion actually needs to narrow member lists: literals, a
/// same-typed ternary, string/list concatenation and repetition, and
/// calls to a constructor or to a builtin/user `def` with a statically
/// known return type. Returns `None` when the expression's type
/// genuinely can't be determined from its own shape alone — no control-
/// flow or cross-call-site analysis is attempted, so most expressions in
/// a real program will fall here, and that's the honest answer: it's
/// exactly when the broader "every field/method" fallback belongs.
///
/// `depth` bounds how many named-variable hops a single inference can
/// chase (`x = y`, `y = z`, ...) so a cycle — even one reached indirectly
/// through a binary op or ternary branch, e.g. `a = a + b` — can never
/// recurse forever; every recursive call in this module threads it
/// through rather than restarting it at the top, which is what makes the
/// bound actually hold.
pub(crate) fn infer_expr_type<'a>(
    model: &'a SemanticModel,
    expr: &crate::ast::Expr,
    at: usize,
    depth: u8,
) -> Option<InferredReceiver<'a>> {
    use crate::ast::Expr;
    if depth > 8 {
        return None;
    }
    match expr {
        Expr::String(_) | Expr::FString(_) => Some(InferredReceiver::Named("String")),
        Expr::Number(_) => Some(InferredReceiver::Named("Int")),
        Expr::Float(_) => Some(InferredReceiver::Named("Float")),
        Expr::Bool(_) => Some(InferredReceiver::Named("Bool")),
        Expr::List(_) | Expr::Comprehension(..) => Some(InferredReceiver::Named("List")),
        Expr::Dict(_) => Some(InferredReceiver::Named("Dict")),
        Expr::Set(_) => Some(InferredReceiver::Named("Set")),
        Expr::Frozenset(_) => Some(InferredReceiver::Named("Frozenset")),
        Expr::Multiset(_) => Some(InferredReceiver::Named("Multiset")),
        Expr::Tuple(_) => Some(InferredReceiver::Named("Tuple")),
        Expr::Ternary(_, a, b) => {
            match (
                infer_expr_type(model, a, at, depth)?,
                infer_expr_type(model, b, at, depth)?,
            ) {
                (InferredReceiver::Named(a), InferredReceiver::Named(b)) if a == b => {
                    Some(InferredReceiver::Named(a))
                }
                (InferredReceiver::Custom(a), InferredReceiver::Custom(b)) if a.id == b.id => {
                    Some(InferredReceiver::Custom(a))
                }
                _ => None,
            }
        }
        Expr::BinaryOp(l, op, r) => infer_binop_type(model, l, op, r, at, depth),
        Expr::Call(callee, _) => infer_call_type(model, callee, depth),
        Expr::Ident(name) => infer_receiver_type(model, name, at, depth + 1),
        _ => None,
    }
}

/// Numeric promotion matching `craw_add2`/`craw_sub2`/`craw_mul2`/
/// `craw_mod2`/`craw_pow2` in `runtime.rs`, which all agree: `Int op Int`
/// stays `Int`; any mix with a `Float` promotes to `Float`. (`craw_pow2`
/// has one narrower exception — `Int ** negative Int` is `Float` — too
/// rare to be worth losing the common case over.)
fn numeric_promotion<'a>(
    a: Option<InferredReceiver<'a>>,
    b: Option<InferredReceiver<'a>>,
) -> Option<InferredReceiver<'a>> {
    match (a?, b?) {
        (InferredReceiver::Named("Int"), InferredReceiver::Named("Int")) => {
            Some(InferredReceiver::Named("Int"))
        }
        (InferredReceiver::Named(x), InferredReceiver::Named(y))
            if matches!(x, "Int" | "Float") && matches!(y, "Int" | "Float") =>
        {
            Some(InferredReceiver::Named("Float"))
        }
        _ => None,
    }
}

/// Binary-operator type rules, each matching a specific `craw_*2`
/// function in `runtime.rs`: `+` is String concatenation whenever either
/// side is a String (`craw_add2` coerces the other side via `Display`
/// unconditionally), List/Tuple concatenation when both sides match, or
/// numeric promotion; `-`/`%`/`**` are numeric promotion; `/` is always
/// `Float` (`craw_div2` never returns an `Int`, even for `Int / Int`);
/// `*` is String/List repetition against an `Int` (either order) or
/// numeric promotion; comparisons and `and`/`or` are always `Bool`.
fn infer_binop_type<'a>(
    model: &'a SemanticModel,
    l: &crate::ast::Expr,
    op: &str,
    r: &crate::ast::Expr,
    at: usize,
    depth: u8,
) -> Option<InferredReceiver<'a>> {
    let lt = infer_expr_type(model, l, at, depth);
    let rt = infer_expr_type(model, r, at, depth);
    match op {
        "+" => {
            if matches!(lt, Some(InferredReceiver::Named("String")))
                || matches!(rt, Some(InferredReceiver::Named("String")))
            {
                return Some(InferredReceiver::Named("String"));
            }
            match (lt, rt) {
                (Some(InferredReceiver::Named(a)), Some(InferredReceiver::Named(b)))
                    if a == b && matches!(a, "List" | "Tuple") =>
                {
                    Some(InferredReceiver::Named(a))
                }
                _ => numeric_promotion(lt, rt),
            }
        }
        "-" | "%" | "**" => numeric_promotion(lt, rt),
        "/" => match (lt?, rt?) {
            (InferredReceiver::Named(a), InferredReceiver::Named(b))
                if matches!(a, "Int" | "Float") && matches!(b, "Int" | "Float") =>
            {
                Some(InferredReceiver::Named("Float"))
            }
            _ => None,
        },
        "*" => match (lt, rt) {
            (
                Some(InferredReceiver::Named(a @ ("String" | "List"))),
                Some(InferredReceiver::Named("Int")),
            ) => Some(InferredReceiver::Named(a)),
            (
                Some(InferredReceiver::Named("Int")),
                Some(InferredReceiver::Named(b @ ("String" | "List"))),
            ) => Some(InferredReceiver::Named(b)),
            _ => numeric_promotion(lt, rt),
        },
        "==" | "!=" | "<" | "<=" | ">" | ">=" | "and" | "or" => {
            Some(InferredReceiver::Named("Bool"))
        }
        _ => None,
    }
}

/// Resolves a rendered type string (a `-> Type`/`name: Type` annotation's
/// [`crate::semantic::type_to_string`] text, generics stripped to their
/// base name) to either a built-in type tag or a known `data`/`class`.
fn resolve_named_type<'a>(model: &'a SemanticModel, raw: &str) -> Option<InferredReceiver<'a>> {
    let base = raw.split('<').next().unwrap_or(raw);
    if let Some(named) = NAMED_TYPES.iter().find(|t| **t == base) {
        return Some(InferredReceiver::Named(named));
    }
    model
        .symbol_named(base)
        .filter(|s| matches!(s.kind, SymbolKind::DataClass | SymbolKind::Class))
        .map(InferredReceiver::Custom)
}

/// A call's type: a known `data`/`class` constructor, a user `def` with
/// an explicit `-> Type` annotation (naming either a built-in type or
/// another known `data`/`class`), or one of [`BUILTIN_RETURN_TYPES`].
fn infer_call_type<'a>(
    model: &'a SemanticModel,
    callee: &crate::ast::Expr,
    depth: u8,
) -> Option<InferredReceiver<'a>> {
    let crate::ast::Expr::Ident(name) = callee else {
        return None;
    };
    if let Some(sym) = model.symbol_named(name) {
        if matches!(sym.kind, SymbolKind::DataClass | SymbolKind::Class) {
            return Some(InferredReceiver::Custom(sym));
        }
        if let Some(sig) = &sym.signature {
            if let Some(rt) = &sig.return_type {
                if let Some(resolved) = resolve_named_type(model, rt) {
                    return Some(resolved);
                }
            } else if matches!(sym.kind, SymbolKind::Function)
                && let Some(t) = infer_function_return(model, sym.def_token, depth + 1)
            {
                return Some(t);
            }
        }
    }
    BUILTIN_RETURN_TYPES
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, t)| InferredReceiver::Named(t))
}

/// The type of the variable `name` as seen from byte offset `at`: `name`
/// is resolved through the scopes enclosing `at` (so a same-named local
/// in some other function never leaks in), then typed from its declared
/// annotation or, failing that, from the nearest assignment to that same
/// binding before `at` (or the last one anywhere, for a module variable
/// read by a function defined above its assignment). `depth` guards
/// against infinite recursion on a self-referential alias (`x = x`); Craw
/// programs don't nest variable aliasing more than a handful of levels
/// deep, so a small bound never affects a real completion.
pub(crate) fn infer_receiver_type<'a>(
    model: &'a SemanticModel,
    name: &str,
    at: usize,
    depth: u8,
) -> Option<InferredReceiver<'a>> {
    if depth > 8 {
        return None;
    }
    let target = model.lookup(name, model.scope_at(at))?;
    // An explicit annotation (a typed parameter or `x: Type = ...`
    // assignment) is the primary signal, trusted ahead of any structural
    // guess — exactly why rust-analyzer treats a declared type as
    // authoritative rather than merely another inference hint.
    if let Some(t) = &model.symbol(target).declared_type
        && let Some(resolved) = resolve_named_type(model, t)
    {
        return Some(resolved);
    }
    fn collect<'s>(
        model: &SemanticModel,
        stmts: &'s [crate::ast::Stmt],
        name: &str,
        target: SymbolId,
        out: &mut Vec<(usize, &'s crate::ast::Expr)>,
    ) {
        for stmt in stmts {
            match stmt {
                crate::ast::Stmt::Assign(crate::ast::Pattern::Var(n, _), expr, tok)
                    if n == name =>
                {
                    if let Some(span) = model.spans().get(*tok)
                        && model.lookup(name, model.scope_at(span.start)) == Some(target)
                    {
                        out.push((span.start, expr));
                    }
                }
                crate::ast::Stmt::FunctionDef { body, .. } => {
                    collect(model, body, name, target, out);
                }
                crate::ast::Stmt::If(_, body, else_body) => {
                    collect(model, body, name, target, out);
                    collect(model, else_body, name, target, out);
                }
                crate::ast::Stmt::While(_, body)
                | crate::ast::Stmt::MatchFor(_, _, body, _)
                | crate::ast::Stmt::ClassDef { body, .. }
                | crate::ast::Stmt::TraitDef(_, body, _)
                | crate::ast::Stmt::ImplBlock(_, _, body, _) => {
                    collect(model, body, name, target, out);
                }
                crate::ast::Stmt::Match(_, arms, _) => {
                    for (_, _, body) in arms {
                        collect(model, body, name, target, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut assigns = Vec::new();
    collect(model, model.ast(), name, target, &mut assigns);
    let (pos, expr) = assigns
        .iter()
        .rev()
        .find(|(pos, _)| *pos < at)
        .or(assigns.last())
        .copied()?;
    infer_expr_type(model, expr, pos, depth)
}

/// Every `return` expression in a function body, not descending into
/// nested functions (whose returns are their own). `bare` is set if any
/// `return` has no value.
pub(crate) fn collect_returns<'s>(
    body: &'s [crate::ast::Stmt],
    out: &mut Vec<&'s crate::ast::Expr>,
    bare: &mut bool,
) {
    use crate::ast::{Expr, Stmt};
    for stmt in body {
        match stmt {
            Stmt::Return(Expr::None) => *bare = true,
            Stmt::Return(e) => out.push(e),
            Stmt::If(_, a, b) => {
                collect_returns(a, out, bare);
                collect_returns(b, out, bare);
            }
            Stmt::While(_, b) | Stmt::MatchFor(_, _, b, _) => collect_returns(b, out, bare),
            Stmt::Match(_, arms, _) => {
                for (_, _, b) in arms {
                    collect_returns(b, out, bare);
                }
            }
            _ => {}
        }
    }
}

/// The return type of the unannotated, non-generator `def` whose `def`
/// token is `def_token`: the type every `return` agrees on, as seen from
/// inside the body. `None` if any return is bare or uninferable, or if
/// they disagree.
pub(crate) fn infer_function_return(
    model: &SemanticModel,
    def_token: usize,
    depth: u8,
) -> Option<InferredReceiver<'_>> {
    use crate::ast::Stmt;
    use crate::lexer::Token;
    if depth > 8 {
        return None;
    }
    let mut found = None;
    crate::ide::walk_stmts(model.ast(), &mut |stmt| {
        if let Stmt::FunctionDef {
            id,
            body,
            return_type: None,
            is_generator: false,
            ..
        } = stmt
            && *id == def_token
        {
            found = Some(body);
        }
    });
    let body = found?;
    let mut exprs = Vec::new();
    let mut bare = false;
    collect_returns(body, &mut exprs, &mut bare);
    if exprs.is_empty() || bare {
        return None;
    }
    let tokens = model.tokens();
    let spans = model.spans();
    let open = (def_token..tokens.len()).find(|&i| matches!(tokens[i], Token::LParen))?;
    let close = crate::ide::matching_close(tokens, open)?;
    // The first body token: after `):` NEWLINE INDENT, or right after `:`
    // for a one-line `def f(x): return x`.
    let body_start = match (tokens.get(close + 2), tokens.get(close + 3)) {
        (Some(Token::Newline), Some(Token::Indent)) => close + 4,
        _ => close + 2,
    };
    let inside = spans.get(body_start)?.start;
    let mut result: Option<InferredReceiver<'_>> = None;
    for e in exprs {
        let t = infer_expr_type(model, e, inside, depth)?;
        match result {
            Some(prev) if crate::ide::type_name(prev) != crate::ide::type_name(t) => return None,
            _ => result = Some(t),
        }
    }
    result
}

/// Every top-level function taking exactly one required parameter (no
/// default, no vararg) — callable UFCS-style as `receiver.f()`.
fn ufcs_candidates(model: &SemanticModel) -> Vec<&Symbol> {
    model
        .symbols()
        .iter()
        .filter(|s| matches!(s.kind, SymbolKind::Function))
        .filter(|s| {
            s.signature.as_ref().is_some_and(|sig| {
                sig.vararg.is_none() && sig.params.len() == 1 && sig.params[0].default.is_none()
            })
        })
        .collect()
}

/// Counts depth-0 commas strictly between `lparen` and `offset` — i.e. how
/// many arguments have already been completed before the cursor's current
/// one. Commas at or after `offset` (already-typed arguments *after*
/// where the cursor currently is) don't count.
fn count_commas_before_cursor(
    tokens: &[crate::lexer::Token],
    spans: &[crate::text::Span],
    lparen: usize,
    offset: usize,
) -> usize {
    use crate::lexer::Token;
    let mut depth = 0i32;
    let mut count = 0usize;
    for i in (lparen + 1)..tokens.len() {
        if spans[i].start >= offset {
            break;
        }
        match &tokens[i] {
            Token::LParen | Token::LBracket | Token::LBrace => depth += 1,
            Token::RParen | Token::RBracket | Token::RBrace => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            Token::Comma if depth == 0 => count += 1,
            _ => {}
        }
    }
    count
}

/// The parameter labels of a builtin's display signature
/// (`"filter(pred, xs)"` → `["pred", "xs"]`), split at top-level commas.
fn builtin_params(signature: &str) -> Vec<String> {
    let (Some(open), Some(close)) = (signature.find('('), signature.rfind(')')) else {
        return Vec::new();
    };
    let inner = &signature[open + 1..close];
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for c in inner.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                out.push(cur.trim().to_string());
                cur.clear();
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// If the argument the cursor is in starts with `name=`, that name.
fn current_keyword_argument(
    tokens: &[crate::lexer::Token],
    spans: &[crate::text::Span],
    lparen: usize,
    offset: usize,
) -> Option<String> {
    use crate::lexer::Token;
    let mut depth = 0i32;
    let mut arg_start = lparen + 1;
    for i in (lparen + 1)..tokens.len() {
        if spans[i].start >= offset {
            break;
        }
        match &tokens[i] {
            Token::LParen | Token::LBracket | Token::LBrace => depth += 1,
            Token::RParen | Token::RBracket | Token::RBrace => depth -= 1,
            Token::Comma if depth == 0 => arg_start = i + 1,
            _ => {}
        }
    }
    match (tokens.get(arg_start), tokens.get(arg_start + 1)) {
        (Some(Token::Ident(n)), Some(Token::Assign)) if spans[arg_start + 1].start < offset => {
            Some(n.clone())
        }
        _ => None,
    }
}

pub(crate) fn render_signature(name: &str, sig: &crate::semantic::Signature) -> String {
    let params: Vec<String> = sig
        .params
        .iter()
        .map(|p| match &p.default {
            Some(d) => format!("{}={}", p.name, d),
            None => p.name.clone(),
        })
        .collect();
    let vararg = sig.vararg.as_ref().map(|v| format!("*{v}"));
    let all: Vec<String> = params.into_iter().chain(vararg).collect();
    format!("{name}({})", all.join(", "))
}

/// Whether `name` could be a Craw identifier at all: an ASCII letter or
/// underscore, then any number of letters/digits/underscores. Used only to
/// validate a *rename target* — real identifiers may include non-ASCII
/// letters, but nobody renames a symbol to a name the lexer itself
/// wouldn't accept plainly typed.
pub(crate) fn is_valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub(crate) const KEYWORDS: &[&str] = &[
    "let",
    "def",
    "fn",
    "gen",
    "data",
    "type",
    "struct",
    "trait",
    "impl",
    "enum",
    "template",
    "macro",
    "class",
    "extends",
    "with",
    "copyclosure",
    "addpattern",
    "if",
    "else",
    "match",
    "case",
    "while",
    "for",
    "in",
    "break",
    "continue",
    "return",
    "yield",
    "then",
    "to",
    "until",
    "where",
    "from",
    "import",
    "use",
    "as",
    "global",
    "nonlocal",
    "select",
    "orderby",
    "ascending",
    "descending",
    "not",
    "and",
    "or",
    "is",
    "true",
    "false",
    "none",
    "True",
    "False",
    "None",
    "operator",
];
