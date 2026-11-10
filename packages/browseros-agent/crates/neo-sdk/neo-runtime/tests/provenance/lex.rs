//! Tokenizers for the languages the gate reads: Rust (neo-runtime) and TypeScript or JavaScript
//! (neo-playwright-injected, neo-quickjs-injected and Playwright's vendored sources).
//!
//! Comments are kept apart from tokens, so a brace or a quote inside a comment, a string, a raw
//! string or a regex never moves where an item ends, and a `const` written inside a string (the
//! JavaScript in `realm.rs`'s raw strings) is never an item. Literals are decoded the way the
//! compilers decode them, so a value compares by what it means: `30_000` equals `30000` and
//! `'a\'b'` equals `"a'b"`.

/// A literal's decoded value. Numbers are canonical decimal text, so `0x10`, `16` and `16u64`
/// are equal, and `1e3` equals `1000`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Lit {
    Num(String),
    Str(String),
    Regex(String),
}

/// One piece of a template literal: decoded text, or a `${…}` placeholder's own tokens.
#[derive(Debug, Clone)]
pub(crate) enum Part {
    Text(String),
    Expr(Vec<Token>),
}

#[derive(Debug, Clone)]
pub(crate) enum Tok {
    Ident(String),
    Punct(String),
    Lit(Lit),
    Template(Vec<Part>),
    Lifetime,
}

#[derive(Debug, Clone)]
pub(crate) struct Token {
    pub(crate) tok: Tok,
    /// Byte offset into the source.
    pub(crate) start: usize,
    /// 1-based line of `start`.
    pub(crate) line: usize,
}

impl Token {
    pub(crate) fn ident(&self) -> Option<&str> {
        match &self.tok {
            Tok::Ident(word) => Some(word),
            _ => None,
        }
    }

    pub(crate) fn is(&self, text: &str) -> bool {
        match &self.tok {
            Tok::Ident(word) | Tok::Punct(word) => word == text,
            _ => false,
        }
    }

    pub(crate) fn is_punct(&self, text: &str) -> bool {
        matches!(&self.tok, Tok::Punct(punct) if punct == text)
    }
}

/// `///` and `/** */` document the next item; `//!` and `/*! */` document the enclosing module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Style {
    Outer,
    Inner,
    Plain,
}

/// A comment, split into its lines: each line's number and its text without the comment
/// markers (`//`, `///`, `/**`, a leading `*`, `*/`), trimmed.
#[derive(Debug, Clone)]
pub(crate) struct Comment {
    pub(crate) style: Style,
    pub(crate) start: usize,
    pub(crate) lines: Vec<(usize, String)>,
}

pub(crate) struct Lexed {
    pub(crate) tokens: Vec<Token>,
    pub(crate) comments: Vec<Comment>,
}

/// Turns byte offsets into 1-based line numbers.
struct Lines(Vec<usize>);

impl Lines {
    fn new(src: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(src.match_indices('\n').map(|(at, _)| at + 1));
        Self(starts)
    }

    fn of(&self, offset: usize) -> usize {
        self.0.partition_point(|&start| start <= offset)
    }
}

const RUST_PUNCTS: [&str; 22] = [
    "..=", "...", "::", "->", "=>", "==", "!=", "<=", ">=", "&&", "||", "..", "+=", "-=", "*=",
    "/=", "%=", "^=", "&=", "|=", "<<", ">>",
];

const JS_PUNCTS: [&str; 33] = [
    ">>>=", "...", "===", "!==", "**=", "<<=", ">>=", ">>>", "&&=", "||=", "??=", "=>", "==", "!=",
    "<=", ">=", "&&", "||", "??", "?.", "++", "--", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=",
    "**", "<<", ">>",
];

/// Words after which a `/` starts a regex rather than a division.
const JS_KEYWORDS_BEFORE_EXPRESSION: [&str; 14] = [
    "return",
    "typeof",
    "instanceof",
    "in",
    "of",
    "new",
    "delete",
    "void",
    "throw",
    "case",
    "do",
    "else",
    "yield",
    "await",
];

/// A byte cursor over UTF-8 source. It only ever slices at ASCII delimiters or the end, so a
/// slice never splits a character.
struct Cursor<'a> {
    src: &'a str,
    pos: usize,
}

impl Cursor<'_> {
    fn byte(&self) -> Option<u8> {
        self.src.as_bytes().get(self.pos).copied()
    }

    fn at(&self, ahead: usize) -> Option<u8> {
        self.src.as_bytes().get(self.pos + ahead).copied()
    }

    fn starts_with(&self, text: &str) -> bool {
        self.src.as_bytes()[self.pos.min(self.src.len())..].starts_with(text.as_bytes())
    }

    fn slice(&self, from: usize, to: usize) -> &str {
        let len = self.src.len();
        self.src.get(from.min(len)..to.min(len)).unwrap_or_default()
    }

    fn eat_while(&mut self, keep: impl Fn(u8) -> bool) {
        while self.byte().is_some_and(&keep) {
            self.pos += 1;
        }
    }

    fn punct(&mut self, multi: &[&str]) -> String {
        if let Some(found) = multi.iter().find(|punct| self.starts_with(punct)) {
            self.pos += found.len();
            return (*found).to_string();
        }
        let ch = self.src[self.pos..].chars().next().unwrap_or(' ');
        self.pos += ch.len_utf8();
        ch.to_string()
    }
}

fn is_ident_start(byte: u8, js: bool) -> bool {
    byte.is_ascii_alphabetic()
        || byte == b'_'
        || byte >= 0x80
        || (js && matches!(byte, b'$' | b'#'))
}

fn is_ident_char(byte: u8, js: bool) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80 || (js && byte == b'$')
}

/// Tokenizes Rust source.
pub(crate) fn rust(src: &str) -> Lexed {
    let lines = Lines::new(src);
    let mut c = Cursor { src, pos: 0 };
    let mut tokens = Vec::new();
    let mut comments = Vec::new();
    while let Some(byte) = c.byte() {
        if byte.is_ascii_whitespace() {
            c.pos += 1;
            continue;
        }
        if c.starts_with("//") {
            comments.push(line_comment(&mut c, &lines, true));
            continue;
        }
        if c.starts_with("/*") {
            comments.push(block_comment(&mut c, &lines, true));
            continue;
        }
        let start = c.pos;
        let tok = match byte {
            b'"' => Tok::Lit(Lit::Str(unescape(quoted(&mut c, b'"', true), true))),
            b'\'' => rust_quote(&mut c),
            b'0'..=b'9' => Tok::Lit(Lit::Num(number(&mut c, false))),
            _ if is_ident_start(byte, false) => rust_word(&mut c),
            _ => Tok::Punct(c.punct(&RUST_PUNCTS)),
        };
        tokens.push(Token {
            tok,
            start,
            line: lines.of(start),
        });
    }
    Lexed { tokens, comments }
}

/// An identifier, or the prefix of a raw, byte or C string (`r#"…"#`, `b"…"`, `b'x'`).
fn rust_word(c: &mut Cursor) -> Tok {
    let start = c.pos;
    c.eat_while(|byte| is_ident_char(byte, false));
    let word = c.slice(start, c.pos).to_string();
    let raw = matches!(word.as_str(), "r" | "br" | "cr");
    if raw && matches!(c.byte(), Some(b'"' | b'#')) {
        let hashes_start = c.pos;
        c.eat_while(|byte| byte == b'#');
        let hashes = c.pos - hashes_start;
        if c.byte() == Some(b'"') {
            c.pos += 1;
            let body = c.pos;
            let closing = format!("\"{}", "#".repeat(hashes));
            let end = c.src[body..]
                .find(&closing)
                .map_or(c.src.len(), |at| body + at);
            c.pos = end + closing.len();
            return Tok::Lit(Lit::Str(c.slice(body, end).to_string()));
        }
        c.pos = hashes_start;
    }
    if matches!(word.as_str(), "b" | "c") && c.byte() == Some(b'"') {
        return Tok::Lit(Lit::Str(unescape(quoted(c, b'"', true), true)));
    }
    if word == "b" && c.byte() == Some(b'\'') {
        return rust_quote(c);
    }
    Tok::Ident(word)
}

/// A char literal, or a lifetime or label: `'a'` and `'\n'` are chars, `'a` and `'static` are
/// not.
fn rust_quote(c: &mut Cursor) -> Tok {
    let start = c.pos;
    c.pos += 1;
    match c.byte() {
        Some(b'\\') => {
            c.pos += 2;
            c.eat_while(|byte| byte != b'\'' && byte != b'\n');
            let body = c.slice(start + 1, c.pos).to_string();
            c.pos += 1;
            Tok::Lit(Lit::Str(unescape(&body, true)))
        }
        Some(_) => {
            let ch = c.src[c.pos..].chars().next().unwrap_or(' ');
            if c.src.as_bytes().get(c.pos + ch.len_utf8()) == Some(&b'\'') {
                c.pos += ch.len_utf8() + 1;
                return Tok::Lit(Lit::Str(ch.to_string()));
            }
            c.eat_while(|byte| is_ident_char(byte, false));
            Tok::Lifetime
        }
        None => Tok::Punct("'".to_string()),
    }
}

/// The raw body of a quoted literal starting at the opening `quote`; the cursor ends past the
/// closing one. JavaScript strings end at a line break; Rust strings may span lines.
fn quoted<'a>(c: &mut Cursor<'a>, quote: u8, multiline: bool) -> &'a str {
    c.pos += 1;
    let body = c.pos;
    while let Some(byte) = c.byte() {
        if byte == b'\\' {
            c.pos += 2;
            continue;
        }
        if byte == quote || (!multiline && byte == b'\n') {
            break;
        }
        c.pos += 1;
    }
    let len = c.src.len();
    let text = c.src.get(body.min(len)..c.pos.min(len)).unwrap_or_default();
    c.pos += 1;
    text
}

fn line_comment(c: &mut Cursor, lines: &Lines, rust: bool) -> Comment {
    let start = c.pos;
    let end = c.src[start..]
        .find('\n')
        .map_or(c.src.len(), |at| start + at);
    let text = c.slice(start, end);
    let (style, body) = if rust && text.starts_with("///") && !text.starts_with("////") {
        (Style::Outer, &text[3..])
    } else if rust && text.starts_with("//!") {
        (Style::Inner, &text[3..])
    } else {
        (Style::Plain, &text[2..])
    };
    let comment = Comment {
        style,
        start,
        lines: vec![(lines.of(start), body.trim().to_string())],
    };
    c.pos = end;
    comment
}

/// A `/* */` comment; Rust's nest, JavaScript's do not.
fn block_comment(c: &mut Cursor, lines: &Lines, nested: bool) -> Comment {
    let start = c.pos;
    let mut depth = 0_usize;
    while c.pos < c.src.len() {
        if c.starts_with("/*") && (nested || depth == 0) {
            depth += 1;
            c.pos += 2;
        } else if c.starts_with("*/") {
            depth -= 1;
            c.pos += 2;
            if depth == 0 {
                break;
            }
        } else {
            c.pos += 1;
        }
    }
    let text = c.slice(start, c.pos);
    let style = if text.starts_with("/**") && !text.starts_with("/**/") && !text.starts_with("/***")
    {
        Style::Outer
    } else if nested && text.starts_with("/*!") {
        Style::Inner
    } else {
        Style::Plain
    };
    let first_line = lines.of(start);
    let pieces: Vec<&str> = text.split('\n').collect();
    let last = pieces.len() - 1;
    let comment_lines = pieces
        .iter()
        .enumerate()
        .map(|(index, piece)| {
            let mut body = piece.trim();
            if index == 0 {
                body = body.trim_start_matches("/**").trim_start_matches("/*!");
                body = body.strip_prefix("/*").unwrap_or(body);
            } else {
                body = body
                    .strip_prefix('*')
                    .filter(|_| !body.starts_with("*/"))
                    .unwrap_or(body);
            }
            if index == last {
                body = body.strip_suffix("*/").unwrap_or(body);
            }
            (first_line + index, body.trim().to_string())
        })
        .collect();
    Comment {
        style,
        start,
        lines: comment_lines,
    }
}

/// A number literal as canonical decimal text. Digit separators, Rust type suffixes and
/// JavaScript's `n` are dropped; a float with an integral value prints as an integer.
fn number(c: &mut Cursor, js: bool) -> String {
    let radix = match (c.byte(), c.at(1)) {
        (Some(b'0'), Some(b'x' | b'X')) => 16,
        (Some(b'0'), Some(b'o' | b'O')) => 8,
        (Some(b'0'), Some(b'b' | b'B')) => 2,
        _ => 10,
    };
    if radix != 10 {
        c.pos += 2;
        let start = c.pos;
        c.eat_while(|byte| byte.is_ascii_hexdigit() || byte == b'_');
        let digits: String = c
            .slice(start, c.pos)
            .chars()
            .filter(|&ch| ch != '_')
            .collect();
        c.eat_while(|byte| is_ident_char(byte, js));
        return u128::from_str_radix(&digits, radix).map_or(digits, |value| value.to_string());
    }
    let start = c.pos;
    let mut float = false;
    c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
    if c.byte() == Some(b'.') && c.at(1).is_some_and(|byte| byte.is_ascii_digit()) {
        float = true;
        c.pos += 1;
        c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
    }
    if matches!(c.byte(), Some(b'e' | b'E')) {
        let sign = usize::from(matches!(c.at(1), Some(b'+' | b'-')));
        if c.at(1 + sign).is_some_and(|byte| byte.is_ascii_digit()) {
            float = true;
            c.pos += 1 + sign;
            c.eat_while(|byte| byte.is_ascii_digit() || byte == b'_');
        }
    }
    let digits: String = c
        .slice(start, c.pos)
        .chars()
        .filter(|&ch| ch != '_')
        .collect();
    let suffix_start = c.pos;
    c.eat_while(|byte| is_ident_char(byte, js));
    float |= c.slice(suffix_start, c.pos).starts_with('f');
    if float {
        return digits
            .parse::<f64>()
            .map_or(digits, |value| value.to_string());
    }
    digits
        .parse::<u128>()
        .map_or(digits, |value| value.to_string())
}

/// Decodes escapes the way Rust (`rust`) or JavaScript decodes a string literal's body.
pub(crate) fn unescape(raw: &str, rust: bool) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let Some(escape) = chars.next() else {
            out.push('\\');
            break;
        };
        match escape {
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            '0' => out.push('\0'),
            'b' if !rust => out.push('\u{8}'),
            'f' if !rust => out.push('\u{c}'),
            'v' if !rust => out.push('\u{b}'),
            'x' => {
                let hex: String = chars.by_ref().take(2).collect();
                push_code_point(&mut out, &hex);
            }
            'u' => {
                let hex: String = if chars.peek() == Some(&'{') {
                    chars.next();
                    chars.by_ref().take_while(|&ch| ch != '}').collect()
                } else {
                    chars.by_ref().take(4).collect()
                };
                push_code_point(&mut out, &hex);
            }
            '\r' | '\n' => {
                if escape == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                if rust {
                    while chars.peek().is_some_and(|ch| ch.is_whitespace()) {
                        chars.next();
                    }
                }
            }
            other => out.push(other),
        }
    }
    out
}

fn push_code_point(out: &mut String, hex: &str) {
    if let Some(ch) = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) {
        out.push(ch);
    }
}

/// Tokenizes TypeScript or JavaScript source.
pub(crate) fn js(src: &str) -> Lexed {
    let lines = Lines::new(src);
    let mut lexer = Js {
        c: Cursor { src, pos: 0 },
        lines: &lines,
        comments: Vec::new(),
    };
    let tokens = lexer.tokens(false);
    Lexed {
        tokens,
        comments: lexer.comments,
    }
}

struct Js<'a, 'l> {
    c: Cursor<'a>,
    lines: &'l Lines,
    comments: Vec<Comment>,
}

impl Js<'_, '_> {
    /// Tokens to the end, or, inside a template's `${`, to the `}` that closes it.
    fn tokens(&mut self, in_placeholder: bool) -> Vec<Token> {
        let mut out: Vec<Token> = Vec::new();
        let mut depth = 0_usize;
        while let Some(byte) = self.c.byte() {
            if byte.is_ascii_whitespace() {
                self.c.pos += 1;
                continue;
            }
            if self.c.starts_with("//") {
                let comment = line_comment(&mut self.c, self.lines, false);
                self.comments.push(comment);
                continue;
            }
            if self.c.starts_with("/*") {
                let comment = block_comment(&mut self.c, self.lines, false);
                self.comments.push(comment);
                continue;
            }
            if in_placeholder && byte == b'}' && depth == 0 {
                self.c.pos += 1;
                return out;
            }
            let start = self.c.pos;
            let tok = match byte {
                b'\'' | b'"' => {
                    Tok::Lit(Lit::Str(unescape(quoted(&mut self.c, byte, false), false)))
                }
                b'`' => self.template(),
                b'/' if regex_may_follow(out.last()) => self
                    .regex()
                    .unwrap_or_else(|| Tok::Punct(self.c.punct(&JS_PUNCTS))),
                b'0'..=b'9' => Tok::Lit(Lit::Num(number(&mut self.c, true))),
                b'.' if self.c.at(1).is_some_and(|next| next.is_ascii_digit()) => {
                    Tok::Lit(Lit::Num(number(&mut self.c, true)))
                }
                _ if is_ident_start(byte, true) => {
                    self.c.pos += 1;
                    self.c.eat_while(|next| is_ident_char(next, true));
                    Tok::Ident(self.c.slice(start, self.c.pos).to_string())
                }
                _ => Tok::Punct(self.c.punct(&JS_PUNCTS)),
            };
            if let Tok::Punct(punct) = &tok {
                match punct.as_str() {
                    "{" => depth += 1,
                    "}" => depth = depth.saturating_sub(1),
                    _ => {}
                }
            }
            out.push(Token {
                tok,
                start,
                line: self.lines.of(start),
            });
        }
        out
    }

    fn template(&mut self) -> Tok {
        self.c.pos += 1;
        let mut parts = Vec::new();
        let mut text_start = self.c.pos;
        loop {
            match self.c.byte() {
                None => {
                    let text = unescape(self.c.slice(text_start, self.c.pos), false);
                    parts.push(Part::Text(text));
                    break;
                }
                Some(b'\\') => self.c.pos += 2,
                Some(b'`') => {
                    let text = unescape(self.c.slice(text_start, self.c.pos), false);
                    parts.push(Part::Text(text));
                    self.c.pos += 1;
                    break;
                }
                Some(b'$') if self.c.at(1) == Some(b'{') => {
                    let text = unescape(self.c.slice(text_start, self.c.pos), false);
                    parts.push(Part::Text(text));
                    self.c.pos += 2;
                    let tokens = self.tokens(true);
                    parts.push(Part::Expr(tokens));
                    text_start = self.c.pos;
                }
                Some(_) => self.c.pos += 1,
            }
        }
        Tok::Template(parts)
    }

    /// A regex literal, or `None` when no `/` closes it on this line (then it was a division).
    fn regex(&mut self) -> Option<Tok> {
        let bytes = self.c.src.as_bytes();
        let mut at = self.c.pos + 1;
        let mut class = false;
        loop {
            match bytes.get(at) {
                None | Some(b'\n') => return None,
                Some(b'\\') => at += 2,
                Some(b'[') => {
                    class = true;
                    at += 1;
                }
                Some(b']') => {
                    class = false;
                    at += 1;
                }
                Some(b'/') if !class => break,
                Some(_) => at += 1,
            }
        }
        let body = self.c.slice(self.c.pos + 1, at).to_string();
        self.c.pos = at + 1;
        self.c.eat_while(|byte| byte.is_ascii_alphabetic());
        Some(Tok::Lit(Lit::Regex(body)))
    }
}

fn regex_may_follow(previous: Option<&Token>) -> bool {
    match previous.map(|token| &token.tok) {
        None => true,
        Some(Tok::Punct(punct)) => punct != ")" && punct != "]",
        Some(Tok::Ident(word)) => JS_KEYWORDS_BEFORE_EXPRESSION.contains(&word.as_str()),
        Some(_) => false,
    }
}

/// The literal values among `tokens`, in order: what a value tie compares. A template counts
/// only when it has no placeholder.
pub(crate) fn literals(tokens: &[Token]) -> Vec<Lit> {
    tokens
        .iter()
        .filter_map(|token| match &token.tok {
            Tok::Lit(lit) => Some(lit.clone()),
            Tok::Template(parts) => match parts.as_slice() {
                [Part::Text(text)] => Some(Lit::Str(text.clone())),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// `[0, 20, 'visible']`: literals as a reader writes them.
pub(crate) fn show(lits: &[Lit]) -> String {
    let shown: Vec<String> = lits
        .iter()
        .map(|lit| match lit {
            Lit::Num(number) => number.clone(),
            Lit::Str(text) => format!("{text:?}"),
            Lit::Regex(body) => format!("/{body}/"),
        })
        .collect();
    format!("[{}]", shown.join(", "))
}
