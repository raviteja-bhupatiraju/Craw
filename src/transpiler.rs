use crate::ast::{CallStyle, Expr, Pattern, QueryClause, Stmt, Type};
use quote::ToTokens;
use std::collections::{HashMap, HashSet};
use syn::{FnArg, Item, ReturnType};

const BUILTINS: &[(&str, &str)] = &[
    ("ident", "craw_ident"),
    ("const", "craw_const"),
    ("range", "craw_range"),
    ("fmap", "craw_fmap"),
    ("filter", "craw_filter"),
    ("orderby", "craw_orderby"),
    ("reduce", "craw_reduce"),
    ("flatten", "craw_flatten"),
    ("groupsof", "craw_groupsof"),
    ("windowsof", "craw_windowsof"),
    ("enumerate", "craw_enumerate"),
    ("list", "craw_list"),
    ("count", "craw_count"),
    ("len", "craw_count"),
    ("str", "craw_str"),
    ("any", "craw_any"),
    ("all", "craw_all"),
    ("sum", "craw_sum"),
    ("product", "craw_product"),
    ("thread_map", "craw_thread_map"),
    ("cartesian_product", "craw_cartesian_product"),
    ("tee", "craw_tee"),
    ("safe_call", "craw_safe_call"),
    ("and_then", "craw_and_then"),
    ("next", "craw_next"),
    ("not", "craw_not"),
    ("print", "craw_print"),
    ("println", "craw_print"),
    ("add", "craw_add"),
    ("sub", "craw_sub"),
    ("mul", "craw_mul"),
    ("div", "craw_div"),
    ("÷", "craw_div_int"),
    ("mod", "craw_mod"),
    ("eq", "craw_eq"),
    ("ne", "craw_ne"),
    ("lt", "craw_lt"),
    ("le", "craw_le"),
    ("gt", "craw_gt"),
    ("ge", "craw_ge"),
    ("hcat", "craw_hcat"),
    ("vcat", "craw_vcat"),
    ("hvcat", "craw_hvcat"),
    ("sqrt", "craw_sqrt"),
    ("pow", "craw_pow"),
    ("approx", "craw_approx"),
    ("to", "craw_to2"),
    ("until", "craw_until2"),
];

struct CodeWriter {
    buffer: String,
    indent_level: usize,
    indent_cache: Vec<String>,
}

impl CodeWriter {
    fn new(capacity: usize) -> Self {
        let mut indent_cache = Vec::with_capacity(10);
        for i in 0..10 {
            indent_cache.push("    ".repeat(i));
        }
        Self {
            buffer: String::with_capacity(capacity),
            indent_level: 0,
            indent_cache,
        }
    }

    fn push(&mut self, s: &str) {
        self.buffer.push_str(s);
    }

    fn push_line(&mut self, s: &str) {
        let indent = &self.indent_cache[self.indent_level];
        self.buffer.push_str(indent);
        self.buffer.push_str(s);
        self.buffer.push('\n');
    }

    fn push_indent(&mut self) {
        let indent = &self.indent_cache[self.indent_level];
        self.buffer.push_str(indent);
    }

    fn indent(&mut self) {
        self.indent_level += 1;
        if self.indent_level >= self.indent_cache.len() {
            self.indent_cache.push("    ".repeat(self.indent_level));
        }
    }

    fn dedent(&mut self) {
        if self.indent_level > 0 {
            self.indent_level -= 1;
        }
    }

    #[allow(dead_code)]
    fn get_indent(&self) -> &str {
        &self.indent_cache[self.indent_level]
    }

    #[allow(dead_code)]
    fn get_indent_at(&mut self, level: usize) -> &str {
        while level >= self.indent_cache.len() {
            self.indent_cache
                .push("    ".repeat(self.indent_cache.len()));
        }
        &self.indent_cache[level]
    }

    fn finish(self) -> String {
        self.buffer
    }
}

#[derive(Debug, Default, Clone)]
pub struct ScopeInfo {
    pub defined: HashSet<String>,
    pub captured: HashSet<String>,
    pub modified: HashSet<String>,
    pub boxed: HashSet<String>,
    pub global_hints: HashSet<String>,
    pub nonlocal_hints: HashSet<String>,
    pub types: HashMap<String, Type>,
}

pub struct TemplateExpander {
    templates: HashMap<String, (Vec<String>, Vec<Stmt>)>,
    next_id: usize,
}

impl Default for TemplateExpander {
    fn default() -> Self {
        Self::new()
    }
}

impl TemplateExpander {
    pub fn new() -> Self {
        Self {
            templates: HashMap::new(),
            next_id: 1000000,
        }
    }

    fn next_id(&mut self) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn expand(&mut self, stmts: Vec<Stmt>) -> Vec<Stmt> {
        // `expand_stmt` alone never matches a statement against `self.templates` -
        // only `expand_block` does. Route top-level statements through it too, or
        // top-level macro-block invocations are left unexpanded.
        let mut rest = Vec::new();
        for stmt in stmts {
            if let Stmt::TemplateDef(name, args, body, _) = stmt {
                self.templates.insert(name, (args, body));
            } else {
                rest.push(stmt);
            }
        }
        self.expand_block(rest)
    }

    fn expand_pattern(&mut self, pat: Pattern) -> Pattern {
        match pat {
            Pattern::Data(name, fields) => Pattern::Data(
                name,
                fields.into_iter().map(|f| self.expand_pattern(f)).collect(),
            ),
            Pattern::Tuple(fields) => {
                Pattern::Tuple(fields.into_iter().map(|f| self.expand_pattern(f)).collect())
            }
            Pattern::View(expr, sub_pat) => Pattern::View(
                Box::new(self.expand_expr(*expr)),
                Box::new(self.expand_pattern(*sub_pat)),
            ),
            _ => pat,
        }
    }

    fn expand_stmt(&mut self, stmt: Stmt) -> Stmt {
        match stmt {
            Stmt::Assign(pat, expr) => {
                Stmt::Assign(self.expand_pattern(pat), self.expand_expr(expr))
            }
            Stmt::Expr(expr) => Stmt::Expr(self.expand_expr(expr)),
            Stmt::DataDef(name, fields, id) => Stmt::DataDef(
                name,
                fields
                    .into_iter()
                    .map(|(n, t, d)| (n, t, d.map(|e| self.expand_expr(e))))
                    .collect(),
                id,
            ),
            Stmt::StructDef(name, fields, id) => Stmt::StructDef(name, fields, id),
            Stmt::MacroBlock {
                name,
                args,
                body,
                branches,
                token_pos,
            } => Stmt::MacroBlock {
                name,
                args: args.into_iter().map(|a| self.expand_expr(a)).collect(),
                body: self.expand_block(body),
                branches: branches
                    .into_iter()
                    .map(|(n, a, b)| {
                        (
                            n,
                            a.into_iter().map(|ae| self.expand_expr(ae)).collect(),
                            self.expand_block(b),
                        )
                    })
                    .collect(),
                token_pos,
            },
            Stmt::FunctionDef {
                name,
                args,
                vararg,
                return_type,
                body,
                is_copyclosure,
                is_addpattern,
                is_generator,
                id,
            } => Stmt::FunctionDef {
                name,
                args: args
                    .into_iter()
                    .map(|(p, d)| (self.expand_pattern(p), d.map(|e| self.expand_expr(e))))
                    .collect(),
                vararg,
                return_type,
                body: self.expand_block(body),
                is_copyclosure,
                is_addpattern,
                is_generator,
                id,
            },
            Stmt::Match(expr, cases) => Stmt::Match(
                self.expand_expr(expr),
                cases
                    .into_iter()
                    .map(|(p, g, b)| {
                        (
                            self.expand_pattern(p),
                            g.map(|ge| self.expand_expr(ge)),
                            self.expand_block(b),
                        )
                    })
                    .collect(),
            ),
            Stmt::MatchFor(pat, expr, body) => Stmt::MatchFor(
                self.expand_pattern(pat),
                self.expand_expr(expr),
                self.expand_block(body),
            ),
            _ => stmt,
        }
    }

    fn expand_block(&mut self, stmts: Vec<Stmt>) -> Vec<Stmt> {
        let mut result = Vec::new();
        for stmt in stmts {
            if let Stmt::Expr(Expr::Call(ref target, ref args)) = stmt
                && let Expr::Ident(ref name) = **target
                && let Some((t_args, t_body)) = self.templates.get(name).cloned()
            {
                let mapping: HashMap<String, Expr> =
                    t_args.into_iter().zip(args.iter().cloned()).collect();
                for t_stmt in t_body {
                    result.push(self.substitute_stmt(t_stmt, &mapping));
                }
                continue;
            }
            if let Stmt::MacroBlock {
                ref name,
                ref args,
                ref body,
                ref branches,
                token_pos,
            } = stmt
                && let Some((t_args, t_body)) = self.templates.get(name).cloned()
            {
                let mut mapping: HashMap<String, Expr> = HashMap::new();
                let mut all_macro_args = Vec::new();
                for a in args {
                    all_macro_args.push(a.clone());
                }
                all_macro_args.push(Expr::Where(
                    Box::new(Expr::None),
                    body.clone(),
                    self.next_id(),
                ));
                for (b_name, b_args, b_body) in branches {
                    all_macro_args.push(Expr::Ident(b_name.clone()));
                    for a in b_args {
                        all_macro_args.push(a.clone());
                    }
                    all_macro_args.push(Expr::Where(
                        Box::new(Expr::None),
                        b_body.clone(),
                        self.next_id(),
                    ));
                }

                if t_args.len() != all_macro_args.len() {
                    panic!(
                        "Macro '{}' arity mismatch at token {}: template expects {} argument(s) but invocation supplied {}",
                        name,
                        token_pos,
                        t_args.len(),
                        all_macro_args.len()
                    );
                }
                for (t_arg, m_arg) in t_args.iter().zip(all_macro_args.into_iter()) {
                    mapping.insert(t_arg.clone(), m_arg);
                }

                for t_stmt in t_body {
                    result.push(self.substitute_stmt(t_stmt, &mapping));
                }
                continue;
            }
            result.push(self.expand_stmt(stmt));
        }
        result
    }

    fn expand_expr(&mut self, expr: Expr) -> Expr {
        match expr {
            Expr::Call(target, args) => {
                if let Expr::Ident(ref name) = *target
                    && let Some((t_args, t_body)) = self.templates.get(name).cloned()
                {
                    let mapping: HashMap<String, Expr> =
                        t_args.into_iter().zip(args.iter().cloned()).collect();

                    let mut body = Vec::new();
                    for t_stmt in t_body {
                        body.push(self.substitute_stmt(t_stmt, &mapping));
                    }

                    let result_expr = if let Some(Stmt::Expr(last_e)) = body.last() {
                        let e = last_e.clone();
                        body.pop();
                        e
                    } else {
                        Expr::None
                    };
                    return Expr::Where(Box::new(result_expr), body, self.next_id());
                }
                Expr::Call(
                    Box::new(self.expand_expr(*target)),
                    args.into_iter().map(|e| self.expand_expr(e)).collect(),
                )
            }
            Expr::BinaryOp(l, op, r) => Expr::BinaryOp(
                Box::new(self.expand_expr(*l)),
                op,
                Box::new(self.expand_expr(*r)),
            ),
            Expr::Lambda(args, body, id) => {
                Expr::Lambda(args, Box::new(self.expand_expr(*body)), id)
            }
            Expr::List(elements) => {
                Expr::List(elements.into_iter().map(|e| self.expand_expr(e)).collect())
            }
            Expr::Dict(pairs) => Expr::Dict(
                pairs
                    .into_iter()
                    .map(|(k, v)| (self.expand_expr(k), self.expand_expr(v)))
                    .collect(),
            ),
            Expr::Index(target, index) => Expr::Index(
                Box::new(self.expand_expr(*target)),
                Box::new(self.expand_expr(*index)),
            ),
            Expr::Attribute(target, name) => {
                Expr::Attribute(Box::new(self.expand_expr(*target)), name)
            }
            Expr::Ternary(c, t, e) => Expr::Ternary(
                Box::new(self.expand_expr(*c)),
                Box::new(self.expand_expr(*t)),
                Box::new(self.expand_expr(*e)),
            ),
            Expr::Tuple(elements) => {
                Expr::Tuple(elements.into_iter().map(|e| self.expand_expr(e)).collect())
            }
            Expr::Range(start, end) => Expr::Range(
                Box::new(self.expand_expr(*start)),
                Box::new(self.expand_expr(*end)),
            ),
            Expr::Gather(coll, field) => Expr::Gather(Box::new(self.expand_expr(*coll)), field),
            Expr::Where(expr, stmts, id) => Expr::Where(
                Box::new(self.expand_expr(*expr)),
                self.expand_block(stmts),
                id,
            ),
            Expr::Comprehension(expr, pat, iterable, is_lazy, id) => Expr::Comprehension(
                Box::new(self.expand_expr(*expr)),
                Box::new(self.expand_pattern(*pat)),
                Box::new(self.expand_expr(*iterable)),
                is_lazy,
                id,
            ),
            Expr::MacroCall(name, args) => Expr::MacroCall(
                name,
                args.into_iter().map(|a| self.expand_expr(a)).collect(),
            ),
            Expr::Splat(inner) => Expr::Splat(Box::new(self.expand_expr(*inner))),
            Expr::Formula(l, r) => Expr::Formula(
                Box::new(self.expand_expr(*l)),
                Box::new(self.expand_expr(*r)),
            ),
            Expr::Query {
                from,
                in_expr,
                clauses,
                select,
                id,
            } => Expr::Query {
                from,
                in_expr: Box::new(self.expand_expr(*in_expr)),
                clauses: clauses
                    .into_iter()
                    .map(|c| match c {
                        QueryClause::Where(e) => QueryClause::Where(self.expand_expr(e)),
                        QueryClause::OrderBy(e, asc) => {
                            QueryClause::OrderBy(self.expand_expr(e), asc)
                        }
                    })
                    .collect(),
                select: Box::new(self.expand_expr(*select)),
                id,
            },
            _ => expr,
        }
    }

    fn substitute_pattern(&self, pat: Pattern, mapping: &HashMap<String, Expr>) -> Pattern {
        match pat {
            Pattern::Var(name, ty) => {
                if let Some(Expr::Ident(replacement)) = mapping.get(&name) {
                    Pattern::Var(replacement.clone(), ty)
                } else {
                    Pattern::Var(name, ty)
                }
            }
            Pattern::Data(name, fields) => Pattern::Data(
                name,
                fields
                    .into_iter()
                    .map(|f| self.substitute_pattern(f, mapping))
                    .collect(),
            ),
            Pattern::Tuple(fields) => Pattern::Tuple(
                fields
                    .into_iter()
                    .map(|f| self.substitute_pattern(f, mapping))
                    .collect(),
            ),
            Pattern::View(expr, sub_pat) => Pattern::View(
                Box::new(self.substitute_expr(*expr, mapping)),
                Box::new(self.substitute_pattern(*sub_pat, mapping)),
            ),
            Pattern::StringSplit(sep, name, rest) => {
                let new_name = if let Some(Expr::Ident(replacement)) = mapping.get(&name) {
                    replacement.clone()
                } else {
                    name
                };
                Pattern::StringSplit(sep, new_name, rest)
            }
            Pattern::Rest(name) => {
                if let Some(Expr::Ident(replacement)) = mapping.get(&name) {
                    Pattern::Rest(replacement.clone())
                } else {
                    Pattern::Rest(name)
                }
            }
            Pattern::Const(_) | Pattern::Wildcard => pat,
        }
    }

    fn substitute_stmt(&self, stmt: Stmt, mapping: &HashMap<String, Expr>) -> Stmt {
        match stmt {
            Stmt::Assign(pat, expr) => Stmt::Assign(
                self.substitute_pattern(pat, mapping),
                self.substitute_expr(expr, mapping),
            ),
            Stmt::Expr(expr) => Stmt::Expr(self.substitute_expr(expr, mapping)),
            Stmt::Return(expr) => Stmt::Return(self.substitute_expr(expr, mapping)),
            Stmt::If(cond, body) => Stmt::If(
                self.substitute_expr(cond, mapping),
                body.into_iter()
                    .map(|s| self.substitute_stmt(s, mapping))
                    .collect(),
            ),
            Stmt::While(cond, body) => Stmt::While(
                self.substitute_expr(cond, mapping),
                body.into_iter()
                    .map(|s| self.substitute_stmt(s, mapping))
                    .collect(),
            ),
            Stmt::Match(expr, cases) => Stmt::Match(
                self.substitute_expr(expr, mapping),
                cases
                    .into_iter()
                    .map(|(p, g, b)| {
                        (
                            self.substitute_pattern(p, mapping),
                            g.map(|e| self.substitute_expr(e, mapping)),
                            b.into_iter()
                                .map(|s| self.substitute_stmt(s, mapping))
                                .collect(),
                        )
                    })
                    .collect(),
            ),
            Stmt::MatchFor(pat, expr, body) => Stmt::MatchFor(
                self.substitute_pattern(pat, mapping),
                self.substitute_expr(expr, mapping),
                body.into_iter()
                    .map(|s| self.substitute_stmt(s, mapping))
                    .collect(),
            ),
            Stmt::FunctionDef {
                name,
                args,
                vararg,
                return_type,
                body,
                is_copyclosure,
                is_addpattern,
                is_generator,
                id,
            } => Stmt::FunctionDef {
                name: name
                    .into_iter()
                    .map(|n| {
                        if let Some(Expr::Ident(replacement)) = mapping.get(&n) {
                            replacement.clone()
                        } else {
                            n
                        }
                    })
                    .collect(),
                args: args
                    .into_iter()
                    .map(|(p, d)| {
                        (
                            self.substitute_pattern(p, mapping),
                            d.map(|e| self.substitute_expr(e, mapping)),
                        )
                    })
                    .collect(),
                vararg: vararg.map(|v| {
                    if let Some(Expr::Ident(replacement)) = mapping.get(&v) {
                        replacement.clone()
                    } else {
                        v
                    }
                }),
                return_type,
                body: body
                    .into_iter()
                    .map(|s| self.substitute_stmt(s, mapping))
                    .collect(),
                is_copyclosure,
                is_addpattern,
                is_generator,
                id,
            },
            Stmt::MacroBlock {
                name,
                args,
                body,
                branches,
                token_pos,
            } => Stmt::MacroBlock {
                name: if let Some(Expr::Ident(replacement)) = mapping.get(&name) {
                    replacement.clone()
                } else {
                    name
                },
                args: args
                    .into_iter()
                    .map(|e| self.substitute_expr(e, mapping))
                    .collect(),
                body: body
                    .into_iter()
                    .map(|s| self.substitute_stmt(s, mapping))
                    .collect(),
                branches: branches
                    .into_iter()
                    .map(|(n, a, b)| {
                        (
                            if let Some(Expr::Ident(replacement)) = mapping.get(&n) {
                                replacement.clone()
                            } else {
                                n
                            },
                            a.into_iter()
                                .map(|ae| self.substitute_expr(ae, mapping))
                                .collect(),
                            b.into_iter()
                                .map(|s| self.substitute_stmt(s, mapping))
                                .collect(),
                        )
                    })
                    .collect(),
                token_pos,
            },
            Stmt::AttributeAssign(target, attr, value) => Stmt::AttributeAssign(
                self.substitute_expr(target, mapping),
                attr,
                self.substitute_expr(value, mapping),
            ),
            Stmt::Yield(expr) => Stmt::Yield(self.substitute_expr(expr, mapping)),
            Stmt::DataDef(name, fields, id) => Stmt::DataDef(
                if let Some(Expr::Ident(replacement)) = mapping.get(&name) {
                    replacement.clone()
                } else {
                    name
                },
                fields
                    .into_iter()
                    .map(|(n, t, d)| {
                        (
                            if let Some(Expr::Ident(replacement)) = mapping.get(&n) {
                                replacement.clone()
                            } else {
                                n
                            },
                            t,
                            d.map(|e| self.substitute_expr(e, mapping)),
                        )
                    })
                    .collect(),
                id,
            ),
            Stmt::StructDef(name, fields, id) => Stmt::StructDef(
                if let Some(Expr::Ident(replacement)) = mapping.get(&name) {
                    replacement.clone()
                } else {
                    name
                },
                fields
                    .into_iter()
                    .map(|(n, t)| {
                        (
                            if let Some(Expr::Ident(replacement)) = mapping.get(&n) {
                                replacement.clone()
                            } else {
                                n
                            },
                            t,
                        )
                    })
                    .collect(),
                id,
            ),
            Stmt::ClassDef {
                name,
                args,
                superclass,
                traits,
                body,
                id,
            } => Stmt::ClassDef {
                name: if let Some(Expr::Ident(replacement)) = mapping.get(&name) {
                    replacement.clone()
                } else {
                    name
                },
                args: args
                    .into_iter()
                    .map(|n| {
                        if let Some(Expr::Ident(replacement)) = mapping.get(&n) {
                            replacement.clone()
                        } else {
                            n
                        }
                    })
                    .collect(),
                superclass: superclass.map(|(n, exprs)| {
                    let new_n = if let Some(Expr::Ident(replacement)) = mapping.get(&n) {
                        replacement.clone()
                    } else {
                        n
                    };
                    (
                        new_n,
                        exprs
                            .into_iter()
                            .map(|e| self.substitute_expr(e, mapping))
                            .collect(),
                    )
                }),
                traits: traits
                    .into_iter()
                    .map(|t| {
                        if let Some(Expr::Ident(replacement)) = mapping.get(&t) {
                            replacement.clone()
                        } else {
                            t
                        }
                    })
                    .collect(),
                body: body
                    .into_iter()
                    .map(|s| self.substitute_stmt(s, mapping))
                    .collect(),
                id,
            },
            _ => stmt,
        }
    }

    fn substitute_expr(&self, expr: Expr, mapping: &HashMap<String, Expr>) -> Expr {
        match expr {
            Expr::Ident(ref name) => {
                if let Some(replacement) = mapping.get(name) {
                    replacement.clone()
                } else {
                    expr
                }
            }
            Expr::Call(target, args) => Expr::Call(
                Box::new(self.substitute_expr(*target, mapping)),
                args.into_iter()
                    .map(|e| self.substitute_expr(e, mapping))
                    .collect(),
            ),
            Expr::BinaryOp(l, op, r) => Expr::BinaryOp(
                Box::new(self.substitute_expr(*l, mapping)),
                op,
                Box::new(self.substitute_expr(*r, mapping)),
            ),
            Expr::Lambda(args, body, id) => {
                Expr::Lambda(args, Box::new(self.substitute_expr(*body, mapping)), id)
            }
            Expr::List(elements) => Expr::List(
                elements
                    .into_iter()
                    .map(|e| self.substitute_expr(e, mapping))
                    .collect(),
            ),
            Expr::Dict(pairs) => Expr::Dict(
                pairs
                    .into_iter()
                    .map(|(k, v)| {
                        (
                            self.substitute_expr(k, mapping),
                            self.substitute_expr(v, mapping),
                        )
                    })
                    .collect(),
            ),
            Expr::Index(target, index) => Expr::Index(
                Box::new(self.substitute_expr(*target, mapping)),
                Box::new(self.substitute_expr(*index, mapping)),
            ),
            Expr::Attribute(target, name) => {
                Expr::Attribute(Box::new(self.substitute_expr(*target, mapping)), name)
            }
            Expr::Ternary(c, t, e) => Expr::Ternary(
                Box::new(self.substitute_expr(*c, mapping)),
                Box::new(self.substitute_expr(*t, mapping)),
                Box::new(self.substitute_expr(*e, mapping)),
            ),
            Expr::Tuple(elements) => Expr::Tuple(
                elements
                    .into_iter()
                    .map(|e| self.substitute_expr(e, mapping))
                    .collect(),
            ),
            Expr::Range(start, end) => Expr::Range(
                Box::new(self.substitute_expr(*start, mapping)),
                Box::new(self.substitute_expr(*end, mapping)),
            ),
            Expr::Gather(coll, field) => {
                Expr::Gather(Box::new(self.substitute_expr(*coll, mapping)), field)
            }
            Expr::Where(expr, stmts, id) => Expr::Where(
                Box::new(self.substitute_expr(*expr, mapping)),
                stmts
                    .into_iter()
                    .map(|s| self.substitute_stmt(s, mapping))
                    .collect(),
                id,
            ),
            Expr::Comprehension(expr, pat, iterable, is_lazy, id) => Expr::Comprehension(
                Box::new(self.substitute_expr(*expr, mapping)),
                Box::new(self.substitute_pattern(*pat, mapping)),
                Box::new(self.substitute_expr(*iterable, mapping)),
                is_lazy,
                id,
            ),
            Expr::MacroCall(name, args) => Expr::MacroCall(
                name,
                args.into_iter()
                    .map(|a| self.substitute_expr(a, mapping))
                    .collect(),
            ),
            Expr::Splat(inner) => Expr::Splat(Box::new(self.substitute_expr(*inner, mapping))),
            Expr::Formula(l, r) => Expr::Formula(
                Box::new(self.substitute_expr(*l, mapping)),
                Box::new(self.substitute_expr(*r, mapping)),
            ),
            Expr::Query {
                from,
                in_expr,
                clauses,
                select,
                id,
            } => Expr::Query {
                from,
                in_expr: Box::new(self.substitute_expr(*in_expr, mapping)),
                clauses: clauses
                    .into_iter()
                    .map(|c| match c {
                        QueryClause::Where(e) => {
                            QueryClause::Where(self.substitute_expr(e, mapping))
                        }
                        QueryClause::OrderBy(e, asc) => {
                            QueryClause::OrderBy(self.substitute_expr(e, mapping), asc)
                        }
                    })
                    .collect(),
                select: Box::new(self.substitute_expr(*select, mapping)),
                id,
            },
            _ => expr,
        }
    }
}

pub struct Analyzer {
    pub scopes: HashMap<usize, ScopeInfo>,
    stack: Vec<ScopeInfo>,
}

impl Default for Analyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer {
    pub fn new() -> Self {
        Self {
            scopes: HashMap::new(),
            stack: vec![ScopeInfo::default()],
        }
    }

    pub fn analyze(stmts: &[Stmt]) -> (ScopeInfo, HashMap<usize, ScopeInfo>) {
        let mut analyzer = Self::new();
        analyzer.visit_stmts(stmts);
        (analyzer.stack.pop().unwrap(), analyzer.scopes)
    }

    fn visit_stmts(&mut self, stmts: &[Stmt]) {
        let mut defs = HashSet::new();
        let mut types = HashMap::new();
        let mut mods = HashSet::new();
        Self::collect_body_definitions(stmts, &mut defs, &mut types);
        Self::collect_body_modifications(stmts, &mut mods);
        let current = self.stack.last_mut().unwrap();
        current.defined.extend(defs);
        current.modified.extend(mods);
        current.types.extend(types);

        for stmt in stmts {
            self.visit_stmt(stmt);
        }
    }

    fn collect_body_modifications(stmts: &[Stmt], mods: &mut HashSet<String>) {
        for stmt in stmts {
            match stmt {
                Stmt::Assign(pat, _) => {
                    let mut vars = HashSet::new();
                    let mut types = HashMap::new();
                    Self::collect_pattern_vars(pat, &mut vars, &mut types);
                    mods.extend(vars);
                }
                Stmt::If(_, body) => Self::collect_body_modifications(body, mods),
                Stmt::While(_, body) => Self::collect_body_modifications(body, mods),
                Stmt::Match(_, cases) => {
                    for (pat, _, body) in cases {
                        let mut vars = HashSet::new();
                        let mut types = HashMap::new();
                        Self::collect_pattern_vars(pat, &mut vars, &mut types);
                        mods.extend(vars);
                        Self::collect_body_modifications(body, mods);
                    }
                }
                Stmt::MatchFor(pat, _, body) => {
                    let mut vars = HashSet::new();
                    let mut types = HashMap::new();
                    Self::collect_pattern_vars(pat, &mut vars, &mut types);
                    mods.extend(vars);
                    Self::collect_body_modifications(body, mods);
                }
                Stmt::IndexAssign(target, _, _) => {
                    // target is modified
                    if let Expr::Ident(name) = target {
                        mods.insert(name.clone());
                    }
                }
                Stmt::AttributeAssign(target, _, _) => {
                    // target is modified
                    if let Expr::Ident(name) = target {
                        mods.insert(name.clone());
                    }
                }
                Stmt::MacroDef { .. } => {}
                Stmt::MacroBlock { .. } => {}
                _ => {}
            }
        }
    }

    fn collect_body_definitions(
        stmts: &[Stmt],
        defs: &mut HashSet<String>,
        types: &mut HashMap<String, Type>,
    ) {
        for stmt in stmts {
            match stmt {
                Stmt::Assign(pat, _) => {
                    Self::collect_pattern_vars(pat, defs, types);
                }
                Stmt::FunctionDef { name, .. } => {
                    if name.len() == 1 {
                        defs.insert(name[0].clone());
                        let is_operator = name[0].chars().all(|c| !c.is_alphanumeric() && c != '_');
                        if is_operator && !name[0].is_empty() {
                            defs.insert(format!("__op_{}", name[0]));
                        }
                    } else if name.len() == 3 {
                        defs.insert(escape_ident(&format!("__op_{}", name[1])));
                    }
                }
                Stmt::DataDef(name, _, _) => {
                    defs.insert(name.clone());
                }
                Stmt::StructDef(name, _, _) => {
                    defs.insert(name.clone());
                    types.insert(name.clone(), Type::Custom(name.clone()));
                }
                Stmt::TraitDef(name, _, _) => {
                    defs.insert(name.clone());
                    types.insert(name.clone(), Type::Custom(name.clone()));
                }
                Stmt::NativeImport(_, items) => {
                    for item in items {
                        defs.insert(item.clone());
                        types.insert(item.clone(), Type::Custom(item.clone()));
                    }
                }
                Stmt::ImplBlock(_, _, body, _) => {
                    Self::collect_body_definitions(body, defs, types);
                }
                Stmt::If(_, body) => Self::collect_body_definitions(body, defs, types),
                Stmt::While(_, body) => Self::collect_body_definitions(body, defs, types),
                Stmt::Match(_, cases) => {
                    for (pat, _, body) in cases {
                        Self::collect_pattern_vars(pat, defs, types);
                        Self::collect_body_definitions(body, defs, types);
                    }
                }
                Stmt::MatchFor(pat, _, body) => {
                    Self::collect_pattern_vars(pat, defs, types);
                    Self::collect_body_definitions(body, defs, types);
                }
                Stmt::Operator(op_spec) => {
                    if op_spec.contains(':') {
                        let parts: Vec<&str> = op_spec.split(':').collect();
                        let op = parts[0];
                        defs.insert(escape_ident(&format!("__op_{}", op)));
                    }
                }
                Stmt::Global(_)
                | Stmt::Nonlocal(_)
                | Stmt::Yield(_)
                | Stmt::Break
                | Stmt::Passthrough(_)
                | Stmt::Expr(_)
                | Stmt::Return(_)
                | Stmt::IndexAssign(_, _, _)
                | Stmt::AttributeAssign(_, _, _)
                | Stmt::Use(_) => {}
                Stmt::TemplateDef(_, _, _, _) => {}
                Stmt::ClassDef { name, .. } => {
                    defs.insert(name.clone());
                    types.insert(name.clone(), Type::Custom(name.clone()));
                }
                Stmt::MacroDef { name, .. } => {
                    defs.insert(name.clone());
                }
                Stmt::MacroBlock { .. } => {}
            }
        }
    }

    fn collect_pattern_vars(
        pat: &Pattern,
        vars: &mut HashSet<String>,
        types: &mut HashMap<String, Type>,
    ) {
        match pat {
            Pattern::Var(name, ty) => {
                vars.insert(name.clone());
                if let Some(t) = ty {
                    types.insert(name.clone(), t.clone());
                }
            }
            Pattern::Data(_, fields) => {
                for f in fields {
                    Self::collect_pattern_vars(f, vars, types);
                }
            }
            Pattern::Tuple(fields) => {
                for f in fields {
                    Self::collect_pattern_vars(f, vars, types);
                }
            }
            Pattern::View(_, sub_pat) => Self::collect_pattern_vars(sub_pat, vars, types),
            Pattern::StringSplit(_, var_name, _) => {
                vars.insert(var_name.clone());
            }
            Pattern::Rest(var_name) => {
                vars.insert(var_name.clone());
            }
            Pattern::Const(_) | Pattern::Wildcard => {}
        }
    }

    fn process_subscope(&mut self, id: usize) {
        let final_info = self.stack.pop().unwrap();
        let current = self.stack.last_mut().unwrap();
        let mut boxed_in_current = HashSet::new();

        for cap in &final_info.captured {
            if !current.defined.contains(cap) {
                current.captured.insert(cap.clone());
            } else {
                if current.modified.contains(cap) || final_info.modified.contains(cap) {
                    boxed_in_current.insert(cap.clone());
                }
            }
        }
        current.boxed.extend(boxed_in_current);

        for mod_var in &final_info.modified {
            if !current.defined.contains(mod_var) {
                current.modified.insert(mod_var.clone());
            }
        }

        self.scopes.insert(id, final_info);
    }

    fn visit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Assign(pat, expr) => {
                self.visit_expr(expr);
                let current = self.stack.last_mut().unwrap();
                let mut vars = HashSet::new();
                let mut types = HashMap::new();
                Self::collect_pattern_vars(pat, &mut vars, &mut types);
                current.defined.extend(vars.clone());
                current.modified.extend(vars);
                current.types.extend(types);
            }
            Stmt::Expr(expr) => self.visit_expr(expr),
            Stmt::FunctionDef {
                name,
                args,
                vararg,
                body,
                id,
                ..
            } => {
                if name.len() == 1 {
                    self.stack
                        .last_mut()
                        .unwrap()
                        .defined
                        .insert(name[0].clone());
                    let is_operator = name[0].chars().all(|c| !c.is_alphanumeric() && c != '_');
                    if is_operator && !name[0].is_empty() {
                        self.stack
                            .last_mut()
                            .unwrap()
                            .defined
                            .insert(format!("__op_{}", name[0]));
                    }
                } else if name.len() == 3 {
                    self.stack
                        .last_mut()
                        .unwrap()
                        .defined
                        .insert(escape_ident(&format!("__op_{}", name[1])));
                }

                for (_, default) in args {
                    if let Some(e) = default {
                        self.visit_expr(e);
                    }
                }

                let info = ScopeInfo::default();
                self.stack.push(info);

                let current = self.stack.last_mut().unwrap();
                for (arg, _) in args {
                    Self::collect_pattern_vars(arg, &mut current.defined, &mut current.types);
                }
                if name.len() == 3 && !name[0].is_empty() && !name[2].is_empty() {
                    current.defined.insert(name[0].clone());
                    current.defined.insert(name[2].clone());
                }
                if let Some(v) = vararg {
                    current.defined.insert(v.clone());
                }

                for s in body {
                    match s {
                        Stmt::Global(names) => {
                            let current = self.stack.last_mut().unwrap();
                            current.global_hints.extend(names.clone());
                            current.captured.extend(names.clone());
                        }
                        Stmt::Nonlocal(names) => {
                            let current = self.stack.last_mut().unwrap();
                            current.nonlocal_hints.extend(names.clone());
                            current.captured.extend(names.clone());
                        }
                        _ => {}
                    }
                }

                self.visit_stmts(body);
                self.process_subscope(*id);
            }
            Stmt::Return(expr) => self.visit_expr(expr),
            Stmt::If(cond, body) => {
                self.visit_expr(cond);
                self.visit_stmts(body);
            }
            Stmt::While(cond, body) => {
                self.visit_expr(cond);
                self.visit_stmts(body);
            }
            Stmt::DataDef(name, fields, id) => {
                self.stack.last_mut().unwrap().defined.insert(name.clone());
                let info = ScopeInfo::default();
                self.stack.push(info);
                for (_, _, default) in fields {
                    if let Some(e) = default {
                        self.visit_expr(e);
                    }
                }
                self.process_subscope(*id);
            }
            Stmt::StructDef(name, _, _) => {
                let current = self.stack.last_mut().unwrap();
                current.defined.insert(name.clone());
                current
                    .types
                    .insert(name.clone(), Type::Custom(name.clone()));
            }
            Stmt::ClassDef { name, body, id, .. } => {
                let current = self.stack.last_mut().unwrap();
                current.defined.insert(name.clone());
                current
                    .types
                    .insert(name.clone(), Type::Custom(name.clone()));
                let info = ScopeInfo::default();
                self.stack.push(info);
                self.visit_stmts(body);
                self.process_subscope(*id);
            }
            Stmt::TraitDef(name, body, id) => {
                let current = self.stack.last_mut().unwrap();
                current.defined.insert(name.clone());
                current
                    .types
                    .insert(name.clone(), Type::Custom(name.clone()));

                let info = ScopeInfo::default();
                self.stack.push(info);
                self.visit_stmts(body);
                self.process_subscope(*id);
            }
            Stmt::ImplBlock(_, _, body, id) => {
                let info = ScopeInfo::default();
                self.stack.push(info);
                self.visit_stmts(body);
                self.process_subscope(*id);
            }
            Stmt::Match(expr, cases) => {
                self.visit_expr(expr);
                for (pat, guard, body) in cases {
                    if let Some(g) = guard {
                        self.visit_expr(g);
                    }
                    let mut pat_vars = HashSet::new();
                    let mut pat_types = HashMap::new();
                    Self::collect_pattern_vars(pat, &mut pat_vars, &mut pat_types);
                    let current = self.stack.last_mut().unwrap();
                    current.defined.extend(pat_vars);
                    current.types.extend(pat_types);
                    self.visit_stmts(body);
                }
            }
            Stmt::MatchFor(pat, expr, body) => {
                self.visit_expr(expr);
                let mut pat_vars = HashSet::new();
                let mut pat_types = HashMap::new();
                Self::collect_pattern_vars(pat, &mut pat_vars, &mut pat_types);
                let current = self.stack.last_mut().unwrap();
                current.defined.extend(pat_vars);
                current.types.extend(pat_types);
                self.visit_stmts(body);
            }
            Stmt::IndexAssign(target, index, value) => {
                self.visit_expr(target);
                self.visit_expr(index);
                self.visit_expr(value);
            }
            Stmt::AttributeAssign(target, _, value) => {
                self.visit_expr(target);
                self.visit_expr(value);
            }
            Stmt::Global(names) => {
                self.stack
                    .last_mut()
                    .unwrap()
                    .global_hints
                    .extend(names.clone());
            }
            Stmt::Nonlocal(names) => {
                for name in names {
                    let current = self.stack.last_mut().unwrap();
                    current.captured.insert(name.clone());
                    current.nonlocal_hints.insert(name.clone());
                }
            }
            Stmt::NativeImport(_, items) => {
                let current = self.stack.last_mut().unwrap();
                for item in items {
                    current.defined.insert(item.clone());
                    current
                        .types
                        .insert(item.clone(), Type::Custom(item.clone()));
                }
            }
            Stmt::Operator(_)
            | Stmt::Break
            | Stmt::Passthrough(_)
            | Stmt::MacroDef { .. }
            | Stmt::MacroBlock { .. }
            | Stmt::TemplateDef(_, _, _, _) => {}
            Stmt::Yield(expr) => self.visit_expr(expr),
            Stmt::Use(_) => {}
        }
    }

    fn visit_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Number(_)
            | Expr::Float(_)
            | Expr::String(_)
            | Expr::Shell(_)
            | Expr::Bool(_)
            | Expr::None => {}
            Expr::FString(exprs) => {
                for expr in exprs {
                    self.visit_expr(expr);
                }
            }
            Expr::Ident(name) => {
                let current = self.stack.last_mut().unwrap();
                if !current.defined.contains(name) {
                    // Check if it's a builtin
                    let is_builtin = BUILTINS.iter().any(|(b, _)| b == name);
                    if !is_builtin {
                        current.captured.insert(name.clone());
                    }
                }
            }
            Expr::BinaryOp(left, op, right) => {
                let current = self.stack.last_mut().unwrap();
                let op_name = format!("__op_{}", op);
                if !current.defined.contains(&op_name) {
                    let mangled_op = escape_ident(&op_name);
                    if !current.defined.contains(&mangled_op) {
                        let standard_ops = [
                            "+", "-", "*", "/", "÷", "%", "==", "!=", "≠", "<", "<=", "≤", ">",
                            ">=", "≥", "and", "or", "is", "in", "∈", "notin", "∉", "**", "≈", "to",
                            "until", "|", "&", "∪", "∩", "⊆", "⊇",
                        ];
                        if !standard_ops.contains(&op.as_str()) {
                            current.captured.insert(mangled_op);
                        }
                    }
                }
                self.visit_expr(left);
                self.visit_expr(right);
            }
            Expr::Call(target, args) => {
                self.visit_expr(target);
                for arg in args {
                    self.visit_expr(arg);
                }
            }
            Expr::BroadcastCall(target, args) => {
                self.visit_expr(target);
                for arg in args {
                    self.visit_expr(arg);
                }
            }
            Expr::Lambda(args, body, id) => {
                let mut info = ScopeInfo::default();
                for arg in args {
                    info.defined.insert(arg.clone());
                }
                self.stack.push(info);
                self.visit_expr(body);
                self.process_subscope(*id);
            }
            Expr::Compose(left, _, right, id) => {
                let info = ScopeInfo::default();
                self.stack.push(info);
                self.visit_expr(left);
                self.visit_expr(right);
                self.process_subscope(*id);
            }
            Expr::Pipe(left, _data, right) => {
                self.visit_expr(left);
                self.visit_expr(right);
            }
            Expr::NoneCoalesce(left, right) => {
                self.visit_expr(left);
                self.visit_expr(right);
            }
            Expr::PartialCall(target, args, id) => {
                let info = ScopeInfo::default();
                self.stack.push(info);
                self.visit_expr(target);
                for e in args.iter().flatten() {
                    self.visit_expr(e);
                }
                self.process_subscope(*id);
            }
            Expr::List(elements) => {
                for e in elements {
                    self.visit_expr(e);
                }
            }
            Expr::Dict(pairs) => {
                for (k, v) in pairs {
                    self.visit_expr(k);
                    self.visit_expr(v);
                }
            }
            Expr::Index(target, index) => {
                self.visit_expr(target);
                self.visit_expr(index);
            }
            Expr::Attribute(target, _) => self.visit_expr(target),
            Expr::AttributePartial(_) => {}
            Expr::ImplicitLambda(body, id) => {
                let mut info = ScopeInfo::default();
                info.defined.insert("_".to_string());
                self.stack.push(info);
                self.visit_expr(body);
                self.process_subscope(*id);
            }
            Expr::Where(expr, stmts, id) => {
                let info = ScopeInfo::default();
                self.stack.push(info);
                self.visit_stmts(stmts);
                self.visit_expr(expr);
                self.process_subscope(*id);
            }
            Expr::Set(elements) => {
                for e in elements {
                    self.visit_expr(e);
                }
            }
            Expr::Frozenset(elements) => {
                for e in elements {
                    self.visit_expr(e);
                }
            }
            Expr::Multiset(elements) => {
                for e in elements {
                    self.visit_expr(e);
                }
            }
            Expr::LazyList(elements, id) => {
                let info = ScopeInfo::default();
                self.stack.push(info);
                for e in elements {
                    self.visit_expr(e);
                }
                self.process_subscope(*id);
            }
            Expr::MacroCall(_, args) => {
                for arg in args {
                    self.visit_expr(arg);
                }
            }
            Expr::Splat(inner) => {
                self.visit_expr(inner);
            }
            Expr::IndexPartial(target) => self.visit_expr(target),
            Expr::OperatorFunction(_) => {}
            Expr::Passthrough(_) => {}
            Expr::Ternary(cond, then_expr, else_expr) => {
                self.visit_expr(cond);
                self.visit_expr(then_expr);
                self.visit_expr(else_expr);
            }
            Expr::Tuple(elements) => {
                for e in elements {
                    self.visit_expr(e);
                }
            }
            Expr::Range(start, end) => {
                self.visit_expr(start);
                self.visit_expr(end);
            }
            Expr::Gather(coll, _) => {
                self.visit_expr(coll);
            }
            Expr::Slice(start, stop, step) => {
                if let Some(e) = start {
                    self.visit_expr(e);
                }
                if let Some(e) = stop {
                    self.visit_expr(e);
                }
                if let Some(e) = step {
                    self.visit_expr(e);
                }
            }
            Expr::Hcat(elements) => {
                for e in elements {
                    self.visit_expr(e);
                }
            }
            Expr::Vcat(elements) => {
                for e in elements {
                    self.visit_expr(e);
                }
            }
            Expr::Placeholder => {}
            Expr::Comprehension(expr, pat, iterable, _, id) => {
                self.visit_expr(iterable);
                let mut info = ScopeInfo::default();
                let mut vars = HashSet::new();
                let mut tys = HashMap::new();
                Analyzer::collect_pattern_vars(pat, &mut vars, &mut tys);
                info.defined.extend(vars);
                info.types.extend(tys);
                self.stack.push(info);
                self.visit_expr(expr);
                self.process_subscope(*id);
            }
            Expr::Formula(_left, _right) => {
                // Symbols in formulas are not treated as variables to be captured
            }
            Expr::Query {
                from,
                in_expr,
                clauses,
                select,
                id,
            } => {
                self.visit_expr(in_expr);
                let current = self.stack.last_mut().unwrap();
                for func in &["fmap", "filter", "orderby"] {
                    if !current.defined.contains(*func) {
                        let is_builtin = BUILTINS.iter().any(|(b, _)| *b == *func);
                        if !is_builtin {
                            current.captured.insert(func.to_string());
                        }
                    }
                }

                let mut info = ScopeInfo::default();
                info.defined.insert(from.clone());
                self.stack.push(info);
                for clause in clauses {
                    match clause {
                        QueryClause::Where(expr) => self.visit_expr(expr),
                        QueryClause::OrderBy(expr, _) => self.visit_expr(expr),
                    }
                }
                self.visit_expr(select);
                self.process_subscope(*id);
            }
        }
    }
}

fn type_to_rust(ty: &Type) -> String {
    match ty {
        Type::Int => "i64".to_string(),
        Type::String => "String".to_string(),
        Type::Custom(s) => s.clone(),
        Type::Generic(s, args) => {
            let arg_strs: Vec<_> = args.iter().map(type_to_rust).collect();
            format!("{}<{}>", s, arg_strs.join(", "))
        }
        Type::Dynamic => "CrawValue".to_string(),
    }
}

fn escape_ident(name: &str) -> String {
    if name.starts_with("$") {
        return name.to_string();
    }

    let mut result = match name {
        "const" | "type" | "mod" | "use" | "extern" | "crate" | "pub" | "fn" | "let" | "mut"
        | "ref" | "match" | "if" | "else" | "loop" | "while" | "for" | "in" | "break"
        | "continue" | "return" | "struct" | "enum" | "trait" | "impl" | "where" | "as"
        | "move" | "static" | "dyn" | "async" | "await" | "try" | "yield" | "str" | "std"
        | "env" | "thread" => format!("r#{}", name),
        "print" => "__craw_id_print".to_string(),
        "println" => "__craw_id_println".to_string(),
        _ => name.to_string(),
    };

    if result
        .chars()
        .any(|c| !c.is_ascii_alphanumeric() && c != '_')
        && !result.starts_with("r#")
    {
        let mut mangled = String::from("__craw_mangled_");
        for c in result.chars() {
            if c.is_ascii_alphanumeric() || c == '_' {
                mangled.push(c);
            } else {
                mangled.push_str(&format!("_u{:x}_", c as u32));
            }
        }
        result = mangled;
    }
    result
}

pub fn transpile(stmts: &[Stmt]) -> String {
    let mut expander = TemplateExpander::new();
    let stmts = expander.expand(stmts.to_vec());
    let (top_scope, scopes) = Analyzer::analyze(&stmts);
    let mut writer = CodeWriter::new(stmts.len() * 100);
    writer.push_line("pub fn craw_main() {");
    let mut native_funcs = HashSet::new();
    for stmt in &stmts {
        if let Stmt::FunctionDef {
            name,
            args,
            return_type,
            vararg,
            is_generator,
            ..
        } = stmt
        {
            if name.len() == 1 && vararg.is_none() && !is_generator {
                let all_typed = args.iter().all(|(pat, _)| {
                    if let Pattern::Var(_, ty) = pat {
                        ty.is_some()
                    } else {
                        false
                    }
                });
                if return_type.is_some() && all_typed {
                    native_funcs.insert(name[0].clone());
                }
            }
        }
    }

    writer.indent();

    let mut defined = HashSet::new();

    for stmt in stmts {
        transpile_stmt(
            &stmt,
            &mut writer,
            &top_scope,
            &mut defined,
            &scopes,
            false,
            false,
            &native_funcs,
        );
    }
    writer.dedent();
    writer.push_line("}");
    writer.finish()
}

fn transpile_block(
    stmts: &[Stmt],
    writer: &mut CodeWriter,
    current_scope: &ScopeInfo,
    defined: &mut HashSet<String>,
    scopes: &HashMap<usize, ScopeInfo>,
    in_native_context: bool,
    is_trait: bool,
    native_funcs: &HashSet<String>,
) {
    for stmt in stmts {
        transpile_stmt(
            stmt,
            writer,
            current_scope,
            defined,
            scopes,
            in_native_context,
            is_trait,
            native_funcs,
        );
    }
}

fn transpile_expr_as_value(
    expr: &Expr,
    writer: &mut CodeWriter,
    current_scope: &ScopeInfo,
    scopes: &HashMap<usize, ScopeInfo>,
    native_funcs: &HashSet<String>,
) {
    match expr {
        Expr::Float(f) => {
            writer.push("CrawValue::Float(");
            let s = f.to_string();
            writer.push(&s);
            if !s.contains('.') {
                writer.push(".0");
            }
            writer.push(")");
        }
        Expr::Number(n) => {
            writer.push(&format!("CrawValue::Int({})", n));
        }
        Expr::String(s) => {
            writer.push(&format!("CrawValue::String(Rc::new({:?}.to_string()))", s));
        }
        Expr::Shell(cmd) => {
            writer.push(&format!("CrawValue::String(Rc::new(String::from_utf8_lossy(&std::process::Command::new(\"sh\").arg(\"-c\").arg({:?}).output().expect(\"Command failed\").stdout).into_owned()))", cmd));
        }
        Expr::Bool(b) => {
            writer.push(&format!("CrawValue::Bool({})", b));
        }
        Expr::None => {
            writer.push("CrawValue::None");
        }
        Expr::Ident(name) if name == "_" => {
            writer.push("__craw_underscore.clone()");
        }
        Expr::Ident(name) => {
            let esc_name = escape_ident(name);
            if let Some(ty) = current_scope.types.get(name)
                && !matches!(ty, Type::Dynamic)
            {
                writer.push("CrawValue::from(");
                writer.push(&esc_name);
                writer.push(")");
            } else if BUILTINS.iter().any(|(b, _)| b == name)
                && !current_scope.defined.contains(name)
            {
                writer.push(&format!("CrawValue::Builtin({:?}.to_string())", name));
            } else if name.starts_with("$") {
                writer.push(&esc_name);
            } else {
                writer.push(&esc_name);
                writer.push(".clone()");
            }
        }
        _ => {
            writer.push("craw_driver(");
            transpile_expr(expr, writer, current_scope, scopes, native_funcs);
            writer.push(")");
        }
    }
}

fn transpile_stmt(
    stmt: &Stmt,
    writer: &mut CodeWriter,
    current_scope: &ScopeInfo,
    defined: &mut HashSet<String>,
    scopes: &HashMap<usize, ScopeInfo>,
    in_native_context: bool,
    is_trait: bool,
    native_funcs: &HashSet<String>,
) {
    let globals = &current_scope.global_hints;
    let nonlocals = &current_scope.nonlocal_hints;
    match stmt {
        Stmt::Assign(pat, expr) => {
            if let Pattern::Var(name, ty) = pat {
                let esc_name = escape_ident(name);
                writer.push_indent();
                if let Some(t) = ty
                    && !matches!(t, Type::Dynamic)
                {
                    writer.push("let mut ");
                    writer.push(&esc_name);
                    writer.push(": ");
                    writer.push(&type_to_rust(t));
                    writer.push(" = (");
                    transpile_expr_as_value(expr, writer, current_scope, scopes, native_funcs);
                    writer.push(").clone().try_into_native::<");
                    writer.push(&type_to_rust(t));
                    writer.push(">().unwrap();\n");
                    defined.insert(name.clone());
                    return;
                }

                let is_boxed = current_scope.boxed.contains(name);

                if globals.contains(name) || nonlocals.contains(name) {
                    writer.push(&format!(
                        "if let CrawValue::Recursive(ref r) = {} {{ *r.borrow_mut() = ",
                        esc_name
                    ));
                    transpile_expr_as_value(expr, writer, current_scope, scopes, native_funcs);
                    writer.push("; } else { ");
                    writer.push("panic!(\"Assignment to non-boxed nonlocal/global variable\");");
                    writer.push(" }\n");
                } else if defined.contains(name) {
                    if is_boxed {
                        writer.push(&format!(
                            "if let CrawValue::Recursive(ref r) = {} {{ *r.borrow_mut() = ",
                            esc_name
                        ));
                        transpile_expr_as_value(expr, writer, current_scope, scopes, native_funcs);
                        writer.push("; }\n");
                    } else {
                        writer.push(&esc_name);
                        writer.push(" = ");
                        transpile_expr_as_value(expr, writer, current_scope, scopes, native_funcs);
                        writer.push(";\n");
                    }
                } else {
                    if is_boxed {
                        writer.push(&format!(
                            "let mut {} = CrawValue::Recursive(Rc::new(RefCell::new(",
                            esc_name
                        ));
                        transpile_expr_as_value(expr, writer, current_scope, scopes, native_funcs);
                        writer.push(")));\n");
                    } else {
                        writer.push("let mut ");
                        writer.push(&esc_name);
                        writer.push(" = ");
                        transpile_expr_as_value(expr, writer, current_scope, scopes, native_funcs);
                        writer.push(";\n");
                    }
                    defined.insert(name.clone());
                }
            } else {
                writer.push_indent();
                writer.push("let __assign_tmp = ");
                transpile_expr_as_value(expr, writer, current_scope, scopes, native_funcs);
                writer.push(";\n");

                let (cond, binds) = build_pattern(
                    pat,
                    "__assign_tmp",
                    "assign",
                    writer.indent_level,
                    current_scope,
                    scopes,
                    native_funcs,
                );
                writer.push_indent();
                writer.push("if !");
                writer.push(&cond);
                writer.push(" { panic!(\"TypeError: assignment failed to match pattern\"); }\n");
                writer.push(&binds);

                let mut vars = HashSet::new();
                let mut types = HashMap::new();
                Analyzer::collect_pattern_vars(pat, &mut vars, &mut types);
                defined.extend(vars);
            }
        }
        Stmt::Expr(expr) => {
            writer.push_indent();
            transpile_expr_as_value(expr, writer, current_scope, scopes, native_funcs);
            writer.push(";\n");
        }
        Stmt::FunctionDef {
            name,
            args,
            vararg,
            body,
            return_type,
            is_copyclosure,
            is_addpattern,
            is_generator,
            id,
        } => {
            let is_native = name.len() == 1
                && vararg.is_none()
                && !is_generator
                && return_type.is_some()
                && args.iter().all(|(pat, _)| {
                    if let Pattern::Var(_, ty) = pat {
                        ty.is_some()
                    } else {
                        false
                    }
                });

            if is_native {
                let esc_name = escape_ident(&name[0]);
                let ret_ty = return_type.as_ref().unwrap();

                writer.push_indent();
                writer.push(&format!("fn __native_{}(", esc_name));
                for (i, (pat, _)) in args.iter().enumerate() {
                    if i > 0 {
                        writer.push(", ");
                    }
                    if let Pattern::Var(n, Some(ty)) = pat {
                        writer.push(&format!("{}: {}", escape_ident(n), type_to_rust(ty)));
                    }
                }
                writer.push(&format!(") -> {} {{\n", type_to_rust(ret_ty)));

                writer.indent();
                let info = scopes.get(id).expect("Scope info not found");
                let mut native_defined = HashSet::new();
                for (pat, _) in args {
                    if let Pattern::Var(n, _) = pat {
                        native_defined.insert(n.clone());
                    }
                }
                transpile_block_native(
                    body,
                    writer,
                    info,
                    &mut native_defined,
                    scopes,
                    native_funcs,
                );
                // In case it falls through without returning:
                writer.push_indent();
                writer.push("unreachable!()\n");
                writer.dedent();
                writer.push_line("}");

                // Now create the dynamic wrapper
                writer.push_indent();
                if globals.contains(&name[0]) || nonlocals.contains(&name[0]) {
                    writer.push(&format!("if let CrawValue::Recursive(ref r) = {} {{ *r.borrow_mut() = CrawValue::Closure(Rc::new(move |args| {{\n", esc_name));
                } else if defined.contains(&name[0]) {
                    writer.push(&format!(
                        "{} = CrawValue::Closure(Rc::new(move |args| {{\n",
                        esc_name
                    ));
                } else {
                    writer.push(&format!(
                        "let mut {} = CrawValue::Closure(Rc::new(move |args| {{\n",
                        esc_name
                    ));
                    defined.insert(name[0].clone());
                }

                writer.indent();
                writer.push_indent();
                writer.push("let mut __final_args: Vec<CrawValue> = vec![];\n");
                for (i, _) in args.iter().enumerate() {
                    writer.push_indent();
                    writer.push(&format!("if args.len() > {} {{ __final_args.push(args[{}].clone()); }} else {{ panic!(\"TypeError: missing required argument at position {}\"); }}\n", i, i, i));
                }

                let mut call_args = Vec::new();
                for (i, (pat, _)) in args.iter().enumerate() {
                    if let Pattern::Var(_n, Some(ty)) = pat {
                        writer.push_indent();
                        writer.push(&format!("let arg_{} = (__final_args[{}]).clone().try_into_native::<{}>().unwrap();\n", i, i, type_to_rust(ty)));
                        call_args.push(format!("arg_{}", i));
                    }
                }

                writer.push_indent();
                writer.push(&format!(
                    "let __res = __native_{}({});\n",
                    esc_name,
                    call_args.join(", ")
                ));

                writer.push_indent();
                if type_to_rust(ret_ty) == "i64" {
                    writer.push("CallResult::Return(CrawValue::Int(__res))\n");
                } else if type_to_rust(ret_ty) == "bool" {
                    writer.push("CallResult::Return(CrawValue::Bool(__res))\n");
                } else if type_to_rust(ret_ty) == "String" {
                    writer.push("CallResult::Return(CrawValue::String(__res))\n");
                } else {
                    writer.push("CallResult::Return(CrawValue::from(__res))\n");
                }
                writer.dedent();

                if globals.contains(&name[0]) || nonlocals.contains(&name[0]) {
                    writer.push_line("})); }");
                } else {
                    writer.push_line("}));");
                }
                return;
            }

            if in_native_context {
                writer.push_indent();
                writer.push("fn ");
                writer.push(&name.join("::"));
                writer.push("(");
                for (i, (arg, _)) in args.iter().enumerate() {
                    if i > 0 {
                        writer.push(", ");
                    }
                    if let Pattern::Var(n, ty) = arg {
                        if n == "self" {
                            writer.push("&self");
                        } else {
                            writer.push(n);
                            writer.push(": ");
                            if let Some(t) = ty {
                                writer.push(&type_to_rust(t));
                            } else {
                                writer.push("CrawValue");
                            }
                        }
                    } else {
                        writer.push(&format!("arg_{}: CrawValue", i));
                    }
                }
                writer.push(") -> ");
                if let Some(t) = return_type {
                    writer.push(&type_to_rust(t));
                } else {
                    writer.push("CrawValue");
                }
                if is_trait {
                    writer.push(";\n");
                } else {
                    writer.push(" {\n");
                    writer.indent();
                    let info = scopes.get(id).expect("Scope info not found");
                    let mut native_defined = HashSet::new();
                    for (i, (arg, _)) in args.iter().enumerate() {
                        let target = if let Pattern::Var(n, _) = arg {
                            if n == "self" {
                                continue;
                            }
                            escape_ident(n)
                        } else {
                            format!("arg_{}", i)
                        };
                        let (cond, binds) = build_pattern(
                            arg,
                            &target,
                            &format!("arg_{}", i),
                            writer.indent_level,
                            info,
                            scopes,
                            native_funcs,
                        );
                        if cond != "true" {
                            writer.push_indent();
                            writer.push("if !");
                            writer.push(&cond);
                            writer.push(
                                " { panic!(\"MatchError: argument did not match pattern\"); }\n",
                            );
                        }
                        writer.push(&binds);
                        let mut vars = HashSet::new();
                        let mut types = HashMap::new();
                        Analyzer::collect_pattern_vars(arg, &mut vars, &mut types);
                        native_defined.extend(vars);
                    }
                    transpile_block(
                        body,
                        writer,
                        info,
                        &mut native_defined,
                        scopes,
                        true,
                        false,
                        native_funcs,
                    );
                    writer.push_indent();
                    writer.push("CrawValue::None\n");
                    writer.dedent();
                    writer.push_line("}");
                }
                return;
            }
            let info = scopes.get(id).expect("Scope info not found");

            let mut captures: Vec<_> = info.captured.iter().collect();
            captures.sort();

            let mut closure_writer = CodeWriter::new(1024);

            if !captures.is_empty() {
                closure_writer.push_line("{");
                closure_writer.indent();
                for cap in &captures {
                    let esc_cap = escape_ident(cap);
                    closure_writer.push_indent();
                    if *is_copyclosure {
                        closure_writer.push("let __cap_");
                        closure_writer.push(cap);
                        closure_writer.push(" = (*");
                        closure_writer.push(&esc_cap);
                        closure_writer.push(").clone(); let ");
                        closure_writer.push(&esc_cap);
                        closure_writer.push(" = __cap_");
                        closure_writer.push(cap);
                        closure_writer.push(".clone());\n");
                    } else {
                        closure_writer.push("let ");
                        closure_writer.push(&esc_cap);
                        closure_writer.push(" = ");
                        closure_writer.push(&esc_cap);
                        closure_writer.push(".clone();\n");
                    }
                }
                closure_writer.push_indent();
            }

            if *is_generator {
                let header = "CrawValue::Closure(Rc::new(move |args| {\n";
                closure_writer.push(header);
                closure_writer.indent();
                closure_writer.push_line("let (v_tx, v_rx) = mpsc::channel();");
                closure_writer.push_line("let (c_tx, c_rx) = mpsc::channel();");
                closure_writer.push_line("let mut __final_args: Vec<CrawValue> = vec![];");
                for (i, (_, default)) in args.iter().enumerate() {
                    closure_writer.push_indent();
                    closure_writer.push("if args.len() > ");
                    closure_writer.push(&i.to_string());
                    closure_writer.push(" { __final_args.push(args[");
                    closure_writer.push(&i.to_string());
                    closure_writer.push("].clone()); }");
                    if let Some(def_val) = default {
                        closure_writer.push(" else { __final_args.push(");
                        transpile_expr_as_value(
                            def_val,
                            &mut closure_writer,
                            current_scope,
                            scopes,
                            native_funcs,
                        );
                        closure_writer.push("); }");
                    } else {
                        closure_writer.push(" else { panic!(\"TypeError: missing required argument at position {}\", ");
                        closure_writer.push(&i.to_string());
                        closure_writer.push("); }");
                    }
                    closure_writer.push("\n");
                }
                if let Some(varg) = vararg {
                    closure_writer.push_indent();
                    closure_writer.push("let mut __vargs = vec![]; ");
                    closure_writer.push("for i in ");
                    closure_writer.push(&args.len().to_string());
                    closure_writer.push("..args.len() { __vargs.push(args[i].clone()); } ");
                    closure_writer.push("let ");
                    closure_writer.push(&escape_ident(varg));
                    closure_writer.push(" = CrawValue::List(Rc::new(RefCell::new(__vargs)));\n");
                }

                closure_writer.push_indent();
                closure_writer.push("let mut __plain_args = vec![]; for a in __final_args { __plain_args.push(a.to_plain()); }\n");
                if let Some(varg) = vararg {
                    closure_writer.push_indent();
                    closure_writer.push(&format!(
                        "let __plain_varg = {}.to_plain();\n",
                        escape_ident(varg)
                    ));
                }

                closure_writer.push_indent();
                closure_writer.push("thread::spawn({");
                closure_writer.push("let __craw_gen_tx = v_tx; let __craw_gen_rx = c_rx; ");

                for cap in &captures {
                    let esc_cap = escape_ident(cap);
                    closure_writer
                        .push(&format!("let {}_plain = {}.to_plain(); ", esc_cap, esc_cap));
                }

                closure_writer.push("move || { ");
                closure_writer.push("let __final_args = __plain_args.into_iter().map(|p| CrawValue::from_plain(p)).collect::<Vec<_>>(); ");

                for cap in &captures {
                    let esc_cap = escape_ident(cap);
                    closure_writer.push(&format!(
                        "let {} = CrawValue::from_plain({}_plain); ",
                        esc_cap, esc_cap
                    ));
                }
                if let Some(varg) = vararg {
                    let esc_varg = escape_ident(varg);
                    closure_writer.push(&format!(
                        "let {} = CrawValue::from_plain(__plain_varg); ",
                        esc_varg
                    ));
                }

                for (i, (arg, _)) in args.iter().enumerate() {
                    let target = format!("__final_args[{}]", i);
                    let path = format!("arg_{}", i);
                    let (cond, binds) = build_pattern(
                        arg,
                        &target,
                        &path,
                        closure_writer.indent_level,
                        info,
                        scopes,
                        native_funcs,
                    );
                    if cond != "true" {
                        closure_writer.push("if !");
                        closure_writer.push(&cond);
                        closure_writer.push(" { panic!(\"TypeError: argument at position {} did not match pattern\", ");
                        closure_writer.push(&i.to_string());
                        closure_writer.push("); } ");
                    }
                    closure_writer.push(&binds);
                }
                let mut defined = HashSet::new();
                transpile_block(
                    body,
                    &mut closure_writer,
                    info,
                    &mut defined,
                    scopes,
                    false,
                    false,
                    native_funcs,
                );
                closure_writer.push_indent();
                closure_writer.push("CallResult::Return(CrawValue::None);\n");
                closure_writer.push(" } });\n");
                closure_writer.push_line(
                    "CallResult::Return(CrawValue::Generator(std::sync::Arc::new(std::sync::Mutex::new(v_rx)), std::sync::Arc::new(std::sync::Mutex::new(c_tx))))",
                );
                closure_writer.dedent();
                closure_writer.push(" } ))");
            } else {
                let header = "CrawValue::Closure(Rc::new(move |args| {\n";
                closure_writer.push(header);
                closure_writer.indent();
                closure_writer.push_line("let mut __final_args: Vec<CrawValue> = vec![];");
                for (i, (_, default)) in args.iter().enumerate() {
                    closure_writer.push_indent();
                    closure_writer.push("if args.len() > ");
                    closure_writer.push(&i.to_string());
                    closure_writer.push(" { __final_args.push(args[");
                    closure_writer.push(&i.to_string());
                    closure_writer.push("].clone()); }");
                    if let Some(def_val) = default {
                        closure_writer.push(" else { __final_args.push(");
                        transpile_expr_as_value(
                            def_val,
                            &mut closure_writer,
                            current_scope,
                            scopes,
                            native_funcs,
                        );
                        closure_writer.push("); }");
                    } else {
                        closure_writer.push(" else { panic!(\"TypeError: missing required argument at position {}\", ");
                        closure_writer.push(&i.to_string());
                        closure_writer.push("); }");
                    }
                    closure_writer.push("\n");
                }
                if let Some(varg) = vararg {
                    closure_writer.push_indent();
                    closure_writer.push("let mut __vargs = vec![]; ");
                    closure_writer.push("for i in ");
                    closure_writer.push(&args.len().to_string());
                    closure_writer.push("..args.len() { __vargs.push(args[i].clone()); } ");
                    closure_writer.push("let ");
                    closure_writer.push(&escape_ident(varg));
                    closure_writer.push(" = CrawValue::List(Rc::new(RefCell::new(__vargs)));\n");
                }
                for (i, (arg, _)) in args.iter().enumerate() {
                    let target = format!("__final_args[{}]", i);
                    let path = format!("arg_{}", i);
                    let (cond, binds) = build_pattern(
                        arg,
                        &target,
                        &path,
                        closure_writer.indent_level,
                        info,
                        scopes,
                        native_funcs,
                    );
                    if cond != "true" {
                        closure_writer.push_indent();
                        closure_writer.push("if !");
                        closure_writer.push(&cond);
                        closure_writer.push(" { panic!(\"TypeError: argument at position {} did not match pattern\", ");
                        closure_writer.push(&i.to_string());
                        closure_writer.push("); }\n");
                    }
                    closure_writer.push(&binds);
                }
                let mut defined = HashSet::new();
                transpile_block(
                    body,
                    &mut closure_writer,
                    info,
                    &mut defined,
                    scopes,
                    false,
                    false,
                    native_funcs,
                );
                closure_writer.push_indent();
                closure_writer.push("CallResult::Return(CrawValue::None)\n");
                closure_writer.dedent();
                closure_writer.push_indent();
                closure_writer.push(" } ))");
            }
            if !captures.is_empty() {
                closure_writer.dedent();
                closure_writer.push_line("}");
            }
            if *is_addpattern {
                let esc_name = escape_ident(&name[0]);
                writer.push_indent();
                writer.push(&format!(
                    "{} = craw_driver(craw_add_pattern({}.clone(), ",
                    esc_name, esc_name
                ));
                writer.push(&closure_writer.finish());
                writer.push("));\n");
            } else {
                writer.push_indent();
                if name.len() == 1 {
                    let esc_name = escape_ident(&name[0]);
                    let is_recursive = captures.contains(&&name[0]);

                    let closure_code = closure_writer.finish();
                    if is_recursive && !defined.contains(&name[0]) {
                        writer.push(&format!(
                            "let mut {} = CrawValue::Recursive(Rc::new(RefCell::new(CrawValue::None)));\n",
                            esc_name
                        ));
                        writer.push_indent();
                        defined.insert(name[0].clone());
                    }

                    if is_recursive || globals.contains(&name[0]) || nonlocals.contains(&name[0]) {
                        writer.push(&format!(
                            "if let CrawValue::Recursive(ref r) = {} {{
",
                            esc_name
                        ));
                        writer.push_indent();
                        writer.push(&format!(
                            "    *r.borrow_mut() = ({}).clone();\n",
                            closure_code
                        ));
                        writer.push_indent();
                        writer.push(&format!(
                            "}} else {{
"
                        ));
                        writer.push_indent();
                        writer.push(&format!("    {} = ({}).clone();\n", esc_name, closure_code));
                        writer.push_indent();
                        let is_operator = name[0].chars().all(|c| !c.is_alphanumeric() && c != '_');
                        if is_operator && !name[0].is_empty() {
                            let op_name = format!("__op_{}", name[0]);
                            writer.push(&format!(
                                "    {} = {}.clone();\n",
                                escape_ident(&op_name),
                                esc_name
                            ));
                            writer.push_indent();
                        }
                        writer.push(&format!("}}\n"));
                    } else if defined.contains(&name[0]) {
                        writer.push(&format!("{} = {};\n", esc_name, closure_code));
                        let is_operator = name[0].chars().all(|c| !c.is_alphanumeric() && c != '_');
                        if is_operator && !name[0].is_empty() {
                            writer.push_indent();
                            let op_name = format!("__op_{}", name[0]);
                            writer.push(&format!(
                                "{} = {}.clone();\n",
                                escape_ident(&op_name),
                                esc_name
                            ));
                        }
                    } else {
                        writer.push(&format!("let mut {} = {};\n", esc_name, closure_code));
                        defined.insert(name[0].clone());
                        let is_operator = name[0].chars().all(|c| !c.is_alphanumeric() && c != '_');
                        if is_operator && !name[0].is_empty() {
                            let op_name = format!("__op_{}", name[0]);
                            writer.push_indent();
                            writer.push(&format!(
                                "let mut {} = {}.clone();\n",
                                escape_ident(&op_name),
                                esc_name
                            ));
                            defined.insert(op_name);
                        }
                    }
                } else if name.len() == 3 {
                    if name[0].is_empty() && name[2].is_empty() {
                        let fn_name = format!("__op_{}", name[1]);
                        let esc_name = escape_ident(&fn_name);
                        let is_recursive = captures.contains(&&fn_name);

                        let closure_code = closure_writer.finish();
                        if is_recursive && !defined.contains(&fn_name) {
                            writer.push(&format!(
                                "let mut {} = CrawValue::Recursive(Rc::new(RefCell::new(CrawValue::None)));\n",
                                esc_name
                            ));
                            writer.push_indent();
                            defined.insert(fn_name.clone());
                        }

                        if is_recursive
                            || globals.contains(&fn_name)
                            || nonlocals.contains(&fn_name)
                        {
                            writer.push(&format!(
                                "if let CrawValue::Recursive(ref r) = {} {{
",
                                esc_name
                            ));
                            writer.push_indent();
                            writer.push(&format!(
                                "    *r.borrow_mut() = ({}).clone();\n",
                                closure_code
                            ));
                            writer.push_indent();
                            writer.push(&format!(
                                "}} else {{
"
                            ));
                            writer.push_indent();
                            writer
                                .push(&format!("    {} = ({}).clone();\n", esc_name, closure_code));
                            writer.push_indent();
                            writer.push(&format!("}}\n"));
                        } else if defined.contains(&fn_name) {
                            writer.push(&format!("{} = {};\n", esc_name, closure_code));
                        } else {
                            writer.push(&format!("let mut {} = {};\n", esc_name, closure_code));
                            defined.insert(fn_name.clone());
                        }
                    } else {
                        // Overloaded operator
                        writer.push("let mut ");
                        writer.push(&escape_ident(&format!("__op_{}", name[1])));
                        writer.push(" = CrawValue::Closure(Rc::new({ ");
                        if !captures.is_empty() {
                            for cap in &captures {
                                let esc_cap = escape_ident(cap);
                                writer.push(&format!("let {} = {}.clone(); ", esc_cap, esc_cap));
                            }
                        }
                        writer.push("move |__op_args| { ");
                        writer.push(&format!(
                            "let mut {} = __op_args[0].clone(); ",
                            escape_ident(&name[0])
                        ));
                        writer.push(&format!(
                            "let mut {} = __op_args[1].clone(); ",
                            escape_ident(&name[2])
                        ));
                        writer.push("CallResult::Return(");
                        writer.push(&closure_writer.finish());
                        writer.push(") } }));\n");
                    }
                }
            }
        }
        Stmt::Return(expr) => {
            writer.push_indent();
            if in_native_context {
                writer.push("return ");
                transpile_expr_as_value(expr, writer, current_scope, scopes, native_funcs);
                writer.push(";\n");
                return;
            }
            if let Expr::Call(target, args) = expr {
                writer.push("return CallResult::TailCall(");
                transpile_expr_as_value(target, writer, current_scope, scopes, native_funcs);
                writer.push(", vec![");
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        writer.push(", ");
                    }
                    transpile_expr_as_value(arg, writer, current_scope, scopes, native_funcs);
                }
                writer.push("]);\n");
            } else {
                writer.push("return ");
                transpile_expr(expr, writer, current_scope, scopes, native_funcs);
                writer.push(";\n");
            }
        }
        Stmt::If(cond, body) => {
            writer.push_indent();
            writer.push("if matches!(&");
            transpile_expr_as_value(cond, writer, current_scope, scopes, native_funcs);
            writer.push(", CrawValue::Bool(true) | CrawValue::Int(1..)) {\n");
            writer.indent();
            transpile_block(
                body,
                writer,
                current_scope,
                defined,
                scopes,
                in_native_context,
                is_trait,
                native_funcs,
            );
            writer.dedent();
            writer.push_line("}");
        }
        Stmt::While(cond, body) => {
            writer.push_indent();
            writer.push("while matches!(&");
            transpile_expr_as_value(cond, writer, current_scope, scopes, native_funcs);
            writer.push(", CrawValue::Bool(true) | CrawValue::Int(1..)) {\n");
            writer.indent();
            transpile_block(
                body,
                writer,
                current_scope,
                defined,
                scopes,
                in_native_context,
                is_trait,
                native_funcs,
            );
            writer.dedent();
            writer.push_line("}");
        }

        Stmt::Break => {
            writer.push_indent();
            writer.push_line("break;");
        }
        Stmt::Yield(expr) => {
            writer.push_indent();
            writer.push("{ let __val = ");
            transpile_expr_as_value(expr, writer, current_scope, scopes, native_funcs);
            writer.push("; if __craw_gen_tx.send(__val.to_plain()).is_err() { return; } if __craw_gen_rx.recv().is_err() { return; } }\n");
        }
        Stmt::Passthrough(s) => {
            writer.push_line(s);
            if let Ok(file) = syn::parse_file(s) {
                for item in file.items {
                    if let Item::Fn(f) = item {
                        let name = f.sig.ident.to_string();
                        if name == "main" {
                            continue;
                        }

                        let esc_name = escape_ident(&name);
                        writer.push_indent();
                        if !globals.contains(&name)
                            && !nonlocals.contains(&name)
                            && !defined.contains(&name)
                        {
                            writer.push("let mut ");
                            writer.push(&esc_name);
                            writer.push(" = CrawValue::None;\n");
                            writer.push_indent();
                            defined.insert(name.clone());
                        }

                        writer.push(&esc_name);
                        writer.push(" = CrawValue::Closure(Rc::new(move |args| {\n");
                        writer.indent();

                        // Extract arguments
                        for (i, arg) in f.sig.inputs.iter().enumerate() {
                            if let FnArg::Typed(pat_type) = arg {
                                let ty = &pat_type.ty;
                                writer.push_indent();
                                writer.push(&format!("let arg_{}: {} = args[{}].clone().try_into_native().unwrap();\n", i, ty.to_token_stream(), i));
                            }
                        }

                        // Call function
                        writer.push_indent();
                        let call_args: Vec<_> = (0..f.sig.inputs.len())
                            .map(|i| format!("arg_{}", i))
                            .collect();
                        writer.push(&format!(
                            "let __res = {}({});\n",
                            name,
                            call_args.join(", ")
                        ));

                        // Handle return
                        writer.push_indent();
                        if let ReturnType::Default = f.sig.output {
                            writer.push("CallResult::Return(CrawValue::None)\n");
                        } else {
                            writer.push("CallResult::Return(CrawValue::from(__res))\n");
                        }

                        writer.dedent();
                        writer.push_line("}));");
                    }
                }
            }
        }
        Stmt::DataDef(name, fields, _) => {
            let esc_name = escape_ident(name);
            writer.push_indent();
            if !globals.contains(name) && !nonlocals.contains(name) && !defined.contains(name) {
                writer.push("let mut ");
                writer.push(&esc_name);
                writer.push(" = CrawValue::None;\n");
                writer.push_indent();
                defined.insert(name.clone());
            }
            writer.push(&esc_name);
            writer.push(" = CrawValue::Closure(Rc::new(|mut args| {\n");
            writer.indent();
            for (i, (_, _, default)) in fields.iter().enumerate() {
                if let Some(def_expr) = default {
                    writer.push_indent();
                    writer.push("if args.len() <= ");
                    writer.push(&i.to_string());
                    writer.push(" { args.push(");
                    transpile_expr_as_value(def_expr, writer, current_scope, scopes, native_funcs);
                    writer.push("); }\n");
                }
            }
            writer.push_indent();
            writer.push("CallResult::Return(CrawValue::Data(\"");
            writer.push(name);
            writer.push("\".to_string(), vec![");
            for (f, _, _) in fields {
                writer.push("\"");
                writer.push(f);
                writer.push("\".to_string(), ");
            }
            writer.push("], Rc::new(RefCell::new(vec![");
            for (i, _) in fields.iter().enumerate() {
                if i > 0 {
                    writer.push(", ");
                }
                writer.push("args[");
                writer.push(&i.to_string());
                writer.push("].clone()");
            }
            writer.push("]))))\n");
            writer.dedent();
            writer.push_line("}));");
        }
        Stmt::Match(expr, cases) => {
            writer.push_indent();
            writer.push("let __match_val_");
            writer.push(&writer.indent_level.to_string());
            writer.push(" = ");
            transpile_expr_as_value(expr, writer, current_scope, scopes, native_funcs);
            writer.push(";\n");

            let match_val_name = format!("__match_val_{}", writer.indent_level);
            let match_path = writer.indent_level.to_string();

            for (i, (pat, guard, body)) in cases.iter().enumerate() {
                let prefix = if i == 0 { "if" } else { "} else if" };
                let (cond, binds) = build_pattern(
                    pat,
                    &match_val_name,
                    &match_path,
                    writer.indent_level,
                    current_scope,
                    scopes,
                    native_funcs,
                );
                writer.push_indent();
                writer.push(prefix);
                writer.push(" ");
                if let Some(g) = guard {
                    writer.push("(");
                    writer.push(&cond);
                    writer.push(") && { ");
                    writer.push(&binds);
                    writer.push("craw_is_truthy(&craw_driver(");
                    transpile_expr(g, writer, current_scope, scopes, native_funcs);
                    writer.push(")) }");
                } else {
                    writer.push(&cond);
                }
                writer.push(" {\n");
                writer.indent();
                writer.push(&binds);
                transpile_block(
                    body,
                    writer,
                    current_scope,
                    defined,
                    scopes,
                    in_native_context,
                    is_trait,
                    native_funcs,
                );
                writer.dedent();
            }

            if !cases.is_empty() {
                writer.push_line("}");
            }
        }
        Stmt::MatchFor(pat, expr, body) => {
            let loop_level = writer.indent_level;
            writer.push_indent();
            writer.push("let __iter_");
            writer.push(&loop_level.to_string());
            writer.push(" = ");
            transpile_expr_as_value(expr, writer, current_scope, scopes, native_funcs);
            writer.push(";\n");

            let iter_name = format!("__iter_{}", loop_level);
            let item_name = format!("__item_{}", loop_level);
            let iter_path = format!("iter_{}", loop_level);

            writer.push_indent();
            writer.push("if let CrawValue::List(__items_");
            writer.push(&loop_level.to_string());
            writer.push(") = &");
            writer.push(&iter_name);
            writer.push(" {\n");
            writer.indent();

            writer.push_indent();
            writer.push("for ");
            writer.push(&item_name);
            writer.push(" in __items_");
            writer.push(&loop_level.to_string());
            writer.push(".borrow().iter() {\n");
            writer.indent();

            let (cond, binds) = build_pattern(
                pat,
                &item_name,
                &iter_path,
                writer.indent_level,
                current_scope,
                scopes,
                native_funcs,
            );
            if cond != "true" {
                writer.push_indent();
                writer.push("if !");
                writer.push(&cond);
                writer.push(" { panic!(\"MatchError: loop item did not match pattern\"); }\n");
            }
            writer.push(&binds);
            transpile_block(
                body,
                writer,
                current_scope,
                defined,
                scopes,
                in_native_context,
                is_trait,
                native_funcs,
            );

            writer.dedent();
            writer.push_line("}");
            writer.dedent();
            writer.push_line("} else { panic!(\"TypeError: expected list for loop\"); }");
        }
        Stmt::IndexAssign(target, index, value) => {
            writer.push_indent();
            writer.push("craw_set_item(");
            transpile_expr_as_value(target, writer, current_scope, scopes, native_funcs);
            writer.push(", ");
            transpile_expr_as_value(index, writer, current_scope, scopes, native_funcs);
            writer.push(", ");
            transpile_expr_as_value(value, writer, current_scope, scopes, native_funcs);
            writer.push(");\n");
        }
        Stmt::AttributeAssign(target, attr, value) => {
            writer.push_indent();
            if in_native_context
                && let Expr::Ident(name) = target
                && name == "self"
            {
                writer.push("self.");
                writer.push(attr);
                writer.push(" = ");
                transpile_expr_as_value(value, writer, current_scope, scopes, native_funcs);
                writer.push(";\n");
            } else {
                writer.push("craw_set_attr(");
                transpile_expr_as_value(target, writer, current_scope, scopes, native_funcs);
                writer.push(", \"");
                writer.push(attr);
                writer.push("\", ");
                transpile_expr_as_value(value, writer, current_scope, scopes, native_funcs);
                writer.push(");\n");
            }
        }
        Stmt::NativeImport(path, items) => {
            writer.push_indent();
            writer.push("use ");
            writer.push(&path.join("::"));
            writer.push("::{");
            writer.push(&items.join(", "));
            writer.push("};\n");
        }
        Stmt::StructDef(name, fields, _) => {
            writer.push_indent();
            writer.push("#[derive(Clone, Debug)] pub struct ");
            writer.push(name);
            writer.push(" { ");
            for (i, (fname, fty)) in fields.iter().enumerate() {
                if i > 0 {
                    writer.push(", ");
                }
                writer.push("pub ");
                writer.push(fname);
                writer.push(": ");
                writer.push(&type_to_rust(fty));
            }
            writer.push(" }\n");
        }
        Stmt::TraitDef(name, body, id) => {
            writer.push_indent();
            writer.push("pub trait ");
            writer.push(name);
            writer.push(" {\n");
            writer.indent();
            let info = scopes.get(id).expect("Trait scope info not found");
            let mut trait_defined = HashSet::new();
            transpile_block(
                body,
                writer,
                info,
                &mut trait_defined,
                scopes,
                true,
                true,
                native_funcs,
            );
            writer.dedent();
            writer.push_line("}");
        }
        Stmt::ImplBlock(trait_ty, target_ty, body, id) => {
            writer.push_indent();
            writer.push("impl ");
            if let Some(t) = trait_ty {
                writer.push(&type_to_rust(t));
                writer.push(" for ");
            }
            writer.push(&type_to_rust(target_ty));
            writer.push(" {\n");
            writer.indent();
            let info = scopes.get(id).expect("Impl scope info not found");
            let mut impl_defined = HashSet::new();
            transpile_block(
                body,
                writer,
                info,
                &mut impl_defined,
                scopes,
                true,
                false,
                native_funcs,
            );
            writer.dedent();
            writer.push_line("}");
        }
        Stmt::Global(_) | Stmt::Nonlocal(_) | Stmt::TemplateDef(_, _, _, _) => {}
        Stmt::Operator(op_spec) => {
            if op_spec.contains(':') {
                let parts: Vec<&str> = op_spec.split(':').collect();
                let op = parts[0];
                let func = parts[1];
                let op_name = escape_ident(&format!("__op_{}", op));
                let func_val = if BUILTINS.iter().any(|(b, _)| *b == func)
                    && !current_scope.defined.contains(func)
                {
                    format!("CrawValue::Builtin({:?}.to_string())", func)
                } else {
                    format!("{}.clone()", escape_ident(func))
                };
                writer.push_indent();
                if defined.contains(&op_name) {
                    writer.push_line(&format!("{} = {};", op_name, func_val));
                } else {
                    writer.push_line(&format!("let mut {} = {};", op_name, func_val));
                    defined.insert(op_name);
                }
            }
        }
        Stmt::Use(path) => {
            writer.push_indent();
            writer.push_line(&format!("use {};", path));
        }
        Stmt::MacroBlock {
            name, token_pos, ..
        } => {
            panic!(
                "Unexpanded MacroBlock at token {} reached transpiler - custom macro '{}' not defined!",
                token_pos, name
            );
        }
        Stmt::MacroDef { name, args, body } => {
            let esc_name = escape_ident(name);
            writer.push_line(&format!("macro_rules! {} {{", esc_name));
            writer.indent();

            // Format arguments as $arg:expr
            let arg_pattern = args
                .iter()
                .map(|a| format!("${}:expr", a))
                .collect::<Vec<_>>()
                .join(", ");

            writer.push_line(&format!("({}) => {{{{", arg_pattern));
            writer.indent();

            // For the body, we need to map the argument identifiers to the macro variables (i.e. `$arg`).
            // The trick is to replace Ident(arg) with a special Passthrough("$arg") or similar.
            // Or just string replace $arg on the generated Rust string. Let's transpile the body to a string first.

            let mut body_writer = CodeWriter::new(100);
            body_writer.indent_level = writer.indent_level;

            // Create a temporary scope
            let mut local_defined = defined.clone();
            // Note: we don't have scope info for the macro body unless we added it in analyze.
            // We can just use the current scope, and assume macro variables are passed through.
            let temp_scope = current_scope.clone();

            // To ensure args are formatted as $arg, we can use the `defined` set or similar.
            // Actually, if we just substitute Expr::Ident(arg) -> Expr::Passthrough(format!("${}", arg)) it works perfectly!

            let mut mapping = HashMap::new();
            for arg in args {
                mapping.insert(arg.clone(), Expr::Ident(format!("${}", arg)));
            }

            let expander = TemplateExpander::new(); // reuse substitute_expr logic
            let mut new_body = Vec::new();
            for stmt in body {
                new_body.push(expander.substitute_stmt(stmt.clone(), &mapping));
            }

            let last_expr = if let Some(Stmt::Expr(_)) = new_body.last() {
                if let Stmt::Expr(e) = new_body.pop().unwrap() {
                    Some(e)
                } else {
                    unreachable!()
                }
            } else {
                None
            };

            transpile_block(
                &new_body,
                &mut body_writer,
                &temp_scope,
                &mut local_defined,
                scopes,
                in_native_context,
                is_trait,
                native_funcs,
            );

            if let Some(e) = last_expr {
                body_writer.push_indent();
                transpile_expr(&e, &mut body_writer, &temp_scope, scopes, native_funcs);
                body_writer.push("\n");
            } else {
                body_writer.push_indent();
                body_writer.push("CallResult::Return(CrawValue::None)\n");
            }

            let body_str = body_writer.finish();
            writer.push(&body_str);

            writer.dedent();
            writer.push_line("}};");
            writer.dedent();
            writer.push_line("}");
        }
        Stmt::ClassDef {
            name,
            args,
            superclass,
            traits,
            body,
            id,
        } => {
            writer.push_indent();
            writer.push("#[derive(Clone, Debug)] pub struct ");
            writer.push(name);
            writer.push(" {\n");
            writer.indent();
            if let Some((super_name, _)) = superclass {
                writer.push_indent();
                writer.push(&format!(
                    "pub parent: {},\n",
                    type_to_rust(&Type::Custom(super_name.clone()))
                ));
            }
            for arg in args {
                writer.push_indent();
                writer.push(&format!("pub {}: CrawValue,\n", arg));
            }
            writer.dedent();
            writer.push_line("}");

            if let Some((super_name, _)) = superclass {
                writer.push_line(&format!("impl std::ops::Deref for {} {{", name));
                writer.indent();
                writer.push_line(&format!(
                    "type Target = {};",
                    type_to_rust(&Type::Custom(super_name.clone()))
                ));
                writer.push_line("fn deref(&self) -> &Self::Target { &self.parent }");
                writer.dedent();
                writer.push_line("}");
            }

            for t in traits {
                let trait_rust = type_to_rust(&Type::Custom(t.clone()));
                if !current_scope.types.contains_key(t) {
                    writer.push_line(&format!("pub trait {} {{}}", trait_rust));
                }
                writer.push_line(&format!("impl {} for {} {{}}", trait_rust, name));
            }

            writer.push_line(&format!("impl From<{}> for CrawValue {{", name));
            writer.indent();
            writer.push_line(&format!("fn from(val: {}) -> Self {{ CrawValue::Native(Rc::new(std::cell::RefCell::new(val))) }}", name));
            writer.dedent();
            writer.push_line("}");

            writer.push_line(&format!("impl TryFrom<CrawValue> for {} {{", name));
            writer.indent();
            writer.push_line("type Error = String;");
            writer.push_line(&format!("fn try_from(val: CrawValue) -> Result<Self, Self::Error> {{ if let CrawValue::Native(ref any) = val {{ if let Some(obj) = any.downcast_ref::<std::cell::RefCell<{}>>() {{ return Ok(obj.borrow().clone()); }} }} Err(\"TypeError: Expected {}\".to_string()) }}", name, name));
            writer.dedent();
            writer.push_line("}");

            writer.push_indent();
            writer.push("impl ");
            writer.push(name);
            writer.push(" {\n");
            writer.indent();
            let info = scopes.get(id).expect("Class scope info not found");
            let mut class_defined = HashSet::new();
            transpile_block(
                body,
                writer,
                info,
                &mut class_defined,
                scopes,
                true,
                false,
                native_funcs,
            );
            writer.dedent();
            writer.push_line("}");

            writer.push_indent();
            if globals.contains(name) || nonlocals.contains(name) {
                writer.push(&format!(
                    "if let CrawValue::Recursive(ref r) = {} {{ *r.borrow_mut() = ",
                    escape_ident(name)
                ));
            } else if defined.contains(name) {
                writer.push(&format!("{} = ", escape_ident(name)));
            } else {
                writer.push(&format!("let mut {} = ", escape_ident(name)));
                defined.insert(name.clone());
            }

            writer.push("CrawValue::Closure(Rc::new({ ");
            let mut class_captures: Vec<_> = info.captured.iter().collect();
            class_captures.sort();
            for cap in &class_captures {
                let esc_cap = escape_ident(cap);
                writer.push(&format!("let {0} = {0}.clone(); ", esc_cap));
            }
            if let Some((super_name, _)) = superclass {
                writer.push(&format!(
                    "let __craw_id_{} = {}.clone(); ",
                    super_name,
                    escape_ident(super_name)
                ));
            }

            writer.push("move |args| { ");
            for (i, arg) in args.iter().enumerate() {
                writer.push(&format!(
                    "let __arg_{} = args.get({}).cloned().unwrap_or(CrawValue::None); ",
                    arg, i
                ));
                writer.push(&format!(
                    "let {} = __arg_{}.clone(); ",
                    escape_ident(arg),
                    arg
                ));
            }

            writer.push(&format!("let mut __obj = {} {{ ", name));
            if let Some((super_name, exprs)) = superclass {
                writer.push("parent: { let __parent_val = craw_driver(craw_call(");
                writer.push(&format!("__craw_id_{}.clone()", super_name));
                writer.push(", vec![");
                for (i, e) in exprs.iter().enumerate() {
                    if i > 0 {
                        writer.push(", ");
                    }
                    transpile_expr_as_value(e, writer, current_scope, scopes, native_funcs);
                }
                writer.push("])); if let CrawValue::Native(ref any) = __parent_val { any.downcast_ref::<std::cell::RefCell<");
                writer.push(super_name);
                writer.push(">>().unwrap().borrow().clone() } else { panic!(\"Expected Native object\"); } }, ");
            }
            for arg in args {
                writer.push(&format!("{}: __arg_{}.clone(), ", arg, arg));
            }
            writer.push("}; ");
            writer.push(
                "CallResult::Return(CrawValue::Native(Rc::new(std::cell::RefCell::new(__obj))))",
            );
            writer.push(" } }));\n");

            if globals.contains(name) || nonlocals.contains(name) {
                writer.push(" }\n");
            }
        }
    }
}

fn broadcast_everything(expr: Expr) -> Expr {
    match expr {
        Expr::BinaryOp(l, op, r) => {
            let l_b = Box::new(broadcast_everything(*l));
            let r_b = Box::new(broadcast_everything(*r));
            let func = match op.as_str() {
                "+" => "add",
                "-" => "sub",
                "*" => "mul",
                "/" => "div",
                "÷" => "div_int",
                "%" => "mod",
                "==" => "eq",
                "!=" | "≠" => "ne",
                "<" => "lt",
                "<=" | "≤" => "le",
                ">" => "gt",
                ">=" | "≥" => "ge",
                "**" => "pow",
                "≈" => "approx",
                "∈" => "in",
                "∉" => "notin",
                "to" => "to",
                "until" => "until",
                _ => &op,
            };
            Expr::BroadcastCall(Box::new(Expr::Ident(func.to_string())), vec![*l_b, *r_b])
        }
        Expr::Call(f, args) => {
            let f_b = Box::new(broadcast_everything(*f));
            let args_b = args.into_iter().map(broadcast_everything).collect();
            Expr::BroadcastCall(f_b, args_b)
        }
        Expr::Tuple(items) => Expr::Tuple(items.into_iter().map(broadcast_everything).collect()),
        Expr::List(items) => Expr::List(items.into_iter().map(broadcast_everything).collect()),
        Expr::Ternary(c, t, e) => Expr::Ternary(
            Box::new(broadcast_everything(*c)),
            Box::new(broadcast_everything(*t)),
            Box::new(broadcast_everything(*e)),
        ),
        Expr::Lambda(args, body, id) => {
            Expr::Lambda(args, Box::new(broadcast_everything(*body)), id)
        }
        Expr::Pipe(l, data, r) => Expr::Pipe(
            Box::new(broadcast_everything(*l)),
            data,
            Box::new(broadcast_everything(*r)),
        ),
        Expr::NoneCoalesce(l, r) => Expr::NoneCoalesce(
            Box::new(broadcast_everything(*l)),
            Box::new(broadcast_everything(*r)),
        ),
        Expr::PartialCall(f, args, id) => Expr::PartialCall(
            Box::new(broadcast_everything(*f)),
            args.into_iter()
                .map(|o| o.map(broadcast_everything))
                .collect(),
            id,
        ),
        Expr::Index(e, i) => Expr::Index(
            Box::new(broadcast_everything(*e)),
            Box::new(broadcast_everything(*i)),
        ),
        Expr::Attribute(e, attr) => Expr::Attribute(Box::new(broadcast_everything(*e)), attr),
        Expr::MacroCall(name, args) => {
            Expr::MacroCall(name, args.into_iter().map(broadcast_everything).collect())
        }
        Expr::Splat(e) => Expr::Splat(Box::new(broadcast_everything(*e))),
        Expr::Formula(l, r) => Expr::Formula(
            Box::new(broadcast_everything(*l)),
            Box::new(broadcast_everything(*r)),
        ),
        Expr::Query {
            from,
            in_expr,
            clauses,
            select,
            id,
        } => Expr::Query {
            from,
            in_expr: Box::new(broadcast_everything(*in_expr)),
            clauses: clauses
                .into_iter()
                .map(|c| match c {
                    QueryClause::Where(e) => QueryClause::Where(broadcast_everything(e)),
                    QueryClause::OrderBy(e, asc) => {
                        QueryClause::OrderBy(broadcast_everything(e), asc)
                    }
                })
                .collect(),
            select: Box::new(broadcast_everything(*select)),
            id,
        },
        _ => expr,
    }
}

fn transpile_call_args(
    args: &[Expr],
    writer: &mut CodeWriter,
    current_scope: &ScopeInfo,
    scopes: &HashMap<usize, ScopeInfo>,
    native_funcs: &HashSet<String>,
) {
    let has_splat = args.iter().any(|arg| matches!(arg, Expr::Splat(_)));
    if has_splat {
        writer.push("{ let mut __args = vec![]; ");
        for arg in args {
            if let Expr::Splat(inner) = arg {
                writer.push("if let CrawValue::List(ref __items) = (");
                transpile_expr_as_value(inner, writer, current_scope, scopes, native_funcs);
                writer.push(") { __args.extend(__items.borrow().iter().cloned()); } else { panic!(\"TypeError: splat expects a list\"); } ");
            } else {
                writer.push("__args.push(");
                transpile_expr_as_value(arg, writer, current_scope, scopes, native_funcs);
                writer.push("); ");
            }
        }
        writer.push("__args }");
    } else {
        writer.push("vec![");
        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                writer.push(", ");
            }
            transpile_expr_as_value(arg, writer, current_scope, scopes, native_funcs);
        }
        writer.push("]");
    }
}

fn expr_to_craw_source(expr: &Expr) -> String {
    match expr {
        Expr::Ident(name) => name.clone(),
        Expr::Number(n) => n.to_string(),
        Expr::Float(f) => {
            let s = f.to_string();
            if !s.contains('.') {
                format!("{}.0", s)
            } else {
                s
            }
        }
        Expr::String(s) => format!("\"{}\"", s),
        Expr::FString(exprs) => {
            let parts: Vec<String> = exprs
                .iter()
                .map(|e| match e {
                    Expr::String(s) => s.clone(),
                    _ => format!("{{{}}}", expr_to_craw_source(e)),
                })
                .collect();
            format!("f\"{}\"", parts.join(""))
        }
        Expr::Shell(cmd) => format!("`{}`", cmd),
        Expr::Bool(b) => if *b { "True" } else { "False" }.to_string(),
        Expr::None => "None".to_string(),
        Expr::BinaryOp(l, op, r) => format!(
            "({} {} {})",
            expr_to_craw_source(l),
            op,
            expr_to_craw_source(r)
        ),
        Expr::Call(f, args) => {
            let args_strs: Vec<_> = args.iter().map(expr_to_craw_source).collect();
            format!("{}({})", expr_to_craw_source(f), args_strs.join(", "))
        }
        Expr::BroadcastCall(f, args) => {
            let args_strs: Vec<_> = args.iter().map(expr_to_craw_source).collect();
            format!("{}.({})", expr_to_craw_source(f), args_strs.join(", "))
        }
        Expr::Lambda(args, body, _) => {
            format!("|{}| => {}", args.join(", "), expr_to_craw_source(body))
        }
        Expr::Pipe(l, data, r) => {
            let op = match data.style {
                CallStyle::Standard => {
                    if data.none_aware {
                        "?|>"
                    } else {
                        "|>"
                    }
                }
                CallStyle::Star => "*|>",
                CallStyle::DoubleStar => "**|>",
            };
            format!(
                "{} {} {}",
                expr_to_craw_source(l),
                op,
                expr_to_craw_source(r)
            )
        }
        Expr::Compose(l, _, r, _) => {
            format!("{} .. {}", expr_to_craw_source(l), expr_to_craw_source(r))
        }
        Expr::NoneCoalesce(l, r) => {
            format!("{} ?? {}", expr_to_craw_source(l), expr_to_craw_source(r))
        }
        Expr::PartialCall(f, args, _) => {
            let args_strs: Vec<_> = args
                .iter()
                .map(|a| match a {
                    Some(e) => expr_to_craw_source(e),
                    None => "_".to_string(),
                })
                .collect();
            format!("{}({})", expr_to_craw_source(f), args_strs.join(", "))
        }
        Expr::List(elements) => {
            let el_strs: Vec<_> = elements.iter().map(expr_to_craw_source).collect();
            format!("[{}]", el_strs.join(", "))
        }
        Expr::Dict(pairs) => {
            let pair_strs: Vec<_> = pairs
                .iter()
                .map(|(k, v)| format!("{}: {}", expr_to_craw_source(k), expr_to_craw_source(v)))
                .collect();
            format!("{{{}}}", pair_strs.join(", "))
        }
        Expr::Index(obj, idx) => {
            format!("{}[{}]", expr_to_craw_source(obj), expr_to_craw_source(idx))
        }
        Expr::Attribute(obj, attr) => format!("{}.{}", expr_to_craw_source(obj), attr),
        Expr::AttributePartial(attr) => format!(".{}", attr),
        Expr::ImplicitLambda(body, _) => format!("|=> {}", expr_to_craw_source(body)),
        Expr::Where(expr, stmts, _) => {
            let stmt_strs: Vec<_> = stmts.iter().map(stmt_to_craw_source).collect();
            format!(
                "({} where {})",
                expr_to_craw_source(expr),
                stmt_strs.join("; ")
            )
        }
        Expr::Set(elements) => {
            let el_strs: Vec<_> = elements.iter().map(expr_to_craw_source).collect();
            format!("set([{}])", el_strs.join(", "))
        }
        Expr::Frozenset(elements) => {
            let el_strs: Vec<_> = elements.iter().map(expr_to_craw_source).collect();
            format!("frozenset([{}])", el_strs.join(", "))
        }
        Expr::Multiset(elements) => {
            let el_strs: Vec<_> = elements.iter().map(expr_to_craw_source).collect();
            format!("multiset([{}])", el_strs.join(", "))
        }
        Expr::LazyList(elements, _) => {
            let el_strs: Vec<_> = elements.iter().map(expr_to_craw_source).collect();
            format!("lazy([{}])", el_strs.join(", "))
        }
        Expr::IndexPartial(obj) => format!("{}[_]", expr_to_craw_source(obj)),
        Expr::OperatorFunction(op) => format!("`{}`", op),
        Expr::Passthrough(s) => format!("`{}`", s),
        Expr::Ternary(c, t, e) => format!(
            "if {} then {} else {}",
            expr_to_craw_source(c),
            expr_to_craw_source(t),
            expr_to_craw_source(e)
        ),
        Expr::Tuple(elements) => {
            let el_strs: Vec<_> = elements.iter().map(expr_to_craw_source).collect();
            format!("({})", el_strs.join(", "))
        }
        Expr::Placeholder => "_".to_string(),
        Expr::Splat(e) => format!("*{}", expr_to_craw_source(e)),
        Expr::MacroCall(name, args) => format!(
            "@{}({})",
            name,
            args.iter()
                .map(expr_to_craw_source)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Expr::Slice(start, stop, step) => {
            format!(
                "{}:{}:{}",
                start
                    .as_ref()
                    .map(|e| expr_to_craw_source(e))
                    .unwrap_or_default(),
                stop.as_ref()
                    .map(|e| expr_to_craw_source(e))
                    .unwrap_or_default(),
                step.as_ref()
                    .map(|e| expr_to_craw_source(e))
                    .unwrap_or_default()
            )
        }
        Expr::Comprehension(expr, pat, iterable, is_lazy, _) => {
            format!(
                "[{} for {} in {}{}]",
                expr_to_craw_source(expr),
                pattern_to_craw_source(pat),
                expr_to_craw_source(iterable),
                if *is_lazy { " lazy" } else { "" }
            )
        }
        Expr::Hcat(elements) => {
            let el_strs: Vec<_> = elements.iter().map(expr_to_craw_source).collect();
            format!("[{}]", el_strs.join(" "))
        }
        Expr::Vcat(elements) => {
            let el_strs: Vec<_> = elements.iter().map(expr_to_craw_source).collect();
            format!("[{}]", el_strs.join("; "))
        }
        Expr::Formula(l, r) => format!("{} ~ {}", expr_to_craw_source(l), expr_to_craw_source(r)),
        Expr::Range(l, r) => format!("{}..{}", expr_to_craw_source(l), expr_to_craw_source(r)),
        Expr::Gather(e, f) => format!("{}..{}", expr_to_craw_source(e), f),
        Expr::Query {
            from,
            in_expr,
            clauses,
            select,
            ..
        } => {
            let mut s = format!("from {} in {} ", from, expr_to_craw_source(in_expr));
            for clause in clauses {
                match clause {
                    QueryClause::Where(e) => {
                        s.push_str(&format!("where {} ", expr_to_craw_source(e)))
                    }
                    QueryClause::OrderBy(e, asc) => s.push_str(&format!(
                        "orderby {} {} ",
                        expr_to_craw_source(e),
                        if *asc { "asc" } else { "desc" }
                    )),
                }
            }
            s.push_str(&format!("select {}", expr_to_craw_source(select)));
            s
        }
    }
}

fn pattern_to_craw_source(pat: &Pattern) -> String {
    match pat {
        Pattern::Var(name, ty) => {
            if let Some(t) = ty {
                format!("{}: {}", name, type_to_craw_source(t))
            } else {
                name.clone()
            }
        }
        Pattern::Const(e) => expr_to_craw_source(e),
        Pattern::Data(name, fields) => {
            let field_strs: Vec<_> = fields.iter().map(pattern_to_craw_source).collect();
            format!("{}({})", name, field_strs.join(", "))
        }
        Pattern::Wildcard => "_".to_string(),
        Pattern::View(expr, pat) => format!(
            "({} -> {})",
            expr_to_craw_source(expr),
            pattern_to_craw_source(pat)
        ),
        Pattern::StringSplit(pat_str, var, is_prefix) => {
            if *is_prefix {
                format!("\"{}\" ++ {}", pat_str, var)
            } else {
                format!("{} ++ \"{}\"", var, pat_str)
            }
        }
        Pattern::Tuple(elements) => {
            let el_strs: Vec<_> = elements.iter().map(pattern_to_craw_source).collect();
            format!("({})", el_strs.join(", "))
        }
        Pattern::Rest(name) => format!("*{}", name),
    }
}

fn type_to_craw_source(ty: &Type) -> String {
    match ty {
        Type::Int => "Int".to_string(),
        Type::String => "String".to_string(),
        Type::Custom(s) => s.clone(),
        Type::Generic(s, args) => {
            let arg_strs: Vec<_> = args.iter().map(type_to_craw_source).collect();
            format!("{}<{}>", s, arg_strs.join(", "))
        }
        Type::Dynamic => "Dynamic".to_string(),
    }
}

fn stmt_to_craw_source(stmt: &Stmt) -> String {
    match stmt {
        Stmt::Assign(pat, expr) => format!(
            "{} = {}",
            pattern_to_craw_source(pat),
            expr_to_craw_source(expr)
        ),
        Stmt::Expr(expr) => expr_to_craw_source(expr),
        Stmt::FunctionDef { name, args, .. } => {
            let arg_strs: Vec<_> = args
                .iter()
                .map(|(p, _)| pattern_to_craw_source(p))
                .collect();
            format!("fn {}({}) => ...", name.join("."), arg_strs.join(", "))
        }
        _ => "...".to_string(),
    }
}

fn transpile_query_lambda(
    arg_name: &str,
    body: &Expr,
    writer: &mut CodeWriter,
    _current_scope: &ScopeInfo,
    scopes: &HashMap<usize, ScopeInfo>,
    id: usize,
    native_funcs: &HashSet<String>,
) {
    let info = scopes.get(&id).expect("Query scope info not found");
    let mut captures: Vec<_> = info.captured.iter().collect();
    captures.sort();

    writer.push("CrawValue::Closure(Rc::new({ ");
    for cap in &captures {
        let esc_cap = escape_ident(cap);
        writer.push(&format!("let {0} = {0}.clone(); ", esc_cap));
    }
    writer.push("move |args| { ");
    writer.push(&format!(
        "let {} = args[0].clone(); ",
        escape_ident(arg_name)
    ));
    transpile_expr(body, writer, info, scopes, native_funcs);
    writer.push(" } }))");
}

fn transpile_query(
    from: &str,
    in_expr: &Expr,
    clauses: &[QueryClause],
    select: &Expr,
    id: usize,
    writer: &mut CodeWriter,
    current_scope: &ScopeInfo,
    scopes: &HashMap<usize, ScopeInfo>,
    native_funcs: &HashSet<String>,
) {
    writer.push("{ let mut __q_res = ");
    transpile_expr_as_value(in_expr, writer, current_scope, scopes, native_funcs);
    writer.push(";\n");
    for clause in clauses {
        match clause {
            QueryClause::Where(cond) => {
                writer.push_indent();
                writer.push(
                    "__q_res = craw_driver(craw_ufcs_call(__q_res.clone(), \"filter\", vec![",
                );
                transpile_query_lambda(from, cond, writer, current_scope, scopes, id, native_funcs);
                writer.push("], ");
                let filter_expr = if current_scope.defined.contains("filter")
                    || current_scope.captured.contains("filter")
                {
                    "Some(filter.clone())".to_string()
                } else {
                    "Some(CrawValue::Builtin(\"filter\".to_string()))".to_string()
                };
                writer.push(&filter_expr);
                writer.push("));\n");
            }
            QueryClause::OrderBy(key, asc) => {
                writer.push_indent();
                writer.push(
                    "__q_res = craw_driver(craw_ufcs_call(__q_res.clone(), \"orderby\", vec![",
                );
                transpile_query_lambda(from, key, writer, current_scope, scopes, id, native_funcs);
                writer.push(&format!(", CrawValue::Bool({})], ", asc));
                let orderby_expr = if current_scope.defined.contains("orderby")
                    || current_scope.captured.contains("orderby")
                {
                    "Some(orderby.clone())".to_string()
                } else {
                    "Some(CrawValue::Builtin(\"orderby\".to_string()))".to_string()
                };
                writer.push(&orderby_expr);
                writer.push("));\n");
            }
        }
    }
    writer.push_indent();
    writer.push("__q_res = craw_driver(craw_ufcs_call(__q_res.clone(), \"fmap\", vec![");
    transpile_query_lambda(
        from,
        select,
        writer,
        current_scope,
        scopes,
        id,
        native_funcs,
    );
    writer.push("], ");
    let fmap_expr =
        if current_scope.defined.contains("fmap") || current_scope.captured.contains("fmap") {
            "Some(fmap.clone())".to_string()
        } else {
            "Some(CrawValue::Builtin(\"fmap\".to_string()))".to_string()
        };
    writer.push(&fmap_expr);
    writer.push("));\n");
    writer.push_indent();
    writer.push("CallResult::Return(__q_res) }");
}

fn transpile_expr(
    expr: &Expr,
    writer: &mut CodeWriter,
    current_scope: &ScopeInfo,
    scopes: &HashMap<usize, ScopeInfo>,
    native_funcs: &HashSet<String>,
) {
    match expr {
        Expr::Float(f) => {
            writer.push("CallResult::Return(CrawValue::Float(");
            let s = f.to_string();
            writer.push(&s);
            if !s.contains('.') {
                writer.push(".0");
            }
            writer.push("))");
        }
        Expr::Number(n) => {
            writer.push("CallResult::Return(CrawValue::Int(");
            writer.push(&n.to_string());
            writer.push("))");
        }
        Expr::String(s) => {
            writer.push(&format!(
                "CallResult::Return(CrawValue::String(Rc::new({:?}.to_string())))",
                s
            ));
        }
        Expr::FString(exprs) => {
            writer.push("CallResult::Return(CrawValue::String(Rc::new(format!(");

            let mut format_str = String::new();
            for e in exprs {
                match e {
                    Expr::String(s) => {
                        format_str.push_str(&s.replace("{", "{{").replace("}", "}}"))
                    }
                    _ => format_str.push_str("{}"),
                }
            }
            writer.push(&format!("{:?}", format_str));

            for e in exprs {
                if !matches!(e, Expr::String(_)) {
                    writer.push(", ");
                    writer.push("match ");
                    transpile_expr(e, writer, current_scope, scopes, native_funcs);
                    writer
                        .push(" { CallResult::Return(v) => v.to_string(), _ => \"\".to_string() }");
                }
            }
            writer.push("))))");
        }
        Expr::Shell(cmd) => {
            writer.push(&format!("CallResult::Return(CrawValue::String(Rc::new(String::from_utf8_lossy(&std::process::Command::new(\"sh\").arg(\"-c\").arg({:?}).output().expect(\"Command failed\").stdout).into_owned())))", cmd));
        }
        Expr::Bool(b) => {
            writer.push("CallResult::Return(CrawValue::Bool(");
            writer.push(if *b { "true" } else { "false" });
            writer.push("))");
        }
        Expr::None => {
            writer.push("CallResult::Return(CrawValue::None)");
        }
        Expr::Ident(name) if name == "_" => {
            writer.push("CallResult::Return(__craw_underscore.clone())");
        }
        Expr::Ident(name) => {
            let esc_name = escape_ident(name);
            if let Some(ty) = current_scope.types.get(name)
                && !matches!(ty, Type::Dynamic)
            {
                writer.push("CallResult::Return(CrawValue::from(");
                writer.push(&esc_name);
                writer.push("))");
                return;
            }
            if BUILTINS.iter().any(|(b, _)| b == name) && !current_scope.defined.contains(name) {
                writer.push(&format!(
                    "CallResult::Return(CrawValue::Builtin({:?}.to_string()))",
                    name
                ));
            } else if name.starts_with("$") {
                writer.push(&esc_name);
            } else {
                writer.push("CallResult::Return(");
                writer.push(&esc_name);
                writer.push(".clone())");
            }
        }
        Expr::BinaryOp(left, op, r) => {
            let op_name = escape_ident(&format!("__op_{}", op));
            if current_scope.defined.contains(&op_name) || current_scope.captured.contains(&op_name)
            {
                writer.push("craw_call(");
                writer.push(&op_name);
                writer.push(".clone(), vec![");
                transpile_expr_as_value(left, writer, current_scope, scopes, native_funcs);
                writer.push(", ");
                transpile_expr_as_value(r, writer, current_scope, scopes, native_funcs);
                writer.push("])");
                return;
            }
            let func2 = match op.as_str() {
                "+" => Some("craw_add2"),
                "-" => Some("craw_sub2"),
                "*" => Some("craw_mul2"),
                "/" => Some("craw_div2"),
                "÷" => Some("craw_div_int2"),
                "%" => Some("craw_mod2"),
                "==" => Some("craw_eq2"),
                "!=" | "≠" => Some("craw_ne2"),
                "<" => Some("craw_lt2"),
                "<=" | "≤" => Some("craw_le2"),
                ">" => Some("craw_gt2"),
                ">=" | "≥" => Some("craw_ge2"),
                "**" => Some("craw_pow2"),
                "in" | "∈" => Some("craw_in2"),
                "notin" | "∉" => Some("craw_notin2"),
                "|" | "∪" => Some("craw_union2"),
                "&" | "∩" => Some("craw_intersection2"),
                "⊆" => Some("craw_subset2"),
                "⊇" => Some("craw_superset2"),
                "to" => Some("craw_to2"),
                "until" => Some("craw_until2"),
                _ => None,
            };

            if let Some(f2) = func2 {
                writer.push("CallResult::Return(");
                writer.push(f2);
                writer.push("(");
                transpile_expr_as_value(left, writer, current_scope, scopes, native_funcs);
                writer.push(", ");
                transpile_expr_as_value(r, writer, current_scope, scopes, native_funcs);
                writer.push("))");
            } else {
                match op.as_str() {
                    "and" => {
                        writer.push("({ let __left = ");
                        transpile_expr_as_value(left, writer, current_scope, scopes, native_funcs);
                        writer.push("; if craw_is_truthy(&__left) { ");
                        transpile_expr(r, writer, current_scope, scopes, native_funcs);
                        writer.push(" } else { CallResult::Return(__left) } })");
                        return;
                    }
                    "or" => {
                        writer.push("({ let __left = ");
                        transpile_expr_as_value(left, writer, current_scope, scopes, native_funcs);
                        writer.push(
                            "; if craw_is_truthy(&__left) { CallResult::Return(__left) } else { ",
                        );
                        transpile_expr(r, writer, current_scope, scopes, native_funcs);
                        writer.push(" } })");
                        return;
                    }
                    "is" => {
                        writer.push("CallResult::Return(craw_is(");
                        transpile_expr_as_value(left, writer, current_scope, scopes, native_funcs);
                        writer.push(", ");
                        transpile_expr_as_value(r, writer, current_scope, scopes, native_funcs);
                        writer.push("))");
                        return;
                    }
                    "≈" => writer.push("craw_approx("),
                    _ => {
                        writer.push("craw_call(");
                        writer.push(&escape_ident(&format!("__op_{}", op)));
                        writer.push(", ");
                    }
                }
                writer.push("vec![");
                transpile_expr_as_value(left, writer, current_scope, scopes, native_funcs);
                writer.push(", ");
                transpile_expr_as_value(r, writer, current_scope, scopes, native_funcs);
                writer.push("])");
            }
        }
        Expr::Call(target, args) => {
            if let Expr::Attribute(obj, name) = &**target {
                if let Expr::Ident(ref target_name) = **obj {
                    if target_name == "self" {
                        writer.push(&format!("CallResult::Return(self.{}(", name));
                        for (i, arg) in args.iter().enumerate() {
                            if i > 0 {
                                writer.push(", ");
                            }
                            transpile_expr_as_value(
                                arg,
                                writer,
                                current_scope,
                                scopes,
                                native_funcs,
                            );
                        }
                        writer.push("))");
                        return;
                    } else if let Some(ty) = current_scope.types.get(target_name) {
                        if !matches!(ty, Type::Dynamic) {
                            writer.push(&format!(
                                "CallResult::Return({}.{}(",
                                escape_ident(target_name),
                                name
                            ));
                            for (i, arg) in args.iter().enumerate() {
                                if i > 0 {
                                    writer.push(", ");
                                }
                                transpile_expr_as_value(
                                    arg,
                                    writer,
                                    current_scope,
                                    scopes,
                                    native_funcs,
                                );
                            }
                            writer.push("))");
                            return;
                        }
                    }
                }
                writer.push("craw_ufcs_call(");
                transpile_expr_as_value(obj, writer, current_scope, scopes, native_funcs);
                writer.push(", \"");
                writer.push(name);
                writer.push("\", ");
                transpile_call_args(args, writer, current_scope, scopes, native_funcs);
                writer.push(", ");

                let esc_name = escape_ident(name);
                if current_scope.defined.contains(name) || current_scope.captured.contains(name) {
                    writer.push("Some(");
                    writer.push(&esc_name);
                    writer.push(".clone())");
                } else if BUILTINS.iter().any(|(b, _)| b == name) {
                    writer.push("Some(CrawValue::Builtin(\"");
                    writer.push(name);
                    writer.push("\".to_string()))");
                } else {
                    writer.push("None");
                }
                writer.push(")");
            } else {
                writer.push("craw_call(");
                transpile_expr_as_value(target, writer, current_scope, scopes, native_funcs);
                writer.push(", ");
                transpile_call_args(args, writer, current_scope, scopes, native_funcs);
                writer.push(")");
            }
        }
        Expr::BroadcastCall(target, args) => {
            writer.push("craw_broadcast(");
            transpile_expr_as_value(target, writer, current_scope, scopes, native_funcs);
            writer.push(", ");
            transpile_call_args(args, writer, current_scope, scopes, native_funcs);
            writer.push(")");
        }
        Expr::Lambda(args, body, id) => {
            let info = scopes.get(id).expect("Lambda scope info not found");
            let mut captures: Vec<_> = info.captured.iter().collect();
            captures.sort();

            if !captures.is_empty() {
                writer.push("{ ");
                for cap in &captures {
                    let esc_cap = escape_ident(cap);
                    writer.push("let ");
                    writer.push(&esc_cap);
                    writer.push(" = ");
                    writer.push(&esc_cap);
                    writer.push(".clone(); ");
                }
                writer.push("CallResult::Return(CrawValue::Closure(Rc::new(move |args| { ");
            } else {
                writer.push("CallResult::Return(CrawValue::Closure(Rc::new(|args| { ");
            }

            for (i, a) in args.iter().enumerate() {
                writer.push("let ");
                writer.push(&escape_ident(a));
                writer.push(" = args[");
                writer.push(&i.to_string());
                writer.push("].clone(); ");
            }
            transpile_expr(body, writer, info, scopes, native_funcs);
            writer.push(" } )))");
            if !captures.is_empty() {
                writer.push(" }");
            }
        }
        Expr::Pipe(left, data, right) => {
            match data.style {
                CallStyle::Standard => {
                    if data.none_aware {
                        writer.push("{ let __l = ");
                        transpile_expr_as_value(left, writer, current_scope, scopes, native_funcs);
                        writer.push("; if matches!(&__l, CrawValue::None) { CallResult::Return(__l) } else { ");
                        writer.push("craw_call(");
                        transpile_expr_as_value(right, writer, current_scope, scopes, native_funcs);
                        writer.push(", vec![__l]) } }");
                    } else {
                        writer.push("craw_call(");
                        transpile_expr_as_value(right, writer, current_scope, scopes, native_funcs);
                        writer.push(", vec![");
                        transpile_expr_as_value(left, writer, current_scope, scopes, native_funcs);
                        writer.push("])");
                    }
                }
                CallStyle::Star => {
                    writer.push("{ let __l = ");
                    transpile_expr_as_value(left, writer, current_scope, scopes, native_funcs);
                    writer.push("; let __items = if let CrawValue::List(__items) = &__l { __items.borrow().clone() } else { panic!(\"TypeError: star pipe expects a list\"); }; ");
                    writer.push("craw_call(");
                    transpile_expr_as_value(right, writer, current_scope, scopes, native_funcs);
                    writer.push(", __items) }");
                }
                CallStyle::DoubleStar => {
                    writer.push("{ let __l = ");
                    transpile_expr_as_value(left, writer, current_scope, scopes, native_funcs);
                    writer.push("; let __items = if let CrawValue::Dict(__items) = &__l { __items.borrow().values().cloned().collect::<Vec<_>>() } else { panic!(\"TypeError: double star pipe expects a dict\"); }; ");
                    writer.push("craw_call(");
                    transpile_expr_as_value(right, writer, current_scope, scopes, native_funcs);
                    writer.push(", __items) }");
                }
            }
        }
        Expr::Compose(left, _, right, id) => {
            let info = scopes.get(id).expect("Compose scope info not found");
            let mut captures: Vec<_> = info.captured.iter().collect();
            captures.sort();

            if !captures.is_empty() {
                writer.push("{ ");
                for cap in &captures {
                    let esc_cap = escape_ident(cap);
                    writer.push("let ");
                    writer.push(&esc_cap);
                    writer.push(" = ");
                    writer.push(&esc_cap);
                    writer.push(".clone(); ");
                }
                writer.push("CallResult::Return(CrawValue::Closure( { ");
            } else {
                writer.push("CallResult::Return(CrawValue::Closure( { ");
            }

            writer.push("let __f = ");
            transpile_expr_as_value(left, writer, current_scope, scopes, native_funcs);
            writer.push("; let __g = ");
            transpile_expr_as_value(right, writer, current_scope, scopes, native_funcs);
            writer.push("; Rc::new(move |args| { ");
            writer.push("let __res_g = craw_driver(craw_call(__g.clone(), args)); ");
            writer.push("craw_call(__f.clone(), vec![__res_g])");
            writer.push(" } ) } ))");
            if !captures.is_empty() {
                writer.push(" }");
            }
        }
        Expr::NoneCoalesce(left, right) => {
            writer.push("{ let __l = ");
            transpile_expr_as_value(left, writer, current_scope, scopes, native_funcs);
            writer.push("; if matches!(&__l, CrawValue::None) { ");
            transpile_expr(right, writer, current_scope, scopes, native_funcs);
            writer.push(" } else { CallResult::Return(__l) } }");
        }
        Expr::PartialCall(target, args, id) => {
            let info = scopes.get(id).expect("PartialCall scope info not found");
            let mut captures: Vec<_> = info.captured.iter().collect();
            captures.sort();

            if !captures.is_empty() {
                writer.push("{ ");
                for cap in &captures {
                    let esc_cap = escape_ident(cap);
                    writer.push("let ");
                    writer.push(&esc_cap);
                    writer.push(" = ");
                    writer.push(&esc_cap);
                    writer.push(".clone(); ");
                }
                writer.push("CallResult::Return(CrawValue::Closure( { ");
            } else {
                writer.push("CallResult::Return(CrawValue::Closure( { ");
            }

            writer.push("let __target = ");
            transpile_expr_as_value(target, writer, current_scope, scopes, native_funcs);
            writer.push("; ");
            for (i, arg) in args.iter().enumerate() {
                if let Some(e) = arg {
                    writer.push(&format!("let __arg_{} = ", i));
                    transpile_expr_as_value(e, writer, current_scope, scopes, native_funcs);
                    writer.push("; ");
                }
            }
            writer.push("Rc::new(move |extra_args| { ");
            writer.push("let mut __final_args = vec![]; ");
            writer.push("let mut __extra_idx = 0; ");
            for (i, arg) in args.iter().enumerate() {
                if arg.is_some() {
                    writer.push(&format!("__final_args.push(__arg_{}.clone()); ", i));
                } else {
                    writer.push(
                        "__final_args.push(extra_args[__extra_idx].clone()); __extra_idx += 1; ",
                    );
                }
            }
            writer.push("while __extra_idx < extra_args.len() { __final_args.push(extra_args[__extra_idx].clone()); __extra_idx += 1; } ");
            writer.push("craw_call(__target.clone(), __final_args)");
            writer.push(" } ) } ))");
            if !captures.is_empty() {
                writer.push(" }");
            }
        }
        Expr::Attribute(target, name) => {
            if let Expr::Ident(ref target_name) = **target {
                if target_name == "self" {
                    writer.push(&format!("CallResult::Return(self.{}.clone())", name));
                    return;
                }
            }
            writer.push("craw_get_attr(");
            transpile_expr_as_value(target, writer, current_scope, scopes, native_funcs);
            writer.push(&format!(", \"{}\")", name));
        }
        Expr::AttributePartial(name) => {
            writer.push(&format!("CallResult::Return(CrawValue::Closure(Rc::new(|args| {{ if args.is_empty() {{ panic!(\"Attribute partial {} requires 1 argument\") }} else {{ let m = craw_driver(craw_get_attr(args[0].clone(), \"{}\")); if args.len() == 1 {{ CallResult::Return(m) }} else {{ craw_call(m, args[1..].to_vec()) }} }} }})))", name, name));
        }
        Expr::ImplicitLambda(body, id) => {
            let info = scopes
                .get(id)
                .expect("Implicit lambda scope info not found");
            let mut captures: Vec<_> = info.captured.iter().collect();
            captures.sort();

            if !captures.is_empty() {
                writer.push("{ ");
                for cap in &captures {
                    let esc_cap = escape_ident(cap);
                    writer.push("let ");
                    writer.push(&esc_cap);
                    writer.push(" = ");
                    writer.push(&esc_cap);
                    writer.push(".clone(); ");
                }
                writer.push("CallResult::Return(CrawValue::Closure(Rc::new(move |args| { ");
            } else {
                writer.push("CallResult::Return(CrawValue::Closure(Rc::new(|args| { ");
            }

            writer.push("let __craw_underscore = args[0].clone(); ");
            transpile_expr(body, writer, info, scopes, native_funcs);
            writer.push(" } )))");
            if !captures.is_empty() {
                writer.push(" }");
            }
        }
        Expr::Where(expr, stmts, id) => {
            let info = scopes.get(id).expect("Where scope info not found");
            let mut captures: Vec<_> = info.captured.iter().collect();
            captures.sort();

            if !captures.is_empty() {
                writer.push("{ ");
                for cap in &captures {
                    let esc_cap = escape_ident(cap);
                    writer.push("let ");
                    writer.push(&esc_cap);
                    writer.push(" = ");
                    writer.push(&esc_cap);
                    writer.push(".clone(); ");
                }
                writer.push("{\n");
            } else {
                writer.push("{\n");
            }

            writer.indent();
            let mut inner_defined = HashSet::new();
            for s in stmts {
                transpile_stmt(
                    s,
                    writer,
                    info,
                    &mut inner_defined,
                    scopes,
                    false,
                    false,
                    native_funcs,
                );
            }
            transpile_expr(expr, writer, info, scopes, native_funcs);
            writer.push("\n");
            writer.dedent();
            writer.push_indent();
            writer.push("}");
            if !captures.is_empty() {
                writer.push(" }");
            }
        }
        Expr::Set(elements) => {
            writer.push("CallResult::Return(CrawValue::Set(Rc::new(RefCell::new({ let mut s = HashSet::new(); ");
            for e in elements {
                writer.push("s.insert(");
                transpile_expr_as_value(e, writer, current_scope, scopes, native_funcs);
                writer.push("); ");
            }
            writer.push("s }))))");
        }
        Expr::Frozenset(elements) => {
            writer.push(
                "CallResult::Return(CrawValue::Frozenset(Rc::new({ let mut s = HashSet::new(); ",
            );
            for e in elements {
                writer.push("s.insert(");
                transpile_expr_as_value(e, writer, current_scope, scopes, native_funcs);
                writer.push("); ");
            }
            writer.push("s })))");
        }
        Expr::Multiset(elements) => {
            writer.push("CallResult::Return(CrawValue::Multiset(Rc::new(RefCell::new({ let mut m = HashMap::new(); ");
            for e in elements {
                writer.push("*m.entry(");
                transpile_expr_as_value(e, writer, current_scope, scopes, native_funcs);
                writer.push(").or_insert(0) += 1; ");
            }
            writer.push("m }))))");
        }
        Expr::LazyList(elements, id) => {
            let info = scopes.get(id).expect("LazyList scope info not found");
            let mut captures: Vec<_> = info.captured.iter().collect();
            captures.sort();

            if !captures.is_empty() {
                writer.push("{ ");
                for cap in &captures {
                    let esc_cap = escape_ident(cap);
                    writer.push("let ");
                    writer.push(&esc_cap);
                    writer.push(" = ");
                    writer.push(&esc_cap);
                    writer.push(".clone(); ");
                }
                writer.push("CallResult::Return(CrawValue::LazyList(Rc::new(move || vec![");
            } else {
                writer.push("CallResult::Return(CrawValue::LazyList(Rc::new(move || vec![");
            }

            for (i, e) in elements.iter().enumerate() {
                if i > 0 {
                    writer.push(", ");
                }
                transpile_expr_as_value(e, writer, current_scope, scopes, native_funcs);
            }
            writer.push("])))");
            if !captures.is_empty() {
                writer.push(" }");
            }
        }
        Expr::IndexPartial(target) => {
            writer.push("CallResult::Return(CrawValue::Closure({ let __t = ");
            transpile_expr_as_value(target, writer, current_scope, scopes, native_funcs);
            writer.push(
                "; Rc::new(move |args| { craw_get_item(__t.clone(), args[0].clone()) }) } ))",
            );
        }
        Expr::OperatorFunction(op) => {
            let func = match op.as_str() {
                "+" => "craw_add",
                "-" => "craw_sub",
                "*" => "craw_mul",
                "/" => "craw_div",
                "%" => "craw_mod",
                "==" => "craw_eq",
                "!=" => "craw_ne",
                "<" => "craw_lt",
                "<=" => "craw_le",
                ">" => "craw_gt",
                ">=" => "craw_ge",
                _ => "craw_ident",
            };
            writer.push(&format!(
                "CallResult::Return(CrawValue::Closure(Rc::new(|args| {{ {}(args) }})))",
                func
            ));
        }
        Expr::List(elements) => {
            writer.push("CallResult::Return(CrawValue::List(Rc::new(RefCell::new(vec![");
            for (i, e) in elements.iter().enumerate() {
                if i > 0 {
                    writer.push(", ");
                }
                transpile_expr_as_value(e, writer, current_scope, scopes, native_funcs);
            }
            writer.push("]))))");
        }
        Expr::Dict(pairs) => {
            writer.push("CallResult::Return(CrawValue::Dict(Rc::new(RefCell::new({ let mut m = HashMap::new(); ");
            for (k, v) in pairs {
                writer.push("m.insert(");
                transpile_expr_as_value(k, writer, current_scope, scopes, native_funcs);
                writer.push(", ");
                transpile_expr_as_value(v, writer, current_scope, scopes, native_funcs);
                writer.push("); ");
            }
            writer.push("m }))))");
        }
        Expr::Index(target, index) => {
            writer.push("craw_get_item(");
            transpile_expr_as_value(target, writer, current_scope, scopes, native_funcs);
            writer.push(", ");
            transpile_expr_as_value(index, writer, current_scope, scopes, native_funcs);
            writer.push(")");
        }
        Expr::Passthrough(s) => {
            writer.push("CallResult::Return(CrawValue::from({\n");
            writer.push(s);
            writer.push("\n}))");
        }
        Expr::Ternary(cond, then_expr, else_expr) => {
            writer.push("{ if matches!(&");
            transpile_expr_as_value(cond, writer, current_scope, scopes, native_funcs);
            writer.push(", CrawValue::Bool(true) | CrawValue::Int(1..)) { ");
            transpile_expr(then_expr, writer, current_scope, scopes, native_funcs);
            writer.push(" } else { ");
            transpile_expr(else_expr, writer, current_scope, scopes, native_funcs);
            writer.push(" } }");
        }
        Expr::Tuple(elements) => {
            writer.push("CallResult::Return(CrawValue::Tuple(Rc::new(vec![");
            for (i, e) in elements.iter().enumerate() {
                if i > 0 {
                    writer.push(", ");
                }
                transpile_expr_as_value(e, writer, current_scope, scopes, native_funcs);
            }
            writer.push("])))");
        }
        Expr::Range(start, end) => {
            writer.push("CallResult::Return(craw_to2(");
            transpile_expr_as_value(start, writer, current_scope, scopes, native_funcs);
            writer.push(", ");
            transpile_expr_as_value(end, writer, current_scope, scopes, native_funcs);
            writer.push("))");
        }
        Expr::Gather(coll, field) => {
            writer.push("craw_fmap(vec![CrawValue::Closure(Rc::new(|__args| { ");
            writer.push("let __item = __args[0].clone(); ");
            writer.push("craw_get_attr(__item, ");
            writer.push(&format!("{:?}", field));
            writer.push(")");
            writer.push(" })), ");
            transpile_expr_as_value(coll, writer, current_scope, scopes, native_funcs);
            writer.push("])");
        }
        Expr::Slice(start, stop, step) => {
            writer.push("CallResult::Return(CrawValue::Slice(");
            if let Some(e) = start {
                writer.push("Some((");
                transpile_expr_as_value(e, writer, current_scope, scopes, native_funcs);
                writer.push(").clone().try_into_native::<i64>().unwrap())");
            } else {
                writer.push("None");
            }
            writer.push(", ");
            if let Some(e) = stop {
                writer.push("Some((");
                transpile_expr_as_value(e, writer, current_scope, scopes, native_funcs);
                writer.push(").clone().try_into_native::<i64>().unwrap())");
            } else {
                writer.push("None");
            }
            writer.push(", ");
            if let Some(e) = step {
                writer.push("Some((");
                transpile_expr_as_value(e, writer, current_scope, scopes, native_funcs);
                writer.push(").clone().try_into_native::<i64>().unwrap())");
            } else {
                writer.push("None");
            }
            writer.push("))");
        }
        Expr::Hcat(items) => {
            writer.push("CallResult::Return(craw_hcat(vec![");
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    writer.push(", ");
                }
                transpile_expr_as_value(item, writer, current_scope, scopes, native_funcs);
            }
            writer.push("]))");
        }
        Expr::Vcat(items) => {
            writer.push("CallResult::Return(craw_vcat(vec![");
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    writer.push(", ");
                }
                transpile_expr_as_value(item, writer, current_scope, scopes, native_funcs);
            }
            writer.push("]))");
        }
        Expr::Formula(l, r) => {
            let l_str = expr_to_craw_source(l);
            let r_str = expr_to_craw_source(r);
            writer.push(&format!(
                "CallResult::Return(CrawValue::Formula(\"{}\".to_string(), \"{}\".to_string()))",
                l_str, r_str
            ));
        }
        Expr::Query {
            from,
            in_expr,
            clauses,
            select,
            id,
        } => {
            transpile_query(
                from,
                in_expr,
                clauses,
                select,
                *id,
                writer,
                current_scope,
                scopes,
                native_funcs,
            );
        }
        Expr::Comprehension(expr, pat, iterable, is_lazy, id) => {
            let info = scopes.get(id).expect("Comprehension scope info not found");
            let mut captures: Vec<_> = info.captured.iter().collect();
            captures.sort();

            if *is_lazy {
                writer.push("{ let __iter = ");
                transpile_expr_as_value(iterable, writer, current_scope, scopes, native_funcs);
                writer.push(";\n");

                writer.push("let (v_tx, v_rx) = std::sync::mpsc::channel();\n");
                writer.push("let (c_tx, c_rx) = std::sync::mpsc::channel();\n");

                writer.push("std::thread::spawn({\n");
                writer.push("let __iter_plain = __iter.to_plain();\n");
                for cap in &captures {
                    let esc_cap = escape_ident(cap);
                    writer.push("let ");
                    writer.push(&esc_cap);
                    writer.push("_plain = ");
                    writer.push(&esc_cap);
                    writer.push(".to_plain();\n");
                }

                writer.push("move || {\n");

                writer.push("let __iter = CrawValue::from_plain(__iter_plain);\n");
                for cap in &captures {
                    let esc_cap = escape_ident(cap);
                    writer.push("let ");
                    writer.push(&esc_cap);
                    writer.push(" = CrawValue::from_plain(");
                    writer.push(&esc_cap);
                    writer.push("_plain);\n");
                }

                writer.push("let __items = match &__iter {\n");
                writer.push("    CrawValue::List(lst) => lst.clone(),\n");
                writer.push("    CrawValue::LazyList(f) => Rc::new(RefCell::new(f())),\n");
                writer.push("    CrawValue::String(s) => Rc::new(RefCell::new(s.chars().map(|c| CrawValue::String(Rc::new(c.to_string()))).collect())),\n");
                writer.push("    _ => panic!(\"TypeError: expected iterable in comprehension\")\n");
                writer.push("};\n");

                writer.push("for __item in __items.borrow().iter() {\n");
                let (cond, binds) = build_pattern(
                    pat,
                    "__item",
                    "comp",
                    writer.indent_level,
                    info,
                    scopes,
                    native_funcs,
                );
                writer.push("if ");
                writer.push(&cond);
                writer.push(" {\n");
                writer.push(&binds);

                writer.push("let __res = ");
                transpile_expr_as_value(expr, writer, current_scope, scopes, native_funcs);
                writer.push(";\n");

                writer.push("if c_rx.recv().is_err() { break; }\n");
                writer.push("if v_tx.send(__res.to_plain()).is_err() { break; }\n");

                writer.push("}\n");
                writer.push("}\n");

                writer.push("();\n");

                writer.push("} });\n");

                writer.push("CallResult::Return(CrawValue::Generator(std::sync::Arc::new(std::sync::Mutex::new(v_rx)), std::sync::Arc::new(std::sync::Mutex::new(c_tx))))\n");
                writer.push("}");
            } else {
                writer.push("{ let __iter = ");
                transpile_expr_as_value(iterable, writer, current_scope, scopes, native_funcs);
                writer.push("; ");

                if !captures.is_empty() {
                    writer.push("{ ");
                    for cap in &captures {
                        let esc_cap = escape_ident(cap);
                        writer.push("let ");
                        writer.push(&esc_cap);
                        writer.push(" = ");
                        writer.push(&esc_cap);
                        writer.push(".clone(); ");
                    }
                }

                writer.push("craw_fmap(vec![CrawValue::Closure(Rc::new(move |__args| { ");
                writer.push("let __item = __args[0].clone(); ");

                let (cond, binds) =
                    build_pattern(pat, "__item", "comp", 0, info, scopes, native_funcs);
                writer.push("if ");
                writer.push(&cond);
                writer.push(" { ");
                writer.push(&binds);
                transpile_expr(expr, writer, info, scopes, native_funcs);
                writer.push(" } else { CallResult::Return(CrawValue::None) } ");
                writer.push(" } )), __iter])");

                if !captures.is_empty() {
                    writer.push(" }");
                }
                writer.push(" }");
            }
        }
        Expr::MacroCall(name, args) => {
            if name == "." {
                let transformed = broadcast_everything(args[0].clone());
                transpile_expr(&transformed, writer, current_scope, scopes, native_funcs);
            } else {
                let esc_name = escape_ident(name);
                writer.push(&format!("{}!(", esc_name));
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        writer.push(", ");
                    }
                    transpile_expr_as_value(arg, writer, current_scope, scopes, native_funcs);
                }
                writer.push(")");
            }
        }
        Expr::Splat(inner) => {
            writer.push("/* splat handled by call */ ");
            transpile_expr(inner, writer, current_scope, scopes, native_funcs);
        }
        Expr::Placeholder => panic!("Placeholder should have been handled by parser"),
    }
}

fn indent(level: usize) -> String {
    "    ".repeat(level)
}

fn build_pattern(
    pat: &Pattern,
    target: &str,
    path: &str,
    level: usize,
    current_scope: &ScopeInfo,
    scopes: &HashMap<usize, ScopeInfo>,
    native_funcs: &HashSet<String>,
) -> (String, String) {
    match pat {
        Pattern::Wildcard => ("true".to_string(), String::new()),
        Pattern::Var(name, _) => {
            let binds = format!(
                "{}let mut {} = {}.clone();\n",
                indent(level),
                escape_ident(name),
                target
            );
            ("true".to_string(), binds)
        }
        Pattern::Data(name, fields) => {
            let mut has_rest = false;
            let mut rest_idx = 0;
            for (i, f) in fields.iter().enumerate() {
                if let Pattern::Rest(_) = f {
                    has_rest = true;
                    rest_idx = i;
                    break;
                }
            }

            let fields_len = fields.len();

            let cond = if name == "List" {
                if has_rest {
                    format!(
                        "{{ if let CrawValue::List(__d_fields_{p}) = &{t} {{ __d_fields_{p}.borrow().len() >= {len} }} else {{ false }} }}",
                        p = path,
                        t = target,
                        len = fields_len - 1
                    )
                } else {
                    format!(
                        "{{ if let CrawValue::List(__d_fields_{p}) = &{t} {{ __d_fields_{p}.borrow().len() == {len} }} else {{ false }} }}",
                        p = path,
                        t = target,
                        len = fields_len
                    )
                }
            } else {
                if has_rest {
                    format!(
                        "{{ if let CrawValue::Data(__d_name_{p}, _, __d_fields_{p}) = &{t} {{ __d_name_{p} == \"{n}\" && __d_fields_{p}.borrow().len() >= {len} }} else {{ false }} }}",
                        p = path,
                        t = target,
                        n = name,
                        len = fields_len - 1
                    )
                } else {
                    format!(
                        "{{ if let CrawValue::Data(__d_name_{p}, _, __d_fields_{p}) = &{t} {{ __d_name_{p} == \"{n}\" && __d_fields_{p}.borrow().len() == {len} }} else {{ false }} }}",
                        p = path,
                        t = target,
                        n = name,
                        len = fields_len
                    )
                }
            };

            let mut binds = String::new();
            binds.push_str(&format!(
                "{}let __d_fields_{p} = match &{t} {{ CrawValue::Data(_, _, f) => f.borrow().clone(), CrawValue::List(f) => f.borrow().clone(), _ => vec![] }};\n",
                indent(level), p = path, t = target
            ));

            let mut nested_conds = Vec::new();
            for (i, sub_pat) in fields.iter().enumerate() {
                if has_rest && i == rest_idx {
                    if let Pattern::Rest(var_name) = sub_pat {
                        let binds_str = format!(
                            "{}let mut {} = CrawValue::List(Rc::new(RefCell::new(__d_fields_{p}[{idx}..__d_fields_{p}.len() - ({len} - 1 - {idx})].to_vec())));\n",
                            indent(level),
                            escape_ident(var_name),
                            p = path,
                            idx = i,
                            len = fields_len
                        );
                        binds.push_str(&binds_str);
                    }
                    continue;
                }

                let actual_idx = if has_rest && i > rest_idx {
                    format!(
                        "__d_fields_{p}.len() - ({len} - {idx})",
                        p = path,
                        len = fields_len,
                        idx = i
                    )
                } else {
                    i.to_string()
                };

                let sub_target = format!("__d_fields_{p}[{idx}]", p = path, idx = actual_idx);
                let sub_path = format!("{p}_{i}", p = path, i = i);
                let (sub_cond, sub_binds) = build_pattern(
                    sub_pat,
                    &sub_target,
                    &sub_path,
                    level,
                    current_scope,
                    scopes,
                    native_funcs,
                );
                if sub_cond != "true" {
                    nested_conds.push(sub_cond);
                }
                binds.push_str(&sub_binds);
            }

            let final_cond = if nested_conds.is_empty() {
                cond
            } else {
                format!(
                    "{} && {{ let __d_fields_{p} = match &{t} {{ CrawValue::Data(_, _, f) => f.borrow().clone(), CrawValue::List(f) => f.borrow().clone(), _ => vec![] }}; {} }}",
                    cond,
                    nested_conds.join(" && "),
                    p = path,
                    t = target
                )
            };

            (final_cond, binds)
        }
        Pattern::Tuple(fields) => {
            let mut has_rest = false;
            let mut rest_idx = 0;
            for (i, f) in fields.iter().enumerate() {
                if let Pattern::Rest(_) = f {
                    has_rest = true;
                    rest_idx = i;
                    break;
                }
            }

            let fields_len = fields.len();
            let cond = if has_rest {
                format!(
                    "{{ if let CrawValue::Tuple(__d_fields_{p}) = &{t} {{ __d_fields_{p}.len() >= {len} }} else {{ false }} }}",
                    p = path,
                    t = target,
                    len = fields_len - 1
                )
            } else {
                format!(
                    "{{ if let CrawValue::Tuple(__d_fields_{p}) = &{t} {{ __d_fields_{p}.len() == {len} }} else {{ false }} }}",
                    p = path,
                    t = target,
                    len = fields_len
                )
            };

            let mut binds = String::new();
            binds.push_str(&format!(
                "{}let __d_fields_{p} = match &{t} {{ CrawValue::Tuple(f) => f.as_ref().clone(), _ => vec![] }};\n",
                indent(level), p = path, t = target
            ));

            let mut nested_conds = Vec::new();
            for (i, sub_pat) in fields.iter().enumerate() {
                if has_rest && i == rest_idx {
                    if let Pattern::Rest(var_name) = sub_pat {
                        let binds_str = format!(
                            "{}let mut {} = CrawValue::Tuple(Rc::new(__d_fields_{p}[{idx}..__d_fields_{p}.len() - ({len} - 1 - {idx})].to_vec()));\n",
                            indent(level),
                            escape_ident(var_name),
                            p = path,
                            idx = i,
                            len = fields_len
                        );
                        binds.push_str(&binds_str);
                    }
                    continue;
                }

                let actual_idx = if has_rest && i > rest_idx {
                    format!(
                        "__d_fields_{p}.len() - ({len} - {idx})",
                        p = path,
                        len = fields_len,
                        idx = i
                    )
                } else {
                    i.to_string()
                };

                let sub_target = format!("__d_fields_{p}[{idx}]", p = path, idx = actual_idx);
                let sub_path = format!("{p}_{i}", p = path, i = i);
                let (sub_cond, sub_binds) = build_pattern(
                    sub_pat,
                    &sub_target,
                    &sub_path,
                    level,
                    current_scope,
                    scopes,
                    native_funcs,
                );
                if sub_cond != "true" {
                    nested_conds.push(sub_cond);
                }
                binds.push_str(&sub_binds);
            }

            let final_cond = if nested_conds.is_empty() {
                cond
            } else {
                format!(
                    "{} && {{ let __d_fields_{p} = match &{t} {{ CrawValue::Tuple(f) => f.as_ref().clone(), _ => vec![] }}; {} }}",
                    cond,
                    nested_conds.join(" && "),
                    p = path,
                    t = target
                )
            };

            (final_cond, binds)
        }
        Pattern::Const(e) => {
            let cond = match e {
                Expr::Number(i) => format!(
                    "{{ if let CrawValue::Int(v) = &{} {{ *v == {} }} else {{ false }} }}",
                    target, i
                ),
                Expr::String(s) => format!(
                    "{{ if let CrawValue::String(v) = &{} {{ v == \"{}\" }} else {{ false }} }}",
                    target, s
                ),
                _ => "false".to_string(),
            };
            (cond, String::new())
        }
        Pattern::View(f, sub_pat) => {
            let mut f_writer = CodeWriter::new(64);
            transpile_expr_as_value(f, &mut f_writer, current_scope, scopes, native_funcs);
            let f_val = f_writer.finish();
            let sub_path = format!("{}_v", path);
            let (sub_cond, sub_binds) = build_pattern(
                sub_pat,
                &format!("__view_res_{}", path),
                &sub_path,
                level,
                current_scope,
                scopes,
                native_funcs,
            );

            let cond = format!(
                "{{ let __view_res_{p} = craw_driver(craw_call({f}, vec![{t}.clone()])); let __res = {sub_cond}; __res }}",
                p = path,
                f = f_val,
                t = target,
                sub_cond = sub_cond
            );

            let mut binds = format!(
                "{}let __view_res_{p} = craw_driver(craw_call({f}, vec![{t}.clone()]));\n",
                indent(level),
                p = path,
                f = f_val,
                t = target
            );
            binds.push_str(&sub_binds);

            (cond, binds)
        }
        Pattern::StringSplit(pat_str, var_name, is_prefix) => {
            let cond = format!(
                "{{ if let CrawValue::String(__s_{p}) = &{t} {{ __s_{p}.{method}(\"{pat}\") }} else {{ false }} }}",
                p = path,
                t = target,
                method = if *is_prefix {
                    "starts_with"
                } else {
                    "ends_with"
                },
                pat = pat_str
            );

            let mut bind = format!(
                "{}let mut {} = CrawValue::String(Rc::new(if let CrawValue::String(__s_{p}) = &{t} {{ ",
                indent(level),
                escape_ident(var_name),
                p = path,
                t = target
            );
            if *is_prefix {
                bind.push_str(&format!(
                    "__s_{p}[\"{pat}\".len()..].to_string()",
                    p = path,
                    pat = pat_str
                ));
            } else {
                bind.push_str(&format!(
                    "__s_{p}[..__s_{p}.len() - \"{pat}\".len()].to_string()",
                    p = path,
                    pat = pat_str
                ));
            }
            bind.push_str(" } else { String::new() }));\n");

            (cond, bind)
        }
        Pattern::Rest(_) => ("true".to_string(), String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transpile_macro_def() {
        use crate::lexer::Lexer;
        use crate::parser::parse;
        let input = "
macro myadd(x, y):
    x + y

@myadd(1, 2)
";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();
        let stmts = parse(&tokens).unwrap();
        let output = transpile(&stmts);
        println!("{}", output);

        assert!(output.contains("macro_rules! myadd {"));
        assert!(output.contains("($x:expr, $y:expr) => {{"));
        assert!(output.contains("craw_add2($x, $y)"));
        assert!(output.contains("myadd!(CrawValue::Int(1), CrawValue::Int(2))"));
    }

    #[test]
    fn test_top_level_macro_block_expansion() {
        // Regression test: a top-level (not nested inside a function) macro-block
        // invocation must actually be expanded by `TemplateExpander::expand`, not
        // merely recursed into and left as an unresolved `MacroBlock`.
        use crate::lexer::Lexer;
        use crate::parser::parse;
        let input = "
template repeat(count, body):
    i = 0
    while i < count:
        body
        i = i + 1

repeat 3:
    print(\"hello\")
";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();
        let stmts = parse(&tokens).unwrap();
        let output = transpile(&stmts);
        println!("{}", output);

        assert!(!output.contains("Unexpanded MacroBlock"));
        assert!(output.contains("while"));
    }

    #[test]
    fn test_macro_block_with_named_branch_expansion() {
        use crate::lexer::Lexer;
        use crate::parser::parse;
        let input = "
template when(cond, then_body, branch_kw, else_body):
    matched = cond
    if matched:
        then_body
    if not matched:
        else_body

when 1 > 0:
    print(\"positive\")
otherwise:
    print(\"non-positive\")
";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();
        let stmts = parse(&tokens).unwrap();
        let output = transpile(&stmts);
        println!("{}", output);

        assert!(!output.contains("Unexpanded MacroBlock"));
        assert!(output.contains("positive"));
        assert!(output.contains("non-positive"));
    }

    #[test]
    #[should_panic(expected = "Macro 'repeat' arity mismatch")]
    fn test_macro_block_arity_mismatch_panics_with_macro_name() {
        use crate::lexer::Lexer;
        use crate::parser::parse;
        let input = "
template repeat(count, body, extra):
    body

repeat 3:
    print(\"hello\")
";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();
        let stmts = parse(&tokens).unwrap();
        transpile(&stmts);
    }

    #[test]
    fn test_typed_assign_lowering() {
        let stmt = Stmt::Assign(Pattern::Var("x".into(), Some(Type::Int)), Expr::Number(42));
        let code = transpile(&[stmt]);
        println!("CODE: {}", code);
        assert!(code.contains(
            "let mut x: i64 = (CrawValue::Int(42)).clone().try_into_native::<i64>().unwrap();"
        ));
    }

    #[test]
    fn test_transpile_trait_impl() {
        let trait_def = Stmt::TraitDef(
            "Drawable".into(),
            vec![Stmt::FunctionDef {
                name: vec!["draw".into()],
                args: vec![(Pattern::Var("self".into(), None), None)],
                vararg: None,
                return_type: Some(Type::Custom("()".into())),
                body: vec![],
                is_copyclosure: false,
                is_addpattern: false,
                is_generator: false,
                id: 1,
            }],
            0,
        );
        let impl_block = Stmt::ImplBlock(
            Some(Type::Custom("Drawable".into())),
            Type::Custom("Point".into()),
            vec![Stmt::FunctionDef {
                name: vec!["draw".into()],
                args: vec![(Pattern::Var("self".into(), None), None)],
                vararg: None,
                return_type: Some(Type::Custom("()".into())),
                body: vec![Stmt::Expr(Expr::Number(0))],
                is_copyclosure: false,
                is_addpattern: false,
                is_generator: false,
                id: 2,
            }],
            2,
        );
        let code = transpile(&[trait_def, impl_block]);
        println!("CODE: {}", code);
        assert!(code.contains("pub trait Drawable {"));
        println!("CODE: {}", code);
        assert!(code.contains("fn draw(&self) -> ();"));
        println!("CODE: {}", code);
        assert!(code.contains("impl Drawable for Point {"));
        println!("CODE: {}", code);
        assert!(code.contains("fn draw(&self) -> () {"));
    }

    #[test]
    fn test_transpile_struct_def() {
        let stmts = vec![Stmt::StructDef(
            "Point".into(),
            vec![("x".into(), Type::Int), ("y".into(), Type::Int)],
            0,
        )];
        let code = transpile(&stmts);
        assert!(
            code.contains("#[derive(Clone, Debug)] pub struct Point { pub x: i64, pub y: i64 }")
        );
    }

    #[test]
    fn test_transpile_native_import() {
        let stmts = vec![Stmt::NativeImport(
            vec!["std".into(), "collections".into()],
            vec!["HashMap".into(), "HashSet".into()],
        )];
        let code = transpile(&stmts);
        println!("CODE: {}", code);
        assert!(code.contains("use std::collections::{HashMap, HashSet};"));
    }

    #[test]
    fn test_type_to_rust() {
        assert_eq!(type_to_rust(&Type::Int), "i64");
        assert_eq!(type_to_rust(&Type::String), "String");
        assert_eq!(
            type_to_rust(&Type::Custom("MyStruct".to_string())),
            "MyStruct"
        );
        assert_eq!(
            type_to_rust(&Type::Generic("Vec".to_string(), vec![Type::Int])),
            "Vec<i64>"
        );
    }

    #[test]
    fn test_transpile_builtins_injection() {
        let stmts = vec![Stmt::Expr(Expr::Ident("fmap".to_string()))];
        let code = transpile(&stmts);
        println!("CODE: {}", code);
        assert!(code.contains("CrawValue::Builtin(\"fmap\".to_string());"));
    }

    #[test]
    fn test_transpile_assign() {
        let stmts = vec![Stmt::Assign(
            Pattern::Var("a".to_string(), None),
            Expr::Number(1),
        )];
        let code = transpile(&stmts);
        println!("CODE: {}", code);
        assert!(code.contains("let mut a = CrawValue::Int(1);"));
        println!("CODE: {}", code);
        assert!(code.contains("pub fn craw_main() {"));
    }

    #[test]
    fn test_transpile_function_def() {
        let stmts = vec![Stmt::FunctionDef {
            name: vec!["add".to_string()],
            args: vec![
                (Pattern::Var("a".to_string(), None), None),
                (Pattern::Var("b".to_string(), None), None),
            ],
            vararg: None,
            return_type: None,
            body: vec![Stmt::Return(Expr::BinaryOp(
                Box::new(Expr::Ident("a".to_string())),
                "+".to_string(),
                Box::new(Expr::Ident("b".to_string())),
            ))],
            is_copyclosure: false,
            is_addpattern: false,
            is_generator: false,
            id: 0,
        }];
        let code = transpile(&stmts);
        println!("Generated code: {}", code);
        assert!(
            code.contains("add = CrawValue::Closure(Rc::new(move |args| {"),
            "Missing base variable for FunctionDef"
        );
    }

    #[test]
    fn test_transpile_formula() {
        let stmts = vec![Stmt::Expr(Expr::Formula(
            Box::new(Expr::Ident("y".to_string())),
            Box::new(Expr::BinaryOp(
                Box::new(Expr::Ident("x1".to_string())),
                "+".to_string(),
                Box::new(Expr::Ident("x2".to_string())),
            )),
        ))];
        let code = transpile(&stmts);
        println!("CODE: {}", code);
        assert!(code.contains("CrawValue::Formula(\"y\".to_string(), \"(x1 + x2)\".to_string())"));
    }

    #[test]
    fn test_transpile_query() {
        let query = Expr::Query {
            from: "p".to_string(),
            in_expr: Box::new(Expr::Ident("players".to_string())),
            clauses: vec![QueryClause::Where(Expr::BinaryOp(
                Box::new(Expr::Attribute(
                    Box::new(Expr::Ident("p".to_string())),
                    "score".to_string(),
                )),
                ">".to_string(),
                Box::new(Expr::Number(100)),
            ))],
            select: Box::new(Expr::Attribute(
                Box::new(Expr::Ident("p".to_string())),
                "name".to_string(),
            )),
            id: 42,
        };
        let stmts = vec![Stmt::Expr(query)];
        let code = transpile(&stmts);
        println!("CODE: {}", code);
        assert!(code.contains("craw_ufcs_call(__q_res.clone(), \"filter\""));
        assert!(code.contains("craw_ufcs_call(__q_res.clone(), \"fmap\""));
        assert!(code.contains("let p = args[0].clone();"));
    }
}

fn transpile_expr_native(
    expr: &Expr,
    writer: &mut CodeWriter,
    current_scope: &ScopeInfo,
    scopes: &HashMap<usize, ScopeInfo>,
    native_funcs: &HashSet<String>,
) {
    match expr {
        Expr::Number(n) => writer.push(&format!("{}i64", n)),
        Expr::String(s) => writer.push(&format!("{:?}.to_string()", s)),
        Expr::FString(exprs) => {
            let mut format_str = String::new();
            for e in exprs {
                match e {
                    Expr::String(s) => format_str.push_str(&s.replace("{", "{{").replace("}", "}}")),
                    _ => format_str.push_str("{}"),
                }
            }
            writer.push(&format!("format!({:?}", format_str));
            for e in exprs {
                if !matches!(e, Expr::String(_)) {
                    writer.push(", ");
                    transpile_expr_native(e, writer, current_scope, scopes, native_funcs);
                }
            }
            writer.push(")");
        }
        Expr::Shell(cmd) => writer.push(&format!("String::from_utf8_lossy(&std::process::Command::new(\"sh\").arg(\"-c\").arg({:?}).output().expect(\"Command failed\").stdout).into_owned()", cmd)),
        Expr::Bool(b) => writer.push(if *b { "true" } else { "false" }),
        Expr::Ident(name) => {
            if name == "True" { writer.push("true"); }
            else if name == "False" { writer.push("false"); }
            else { writer.push(&escape_ident(name)); }
        },
        Expr::BinaryOp(l, op, r) => {
            writer.push("(");
            transpile_expr_native(l, writer, current_scope, scopes, native_funcs);
            let op_str = match op.as_str() {
                "==" => " == ".to_string(),
                "!=" | "≠" => " != ".to_string(),
                "<" => " < ".to_string(),
                "<=" | "≤" => " <= ".to_string(),
                ">" => " > ".to_string(),
                ">=" | "≥" => " >= ".to_string(),
                "and" => " && ".to_string(),
                "or" => " || ".to_string(),
                "%" => " % ".to_string(),
                _ => format!(" {} ", op),
            };
            writer.push(&op_str);
            transpile_expr_native(r, writer, current_scope, scopes, native_funcs);
            writer.push(")");
        }
        Expr::Index(target, index) => {
            transpile_expr_native(target, writer, current_scope, scopes, native_funcs);
            writer.push("[(");
            transpile_expr_native(index, writer, current_scope, scopes, native_funcs);
            writer.push(") as usize]");
        }
        Expr::List(elements) => {
            writer.push("vec![");
            for (i, e) in elements.iter().enumerate() {
                if i > 0 { writer.push(", "); }
                transpile_expr_native(e, writer, current_scope, scopes, native_funcs);
            }
            writer.push("]");
        }
        Expr::Call(target, args) => {
            if let Expr::Attribute(obj, name) = &**target {
                if name == "append" {
                    transpile_expr_native(obj, writer, current_scope, scopes, native_funcs);
                    writer.push(".push(");
                    transpile_expr_native(&args[0], writer, current_scope, scopes, native_funcs);
                    writer.push(")");
                } else if name == "len" {
                    transpile_expr_native(obj, writer, current_scope, scopes, native_funcs);
                    writer.push(".len() as i64");
                } else {
                    panic!("Unsupported native method: {}", name);
                }
            } else if let Expr::Ident(name) = &**target {
                if native_funcs.contains(name) {
                    writer.push(&format!("__native_{}(", escape_ident(name)));
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 { writer.push(", "); }
                        transpile_expr_native(arg, writer, current_scope, scopes, native_funcs);
                    }
                    writer.push(")");
                } else if name == "print" || name == "println" {
                    writer.push("println!(\"{:?}\", vec![");
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 { writer.push(", "); }
                        writer.push("format!(\"{:?}\", ");
                        transpile_expr_native(arg, writer, current_scope, scopes, native_funcs);
                        writer.push(")");
                    }
                    writer.push("].join(\" \"))");
                } else {
                    panic!("Cannot call non-native function {} in native context", name);
                }
            } else {
                panic!("Unsupported native call");
            }
        }
        Expr::Formula(_, _) => panic!("Formula not supported in native context. Use standard mode for statistical models."),
        Expr::Query { .. } => panic!("Query (LINQ) not supported in native context. Use standard mode for queries."),
        _ => panic!("Unsupported native expression: {:?}", expr),
    }
}

fn transpile_stmt_native(
    stmt: &Stmt,
    writer: &mut CodeWriter,
    current_scope: &ScopeInfo,
    defined: &mut HashSet<String>,
    scopes: &HashMap<usize, ScopeInfo>,
    native_funcs: &HashSet<String>,
) {
    match stmt {
        Stmt::Assign(pat, expr) => {
            if let Pattern::Var(name, _) = pat {
                let esc_name = escape_ident(name);
                writer.push_indent();
                if defined.contains(name) {
                    writer.push(&format!("{} = ", esc_name));
                    transpile_expr_native(expr, writer, current_scope, scopes, native_funcs);
                    writer.push(";\n");
                } else {
                    writer.push(&format!("let mut {} = ", esc_name));
                    transpile_expr_native(expr, writer, current_scope, scopes, native_funcs);
                    writer.push(";\n");
                    defined.insert(name.clone());
                }
            } else {
                writer.push_indent();
                writer.push("let __assign_tmp = CrawValue::from(");
                transpile_expr_native(expr, writer, current_scope, scopes, native_funcs);
                writer.push(");\n");

                let (cond, binds) = build_pattern(
                    pat,
                    "__assign_tmp",
                    "assign",
                    writer.indent_level,
                    current_scope,
                    scopes,
                    native_funcs,
                );
                writer.push_indent();
                writer.push("if !");
                writer.push(&cond);
                writer.push(" { panic!(\"TypeError: assignment failed to match pattern\"); }\n");
                writer.push(&binds);

                let mut vars = HashSet::new();
                let mut types = HashMap::new();
                Analyzer::collect_pattern_vars(pat, &mut vars, &mut types);
                defined.extend(vars);
            }
        }
        Stmt::IndexAssign(target, index, value) => {
            writer.push_indent();
            transpile_expr_native(target, writer, current_scope, scopes, native_funcs);
            writer.push("[(");
            transpile_expr_native(index, writer, current_scope, scopes, native_funcs);
            writer.push(") as usize] = ");
            transpile_expr_native(value, writer, current_scope, scopes, native_funcs);
            writer.push(";\n");
        }
        Stmt::AttributeAssign(target, attr, value) => {
            writer.push_indent();
            transpile_expr_native(target, writer, current_scope, scopes, native_funcs);
            writer.push(".");
            writer.push(attr);
            writer.push(" = ");
            transpile_expr_native(value, writer, current_scope, scopes, native_funcs);
            writer.push(";\n");
        }
        Stmt::If(cond, body) => {
            writer.push_indent();
            writer.push("if ");
            transpile_expr_native(cond, writer, current_scope, scopes, native_funcs);
            writer.push(" {\n");
            writer.indent();
            transpile_block_native(body, writer, current_scope, defined, scopes, native_funcs);
            writer.dedent();
            writer.push_line("}");
        }
        Stmt::While(cond, body) => {
            writer.push_indent();
            writer.push("while ");
            transpile_expr_native(cond, writer, current_scope, scopes, native_funcs);
            writer.push(" {\n");
            writer.indent();
            transpile_block_native(body, writer, current_scope, defined, scopes, native_funcs);
            writer.dedent();
            writer.push_line("}");
        }
        Stmt::Return(expr) => {
            writer.push_indent();
            writer.push("return ");
            transpile_expr_native(expr, writer, current_scope, scopes, native_funcs);
            writer.push(";\n");
        }
        Stmt::Expr(expr) => {
            writer.push_indent();
            transpile_expr_native(expr, writer, current_scope, scopes, native_funcs);
            writer.push(";\n");
        }
        _ => {}
    }
}

fn transpile_block_native(
    stmts: &[Stmt],
    writer: &mut CodeWriter,
    current_scope: &ScopeInfo,
    defined: &mut HashSet<String>,
    scopes: &HashMap<usize, ScopeInfo>,
    native_funcs: &HashSet<String>,
) {
    for stmt in stmts {
        transpile_stmt_native(stmt, writer, current_scope, defined, scopes, native_funcs);
    }
}
