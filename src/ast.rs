#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Int,
    String,
    Custom(String),
    Generic(String, Vec<Type>),
    Dynamic,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CallStyle {
    Standard,
    Star,
    DoubleStar,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PipeData {
    pub style: CallStyle,
    pub none_aware: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum QueryClause {
    Where(Expr),
    OrderBy(Expr, bool), // bool: true for ascending, false for descending
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(i64),
    Float(f64),
    String(String),
    Bool(bool),
    None,
    Ident(String),
    Call(Box<Expr>, Vec<Expr>),
    BroadcastCall(Box<Expr>, Vec<Expr>),
    BinaryOp(Box<Expr>, String, Box<Expr>),
    Lambda(Vec<String>, Box<Expr>, usize),
    Pipe(Box<Expr>, PipeData, Box<Expr>), // left |> right
    Compose(Box<Expr>, PipeData, Box<Expr>, usize), // f .. g
    NoneCoalesce(Box<Expr>, Box<Expr>),   // left ?? right
    PartialCall(Box<Expr>, Vec<Option<Expr>>, usize),
    List(Vec<Expr>),
    Dict(Vec<(Expr, Expr)>),
    Index(Box<Expr>, Box<Expr>),
    Attribute(Box<Expr>, String),
    AttributePartial(String),
    ImplicitLambda(Box<Expr>, usize),
    Where(Box<Expr>, Vec<Stmt>, usize),
    Set(Vec<Expr>),
    Frozenset(Vec<Expr>),
    Multiset(Vec<Expr>),
    Range(Box<Expr>, Box<Expr>),
    Gather(Box<Expr>, String),
    LazyList(Vec<Expr>, usize),
    IndexPartial(Box<Expr>),
    OperatorFunction(String),
    Passthrough(String),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Tuple(Vec<Expr>),
    Placeholder,
    Splat(Box<Expr>),
    MacroCall(String, Vec<Expr>),
    Slice(Option<Box<Expr>>, Option<Box<Expr>>, Option<Box<Expr>>),
    Comprehension(
        Box<Expr>,
        Box<Pattern>,
        Box<Expr>,
        bool,
        usize,
        Option<Box<Expr>>,
    ),
    Hcat(Vec<Expr>),
    Vcat(Vec<Expr>),
    Formula(Box<Expr>, Box<Expr>),
    Shell(String),
    Query {
        from: String,
        in_expr: Box<Expr>,
        clauses: Vec<QueryClause>,
        select: Box<Expr>,
        id: usize,
    },
    FString(Vec<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Var(String, Option<Type>),
    Const(Expr),
    Data(String, Vec<Pattern>),
    Wildcard,
    View(Box<Expr>, Box<Pattern>),
    StringSplit(String, String, bool),
    Tuple(Vec<Pattern>),
    Rest(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Assign(Pattern, Expr),
    Expr(Expr),
    FunctionDef {
        name: Vec<String>,
        args: Vec<(Pattern, Option<Expr>)>,
        vararg: Option<String>,
        return_type: Option<Type>,
        body: Vec<Stmt>,
        is_copyclosure: bool,
        is_addpattern: bool,
        is_generator: bool,
        id: usize,
    },
    Return(Expr),
    If(Expr, Vec<Stmt>),
    DataDef(String, Vec<(String, Option<Type>, Option<Expr>)>, usize),
    StructDef(String, Vec<(String, Type)>, usize),
    TraitDef(String, Vec<Stmt>, usize),
    ImplBlock(Option<Type>, Type, Vec<Stmt>, usize),
    NativeImport(Vec<String>, Vec<String>),
    Match(Expr, Vec<(Pattern, Option<Expr>, Vec<Stmt>)>),
    MatchFor(Pattern, Expr, Vec<Stmt>),
    IndexAssign(Expr, Expr, Expr),
    AttributeAssign(Expr, String, Expr),
    Operator(String),
    Yield(Expr),
    While(Expr, Vec<Stmt>),
    Break,
    Passthrough(String),
    Global(Vec<String>),
    Nonlocal(Vec<String>),
    Use(String),
    TemplateDef(String, Vec<String>, Vec<Stmt>, usize),
    MacroDef {
        name: String,
        args: Vec<String>,
        body: Vec<Stmt>,
    },
    ClassDef {
        name: String,
        args: Vec<String>,
        superclass: Option<(String, Vec<Expr>)>,
        traits: Vec<String>,
        body: Vec<Stmt>,
        id: usize,
    },
    MacroBlock {
        name: String,
        args: Vec<Expr>,
        body: Vec<Stmt>,
        branches: Vec<(String, Vec<Expr>, Vec<Stmt>)>,
        /// Index of the invocation's leading token in the parsed token
        /// stream, used to point diagnostics (arity mismatches, unknown
        /// macro names) at the invocation site.
        token_pos: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_functional_ast_nodes() {
        let _lambda = Expr::Lambda(
            vec!["x".to_string()],
            Box::new(Expr::Ident("x".to_string())),
            0,
        );
        let pipe_data = PipeData {
            style: CallStyle::Standard,
            none_aware: false,
        };
        let _pipe = Expr::Pipe(
            Box::new(Expr::Ident("x".to_string())),
            pipe_data.clone(),
            Box::new(Expr::Ident("f".to_string())),
        );
        let _compose = Expr::Compose(
            Box::new(Expr::Ident("f".to_string())),
            pipe_data,
            Box::new(Expr::Ident("g".to_string())),
            0,
        );
        let _data_def = Stmt::DataDef(
            "User".to_string(),
            vec![("name".to_string(), None, None)],
            0,
        );
        let _struct_def = Stmt::StructDef(
            "Point".to_string(),
            vec![("x".to_string(), Type::Int), ("y".to_string(), Type::Int)],
            1,
        );
    }

    #[test]
    fn test_collection_ast() {
        let _list = Expr::List(vec![Expr::Number(1)]);
        let _dict = Expr::Dict(vec![(Expr::String("a".to_string()), Expr::Number(1))]);
    }

    #[test]
    fn test_query_and_formula() {
        let formula = Expr::Formula(
            Box::new(Expr::Ident("x".to_string())),
            Box::new(Expr::BinaryOp(
                Box::new(Expr::Ident("x".to_string())),
                "+".to_string(),
                Box::new(Expr::Number(1)),
            )),
        );
        assert_eq!(formula, formula.clone());

        let query = Expr::Query {
            from: "x".to_string(),
            in_expr: Box::new(Expr::Ident("data".to_string())),
            clauses: vec![
                QueryClause::Where(Expr::BinaryOp(
                    Box::new(Expr::Ident("x".to_string())),
                    ">".to_string(),
                    Box::new(Expr::Number(0)),
                )),
                QueryClause::OrderBy(Expr::Ident("x".to_string()), true),
            ],
            select: Box::new(Expr::Ident("x".to_string())),
            id: 42,
        };
        assert_eq!(query, query.clone());
    }
}
