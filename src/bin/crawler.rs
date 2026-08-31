use chumsky::Parser;
use craw::interpreter::{ControlFlow, Scope, eval_expr, exec_stmt};
use craw::lexer::Lexer;
use craw::parser;
use craw::runtime::CrawValue;
use craw::transpiler;
use rustyline::Editor;
use rustyline::error::ReadlineError;
use rustyline::highlight::{CmdKind, Highlighter};
use rustyline::hint::Hinter;
use rustyline::validate::{ValidationContext, ValidationResult, Validator};
use rustyline_derive::Helper;
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Helper)]
struct CrawlerHelper {
    globals: Rc<RefCell<HashMap<String, CrawValue>>>,
}

impl Highlighter for CrawlerHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        let mut highlighted = String::new();
        let mut pos = 0;
        let chars: Vec<char> = line.chars().collect();
        while pos < chars.len() {
            let ch = chars[pos];
            if ch.is_whitespace() {
                highlighted.push(ch);
                pos += 1;
            } else if ch == '#' {
                highlighted.push_str("\x1b[32m"); // Green for comments
                while pos < chars.len() {
                    highlighted.push(chars[pos]);
                    pos += 1;
                }
                highlighted.push_str("\x1b[0m");
            } else if ch.is_alphabetic() || ch == '_' {
                let start = pos;
                while pos < chars.len() && (chars[pos].is_alphanumeric() || chars[pos] == '_') {
                    pos += 1;
                }
                let word: String = chars[start..pos].iter().collect();
                let is_keyword = match word.as_str() {
                    "def" | "return" | "if" | "else" | "data" | "class" | "match" | "case"
                    | "in" | "for" | "while" | "break" | "yield" | "is" | "not" | "and" | "or"
                    | "None" | "True" | "False" | "as" | "use" | "from" | "import" | "global"
                    | "nonlocal" | "where" | "matchfor" | "copyclosure" | "addpattern" => true,
                    _ => false,
                };
                if is_keyword {
                    highlighted.push_str("\x1b[1;35m"); // Bold Magenta for keywords
                } else if self.globals.borrow().get(&word).is_some() {
                    highlighted.push_str("\x1b[1;36m"); // Bold Cyan for defined names
                }
                highlighted.push_str(&word);
                highlighted.push_str("\x1b[0m");
            } else if ch.is_digit(10) {
                highlighted.push_str("\x1b[33m"); // Yellow for numbers
                while pos < chars.len() && (chars[pos].is_digit(10) || chars[pos] == '.') {
                    highlighted.push(chars[pos]);
                    pos += 1;
                }
                highlighted.push_str("\x1b[0m");
            } else if ch == '"' || ch == '\'' {
                highlighted.push_str("\x1b[31m"); // Red for strings
                let quote = ch;
                highlighted.push(ch);
                pos += 1;
                while pos < chars.len() {
                    let c = chars[pos];
                    highlighted.push(c);
                    pos += 1;
                    if c == quote {
                        break;
                    }
                    if c == '\\' && pos < chars.len() {
                        highlighted.push(chars[pos]);
                        pos += 1;
                    }
                }
                highlighted.push_str("\x1b[0m");
            } else {
                highlighted.push(ch);
                pos += 1;
            }
        }
        Cow::Owned(highlighted)
    }

    fn highlight_char(&self, _line: &str, _pos: usize, _kind: CmdKind) -> bool {
        true
    }
}

impl Hinter for CrawlerHelper {
    type Hint = String;
    fn hint(&self, line: &str, pos: usize, _ctx: &rustyline::Context<'_>) -> Option<String> {
        if line.is_empty() || pos < line.len() {
            return None;
        }
        let last_word = match line[..pos].rfind(|c: char| !c.is_alphanumeric() && c != '_') {
            Some(i) => &line[i + 1..pos],
            None => &line[..pos],
        };
        if last_word.len() < 1 {
            return None;
        }

        let globals = self.globals.borrow();
        for name in globals.keys() {
            if name.starts_with(last_word) && name.len() > last_word.len() {
                return Some(name[last_word.len()..].to_string());
            }
        }
        None
    }
}

impl Validator for CrawlerHelper {
    fn validate(&self, ctx: &mut ValidationContext) -> rustyline::Result<ValidationResult> {
        let input = ctx.input();
        if input.ends_with(':') {
            return Ok(ValidationResult::Incomplete);
        }
        let mut balance = 0i32;
        for ch in input.chars() {
            match ch {
                '(' | '[' | '{' => balance += 1,
                ')' | ']' | '}' => balance -= 1,
                _ => {}
            }
        }
        if balance > 0 {
            Ok(ValidationResult::Incomplete)
        } else {
            Ok(ValidationResult::Valid(None))
        }
    }
}

impl rustyline::completion::Completer for CrawlerHelper {
    type Candidate = String;
    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<String>)> {
        let (start, word) = match line[..pos].rfind(|c: char| !c.is_alphanumeric() && c != '_') {
            Some(i) => (i + 1, &line[i + 1..pos]),
            None => (0, &line[..pos]),
        };

        let mut candidates = Vec::new();
        let globals = self.globals.borrow();
        for name in globals.keys() {
            if name.starts_with(word) {
                candidates.push(name.clone());
            }
        }
        candidates.sort();
        Ok((start, candidates))
    }
}

fn get_history_path() -> Option<PathBuf> {
    home::home_dir().map(|mut path| {
        path.push(".crawler_history");
        path
    })
}

fn setup_builtins(scope: &mut Scope) {
    let builtins = vec![
        "abs",
        "add",
        "sub",
        "mul",
        "div",
        "mod",
        "sqrt",
        "pow",
        "approx",
        "eq",
        "ne",
        "lt",
        "le",
        "gt",
        "ge",
        "print",
        "println",
        "range",
        "enumerate",
        "list",
        "count",
        "len",
        "str",
        "fmap",
        "filter",
        "reduce",
        "orderby",
        "flatten",
        "any",
        "all",
        "sum",
        "product",
        "hcat",
        "vcat",
        "hvcat",
        "in",
        "notin",
        "next",
        "not",
        "to",
        "until",
        "÷",
    ];
    let mut g = scope.globals.borrow_mut();
    for b in builtins {
        g.insert(b.to_string(), CrawValue::Builtin(b.to_string()));
    }
    g.insert(
        "exit".to_string(),
        CrawValue::Closure(Rc::new(|_| {
            std::process::exit(0);
        })),
    );
}

fn craw_type_name(v: &CrawValue) -> &'static str {
    match v {
        CrawValue::Int(_) => "Int",
        CrawValue::Float(_) => "Float",
        CrawValue::String(_) => "String",
        CrawValue::Bool(_) => "Bool",
        CrawValue::None => "None",
        CrawValue::Data(..) => "Data",
        CrawValue::Closure(_) => "Closure",
        CrawValue::List(_) => "List",
        CrawValue::Tuple(_) => "Tuple",
        CrawValue::Dict(_) => "Dict",
        CrawValue::Set(_) => "Set",
        CrawValue::Frozenset(_) => "Frozenset",
        CrawValue::Multiset(_) => "Multiset",
        CrawValue::LazyList(_) => "LazyList",
        CrawValue::Builtin(_) => "Builtin",
        CrawValue::Expected(_) => "Expected",
        CrawValue::Slice(..) => "Slice",
        CrawValue::Generator(..) => "Generator",
        CrawValue::Array(_) => "Array",
        CrawValue::Recursive(_) => "Recursive",
        CrawValue::Native(_) => "Native",
        CrawValue::Formula(..) => "Formula",
    }
}

fn print_help() {
    println!(
        r#"crawler {}
Interactive REPL and interpreter for the Craw language.

USAGE:
    crawler [file]

FLAGS:
    -h, --help      Print help information
    -v, --version   Print version information"#,
        env!("CARGO_PKG_VERSION")
    );
}

fn print_version() {
    println!("crawler {}", env!("CARGO_PKG_VERSION"));
}

fn main() -> rustyline::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--version" || arg == "-v") {
        print_version();
        return Ok(());
    }

    // If a filename is provided, interpret the file and exit.
    if args.len() > 1 {
        let filename = &args[1];
        let content = std::fs::read_to_string(filename).expect("Failed to read file");

        let mut scope = Scope::new();
        setup_builtins(&mut scope);

        let mut lexer = Lexer::new(&content);
        let tokens = lexer.tokenize();
        let (ast, errors) = parser::parser().parse(&tokens).into_output_errors();

        if !errors.is_empty() {
            for err in errors {
                eprintln!("\x1b[31mParse error: {:?}\x1b[0m", err);
            }
            std::process::exit(1);
        } else if let Some(stmts) = ast {
            for stmt in stmts {
                if let Err(e) = exec_stmt(&stmt, &mut scope) {
                    eprintln!("\x1b[31mError: {}\x1b[0m", e);
                    std::process::exit(1);
                }
            }
        }
        return Ok(());
    }

    // Explicitly disable any lingering mouse tracking modes
    // ?1000: SET_ANY_EVENT_MOUSE
    // ?1002: SET_DRAG_EVENT_MOUSE
    // ?1003: SET_ANY_EVENT_MOUSE
    // ?1006: SET_SGR_EXT_MODE_MOUSE
    print!("\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l");
    let _ = io::stdout().flush();

    println!(
        "\x1b[1;36mCraw Interpreter (Crawler) {}\x1b[0m",
        env!("CARGO_PKG_VERSION")
    );
    println!("Type 'exit()' or Ctrl-D to exit.");

    let config = rustyline::Config::builder()
        .history_ignore_space(true)
        .completion_type(rustyline::CompletionType::List)
        .check_cursor_position(false) // Prevents misinterpretation of cursor queries
        .build();

    let h = CrawlerHelper {
        globals: Rc::new(RefCell::new(HashMap::new())),
    };

    let mut scope = Scope {
        globals: h.globals.clone(),
        locals: vec![],
    };
    setup_builtins(&mut scope);

    let mut rl = Editor::with_config(config)?;
    rl.set_helper(Some(h));

    // Load history
    let history_path = get_history_path();
    if let Some(ref path) = history_path {
        if rl.load_history(path).is_err() {
            // No history yet or error loading
        }
    }

    loop {
        let readline = rl.readline(">>> ");
        match readline {
            Ok(line) => {
                if line.trim().is_empty() {
                    continue;
                }

                let trimmed = line.trim();
                if trimmed.starts_with(':') {
                    let parts: Vec<&str> = trimmed.splitn(2, ' ').collect();
                    let cmd = parts[0];
                    match cmd {
                        ":help" => {
                            println!("Crawler REPL Commands:");
                            println!("  :env             Show user-defined active bindings");
                            println!("  :ast <expr>      Show the abstract syntax tree for an expression");
                            println!("  :type <expr>     Show the runtime type of an expression's value");
                            println!(
                                "  :transpile <code> Show the Rust code the transpiler would generate"
                            );
                            println!("  :help            Show this help info");
                        }
                        ":env" => {
                            let globals = scope.globals.borrow();
                            let builtins = [
                                "abs", "add", "sub", "mul", "div", "mod", "sqrt", "pow", "approx",
                                "eq", "ne", "lt", "le", "gt", "ge", "print", "println", "range",
                                "enumerate", "list", "count", "len", "str", "fmap", "filter",
                                "reduce", "orderby", "flatten", "any", "all", "sum", "product",
                                "hcat", "vcat", "hvcat", "in", "notin", "next", "not", "to",
                                "until", "exit", "÷",
                            ];
                            let mut user_vars: Vec<(&String, &CrawValue)> = globals
                                .iter()
                                .filter(|(k, _)| !builtins.contains(&k.as_str()))
                                .collect();
                            user_vars.sort_by_key(|(k, _)| *k);

                            if user_vars.is_empty() {
                                println!("No user-defined variables in current session.");
                            } else {
                                for (k, v) in user_vars {
                                    println!("  {} = {}", k, v);
                                }
                            }
                        }
                        ":ast" => {
                            if parts.len() < 2 || parts[1].trim().is_empty() {
                                println!("Usage: :ast <expr/stmt>");
                            } else {
                                let code = parts[1];
                                let mut lexer = Lexer::new(code);
                                let tokens = lexer.tokenize();
                                let (ast, errors) = parser::parser().parse(&tokens).into_output_errors();
                                if !errors.is_empty() {
                                    for err in errors {
                                        println!("Parse error: {:?}", err);
                                    }
                                } else if let Some(stmts) = ast {
                                    for stmt in stmts {
                                        println!("{:#?}", stmt);
                                    }
                                }
                            }
                        }
                        ":type" => {
                            if parts.len() < 2 || parts[1].trim().is_empty() {
                                println!("Usage: :type <expr>");
                            } else {
                                let code = parts[1];
                                let mut lexer = Lexer::new(code);
                                let tokens = lexer.tokenize();
                                let (ast, errors) =
                                    parser::parser().parse(&tokens).into_output_errors();
                                if !errors.is_empty() {
                                    for err in errors {
                                        println!("Parse error: {:?}", err);
                                    }
                                } else if let Some(stmts) = ast
                                    && stmts.len() == 1
                                    && let craw::ast::Stmt::Expr(expr) = &stmts[0]
                                {
                                    match eval_expr(expr, &mut scope) {
                                        Ok(v) => println!("{}", craw_type_name(&v)),
                                        Err(e) => eprintln!("\x1b[31mError: {}\x1b[0m", e),
                                    }
                                } else {
                                    println!("Usage: :type <expr>");
                                }
                            }
                        }
                        ":transpile" => {
                            if parts.len() < 2 || parts[1].trim().is_empty() {
                                println!("Usage: :transpile <code>");
                            } else {
                                let code = parts[1];
                                let mut lexer = Lexer::new(code);
                                let tokens = lexer.tokenize();
                                let (ast, errors) =
                                    parser::parser().parse(&tokens).into_output_errors();
                                if !errors.is_empty() {
                                    for err in errors {
                                        println!("Parse error: {:?}", err);
                                    }
                                } else if let Some(stmts) = ast {
                                    println!("{}", transpiler::transpile(&stmts));
                                }
                            }
                        }
                        _ => {
                            println!("Unknown REPL command: {}. Type :help for info.", cmd);
                        }
                    }
                    continue;
                }

                rl.add_history_entry(line.as_str())?;

                let input = if !line.ends_with('\n') {
                    format!("{}\n", line)
                } else {
                    line
                };

                let mut lexer = Lexer::new(&input);
                let tokens = lexer.tokenize();

                // Try to parse as an expression first for convenience (print result)
                let expr_result = parser::parser().parse(&tokens).into_output_errors();
                if let (Some(ast), errors) = expr_result {
                    if errors.is_empty() && ast.len() == 1 {
                        if let craw::ast::Stmt::Expr(expr) = &ast[0] {
                            match eval_expr(expr, &mut scope) {
                                Ok(result) => {
                                    if !matches!(result, CrawValue::None) {
                                        println!("\x1b[1;33mOut: \x1b[0m{}", result);
                                    }
                                }
                                Err(e) => {
                                    eprintln!("\x1b[31mError: {}\x1b[0m", e);
                                }
                            }
                            continue;
                        }
                    }
                }

                // Otherwise parse as statements
                let (ast, errors) = parser::parser().parse(&tokens).into_output_errors();
                if !errors.is_empty() {
                    for err in errors {
                        eprintln!("\x1b[31mParse error: {:?}\x1b[0m", err);
                    }
                } else if let Some(stmts) = ast {
                    for stmt in stmts {
                        match exec_stmt(&stmt, &mut scope) {
                            Ok(ControlFlow::Return(v)) => {
                                println!("\x1b[1;33mReturn: \x1b[0m{}", v);
                            }
                            Ok(_) => {}
                            Err(e) => {
                                eprintln!("\x1b[31mError: {}\x1b[0m", e);
                                break;
                            }
                        }
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("Interrupted");
            }
            Err(ReadlineError::Eof) => {
                println!("Goodbye!");
                break;
            }
            Err(err) => {
                println!("Error: {:?}", err);
                break;
            }
        }
    }

    if let Some(path) = history_path {
        let _ = rl.save_history(&path);
    }
    Ok(())
}
