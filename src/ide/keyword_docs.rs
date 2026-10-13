//! Hover documentation for Craw's keywords, shown when the cursor is on a
//! keyword token (identifiers are handled by the semantic model).

use crate::lexer::Token;

/// `(keyword, one-line syntax, explanation)`.
const DOCS: &[(&str, &str, &str)] = &[
    (
        "def",
        "def name(params) -> Type:",
        "Defines a function. Parameters may carry defaults (`b=2`), a `*rest` vararg, and `name: Type` annotations. A one-expression body can be written `def name(params) = expr`.",
    ),
    (
        "gen",
        "gen name(params):",
        "Defines a generator: a function whose `yield`s produce a lazy sequence.",
    ),
    (
        "return",
        "return expr",
        "Returns a value from the enclosing function.",
    ),
    (
        "yield",
        "yield expr",
        "Produces the next value of the enclosing generator.",
    ),
    (
        "if",
        "if cond:",
        "Runs the block when `cond` is truthy. As an expression: `if cond then a else b`.",
    ),
    (
        "else",
        "else:",
        "The branch taken when the preceding `if` condition was falsy. `else if` chains further conditions.",
    ),
    (
        "then",
        "if cond then a else b",
        "Separates the condition from the result in an expression-form `if`.",
    ),
    (
        "while",
        "while cond:",
        "Repeats the block while `cond` is truthy.",
    ),
    (
        "for",
        "for pattern in iterable:",
        "Iterates over `iterable`, destructuring each element with `pattern`.",
    ),
    (
        "in",
        "x in xs",
        "Membership test, or the separator in `for x in xs`.",
    ),
    ("break", "break", "Exits the innermost loop."),
    (
        "continue",
        "continue",
        "Skips to the next iteration of the innermost loop.",
    ),
    (
        "match",
        "match expr:",
        "Pattern matching: the first `case` whose pattern (and optional guard) matches runs.",
    ),
    (
        "case",
        "case pattern:",
        "One arm of a `match`. Patterns can destructure data classes, tuples and lists.",
    ),
    (
        "data",
        "data Name(field, field: Type, field=default)",
        "Declares an immutable data class with named fields and a generated constructor.",
    ),
    (
        "class",
        "class Name(args) extends Base with Trait:",
        "Declares a class with methods.",
    ),
    (
        "extends",
        "class A extends B:",
        "Names a class's superclass.",
    ),
    (
        "with",
        "class A with Trait:",
        "Lists traits a class implements.",
    ),
    (
        "struct",
        "struct Name:",
        "Declares a Rust-backed struct with typed fields.",
    ),
    (
        "trait",
        "trait Name:",
        "Declares a trait (a set of method signatures).",
    ),
    (
        "impl",
        "impl Trait for Type:",
        "Implements a trait's methods for a type.",
    ),
    (
        "template",
        "template name(args):",
        "Defines a template, expanded by textual substitution at each use.",
    ),
    (
        "macro",
        "macro name(args):",
        "Defines a macro whose invocations are expanded at compile time.",
    ),
    (
        "operator",
        "operator symbol",
        "Declares a custom infix operator.",
    ),
    (
        "lambda",
        "x => expr",
        "An anonymous function. The usual spelling is `x => expr`; `(a, b) => expr` takes several parameters.",
    ),
    (
        "where",
        "expr where:",
        "Evaluates `expr` with the bindings of the indented block in scope.",
    ),
    (
        "from",
        "from path import names",
        "Imports names from a module, or begins a query (`from x in xs ...`).",
    ),
    ("import", "import path", "Imports a module."),
    (
        "use",
        "use path;",
        "A raw Rust `use` declaration passed through to the generated code.",
    ),
    ("as", "import x as y", "Renames an import."),
    (
        "global",
        "global name",
        "Assignments to `name` in this function rebind the module-level variable.",
    ),
    (
        "nonlocal",
        "nonlocal name",
        "Assignments to `name` rebind the variable of the nearest enclosing function.",
    ),
    (
        "select",
        "from x in xs where cond select expr",
        "The projection of a query expression.",
    ),
    (
        "orderby",
        "orderby key ascending",
        "Sorts a query's results by `key`.",
    ),
    (
        "ascending",
        "orderby key ascending",
        "Sort order for `orderby` (the default).",
    ),
    (
        "descending",
        "orderby key descending",
        "Reverse sort order for `orderby`.",
    ),
    ("not", "not x", "Boolean negation."),
    ("and", "a and b", "Short-circuiting boolean and."),
    ("or", "a or b", "Short-circuiting boolean or."),
    ("is", "a is b", "Identity comparison."),
    (
        "copyclosure",
        "copyclosure def f():",
        "Makes the function capture its environment by copy.",
    ),
    (
        "addpattern",
        "addpattern def f(pattern):",
        "Adds another pattern-matched clause to an existing function.",
    ),
    ("to", "to", "Reserved range keyword."),
    ("until", "until", "Reserved range keyword."),
    (
        "rust",
        "rust:",
        "An inline block of raw Rust, copied verbatim into the generated program.",
    ),
];

/// Whether a token is a keyword (so hovering it should show keyword docs).
pub fn is_keyword_token(t: &Token) -> bool {
    matches!(
        t,
        Token::Def
            | Token::Gen
            | Token::Return
            | Token::Yield
            | Token::If
            | Token::Else
            | Token::Then
            | Token::While
            | Token::For
            | Token::In
            | Token::Break
            | Token::Continue
            | Token::Match
            | Token::Case
            | Token::Data
            | Token::Class
            | Token::Extends
            | Token::With
            | Token::Struct
            | Token::Trait
            | Token::Impl
            | Token::Template
            | Token::Macro
            | Token::OperatorKeyword
            | Token::Lambda
            | Token::Where
            | Token::From
            | Token::Import
            | Token::Use
            | Token::As
            | Token::Global
            | Token::Nonlocal
            | Token::Select
            | Token::OrderBy
            | Token::Ascending
            | Token::Descending
            | Token::Not
            | Token::And
            | Token::Or
            | Token::Is
            | Token::Copyclosure
            | Token::Addpattern
            | Token::To
            | Token::Until
            | Token::Rust
    )
}

/// Hover markdown for `keyword`, if documented.
pub fn keyword_hover(keyword: &str) -> Option<String> {
    DOCS.iter()
        .find(|(k, _, _)| *k == keyword)
        .map(|(_, syntax, doc)| format!("```craw\n{syntax}\n```\n\n---\n{doc}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_keyword_in_the_lexer_table_is_documented() {
        for kw in [
            "def",
            "return",
            "if",
            "else",
            "data",
            "class",
            "extends",
            "with",
            "match",
            "case",
            "in",
            "for",
            "copyclosure",
            "addpattern",
            "where",
            "global",
            "nonlocal",
            "lambda",
            "struct",
            "trait",
            "impl",
            "from",
            "select",
            "orderby",
            "ascending",
            "descending",
            "import",
            "yield",
            "while",
            "break",
            "continue",
            "then",
            "not",
            "and",
            "or",
            "is",
            "operator",
            "template",
            "macro",
            "to",
            "until",
            "gen",
            "use",
            "as",
        ] {
            assert!(keyword_hover(kw).is_some(), "no docs for `{kw}`");
            let (tokens, _) = crate::lexer::Lexer::new(&format!("{kw}\n")).tokenize_spanned();
            assert!(
                is_keyword_token(&tokens[0]),
                "`{kw}` lexed as {:?}",
                tokens[0]
            );
        }
    }
}
