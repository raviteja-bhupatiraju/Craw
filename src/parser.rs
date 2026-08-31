use crate::ast::*;
use crate::lexer::Token;
use chumsky::prelude::*;

pub type Span = SimpleSpan<usize>;
pub type Error<'a> = extra::Err<Rich<'a, Token, Span>>;

pub fn type_parser<'a>() -> impl Parser<'a, &'a [Token], Type, Error<'a>> + Clone {
    recursive(|type_p| {
        let ident = select! { Token::Ident(name) => name };
        let generic = ident
            .then(
                type_p
                    .separated_by(just(Token::Comma))
                    .allow_trailing()
                    .collect::<Vec<_>>()
                    .delimited_by(just(Token::LessThan), just(Token::GreaterThan)),
            )
            .map(|(name, args)| Type::Generic(name, args));
        let basic = ident.map(|name| match name.as_str() {
            "i32" | "Int" => Type::Int,
            "str" | "String" => Type::String,
            "Dynamic" => Type::Dynamic,
            _ => Type::Custom(name),
        });
        generic.or(basic)
    })
}

fn is_generator(body: &[Stmt]) -> bool {
    for stmt in body {
        match stmt {
            Stmt::Yield(_) => return true,
            Stmt::If(_, b) if is_generator(b) => {
                return true;
            }
            Stmt::While(_, b) if is_generator(b) => {
                return true;
            }
            Stmt::Match(_, cases) => {
                for (_, _, b) in cases {
                    if is_generator(b) {
                        return true;
                    }
                }
            }
            Stmt::MatchFor(_, _, b) if is_generator(b) => {
                return true;
            }
            _ => {}
        }
    }
    false
}

fn expr_parser_internal<'a, S, E>(
    stmt: S,
    expr_full: E,
) -> impl Parser<'a, &'a [Token], Expr, Error<'a>> + Clone
where
    S: Parser<'a, &'a [Token], Stmt, Error<'a>> + Clone + 'a,
    E: Parser<'a, &'a [Token], Expr, Error<'a>> + Clone + 'a,
{
    recursive(|expr| {
        let ident = select! { Token::Ident(name) => name };
        let pattern = pattern_parser_internal(expr.clone());

        let passthrough_expr = just(Token::Rust)
            .ignore_then(just(Token::Colon).or_not())
            .ignore_then(just(Token::Newline).or_not())
            .ignore_then(just(Token::Indent).or_not())
            .ignore_then(select! { Token::String(s) => s })
            .then_ignore(just(Token::PassthroughStart))
            .then_ignore(just(Token::Dedent).or_not())
            .then_ignore(just(Token::Newline).or_not())
            .map(Expr::Passthrough)
            .or(just(Token::Rust)
                .ignore_then(just(Token::Colon).or_not())
                .ignore_then(just(Token::Newline).or_not())
                .ignore_then(just(Token::Indent).or_not())
                .ignore_then(select! { Token::String(s) => s })
                .then_ignore(just(Token::PassthroughStart))
                .then_ignore(just(Token::Dedent).or_not())
                .map(Expr::Passthrough))
            .or(select! { Token::String(s) => s }
                .then_ignore(just(Token::PassthroughStart))
                .map(Expr::Passthrough))
            .boxed();

        let comprehension = expr
            .clone()
            .then_ignore(just(Token::For))
            .then(pattern.clone())
            .then_ignore(just(Token::In))
            .then(expr.clone())
            .map_with(|((e, p), i), extra| {
                Expr::Comprehension(
                    Box::new(e),
                    Box::new(p),
                    Box::new(i),
                    false,
                    extra.span().start,
                )
            });

        let primary = choice((
            select! { Token::Number(n) => Expr::Number(n) },
            select! { Token::Float(f) => Expr::Float(f) },
            passthrough_expr,
            select! { Token::String(s) => Expr::String(s) },
            select! { Token::FStringRaw(s) => Expr::FString(parse_fstring(&s)) },
            select! { Token::BacktickString(s) => Expr::Shell(s) },
            just(Token::Ident("_".to_string())).to(Expr::Placeholder),
            just(Token::Question).to(Expr::Placeholder),
            just(Token::Dot)
                .ignore_then(ident)
                .map(Expr::AttributePartial),
            just(Token::Dot).to(Expr::Placeholder),
            just(Token::Partial)
                .ignore_then(
                    expr.clone()
                        .delimited_by(just(Token::LBracket), just(Token::RBracket)),
                )
                .map(|e| Expr::IndexPartial(Box::new(e))),
            select! { Token::Operator(op) => Expr::OperatorFunction(op) },
            just(Token::Ident("s".to_string()))
                .ignore_then(
                    expr.clone()
                        .separated_by(just(Token::Comma))
                        .allow_trailing()
                        .collect()
                        .delimited_by(just(Token::LBrace), just(Token::RBrace)),
                )
                .map(Expr::Set),
            just(Token::Ident("f".to_string()))
                .ignore_then(
                    expr.clone()
                        .separated_by(just(Token::Comma))
                        .allow_trailing()
                        .collect()
                        .delimited_by(just(Token::LBrace), just(Token::RBrace)),
                )
                .map(Expr::Frozenset),
            just(Token::Ident("m".to_string()))
                .ignore_then(
                    expr.clone()
                        .separated_by(just(Token::Comma))
                        .allow_trailing()
                        .collect()
                        .delimited_by(just(Token::LBrace), just(Token::RBrace)),
                )
                .map(Expr::Multiset),
            expr.clone()
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .collect()
                .delimited_by(
                    just(Token::LParen).then(just(Token::Pipe)),
                    just(Token::Pipe).then(just(Token::RParen)),
                )
                .map_with(|items, extra| {
                    let span: Span = extra.span();
                    Expr::LazyList(items, span.start)
                }),
            ident.map(|name| match name.as_str() {
                "None" => Expr::None,
                "True" => Expr::Bool(true),
                "False" => Expr::Bool(false),
                _ => Expr::Ident(name),
            }),
            choice((
                comprehension.clone().map(|comp| match comp {
                    Expr::Comprehension(e, p, i, _, id) => Expr::Comprehension(e, p, i, true, id),
                    _ => unreachable!(),
                }),
                expr_full.clone(),
            ))
            .delimited_by(just(Token::LParen), just(Token::RParen)),
            choice((
                comprehension.delimited_by(just(Token::LBracket), just(Token::RBracket)),
                // Array literal `[1, 2; 3, 4]` or List `[1, 2, 3]`
                expr.clone()
                    .separated_by(just(Token::Comma))
                    .allow_trailing()
                    .collect::<Vec<_>>()
                    .map(|items| {
                        if items.len() == 1 {
                            items.into_iter().next().unwrap()
                        } else {
                            Expr::Hcat(items)
                        }
                    })
                    .separated_by(just(Token::Semicolon))
                    .allow_trailing()
                    .collect::<Vec<_>>()
                    .map(|rows| {
                        if rows.len() == 1 {
                            match &rows[0] {
                                Expr::Hcat(items) => Expr::List(items.clone()),
                                item => Expr::List(vec![item.clone()]),
                            }
                        } else {
                            Expr::Vcat(rows)
                        }
                    })
                    .delimited_by(just(Token::LBracket), just(Token::RBracket)),
            )),
            choice((
                // Empty dict {}
                just(Token::LBrace)
                    .then(just(Token::RBrace))
                    .to(Expr::Dict(vec![])),
                // Dict {k: v, ...}
                expr.clone()
                    .then_ignore(just(Token::Colon))
                    .then(expr.clone())
                    .separated_by(just(Token::Comma))
                    .allow_trailing()
                    .collect()
                    .delimited_by(just(Token::LBrace), just(Token::RBrace))
                    .map(Expr::Dict),
                // Set {e, ...}
                expr.clone()
                    .separated_by(just(Token::Comma))
                    .allow_trailing()
                    .at_least(1)
                    .collect()
                    .delimited_by(just(Token::LBrace), just(Token::RBrace))
                    .map(Expr::Set),
            )),
        ))
        .boxed();

        let slice = expr
            .clone()
            .or_not()
            .then_ignore(just(Token::Colon))
            .then(expr.clone().or_not())
            .then(just(Token::Colon).ignore_then(expr.clone()).or_not())
            .map(|((start, stop), step)| {
                Expr::Slice(start.map(Box::new), stop.map(Box::new), step.map(Box::new))
            })
            .boxed();

        enum Postfix {
            Call(Vec<Expr>),
            BroadcastCall(Vec<Expr>),
            Index(Expr),
            Attribute(String),
            Gather(String),
            PartialCall(Vec<Option<Expr>>, usize),
        }

        let call_arg = choice((
            ident
                .then_ignore(just(Token::Assign))
                .then(expr.clone().or_not())
                .map(|(name, val)| val.unwrap_or(Expr::Ident(name))),
            just(Token::Star)
                .ignore_then(expr.clone())
                .map(|e| Expr::Splat(Box::new(e))),
            expr.clone(),
        ));

        let postfix = primary
            .foldl(
                choice((
                    call_arg
                        .clone()
                        .separated_by(just(Token::Comma))
                        .allow_trailing()
                        .collect()
                        .delimited_by(just(Token::LParen), just(Token::RParen))
                        .map(Postfix::Call),
                    choice((slice.clone(), expr_full.clone()))
                        .delimited_by(just(Token::LBracket), just(Token::RBracket))
                        .map(Postfix::Index),
                    just(Token::Dot).ignore_then(choice((
                        call_arg
                            .separated_by(just(Token::Comma))
                            .allow_trailing()
                            .collect()
                            .delimited_by(just(Token::LParen), just(Token::RParen))
                            .map(Postfix::BroadcastCall),
                        ident.map(Postfix::Attribute),
                    ))),
                    just(Token::DotDot).ignore_then(ident).map(Postfix::Gather),
                    just(Token::Partial)
                        .ignore_then(
                            expr.clone()
                                .map(|e| {
                                    if e == Expr::Placeholder {
                                        None
                                    } else {
                                        Some(e)
                                    }
                                })
                                .separated_by(just(Token::Comma))
                                .allow_trailing()
                                .collect()
                                .delimited_by(just(Token::LParen), just(Token::RParen)),
                        )
                        .map_with(|args, extra| Postfix::PartialCall(args, extra.span().start)),
                ))
                .repeated(),
                |lhs, op| match op {
                    Postfix::Call(args) => Expr::Call(Box::new(lhs), args),
                    Postfix::BroadcastCall(args) => Expr::BroadcastCall(Box::new(lhs), args),
                    Postfix::Index(index) => Expr::Index(Box::new(lhs), Box::new(index)),
                    Postfix::Attribute(attr) => Expr::Attribute(Box::new(lhs), attr),
                    Postfix::Gather(attr) => Expr::Gather(Box::new(lhs), attr),
                    Postfix::PartialCall(args, id) => Expr::PartialCall(Box::new(lhs), args, id),
                },
            )
            .boxed();

        let unary = choice((
            just(Token::Minus).to("-"),
            just(Token::Not).to("not"),
            select! { Token::Operator(op) if op == "√" => "sqrt" },
            select! { Token::Operator(op) if op == "∑" => "sum" },
            select! { Token::Operator(op) if op == "∏" => "product" },
        ))
        .repeated()
        .foldr(postfix, |op, rhs| match op {
            "-" => Expr::BinaryOp(Box::new(Expr::Number(0)), "-".to_string(), Box::new(rhs)),
            "not" => Expr::Call(Box::new(Expr::Ident("not".to_string())), vec![rhs]),
            "sqrt" => Expr::Call(Box::new(Expr::Ident("sqrt".to_string())), vec![rhs]),
            "sum" => Expr::Call(Box::new(Expr::Ident("sum".to_string())), vec![rhs]),
            "product" => Expr::Call(Box::new(Expr::Ident("product".to_string())), vec![rhs]),
            _ => unreachable!(),
        })
        .boxed();

        let compose_op = choice((
            just(Token::ComposeStar).to((
                PipeData {
                    style: CallStyle::Star,
                    none_aware: false,
                },
                true,
            )),
            just(Token::ComposeDoubleStar).to((
                PipeData {
                    style: CallStyle::DoubleStar,
                    none_aware: false,
                },
                true,
            )),
            just(Token::ComposeNone).to((
                PipeData {
                    style: CallStyle::Standard,
                    none_aware: true,
                },
                true,
            )),
            just(Token::ComposeForward).to((
                PipeData {
                    style: CallStyle::Standard,
                    none_aware: false,
                },
                false,
            )),
            just(Token::ComposeBackward).to((
                PipeData {
                    style: CallStyle::Standard,
                    none_aware: false,
                },
                true,
            )),
            select! { Token::Operator(op) if op == "∘" => (PipeData {
                style: CallStyle::Standard,
                none_aware: false,
            }, true)},
        ));

        let composition = unary
            .clone()
            .then(
                compose_op
                    .then(unary.clone())
                    .map_with(|v, e| (v, e.span()))
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .map(|(lhs, rest)| {
                rest.into_iter()
                    .fold(lhs, |acc, (((op, is_backward), rhs), span)| {
                        if is_backward {
                            Expr::Compose(Box::new(acc), op, Box::new(rhs), span.start)
                        } else {
                            Expr::Compose(Box::new(rhs), op, Box::new(acc), span.start)
                        }
                    })
            })
            .boxed();

        let pipe_op = choice((
            just(Token::PipeForward).to((
                PipeData {
                    style: CallStyle::Standard,
                    none_aware: false,
                },
                false,
            )),
            just(Token::PipeForwardStar).to((
                PipeData {
                    style: CallStyle::Star,
                    none_aware: false,
                },
                false,
            )),
            just(Token::PipeForwardDoubleStar).to((
                PipeData {
                    style: CallStyle::DoubleStar,
                    none_aware: false,
                },
                false,
            )),
            just(Token::PipeForwardNone).to((
                PipeData {
                    style: CallStyle::Standard,
                    none_aware: true,
                },
                false,
            )),
            just(Token::PipeForwardNoneStar).to((
                PipeData {
                    style: CallStyle::Star,
                    none_aware: true,
                },
                false,
            )),
            just(Token::PipeForwardNoneDoubleStar).to((
                PipeData {
                    style: CallStyle::DoubleStar,
                    none_aware: true,
                },
                false,
            )),
            just(Token::PipeBackward).to((
                PipeData {
                    style: CallStyle::Standard,
                    none_aware: false,
                },
                true,
            )),
            just(Token::PipeBackwardStar).to((
                PipeData {
                    style: CallStyle::Star,
                    none_aware: false,
                },
                true,
            )),
            just(Token::PipeBackwardDoubleStar).to((
                PipeData {
                    style: CallStyle::DoubleStar,
                    none_aware: false,
                },
                true,
            )),
            just(Token::PipeBackwardNone).to((
                PipeData {
                    style: CallStyle::Standard,
                    none_aware: true,
                },
                true,
            )),
            just(Token::PipeBackwardNoneStar).to((
                PipeData {
                    style: CallStyle::Star,
                    none_aware: true,
                },
                true,
            )),
            just(Token::PipeBackwardNoneDoubleStar).to((
                PipeData {
                    style: CallStyle::DoubleStar,
                    none_aware: true,
                },
                true,
            )),
        ));

        let pipe = composition
            .clone()
            .foldl(
                pipe_op.then(composition.clone()).repeated(),
                |lhs, ((op, rev), rhs)| {
                    if rev {
                        Expr::Pipe(Box::new(rhs), op, Box::new(lhs))
                    } else {
                        Expr::Pipe(Box::new(lhs), op, Box::new(rhs))
                    }
                },
            )
            .boxed();

        let none_coalesce = pipe
            .clone()
            .foldl(
                just(Token::NoneCoalesce)
                    .ignore_then(pipe.clone())
                    .repeated(),
                |lhs, rhs| Expr::NoneCoalesce(Box::new(lhs), Box::new(rhs)),
            )
            .boxed();

        let multiplicative_op = choice((
            just(Token::Star).to(("*", false)),
            just(Token::Slash).to(("/", false)),
            select! { Token::Operator(op) if op == "÷" => ("÷", false) },
            just(Token::Percent).to(("%", false)),
            just(Token::Power).to(("**", false)),
            just(Token::DotStar).to(("mul", true)),
            just(Token::DotSlash).to(("div", true)),
            just(Token::DotPercent).to(("mod", true)),
        ));

        let multiplicative = none_coalesce
            .clone()
            .foldl(
                multiplicative_op.then(none_coalesce.clone()).repeated(),
                |lhs, ((op, is_broadcast), rhs)| {
                    if is_broadcast {
                        Expr::BroadcastCall(Box::new(Expr::Ident(op.to_string())), vec![lhs, rhs])
                    } else {
                        Expr::BinaryOp(Box::new(lhs), op.to_string(), Box::new(rhs))
                    }
                },
            )
            .boxed();

        let additive = multiplicative
            .clone()
            .foldl(
                choice((
                    just(Token::Plus).to(("+", false)),
                    just(Token::Minus).to(("-", false)),
                    just(Token::DotPlus).to(("add", true)),
                    just(Token::DotMinus).to(("sub", true)),
                ))
                .then(multiplicative.clone())
                .repeated(),
                |lhs, ((op, is_broadcast), rhs)| {
                    if is_broadcast {
                        Expr::BroadcastCall(Box::new(Expr::Ident(op.to_string())), vec![lhs, rhs])
                    } else {
                        Expr::BinaryOp(Box::new(lhs), op.to_string(), Box::new(rhs))
                    }
                },
            )
            .boxed();

        let range = additive
            .clone()
            .then(just(Token::DotDot).ignore_then(additive.clone()).or_not())
            .map(|(lhs, rhs)| {
                if let Some(rhs) = rhs {
                    Expr::Range(Box::new(lhs), Box::new(rhs))
                } else {
                    lhs
                }
            })
            .boxed();

        let comparison = range
            .clone()
            .foldl(
                choice((
                    just(Token::Pipe).to(("|".to_string(), false)),
                    just(Token::Equal).to(("==".to_string(), false)),
                    just(Token::NotEqual).to(("!=".to_string(), false)),
                    just(Token::LessThan).to(("<".to_string(), false)),
                    just(Token::LessEqual).to(("<=".to_string(), false)),
                    just(Token::GreaterThan).to((">".to_string(), false)),
                    just(Token::GreaterEqual).to((">=".to_string(), false)),
                    just(Token::Is).to(("is".to_string(), false)),
                    just(Token::In).to(("in".to_string(), false)),
                    select! { Token::Operator(op) if op == "notin" => ("notin".to_string(), false) },
                    select! { Token::Operator(op) if op == "∈" => ("in".to_string(), false) },
                    select! { Token::Operator(op) if op == "∉" => ("notin".to_string(), false) },
                    just(Token::DotEqual).to(("eq".to_string(), true)),
                    just(Token::DotNotEqual).to(("ne".to_string(), true)),
                    just(Token::DotLess).to(("lt".to_string(), true)),
                    just(Token::DotLessEqual).to(("le".to_string(), true)),
                    just(Token::DotGreater).to(("gt".to_string(), true)),
                    just(Token::DotGreaterEqual).to(("ge".to_string(), true)),
                    select! { Token::Operator(op) => (op, false) },
                ))
                .then(range.clone())
                .repeated(),
                |lhs, ((op, is_broadcast), rhs)| {
                    if is_broadcast {
                        Expr::BroadcastCall(Box::new(Expr::Ident(op)), vec![lhs, rhs])
                    } else {
                        Expr::BinaryOp(Box::new(lhs), op, Box::new(rhs))
                    }
                },
            )
            .boxed();

        let logical_and = comparison
            .clone()
            .foldl(
                just(Token::And).ignore_then(comparison.clone()).repeated(),
                |lhs, rhs| Expr::BinaryOp(Box::new(lhs), "and".to_string(), Box::new(rhs)),
            )
            .boxed();

        let logical_or = logical_and
            .clone()
            .foldl(
                just(Token::Or).ignore_then(logical_and.clone()).repeated(),
                |lhs, rhs| Expr::BinaryOp(Box::new(lhs), "or".to_string(), Box::new(rhs)),
            )
            .boxed();

        let ternary = choice((
            just(Token::If)
                .ignore_then(expr.clone())
                .then_ignore(just(Token::Then))
                .then(expr.clone())
                .then_ignore(just(Token::Else))
                .then(expr.clone())
                .map(|((cond, then_e), else_e)| {
                    Expr::Ternary(Box::new(cond), Box::new(then_e), Box::new(else_e))
                }),
            logical_or
                .clone()
                .then_ignore(just(Token::Question))
                .then(expr.clone())
                .then_ignore(just(Token::Colon))
                .then(expr.clone())
                .map(|((cond, then_e), else_e)| {
                    Expr::Ternary(Box::new(cond), Box::new(then_e), Box::new(else_e))
                }),
            logical_or
                .clone()
                .then_ignore(just(Token::If))
                .then(expr.clone())
                .then_ignore(just(Token::Else))
                .then(expr.clone())
                .map(|((then_e, cond), else_e)| {
                    Expr::Ternary(Box::new(cond), Box::new(then_e), Box::new(else_e))
                }),
        ))
        .boxed();

        let where_block = logical_or
            .clone()
            .then_ignore(just(Token::Where))
            .then(block(stmt.clone()))
            .map_with(|(e, stmts), extra| Expr::Where(Box::new(e), stmts, extra.span().start))
            .boxed();

        let lambda = choice((
            just(Token::Lambda)
                .ignore_then(ident.separated_by(just(Token::Comma)).collect())
                .then_ignore(just(Token::Colon))
                .then(expr.clone()),
            just(Token::At)
                .ignore_then(ident.separated_by(just(Token::Comma)).collect())
                .then_ignore(just(Token::ThinArrow))
                .then(expr.clone()),
        ))
        .map_with(|(args, body), extra| Expr::Lambda(args, Box::new(body), extra.span().start))
        .boxed();

        let arrow_lambda = choice((
            ident.map(|name| vec![name]).then_ignore(just(Token::Arrow)),
            ident
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .collect()
                .delimited_by(just(Token::LParen), just(Token::RParen))
                .then_ignore(just(Token::Arrow)),
            just(Token::Arrow).to(vec![]),
        ))
        .then(expr.clone())
        .map_with(|(args, body), extra| Expr::Lambda(args, Box::new(body), extra.span().start))
        .boxed();

        let macro_call = just(Token::At)
            .ignore_then(choice((just(Token::Dot).to(".".to_string()), ident)))
            .then(
                expr.clone()
                    .separated_by(just(Token::Comma))
                    .allow_trailing()
                    .collect()
                    .delimited_by(just(Token::LParen), just(Token::RParen))
                    .or_not(),
            )
            .then(expr.clone().or_not())
            .map(|((name, args_opt), e_opt)| {
                let mut args: Vec<Expr> = args_opt.unwrap_or_default();
                if let Some(e) = e_opt {
                    args.push(e);
                }
                Expr::MacroCall(name, args)
            })
            .boxed();

        let query_clause = choice((
            just(Token::Where)
                .ignore_then(expr.clone())
                .map(QueryClause::Where),
            just(Token::OrderBy)
                .ignore_then(expr.clone())
                .then(
                    choice((
                        just(Token::Ascending).to(true),
                        just(Token::Descending).to(false),
                    ))
                    .or_not()
                    .map(|d| d.unwrap_or(true)),
                )
                .map(|(e, d)| QueryClause::OrderBy(e, d)),
        ));

        let query = just(Token::From)
            .ignore_then(ident)
            .then_ignore(just(Token::In))
            .then(expr.clone())
            .then(query_clause.repeated().collect::<Vec<_>>())
            .then_ignore(just(Token::Select))
            .then(expr.clone())
            .map_with(|(((from, in_expr), clauses), select), extra| Expr::Query {
                from,
                in_expr: Box::new(in_expr),
                clauses,
                select: Box::new(select),
                id: extra.span().start,
            })
            .boxed();

        let formula = logical_or
            .clone()
            .foldl(
                just(Token::Tilde)
                    .ignore_then(logical_or.clone())
                    .repeated(),
                |lhs, rhs| Expr::Formula(Box::new(lhs), Box::new(rhs)),
            )
            .boxed();

        choice((
            lambda,
            arrow_lambda,
            ternary,
            where_block,
            macro_call,
            query,
            formula,
        ))
        .boxed()
    })
}

fn stmt_parser_internal<'a, E, P, PT>(
    expr: E,
    stmt: impl Parser<'a, &'a [Token], Stmt, Error<'a>> + Clone + 'a,
    pattern: P,
    pattern_top: PT,
) -> impl Parser<'a, &'a [Token], Stmt, Error<'a>> + Clone
where
    E: Parser<'a, &'a [Token], Expr, Error<'a>> + Clone + 'a,
    P: Parser<'a, &'a [Token], Pattern, Error<'a>> + Clone + 'a,
    PT: Parser<'a, &'a [Token], Pattern, Error<'a>> + Clone + 'a,
{
    let ident = select! { Token::Ident(name) => name };

    let operator_stmt = just(Token::OperatorKeyword)
        .ignore_then(select! { Token::Operator(op) => op })
        .then(just(Token::Assign).ignore_then(ident).or_not())
        .map(|(op, name)| {
            if let Some(n) = name {
                Stmt::Operator(format!("{}:{}", op, n))
            } else {
                Stmt::Operator(op)
            }
        });

    let func_def = {
        let prefix = choice((
            just(Token::Copyclosure).to(0),
            just(Token::Addpattern).to(1),
            just(Token::Yield).to(2),
        ))
        .repeated()
        .collect::<Vec<_>>();

        let func_name = choice((
            ident.map(|n| vec![n]),
            select! { Token::BacktickString(s) => vec![s] },
            select! { Token::Operator(op) => vec![format!("__op_{}", op)] },
        ));

        let func_body = choice((
            block(stmt.clone()),
            just(Token::Assign)
                .ignore_then(expr.clone())
                .map(|e| vec![Stmt::Return(e)]),
        ));

        prefix
            .then(choice((
                just(Token::Def).to(false),
                just(Token::Gen).to(true),
            )))
            .then(func_name)
            .then(
                choice((
                    pattern_top
                        .clone()
                        .then(just(Token::Assign).ignore_then(expr.clone()).or_not())
                        .map(|(p, d)| (Some((p, d)), None)),
                    just(Token::Star)
                        .ignore_then(ident)
                        .map(|n| (None, Some(n))),
                ))
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LParen), just(Token::RParen)),
            )
            .then(just(Token::ThinArrow).ignore_then(type_parser()).or_not())
            .then(func_body)
            .map_with(
                |(((((prefixes, is_gen), name), params), return_type), body), extra| {
                    let span: Span = extra.span();
                    let mut args = vec![];
                    let mut vararg = None;
                    for (p, v) in params {
                        if let Some((arg_pat, default)) = p {
                            if let Pattern::Rest(ref name) = arg_pat {
                                vararg = Some(name.clone());
                            } else {
                                args.push((arg_pat, default));
                            }
                        }
                        if let Some(varg) = v {
                            vararg = Some(varg);
                        }
                    }
                    let mut is_copyclosure = false;
                    let mut is_addpattern = false;
                    let mut forced_generator = is_gen;
                    for p in prefixes {
                        if p == 0 {
                            is_copyclosure = true;
                        } else if p == 1 {
                            is_addpattern = true;
                        } else if p == 2 {
                            forced_generator = true;
                        }
                    }

                    Stmt::FunctionDef {
                        name,
                        args,
                        vararg,
                        return_type,
                        is_generator: forced_generator || is_generator(&body),
                        body,
                        is_copyclosure,
                        is_addpattern,
                        id: span.start,
                    }
                },
            )
    };

    let macro_def = just(Token::Macro)
        .ignore_then(ident)
        .then(
            ident
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LParen), just(Token::RParen)),
        )
        .then(block(stmt.clone()))
        .map(|((name, args), body)| Stmt::MacroDef { name, args, body });

    let data_def = just(Token::Data)
        .ignore_then(ident)
        .then(
            ident
                .then(just(Token::Colon).ignore_then(type_parser()).or_not())
                .then(just(Token::Assign).ignore_then(expr.clone()).or_not())
                .map(|((name, ty), default)| (name, ty, default))
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .collect()
                .delimited_by(just(Token::LParen), just(Token::RParen)),
        )
        .map_with(|(name, fields), extra| {
            let span: Span = extra.span();
            Stmt::DataDef(name, fields, span.start)
        });

    let struct_def = just(Token::Struct)
        .ignore_then(ident)
        .then_ignore(just(Token::Colon))
        .then_ignore(just(Token::Newline).repeated())
        .then(
            ident
                .then_ignore(just(Token::Colon))
                .then(type_parser())
                .separated_by(stmt_sep())
                .allow_leading()
                .allow_trailing()
                .collect()
                .delimited_by(just(Token::Indent), just(Token::Dedent)),
        )
        .map_with(|(name, fields), extra| {
            let span: Span = extra.span();
            Stmt::StructDef(name, fields, span.start)
        });
    let class_def = just(Token::Class)
        .ignore_then(ident)
        .then(
            ident
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LParen), just(Token::RParen))
                .or_not()
                .map(|v| v.unwrap_or_default()),
        )
        .then(
            just(Token::Extends)
                .ignore_then(ident)
                .then(
                    expr.clone()
                        .separated_by(just(Token::Comma))
                        .allow_trailing()
                        .collect::<Vec<_>>()
                        .delimited_by(just(Token::LParen), just(Token::RParen))
                        .or_not()
                        .map(|v| v.unwrap_or_default()),
                )
                .or_not(),
        )
        .then(
            just(Token::With)
                .ignore_then(
                    ident
                        .separated_by(just(Token::Comma))
                        .allow_trailing()
                        .collect::<Vec<_>>(),
                )
                .or_not()
                .map(|v| v.unwrap_or_default()),
        )
        .then(block(stmt.clone()))
        .map_with(|((((name, args), superclass), traits), body), extra| {
            let span: Span = extra.span();
            Stmt::ClassDef {
                name,
                args,
                superclass,
                traits,
                body,
                id: span.start,
            }
        });
    let trait_def = just(Token::Trait)
        .ignore_then(ident)
        .then(block(stmt.clone()))
        .map_with(|(name, body), extra| {
            let span: Span = extra.span();
            Stmt::TraitDef(name, body, span.start)
        });

    let impl_block = just(Token::Impl)
        .ignore_then(
            type_parser()
                .then_ignore(just(Token::For))
                .or_not()
                .then(type_parser()),
        )
        .then(block(stmt.clone()))
        .map_with(|((trait_type, target_type), body), extra| {
            let span: Span = extra.span();
            Stmt::ImplBlock(trait_type, target_type, body, span.start)
        });

    let return_stmt = just(Token::Return)
        .ignore_then(expr.clone())
        .map(Stmt::Return);

    let break_stmt = just(Token::Break).to(Stmt::Break);

    let yield_stmt = just(Token::Yield)
        .ignore_then(expr.clone())
        .map(Stmt::Yield);

    let global_stmt = just(Token::Global)
        .ignore_then(ident.separated_by(just(Token::Comma)).at_least(1).collect())
        .map(Stmt::Global);

    let nonlocal_stmt = just(Token::Nonlocal)
        .ignore_then(ident.separated_by(just(Token::Comma)).at_least(1).collect())
        .map(Stmt::Nonlocal);

    let template_def = just(Token::Template)
        .ignore_then(ident)
        .then(
            ident
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .collect()
                .delimited_by(just(Token::LParen), just(Token::RParen)),
        )
        .then(block(stmt.clone()))
        .map_with(|((name, args), body), extra| {
            Stmt::TemplateDef(name, args, body, extra.span().start)
        });

    let passthrough_stmt = just(Token::Rust)
        .ignore_then(just(Token::Colon).or_not())
        .ignore_then(just(Token::Newline).or_not())
        .ignore_then(just(Token::Indent).or_not())
        .ignore_then(select! { Token::String(s) => s })
        .then_ignore(just(Token::PassthroughStart))
        .then_ignore(just(Token::Dedent).or_not())
        .then_ignore(just(Token::Newline).or_not())
        .map(Stmt::Passthrough)
        .or(just(Token::Rust)
            .ignore_then(just(Token::Colon).or_not())
            .ignore_then(just(Token::Newline).or_not())
            .ignore_then(just(Token::Indent).or_not())
            .ignore_then(select! { Token::String(s) => s })
            .then_ignore(just(Token::PassthroughStart))
            .then_ignore(just(Token::Dedent).or_not())
            .map(Stmt::Passthrough))
        .or(select! { Token::String(s) => s }
            .then_ignore(just(Token::PassthroughStart))
            .map(Stmt::Passthrough));

    let native_import_stmt = just(Token::From)
        .ignore_then(ident.separated_by(just(Token::Dot)).collect())
        .then_ignore(just(Token::Import))
        .then(ident.separated_by(just(Token::Comma)).collect())
        .map(|(path, items)| Stmt::NativeImport(path, items));

    let use_stmt = just(Token::Use)
        .ignore_then(
            any()
                .filter(|t| !matches!(t, Token::Newline | Token::Semicolon | Token::Eof))
                .repeated()
                .at_least(1)
                .collect::<Vec<_>>(),
        )
        .map(|tokens| {
            let mut s = String::new();
            for t in tokens {
                match t {
                    Token::Ident(i) => s.push_str(&i),
                    Token::Colon => s.push(':'),
                    Token::Dot => s.push('.'),
                    Token::Star => s.push('*'),
                    Token::LBrace => s.push('{'),
                    Token::RBrace => s.push('}'),
                    Token::Comma => s.push_str(", "),
                    Token::As => s.push_str(" as "),
                    _ => s.push_str(&format!("{:?}", t)),
                }
            }
            Stmt::Use(s)
        });

    let if_stmt = just(Token::If)
        .ignore_then(expr.clone())
        .then(block(stmt.clone()))
        .map(|(cond, body)| Stmt::If(cond, body));

    let while_stmt = just(Token::While)
        .ignore_then(expr.clone())
        .then(block(stmt.clone()))
        .map(|(cond, body)| Stmt::While(cond, body));

    let match_for_stmt = just(Token::Match)
        .ignore_then(just(Token::For))
        .ignore_then(pattern.clone())
        .then_ignore(just(Token::In))
        .then(expr.clone())
        .then(block(stmt.clone()))
        .map(|((pat, e), body)| Stmt::MatchFor(pat, e, body));

    let for_stmt = just(Token::For)
        .ignore_then(pattern.clone())
        .then_ignore(just(Token::In))
        .then(expr.clone())
        .then(block(stmt.clone()))
        .map(|((pat, e), body)| Stmt::MatchFor(pat, e, body));

    let case_parser = just(Token::Case)
        .ignore_then(pattern.clone())
        .then(just(Token::If).ignore_then(expr.clone()).or_not())
        .then(block(stmt.clone()))
        .map(|((p, g), b)| (p, g, b));

    let match_stmt = just(Token::Match)
        .ignore_then(expr.clone())
        .then_ignore(just(Token::Colon))
        .then_ignore(just(Token::Newline).repeated())
        .then(
            case_parser
                .separated_by(stmt_sep().or_not())
                .allow_leading()
                .allow_trailing()
                .collect()
                .delimited_by(just(Token::Indent), just(Token::Dedent)),
        )
        .map(|(e, cases)| Stmt::Match(e, cases));

    let index_assign = expr
        .clone()
        .then_ignore(just(Token::Assign))
        .then(expr.clone())
        .try_map(|(lhs, rhs), span| match lhs {
            Expr::Index(base, index) => Ok(Stmt::IndexAssign(*base, *index, rhs)),
            Expr::Attribute(base, attr) => Ok(Stmt::AttributeAssign(*base, attr, rhs)),
            _ => Err(Rich::custom(
                span,
                "Expected index or attribute expression on LHS of assignment",
            )),
        })
        .boxed();

    let assign = pattern
        .clone()
        .then_ignore(just(Token::Assign))
        .then(expr.clone())
        .map(|(p, e)| Stmt::Assign(p, e))
        .boxed();

    let augmented_assign = expr
        .clone()
        .then(choice((
            just(Token::PlusAssign).to("+"),
            just(Token::MinusAssign).to("-"),
            just(Token::StarAssign).to("*"),
            just(Token::SlashAssign).to("/"),
        )))
        .then(expr.clone())
        .try_map(|((lhs, op), rhs), span| {
            if let Expr::Ident(name) = lhs.clone() {
                Ok(Stmt::Assign(
                    Pattern::Var(name, None),
                    Expr::BinaryOp(Box::new(lhs), op.to_string(), Box::new(rhs)),
                ))
            } else {
                Err(Rich::custom(
                    span,
                    "Expected identifier on LHS of augmented assignment",
                ))
            }
        });

    let macro_block_stmt = ident
        .then(expr.clone().repeated().collect::<Vec<_>>())
        .then(block(stmt.clone()))
        .then(
            ident
                .then(expr.clone().repeated().collect::<Vec<_>>())
                .then(block(stmt.clone()))
                .map(|((n, a), b)| (n, a, b))
                .repeated()
                .collect::<Vec<_>>(),
        )
        .map_with(|(((name, args), body), branches), extra| Stmt::MacroBlock {
            name,
            args,
            body,
            branches,
            token_pos: extra.span().start,
        })
        .boxed();

    choice((
        choice((
            return_stmt,
            break_stmt,
            yield_stmt,
            global_stmt,
            nonlocal_stmt,
            passthrough_stmt,
            native_import_stmt,
            use_stmt,
            macro_block_stmt,
            if_stmt,
            while_stmt,
            match_for_stmt,
            for_stmt,
        )),
        choice((
            match_stmt,
            macro_def,
            func_def,
            template_def,
            class_def,
            data_def,
            struct_def,
            trait_def,
            impl_block,
            operator_stmt,
            index_assign,
            augmented_assign,
            assign,
            expr.map(Stmt::Expr),
        )),
    ))
}

pub fn pattern_parser_internal<'a, E>(
    expr: E,
) -> impl Parser<'a, &'a [Token], Pattern, Error<'a>> + Clone
where
    E: Parser<'a, &'a [Token], Expr, Error<'a>> + Clone + 'a,
{
    recursive(|pattern_p| {
        let ident = select! { Token::Ident(name) => name };
        let wildcard = ident.filter(|name| name == "_").map(|_| Pattern::Wildcard);

        let view_pattern = expr
            .clone()
            .delimited_by(just(Token::LParen), just(Token::RParen))
            .then_ignore(just(Token::ThinArrow))
            .then(pattern_p.clone())
            .map(|(f, p)| Pattern::View(Box::new(f), Box::new(p)));

        let string_split = select! { Token::String(s) => s }
            .then_ignore(just(Token::Plus))
            .then(ident)
            .map(|(s, var)| Pattern::StringSplit(s, var, true));

        let rest_pattern = just(Token::Star).ignore_then(ident).map(Pattern::Rest);

        choice((
            wildcard,
            view_pattern,
            string_split,
            rest_pattern,
            ident
                .then(
                    pattern_p
                        .clone()
                        .separated_by(just(Token::Comma))
                        .allow_trailing()
                        .collect()
                        .delimited_by(just(Token::LParen), just(Token::RParen)),
                )
                .map(|(name, args)| Pattern::Data(name, args)),
            pattern_p
                .clone()
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .collect()
                .delimited_by(just(Token::LParen), just(Token::RParen))
                .map(Pattern::Tuple),
            pattern_p
                .clone()
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .collect()
                .delimited_by(just(Token::LBracket), just(Token::RBracket))
                .map(|pats| Pattern::Data("List".to_string(), pats)),
            choice((
                select! { Token::Number(n) => Expr::Number(n) },
                select! { Token::Float(f) => Expr::Float(f) },
                select! { Token::String(s) => Expr::String(s) },
                select! { Token::Ident(name) if name == "None" => Expr::None },
                select! { Token::Ident(name) if name == "True" => Expr::Bool(true) },
                select! { Token::Ident(name) if name == "False" => Expr::Bool(false) },
            ))
            .map(Pattern::Const),
            ident
                .then(just(Token::Colon).ignore_then(type_parser()).or_not())
                .map(|(name, ty)| Pattern::Var(name, ty)),
        ))
        .boxed()
    })
}

pub fn expr_parser<'a>() -> impl Parser<'a, &'a [Token], Expr, Error<'a>> + Clone {
    recursive(|expr_full| {
        let mut stmt = Recursive::declare();
        let expr_top = expr_parser_internal(stmt.clone(), expr_full.clone());
        let pattern_top = pattern_parser_internal(expr_top.clone());

        let tuple_expr = expr_top
            .clone()
            .then(
                just(Token::Comma)
                    .ignore_then(expr_top.clone())
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .map(|(first, rest)| {
                if rest.is_empty() {
                    first
                } else {
                    let mut exprs = vec![first];
                    exprs.extend(rest);
                    Expr::Tuple(exprs)
                }
            })
            .map_with(|e, extra| {
                if contains_placeholder(&e) {
                    Expr::ImplicitLambda(Box::new(replace_placeholders(e)), extra.span().start)
                } else {
                    e
                }
            });

        let tuple_pattern = pattern_top
            .clone()
            .then(
                just(Token::Comma)
                    .ignore_then(pattern_top.clone())
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .map(|(first, rest)| {
                if rest.is_empty() {
                    first
                } else {
                    let mut pats = vec![first];
                    pats.extend(rest);
                    Pattern::Tuple(pats)
                }
            });

        stmt.define(stmt_parser_internal(
            expr_full.clone(),
            stmt.clone(),
            tuple_pattern.clone(),
            pattern_top.clone(),
        ));
        tuple_expr
    })
}

pub fn stmt_parser<'a>() -> impl Parser<'a, &'a [Token], Stmt, Error<'a>> + Clone {
    recursive(|stmt| {
        let mut expr_full = Recursive::declare();
        let expr_top = expr_parser_internal(stmt.clone(), expr_full.clone());
        let pattern_top = pattern_parser_internal(expr_top.clone());

        let tuple_expr = expr_top
            .clone()
            .then(
                just(Token::Comma)
                    .ignore_then(expr_top.clone())
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .map(|(first, rest)| {
                if rest.is_empty() {
                    first
                } else {
                    let mut exprs = vec![first];
                    exprs.extend(rest);
                    Expr::Tuple(exprs)
                }
            })
            .map_with(|e, extra| {
                if contains_placeholder(&e) {
                    Expr::ImplicitLambda(Box::new(replace_placeholders(e)), extra.span().start)
                } else {
                    e
                }
            });

        let tuple_pattern = pattern_top
            .clone()
            .then(
                just(Token::Comma)
                    .ignore_then(pattern_top.clone())
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .map(|(first, rest)| {
                if rest.is_empty() {
                    first
                } else {
                    let mut pats = vec![first];
                    pats.extend(rest);
                    Pattern::Tuple(pats)
                }
            });

        expr_full.define(tuple_expr.clone());
        stmt_parser_internal(
            expr_full.clone(),
            stmt.clone(),
            tuple_pattern.clone(),
            pattern_top.clone(),
        )
    })
}

pub fn parser<'a>() -> impl Parser<'a, &'a [Token], Vec<Stmt>, Error<'a>> {
    let sep = choice((
        just::<Token, _, Error>(Token::Newline),
        just(Token::Semicolon),
    ))
    .repeated()
    .at_least(1)
    .ignored();
    stmt_parser()
        .separated_by(sep.or_not())
        .allow_leading()
        .allow_trailing()
        .collect()
        .then_ignore(just(Token::Eof))
}

pub fn parse(tokens: &[Token]) -> Result<Vec<Stmt>, Vec<Rich<'_, Token>>> {
    let (stmts, errors) = parser().parse(tokens).into_output_errors();
    if errors.is_empty() {
        Ok(stmts.unwrap_or_default())
    } else {
        Err(errors)
    }
}

pub(crate) fn stmt_sep<'a>() -> impl Parser<'a, &'a [Token], (), Error<'a>> + Clone {
    choice((just(Token::Newline), just(Token::Semicolon)))
        .repeated()
        .at_least(1)
        .ignored()
}

pub(crate) fn block<'a, P>(stmt: P) -> impl Parser<'a, &'a [Token], Vec<Stmt>, Error<'a>> + Clone
where
    P: Parser<'a, &'a [Token], Stmt, Error<'a>> + Clone,
{
    let sep = choice((just(Token::Newline), just(Token::Semicolon)))
        .repeated()
        .at_least(1)
        .ignored();
    just(Token::Colon)
        .then(just(Token::Newline).repeated())
        .ignore_then(
            stmt.separated_by(sep.or_not())
                .allow_leading()
                .allow_trailing()
                .collect()
                .delimited_by(just(Token::Indent), just(Token::Dedent)),
        )
}

fn replace_placeholders(expr: Expr) -> Expr {
    match expr {
        Expr::Placeholder => Expr::Ident("_".to_string()),
        Expr::Call(e, args) => Expr::Call(
            Box::new(replace_placeholders(*e)),
            args.into_iter().map(replace_placeholders).collect(),
        ),
        Expr::BroadcastCall(e, args) => Expr::BroadcastCall(
            Box::new(replace_placeholders(*e)),
            args.into_iter().map(replace_placeholders).collect(),
        ),
        Expr::BinaryOp(l, op, r) => Expr::BinaryOp(
            Box::new(replace_placeholders(*l)),
            op,
            Box::new(replace_placeholders(*r)),
        ),
        Expr::Lambda(args, body, id) => {
            Expr::Lambda(args, Box::new(replace_placeholders(*body)), id)
        }
        Expr::ImplicitLambda(body, id) => {
            Expr::ImplicitLambda(Box::new(replace_placeholders(*body)), id)
        }
        Expr::Pipe(l, op, r) => Expr::Pipe(
            Box::new(replace_placeholders(*l)),
            op,
            Box::new(replace_placeholders(*r)),
        ),
        Expr::Compose(l, op, r, id) => Expr::Compose(
            Box::new(replace_placeholders(*l)),
            op,
            Box::new(replace_placeholders(*r)),
            id,
        ),
        Expr::NoneCoalesce(l, r) => Expr::NoneCoalesce(
            Box::new(replace_placeholders(*l)),
            Box::new(replace_placeholders(*r)),
        ),
        Expr::PartialCall(e, args, id) => Expr::PartialCall(
            Box::new(replace_placeholders(*e)),
            args.into_iter()
                .map(|a| a.map(replace_placeholders))
                .collect(),
            id,
        ),
        Expr::List(exprs) => Expr::List(exprs.into_iter().map(replace_placeholders).collect()),
        Expr::Dict(kvs) => Expr::Dict(
            kvs.into_iter()
                .map(|(k, v)| (replace_placeholders(k), replace_placeholders(v)))
                .collect(),
        ),
        Expr::Index(e, i) => Expr::Index(
            Box::new(replace_placeholders(*e)),
            Box::new(replace_placeholders(*i)),
        ),
        Expr::Attribute(e, attr) => Expr::Attribute(Box::new(replace_placeholders(*e)), attr),
        Expr::AttributePartial(name) => {
            Expr::Attribute(Box::new(Expr::Ident("_".to_string())), name)
        }
        Expr::Where(e, stmts, id) => Expr::Where(Box::new(replace_placeholders(*e)), stmts, id),
        Expr::Set(exprs) => Expr::Set(exprs.into_iter().map(replace_placeholders).collect()),
        Expr::Frozenset(exprs) => {
            Expr::Frozenset(exprs.into_iter().map(replace_placeholders).collect())
        }
        Expr::Multiset(exprs) => {
            Expr::Multiset(exprs.into_iter().map(replace_placeholders).collect())
        }
        Expr::Tuple(exprs) => Expr::Tuple(exprs.into_iter().map(replace_placeholders).collect()),
        Expr::Ternary(c, t, e) => Expr::Ternary(
            Box::new(replace_placeholders(*c)),
            Box::new(replace_placeholders(*t)),
            Box::new(replace_placeholders(*e)),
        ),
        Expr::LazyList(exprs, id) => {
            Expr::LazyList(exprs.into_iter().map(replace_placeholders).collect(), id)
        }
        Expr::IndexPartial(idx) => Expr::Index(
            Box::new(Expr::Ident("_".to_string())),
            Box::new(replace_placeholders(*idx)),
        ),
        Expr::Comprehension(e, p, i, is_lazy, id) => Expr::Comprehension(
            Box::new(replace_placeholders(*e)),
            p,
            Box::new(replace_placeholders(*i)),
            is_lazy,
            id,
        ),
        Expr::Hcat(exprs) => Expr::Hcat(exprs.into_iter().map(replace_placeholders).collect()),
        Expr::Vcat(exprs) => Expr::Vcat(exprs.into_iter().map(replace_placeholders).collect()),
        Expr::Splat(e) => Expr::Splat(Box::new(replace_placeholders(*e))),
        Expr::MacroCall(name, args) => {
            Expr::MacroCall(name, args.into_iter().map(replace_placeholders).collect())
        }
        Expr::Slice(start, stop, step) => Expr::Slice(
            start.map(|e| Box::new(replace_placeholders(*e))),
            stop.map(|e| Box::new(replace_placeholders(*e))),
            step.map(|e| Box::new(replace_placeholders(*e))),
        ),
        Expr::Formula(lhs, rhs) => Expr::Formula(
            Box::new(replace_placeholders(*lhs)),
            Box::new(replace_placeholders(*rhs)),
        ),
        Expr::Query {
            from,
            in_expr,
            clauses,
            select,
            id,
        } => Expr::Query {
            from,
            in_expr: Box::new(replace_placeholders(*in_expr)),
            clauses: clauses
                .into_iter()
                .map(|c| match c {
                    QueryClause::Where(e) => QueryClause::Where(replace_placeholders(e)),
                    QueryClause::OrderBy(e, d) => QueryClause::OrderBy(replace_placeholders(e), d),
                })
                .collect(),
            select: Box::new(replace_placeholders(*select)),
            id,
        },
        Expr::Range(l, r) => Expr::Range(
            Box::new(replace_placeholders(*l)),
            Box::new(replace_placeholders(*r)),
        ),
        Expr::Gather(e, attr) => Expr::Gather(Box::new(replace_placeholders(*e)), attr),
        Expr::Number(_)
        | Expr::Float(_)
        | Expr::String(_)
        | Expr::Bool(_)
        | Expr::None
        | Expr::Ident(_)
        | Expr::OperatorFunction(_)
        | Expr::Shell(_)
        | Expr::Passthrough(_) => expr,
        Expr::FString(exprs) => {
            Expr::FString(exprs.into_iter().map(replace_placeholders).collect())
        }
    }
}

fn contains_placeholder(expr: &Expr) -> bool {
    match expr {
        Expr::Placeholder => true,
        Expr::Call(e, args) => contains_placeholder(e) || args.iter().any(contains_placeholder),
        Expr::BroadcastCall(e, args) => {
            contains_placeholder(e) || args.iter().any(contains_placeholder)
        }
        Expr::BinaryOp(l, _, r) => contains_placeholder(l) || contains_placeholder(r),
        Expr::Lambda(_, body, _) => contains_placeholder(body),
        Expr::ImplicitLambda(body, _) => contains_placeholder(body),
        Expr::Pipe(l, _, r) => contains_placeholder(l) || contains_placeholder(r),
        Expr::Compose(l, _, r, _) => contains_placeholder(l) || contains_placeholder(r),
        Expr::NoneCoalesce(l, r) => contains_placeholder(l) || contains_placeholder(r),
        Expr::PartialCall(e, args, _) => {
            contains_placeholder(e)
                || args
                    .iter()
                    .any(|a| a.as_ref().is_some_and(contains_placeholder))
        }
        Expr::List(exprs) => exprs.iter().any(contains_placeholder),
        Expr::Dict(kvs) => kvs
            .iter()
            .any(|(k, v)| contains_placeholder(k) || contains_placeholder(v)),
        Expr::Index(e, i) => contains_placeholder(e) || contains_placeholder(i),
        Expr::Attribute(e, _) => contains_placeholder(e),
        Expr::AttributePartial(_) => true,
        Expr::Where(e, _, _) => contains_placeholder(e),
        Expr::Set(exprs) => exprs.iter().any(contains_placeholder),
        Expr::Frozenset(exprs) => exprs.iter().any(contains_placeholder),
        Expr::Multiset(exprs) => exprs.iter().any(contains_placeholder),
        Expr::Tuple(exprs) => exprs.iter().any(contains_placeholder),
        Expr::Ternary(c, t, e) => {
            contains_placeholder(c) || contains_placeholder(t) || contains_placeholder(e)
        }
        Expr::LazyList(exprs, _) => exprs.iter().any(contains_placeholder),
        Expr::IndexPartial(_) => true,
        Expr::Comprehension(e, _, i, _, _) => contains_placeholder(e) || contains_placeholder(i),
        Expr::Hcat(exprs) => exprs.iter().any(contains_placeholder),
        Expr::Vcat(exprs) => exprs.iter().any(contains_placeholder),
        Expr::Splat(e) => contains_placeholder(e),
        Expr::MacroCall(_, args) => args.iter().any(contains_placeholder),
        Expr::Slice(start, stop, step) => {
            start.as_ref().is_some_and(|e| contains_placeholder(e))
                || stop.as_ref().is_some_and(|e| contains_placeholder(e))
                || step.as_ref().is_some_and(|e| contains_placeholder(e))
        }
        Expr::Formula(lhs, rhs) => contains_placeholder(lhs) || contains_placeholder(rhs),
        Expr::Range(l, r) => contains_placeholder(l) || contains_placeholder(r),
        Expr::Gather(e, _) => contains_placeholder(e),
        Expr::Query {
            in_expr,
            clauses,
            select,
            ..
        } => {
            contains_placeholder(in_expr)
                || clauses.iter().any(|c| match c {
                    QueryClause::Where(e) => contains_placeholder(e),
                    QueryClause::OrderBy(e, _) => contains_placeholder(e),
                })
                || contains_placeholder(select)
        }
        Expr::Number(_)
        | Expr::Float(_)
        | Expr::String(_)
        | Expr::Bool(_)
        | Expr::None
        | Expr::Ident(_)
        | Expr::OperatorFunction(_)
        | Expr::Shell(_)
        | Expr::Passthrough(_) => false,
        Expr::FString(exprs) => exprs.iter().any(contains_placeholder),
    }
}

pub fn pattern_parser<'a>() -> impl Parser<'a, &'a [Token], Pattern, Error<'a>> + Clone {
    recursive(|_pattern_full| {
        let expr_full = Recursive::declare();
        let stmt = Recursive::declare();
        let expr_top = expr_parser_internal(stmt.clone(), expr_full.clone());
        let pattern_top = pattern_parser_internal(expr_top.clone());

        pattern_top
            .clone()
            .then(
                just(Token::Comma)
                    .ignore_then(pattern_top.clone())
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .map(|(first, rest)| {
                if rest.is_empty() {
                    first
                } else {
                    let mut pats = vec![first];
                    pats.extend(rest);
                    Pattern::Tuple(pats)
                }
            })
    })
}

pub fn parse_fstring(s: &str) -> Vec<Expr> {
    let mut parts = Vec::new();
    let mut current_text = String::new();
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '{' {
            if chars.peek() == Some(&'{') {
                chars.next(); // skip second '{'
                current_text.push('{');
            } else {
                if !current_text.is_empty() {
                    parts.push(Expr::String(current_text.clone()));
                    current_text.clear();
                }

                let mut expr_str = String::new();
                let mut brace_count = 1;
                while let Some(ec) = chars.next() {
                    if ec == '{' {
                        brace_count += 1;
                    } else if ec == '}' {
                        brace_count -= 1;
                        if brace_count == 0 {
                            break;
                        }
                    }
                    expr_str.push(ec);
                }

                let mut lexer = crate::lexer::Lexer::new(&expr_str);
                let tokens: Vec<_> = lexer
                    .tokenize()
                    .into_iter()
                    .filter(|t| {
                        !matches!(t, crate::lexer::Token::Newline | crate::lexer::Token::Eof)
                    })
                    .collect();
                if let Ok(expr) = expr_parser().parse(&tokens).into_result() {
                    parts.push(expr);
                } else {
                    // Fallback to empty string on parse error?
                    parts.push(Expr::String(expr_str));
                }
            }
        } else if c == '}' {
            if chars.peek() == Some(&'}') {
                chars.next(); // skip second '}'
                current_text.push('}');
            } else {
                current_text.push('}');
            }
        } else {
            current_text.push(c);
        }
    }

    if !current_text.is_empty() {
        parts.push(Expr::String(current_text));
    }

    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use chumsky::Parser;

    #[test]
    fn test_type_parser() {
        let test_cases = vec![
            (vec![Token::Ident("Int".to_string())], Type::Int),
            (vec![Token::Ident("String".to_string())], Type::String),
            (vec![Token::Ident("Dynamic".to_string())], Type::Dynamic),
            (
                vec![Token::Ident("MyType".to_string())],
                Type::Custom("MyType".to_string()),
            ),
            (
                vec![
                    Token::Ident("List".to_string()),
                    Token::LessThan,
                    Token::Ident("Int".to_string()),
                    Token::Comma,
                    Token::GreaterThan,
                ],
                Type::Generic("List".to_string(), vec![Type::Int]),
            ),
        ];

        for (tokens, expected) in test_cases {
            let result = type_parser().parse(&tokens).into_result();
            assert_eq!(result.unwrap(), expected);
        }
    }

    #[test]
    fn test_pattern_parser() {
        let test_cases = vec![
            (vec![Token::Ident("_".to_string())], Pattern::Wildcard),
            (
                vec![Token::Ident("x".to_string())],
                Pattern::Var("x".to_string(), None),
            ),
            (
                vec![
                    Token::Ident("x".to_string()),
                    Token::Colon,
                    Token::Ident("Int".to_string()),
                ],
                Pattern::Var("x".to_string(), Some(Type::Int)),
            ),
            (
                vec![
                    Token::LParen,
                    Token::Ident("x".to_string()),
                    Token::Comma,
                    Token::Ident("y".to_string()),
                    Token::RParen,
                ],
                Pattern::Tuple(vec![
                    Pattern::Var("x".to_string(), None),
                    Pattern::Var("y".to_string(), None),
                ]),
            ),
            (
                vec![
                    Token::Ident("User".to_string()),
                    Token::LParen,
                    Token::Ident("name".to_string()),
                    Token::Comma,
                    Token::Ident("age".to_string()),
                    Token::RParen,
                ],
                Pattern::Data(
                    "User".to_string(),
                    vec![
                        Pattern::Var("name".to_string(), None),
                        Pattern::Var("age".to_string(), None),
                    ],
                ),
            ),
            (vec![Token::Number(42)], Pattern::Const(Expr::Number(42))),
            (
                vec![Token::String("hi".to_string())],
                Pattern::Const(Expr::String("hi".to_string())),
            ),
        ];

        for (tokens, expected) in test_cases {
            let result = pattern_parser().parse(&tokens).into_result();
            assert_eq!(result.unwrap(), expected);
        }
    }

    #[test]
    fn test_array_parsing() {
        use crate::lexer::Lexer;

        let test_cases = vec![
            (
                "[1, 2, 3]",
                Expr::List(vec![Expr::Number(1), Expr::Number(2), Expr::Number(3)]),
            ),
            (
                "[1; 2; 3]",
                Expr::Vcat(vec![Expr::Number(1), Expr::Number(2), Expr::Number(3)]),
            ),
            (
                "[1, 2; 3, 4]",
                Expr::Vcat(vec![
                    Expr::Hcat(vec![Expr::Number(1), Expr::Number(2)]),
                    Expr::Hcat(vec![Expr::Number(3), Expr::Number(4)]),
                ]),
            ),
            (
                "[x for x in y]",
                Expr::Comprehension(
                    Box::new(Expr::Ident("x".to_string())),
                    Box::new(Pattern::Var("x".to_string(), None)),
                    Box::new(Expr::Ident("y".to_string())),
                    false,
                    1,
                ),
            ),
            (
                "a .+ b",
                Expr::BroadcastCall(
                    Box::new(Expr::Ident("add".to_string())),
                    vec![Expr::Ident("a".to_string()), Expr::Ident("b".to_string())],
                ),
            ),
            (
                "a .* b",
                Expr::BroadcastCall(
                    Box::new(Expr::Ident("mul".to_string())),
                    vec![Expr::Ident("a".to_string()), Expr::Ident("b".to_string())],
                ),
            ),
        ];

        for (input, expected) in test_cases {
            let mut lexer = Lexer::new(input);
            let tokens: Vec<_> = lexer
                .tokenize()
                .into_iter()
                .filter(|t| !matches!(t, Token::Newline | Token::Eof))
                .collect();
            let result = expr_parser().parse(&tokens).into_result();
            assert_eq!(result.unwrap(), expected, "Failed on input: {}", input);
        }
    }

    #[test]
    fn test_formula_and_query_parser() {
        use crate::lexer::Lexer;

        let test_cases = vec![
            (
                "x ~ x + 1",
                Expr::Formula(
                    Box::new(Expr::Ident("x".to_string())),
                    Box::new(Expr::BinaryOp(
                        Box::new(Expr::Ident("x".to_string())),
                        "+".to_string(),
                        Box::new(Expr::Number(1)),
                    )),
                ),
            ),
            (
                "from x in dataset select x",
                Expr::Query {
                    from: "x".to_string(),
                    in_expr: Box::new(Expr::Ident("dataset".to_string())),
                    clauses: vec![],
                    select: Box::new(Expr::Ident("x".to_string())),
                    id: 0,
                },
            ),
            (
                "from x in dataset where x > 0 select x",
                Expr::Query {
                    from: "x".to_string(),
                    in_expr: Box::new(Expr::Ident("dataset".to_string())),
                    clauses: vec![QueryClause::Where(Expr::BinaryOp(
                        Box::new(Expr::Ident("x".to_string())),
                        ">".to_string(),
                        Box::new(Expr::Number(0)),
                    ))],
                    select: Box::new(Expr::Ident("x".to_string())),
                    id: 0,
                },
            ),
            (
                "from x in dataset orderby x select x",
                Expr::Query {
                    from: "x".to_string(),
                    in_expr: Box::new(Expr::Ident("dataset".to_string())),
                    clauses: vec![QueryClause::OrderBy(Expr::Ident("x".to_string()), true)],
                    select: Box::new(Expr::Ident("x".to_string())),
                    id: 0,
                },
            ),
            (
                "from x in dataset orderby x descending select x",
                Expr::Query {
                    from: "x".to_string(),
                    in_expr: Box::new(Expr::Ident("dataset".to_string())),
                    clauses: vec![QueryClause::OrderBy(Expr::Ident("x".to_string()), false)],
                    select: Box::new(Expr::Ident("x".to_string())),
                    id: 0,
                },
            ),
            (
                "from x in dataset where x > 0 orderby x select x * 2",
                Expr::Query {
                    from: "x".to_string(),
                    in_expr: Box::new(Expr::Ident("dataset".to_string())),
                    clauses: vec![
                        QueryClause::Where(Expr::BinaryOp(
                            Box::new(Expr::Ident("x".to_string())),
                            ">".to_string(),
                            Box::new(Expr::Number(0)),
                        )),
                        QueryClause::OrderBy(Expr::Ident("x".to_string()), true),
                    ],
                    select: Box::new(Expr::BinaryOp(
                        Box::new(Expr::Ident("x".to_string())),
                        "*".to_string(),
                        Box::new(Expr::Number(2)),
                    )),
                    id: 0,
                },
            ),
            (
                "from x in dataset orderby x ascending select x",
                Expr::Query {
                    from: "x".to_string(),
                    in_expr: Box::new(Expr::Ident("dataset".to_string())),
                    clauses: vec![QueryClause::OrderBy(Expr::Ident("x".to_string()), true)],
                    select: Box::new(Expr::Ident("x".to_string())),
                    id: 0,
                },
            ),
            (
                "_ ~ dataset",
                Expr::ImplicitLambda(
                    Box::new(Expr::Formula(
                        Box::new(Expr::Ident("_".to_string())),
                        Box::new(Expr::Ident("dataset".to_string())),
                    )),
                    0,
                ),
            ),
            (
                "from x in dataset select _",
                Expr::ImplicitLambda(
                    Box::new(Expr::Query {
                        from: "x".to_string(),
                        in_expr: Box::new(Expr::Ident("dataset".to_string())),
                        clauses: vec![],
                        select: Box::new(Expr::Ident("_".to_string())),
                        id: 0,
                    }),
                    0,
                ),
            ),
            (
                ".height > 180",
                Expr::ImplicitLambda(
                    Box::new(Expr::BinaryOp(
                        Box::new(Expr::Attribute(
                            Box::new(Expr::Ident("_".to_string())),
                            "height".to_string(),
                        )),
                        ">".to_string(),
                        Box::new(Expr::Number(180)),
                    )),
                    0,
                ),
            ),
            (
                "$[_]",
                Expr::ImplicitLambda(
                    Box::new(Expr::Index(
                        Box::new(Expr::Ident("_".to_string())),
                        Box::new(Expr::Ident("_".to_string())),
                    )),
                    0,
                ),
            ),
            (
                "$[1]",
                Expr::ImplicitLambda(
                    Box::new(Expr::Index(
                        Box::new(Expr::Ident("_".to_string())),
                        Box::new(Expr::Number(1)),
                    )),
                    0,
                ),
            ),
        ];

        for (input, expected) in test_cases {
            let mut lexer = Lexer::new(input);
            let tokens: Vec<_> = lexer
                .tokenize()
                .into_iter()
                .filter(|t| !matches!(t, Token::Newline | Token::Eof))
                .collect();
            let result = expr_parser().parse(&tokens).into_result();
            assert_eq!(result.unwrap(), expected, "Failed on input: {}", input);
        }
    }
    #[test]
    fn test_partial_application() {
        use chumsky::Parser;
        let input = "add$(1, _)";
        let mut lexer = crate::lexer::Lexer::new(input);
        let tokens: Vec<_> = lexer
            .tokenize()
            .into_iter()
            .filter(|t| !matches!(t, crate::lexer::Token::Newline | crate::lexer::Token::Eof))
            .collect();
        let result = expr_parser().parse(&tokens).into_result();
        assert_eq!(
            result.unwrap(),
            crate::ast::Expr::PartialCall(
                Box::new(crate::ast::Expr::Ident("add".to_string())),
                vec![Some(crate::ast::Expr::Number(1)), None],
                1
            )
        );
    }

    #[test]
    fn test_inline_ternary_parser() {
        use crate::lexer::Lexer;
        let input = "1 if True else 2";
        let mut lexer = Lexer::new(input);
        let tokens: Vec<_> = lexer
            .tokenize()
            .into_iter()
            .filter(|t| !matches!(t, Token::Newline | Token::Eof))
            .collect();
        let result = expr_parser().parse(&tokens).into_result();
        assert_eq!(
            result.unwrap(),
            Expr::Ternary(
                Box::new(Expr::Bool(true)),
                Box::new(Expr::Number(1)),
                Box::new(Expr::Number(2))
            )
        );
    }

    #[test]
    fn test_fstring_parser_logic() {
        let input = "hello {name}!";
        let ast = crate::parser::parse_fstring(input);
        assert_eq!(ast.len(), 3);
        assert_eq!(ast[0], Expr::String("hello ".to_string()));
        assert_eq!(ast[1], Expr::Ident("name".to_string()));
        assert_eq!(ast[2], Expr::String("!".to_string()));
    }
}
