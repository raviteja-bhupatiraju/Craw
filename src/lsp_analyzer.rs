use crate::ast::{Pattern, Stmt, Type};
use std::collections::HashMap;
use syn::Item;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, Diagnostic, Hover, HoverContents, Location, MarkedString,
    Position, Range, TextEdit, Url, WorkspaceEdit,
};

#[derive(Debug)]
pub struct DocumentState {
    pub text: String,
    pub scope_tree: ScopeTree,
    pub document_symbols: Vec<tower_lsp::lsp_types::DocumentSymbol>,
}

#[derive(Debug, Clone)]
pub struct VariableDef {
    pub name: String,
    pub line: u32,
    pub col: u32,
    pub range: Range,
    pub kind: Option<String>,
}

impl VariableDef {
    pub fn new(name: String, line: u32, col: u32, kind: Option<String>) -> Self {
        let name_len = name.encode_utf16().count() as u32;
        Self {
            name,
            line,
            col,
            range: Range {
                start: Position {
                    line,
                    character: col,
                },
                end: Position {
                    line,
                    character: col + name_len,
                },
            },
            kind,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScopeNode {
    pub start_line: u32,
    pub end_line: u32,
    pub variables: Vec<VariableDef>,
    pub children: Vec<ScopeNode>,
}

#[derive(Debug, Clone, Default)]
pub struct ScopeTree {
    pub root: ScopeNode,
}

fn type_to_string(t: &Type) -> String {
    match t {
        Type::Int => "Int".to_string(),
        Type::String => "String".to_string(),
        Type::Custom(name) => name.clone(),
        Type::Generic(name, params) => {
            let params_str: Vec<String> = params.iter().map(type_to_string).collect();
            format!("{}<{}>", name, params_str.join(", "))
        }
        Type::Dynamic => "Dynamic".to_string(),
    }
}

impl Default for ScopeNode {
    fn default() -> Self {
        Self {
            start_line: 0,
            end_line: u32::MAX,
            variables: Vec::new(),
            children: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub struct LspAnalyzer {
    documents: HashMap<String, DocumentState>,
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
        }
    }

    pub fn update_document(&mut self, uri: &str, text: &str) -> Vec<Diagnostic> {
        let mut lexer = crate::lexer::Lexer::new(text);
        let tokens = lexer.tokenize();

        let mut diags = Vec::new();
        let mut scope_tree = ScopeTree::default();
        let mut parse_success = false;
        let mut document_symbols = Vec::new();

        match crate::parser::parse(&tokens) {
            Ok(stmts) => {
                let mut builder = ScopeBuilder::new(text);
                document_symbols =
                    self.populate_scope_tree(&stmts, &mut scope_tree.root, &mut builder);
                self.normalize_end_lines(&mut scope_tree.root);
                parse_success = true;
            }
            Err(errors) => {
                eprintln!("LSP Parse Errors for input '{}': {:?}", text, errors);
                for err in errors {
                    let span = err.span();
                    // A simple approximation of line numbers: count newlines before the error token
                    let mut line = 0;
                    for token in tokens.iter().take(span.start) {
                        if let crate::lexer::Token::Newline = token {
                            line += 1;
                        }
                    }

                    let range = Range {
                        start: Position {
                            line: line as u32,
                            character: 0,
                        },
                        end: Position {
                            line: line as u32,
                            character: 1,
                        },
                    };

                    diags.push(Diagnostic {
                        range,
                        severity: Some(tower_lsp::lsp_types::DiagnosticSeverity::ERROR),
                        message: format!("Syntax error: {:?}", err),
                        ..Default::default()
                    });
                }
            }
        }

        let document_symbols = if parse_success {
            document_symbols
        } else if let Some(old_doc) = self.documents.get(uri) {
            old_doc.document_symbols.clone()
        } else {
            document_symbols
        };
        let final_scope_tree = if parse_success {
            scope_tree
        } else if let Some(old_doc) = self.documents.get(uri) {
            old_doc.scope_tree.clone()
        } else {
            scope_tree
        };

        self.documents.insert(
            uri.to_string(),
            DocumentState {
                text: text.to_string(),
                scope_tree: final_scope_tree,
                document_symbols,
            },
        );

        diags
    }

    fn normalize_end_lines(&self, node: &mut ScopeNode) {
        let child_count = node.children.len();
        for i in 0..child_count {
            let mut next_bound = if i + 1 < child_count {
                node.children[i + 1].start_line
            } else {
                node.end_line
            };

            // A child scope cannot extend past the next variable defined in the parent scope
            if let Some(next_var_line) = node
                .variables
                .iter()
                .map(|v| v.line)
                .filter(|&l| l > node.children[i].end_line)
                .min()
            {
                next_bound = next_bound.min(next_var_line.saturating_sub(1));
            }

            // Ensure we don't shrink the scope if it already had a valid end_line
            node.children[i].end_line = node.children[i].end_line.max(next_bound);
        }

        for child in &mut node.children {
            self.normalize_end_lines(child);
        }
    }

    #[allow(deprecated)]
    fn populate_scope_tree(
        &self,
        stmts: &[Stmt],
        node: &mut ScopeNode,
        builder: &mut ScopeBuilder,
    ) -> Vec<tower_lsp::lsp_types::DocumentSymbol> {
        let mut symbols = Vec::new();
        for stmt in stmts {
            match stmt {
                Stmt::Assign(pat, _) => self.extract_pattern_vars(pat, node, builder),
                Stmt::FunctionDef {
                    name, args, body, ..
                } => {
                    let child_symbols;
                    let mut symbol_range = None;
                    let mut symbol_name = None;

                    if let Some(n) = name.last() {
                        symbol_name = Some(n.clone());
                        let (line, col) = builder.find_ident(n);
                        node.variables.push(VariableDef::new(
                            n.clone(),
                            line,
                            col,
                            Some("fn".to_string()),
                        ));

                        let name_len = n.encode_utf16().count() as u32;
                        symbol_range = Some(tower_lsp::lsp_types::Range {
                            start: tower_lsp::lsp_types::Position {
                                line,
                                character: col,
                            },
                            end: tower_lsp::lsp_types::Position {
                                line,
                                character: col + name_len,
                            },
                        });
                    }

                    let mut child_node = ScopeNode {
                        start_line: builder.line,
                        ..Default::default()
                    };

                    for (pat, _) in args {
                        self.extract_pattern_vars(pat, &mut child_node, builder);
                    }
                    child_symbols = self.populate_scope_tree(body, &mut child_node, builder);

                    child_node.end_line = builder.line;
                    node.children.push(child_node);

                    if let (Some(name), Some(range)) = (symbol_name, symbol_range) {
                        let mut full_range = range;
                        full_range.end.line = builder.line;
                        symbols.push(tower_lsp::lsp_types::DocumentSymbol {
                            name,
                            detail: None,
                            kind: tower_lsp::lsp_types::SymbolKind::FUNCTION,
                            tags: None,
                            deprecated: None,
                            range: full_range,
                            selection_range: range,
                            children: Some(child_symbols).filter(|c| !c.is_empty()),
                        });
                    }
                }
                Stmt::DataDef(name, variants, _) => {
                    let (line, col) = builder.find_ident(name);
                    node.variables.push(VariableDef::new(
                        name.clone(),
                        line,
                        col,
                        Some("data".to_string()),
                    ));

                    let name_len = name.encode_utf16().count() as u32;
                    let symbol_range = tower_lsp::lsp_types::Range {
                        start: tower_lsp::lsp_types::Position {
                            line,
                            character: col,
                        },
                        end: tower_lsp::lsp_types::Position {
                            line,
                            character: col + name_len,
                        },
                    };

                    let mut child_symbols = Vec::new();
                    for (variant_name, _, _) in variants {
                        let (v_line, v_col) = builder.find_ident(variant_name);
                        let v_len = variant_name.encode_utf16().count() as u32;
                        let v_range = tower_lsp::lsp_types::Range {
                            start: tower_lsp::lsp_types::Position {
                                line: v_line,
                                character: v_col,
                            },
                            end: tower_lsp::lsp_types::Position {
                                line: v_line,
                                character: v_col + v_len,
                            },
                        };
                        child_symbols.push(tower_lsp::lsp_types::DocumentSymbol {
                            name: variant_name.clone(),
                            detail: None,
                            kind: tower_lsp::lsp_types::SymbolKind::ENUM_MEMBER,
                            tags: None,
                            deprecated: None,
                            range: v_range,
                            selection_range: v_range,
                            children: None,
                        });
                    }

                    let mut full_range = symbol_range;
                    full_range.end.line = builder.line;
                    symbols.push(tower_lsp::lsp_types::DocumentSymbol {
                        name: name.clone(),
                        detail: None,
                        kind: tower_lsp::lsp_types::SymbolKind::ENUM,
                        tags: None,
                        deprecated: None,
                        range: full_range,
                        selection_range: symbol_range,
                        children: Some(child_symbols).filter(|c| !c.is_empty()),
                    });
                }
                Stmt::StructDef(name, fields, _) => {
                    let (line, col) = builder.find_ident(name);
                    node.variables.push(VariableDef::new(
                        name.clone(),
                        line,
                        col,
                        Some("struct".to_string()),
                    ));

                    let name_len = name.encode_utf16().count() as u32;
                    let symbol_range = tower_lsp::lsp_types::Range {
                        start: tower_lsp::lsp_types::Position {
                            line,
                            character: col,
                        },
                        end: tower_lsp::lsp_types::Position {
                            line,
                            character: col + name_len,
                        },
                    };

                    let mut child_symbols = Vec::new();
                    for (field_name, _) in fields {
                        let (f_line, f_col) = builder.find_ident(field_name);
                        let f_len = field_name.encode_utf16().count() as u32;
                        let f_range = tower_lsp::lsp_types::Range {
                            start: tower_lsp::lsp_types::Position {
                                line: f_line,
                                character: f_col,
                            },
                            end: tower_lsp::lsp_types::Position {
                                line: f_line,
                                character: f_col + f_len,
                            },
                        };
                        child_symbols.push(tower_lsp::lsp_types::DocumentSymbol {
                            name: field_name.clone(),
                            detail: None,
                            kind: tower_lsp::lsp_types::SymbolKind::FIELD,
                            tags: None,
                            deprecated: None,
                            range: f_range,
                            selection_range: f_range,
                            children: None,
                        });
                    }

                    let mut full_range = symbol_range;
                    full_range.end.line = builder.line;
                    symbols.push(tower_lsp::lsp_types::DocumentSymbol {
                        name: name.clone(),
                        detail: None,
                        kind: tower_lsp::lsp_types::SymbolKind::STRUCT,
                        tags: None,
                        deprecated: None,
                        range: full_range,
                        selection_range: symbol_range,
                        children: Some(child_symbols).filter(|c| !c.is_empty()),
                    });
                }
                Stmt::TraitDef(name, body, _) => {
                    let (line, col) = builder.find_ident(name);
                    node.variables.push(VariableDef::new(
                        name.clone(),
                        line,
                        col,
                        Some("trait".to_string()),
                    ));

                    let mut child_node = ScopeNode {
                        start_line: builder.line,
                        ..Default::default()
                    };
                    let child_symbols = self.populate_scope_tree(body, &mut child_node, builder);
                    child_node.end_line = builder.line;
                    node.children.push(child_node);

                    let name_len = name.encode_utf16().count() as u32;
                    let symbol_range = tower_lsp::lsp_types::Range {
                        start: tower_lsp::lsp_types::Position {
                            line,
                            character: col,
                        },
                        end: tower_lsp::lsp_types::Position {
                            line,
                            character: col + name_len,
                        },
                    };
                    let mut full_range = symbol_range;
                    full_range.end.line = builder.line;
                    symbols.push(tower_lsp::lsp_types::DocumentSymbol {
                        name: name.clone(),
                        detail: None,
                        kind: tower_lsp::lsp_types::SymbolKind::INTERFACE,
                        tags: None,
                        deprecated: None,
                        range: full_range,
                        selection_range: symbol_range,
                        children: Some(child_symbols).filter(|c| !c.is_empty()),
                    });
                }
                Stmt::ImplBlock(trait_name, target_type, body, _) => {
                    let mut child_node = ScopeNode {
                        start_line: builder.line,
                        ..Default::default()
                    };
                    let child_symbols = self.populate_scope_tree(body, &mut child_node, builder);
                    child_node.end_line = builder.line;

                    let full_range = tower_lsp::lsp_types::Range {
                        start: tower_lsp::lsp_types::Position {
                            line: child_node.start_line,
                            character: 0,
                        },
                        end: tower_lsp::lsp_types::Position {
                            line: builder.line,
                            character: 0,
                        },
                    };

                    let name = match trait_name {
                        Some(tr) => format!(
                            "impl {} for {}",
                            type_to_string(tr),
                            type_to_string(target_type)
                        ),
                        None => format!("impl {}", type_to_string(target_type)),
                    };

                    symbols.push(tower_lsp::lsp_types::DocumentSymbol {
                        name,
                        detail: None,
                        kind: tower_lsp::lsp_types::SymbolKind::NAMESPACE,
                        tags: None,
                        deprecated: None,
                        range: full_range,
                        selection_range: full_range,
                        children: Some(child_symbols).filter(|c| !c.is_empty()),
                    });
                    node.children.push(child_node);
                }
                Stmt::If(_, body) | Stmt::While(_, body) => {
                    let mut child_node = ScopeNode {
                        start_line: builder.line,
                        ..Default::default()
                    };
                    let child_symbols = self.populate_scope_tree(body, &mut child_node, builder);
                    symbols.extend(child_symbols);
                    child_node.end_line = builder.line;
                    node.children.push(child_node);
                }
                Stmt::Match(_, arms) => {
                    for (pat, _, body) in arms {
                        let mut child_node = ScopeNode {
                            start_line: builder.line,
                            ..Default::default()
                        };
                        self.extract_pattern_vars(pat, &mut child_node, builder);
                        let child_symbols =
                            self.populate_scope_tree(body, &mut child_node, builder);
                        symbols.extend(child_symbols);
                        child_node.end_line = builder.line;
                        node.children.push(child_node);
                    }
                }
                Stmt::MatchFor(pat, _, body) => {
                    let mut child_node = ScopeNode {
                        start_line: builder.line,
                        ..Default::default()
                    };
                    self.extract_pattern_vars(pat, &mut child_node, builder);
                    let child_symbols = self.populate_scope_tree(body, &mut child_node, builder);
                    symbols.extend(child_symbols);
                    child_node.end_line = builder.line;
                    node.children.push(child_node);
                }
                Stmt::Passthrough(code) => {
                    if code.starts_with("let ") {
                        let parts: Vec<&str> = code.split_whitespace().collect();
                        if parts.len() >= 2 {
                            let var_name = parts[1].trim_end_matches(':').trim_end_matches(';');
                            let (line, col) = builder.find_ident(var_name);
                            node.variables.push(VariableDef::new(
                                var_name.to_string(),
                                line,
                                col,
                                Some("variable".to_string()),
                            ));
                        }
                    } else if let Ok(file) = syn::parse_file(code) {
                        for item in file.items {
                            if let Item::Fn(f) = item {
                                let name = f.sig.ident.to_string();
                                if name != "main" {
                                    let (line, col) = builder.find_ident(&name);
                                    node.variables.push(VariableDef::new(
                                        name,
                                        line,
                                        col,
                                        Some("fn".to_string()),
                                    ));
                                }
                            }
                        }
                    }
                }
                Stmt::Use(path) => {
                    let parts: Vec<&str> = path.split("::").collect();
                    if let Some(last) = parts.last() {
                        let last = last.trim();
                        if last.starts_with('{') && last.ends_with('}') {
                            let subparts = last[1..last.len() - 1].split(',');
                            for sub in subparts {
                                let name = sub.trim().split(" as ").last().unwrap().trim();
                                if !name.is_empty() && name != "self" {
                                    let (line, col) = builder.find_ident(name);
                                    node.variables.push(VariableDef::new(
                                        name.to_string(),
                                        line,
                                        col,
                                        Some("variable".to_string()),
                                    ));
                                }
                            }
                        } else {
                            let name = last.split(" as ").last().unwrap().trim();
                            if !name.is_empty() && name != "*" {
                                let (line, col) = builder.find_ident(name);
                                node.variables.push(VariableDef::new(
                                    name.to_string(),
                                    line,
                                    col,
                                    Some("variable".to_string()),
                                ));
                            }
                        }
                    }
                }
                Stmt::NativeImport(_, items) => {
                    for name in items {
                        let (line, col) = builder.find_ident(name);
                        node.variables.push(VariableDef::new(
                            name.clone(),
                            line,
                            col,
                            Some("variable".to_string()),
                        ));
                    }
                }
                _ => {}
            }
        }
        symbols
    }

    fn extract_pattern_vars(
        &self,
        pat: &Pattern,
        node: &mut ScopeNode,
        builder: &mut ScopeBuilder,
    ) {
        match pat {
            Pattern::Var(name, _) => {
                let (line, col) = builder.find_ident(name);
                node.variables.push(VariableDef::new(
                    name.clone(),
                    line,
                    col,
                    Some("variable".to_string()),
                ));
            }
            Pattern::Data(_, fields) => {
                for field in fields {
                    self.extract_pattern_vars(field, node, builder);
                }
            }
            Pattern::View(_, inner) => self.extract_pattern_vars(inner, node, builder),
            Pattern::Tuple(items) => {
                for item in items {
                    self.extract_pattern_vars(item, node, builder);
                }
            }
            _ => {}
        }
    }

    pub fn get_completions(&self, uri: &str, line: u32, col: u32) -> Vec<CompletionItem> {
        let mut completions = Vec::new();

        if let Some(doc) = self.get_document(uri) {
            let mut visible_vars = Vec::new();
            self.collect_visible_vars(&doc.scope_tree.root, line, col, &mut visible_vars);

            // Filter duplicates, keeping the innermost occurrences (which are added last)
            let mut unique_vars = std::collections::BTreeMap::new();
            for var in visible_vars {
                unique_vars.insert(var.name.clone(), var);
            }

            for (_, var) in unique_vars {
                completions.push(CompletionItem {
                    label: var.name,
                    kind: Some(CompletionItemKind::VARIABLE),
                    ..Default::default()
                });
            }
        }

        // Add basic built-ins/keywords
        for kw in [
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
        ] {
            completions.push(CompletionItem {
                label: kw.to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                ..Default::default()
            });
        }

        completions
    }

    fn collect_visible_vars(
        &self,
        node: &ScopeNode,
        line: u32,
        _col: u32,
        vars: &mut Vec<VariableDef>,
    ) {
        // Only include variables from this scope if the cursor is within this scope
        if line >= node.start_line && line <= node.end_line {
            // Add variables defined before or on the current line
            for var in &node.variables {
                if var.line <= line {
                    vars.push(var.clone());
                }
            }

            // Check children scopes. Only one child scope should contain the cursor.
            for child in &node.children {
                if line >= child.start_line && line <= child.end_line {
                    self.collect_visible_vars(child, line, _col, vars);
                    // Assume only one child scope can contain the cursor at a given time
                    break;
                }
            }
        }
    }

    fn extract_word_at(text: &str, line: u32, col: u32) -> Option<String> {
        let line_str = text.lines().nth(line as usize)?;

        // Find byte indices for UTF-16 column offset
        let mut current_utf16_col = 0;
        let mut target_byte_idx = line_str.len(); // default to end of line
        let mut prev_char_byte_idx = None;

        for (b_idx, c) in line_str.char_indices() {
            if current_utf16_col >= col {
                target_byte_idx = b_idx;
                break;
            }
            prev_char_byte_idx = Some(b_idx);
            current_utf16_col += c.len_utf16() as u32;
        }

        if target_byte_idx == line_str.len() && current_utf16_col < col {
            return None; // col is out of bounds
        }

        let mut start_byte = target_byte_idx;

        // If cursor is at the end of the line, or not on a word character,
        // try to look at the previous character.
        let is_word_char = |c: char| c.is_alphanumeric() || c == '_';

        if start_byte == line_str.len()
            || !line_str[start_byte..]
                .chars()
                .next()
                .map_or(false, is_word_char)
        {
            if let Some(prev_idx) = prev_char_byte_idx {
                if line_str[prev_idx..]
                    .chars()
                    .next()
                    .map_or(false, is_word_char)
                {
                    start_byte = prev_idx;
                } else {
                    return None;
                }
            } else {
                return None;
            }
        }

        // Now find the start and end of the word
        // Since Rust strings are UTF-8, we can find word boundaries
        // To find start boundary:
        let mut word_start = start_byte;
        for (b_idx, c) in line_str[..start_byte].char_indices().rev() {
            if is_word_char(c) {
                word_start = b_idx;
            } else {
                break;
            }
        }

        // To find end boundary:
        let mut word_end = start_byte;
        for (i, c) in line_str[start_byte..].char_indices() {
            if is_word_char(c) {
                word_end = start_byte + i + c.len_utf8();
            } else {
                break;
            }
        }

        Some(line_str[word_start..word_end].to_string())
    }

    fn resolve_symbol_at(&self, uri: &str, line: u32, col: u32) -> Option<VariableDef> {
        let doc = self.get_document(uri)?;
        let word = Self::extract_word_at(&doc.text, line, col)?;

        let mut visible_vars = Vec::new();
        self.collect_visible_vars(&doc.scope_tree.root, line, col, &mut visible_vars);

        for var in visible_vars.into_iter().rev() {
            if var.name == word {
                return Some(var);
            }
        }

        None
    }

    pub fn get_definition(&self, uri: &str, line: u32, col: u32) -> Option<Location> {
        let var = self.resolve_symbol_at(uri, line, col)?;
        Some(Location {
            uri: Url::parse(uri).ok()?,
            range: var.range,
        })
    }

    pub fn get_hover_info(&self, uri: &str, line: u32, col: u32) -> Option<Hover> {
        let var = self.resolve_symbol_at(uri, line, col)?;
        let kind_str = var.kind.as_deref().unwrap_or("variable");
        Some(Hover {
            contents: HoverContents::Scalar(MarkedString::LanguageString(
                tower_lsp::lsp_types::LanguageString {
                    language: "craw".to_string(),
                    value: format!("({}) {}", kind_str, var.name),
                },
            )),
            range: Some(var.range),
        })
    }

    pub fn find_references(&self, uri: &str, line: u32, col: u32) -> Option<Vec<Location>> {
        let target_var = self.resolve_symbol_at(uri, line, col)?;
        let doc = self.get_document(uri)?;
        let text = &doc.text;

        let mut refs = Vec::new();
        let tokens = crate::lexer::Lexer::new(text).tokenize();

        let mut cursor = 0;
        let mut prev_token_was_dot = false;

        for token in tokens {
            if let crate::lexer::Token::Ident(ref name) = token {
                if let Some(idx) = Self::find_ident_safe(text, cursor, name) {
                    cursor = idx + name.len();

                    if *name == target_var.name && !prev_token_was_dot {
                        let mut current_line = 0;
                        let mut current_col = 0;
                        for ch in text[..idx].chars() {
                            if ch == '\n' {
                                current_line += 1;
                                current_col = 0;
                            } else {
                                current_col += ch.len_utf16() as u32;
                            }
                        }

                        if let Some(resolved_var) =
                            self.resolve_symbol_at(uri, current_line, current_col)
                        {
                            if resolved_var.line == target_var.line
                                && resolved_var.col == target_var.col
                            {
                                let name_len_utf16 = target_var.name.encode_utf16().count() as u32;
                                refs.push(Location {
                                    uri: Url::parse(uri).unwrap(),
                                    range: Range {
                                        start: Position {
                                            line: current_line,
                                            character: current_col,
                                        },
                                        end: Position {
                                            line: current_line,
                                            character: current_col + name_len_utf16,
                                        },
                                    },
                                });
                            }
                        }
                    }
                }
            }
            prev_token_was_dot = matches!(token, crate::lexer::Token::Dot);
        }

        Some(refs)
    }

    fn find_ident_safe(text: &str, mut cursor: usize, target: &str) -> Option<usize> {
        while cursor < text.len() {
            let ch = text[cursor..].chars().next().unwrap();

            if ch == '#' {
                while cursor < text.len() && !text[cursor..].starts_with('\n') {
                    cursor += text[cursor..].chars().next().unwrap().len_utf8();
                }
                continue;
            }

            if ch == '"' {
                cursor += ch.len_utf8();
                while cursor < text.len() {
                    let inner_ch = text[cursor..].chars().next().unwrap();
                    cursor += inner_ch.len_utf8();
                    if inner_ch == '"' {
                        break;
                    }
                    if inner_ch == '\\' && cursor < text.len() {
                        cursor += text[cursor..].chars().next().unwrap().len_utf8();
                    }
                }
                continue;
            }

            if ch == '\'' {
                cursor += ch.len_utf8();
                while cursor < text.len() {
                    let inner_ch = text[cursor..].chars().next().unwrap();
                    cursor += inner_ch.len_utf8();
                    if inner_ch == '\'' {
                        break;
                    }
                    if inner_ch == '\\' && cursor < text.len() {
                        cursor += text[cursor..].chars().next().unwrap().len_utf8();
                    }
                }
                continue;
            }

            if ch.is_alphabetic() || ch == '_' {
                let start_cursor = cursor;
                let mut ident_len = 0;
                while cursor < text.len() {
                    let c = text[cursor..].chars().next().unwrap();
                    if c.is_alphanumeric() || c == '_' {
                        ident_len += c.len_utf8();
                        cursor += c.len_utf8();
                    } else {
                        break;
                    }
                }
                if &text[start_cursor..start_cursor + ident_len] == target {
                    return Some(start_cursor);
                }
                continue;
            }

            cursor += ch.len_utf8();
        }
        None
    }

    pub fn rename_symbol(
        &self,
        uri: &str,
        line: u32,
        col: u32,
        new_name: String,
    ) -> Option<WorkspaceEdit> {
        let refs = self.find_references(uri, line, col)?;

        let edits: Vec<TextEdit> = refs
            .into_iter()
            .map(|r| TextEdit {
                range: r.range,
                new_text: new_name.clone(),
            })
            .collect();

        let mut changes = HashMap::new();
        changes.insert(Url::parse(uri).unwrap(), edits);

        Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        })
    }

    pub fn get_document_symbols(
        &self,
        uri: &str,
    ) -> Option<Vec<tower_lsp::lsp_types::DocumentSymbol>> {
        self.get_document(uri)
            .map(|doc| doc.document_symbols.clone())
    }

    pub fn get_document(&self, uri: &str) -> Option<&DocumentState> {
        self.documents.get(uri)
    }

    pub fn remove_document(&mut self, uri: &str) {
        self.documents.remove(uri);
    }
}
struct ScopeBuilder<'a> {
    text: &'a str,
    cursor: usize,
    line: u32,
    col: u32,
}

impl<'a> ScopeBuilder<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            cursor: 0,
            line: 0,
            col: 0,
        }
    }

    fn find_ident(&mut self, name: &str) -> (u32, u32) {
        let mut search_start = self.cursor;
        while let Some(idx) = self.text[search_start..].find(name) {
            let absolute_idx = search_start + idx;

            // Check boundaries
            let is_start_boundary = if absolute_idx == 0 {
                true
            } else {
                let prev_char = self.text[..absolute_idx].chars().last().unwrap();
                !prev_char.is_alphanumeric() && prev_char != '_'
            };

            let is_end_boundary = if absolute_idx + name.len() == self.text.len() {
                true
            } else {
                let next_char = self.text[absolute_idx + name.len()..]
                    .chars()
                    .next()
                    .unwrap();
                !next_char.is_alphanumeric() && next_char != '_'
            };

            if is_start_boundary && is_end_boundary {
                let skipped = &self.text[self.cursor..absolute_idx];
                for c in skipped.chars() {
                    if c == '\n' {
                        self.line += 1;
                        self.col = 0;
                    } else {
                        self.col += c.len_utf16() as u32;
                    }
                }
                let res = (self.line, self.col);
                self.cursor = absolute_idx + name.len();
                for c in name.chars() {
                    if c == '\n' {
                        self.line += 1;
                        self.col = 0;
                    } else {
                        self.col += c.len_utf16() as u32;
                    }
                }
                return res;
            }
            search_start = absolute_idx + name.len();
        }
        (self.line, self.col)
    }
}
