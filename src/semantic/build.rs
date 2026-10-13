//! Walks a (possibly partially-recovered) AST to collect [`Symbol`]s for
//! every definition and binder, alongside the [`Scope`] tree they live in.

use super::symbols::{Param, Scope, ScopeId, ScopeKind, Signature, Symbol, SymbolId, SymbolKind};
use crate::ast::{Expr, Pattern, Stmt};
use crate::lexer::Token;
use crate::text::{LineIndex, Span};
use std::collections::HashSet;

/// Index one past the last token of the statement/definition starting at
/// `start` (which should be the definition's own first token, e.g. the
/// `def`/`data`/`class` keyword or an assignment's target): if the
/// statement's header is followed (after any blank-line `Newline`s) by an
/// `Indent`, one past that block's matching `Dedent`; otherwise the
/// position of the statement's own terminating `Newline`/`Semicolon`, or
/// the end of the token stream.
fn stmt_end(tokens: &[Token], start: usize) -> usize {
    let mut i = start;
    while i < tokens.len() {
        match &tokens[i] {
            Token::Indent => return matching_dedent(tokens, i) + 1,
            Token::Newline => {
                let mut j = i + 1;
                while matches!(tokens.get(j), Some(Token::Newline)) {
                    j += 1;
                }
                if matches!(tokens.get(j), Some(Token::Indent)) {
                    return matching_dedent(tokens, j) + 1;
                }
                return i;
            }
            Token::Semicolon | Token::Eof => return i,
            _ => i += 1,
        }
    }
    tokens.len()
}

fn matching_dedent(tokens: &[Token], indent_idx: usize) -> usize {
    let mut depth = 0i32;
    for (i, tok) in tokens.iter().enumerate().skip(indent_idx) {
        match tok {
            Token::Indent => depth += 1,
            Token::Dedent => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    tokens.len().saturating_sub(1)
}

/// Index one past `open_idx`'s matching close bracket, counting nested
/// occurrences of the same open/close pair (so a tuple default value like
/// `b=(1, 2)` inside a parameter list doesn't confuse the search for the
/// parameter list's own closing `)`).
fn matching_close(tokens: &[Token], open_idx: usize, open: &Token, close: &Token) -> usize {
    let mut depth = 0i32;
    for (i, tok) in tokens.iter().enumerate().skip(open_idx) {
        if tok == open {
            depth += 1;
        } else if tok == close {
            depth -= 1;
            if depth == 0 {
                return i;
            }
        }
    }
    tokens.len().saturating_sub(1)
}

/// First `Token::Ident(name)` in `[from, to)`.
fn find_ident(tokens: &[Token], name: &str, from: usize, to: usize) -> Option<usize> {
    (from..to.min(tokens.len())).find(|&i| matches!(&tokens[i], Token::Ident(n) if n == name))
}

/// First occurrence of `target` in `[from, to)`.
fn find_token(tokens: &[Token], target: &Token, from: usize, to: usize) -> Option<usize> {
    (from..to.min(tokens.len())).find(|&i| &tokens[i] == target)
}

/// Index of the first depth-0 `Comma` in `[from, to)` (bracket-aware, so a
/// tuple/list/call inside a default value doesn't end the scan early), or
/// `to` if none.
fn find_arg_boundary(tokens: &[Token], from: usize, to: usize) -> usize {
    let mut depth = 0i32;
    for (i, tok) in tokens
        .iter()
        .enumerate()
        .take(to.min(tokens.len()))
        .skip(from)
    {
        match tok {
            Token::LParen | Token::LBracket | Token::LBrace => depth += 1,
            Token::RParen | Token::RBracket | Token::RBrace => depth -= 1,
            Token::Comma if depth == 0 => return i,
            _ => {}
        }
    }
    to
}

/// The exact original source text spanning tokens `[from, to)`, empty if
/// the range is empty.
fn source_between(ctx: &Ctx, from: usize, to: usize) -> String {
    if from >= to {
        return String::new();
    }
    let start = ctx.spans[from].start;
    let end = ctx.spans[to - 1].end;
    ctx.line_index.text()[start..end].to_string()
}

/// The byte end of the last non-structural token before `end_idx`, used as
/// a definition's `full_span` end: `end_idx` itself is one past a
/// `Dedent`/`Newline`, whose own span sits at the *next* line, not the
/// last line of the definition's actual content.
fn last_content_end(tokens: &[Token], spans: &[Span], end_idx: usize) -> usize {
    let mut i = end_idx;
    while i > 0 {
        i -= 1;
        if !matches!(tokens[i], Token::Newline | Token::Indent | Token::Dedent) {
            return spans[i].end;
        }
    }
    spans.first().map(|s| s.start).unwrap_or(0)
}

/// Contiguous `#` comment lines immediately above `def_line`, joined with
/// `\n` and stripped of their leading `#`/`# `. `None` if the line above
/// isn't a comment.
fn leading_doc_comment(line_index: &LineIndex, def_line: u32) -> Option<String> {
    if def_line == 0 {
        return None;
    }
    let mut lines = Vec::new();
    let mut line = def_line;
    loop {
        if line == 0 {
            break;
        }
        line -= 1;
        let text = line_index.line_text(line).trim_start();
        if let Some(comment) = text.strip_prefix('#') {
            lines.push(comment.strip_prefix(' ').unwrap_or(comment).to_string());
        } else {
            break;
        }
    }
    if lines.is_empty() {
        return None;
    }
    lines.reverse();
    Some(lines.join("\n"))
}

/// Names a `global`/`nonlocal` statement directly inside `stmts` (not in a
/// nested block) declares. An assignment to one of these must not create a
/// new local binding — it should resolve to (and, at the runtime level,
/// mutate) the outer binding instead, exactly as if no local of that name
/// existed in this scope at all. That's the only special handling
/// `global`/`nonlocal` need here: plain parent-chain lookup already finds
/// the right outer binding once the local one is never created, so no
/// separate scope-redirect table is needed.
fn non_local_names(stmts: &[Stmt]) -> HashSet<String> {
    let mut names = HashSet::new();
    for stmt in stmts {
        match stmt {
            Stmt::Global(ns, _) | Stmt::Nonlocal(ns, _) => names.extend(ns.iter().cloned()),
            _ => {}
        }
    }
    names
}

pub(super) struct Ctx<'a> {
    tokens: &'a [Token],
    spans: &'a [Span],
    line_index: &'a LineIndex,
}

impl<'a> Ctx<'a> {
    pub(super) fn new(tokens: &'a [Token], spans: &'a [Span], line_index: &'a LineIndex) -> Self {
        Self {
            tokens,
            spans,
            line_index,
        }
    }
}

impl Ctx<'_> {
    /// Registers a top-level definition (one with its own statement span):
    /// the name is searched for from `def_token` through the end of the
    /// whole statement (its own body included, if any), and `full_span`
    /// covers that entire statement. Not added to any scope's `bindings`
    /// by itself — callers that want the definition name visible by plain
    /// lookup call `register` afterward.
    fn push_symbol(
        &self,
        symbols: &mut Vec<Symbol>,
        name: String,
        kind: SymbolKind,
        def_token: usize,
        scope: ScopeId,
        parent: Option<SymbolId>,
    ) -> SymbolId {
        let end_idx = stmt_end(self.tokens, def_token);
        let name_tok = find_ident(self.tokens, &name, def_token, end_idx).unwrap_or(def_token);
        let name_span = self.spans[name_tok];
        let full_span = Span::new(
            self.spans[def_token].start,
            last_content_end(self.tokens, self.spans, end_idx),
        );
        let def_line = self.line_index.position(self.spans[def_token].start).line;
        let id = SymbolId(symbols.len() as u32);
        symbols.push(Symbol {
            id,
            name,
            kind,
            name_span,
            full_span,
            scope,
            parent,
            signature: None,
            docs: leading_doc_comment(self.line_index, def_line),
            def_token,
            declared_type: None,
            import: None,
        });
        id
    }

    /// Registers a child of a container definition (a data/struct field, a
    /// class constructor argument, a method): just a single name token,
    /// searched for from `search_from` onward (bounded by `search_to`) so
    /// repeated calls for successive fields don't re-match an earlier
    /// field's name. Not added to any scope's `bindings` — a field is
    /// reached through attribute syntax, never plain-name lookup. Returns
    /// the symbol and the token index its name was found at, so the
    /// caller can advance its search cursor past it.
    #[allow(clippy::too_many_arguments)]
    fn push_child(
        &self,
        symbols: &mut Vec<Symbol>,
        name: String,
        kind: SymbolKind,
        search_from: usize,
        search_to: usize,
        scope: ScopeId,
        parent: SymbolId,
    ) -> (SymbolId, usize) {
        let name_tok =
            find_ident(self.tokens, &name, search_from, search_to).unwrap_or(search_from);
        let span = self.spans[name_tok];
        let id = SymbolId(symbols.len() as u32);
        symbols.push(Symbol {
            id,
            name,
            kind,
            name_span: span,
            full_span: span,
            scope,
            parent: Some(parent),
            signature: None,
            docs: None,
            def_token: name_tok,
            declared_type: None,
            import: None,
        });
        (id, name_tok)
    }

    /// Registers a plain-name binding (a parameter, a pattern-destructured
    /// local): a single name token found the same way as `push_child`, but
    /// also added to `scope`'s `bindings` so ordinary lookup finds it.
    /// Returns the token index one past the name, for advancing a search
    /// cursor across successive bindings from the same pattern.
    #[allow(clippy::too_many_arguments)]
    fn push_binding(
        &self,
        symbols: &mut Vec<Symbol>,
        scopes: &mut [Scope],
        write_targets: &mut Vec<Span>,
        name: String,
        kind: SymbolKind,
        search_from: usize,
        search_to: usize,
        scope: ScopeId,
        parent: Option<SymbolId>,
    ) -> usize {
        let name_tok =
            find_ident(self.tokens, &name, search_from, search_to).unwrap_or(search_from);
        let span = self.spans[name_tok];
        let id = SymbolId(symbols.len() as u32);
        symbols.push(Symbol {
            id,
            name: name.clone(),
            kind,
            name_span: span,
            full_span: span,
            scope,
            parent,
            signature: None,
            docs: None,
            def_token: name_tok,
            declared_type: None,
            import: None,
        });
        scopes[scope.0 as usize].bindings.insert(name, id);
        write_targets.push(span);
        name_tok + 1
    }
}

fn register(scopes: &mut [Scope], scope: ScopeId, name: &str, id: SymbolId) {
    scopes[scope.0 as usize]
        .bindings
        .insert(name.to_string(), id);
}

fn push_scope(
    scopes: &mut Vec<Scope>,
    parent: ScopeId,
    span: Span,
    kind: ScopeKind,
    owner: Option<SymbolId>,
) -> ScopeId {
    let id = ScopeId(scopes.len() as u32);
    scopes.push(Scope {
        id,
        parent: Some(parent),
        span,
        kind,
        bindings: Default::default(),
        owner,
    });
    id
}

/// Recursively binds every name a pattern introduces, in source order,
/// advancing `cursor` past each as it's found so a later identically-named
/// binder in the same pattern (rare, but `Rest`/`Var` names aren't required
/// to be unique) doesn't re-match an earlier one's token.
#[allow(clippy::too_many_arguments)]
fn bind_pattern(
    ctx: &Ctx,
    pat: &Pattern,
    kind: SymbolKind,
    cursor: usize,
    search_to: usize,
    scope: ScopeId,
    parent: Option<SymbolId>,
    symbols: &mut Vec<Symbol>,
    scopes: &mut [Scope],
    write_targets: &mut Vec<Span>,
) -> usize {
    match pat {
        Pattern::Var(name, ty) => {
            let next = ctx.push_binding(
                symbols,
                scopes,
                write_targets,
                name.clone(),
                kind,
                cursor,
                search_to,
                scope,
                parent,
            );
            if let Some(t) = ty
                && let Some(&id) = scopes[scope.0 as usize].bindings.get(name)
            {
                symbols[id.0 as usize].declared_type = Some(super::symbols::type_to_string(t));
            }
            next
        }
        Pattern::Rest(name) => ctx.push_binding(
            symbols,
            scopes,
            write_targets,
            name.clone(),
            kind,
            cursor,
            search_to,
            scope,
            parent,
        ),
        Pattern::Tuple(items) => {
            let mut c = cursor;
            for item in items {
                c = bind_pattern(
                    ctx,
                    item,
                    kind,
                    c,
                    search_to,
                    scope,
                    parent,
                    symbols,
                    scopes,
                    write_targets,
                );
            }
            c
        }
        Pattern::Data(_, fields) => {
            let mut c = cursor;
            for field in fields {
                c = bind_pattern(
                    ctx,
                    field,
                    kind,
                    c,
                    search_to,
                    scope,
                    parent,
                    symbols,
                    scopes,
                    write_targets,
                );
            }
            c
        }
        Pattern::View(_, inner) => bind_pattern(
            ctx,
            inner,
            kind,
            cursor,
            search_to,
            scope,
            parent,
            symbols,
            scopes,
            write_targets,
        ),
        Pattern::StringSplit(a, b, _) => {
            let c = ctx.push_binding(
                symbols,
                scopes,
                write_targets,
                a.clone(),
                kind,
                cursor,
                search_to,
                scope,
                parent,
            );
            ctx.push_binding(
                symbols,
                scopes,
                write_targets,
                b.clone(),
                kind,
                c,
                search_to,
                scope,
                parent,
            )
        }
        Pattern::Const(_) | Pattern::Wildcard => cursor,
    }
}

/// Recursively visits every sub-expression of `expr`, creating a new scope
/// (and binding its introduced names) for each `Lambda`/`Comprehension`/
/// `Where`/`Query` found, wherever it's nested.
fn walk_expr(
    ctx: &Ctx,
    expr: &Expr,
    scope: ScopeId,
    symbols: &mut Vec<Symbol>,
    scopes: &mut Vec<Scope>,
    write_targets: &mut Vec<Span>,
) {
    let (tokens, spans) = (ctx.tokens, ctx.spans);
    match expr {
        Expr::Number(_)
        | Expr::Float(_)
        | Expr::String(_)
        | Expr::Bool(_)
        | Expr::None
        | Expr::Ident(_)
        | Expr::AttributePartial(_)
        | Expr::OperatorFunction(_)
        | Expr::Passthrough(_)
        | Expr::Placeholder
        | Expr::Shell(_) => {}

        Expr::Call(f, args) | Expr::BroadcastCall(f, args) => {
            walk_expr(ctx, f, scope, symbols, scopes, write_targets);
            for a in args {
                walk_expr(ctx, a, scope, symbols, scopes, write_targets);
            }
        }
        Expr::MacroCall(_, args) => {
            for a in args {
                walk_expr(ctx, a, scope, symbols, scopes, write_targets);
            }
        }
        Expr::BinaryOp(l, _, r)
        | Expr::NoneCoalesce(l, r)
        | Expr::Range(l, r)
        | Expr::Formula(l, r) => {
            walk_expr(ctx, l, scope, symbols, scopes, write_targets);
            walk_expr(ctx, r, scope, symbols, scopes, write_targets);
        }
        Expr::Pipe(l, _, r) => {
            walk_expr(ctx, l, scope, symbols, scopes, write_targets);
            walk_expr(ctx, r, scope, symbols, scopes, write_targets);
        }
        Expr::Compose(l, _, r, _) => {
            walk_expr(ctx, l, scope, symbols, scopes, write_targets);
            walk_expr(ctx, r, scope, symbols, scopes, write_targets);
        }
        Expr::Ternary(c, t, f) => {
            walk_expr(ctx, c, scope, symbols, scopes, write_targets);
            walk_expr(ctx, t, scope, symbols, scopes, write_targets);
            walk_expr(ctx, f, scope, symbols, scopes, write_targets);
        }
        Expr::Lambda(params, body, pos) => {
            let end = stmt_end(tokens, *pos);
            let lambda_scope = push_scope(
                scopes,
                scope,
                Span::new(spans[*pos].start, last_content_end(tokens, spans, end)),
                ScopeKind::Lambda,
                None,
            );
            let mut cursor = *pos;
            for p in params {
                cursor = ctx.push_binding(
                    symbols,
                    scopes,
                    write_targets,
                    p.clone(),
                    SymbolKind::Parameter,
                    cursor,
                    end,
                    lambda_scope,
                    None,
                );
            }
            walk_expr(ctx, body, lambda_scope, symbols, scopes, write_targets);
        }
        Expr::ImplicitLambda(body, _) => {
            walk_expr(ctx, body, scope, symbols, scopes, write_targets);
        }
        Expr::PartialCall(f, args, _) => {
            walk_expr(ctx, f, scope, symbols, scopes, write_targets);
            for a in args.iter().flatten() {
                walk_expr(ctx, a, scope, symbols, scopes, write_targets);
            }
        }
        Expr::List(items)
        | Expr::Set(items)
        | Expr::Frozenset(items)
        | Expr::Multiset(items)
        | Expr::Tuple(items)
        | Expr::Hcat(items)
        | Expr::Vcat(items)
        | Expr::FString(items) => {
            for it in items {
                walk_expr(ctx, it, scope, symbols, scopes, write_targets);
            }
        }
        Expr::LazyList(items, _) => {
            for it in items {
                walk_expr(ctx, it, scope, symbols, scopes, write_targets);
            }
        }
        Expr::Dict(entries) => {
            for (k, v) in entries {
                walk_expr(ctx, k, scope, symbols, scopes, write_targets);
                walk_expr(ctx, v, scope, symbols, scopes, write_targets);
            }
        }
        Expr::Index(base, idx) => {
            walk_expr(ctx, base, scope, symbols, scopes, write_targets);
            walk_expr(ctx, idx, scope, symbols, scopes, write_targets);
        }
        Expr::Attribute(base, _)
        | Expr::OptionalAttribute(base, _)
        | Expr::Try(base)
        | Expr::Gather(base, _)
        | Expr::Splat(base)
        | Expr::IndexPartial(base) => {
            walk_expr(ctx, base, scope, symbols, scopes, write_targets);
        }
        Expr::Slice(a, b, c) => {
            for e in [a, b, c].into_iter().flatten() {
                walk_expr(ctx, e, scope, symbols, scopes, write_targets);
            }
        }
        Expr::Where(inner, stmts, pos) => {
            let end = stmt_end(tokens, *pos);
            walk_expr(ctx, inner, scope, symbols, scopes, write_targets);
            let where_scope = push_scope(
                scopes,
                scope,
                Span::new(spans[*pos].start, last_content_end(tokens, spans, end)),
                ScopeKind::Where,
                None,
            );
            collect(
                stmts,
                ctx,
                where_scope,
                None,
                symbols,
                scopes,
                write_targets,
            );
        }
        Expr::Comprehension(out, pat, iter, _, pos, filter) => {
            let end = stmt_end(tokens, *pos);
            let comp_scope = push_scope(
                scopes,
                scope,
                Span::new(spans[*pos].start, last_content_end(tokens, spans, end)),
                ScopeKind::Comprehension,
                None,
            );
            let for_idx = find_token(tokens, &Token::For, *pos, end).unwrap_or(*pos);
            let in_idx = find_token(tokens, &Token::In, for_idx, end).unwrap_or(end);
            bind_pattern(
                ctx,
                pat,
                SymbolKind::Variable,
                for_idx + 1,
                in_idx,
                comp_scope,
                None,
                symbols,
                scopes,
                write_targets,
            );
            walk_expr(ctx, out, comp_scope, symbols, scopes, write_targets);
            walk_expr(ctx, iter, comp_scope, symbols, scopes, write_targets);
            if let Some(f) = filter {
                walk_expr(ctx, f, comp_scope, symbols, scopes, write_targets);
            }
        }
        Expr::Query {
            from,
            in_expr,
            clauses,
            select,
            id,
        } => {
            let end = stmt_end(tokens, *id);
            let query_scope = push_scope(
                scopes,
                scope,
                Span::new(spans[*id].start, last_content_end(tokens, spans, end)),
                ScopeKind::Query,
                None,
            );
            walk_expr(ctx, in_expr, scope, symbols, scopes, write_targets);
            ctx.push_binding(
                symbols,
                scopes,
                write_targets,
                from.clone(),
                SymbolKind::Variable,
                *id,
                end,
                query_scope,
                None,
            );
            for clause in clauses {
                match clause {
                    crate::ast::QueryClause::Where(e) | crate::ast::QueryClause::OrderBy(e, _) => {
                        walk_expr(ctx, e, query_scope, symbols, scopes, write_targets);
                    }
                }
            }
            walk_expr(ctx, select, query_scope, symbols, scopes, write_targets);
        }
    }
}

/// Walks `stmts`, collecting a [`Symbol`] for every definition and binder
/// (and, for container definitions, recursively for the fields/methods/
/// nested scopes they introduce), appending to `symbols` and registering
/// bindings visible by plain-name lookup into `scopes[scope.0]` (or a new
/// nested scope it creates).
pub(super) fn collect(
    stmts: &[Stmt],
    ctx: &Ctx,
    scope: ScopeId,
    parent: Option<SymbolId>,
    symbols: &mut Vec<Symbol>,
    scopes: &mut Vec<Scope>,
    write_targets: &mut Vec<Span>,
) {
    let (tokens, spans) = (ctx.tokens, ctx.spans);
    let no_local = non_local_names(stmts);

    for stmt in stmts {
        match stmt {
            Stmt::FunctionDef {
                name,
                id,
                args,
                vararg,
                body,
                is_generator,
                return_type,
                ..
            } => {
                let Some(fn_name) = name.last() else {
                    continue;
                };
                let kind = if parent.is_some() {
                    SymbolKind::Method
                } else if *is_generator {
                    SymbolKind::Generator
                } else {
                    SymbolKind::Function
                };
                let sym_id = ctx.push_symbol(symbols, fn_name.clone(), kind, *id, scope, parent);
                register(scopes, scope, fn_name, sym_id);

                let end = stmt_end(tokens, *id);
                let header_end = find_token(tokens, &Token::LParen, *id, end)
                    .map(|lp| matching_close(tokens, lp, &Token::LParen, &Token::RParen))
                    .unwrap_or(end);
                let fn_scope = push_scope(
                    scopes,
                    scope,
                    Span::new(spans[*id].start, last_content_end(tokens, spans, end)),
                    ScopeKind::Function,
                    Some(sym_id),
                );
                let mut cursor = *id;
                let mut params = Vec::new();
                for (pat, default) in args {
                    let param_start = cursor;
                    cursor = bind_pattern(
                        ctx,
                        pat,
                        SymbolKind::Parameter,
                        cursor,
                        header_end,
                        fn_scope,
                        None,
                        symbols,
                        scopes,
                        write_targets,
                    );
                    let param_name = match pat {
                        Pattern::Var(n, _) | Pattern::Rest(n) => n.clone(),
                        _ => "?".to_string(),
                    };
                    let param_span = ctx.spans[param_start.min(cursor.saturating_sub(1))];
                    let default_text = default
                        .as_ref()
                        .and_then(|_| find_token(tokens, &Token::Assign, param_start, header_end))
                        .map(|assign_idx| {
                            let arg_end = find_arg_boundary(tokens, assign_idx + 1, header_end);
                            source_between(ctx, assign_idx + 1, arg_end)
                        });
                    if let Some(default_expr) = default {
                        walk_expr(ctx, default_expr, scope, symbols, scopes, write_targets);
                    }
                    params.push(Param {
                        name: param_name,
                        default: default_text,
                        span: param_span,
                    });
                }
                if let Some(v) = vararg {
                    ctx.push_binding(
                        symbols,
                        scopes,
                        write_targets,
                        v.clone(),
                        SymbolKind::Parameter,
                        cursor,
                        header_end,
                        fn_scope,
                        None,
                    );
                }
                symbols[sym_id.0 as usize].signature = Some(Signature {
                    params,
                    vararg: vararg.clone(),
                    return_type: return_type.as_ref().map(super::symbols::type_to_string),
                });
                collect(body, ctx, fn_scope, None, symbols, scopes, write_targets);
            }
            Stmt::DataDef(name, fields, id) => {
                let sym_id = ctx.push_symbol(
                    symbols,
                    name.clone(),
                    SymbolKind::DataClass,
                    *id,
                    scope,
                    parent,
                );
                register(scopes, scope, name, sym_id);
                let end = stmt_end(tokens, *id);
                let mut cursor = *id;
                let mut params = Vec::new();
                for (fname, _, default) in fields {
                    let (_, found) = ctx.push_child(
                        symbols,
                        fname.clone(),
                        SymbolKind::Field,
                        cursor,
                        end,
                        scope,
                        sym_id,
                    );
                    let default_text = default.as_ref().and_then(|_| {
                        find_token(tokens, &Token::Assign, found, end).map(|assign_idx| {
                            let arg_end = find_arg_boundary(tokens, assign_idx + 1, end);
                            source_between(ctx, assign_idx + 1, arg_end)
                        })
                    });
                    params.push(Param {
                        name: fname.clone(),
                        default: default_text,
                        span: spans[found],
                    });
                    cursor = found + 1;
                    if let Some(default_expr) = default {
                        walk_expr(ctx, default_expr, scope, symbols, scopes, write_targets);
                    }
                }
                symbols[sym_id.0 as usize].signature = Some(Signature {
                    params,
                    vararg: None,
                    return_type: None,
                });
            }
            Stmt::StructDef(name, fields, id) => {
                let sym_id = ctx.push_symbol(
                    symbols,
                    name.clone(),
                    SymbolKind::Struct,
                    *id,
                    scope,
                    parent,
                );
                register(scopes, scope, name, sym_id);
                let end = stmt_end(tokens, *id);
                let mut cursor = *id;
                for (fname, _) in fields {
                    let (_, found) = ctx.push_child(
                        symbols,
                        fname.clone(),
                        SymbolKind::Field,
                        cursor,
                        end,
                        scope,
                        sym_id,
                    );
                    cursor = found + 1;
                }
            }
            Stmt::TraitDef(name, body, id) => {
                let sym_id =
                    ctx.push_symbol(symbols, name.clone(), SymbolKind::Trait, *id, scope, parent);
                register(scopes, scope, name, sym_id);
                collect(
                    body,
                    ctx,
                    scope,
                    Some(sym_id),
                    symbols,
                    scopes,
                    write_targets,
                );
            }
            Stmt::ClassDef {
                name,
                args,
                body,
                id,
                ..
            } => {
                let sym_id =
                    ctx.push_symbol(symbols, name.clone(), SymbolKind::Class, *id, scope, parent);
                register(scopes, scope, name, sym_id);
                let end = stmt_end(tokens, *id);
                let mut cursor = *id;
                for arg in args {
                    let (_, found) = ctx.push_child(
                        symbols,
                        arg.clone(),
                        SymbolKind::Field,
                        cursor,
                        end,
                        scope,
                        sym_id,
                    );
                    cursor = found + 1;
                }
                collect(
                    body,
                    ctx,
                    scope,
                    Some(sym_id),
                    symbols,
                    scopes,
                    write_targets,
                );
            }
            Stmt::ImplBlock(_, _, body, _) => {
                collect(body, ctx, scope, parent, symbols, scopes, write_targets);
            }
            Stmt::TemplateDef(name, params, body, id) => {
                let sym_id = ctx.push_symbol(
                    symbols,
                    name.clone(),
                    SymbolKind::Template,
                    *id,
                    scope,
                    parent,
                );
                register(scopes, scope, name, sym_id);
                let end = stmt_end(tokens, *id);
                let tmpl_scope = push_scope(
                    scopes,
                    scope,
                    Span::new(spans[*id].start, last_content_end(tokens, spans, end)),
                    ScopeKind::Template,
                    Some(sym_id),
                );
                let mut cursor = *id;
                for p in params {
                    cursor = ctx.push_binding(
                        symbols,
                        scopes,
                        write_targets,
                        p.clone(),
                        SymbolKind::Parameter,
                        cursor,
                        end,
                        tmpl_scope,
                        None,
                    );
                }
                collect(body, ctx, tmpl_scope, None, symbols, scopes, write_targets);
            }
            Stmt::MacroDef {
                name,
                args,
                body,
                pos,
            } => {
                let sym_id = ctx.push_symbol(
                    symbols,
                    name.clone(),
                    SymbolKind::Macro,
                    *pos,
                    scope,
                    parent,
                );
                register(scopes, scope, name, sym_id);
                let end = stmt_end(tokens, *pos);
                let macro_scope = push_scope(
                    scopes,
                    scope,
                    Span::new(spans[*pos].start, last_content_end(tokens, spans, end)),
                    ScopeKind::Macro,
                    Some(sym_id),
                );
                let mut cursor = *pos;
                for p in args {
                    cursor = ctx.push_binding(
                        symbols,
                        scopes,
                        write_targets,
                        p.clone(),
                        SymbolKind::Parameter,
                        cursor,
                        end,
                        macro_scope,
                        None,
                    );
                }
                collect(body, ctx, macro_scope, None, symbols, scopes, write_targets);
            }
            Stmt::Assign(pat, expr, pos) => {
                walk_expr(ctx, expr, scope, symbols, scopes, write_targets);
                let end = stmt_end(tokens, *pos);
                if let Pattern::Var(name, _) = pat {
                    let already_bound = scopes[scope.0 as usize].bindings.contains_key(name);
                    if no_local.contains(name) || already_bound {
                        // `global`/`nonlocal` names, and plain reassignment
                        // of an existing local, are writes to a binding
                        // that already exists elsewhere — not a new
                        // definition, so no new Symbol is created for
                        // them; the token still resolves (as a Write) once
                        // this span is recorded here.
                        let tok = find_ident(tokens, name, *pos, end).unwrap_or(*pos);
                        write_targets.push(spans[tok]);
                        continue;
                    }
                }
                bind_pattern(
                    ctx,
                    pat,
                    SymbolKind::Variable,
                    *pos,
                    end,
                    scope,
                    parent,
                    symbols,
                    scopes,
                    write_targets,
                );
            }
            Stmt::MatchFor(pat, iter, body, pos) => {
                walk_expr(ctx, iter, scope, symbols, scopes, write_targets);
                let for_end =
                    find_token(tokens, &Token::Colon, *pos, stmt_end(tokens, *pos)).unwrap_or(*pos);
                bind_pattern(
                    ctx,
                    pat,
                    SymbolKind::Variable,
                    *pos,
                    for_end,
                    scope,
                    parent,
                    symbols,
                    scopes,
                    write_targets,
                );
                collect(body, ctx, scope, parent, symbols, scopes, write_targets);
            }
            Stmt::Match(matched, arms, pos) => {
                walk_expr(ctx, matched, scope, symbols, scopes, write_targets);
                let mut case_search_from = *pos;
                let stmt_end_idx = stmt_end(tokens, *pos);
                for (pat, guard, body) in arms {
                    let case_idx = find_token(tokens, &Token::Case, case_search_from, stmt_end_idx)
                        .unwrap_or(case_search_from);
                    let colon_idx = find_token(tokens, &Token::Colon, case_idx, stmt_end_idx)
                        .unwrap_or(stmt_end_idx);
                    bind_pattern(
                        ctx,
                        pat,
                        SymbolKind::Variable,
                        case_idx + 1,
                        colon_idx,
                        scope,
                        parent,
                        symbols,
                        scopes,
                        write_targets,
                    );
                    if let Some(g) = guard {
                        walk_expr(ctx, g, scope, symbols, scopes, write_targets);
                    }
                    collect(body, ctx, scope, parent, symbols, scopes, write_targets);
                    case_search_from = colon_idx;
                }
            }
            Stmt::If(cond, body, else_body) => {
                walk_expr(ctx, cond, scope, symbols, scopes, write_targets);
                collect(body, ctx, scope, parent, symbols, scopes, write_targets);
                collect(
                    else_body,
                    ctx,
                    scope,
                    parent,
                    symbols,
                    scopes,
                    write_targets,
                );
            }
            Stmt::While(cond, body) => {
                walk_expr(ctx, cond, scope, symbols, scopes, write_targets);
                collect(body, ctx, scope, parent, symbols, scopes, write_targets);
            }
            Stmt::Return(e) | Stmt::Yield(e) | Stmt::Expr(e) => {
                walk_expr(ctx, e, scope, symbols, scopes, write_targets);
            }
            Stmt::IndexAssign(base, idx, val, _) => {
                walk_expr(ctx, base, scope, symbols, scopes, write_targets);
                walk_expr(ctx, idx, scope, symbols, scopes, write_targets);
                walk_expr(ctx, val, scope, symbols, scopes, write_targets);
            }
            Stmt::AttributeAssign(base, _, val, _) => {
                walk_expr(ctx, base, scope, symbols, scopes, write_targets);
                walk_expr(ctx, val, scope, symbols, scopes, write_targets);
            }
            Stmt::FromImport(path, items, pos) => {
                let end = stmt_end(tokens, *pos);
                // Skip the module path, whose segments may repeat an
                // imported name (`from x import x`).
                let mut cursor = (*pos..end)
                    .find(|&i| matches!(tokens[i], Token::Import))
                    .map_or(*pos, |i| i + 1);
                for (name, alias) in items {
                    // `from m import *` binds names only another file knows.
                    if name == "*" {
                        continue;
                    }
                    // The binding is the alias when there is one (its
                    // token comes after the original name's).
                    let (id, found) = match alias {
                        Some(a) => {
                            let orig = find_ident(tokens, name, cursor, end).unwrap_or(cursor);
                            ctx.push_child(
                                symbols,
                                a.clone(),
                                SymbolKind::Import,
                                orig + 1,
                                end,
                                scope,
                                parent.unwrap_or(SymbolId(u32::MAX)),
                            )
                        }
                        None => ctx.push_child(
                            symbols,
                            name.clone(),
                            SymbolKind::Import,
                            cursor,
                            end,
                            scope,
                            parent.unwrap_or(SymbolId(u32::MAX)),
                        ),
                    };
                    // Imports are top-level bindings, not children of
                    // anything, but reuse push_child for the "single
                    // token, no scope-span" shape; drop the placeholder
                    // parent it forces when there truly is none.
                    if parent.is_none() {
                        symbols[id.0 as usize].parent = None;
                    }
                    symbols[id.0 as usize].import = Some(super::symbols::ImportSource {
                        module: path.clone(),
                        name: Some(name.clone()),
                    });
                    let local = alias.as_ref().unwrap_or(name);
                    register(scopes, scope, local, id);
                    cursor = found + 1;
                }
            }
            Stmt::Import(modules, pos) => {
                let end = stmt_end(tokens, *pos);
                // One comma-separated segment of tokens per imported module.
                let mut segments = Vec::new();
                let mut seg_start = *pos + 1;
                for (i, t) in tokens.iter().enumerate().take(end).skip(*pos + 1) {
                    if matches!(t, Token::Comma) {
                        segments.push((seg_start, i));
                        seg_start = i + 1;
                    }
                }
                segments.push((seg_start, end));
                for ((path, alias), (a, b)) in modules.iter().zip(segments) {
                    // As in Python: `import a.b` binds `a` (the package,
                    // used as `a.b.name`); `import a.b as c` binds `c` (the
                    // module `a.b` itself).
                    let (local, module, from) = match alias {
                        Some(al) => {
                            let as_tok = (a..b).find(|&i| matches!(tokens[i], Token::As)).unwrap_or(a);
                            (al.clone(), path.clone(), as_tok)
                        }
                        None => (path[0].clone(), vec![path[0].clone()], a),
                    };
                    // `import a.b` then `import a.c`: one binding for `a`.
                    if alias.is_none()
                        && let Some(&existing) = scopes[scope.0 as usize].bindings.get(&local)
                        && symbols[existing.0 as usize].import.as_ref().is_some_and(|s| {
                            s.name.is_none() && s.module == module
                        })
                    {
                        continue;
                    }
                    let (id, _) = ctx.push_child(
                        symbols,
                        local.clone(),
                        SymbolKind::Import,
                        from,
                        b,
                        scope,
                        parent.unwrap_or(SymbolId(u32::MAX)),
                    );
                    if parent.is_none() {
                        symbols[id.0 as usize].parent = None;
                    }
                    symbols[id.0 as usize].import =
                        Some(super::symbols::ImportSource { module, name: None });
                    register(scopes, scope, &local, id);
                }
            }
            Stmt::Passthrough(code, pos) => {
                // A top-level `let NAME = ...;` line is captured whole as
                // inline Rust (see the lexer's column-0 passthrough
                // detection), so its binding would otherwise be invisible
                // to the rest of the file; every other passthrough form is
                // left alone; it also could not be interpreted as a
                // craw_binding.
                if let Some(rest) = code.strip_prefix("let ") {
                    if let Some(raw_name) = rest.split_whitespace().next() {
                        let var_name = raw_name.trim_end_matches([':', ';']);
                        if !var_name.is_empty() {
                            ctx.push_binding(
                                symbols,
                                scopes,
                                write_targets,
                                var_name.to_string(),
                                SymbolKind::Variable,
                                *pos,
                                stmt_end(tokens, *pos),
                                scope,
                                parent,
                            );
                        }
                    }
                } else if let Ok(file) = syn::parse_file(code) {
                    // A `rust:` block (or bare top-level `fn`) defining
                    // plain Rust functions: not resolvable to a precise
                    // token position (there is none — this text lives
                    // inside one opaque passthrough token), but still
                    // worth surfacing for completion.
                    for item in file.items {
                        if let syn::Item::Fn(f) = item {
                            let name = f.sig.ident.to_string();
                            if name != "main" {
                                ctx.push_binding(
                                    symbols,
                                    scopes,
                                    write_targets,
                                    name,
                                    SymbolKind::Function,
                                    *pos,
                                    stmt_end(tokens, *pos),
                                    scope,
                                    parent,
                                );
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

pub(super) fn new_module_scope(span: Span) -> Scope {
    Scope {
        id: ScopeId(0),
        parent: None,
        span,
        kind: ScopeKind::Module,
        bindings: Default::default(),
        owner: None,
    }
}
