//! One neo production file, scanned: its items, its comments, and the provenance lines in
//! them, each attached to what it describes.
//!
//! Test code is set aside here, so no rule ever reads it: a Rust `#[cfg(test)]` item, from its
//! doc comment to its closing brace, found by token, so a brace inside a string or a raw
//! string never ends it early.

use std::cmp::Reverse;
use std::ops::Range;

use crate::lex::{self, Comment, Lexed, Style, Tok, Token};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lang {
    Rust,
    Ts,
}

/// What a coverage rule looks at. `Rust` and `Ts` are every other item (a struct, a type, an
/// interface, a `let`): they own the tags written inside them but need none of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ItemKind {
    Const,
    Static,
    Fn,
    Impl,
    Trait,
    Rust,
    /// A top-level `const` whose value is literals (`30_000`, `['visible', 'stable']`).
    TsLiteralConst,
    TsFunction,
    /// A top-level `const` object holding functions, such as the glue table.
    TsTable,
    TsClass,
    /// A member of a table or a class.
    TsMember,
    Ts,
}

#[derive(Debug, Clone)]
pub(crate) struct Item {
    pub(crate) kind: ItemKind,
    /// The declared name; an `impl`'s is its self type, a trait impl's `Trait for Type`.
    pub(crate) name: String,
    /// Token indices: the first is its first attribute or modifier, the last its closing
    /// brace or the end of its value.
    pub(crate) first: usize,
    pub(crate) last: usize,
    /// The line of its keyword (`const`, `fn`, a member's name).
    pub(crate) line: usize,
    /// A constant's value, as token indices.
    pub(crate) init: Option<Range<usize>>,
    pub(crate) test: bool,
    pub(crate) parent: Option<usize>,
    /// The tags of its doc comment, as indices into [`Source::tags`].
    pub(crate) doc: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Citation {
    /// As written: `server/frames.ts`.
    pub(crate) path: String,
    /// In Playwright's repository: `packages/playwright-core/src/server/frames.ts`.
    pub(crate) repo: String,
    pub(crate) symbol: String,
    pub(crate) quote: Option<String>,
}

impl Citation {
    pub(crate) fn show(&self) -> String {
        format!("`{}` `{}`", self.path, self.symbol)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TagKind {
    Playwright(Citation),
    Deviation(Citation, String),
    Neo(String),
    Platform(String),
    Unexplained(String),
    /// A line that starts like a tag and does not parse; the fix.
    Malformed(String),
}

impl TagKind {
    pub(crate) fn citation(&self) -> Option<&Citation> {
        match self {
            Self::Playwright(citation) | Self::Deviation(citation, _) => Some(citation),
            _ => None,
        }
    }

    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Playwright(_) => "Playwright",
            Self::Deviation(..) => "Deviation",
            Self::Neo(_) => "Neo",
            Self::Platform(_) => "Platform",
            Self::Unexplained(_) => "Unexplained",
            Self::Malformed(_) => "malformed",
        }
    }
}

/// One provenance line. Its reason may continue on the comment lines below it, to a blank
/// comment line or the next tag.
#[derive(Debug, Clone)]
pub(crate) struct Tag {
    pub(crate) line: usize,
    pub(crate) kind: TagKind,
    /// The code line the comment is about: the line after a comment on its own lines, or the
    /// comment's own line for a trailing comment.
    pub(crate) attach_line: usize,
    pub(crate) header: bool,
    /// The item whose doc comment holds it.
    pub(crate) doc_of: Option<usize>,
    pub(crate) test: bool,
}

/// What a file's header says it is; its first tag decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileKind {
    /// `Neo:` first: Neo's own module.
    Neo,
    /// One `Playwright:` line: a port of one file or class, whose members inherit it.
    PortOfOne,
    /// Several: every free function, `impl`, trait, class and table member carries a tag.
    PortOfMany,
    /// No tag, or another kind first.
    Unclassified,
}

pub(crate) struct Source {
    /// Relative to `crates/neo-sdk/`, with `/` separators: `neo-runtime/src/playwright_methods/click.rs`.
    pub(crate) path: String,
    pub(crate) lang: Lang,
    pub(crate) tokens: Vec<Token>,
    pub(crate) items: Vec<Item>,
    pub(crate) tags: Vec<Tag>,
    /// Comment lines outside tags and test code: where prose lives.
    pub(crate) prose: Vec<(usize, String)>,
    pub(crate) kind: FileKind,
    /// The first and last token of each test item.
    test_spans: Vec<(usize, usize)>,
}

const TAG_KINDS: [&str; 5] = ["Playwright", "Deviation", "Neo", "Platform", "Unexplained"];

const PLAYWRIGHT_GRAMMAR: &str = "write `Playwright: `<path>` `<symbol>`: `<quote>``, the quote \
                                  copied from that symbol on one line";
const DEVIATION_GRAMMAR: &str = "write `Deviation: `<path>` `<symbol>`[: `<quote>`] <why>`";

impl Source {
    pub(crate) fn scan(path: String, lang: Lang, text: &str) -> Self {
        let Lexed { tokens, comments } = match lang {
            Lang::Rust => lex::rust(text),
            Lang::Ts => lex::js(text),
        };
        let mut items = match lang {
            Lang::Rust => rust_items(&tokens),
            Lang::Ts => ts_items(&tokens),
        };
        nest(&mut items);
        let blocks = blocks(&comments, &tokens);
        let mut source = Self {
            path,
            lang,
            tokens,
            items,
            tags: Vec::new(),
            prose: Vec::new(),
            kind: FileKind::Unclassified,
            test_spans: Vec::new(),
        };
        let test_lines = source.mark_tests(&blocks);
        let in_test = |line: usize| test_lines.iter().any(|lines| lines.contains(&line));
        for (index, block) in blocks.iter().enumerate() {
            // A TypeScript file's first comment is its header, and also the doc of an item
            // right below it (a one-constant file); a Rust `//!` header documents no item.
            let header = match lang {
                Lang::Rust => block.style == Style::Inner,
                Lang::Ts => index == 0 && block.next.is_none_or(|next| next == 0),
            };
            let doc_of = if header && lang == Lang::Rust {
                None
            } else {
                source.documented_item(block)
            };
            let attach_line = if block.trailing {
                block.lines[0].0
            } else {
                block
                    .next
                    .map_or(usize::MAX, |next| source.tokens[next].line)
            };
            let mut tagged = Vec::new();
            for (line, last_line, kind) in block_tags(&block.lines) {
                let in_test_code = in_test(line);
                if let (Some(item), false) = (doc_of, in_test_code) {
                    source.items[item].doc.push(source.tags.len());
                }
                tagged.extend(line..=last_line);
                source.tags.push(Tag {
                    line,
                    kind,
                    attach_line,
                    header,
                    doc_of,
                    test: in_test_code,
                });
            }
            source.prose.extend(
                block
                    .lines
                    .iter()
                    .filter(|(line, _)| !tagged.contains(line) && !in_test(*line))
                    .cloned(),
            );
        }
        source.kind = file_kind(source.header_tags());
        source
    }

    pub(crate) fn header_tags(&self) -> impl Iterator<Item = &Tag> {
        self.tags.iter().filter(|tag| tag.header && !tag.test)
    }

    /// Tags outside test code.
    pub(crate) fn live_tags(&self) -> impl Iterator<Item = &Tag> {
        self.tags.iter().filter(|tag| !tag.test)
    }

    pub(crate) fn in_test(&self, token: usize) -> bool {
        self.test_spans
            .iter()
            .any(|&(first, last)| (first..=last).contains(&token))
    }

    /// The items whose span holds `token`, innermost first.
    pub(crate) fn chain(&self, token: usize) -> Vec<usize> {
        let mut chain: Vec<usize> = (0..self.items.len())
            .filter(|&index| (self.items[index].first..=self.items[index].last).contains(&token))
            .collect();
        chain.sort_by_key(|&index| self.items[index].last - self.items[index].first);
        chain
    }

    /// The item a tag describes, as the register names it.
    pub(crate) fn owner(&self, tag: &Tag) -> String {
        if let Some(item) = tag.doc_of {
            return self.display(item);
        }
        if tag.header {
            return "(file)".to_string();
        }
        (0..self.items.len())
            .filter(|&index| {
                let item = &self.items[index];
                (self.tokens[item.first].line..=self.tokens[item.last].line).contains(&tag.line)
            })
            .min_by_key(|&index| self.items[index].last - self.items[index].first)
            .map_or_else(|| "(file)".to_string(), |index| self.display(index))
    }

    /// An item's name qualified by its parent: `PageSession::click`, `glue.fill`.
    pub(crate) fn display(&self, index: usize) -> String {
        let item = &self.items[index];
        if item.kind == ItemKind::Impl {
            return format!("impl {}", item.name);
        }
        let Some(parent) = item.parent else {
            return item.name.clone();
        };
        let parent = &self.items[parent];
        let owner = parent.name.rsplit(" for ").next().unwrap_or(&parent.name);
        match parent.kind {
            ItemKind::Impl | ItemKind::Trait => format!("{owner}::{}", item.name),
            ItemKind::TsClass | ItemKind::TsTable => format!("{owner}.{}", item.name),
            _ => item.name.clone(),
        }
    }

    /// Marks `#[cfg(test)]` items, from the doc comment above each to its end, and returns
    /// their line ranges.
    fn mark_tests(&mut self, blocks: &[Block]) -> Vec<Range<usize>> {
        let mut lines = Vec::new();
        for index in 0..self.items.len() {
            if let Some(parent) = self.items[index].parent {
                self.items[index].test |= self.items[parent].test;
            }
            let item = &self.items[index];
            if !item.test || item.parent.is_some_and(|parent| self.items[parent].test) {
                continue;
            }
            self.test_spans.push((item.first, item.last));
            let first_line = blocks
                .iter()
                .find(|block| {
                    block.style != Style::Inner && self.documented_item(block) == Some(index)
                })
                .map_or(self.tokens[item.first].line, |block| block.lines[0].0);
            lines.push(first_line..self.tokens[item.last].line + 1);
        }
        lines
    }

    /// The item a comment block documents: the one whose first token is on the next line.
    fn documented_item(&self, block: &Block) -> Option<usize> {
        let next = block.next?;
        let last_line = block.lines.last()?.0;
        if block.trailing || self.tokens[next].line != last_line + 1 {
            return None;
        }
        self.items.iter().position(|item| item.first == next)
    }
}

fn file_kind<'a>(mut header: impl Iterator<Item = &'a Tag>) -> FileKind {
    match header.next().map(|tag| &tag.kind) {
        Some(TagKind::Neo(_)) => FileKind::Neo,
        Some(TagKind::Playwright(_)) => {
            if header.any(|tag| matches!(tag.kind, TagKind::Playwright(_))) {
                FileKind::PortOfMany
            } else {
                FileKind::PortOfOne
            }
        }
        _ => FileKind::Unclassified,
    }
}

/// Consecutive comment lines with no code between them: one doc comment, one header, or one
/// note in a body.
struct Block {
    lines: Vec<(usize, String)>,
    style: Style,
    /// The first token after the block.
    next: Option<usize>,
    /// The block follows code on its first line.
    trailing: bool,
}

fn blocks(comments: &[Comment], tokens: &[Token]) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    for comment in comments {
        let line = comment.lines[0].0;
        let next_index = tokens.partition_point(|token| token.start < comment.start);
        let next = (next_index < tokens.len()).then_some(next_index);
        let trailing = next_index > 0 && tokens[next_index - 1].line == line;
        if let Some(block) = out.last_mut() {
            let joins = !trailing
                && !block.trailing
                && block.next == next
                && (block.style == Style::Inner) == (comment.style == Style::Inner)
                && block.lines.last().is_some_and(|last| last.0 + 1 == line);
            if joins {
                block.lines.extend(comment.lines.iter().cloned());
                continue;
            }
        }
        out.push(Block {
            lines: comment.lines.clone(),
            style: comment.style,
            next,
            trailing,
        });
    }
    out
}

/// The tags in a comment block: each tag's first and last line, and what it says.
fn block_tags(lines: &[(usize, String)]) -> Vec<(usize, usize, TagKind)> {
    let mut tags = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let (line, text) = &lines[index];
        index += 1;
        let Some((kind, rest)) = tag_start(text) else {
            continue;
        };
        let mut last_line = *line;
        let mut continuation = Vec::new();
        if kind != "Playwright" {
            while let Some((next_line, next_text)) = lines.get(index) {
                if next_text.is_empty() || tag_start(next_text).is_some() {
                    break;
                }
                continuation.push(next_text.as_str());
                last_line = *next_line;
                index += 1;
            }
        }
        tags.push((*line, last_line, parse(kind, rest, &continuation.join(" "))));
    }
    tags
}

/// `Kind: rest`, optionally as a Markdown bullet.
fn tag_start(text: &str) -> Option<(&'static str, &str)> {
    let text = text.strip_prefix("- ").unwrap_or(text);
    TAG_KINDS.iter().find_map(|&kind| {
        let rest = text.strip_prefix(kind)?.strip_prefix(':')?;
        (rest.is_empty() || rest.starts_with(' ')).then(|| (kind, rest.trim()))
    })
}

fn parse(kind: &str, rest: &str, continuation: &str) -> TagKind {
    let reason = |text: &str| {
        let mut why = text.trim().to_string();
        if !continuation.is_empty() {
            if !why.is_empty() {
                why.push(' ');
            }
            why.push_str(continuation);
        }
        why
    };
    match kind {
        "Playwright" => match citation(rest, PLAYWRIGHT_GRAMMAR) {
            Ok((cited, tail)) if tail.trim().is_empty() => TagKind::Playwright(cited),
            Ok(_) => TagKind::Malformed(
                "a `Playwright:` line ends at its citation or quote; write prose on its own \
                 comment line"
                    .to_string(),
            ),
            Err(fix) => TagKind::Malformed(fix),
        },
        "Deviation" => match citation(rest, DEVIATION_GRAMMAR) {
            Ok((cited, tail)) => TagKind::Deviation(cited, reason(tail)),
            Err(fix) => TagKind::Malformed(fix),
        },
        "Neo" => TagKind::Neo(reason(rest)),
        "Platform" => TagKind::Platform(reason(rest)),
        _ => TagKind::Unexplained(reason(rest)),
    }
}

/// `` `path` `symbol` `` and an optional `` : `quote` ``; what follows is returned.
fn citation<'a>(rest: &'a str, grammar: &str) -> Result<(Citation, &'a str), String> {
    let (path, after) = backticked(rest).ok_or_else(|| grammar.to_string())?;
    let (symbol, after) = backticked(after.trim_start()).ok_or_else(|| grammar.to_string())?;
    let (quote, tail) = match after.strip_prefix(':') {
        Some(quoted) => {
            let (quote, tail) = backticked(quoted.trim_start()).ok_or_else(|| {
                format!("a `:` after the symbol starts a quote in backticks; {grammar}")
            })?;
            if quote.is_empty() {
                return Err(format!("the quote is empty; {grammar}"));
            }
            (Some(quote.to_string()), tail)
        }
        None => (None, after),
    };
    let repo = repo_path(path)?;
    check_symbol(symbol)?;
    let cited = Citation {
        path: path.to_string(),
        repo,
        symbol: symbol.to_string(),
        quote,
    };
    Ok((cited, tail))
}

fn backticked(text: &str) -> Option<(&str, &str)> {
    let inner = text.strip_prefix('`')?;
    let close = inner.find('`')?;
    Some((&inner[..close], &inner[close + 1..]))
}

/// Where a cited path lives in Playwright's repository.
pub(crate) fn repo_path(path: &str) -> Result<String, String> {
    let bad_segment = path
        .split('/')
        .skip(usize::from(path.starts_with('/')))
        .any(|segment| segment.is_empty() || segment == "." || segment == "..");
    if path.chars().any(|ch| ch.is_whitespace() || ch == '\\') || bad_segment {
        return Err(format!(
            "`{path}` is not a path in Playwright's repository; write it with `/` and no `.` or \
             `..` segments"
        ));
    }
    if let Some(root) = path.strip_prefix('/') {
        return Ok(root.to_string());
    }
    if let Some(under) = path.strip_prefix("packages/") {
        return Err(format!(
            "start the path at the directory under `packages/`: `{under}`"
        ));
    }
    if path.starts_with("server/") || path.starts_with("client/") {
        return Ok(format!("packages/playwright-core/src/{path}"));
    }
    if path.contains('/') {
        return Ok(format!("packages/{path}"));
    }
    Err(format!(
        "`{path}` has no directory; start it at `server/` or `client/` (playwright-core's `src/`), \
         at a `packages/` directory (`isomorphic/yaml.ts`), or at `/` for the repository root"
    ))
}

fn check_symbol(symbol: &str) -> Result<(), String> {
    let good = !symbol.is_empty()
        && symbol.split(['.', '/']).all(|segment| {
            segment
                .chars()
                .next()
                .is_some_and(|ch| !ch.is_ascii_digit())
                && segment
                    .chars()
                    .all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '$' | '#' | '-'))
        });
    if good {
        Ok(())
    } else {
        Err(format!(
            "`{symbol}` is not a symbol; cite `Class.member`, `function`, `CONSTANT` or \
             `outer/inner`"
        ))
    }
}

/// Every token's bracket depth: open `(`, `[` and `{` before it.
fn depths(tokens: &[Token]) -> Vec<usize> {
    let mut depth = 0_usize;
    tokens
        .iter()
        .map(|token| {
            let before = depth;
            if let Tok::Punct(punct) = &token.tok {
                match punct.as_str() {
                    "(" | "[" | "{" => depth += 1,
                    ")" | "]" | "}" => depth = depth.saturating_sub(1),
                    _ => {}
                }
                if matches!(punct.as_str(), ")" | "]" | "}") {
                    return depth;
                }
            }
            before
        })
        .collect()
}

/// The index of the bracket that closes the one at `open`.
fn closing(tokens: &[Token], open: usize) -> usize {
    let mut depth = 0_usize;
    for (index, token) in tokens.iter().enumerate().skip(open) {
        match &token.tok {
            Tok::Punct(punct) if matches!(punct.as_str(), "(" | "[" | "{") => depth += 1,
            Tok::Punct(punct) if matches!(punct.as_str(), ")" | "]" | "}") => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return index;
                }
            }
            _ => {}
        }
    }
    tokens.len() - 1
}

/// Where a Rust item or statement starting at `from` ends: its `;`, or the `}` of its body for
/// an item with one.
fn rust_end(tokens: &[Token], from: usize, braced: bool) -> usize {
    let mut depth = 0_usize;
    for (index, token) in tokens.iter().enumerate().skip(from) {
        let Tok::Punct(punct) = &token.tok else {
            continue;
        };
        match punct.as_str() {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => {
                depth = depth.saturating_sub(1);
                if braced && depth == 0 && punct == "}" {
                    return index;
                }
            }
            ";" if depth == 0 => return index,
            _ => {}
        }
    }
    tokens.len() - 1
}

fn rust_items(tokens: &[Token]) -> Vec<Item> {
    (0..tokens.len())
        .filter(|&index| {
            index == 0
                || tokens[index - 1].is_punct(";")
                || tokens[index - 1].is_punct("{")
                || tokens[index - 1].is_punct("}")
        })
        .filter_map(|index| rust_item(tokens, index))
        .collect()
}

/// The item that starts at `first`, after its attributes, visibility and qualifiers. A
/// `#[cfg(test)]` statement that is not an item is still returned, so its span is test code.
fn rust_item(tokens: &[Token], first: usize) -> Option<Item> {
    let at = |index: usize| tokens.get(index);
    let mut first = first;
    let mut next = first;
    let mut test = false;
    while at(next)?.is_punct("#") {
        let inner = at(next + 1)?.is_punct("!");
        let open = next + 1 + usize::from(inner);
        if !at(open)?.is_punct("[") {
            return None;
        }
        let close = closing(tokens, open);
        let words: Vec<&str> = tokens[open + 1..close]
            .iter()
            .filter_map(|token| match &token.tok {
                Tok::Ident(word) | Tok::Punct(word) => Some(word.as_str()),
                _ => None,
            })
            .collect();
        test |= words == ["cfg", "(", "test", ")"];
        next = close + 1;
        if inner {
            first = next;
        }
    }
    if at(next)?.is("pub") {
        next += 1;
        if at(next)?.is_punct("(") {
            next = closing(tokens, next) + 1;
        }
    }
    loop {
        let token = at(next)?;
        let qualifier = matches!(token.ident(), Some("async" | "unsafe" | "default" | "safe"))
            || (token.is("const")
                && at(next + 1).is_some_and(|after| {
                    matches!(after.ident(), Some("fn" | "unsafe" | "async" | "extern"))
                }))
            || (token.is("extern") && at(next + 1).is_some_and(|after| !after.is("crate")));
        if !qualifier {
            break;
        }
        next += 1;
        if token.is("extern") && matches!(at(next)?.tok, Tok::Lit(_)) {
            next += 1;
        }
    }
    let keyword = at(next)?;
    let name_at = |index: usize| at(index).and_then(Token::ident).map(str::to_string);
    let (kind, name, braced) = match keyword.ident()? {
        "const" | "static" => {
            let name_index = next + 1 + usize::from(at(next + 1)?.is("mut"));
            if !at(name_index + 1)?.is_punct(":") {
                return None;
            }
            let kind = if keyword.is("const") {
                ItemKind::Const
            } else {
                ItemKind::Static
            };
            (kind, name_at(name_index)?, false)
        }
        "fn" => (ItemKind::Fn, name_at(next + 1)?, true),
        "impl" => (ItemKind::Impl, impl_name(tokens, next), true),
        "trait" => (ItemKind::Trait, name_at(next + 1)?, true),
        "struct" | "enum" | "union" | "mod" => (ItemKind::Rust, name_at(next + 1)?, true),
        "macro_rules" => (ItemKind::Rust, name_at(next + 2)?, true),
        "type" | "use" | "extern" => (ItemKind::Rust, name_at(next + 1).unwrap_or_default(), false),
        _ if test => (ItemKind::Rust, String::new(), false),
        _ => return None,
    };
    let last = rust_end(tokens, next, braced);
    let init = matches!(kind, ItemKind::Const | ItemKind::Static)
        .then(|| {
            let equals = (next..last).find(|&index| tokens[index].is_punct("="))?;
            Some(equals + 1..last)
        })
        .flatten();
    Some(Item {
        kind,
        name,
        first,
        last,
        line: keyword.line,
        init,
        test,
        parent: None,
        doc: Vec::new(),
    })
}

/// `Type` or `Trait for Type` of an `impl` at `keyword`: the last segment of each path,
/// generics dropped.
fn impl_name(tokens: &[Token], keyword: usize) -> String {
    let (mut trait_name, mut self_name, mut after_for) = ("", "", false);
    let mut generics = 0_usize;
    for token in &tokens[keyword + 1..] {
        match &token.tok {
            Tok::Punct(punct) if punct == "{" && generics == 0 => break,
            Tok::Punct(punct) if punct == "<" || punct == "<<" => generics += punct.len(),
            Tok::Punct(punct) if punct == ">" || punct == ">>" => {
                generics = generics.saturating_sub(punct.len());
            }
            Tok::Ident(word) if generics == 0 => match word.as_str() {
                "where" => break,
                "for" => {
                    trait_name = self_name;
                    self_name = "";
                    after_for = true;
                }
                "dyn" | "mut" => {}
                _ => self_name = word,
            },
            _ => {}
        }
    }
    if after_for {
        format!("{trait_name} for {self_name}")
    } else {
        self_name.to_string()
    }
}

fn ts_items(tokens: &[Token]) -> Vec<Item> {
    let depth = depths(tokens);
    let mut items = Vec::new();
    for index in 0..tokens.len() {
        let starts = index == 0
            || tokens[index - 1].is_punct(";")
            || tokens[index - 1].is_punct("}")
            || tokens[index].line > tokens[index - 1].line;
        if depth[index] != 0 || !starts {
            continue;
        }
        let Some(item) = ts_item(tokens, &depth, index) else {
            continue;
        };
        let members = match item.kind {
            ItemKind::TsClass => ts_body(tokens, &depth, index, item.last)
                .map(|open| class_members(tokens, &depth, open))
                .unwrap_or_default(),
            ItemKind::TsTable => item
                .init
                .as_ref()
                .map(|init| table_members(tokens, &depth, init.start))
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        items.push(item);
        items.extend(members);
    }
    items
}

fn ts_item(tokens: &[Token], depth: &[usize], first: usize) -> Option<Item> {
    let at = |index: usize| tokens.get(index);
    let mut next = first;
    for modifier in ["export", "default", "declare", "abstract", "async"] {
        if at(next)?.is(modifier) {
            next += 1;
        }
    }
    let keyword = at(next)?;
    let name_at = |index: usize| at(index).and_then(Token::ident).map(str::to_string);
    let item = |kind, name, last, init| Item {
        kind,
        name,
        first,
        last,
        line: keyword.line,
        init,
        test: false,
        parent: None,
        doc: Vec::new(),
    };
    let block_end = || {
        ts_body(tokens, depth, next, tokens.len() - 1)
            .map_or(tokens.len() - 1, |open| closing(tokens, open))
    };
    match keyword.ident()? {
        "function" => {
            let name_index = next + 1 + usize::from(at(next + 1)?.is_punct("*"));
            let name = name_at(name_index).unwrap_or_else(|| "default".to_string());
            Some(item(ItemKind::TsFunction, name, block_end(), None))
        }
        "class" => {
            let name = name_at(next + 1).unwrap_or_else(|| "default".to_string());
            Some(item(ItemKind::TsClass, name, block_end(), None))
        }
        "interface" | "enum" => Some(item(
            ItemKind::Ts,
            name_at(next + 1).unwrap_or_default(),
            block_end(),
            None,
        )),
        word @ ("const" | "let" | "var" | "type") => {
            let name = name_at(next + 1).unwrap_or_default();
            let Some(equals) = ts_equals(tokens, next + 1) else {
                return Some(item(ItemKind::Ts, name, ts_expr_end(tokens, next), None));
            };
            let last = ts_expr_end(tokens, equals + 1);
            let init = equals + 1..last + 1;
            let kind = if word == "const" {
                ts_const_kind(tokens, depth, init.clone())
            } else {
                ItemKind::Ts
            };
            Some(item(kind, name, last, Some(init)))
        }
        _ => None,
    }
}

/// The `=` of a declaration starting at `from`, if it has one before the declaration ends.
fn ts_equals(tokens: &[Token], from: usize) -> Option<usize> {
    let mut depth = 0_usize;
    for index in from..tokens.len() {
        let token = &tokens[index];
        if let Tok::Punct(punct) = &token.tok {
            match punct.as_str() {
                "=" if depth == 0 => return Some(index),
                ";" | ")" | "]" | "}" if depth == 0 => return None,
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => depth -= 1,
                _ => {}
            }
        }
        let next = tokens.get(index + 1)?;
        if depth == 0 && next.line > token.line && !carries(token, next) {
            return None;
        }
    }
    None
}

/// A `const`'s kind from its value: a function, a table of functions, literals, or other code.
fn ts_const_kind(tokens: &[Token], depth: &[usize], init: Range<usize>) -> ItemKind {
    let value = &tokens[init.clone()];
    let Some(head) = value.first() else {
        return ItemKind::Ts;
    };
    let base = depth[init.start];
    let arrow = init
        .clone()
        .any(|index| depth[index] == base && tokens[index].is_punct("=>"));
    if head.is("async") || head.is("function") || arrow {
        return ItemKind::TsFunction;
    }
    let code = value
        .iter()
        .any(|token| token.is_punct("=>") || token.is("function"));
    let methods = head.is_punct("{")
        && table_members(tokens, depth, init.start)
            .iter()
            .any(|member| member.init.is_none());
    if head.is_punct("{") && (code || methods) {
        return ItemKind::TsTable;
    }
    if !code && !lex::literals(value).is_empty() {
        return ItemKind::TsLiteralConst;
    }
    ItemKind::Ts
}

/// The `{` that opens a function's or class's body at the depth of `from`, past any object
/// type in its signature; `None` before leaving the enclosing scope.
fn ts_body(tokens: &[Token], depth: &[usize], from: usize, last: usize) -> Option<usize> {
    const TYPE_POSITIONS: [&str; 6] = [":", "|", "&", "<", ",", "=>"];
    let base = depth[from];
    let mut index = from;
    while index <= last {
        if depth[index] < base {
            return None;
        }
        if depth[index] == base && tokens[index].is_punct("{") {
            let after_type = TYPE_POSITIONS
                .iter()
                .any(|punct| tokens[index - 1].is_punct(punct));
            if !after_type {
                return Some(index);
            }
            index = closing(tokens, index);
        }
        index += 1;
    }
    None
}

/// Whether a line break between `token` and `next` continues an expression: the neo
/// TypeScript sources end statements at line breaks, not semicolons.
fn carries(token: &Token, next: &Token) -> bool {
    const AFTER: [&str; 22] = [
        "=", "+", "-", "*", "/", "%", "?", ":", ",", "||", "&&", "??", "=>", ".", "?.", "|", "&",
        "(", "[", "{", "<", "**",
    ];
    const BEFORE: [&str; 13] = [
        ".", "?.", "?", ":", "+", "-", "*", "/", "||", "&&", "??", "=>", "|",
    ];
    AFTER.iter().any(|punct| token.is_punct(punct))
        || BEFORE.iter().any(|punct| next.is_punct(punct))
        || matches!(
            token.ident(),
            Some("as" | "satisfies" | "new" | "typeof" | "keyof")
        )
        || matches!(next.ident(), Some("as" | "satisfies"))
}

/// The last token of the expression starting at `from`.
fn ts_expr_end(tokens: &[Token], from: usize) -> usize {
    let mut depth = 0_usize;
    for index in from..tokens.len() {
        let token = &tokens[index];
        if let Tok::Punct(punct) = &token.tok {
            match punct.as_str() {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" | ";" if depth == 0 => return index.saturating_sub(1).max(from),
                ")" | "]" | "}" => depth -= 1,
                _ => {}
            }
        }
        let Some(next) = tokens.get(index + 1) else {
            return index;
        };
        if depth == 0 && next.line > token.line && !carries(token, next) {
            return index;
        }
    }
    tokens.len() - 1
}

/// The members of the object literal opening at `open`: `name: value`, `name(…) {…}` or a
/// shorthand `name`. A member whose value is code has no `init`.
fn table_members(tokens: &[Token], depth: &[usize], open: usize) -> Vec<Item> {
    let close = closing(tokens, open);
    let inner = depth[open] + 1;
    let mut members = Vec::new();
    let mut index = open + 1;
    while index < close {
        let first = index;
        let mut key = index;
        while tokens[key]
            .ident()
            .is_some_and(|word| matches!(word, "async" | "get" | "set"))
            && tokens[key + 1].ident().is_some()
        {
            key += 1;
        }
        let end = (key..close)
            .find(|&at| depth[at] == inner && tokens[at].is_punct(","))
            .unwrap_or(close);
        let name = match &tokens[key].tok {
            Tok::Ident(word) => Some(word.clone()),
            Tok::Lit(lex::Lit::Str(text)) => Some(text.clone()),
            _ => None,
        };
        if let Some(name) = name {
            let after = &tokens[key + 1];
            let value = key + 2..end.max(key + 2);
            let code = if after.is_punct(":") {
                tokens[value.clone()]
                    .iter()
                    .any(|token| token.is_punct("=>") || token.is("function"))
            } else {
                after.is_punct("(") || after.is_punct("<")
            };
            members.push(Item {
                kind: ItemKind::TsMember,
                name,
                first,
                last: end.saturating_sub(1).max(key),
                line: tokens[key].line,
                init: (!code).then_some(value),
                test: false,
                parent: None,
                doc: Vec::new(),
            });
        }
        index = end + 1;
    }
    members
}

/// A class body's members: each starts a line one level inside the body.
fn class_members(tokens: &[Token], depth: &[usize], open: usize) -> Vec<Item> {
    const MODIFIERS: [&str; 12] = [
        "public",
        "private",
        "protected",
        "static",
        "readonly",
        "async",
        "override",
        "abstract",
        "declare",
        "get",
        "set",
        "accessor",
    ];
    let close = closing(tokens, open);
    let inner = depth[open] + 1;
    let mut members = Vec::new();
    let mut index = open + 1;
    while index < close {
        if depth[index] != inner || tokens[index].line == tokens[index - 1].line {
            index += 1;
            continue;
        }
        let first = index;
        let mut key = index;
        while tokens[key]
            .ident()
            .is_some_and(|word| MODIFIERS.contains(&word))
            && tokens[key + 1].ident().is_some()
        {
            key += 1;
        }
        let Some(name) = tokens[key].ident().map(str::to_string) else {
            index += 1;
            continue;
        };
        let method = tokens[key + 1].is_punct("(") || tokens[key + 1].is_punct("<");
        let last = if method {
            ts_body(tokens, depth, key, close - 1).map_or(close - 1, |body| closing(tokens, body))
        } else {
            ts_expr_end(tokens, key)
        };
        members.push(Item {
            kind: ItemKind::TsMember,
            name,
            first,
            last,
            line: tokens[key].line,
            init: None,
            test: false,
            parent: None,
            doc: Vec::new(),
        });
        index = last.max(index) + 1;
    }
    members
}

/// Orders items outer-first and links each to the innermost item holding it.
fn nest(items: &mut [Item]) {
    items.sort_by_key(|item| (item.first, Reverse(item.last)));
    let mut open: Vec<usize> = Vec::new();
    for index in 0..items.len() {
        while open
            .last()
            .is_some_and(|&outer| items[outer].last < items[index].first)
        {
            open.pop();
        }
        items[index].parent = open.last().copied();
        open.push(index);
    }
}
