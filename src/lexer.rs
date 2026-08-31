#[derive(Debug, PartialEq, Clone)]
pub enum Token {
    Ident(String),
    Number(i64),
    Float(f64),
    String(String),
    Assign,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Equal,
    NotEqual,
    LessEqual,
    GreaterEqual,
    LessThan,
    GreaterThan,
    Def,
    Return,
    If,
    Else,
    Indent,
    Dedent,
    Newline,
    Colon,
    Comma,
    LParen,
    RParen,
    Eof,
    Data,
    Class,
    Extends,
    With,
    Arrow,
    ThinArrow,
    PipeForward,
    PipeBackward,
    DotDot,
    ComposeForward,
    ComposeBackward,
    Match,
    Case,
    In,
    For,
    NoneCoalesce,
    Partial,
    Pipe,
    Dot,
    DotPlus,
    DotMinus,
    DotStar,
    DotSlash,
    DotPercent,
    DotEqual,
    DotNotEqual,
    DotLess,
    DotLessEqual,
    DotGreater,
    DotGreaterEqual,
    Question,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    PipeForwardStar,
    PipeForwardDoubleStar,
    PipeForwardNone,
    PipeForwardNoneStar,
    PipeForwardNoneDoubleStar,
    PipeBackwardStar,
    PipeBackwardDoubleStar,
    PipeBackwardNone,
    PipeBackwardNoneStar,
    PipeBackwardNoneDoubleStar,
    ComposeStar,
    ComposeDoubleStar,
    ComposeNone,
    BacktickString(String),
    Copyclosure,
    Addpattern,
    Where,
    Global,
    Nonlocal,
    Semicolon,
    Lambda,
    Operator(String),
    OperatorKeyword,
    Struct,
    Trait,
    Impl,
    From,
    Import,
    Yield,
    While,
    Break,
    Then,
    At,
    PassthroughStart,
    Not,
    And,
    Or,
    Is,
    PlusAssign,
    MinusAssign,
    StarAssign,
    SlashAssign,
    Power,
    PowerAssign,
    Template,
    Select,
    OrderBy,
    Ascending,
    Descending,
    Tilde,
    Macro,
    FStringRaw(String),
    To,
    Until,
    Gen,
    Use,
    As,
    Rust,
}

pub struct Lexer {
    input: Vec<char>,
    pos: usize,
    indent_stack: Vec<usize>,
    pending_tokens: Vec<Token>,
    nesting_level: usize,
    current_line_indent: usize,
}

impl Lexer {
    pub fn new(input: &str) -> Self {
        Self {
            input: input.chars().collect(),
            pos: 0,
            indent_stack: vec![0],
            pending_tokens: Vec::new(),
            nesting_level: 0,
            current_line_indent: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        if self.pos < self.input.len() {
            Some(self.input[self.pos])
        } else {
            None
        }
    }

    fn peek_next(&self) -> Option<char> {
        if self.pos + 1 < self.input.len() {
            Some(self.input[self.pos + 1])
        } else {
            None
        }
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.peek();
        self.pos += 1;
        ch
    }

    fn match_sequence(&mut self, seq: &[char]) -> bool {
        if self.pos + seq.len() > self.input.len() {
            return false;
        }
        for (i, &ch) in seq.iter().enumerate() {
            if self.input[self.pos + i] != ch {
                return false;
            }
        }
        self.pos += seq.len();
        true
    }

    fn skip_whitespace_on_line(&mut self) {
        while let Some(ch) = self.peek() {
            if ch == ' ' || ch == '\t' || ch == '\r' {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn read_ident_or_keyword(&mut self) -> Token {
        let mut result = String::new();
        while let Some(ch) = self.peek() {
            if ch.is_alphanumeric() || ch == '_' {
                result.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        match result.as_str() {
            "def" => Token::Def,
            "return" => Token::Return,
            "if" => Token::If,
            "else" => Token::Else,
            "data" => Token::Data,
            "class" => Token::Class,
            "extends" => Token::Extends,
            "with" => Token::With,
            "match" => Token::Match,
            "case" => Token::Case,
            "in" => Token::In,
            "for" => Token::For,
            "copyclosure" => Token::Copyclosure,
            "addpattern" => Token::Addpattern,
            "where" => Token::Where,
            "global" => Token::Global,
            "nonlocal" => Token::Nonlocal,
            "lambda" => Token::Lambda,
            "struct" => Token::Struct,
            "trait" => Token::Trait,
            "impl" => Token::Impl,
            "from" => Token::From,
            "select" => Token::Select,
            "orderby" => Token::OrderBy,
            "ascending" => Token::Ascending,
            "descending" => Token::Descending,
            "import" => Token::Import,
            "yield" => Token::Yield,
            "while" => Token::While,
            "break" => Token::Break,
            "then" => Token::Then,
            "not" => Token::Not,
            "notin" => Token::Operator("notin".to_string()),
            "and" => Token::And,
            "or" => Token::Or,
            "is" => Token::Is,
            "operator" => Token::OperatorKeyword,
            "template" => Token::Template,
            "macro" => Token::Macro,
            "to" => Token::To,
            "until" => Token::Until,
            "gen" => Token::Gen,
            "use" => Token::Use,
            "as" => Token::As,
            _ => Token::Ident(result),
        }
    }

    fn read_number(&mut self) -> Token {
        let mut result = String::new();
        let mut is_float = false;
        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() {
                result.push(ch);
                self.advance();
            } else if ch == '.' && !is_float {
                // Peek ahead to see if next is digit
                if let Some(next) = self.input.get(self.pos + 1) {
                    if next.is_ascii_digit() {
                        is_float = true;
                        result.push(ch);
                        self.advance();
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        if is_float {
            Token::Float(result.parse().unwrap_or(0.0))
        } else {
            Token::Number(result.parse().unwrap_or(0))
        }
    }

    fn read_string(&mut self) -> Token {
        let quote = self.advance().unwrap(); // skip quote and remember it
        let mut result = String::new();
        while let Some(ch) = self.peek() {
            if ch == quote {
                self.advance();
                break;
            } else if ch == '\\' {
                self.advance();
                if let Some(next) = self.advance() {
                    match next {
                        'n' => result.push('\n'),
                        'r' => result.push('\r'),
                        't' => result.push('\t'),
                        _ => result.push(next),
                    }
                }
            } else {
                result.push(ch);
                self.advance();
            }
        }
        Token::String(result)
    }

    pub fn tokenize(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        let mut is_new_line = true;

        'outer: while self.pos < self.input.len() {
            if !self.pending_tokens.is_empty() {
                tokens.push(self.pending_tokens.remove(0));
                continue;
            }

            if is_new_line {
                let mut spaces = 0;
                while let Some(ch) = self.peek() {
                    if ch == ' ' {
                        spaces += 1;
                        self.advance();
                    } else if ch == '\t' {
                        spaces += 4;
                        self.advance();
                    } else {
                        break;
                    }
                }

                self.current_line_indent = spaces;

                if let Some(ch) = self.peek() {
                    if ch == '\n' || ch == '\r' {
                        self.advance();
                        continue;
                    } else if ch == '#' {
                        while let Some(c) = self.peek() {
                            if c == '\n' || c == '\r' {
                                break;
                            }
                            self.advance();
                        }
                        if let Some(c) = self.peek()
                            && (c == '\n' || c == '\r')
                        {
                            self.advance();
                        }
                        continue;
                    }
                } else {
                    break;
                }

                let current_indent = *self.indent_stack.last().unwrap();
                if self.nesting_level == 0 {
                    if spaces > current_indent {
                        self.indent_stack.push(spaces);
                        tokens.push(Token::Indent);
                    } else if spaces < current_indent {
                        while *self.indent_stack.last().unwrap() > spaces {
                            self.indent_stack.pop();
                            tokens.push(Token::Dedent);
                        }
                    }
                }

                if self.input.get(self.pos..).is_some_and(|s| {
                    s.starts_with(&['e', 'x', 't', 'e', 'r', 'n', ' '])
                        || s.starts_with(&['u', 's', 'e', ' '])
                        || (self.indent_stack.len() == 1
                            && spaces == 0
                            && s.starts_with(&['l', 'e', 't', ' ']))
                }) {
                    let mut line = String::new();
                    while let Some(ch) = self.peek() {
                        if ch == '\n' || ch == '\r' {
                            break;
                        }
                        line.push(ch);
                        self.advance();
                    }
                    tokens.push(Token::String(line));
                    tokens.push(Token::PassthroughStart);
                    is_new_line = false;
                    continue;
                }

                if self.input.get(self.pos..).is_some_and(|s| {
                    s.starts_with(&['f', 'n', ' ']) || s.starts_with(&['f', 'n', '('])
                }) {
                    let mut content = String::new();
                    let mut brace_depth = 0;
                    let mut first_brace_found = false;

                    enum CaptureState {
                        Normal,
                        InString,
                        InChar,
                        InLineComment,
                        InBlockComment,
                        InRawString(usize),
                    }
                    let mut state = CaptureState::Normal;
                    let mut escaped = false;

                    while let Some(ch) = self.peek() {
                        match state {
                            CaptureState::Normal => {
                                if ch == '/' && self.input.get(self.pos + 1) == Some(&'/') {
                                    state = CaptureState::InLineComment;
                                } else if ch == '/' && self.input.get(self.pos + 1) == Some(&'*') {
                                    state = CaptureState::InBlockComment;
                                } else if ch == '"' {
                                    state = CaptureState::InString;
                                } else if ch == '\'' {
                                    state = CaptureState::InChar;
                                } else if ch == 'r' {
                                    let mut i = 1;
                                    while self.input.get(self.pos + i) == Some(&'#') {
                                        i += 1;
                                    }
                                    if self.input.get(self.pos + i) == Some(&'"') {
                                        state = CaptureState::InRawString(i - 1);
                                    }
                                } else if ch == '{' {
                                    brace_depth += 1;
                                    first_brace_found = true;
                                } else if ch == '}' {
                                    brace_depth -= 1;
                                    if first_brace_found && brace_depth == 0 {
                                        content.push(ch);
                                        self.advance();
                                        break;
                                    }
                                }
                            }
                            CaptureState::InString => {
                                if escaped {
                                    escaped = false;
                                } else if ch == '\\' {
                                    escaped = true;
                                } else if ch == '"' {
                                    state = CaptureState::Normal;
                                }
                            }
                            CaptureState::InChar => {
                                if escaped {
                                    escaped = false;
                                } else if ch == '\\' {
                                    escaped = true;
                                } else if ch == '\'' {
                                    state = CaptureState::Normal;
                                }
                            }
                            CaptureState::InLineComment => {
                                if ch == '\n' {
                                    state = CaptureState::Normal;
                                }
                            }
                            CaptureState::InBlockComment => {
                                if ch == '*' && self.input.get(self.pos + 1) == Some(&'/') {
                                    content.push(ch);
                                    self.advance();
                                    content.push(self.peek().unwrap());
                                    self.advance();
                                    state = CaptureState::Normal;
                                    continue;
                                }
                            }
                            CaptureState::InRawString(depth) => {
                                if ch == '"' {
                                    let mut match_hashes = true;
                                    for i in 1..=depth {
                                        if self.input.get(self.pos + i) != Some(&'#') {
                                            match_hashes = false;
                                            break;
                                        }
                                    }
                                    if match_hashes {
                                        content.push(ch);
                                        self.advance();
                                        for _ in 0..depth {
                                            content.push(self.peek().unwrap());
                                            self.advance();
                                        }
                                        state = CaptureState::Normal;
                                        continue;
                                    }
                                }
                            }
                        }
                        content.push(ch);
                        self.advance();
                    }

                    tokens.push(Token::String(content));
                    tokens.push(Token::PassthroughStart);

                    while let Some(ch) = self.peek() {
                        if ch == ' ' || ch == '\t' || ch == '\r' {
                            self.advance();
                        } else if ch == '\n' {
                            tokens.push(Token::Newline);
                            self.advance();
                            is_new_line = true;
                            continue 'outer;
                        } else {
                            break;
                        }
                    }
                    is_new_line = false;
                    continue;
                }

                is_new_line = false;
            }

            self.skip_whitespace_on_line();

            if self
                .input
                .get(self.pos..)
                .is_some_and(|s| s.starts_with(&['r', 'u', 's', 't', ':']))
            {
                self.pos += 5;
                let block_base_indent = self.current_line_indent;

                // Skip rest of the line
                while let Some(ch) = self.peek() {
                    if ch == '\n' || ch == '\r' {
                        if ch == '\r' {
                            self.advance();
                            if self.peek() == Some('\n') {
                                self.advance();
                            }
                        } else {
                            self.advance();
                        }
                        break;
                    }
                    self.advance();
                }

                let mut block_content = String::new();
                let mut first_line_indent = None;

                loop {
                    let line_start_pos = self.pos;
                    let mut line_spaces = 0;
                    while let Some(ch) = self.peek() {
                        if ch == ' ' {
                            line_spaces += 1;
                            self.advance();
                        } else if ch == '\t' {
                            line_spaces += 4;
                            self.advance();
                        } else {
                            break;
                        }
                    }

                    if let Some(ch) = self.peek() {
                        if ch == '\n' || ch == '\r' {
                            block_content.push('\n');
                            if ch == '\r' {
                                self.advance();
                                if self.peek() == Some('\n') {
                                    self.advance();
                                }
                            } else {
                                self.advance();
                            }
                            continue;
                        }

                        if line_spaces > block_base_indent {
                            if first_line_indent.is_none() {
                                first_line_indent = Some(line_spaces);
                            }

                            let strip = first_line_indent.unwrap();
                            if line_spaces > strip {
                                for _ in 0..(line_spaces - strip) {
                                    block_content.push(' ');
                                }
                            }

                            while let Some(c) = self.peek() {
                                if c == '\n' || c == '\r' {
                                    if c == '\r' {
                                        self.advance();
                                        if self.peek() == Some('\n') {
                                            self.advance();
                                        }
                                    } else {
                                        self.advance();
                                    }
                                    break;
                                }

                                block_content.push(c);
                                self.advance();
                            }
                            block_content.push('\n');
                        } else {
                            self.pos = line_start_pos;
                            break;
                        }
                    } else {
                        break;
                    }
                }

                if block_content.ends_with('\n') {
                    block_content.pop();
                }

                tokens.push(Token::Rust);
                tokens.push(Token::Colon);
                tokens.push(Token::Newline);

                if let Some(indent) = first_line_indent {
                    tokens.push(Token::Indent);
                    self.indent_stack.push(indent);
                }

                tokens.push(Token::String(block_content));
                tokens.push(Token::PassthroughStart);

                if first_line_indent.is_some() {
                    self.indent_stack.pop();
                    tokens.push(Token::Dedent);
                }

                tokens.push(Token::Newline);
                is_new_line = true;
                continue;
            }

            let Some(ch) = self.peek() else {
                break;
            };

            if ch == '\n' {
                if self.nesting_level == 0 {
                    tokens.push(Token::Newline);
                }
                is_new_line = true;
                self.advance();
                continue;
            }

            if ch == 'f' && self.peek_next() == Some('"') {
                self.advance(); // skip 'f'
                self.advance(); // skip '"'
                let mut result = String::new();
                while let Some(c) = self.peek() {
                    if c == '"' {
                        self.advance();
                        break;
                    } else {
                        result.push(c);
                        self.advance();
                    }
                }
                tokens.push(Token::FStringRaw(result));
            } else if ch.is_alphabetic() || ch == '_' {
                tokens.push(self.read_ident_or_keyword());
            } else if ch.is_ascii_digit() {
                tokens.push(self.read_number());
            } else if ch == '"' || ch == '\'' {
                tokens.push(self.read_string());
            } else {
                match ch {
                    '=' => {
                        self.advance();
                        if self.peek() == Some('>') {
                            tokens.push(Token::Arrow);
                            self.advance();
                        } else if self.peek() == Some('=') {
                            tokens.push(Token::Equal);
                            self.advance();
                        } else {
                            tokens.push(Token::Assign);
                        }
                    }
                    '!' => {
                        self.advance();
                        if self.peek() == Some('=') {
                            tokens.push(Token::NotEqual);
                            self.advance();
                        }
                    }
                    '|' => {
                        self.advance();
                        if self.match_sequence(&['?', '*', '*', '>']) {
                            tokens.push(Token::PipeForwardNoneDoubleStar);
                        } else if self.match_sequence(&['?', '*', '>']) {
                            tokens.push(Token::PipeForwardNoneStar);
                        } else if self.match_sequence(&['?', '>']) {
                            tokens.push(Token::PipeForwardNone);
                        } else if self.match_sequence(&['*', '*', '>']) {
                            tokens.push(Token::PipeForwardDoubleStar);
                        } else if self.match_sequence(&['*', '>']) {
                            tokens.push(Token::PipeForwardStar);
                        } else if self.match_sequence(&['>']) {
                            tokens.push(Token::PipeForward);
                        } else {
                            tokens.push(Token::Pipe);
                        }
                    }
                    '<' => {
                        self.advance();
                        if self.match_sequence(&['*', '*', '?', '|']) {
                            tokens.push(Token::PipeBackwardNoneDoubleStar);
                        } else if self.match_sequence(&['*', '?', '|']) {
                            tokens.push(Token::PipeBackwardNoneStar);
                        } else if self.match_sequence(&['?', '|']) {
                            tokens.push(Token::PipeBackwardNone);
                        } else if self.match_sequence(&['*', '*', '|']) {
                            tokens.push(Token::PipeBackwardDoubleStar);
                        } else if self.match_sequence(&['*', '|']) {
                            tokens.push(Token::PipeBackwardStar);
                        } else if self.match_sequence(&['|']) {
                            tokens.push(Token::PipeBackward);
                        } else if self.peek() == Some('=') {
                            tokens.push(Token::LessEqual);
                            self.advance();
                        } else if self.peek() == Some('<') {
                            tokens.push(Token::ComposeBackward);
                            self.advance();
                        } else {
                            tokens.push(Token::LessThan);
                        }
                    }
                    '>' => {
                        self.advance();
                        if self.peek() == Some('=') {
                            tokens.push(Token::GreaterEqual);
                            self.advance();
                        } else if self.peek() == Some('>') {
                            tokens.push(Token::ComposeForward);
                            self.advance();
                        } else {
                            tokens.push(Token::GreaterThan);
                        }
                    }
                    '-' => {
                        self.advance();
                        if self.peek() == Some('>') {
                            tokens.push(Token::ThinArrow);
                            self.advance();
                        } else if self.peek() == Some('=') {
                            tokens.push(Token::MinusAssign);
                            self.advance();
                        } else {
                            tokens.push(Token::Minus);
                        }
                    }
                    '*' => {
                        self.advance();
                        if self.peek() == Some('=') {
                            tokens.push(Token::StarAssign);
                            self.advance();
                        } else if self.peek() == Some('*') {
                            self.advance();
                            if self.peek() == Some('=') {
                                tokens.push(Token::PowerAssign);
                                self.advance();
                            } else {
                                tokens.push(Token::Power);
                            }
                        } else {
                            tokens.push(Token::Star);
                        }
                    }
                    '/' => {
                        self.advance();
                        if self.peek() == Some('=') {
                            tokens.push(Token::SlashAssign);
                            self.advance();
                        } else {
                            tokens.push(Token::Slash);
                        }
                    }
                    '%' => {
                        tokens.push(Token::Percent);
                        self.advance();
                    }
                    '.' => {
                        self.advance();
                        if self.match_sequence(&['.', '*', '*', '>']) {
                            tokens.push(Token::ComposeDoubleStar);
                        } else if self.match_sequence(&['.', '*', '>']) {
                            tokens.push(Token::ComposeStar);
                        } else if self.match_sequence(&['.', '?', '>']) {
                            tokens.push(Token::ComposeNone);
                        } else if self.match_sequence(&['.']) {
                            tokens.push(Token::DotDot);
                        } else if self.peek() == Some('+') {
                            self.advance();
                            tokens.push(Token::DotPlus);
                        } else if self.peek() == Some('-') {
                            self.advance();
                            tokens.push(Token::DotMinus);
                        } else if self.peek() == Some('*') {
                            self.advance();
                            tokens.push(Token::DotStar);
                        } else if self.peek() == Some('/') {
                            self.advance();
                            tokens.push(Token::DotSlash);
                        } else if self.peek() == Some('%') {
                            self.advance();
                            tokens.push(Token::DotPercent);
                        } else if self.peek() == Some('=') {
                            self.advance();
                            if self.peek() == Some('=') {
                                self.advance();
                                tokens.push(Token::DotEqual);
                            } else {
                                self.pending_tokens.push(Token::Assign);
                                tokens.push(Token::Dot);
                            }
                        } else if self.peek() == Some('!') {
                            self.advance();
                            if self.peek() == Some('=') {
                                self.advance();
                                tokens.push(Token::DotNotEqual);
                            } else {
                                self.pending_tokens.push(Token::Operator("!".to_string()));
                                tokens.push(Token::Dot);
                            }
                        } else if self.peek() == Some('<') {
                            self.advance();
                            if self.peek() == Some('=') {
                                self.advance();
                                tokens.push(Token::DotLessEqual);
                            } else {
                                tokens.push(Token::DotLess);
                            }
                        } else if self.peek() == Some('>') {
                            self.advance();
                            if self.peek() == Some('=') {
                                self.advance();
                                tokens.push(Token::DotGreaterEqual);
                            } else {
                                tokens.push(Token::DotGreater);
                            }
                        } else {
                            tokens.push(Token::Dot);
                        }
                    }
                    '`' => {
                        self.advance();
                        let mut content = String::new();
                        while let Some(c) = self.peek() {
                            if c == '`' {
                                self.advance();
                                break;
                            }
                            content.push(c);
                            self.advance();
                        }
                        tokens.push(Token::BacktickString(content));
                    }
                    '?' => {
                        self.advance();
                        if self.peek() == Some('?') {
                            tokens.push(Token::NoneCoalesce);
                            self.advance();
                        } else {
                            tokens.push(Token::Question);
                        }
                    }
                    '$' => {
                        tokens.push(Token::Partial);
                        self.advance();
                    }
                    '+' => {
                        self.advance();
                        if self.peek() == Some('=') {
                            tokens.push(Token::PlusAssign);
                            self.advance();
                        } else {
                            tokens.push(Token::Plus);
                        }
                    }
                    ':' => {
                        tokens.push(Token::Colon);
                        self.advance();
                    }
                    ',' => {
                        tokens.push(Token::Comma);
                        self.advance();
                    }
                    ';' => {
                        tokens.push(Token::Semicolon);
                        self.advance();
                    }
                    '@' => {
                        tokens.push(Token::At);
                        self.advance();
                    }
                    '~' => {
                        tokens.push(Token::Tilde);
                        self.advance();
                    }
                    '®' | '⚙' | '🦀' | '\\' => {
                        self.handle_rust_escape(&mut tokens);
                    }
                    '(' => {
                        self.advance();
                        if let Some(ch2) = self.peek()
                            && !ch2.is_alphanumeric()
                            && ch2 != '_'
                            && ch2 != '('
                            && ch2 != ')'
                            && ch2 != ' '
                            && ch2 != '\n'
                            && ch2 != '|'
                            && ch2 != ','
                            && ch2 != '?'
                        {
                            let mut op = String::new();
                            let mut valid = true;
                            let start_pos = self.pos;
                            while let Some(c) = self.peek() {
                                if c == ')' {
                                    break;
                                }
                                if c.is_alphanumeric() || c == ' ' || c == ',' || c == '?' {
                                    valid = false;
                                    break;
                                }
                                op.push(c);
                                self.advance();
                            }
                            if valid && self.peek() == Some(')') {
                                self.advance();
                                tokens.push(Token::Operator(op));
                                continue 'outer;
                            } else {
                                self.pos = start_pos;
                            }
                        }
                        self.nesting_level += 1;
                        tokens.push(Token::LParen);
                    }
                    ')' => {
                        self.nesting_level = self.nesting_level.saturating_sub(1);
                        tokens.push(Token::RParen);
                        self.advance();
                    }
                    '[' => {
                        self.nesting_level += 1;
                        tokens.push(Token::LBracket);
                        self.advance();
                    }
                    ']' => {
                        self.nesting_level = self.nesting_level.saturating_sub(1);
                        tokens.push(Token::RBracket);
                        self.advance();
                    }
                    '{' => {
                        self.nesting_level += 1;
                        tokens.push(Token::LBrace);
                        self.advance();
                    }
                    '}' => {
                        self.nesting_level = self.nesting_level.saturating_sub(1);
                        tokens.push(Token::RBrace);
                        self.advance();
                    }
                    '#' => {
                        while let Some(c) = self.peek() {
                            if c == '\n' || c == '\r' {
                                break;
                            }
                            self.advance();
                        }
                    }
                    _ => {
                        if !ch.is_whitespace() {
                            tokens.push(Token::Operator(ch.to_string()));
                        }
                        self.advance();
                    }
                }
            }
        }

        while !self.pending_tokens.is_empty() {
            tokens.push(self.pending_tokens.remove(0));
        }

        if !is_new_line {
            tokens.push(Token::Newline);
        }
        while self.indent_stack.len() > 1 {
            self.indent_stack.pop();
            tokens.push(Token::Dedent);
        }

        tokens.push(Token::Eof);
        tokens
    }

    fn handle_rust_escape(&mut self, tokens: &mut Vec<Token>) {
        let trigger = self.advance().unwrap(); // consume trigger
        if self.peek() == Some('(') {
            self.advance();
            let mut content = String::new();
            let mut depth = 1;

            enum CaptureState {
                Normal,
                InString,
                InChar,
                InLineComment,
                InBlockComment,
                InRawString(usize),
            }
            let mut state = CaptureState::Normal;
            let mut escaped = false;

            while let Some(c) = self.peek() {
                match state {
                    CaptureState::Normal => {
                        if c == '/' && self.input.get(self.pos + 1) == Some(&'/') {
                            state = CaptureState::InLineComment;
                        } else if c == '/' && self.input.get(self.pos + 1) == Some(&'*') {
                            state = CaptureState::InBlockComment;
                        } else if c == '"' {
                            state = CaptureState::InString;
                        } else if c == '\'' {
                            state = CaptureState::InChar;
                        } else if c == 'r' {
                            let mut i = 1;
                            while self.input.get(self.pos + i) == Some(&'#') {
                                i += 1;
                            }
                            if self.input.get(self.pos + i) == Some(&'"') {
                                state = CaptureState::InRawString(i - 1);
                            }
                        } else if c == '(' {
                            depth += 1;
                        } else if c == ')' {
                            depth -= 1;
                            if depth == 0 {
                                self.advance();
                                break;
                            }
                        }
                    }
                    CaptureState::InString => {
                        if escaped {
                            escaped = false;
                        } else if c == '\\' {
                            escaped = true;
                        } else if c == '"' {
                            state = CaptureState::Normal;
                        }
                    }
                    CaptureState::InChar => {
                        if escaped {
                            escaped = false;
                        } else if c == '\\' {
                            escaped = true;
                        } else if c == '\'' {
                            state = CaptureState::Normal;
                        }
                    }
                    CaptureState::InLineComment => {
                        if c == '\n' {
                            state = CaptureState::Normal;
                        }
                    }
                    CaptureState::InBlockComment => {
                        if c == '*' && self.input.get(self.pos + 1) == Some(&'/') {
                            content.push(c);
                            self.advance();
                            content.push(self.peek().unwrap());
                            self.advance();
                            state = CaptureState::Normal;
                            continue;
                        }
                    }
                    CaptureState::InRawString(d) => {
                        if c == '"' {
                            let mut match_hashes = true;
                            for i in 1..=d {
                                if self.input.get(self.pos + i) != Some(&'#') {
                                    match_hashes = false;
                                    break;
                                }
                            }
                            if match_hashes {
                                content.push(c);
                                self.advance();
                                for _ in 0..d {
                                    content.push(self.peek().unwrap());
                                    self.advance();
                                }
                                state = CaptureState::Normal;
                                continue;
                            }
                        }
                    }
                }
                content.push(c);
                self.advance();
            }
            tokens.push(Token::String(content));
            tokens.push(Token::PassthroughStart);
        } else if self.peek() == Some(' ') || self.peek() == Some('\t') {
            self.advance(); // skip space or tab
            let mut line = String::new();
            while let Some(c) = self.peek() {
                if c == '\n' || c == '\r' {
                    break;
                }
                line.push(c);
                self.advance();
            }
            tokens.push(Token::String(line));
            tokens.push(Token::PassthroughStart);
        } else {
            tokens.push(Token::Operator(trigger.to_string()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backtick_string() {
        let mut lexer = Lexer::new("`git log`");
        let tokens = lexer.tokenize();
        assert!(tokens.contains(&Token::BacktickString("git log".to_string())));
    }

    #[test]
    fn test_escape_syntax() {
        let mut lexer = Lexer::new("🦀 println!(\"hi\")");
        let tokens = lexer.tokenize();
        assert!(tokens.contains(&Token::String("println!(\"hi\")".to_string())));
        assert!(tokens.contains(&Token::PassthroughStart));

        let mut lexer = Lexer::new("⚙( 1 + 1 )");
        let tokens = lexer.tokenize();
        assert!(tokens.contains(&Token::String(" 1 + 1 ".to_string())));
        assert!(tokens.contains(&Token::PassthroughStart));

        // Test whitespace preservation
        let mut lexer = Lexer::new("®   indented code");
        let tokens = lexer.tokenize();
        assert!(tokens.contains(&Token::String("  indented code".to_string())));

        // Test strings in parens
        let mut lexer = Lexer::new("⚙(call(\")\"))");
        let tokens = lexer.tokenize();
        assert!(tokens.contains(&Token::String("call(\")\")".to_string())));
    }

    #[test]
    fn test_rust_block_capture() {
        let input = "rust:\n    line 1\n    line 2\nnext_craw_line";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();

        let rust_code = tokens
            .iter()
            .find_map(|t| {
                if let Token::String(s) = t {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(rust_code, "line 1\nline 2");
        assert!(tokens.contains(&Token::Ident("next_craw_line".to_string())));
    }

    #[test]
    fn test_rust_block_with_empty_lines() {
        let input = "rust:\n    line 1\n\n    line 2\nnext";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();

        let rust_code = tokens
            .iter()
            .find_map(|t| {
                if let Token::String(s) = t {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(rust_code, "line 1\n\nline 2");
    }

    #[test]
    fn test_nested_rust_block() {
        let input = "if True:\n    rust:\n        fn hello() {\n            println!(\"hi\");\n        }\n    next";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();

        // Find the String token
        let rust_code = tokens
            .iter()
            .find_map(|t| {
                if let Token::String(s) = t {
                    if s.contains("fn hello") {
                        Some(s)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .expect("Should find rust code string");

        assert_eq!(rust_code, "fn hello() {\n    println!(\"hi\");\n}");

        // Ensure "next" is lexed after dedent
        assert!(
            tokens
                .iter()
                .any(|t| matches!(t, Token::Ident(s) if s == "next"))
        );
    }

    #[test]
    fn test_rust_block_with_cr_lf() {
        let input = "rust:\r\n    line1\r\n    line2";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();

        let rust_code = tokens
            .iter()
            .find_map(|t| {
                if let Token::String(s) = t {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(rust_code, "line1\nline2");
    }

    #[test]
    fn test_fn_block_capture() {
        let input = "fn hello() {\n    println!(\"hi\");\n}\nnext";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();

        assert_eq!(
            tokens[0],
            Token::String("fn hello() {\n    println!(\"hi\");\n}".to_string())
        );
        assert_eq!(tokens[1], Token::PassthroughStart);
        assert_eq!(tokens[2], Token::Newline);
        assert_eq!(tokens[3], Token::Ident("next".to_string()));
    }

    #[test]
    fn test_fn_block_nested_braces() {
        let input = "fn test() {\n    if true {\n        println!(\"nested\");\n    }\n}";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();

        assert_eq!(
            tokens[0],
            Token::String(
                "fn test() {\n    if true {\n        println!(\"nested\");\n    }\n}".to_string()
            )
        );
    }

    #[test]
    fn test_fn_block_braces_in_string() {
        let input = "fn test() {\n    println!(\"}\");\n}";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();

        assert_eq!(
            tokens[0],
            Token::String("fn test() {\n    println!(\"}\");\n}".to_string())
        );
    }

    #[test]
    fn test_fn_inside_string_not_triggered() {
        let input = "x = \"fn test() {}\"";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();
        // Should NOT see PassthroughStart
        assert!(!tokens.contains(&Token::PassthroughStart));
    }

    #[test]
    fn test_fn_inside_comment_not_triggered() {
        let input = "# fn test() {}";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();
        // Should NOT see PassthroughStart
        assert!(!tokens.contains(&Token::PassthroughStart));
    }

    #[test]
    fn test_fn_not_at_line_start() {
        let input = "x = fn test() {}";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();
        // Should NOT see PassthroughStart (it's not at line start)
        assert!(!tokens.contains(&Token::PassthroughStart));
    }

    #[test]
    fn test_fn_with_escaped_quote() {
        let input = "fn test() {\n    println!(\"\\\"}\");\n}";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();

        assert_eq!(
            tokens[0],
            Token::String("fn test() {\n    println!(\"\\\"}\");\n}".to_string())
        );
    }

    #[test]
    fn test_fn_block_with_comments() {
        let input = "fn test() {\n    // }\n    /* } */\n}";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();

        assert_eq!(
            tokens[0],
            Token::String("fn test() {\n    // }\n    /* } */\n}".to_string())
        );
    }

    #[test]
    fn test_fn_block_with_char_literal() {
        let input = "fn test() {\n    let c = '}';\n}";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();

        assert_eq!(
            tokens[0],
            Token::String("fn test() {\n    let c = '}';\n}".to_string())
        );
    }

    #[test]
    fn test_fn_block_with_raw_string() {
        let input = "fn test() {\n    let s = r#\"}\"#;\n}";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();

        assert_eq!(
            tokens[0],
            Token::String("fn test() {\n    let s = r#\"}\"#;\n}".to_string())
        );
    }

    #[test]
    fn test_use_line_capture() {
        let input = "use std::collections::HashMap;\nnext";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();

        assert_eq!(
            tokens[0],
            Token::String("use std::collections::HashMap;".to_string())
        );
        assert_eq!(tokens[1], Token::PassthroughStart);
        assert_eq!(tokens[2], Token::Newline);
        assert_eq!(tokens[3], Token::Ident("next".to_string()));
    }

    #[test]
    fn test_extern_line_capture() {
        let input = "extern crate rand;\nnext";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();

        assert_eq!(tokens[0], Token::String("extern crate rand;".to_string()));
        assert_eq!(tokens[1], Token::PassthroughStart);
        assert_eq!(tokens[2], Token::Newline);
    }

    #[test]
    fn test_top_level_let_line_capture() {
        let input = "let x: i32 = 42;\nnext";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();

        assert_eq!(tokens[0], Token::String("let x: i32 = 42;".to_string()));
        assert_eq!(tokens[1], Token::PassthroughStart);
        assert_eq!(tokens[2], Token::Newline);
    }

    #[test]
    fn test_nested_let_not_captured() {
        let input = "def foo():\n    let x = 10\n    return x";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();

        // Should NOT find PassthroughStart for "let x = 10"
        assert!(!tokens.iter().any(|t| matches!(t, Token::PassthroughStart)));

        // Verify it was lexed as Craw tokens
        assert!(
            tokens
                .iter()
                .any(|t| matches!(t, Token::Ident(s) if s == "let"))
        );
    }

    #[test]
    fn test_indented_use_extern_captured() {
        let input = "def foo():\n    use std::io\n    extern crate rand";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();

        assert!(
            tokens
                .iter()
                .any(|t| matches!(t, Token::String(s) if s == "use std::io"))
        );
        assert!(
            tokens
                .iter()
                .any(|t| matches!(t, Token::String(s) if s == "extern crate rand"))
        );
        assert!(tokens.contains(&Token::PassthroughStart));
    }

    #[test]
    fn test_top_level_let_after_block() {
        let input = "def foo():\n    return 1\nlet x = 10";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();

        assert!(
            tokens
                .iter()
                .any(|t| matches!(t, Token::String(s) if s == "let x = 10"))
        );
        assert!(tokens.contains(&Token::PassthroughStart));
    }

    #[test]
    fn test_unicode_rust_symbols() {
        // Test @ symbol as macro trigger
        let input = "@my_macro";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();
        assert_eq!(tokens[0], Token::At);
        assert_eq!(tokens[1], Token::Ident("my_macro".to_string()));

        // Test ⚙ symbol with parens
        let input = "⚙( 1 + 1 )";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();
        assert_eq!(tokens[0], Token::String(" 1 + 1 ".to_string()));
        assert_eq!(tokens[1], Token::PassthroughStart);

        // Test 🦀 symbol with parens
        let input = "🦀( true )";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();
        assert_eq!(tokens[0], Token::String(" true ".to_string()));
        assert_eq!(tokens[1], Token::PassthroughStart);
    }

    #[test]
    fn test_lex_macro() {
        let input = "macro foo(a, b):";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();
        assert_eq!(tokens[0], Token::Macro);
        assert_eq!(tokens[1], Token::Ident("foo".to_string()));
    }

    #[test]
    fn test_robust_rust_escape() {
        // Test with comments and strings inside parens
        let input = "®( println!(\"escaped ) quote\"); // comment with ) \n let x = 1; )";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();

        // The escape should capture everything until the final ')'
        if let Token::String(s) = &tokens[0] {
            assert!(s.contains("escaped ) quote"));
            assert!(s.contains("// comment with )"));
            assert!(s.contains("let x = 1;"));
        } else {
            panic!("Expected Token::String, got {:?}", tokens[0]);
        }
        assert_eq!(tokens[1], Token::PassthroughStart);
    }

    #[test]
    fn test_unicode_trigger_fallback() {
        // Test ® without ( or space
        let input = "®ident";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();
        assert_eq!(tokens[0], Token::Operator("®".to_string()));
        assert_eq!(tokens[1], Token::Ident("ident".to_string()));

        // Test ⚙ without ( or space
        let input = "⚙123";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();
        assert_eq!(tokens[0], Token::Operator("⚙".to_string()));
        assert_eq!(tokens[1], Token::Number(123));

        // Test 🦀 without ( or space
        let input = "🦀\"string\"";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();
        assert_eq!(tokens[0], Token::Operator("🦀".to_string()));
        assert_eq!(tokens[1], Token::String("string".to_string()));
    }

    #[test]
    fn test_unicode_fallback() {
        // '１' is a Unicode digit (U+FF11), not an ASCII digit.
        let input = "１";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();
        // It should not be silently dropped.
        // Given the current logic, it should probably be an Operator if we fix the bug.
        assert!(
            tokens
                .iter()
                .any(|t| matches!(t, Token::Operator(s) if s == "１"))
        );
    }

    #[test]
    fn test_dotted_operators() {
        let input = ".+ .- .* ./ .% .== .!= .< .<= .> .>=";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();

        assert_eq!(tokens[0], Token::DotPlus);
        assert_eq!(tokens[1], Token::DotMinus);
        assert_eq!(tokens[2], Token::DotStar);
        assert_eq!(tokens[3], Token::DotSlash);
        assert_eq!(tokens[4], Token::DotPercent);
        assert_eq!(tokens[5], Token::DotEqual);
        assert_eq!(tokens[6], Token::DotNotEqual);
        assert_eq!(tokens[7], Token::DotLess);
        assert_eq!(tokens[8], Token::DotLessEqual);
        assert_eq!(tokens[9], Token::DotGreater);
        assert_eq!(tokens[10], Token::DotGreaterEqual);
    }

    #[test]
    fn test_broadcast_call_lexing() {
        let input = "f.(x)";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();

        assert_eq!(tokens[0], Token::Ident("f".to_string()));
        assert_eq!(tokens[1], Token::Dot);
        assert_eq!(tokens[2], Token::LParen);
        assert_eq!(tokens[3], Token::Ident("x".to_string()));
        assert_eq!(tokens[4], Token::RParen);
    }

    #[test]
    fn test_linq_and_tilde() {
        let input = "from x in y where x > 0 orderby x ascending select x ~ y";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();

        assert_eq!(tokens[0], Token::From);
        assert_eq!(tokens[1], Token::Ident("x".to_string()));
        assert_eq!(tokens[2], Token::In);
        assert_eq!(tokens[3], Token::Ident("y".to_string()));
        assert_eq!(tokens[4], Token::Where);
        assert_eq!(tokens[5], Token::Ident("x".to_string()));
        assert_eq!(tokens[6], Token::GreaterThan);
        assert_eq!(tokens[7], Token::Number(0));
        assert_eq!(tokens[8], Token::OrderBy);
        assert_eq!(tokens[9], Token::Ident("x".to_string()));
        assert_eq!(tokens[10], Token::Ascending);
        assert_eq!(tokens[11], Token::Select);
        assert_eq!(tokens[12], Token::Ident("x".to_string()));
        assert_eq!(tokens[13], Token::Tilde);
        assert_eq!(tokens[14], Token::Ident("y".to_string()));
    }

    #[test]
    fn test_descending() {
        let input = "orderby x descending";
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();
        assert_eq!(tokens[0], Token::OrderBy);
        assert_eq!(tokens[1], Token::Ident("x".to_string()));
        assert_eq!(tokens[2], Token::Descending);
    }

    #[test]
    fn test_fstring_lexing() {
        let mut lexer = Lexer::new("f\"hello {name}!\"");
        let tokens = lexer.tokenize();
        assert_eq!(
            tokens,
            vec![
                Token::FStringRaw("hello {name}!".to_string()),
                Token::Newline,
                Token::Eof
            ]
        );
    }
}
