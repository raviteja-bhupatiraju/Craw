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
    Continue,
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
    /// Tokens queued mid-dispatch when disambiguating a sequence like `.=`
    /// into two separate tokens (`Dot` now, `Assign` on the next
    /// iteration). Paired with the span each was queued with, since by the
    /// time they're dequeued `self.pos` no longer points at them.
    pending_tokens: Vec<(Token, crate::text::Span)>,
    nesting_level: usize,
    current_line_indent: usize,
    /// Byte offset of each char in `input`, plus a final sentinel entry
    /// equal to the total byte length. Lets span computation convert a
    /// char index (what `pos` counts in) to a byte offset (what every
    /// `Span` is expressed in) in O(1).
    byte_offsets: Vec<usize>,
    /// One span per token emitted so far via `emit`/`emit_at`, populated
    /// only by `tokenize_spanned`'s run.
    spans: Vec<crate::text::Span>,
    /// Char index where the token currently being scanned began.
    token_start: usize,
}

impl Lexer {
    pub fn new(input: &str) -> Self {
        let chars: Vec<char> = input.chars().collect();
        let mut byte_offsets = Vec::with_capacity(chars.len() + 1);
        let mut byte_pos = 0;
        for &c in &chars {
            byte_offsets.push(byte_pos);
            byte_pos += c.len_utf8();
        }
        byte_offsets.push(byte_pos);
        Self {
            input: chars,
            pos: 0,
            indent_stack: vec![0],
            pending_tokens: Vec::new(),
            nesting_level: 0,
            current_line_indent: 0,
            byte_offsets,
            spans: Vec::new(),
            token_start: 0,
        }
    }

    /// Byte offset of the char at `idx`; `idx == input.len()` yields the
    /// total byte length (an "end of file" position).
    fn char_byte(&self, idx: usize) -> usize {
        self.byte_offsets[idx.min(self.byte_offsets.len() - 1)]
    }

    /// Pushes `tok`, with a span covering every char scanned since
    /// `self.token_start`, then moves `token_start` up to `self.pos` so the
    /// next token's span starts fresh from here. This is correct as long as
    /// nothing advances `self.pos` between tokens without going through
    /// `skip_whitespace_on_line`/the indentation scan (both of which also
    /// bump `token_start`) — see the comments at their call sites.
    fn emit(&mut self, tokens: &mut Vec<Token>, tok: Token) {
        let span =
            crate::text::Span::new(self.char_byte(self.token_start), self.char_byte(self.pos));
        tokens.push(tok);
        self.spans.push(span);
        self.token_start = self.pos;
    }

    /// Pushes `tok` with an explicit span, bypassing `token_start`/`pos`
    /// bookkeeping entirely. Needed where several tokens are synthesized
    /// together after a single lookahead/consumption pass that has already
    /// moved `pos` past all of them (a `rust:` block), so their individual
    /// spans must be computed by hand from checkpoints saved during the
    /// scan rather than derived from the lexer's current position.
    fn emit_at(&mut self, tokens: &mut Vec<Token>, tok: Token, span: crate::text::Span) {
        tokens.push(tok);
        self.spans.push(span);
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
            "continue" => Token::Continue,
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
        // Hex/octal/binary integers: 0x1F, 0o17, 0b101 (with optional `_`
        // digit-group separators). These never have a fractional or
        // exponent part.
        if self.peek() == Some('0') {
            let radix = match self.input.get(self.pos + 1) {
                Some('x') | Some('X') => Some(16u32),
                Some('o') | Some('O') => Some(8u32),
                Some('b') | Some('B') => Some(2u32),
                _ => None,
            };
            if let Some(radix) = radix {
                self.advance();
                self.advance();
                let mut digits = String::new();
                while let Some(c) = self.peek() {
                    if c == '_' {
                        self.advance();
                    } else if c.is_digit(radix) {
                        digits.push(c);
                        self.advance();
                    } else {
                        break;
                    }
                }
                return Token::Number(i64::from_str_radix(&digits, radix).unwrap_or(0));
            }
        }

        let mut result = String::new();
        let mut is_float = false;
        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() {
                result.push(ch);
                self.advance();
            } else if ch == '_'
                && self
                    .input
                    .get(self.pos + 1)
                    .is_some_and(char::is_ascii_digit)
            {
                // Digit-group separator (`1_000_000`); dropped, not part of
                // the parsed value.
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
            } else if (ch == 'e' || ch == 'E') && self.exponent_follows() {
                is_float = true;
                result.push('e');
                self.advance();
                if let Some(sign @ ('+' | '-')) = self.peek() {
                    result.push(sign);
                    self.advance();
                }
                while let Some(c) = self.peek() {
                    if c.is_ascii_digit() {
                        result.push(c);
                        self.advance();
                    } else if c == '_' {
                        self.advance();
                    } else {
                        break;
                    }
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

    /// Whether the `e`/`E` at the current position starts a valid exponent
    /// (`e9`, `e+9`, `e-9`) rather than an identifier starting with `e`
    /// (`1e_bad` should lex as `Number(1)` then `Ident("e_bad")`, not a
    /// malformed exponent).
    fn exponent_follows(&self) -> bool {
        match self.input.get(self.pos + 1) {
            Some(c) if c.is_ascii_digit() => true,
            Some('+') | Some('-') => self
                .input
                .get(self.pos + 2)
                .is_some_and(char::is_ascii_digit),
            _ => false,
        }
    }

    fn read_string(&mut self) -> Token {
        let quote = self.advance().unwrap(); // skip quote and remember it
        // Triple-quoted string (`"""..."""` / `'''...'''`): the opening
        // quote was just consumed above, so two more of the same quote
        // character right after it means this is the triple form, which
        // terminates on three consecutive matching quotes instead of one.
        // Without this, each `"` was treated as its own single-quote
        // delimiter and `"""..."""` silently lexed as three unrelated
        // (mostly empty) string tokens instead of one.
        if self.peek() == Some(quote) && self.peek_next() == Some(quote) {
            self.advance(); // second quote
            self.advance(); // third quote
            let mut result = String::new();
            loop {
                match self.peek() {
                    None => break,
                    Some(ch)
                        if ch == quote
                            && self.peek_next() == Some(quote)
                            && self.input.get(self.pos + 2) == Some(&quote) =>
                    {
                        self.advance();
                        self.advance();
                        self.advance();
                        break;
                    }
                    Some('\\') => {
                        self.advance();
                        if let Some(next) = self.advance() {
                            match next {
                                'n' => result.push('\n'),
                                'r' => result.push('\r'),
                                't' => result.push('\t'),
                                _ => result.push(next),
                            }
                        }
                    }
                    Some(ch) => {
                        result.push(ch);
                        self.advance();
                    }
                }
            }
            return Token::String(result);
        }
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
        self.tokenize_spanned().0
    }

    /// Like [`tokenize`], but also returns one [`crate::text::Span`] per
    /// token giving its exact byte range in the original source text.
    /// `tokens[i]` and the returned `Vec<Span>[i]` always describe the same
    /// lexical token.
    pub fn tokenize_spanned(&mut self) -> (Vec<Token>, Vec<crate::text::Span>) {
        let tokens = self.tokenize_impl();
        (tokens, std::mem::take(&mut self.spans))
    }

    fn tokenize_impl(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        let mut is_new_line = true;

        'outer: while self.pos < self.input.len() {
            if !self.pending_tokens.is_empty() {
                let (tok, span) = self.pending_tokens.remove(0);
                tokens.push(tok);
                self.spans.push(span);
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
                // `Indent`/`Dedent` are zero-width tokens sitting right
                // after the leading whitespace just consumed; anything
                // emitted next (them, or the first real token on the line)
                // must not have the whitespace itself in its span.
                self.token_start = self.pos;

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
                        self.emit(&mut tokens, Token::Indent);
                    } else if spaces < current_indent {
                        while *self.indent_stack.last().unwrap() > spaces {
                            self.indent_stack.pop();
                            self.emit(&mut tokens, Token::Dedent);
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
                    self.emit(&mut tokens, Token::String(line));
                    self.emit(&mut tokens, Token::PassthroughStart);
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

                    self.emit(&mut tokens, Token::String(content));
                    self.emit(&mut tokens, Token::PassthroughStart);

                    while let Some(ch) = self.peek() {
                        if ch == ' ' || ch == '\t' || ch == '\r' {
                            self.advance();
                        } else if ch == '\n' {
                            self.advance();
                            self.emit(&mut tokens, Token::Newline);
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
            // Re-anchor the next token's span to start after whatever
            // inter-token whitespace (or, on the way back from a comment,
            // the comment text) was just skipped.
            self.token_start = self.pos;

            if self
                .input
                .get(self.pos..)
                .is_some_and(|s| s.starts_with(&['r', 'u', 's', 't', ':']))
            {
                // `token_start` currently points at the 'r' of "rust:"; the
                // whole block is consumed via direct `self.pos`/`advance()`
                // manipulation below with no `emit` calls along the way, so
                // every token's span is computed by hand from checkpoints
                // saved here rather than derived from `token_start`/`pos`.
                let rust_kw_start = self.token_start;
                self.pos += 5;
                let after_keyword = self.pos;
                let block_base_indent = self.current_line_indent;

                // The rest of the `rust:` line: Rust code written on the
                // same line (`rust: panic!("x")`) is the block's first line.
                // A Craw `# comment` there is still a comment — but `#[`
                // and `#!` start Rust attributes, not comments.
                let mut same_line = String::new();
                let mut same_line_start: Option<usize> = None;
                while let Some(ch) = self.peek() {
                    if ch != '\n' && ch != '\r' {
                        if same_line_start.is_none() && !ch.is_whitespace() {
                            same_line_start = Some(self.pos);
                        }
                        if same_line_start.is_some() {
                            same_line.push(ch);
                        }
                    }
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
                let first_line_end = self.pos;
                let same_line = same_line.trim_end();
                let is_comment = same_line.starts_with('#')
                    && !same_line.starts_with("#[")
                    && !same_line.starts_with("#!");
                let same_line_code = if same_line.is_empty() || is_comment {
                    None
                } else {
                    same_line_start.map(|start| (start, same_line.to_string()))
                };

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
                if let Some((_, code)) = &same_line_code {
                    block_content = if block_content.is_empty() {
                        code.clone()
                    } else {
                        format!("{code}\n{block_content}")
                    };
                }
                let content_region_end = self.pos;

                self.emit_at(
                    &mut tokens,
                    Token::Rust,
                    crate::text::Span::new(
                        self.char_byte(rust_kw_start),
                        self.char_byte(rust_kw_start + 4),
                    ),
                );
                self.emit_at(
                    &mut tokens,
                    Token::Colon,
                    crate::text::Span::new(
                        self.char_byte(rust_kw_start + 4),
                        self.char_byte(after_keyword),
                    ),
                );
                // With same-line code the content starts right there, so
                // no `Newline` separates it from `rust:` (the parser takes
                // that `Newline` as optional) and the spans stay in order.
                let content_start_byte = match &same_line_code {
                    Some((start, _)) => self.char_byte(*start),
                    None => {
                        let first_line_end_byte = self.char_byte(first_line_end);
                        self.emit_at(
                            &mut tokens,
                            Token::Newline,
                            crate::text::Span::new(first_line_end_byte, first_line_end_byte),
                        );
                        first_line_end_byte
                    }
                };

                if let Some(indent) = first_line_indent {
                    self.emit_at(
                        &mut tokens,
                        Token::Indent,
                        crate::text::Span::new(content_start_byte, content_start_byte),
                    );
                    self.indent_stack.push(indent);
                }

                let content_end_byte = self.char_byte(content_region_end);
                self.emit_at(
                    &mut tokens,
                    Token::String(block_content),
                    crate::text::Span::new(content_start_byte, content_end_byte),
                );
                self.emit_at(
                    &mut tokens,
                    Token::PassthroughStart,
                    crate::text::Span::new(content_end_byte, content_end_byte),
                );

                if first_line_indent.is_some() {
                    self.indent_stack.pop();
                    self.emit_at(
                        &mut tokens,
                        Token::Dedent,
                        crate::text::Span::new(content_end_byte, content_end_byte),
                    );
                }

                self.emit_at(
                    &mut tokens,
                    Token::Newline,
                    crate::text::Span::new(content_end_byte, content_end_byte),
                );
                self.token_start = self.pos;
                is_new_line = true;
                continue;
            }

            let Some(ch) = self.peek() else {
                break;
            };

            if ch == '\n' {
                if self.nesting_level == 0 {
                    self.emit(&mut tokens, Token::Newline);
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
                self.emit(&mut tokens, Token::FStringRaw(result));
            } else if ch.is_alphabetic() || ch == '_' {
                let tok = self.read_ident_or_keyword();
                self.emit(&mut tokens, tok);
            } else if ch.is_ascii_digit() {
                let tok = self.read_number();
                self.emit(&mut tokens, tok);
            } else if ch == '"' || ch == '\'' {
                let tok = self.read_string();
                self.emit(&mut tokens, tok);
            } else {
                match ch {
                    '=' => {
                        self.advance();
                        if self.peek() == Some('>') {
                            self.advance();
                            self.emit(&mut tokens, Token::Arrow);
                        } else if self.peek() == Some('=') {
                            self.advance();
                            self.emit(&mut tokens, Token::Equal);
                        } else {
                            self.emit(&mut tokens, Token::Assign);
                        }
                    }
                    '!' => {
                        self.advance();
                        if self.peek() == Some('=') {
                            self.advance();
                            self.emit(&mut tokens, Token::NotEqual);
                        } else {
                            // Like any other symbol with no meaning of its
                            // own: an operator token, so the parser reports
                            // a misplaced `!` instead of it vanishing.
                            self.emit(&mut tokens, Token::Operator("!".to_string()));
                        }
                    }
                    '|' => {
                        self.advance();
                        if self.match_sequence(&['?', '*', '*', '>']) {
                            self.emit(&mut tokens, Token::PipeForwardNoneDoubleStar);
                        } else if self.match_sequence(&['?', '*', '>']) {
                            self.emit(&mut tokens, Token::PipeForwardNoneStar);
                        } else if self.match_sequence(&['?', '>']) {
                            self.emit(&mut tokens, Token::PipeForwardNone);
                        } else if self.match_sequence(&['*', '*', '>']) {
                            self.emit(&mut tokens, Token::PipeForwardDoubleStar);
                        } else if self.match_sequence(&['*', '>']) {
                            self.emit(&mut tokens, Token::PipeForwardStar);
                        } else if self.match_sequence(&['>']) {
                            self.emit(&mut tokens, Token::PipeForward);
                        } else {
                            self.emit(&mut tokens, Token::Pipe);
                        }
                    }
                    '<' => {
                        self.advance();
                        if self.match_sequence(&['*', '*', '?', '|']) {
                            self.emit(&mut tokens, Token::PipeBackwardNoneDoubleStar);
                        } else if self.match_sequence(&['*', '?', '|']) {
                            self.emit(&mut tokens, Token::PipeBackwardNoneStar);
                        } else if self.match_sequence(&['?', '|']) {
                            self.emit(&mut tokens, Token::PipeBackwardNone);
                        } else if self.match_sequence(&['*', '*', '|']) {
                            self.emit(&mut tokens, Token::PipeBackwardDoubleStar);
                        } else if self.match_sequence(&['*', '|']) {
                            self.emit(&mut tokens, Token::PipeBackwardStar);
                        } else if self.match_sequence(&['|']) {
                            self.emit(&mut tokens, Token::PipeBackward);
                        } else if self.peek() == Some('=') {
                            self.advance();
                            self.emit(&mut tokens, Token::LessEqual);
                        } else if self.peek() == Some('<') {
                            self.advance();
                            self.emit(&mut tokens, Token::ComposeBackward);
                        } else {
                            self.emit(&mut tokens, Token::LessThan);
                        }
                    }
                    '>' => {
                        self.advance();
                        if self.peek() == Some('=') {
                            self.advance();
                            self.emit(&mut tokens, Token::GreaterEqual);
                        } else if self.peek() == Some('>') {
                            self.advance();
                            self.emit(&mut tokens, Token::ComposeForward);
                        } else {
                            self.emit(&mut tokens, Token::GreaterThan);
                        }
                    }
                    '-' => {
                        self.advance();
                        if self.peek() == Some('>') {
                            self.advance();
                            self.emit(&mut tokens, Token::ThinArrow);
                        } else if self.peek() == Some('=') {
                            self.advance();
                            self.emit(&mut tokens, Token::MinusAssign);
                        } else {
                            self.emit(&mut tokens, Token::Minus);
                        }
                    }
                    '*' => {
                        self.advance();
                        if self.peek() == Some('=') {
                            self.advance();
                            self.emit(&mut tokens, Token::StarAssign);
                        } else if self.peek() == Some('*') {
                            self.advance();
                            if self.peek() == Some('=') {
                                self.advance();
                                self.emit(&mut tokens, Token::PowerAssign);
                            } else {
                                self.emit(&mut tokens, Token::Power);
                            }
                        } else {
                            self.emit(&mut tokens, Token::Star);
                        }
                    }
                    '/' => {
                        self.advance();
                        if self.peek() == Some('=') {
                            self.advance();
                            self.emit(&mut tokens, Token::SlashAssign);
                        } else if self.peek() == Some('/') {
                            self.advance();
                            self.emit(&mut tokens, Token::Operator("÷".to_string()));
                        } else {
                            self.emit(&mut tokens, Token::Slash);
                        }
                    }
                    '%' => {
                        self.advance();
                        self.emit(&mut tokens, Token::Percent);
                    }
                    '.' => {
                        self.advance();
                        if self.match_sequence(&['.', '*', '*', '>']) {
                            self.emit(&mut tokens, Token::ComposeDoubleStar);
                        } else if self.match_sequence(&['.', '*', '>']) {
                            self.emit(&mut tokens, Token::ComposeStar);
                        } else if self.match_sequence(&['.', '?', '>']) {
                            self.emit(&mut tokens, Token::ComposeNone);
                        } else if self.match_sequence(&['.']) {
                            self.emit(&mut tokens, Token::DotDot);
                        } else if self.peek() == Some('+') {
                            self.advance();
                            self.emit(&mut tokens, Token::DotPlus);
                        } else if self.peek() == Some('-') {
                            self.advance();
                            self.emit(&mut tokens, Token::DotMinus);
                        } else if self.peek() == Some('*') {
                            self.advance();
                            self.emit(&mut tokens, Token::DotStar);
                        } else if self.peek() == Some('/') {
                            self.advance();
                            self.emit(&mut tokens, Token::DotSlash);
                        } else if self.peek() == Some('%') {
                            self.advance();
                            self.emit(&mut tokens, Token::DotPercent);
                        } else if self.peek() == Some('=') {
                            // `dot_end` is the position right after the
                            // '.' we already consumed and before the '='
                            // about to be consumed, i.e. the boundary
                            // between the eventual `Dot` and `Assign`
                            // tokens this splits into on non-`==` input.
                            let dot_end = self.pos;
                            self.advance();
                            if self.peek() == Some('=') {
                                self.advance();
                                self.emit(&mut tokens, Token::DotEqual);
                            } else {
                                self.pending_tokens.push((
                                    Token::Assign,
                                    crate::text::Span::new(
                                        self.char_byte(dot_end),
                                        self.char_byte(self.pos),
                                    ),
                                ));
                                self.emit_at(
                                    &mut tokens,
                                    Token::Dot,
                                    crate::text::Span::new(
                                        self.char_byte(self.token_start),
                                        self.char_byte(dot_end),
                                    ),
                                );
                                self.token_start = self.pos;
                            }
                        } else if self.peek() == Some('!') {
                            let dot_end = self.pos;
                            self.advance();
                            if self.peek() == Some('=') {
                                self.advance();
                                self.emit(&mut tokens, Token::DotNotEqual);
                            } else {
                                self.pending_tokens.push((
                                    Token::Operator("!".to_string()),
                                    crate::text::Span::new(
                                        self.char_byte(dot_end),
                                        self.char_byte(self.pos),
                                    ),
                                ));
                                self.emit_at(
                                    &mut tokens,
                                    Token::Dot,
                                    crate::text::Span::new(
                                        self.char_byte(self.token_start),
                                        self.char_byte(dot_end),
                                    ),
                                );
                                self.token_start = self.pos;
                            }
                        } else if self.peek() == Some('<') {
                            self.advance();
                            if self.peek() == Some('=') {
                                self.advance();
                                self.emit(&mut tokens, Token::DotLessEqual);
                            } else {
                                self.emit(&mut tokens, Token::DotLess);
                            }
                        } else if self.peek() == Some('>') {
                            self.advance();
                            if self.peek() == Some('=') {
                                self.advance();
                                self.emit(&mut tokens, Token::DotGreaterEqual);
                            } else {
                                self.emit(&mut tokens, Token::DotGreater);
                            }
                        } else {
                            self.emit(&mut tokens, Token::Dot);
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
                        self.emit(&mut tokens, Token::BacktickString(content));
                    }
                    '?' => {
                        self.advance();
                        if self.peek() == Some('?') {
                            self.advance();
                            self.emit(&mut tokens, Token::NoneCoalesce);
                        } else {
                            self.emit(&mut tokens, Token::Question);
                        }
                    }
                    '$' => {
                        self.advance();
                        self.emit(&mut tokens, Token::Partial);
                    }
                    '+' => {
                        self.advance();
                        if self.peek() == Some('=') {
                            self.advance();
                            self.emit(&mut tokens, Token::PlusAssign);
                        } else {
                            self.emit(&mut tokens, Token::Plus);
                        }
                    }
                    ':' => {
                        self.advance();
                        self.emit(&mut tokens, Token::Colon);
                    }
                    ',' => {
                        self.advance();
                        self.emit(&mut tokens, Token::Comma);
                    }
                    ';' => {
                        self.advance();
                        self.emit(&mut tokens, Token::Semicolon);
                    }
                    '@' => {
                        self.advance();
                        self.emit(&mut tokens, Token::At);
                    }
                    '~' => {
                        self.advance();
                        self.emit(&mut tokens, Token::Tilde);
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
                            // A string literal right after `(` (`("")`,
                            // `("-")`, ...) is never an operator section —
                            // without this, the section scan below would
                            // swallow the quote characters and any
                            // non-alphanumeric content as bogus operator
                            // text.
                            && ch2 != '"'
                            && ch2 != '\''
                            // Likewise `([...])`/`({...})` (an empty list or
                            // dict literal right after `(`) must never be
                            // read as an operator section.
                            && ch2 != '['
                            && ch2 != '{'
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
                                self.emit(&mut tokens, Token::Operator(op));
                                continue 'outer;
                            } else {
                                self.pos = start_pos;
                            }
                        }
                        self.nesting_level += 1;
                        self.emit(&mut tokens, Token::LParen);
                    }
                    ')' => {
                        self.nesting_level = self.nesting_level.saturating_sub(1);
                        self.advance();
                        self.emit(&mut tokens, Token::RParen);
                    }
                    '[' => {
                        self.nesting_level += 1;
                        self.advance();
                        self.emit(&mut tokens, Token::LBracket);
                    }
                    ']' => {
                        self.nesting_level = self.nesting_level.saturating_sub(1);
                        self.advance();
                        self.emit(&mut tokens, Token::RBracket);
                    }
                    '{' => {
                        self.nesting_level += 1;
                        self.advance();
                        self.emit(&mut tokens, Token::LBrace);
                    }
                    '}' => {
                        self.nesting_level = self.nesting_level.saturating_sub(1);
                        self.advance();
                        self.emit(&mut tokens, Token::RBrace);
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
                        self.advance();
                        if !ch.is_whitespace() {
                            self.emit(&mut tokens, Token::Operator(ch.to_string()));
                        } else {
                            // Not one of the three whitespace chars
                            // `skip_whitespace_on_line` already consumed
                            // before dispatch (rare: other Unicode
                            // whitespace). No token to emit, but the next
                            // one must not have this char in its span.
                            self.token_start = self.pos;
                        }
                    }
                }
            }
        }
        // Defensive: every token so far has kept `token_start` in sync with
        // `pos`, but re-anchor explicitly before the purely-structural
        // (zero-width) tokens below so that invariant is never load-bearing
        // for what follows.
        self.token_start = self.pos;

        while !self.pending_tokens.is_empty() {
            let (tok, span) = self.pending_tokens.remove(0);
            tokens.push(tok);
            self.spans.push(span);
        }

        if !is_new_line {
            self.emit(&mut tokens, Token::Newline);
        }
        while self.indent_stack.len() > 1 {
            self.indent_stack.pop();
            self.emit(&mut tokens, Token::Dedent);
        }

        self.emit(&mut tokens, Token::Eof);
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
            self.emit(tokens, Token::String(content));
            self.emit(tokens, Token::PassthroughStart);
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
            self.emit(tokens, Token::String(line));
            self.emit(tokens, Token::PassthroughStart);
        } else {
            self.emit(tokens, Token::Operator(trigger.to_string()));
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
