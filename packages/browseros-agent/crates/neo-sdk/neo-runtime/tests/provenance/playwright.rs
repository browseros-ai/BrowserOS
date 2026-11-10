//! Playwright's side of a citation: the trees that keep its files unchanged at the pinned
//! commit, the block of source a cited symbol names, the messages a wording tie compares, the
//! copies of the pin, and the sync that vendors what the neo crates cite.
//!
//! Symbols resolve by indentation and brackets, not by a TypeScript parser, as the prototype
//! that checked the design's 97 real citations did (`probe-cite/resolve.py`): Playwright's
//! sources are uniformly formatted, and an unusual construct is answered by citing the
//! enclosing symbol.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::process::Command;

use regex::Regex;
use sha2::{Digest, Sha256};

use crate::lex::{self, Lit, Part, Tok, Token};

/// A directory of Playwright files kept unchanged at the pin, and where its paths sit in
/// Playwright's repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Tree {
    /// Relative to `crates/neo-sdk/`.
    pub(crate) dir: &'static str,
    /// What a path in the tree is prefixed with in Playwright's repository.
    repo_prefix: &'static str,
    /// Whether a `VENDOR.md` lists its files with their sha256.
    manifest: bool,
}

/// neo-runtime's tree, which sync writes: the cited files no other tree holds, at their paths
/// in Playwright's repository.
pub(crate) const RUNTIME: Tree = Tree {
    dir: "neo-runtime/vendor/playwright",
    repo_prefix: "",
    manifest: true,
};

/// Every tree a citation may resolve in. The spec's YAML is hashed by neo-playwright-protocol's own
/// tests (`UPSTREAM`), so it has no `VENDOR.md` here.
pub(crate) const TREES: [Tree; 4] = [
    RUNTIME,
    Tree {
        dir: "neo-playwright-injected/vendor/playwright",
        repo_prefix: "packages/",
        manifest: true,
    },
    Tree {
        dir: "neo-quickjs-injected/vendor/playwright",
        repo_prefix: "packages/",
        manifest: true,
    },
    Tree {
        dir: "neo-playwright-protocol/spec/playwright",
        repo_prefix: "packages/protocol/spec/",
        manifest: false,
    },
];

/// Files in a tree that are not Playwright sources.
const NOT_SOURCES: [&str; 4] = ["VENDOR.md", "LICENSE", "NOTICE", "UPSTREAM"];

pub(crate) const UPSTREAM: &str = "neo-playwright-protocol/spec/playwright/UPSTREAM";

/// One copy of the pinned commit outside `UPSTREAM`: where it is, how to find it, and whether
/// it may be an abbreviation.
struct PinCopy {
    file: &'static str,
    pattern: &'static str,
    prefix: bool,
}

const PIN_COPIES: [PinCopy; 6] = [
    PinCopy {
        file: "neo-runtime/vendor/playwright/VENDOR.md",
        pattern: r"at commit `([0-9a-f]+)`",
        prefix: false,
    },
    PinCopy {
        file: "neo-playwright-injected/vendor/playwright/VENDOR.md",
        pattern: r"at commit `([0-9a-f]+)`",
        prefix: false,
    },
    PinCopy {
        file: "neo-quickjs-injected/vendor/playwright/VENDOR.md",
        pattern: r"at commit `([0-9a-f]+)`",
        prefix: false,
    },
    PinCopy {
        file: "neo-playwright-injected/scripts/sync-vendor.ts",
        pattern: r"const PINNED_COMMIT = '([0-9a-f]+)'",
        prefix: false,
    },
    PinCopy {
        file: "neo-quickjs-injected/tests/goldens/selector-corpus.ts",
        pattern: r"const EXPECTED_COMMIT = '([0-9a-f]+)'",
        prefix: true,
    },
    PinCopy {
        file: "neo-playwright-injected/tests/aria-yaml-goldens.json",
        pattern: r#""source": "[^"]*\bPlaywright ([0-9a-f]{7,40})\b"#,
        prefix: true,
    },
];

/// A pin-copy or manifest finding, before the gate turns it into a problem.
pub(crate) struct Finding {
    pub(crate) file: String,
    pub(crate) line: usize,
    pub(crate) what: String,
    pub(crate) fix: String,
}

/// Every file under `root`, relative to it with `/` separators, sorted; none when `root` is
/// absent. Dotfiles (`.DS_Store`) are not files of a tree.
pub(crate) fn walk(root: &Path) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("{}: {error}", dir.display())),
        };
        for entry in entries {
            let path = entry.map_err(|error| error.to_string())?.path();
            if path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with('.'))
            {
                continue;
            }
            if path.is_dir() {
                pending.push(path);
            } else if let Ok(relative) = path.strip_prefix(root) {
                let parts: Vec<String> = relative
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().into_owned())
                    .collect();
                files.push(parts.join("/"));
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Every vendored Playwright source, by its path in Playwright's repository.
pub(crate) struct Vendors {
    /// Repository path → the trees holding it, and its path in each.
    files: BTreeMap<String, Vec<(Tree, String)>>,
}

impl Vendors {
    pub(crate) fn load(crates: &Path) -> Result<Self, String> {
        let mut files: BTreeMap<String, Vec<(Tree, String)>> = BTreeMap::new();
        for tree in TREES {
            for relative in walk(&crates.join(tree.dir))? {
                let top_level_extra = NOT_SOURCES.contains(&relative.as_str());
                let spec_extra = tree.repo_prefix.ends_with("spec/")
                    && !Path::new(&relative)
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("yml"));
                if top_level_extra || spec_extra {
                    continue;
                }
                let repo = format!("{}{relative}", tree.repo_prefix);
                files.entry(repo).or_default().push((tree, relative));
            }
        }
        Ok(Self { files })
    }

    /// Where a repository path is vendored, as a path under `crates/neo-sdk/`.
    pub(crate) fn locate(&self, repo: &str) -> Option<String> {
        let (tree, relative) = self.files.get(repo)?.first()?;
        Some(format!("{}/{relative}", tree.dir))
    }

    /// Whether a tree other than neo-runtime's holds `repo`.
    pub(crate) fn elsewhere(&self, repo: &str) -> bool {
        self.files
            .get(repo)
            .is_some_and(|trees| trees.iter().any(|(tree, _)| *tree != RUNTIME))
    }

    pub(crate) fn in_runtime(&self, repo: &str) -> bool {
        self.files
            .get(repo)
            .is_some_and(|trees| trees.iter().any(|(tree, _)| *tree == RUNTIME))
    }

    /// The repository paths neo-runtime's tree holds.
    pub(crate) fn runtime_files(&self) -> impl Iterator<Item = &String> {
        self.files
            .iter()
            .filter(|(_, trees)| trees.iter().any(|(tree, _)| *tree == RUNTIME))
            .map(|(repo, _)| repo)
    }

    /// The file names of every vendored source: a bare `frames.ts` in prose names one.
    pub(crate) fn basenames(&self) -> BTreeSet<&str> {
        self.files
            .keys()
            .filter_map(|repo| repo.rsplit('/').next())
            .collect()
    }

    /// The `packages/` directories the vendored sources come from: `isomorphic/yaml.ts` in
    /// prose names one.
    pub(crate) fn package_dirs(&self) -> BTreeSet<&str> {
        self.files
            .keys()
            .filter_map(|repo| repo.strip_prefix("packages/")?.split('/').next())
            .collect()
    }
}

/// A vendored Playwright file, as lines.
pub(crate) struct PlaywrightFile {
    pub(crate) lines: Vec<String>,
    yaml: bool,
}

impl PlaywrightFile {
    pub(crate) fn read(path: &Path) -> Result<Self, String> {
        let text =
            fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let yaml = path
            .extension()
            .is_some_and(|extension| extension == "yml" || extension == "yaml");
        Ok(Self {
            lines: text.split('\n').map(str::to_string).collect(),
            yaml,
        })
    }

    fn text(&self, block: Block) -> String {
        self.lines[block.0..=block.1].join("\n")
    }
}

/// A symbol's lines in a file, 0-based and inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Block(usize, usize);

impl Block {
    /// 1-based, as an editor shows them.
    pub(crate) fn lines(self) -> (usize, usize) {
        (self.0 + 1, self.1 + 1)
    }
}

pub(crate) enum Unresolved {
    /// No definition of this segment where the symbol puts it.
    Symbol(String),
    /// The symbol resolves, but no definition of it holds the quote: its first block.
    Quote(Block),
}

/// The blocks a symbol names, holding the quote when there is one.
///
/// `A.b` is a member of `A`, exactly one indent level (two spaces) inside it, so a local
/// variable deeper in another method never stands in for the member (the `ariaSnapshot`
/// case); `a/b` is defined anywhere inside `a`, for a closure; the first segment is top-level.
/// A block runs from its definition to the first later line at the same indent that starts
/// with `}`, `]` or `)` and does not reopen with `{` (a multi-line signature's `): T {`). A
/// definition that ends on its own line (`;`, `,`, or balanced brackets) is one line. YAML
/// keys nest by indentation. When a symbol is defined more than once (overloads, a getter and
/// a setter), any definition holding the quote satisfies it.
pub(crate) fn resolve(
    file: &PlaywrightFile,
    symbol: &str,
    quote: Option<&str>,
) -> Result<Vec<Block>, Unresolved> {
    let mut scopes = vec![(Block(0, file.lines.len().saturating_sub(1)), None::<usize>)];
    let mut separator = None;
    let mut rest = symbol;
    loop {
        let end = rest.find(['.', '/']).unwrap_or(rest.len());
        let segment = &rest[..end];
        let mut found = Vec::new();
        for (scope, indent) in &scopes {
            let (from, want) = match separator {
                None => (scope.0, Some(0)),
                Some('.') => (scope.0 + 1, indent.map(|indent| indent + 2)),
                Some(_) => (scope.0 + 1, None),
            };
            for line in definitions(file, segment, from, scope.1, want) {
                found.push((block(file, line), Some(indent_of(&file.lines[line]))));
            }
        }
        if found.is_empty() {
            return Err(Unresolved::Symbol(segment.to_string()));
        }
        scopes = found;
        let Some(next) = rest[end..].chars().next() else {
            break;
        };
        separator = Some(next);
        rest = &rest[end + 1..];
    }
    let blocks: Vec<Block> = scopes.into_iter().map(|(block, _)| block).collect();
    let Some(quote) = quote else {
        return Ok(blocks);
    };
    let holding: Vec<Block> = blocks
        .iter()
        .copied()
        .filter(|&block| file.text(block).contains(quote))
        .collect();
    if holding.is_empty() {
        return Err(Unresolved::Quote(blocks[0]));
    }
    Ok(holding)
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The lines in `from..=to` that define `name`, at `indent` when given.
fn definitions(
    file: &PlaywrightFile,
    name: &str,
    from: usize,
    to: usize,
    indent: Option<usize>,
) -> Vec<usize> {
    let name = regex::escape(name);
    let pattern = if file.yaml {
        format!(r"^(\s*){name}\s*:(?:\s|$)")
    } else {
        format!(
            r"^(\s*)(?:export\s+)?(?:default\s+)?(?:abstract\s+)?(?:declare\s+)?(?:(?:public|private|protected|static|readonly|async|override|get|set)\s+)*(?:(?:class|function\*?|const|let|var|type|interface|enum)\s+)?{name}\b\s*(?:[<(:=]|extends|implements|\{{)"
        )
    };
    let Ok(definition) = Regex::new(&pattern) else {
        return Vec::new();
    };
    (from..=to.min(file.lines.len().saturating_sub(1)))
        .filter(|&line| {
            definition
                .captures(&file.lines[line])
                .is_some_and(|found| indent.is_none_or(|indent| found[1].len() == indent))
        })
        .collect()
}

fn block(file: &PlaywrightFile, start: usize) -> Block {
    let lines = &file.lines;
    let indent = indent_of(&lines[start]);
    let last = lines.len().saturating_sub(1);
    if file.yaml {
        let end = (start + 1..lines.len()).find(|&line| {
            let text = lines[line].trim();
            let deeper = indent_of(&lines[line]) > indent
                || (indent_of(&lines[line]) == indent && text.starts_with("- "));
            !text.is_empty() && !text.starts_with('#') && !deeper
        });
        return Block(start, end.map_or(last, |end| end - 1));
    }
    let head = lines[start].trim_end();
    let balanced = head.contains('{')
        && ["{}", "()", "[]"].iter().all(|pair| {
            let mut chars = pair.chars();
            let (open, close) = (chars.next(), chars.next());
            head.chars().filter(|&ch| Some(ch) == open).count()
                == head.chars().filter(|&ch| Some(ch) == close).count()
        });
    if head.ends_with(';') || head.ends_with(',') || balanced {
        return Block(start, start);
    }
    let end = (start + 1..lines.len()).find(|&line| {
        let text = lines[line].trim();
        !text.is_empty()
            && indent_of(&lines[line]) == indent
            && text.starts_with(['}', ']', ')'])
            && !text.ends_with('{')
    });
    Block(start, end.unwrap_or(last))
}

/// The message patterns of a cited symbol: every string and template literal in its blocks,
/// placeholders and nested literals included, as regexes over a Neo message. A Neo message
/// writes a placeholder as `{}`. A template's `${…}` matches `{}` or any text, and a ternary
/// of literals (`${trial ? ' (trial run)' : ''}`) matches `{}` or one of its branches. A
/// literal with no fixed text would match anything, so it is left out.
pub(crate) fn message_patterns(file: &PlaywrightFile, blocks: &[Block]) -> Vec<Regex> {
    let mut literals = Vec::new();
    for &block in blocks {
        collect_literals(&lex::js(&file.text(block)).tokens, &mut literals);
    }
    literals.iter().filter_map(pattern).collect()
}

fn collect_literals(tokens: &[Token], out: &mut Vec<Token>) {
    for token in tokens {
        match &token.tok {
            Tok::Lit(Lit::Str(_)) => out.push(token.clone()),
            Tok::Template(parts) => {
                out.push(token.clone());
                for part in parts {
                    if let Part::Expr(inner) = part {
                        collect_literals(inner, out);
                    }
                }
            }
            _ => {}
        }
    }
}

fn pattern(token: &Token) -> Option<Regex> {
    let mut regex = String::from(r"(?s)\A");
    let mut fixed = false;
    match &token.tok {
        Tok::Lit(Lit::Str(text)) => {
            fixed = !text.trim().is_empty();
            regex.push_str(&regex::escape(text));
        }
        Tok::Template(parts) => {
            for part in parts {
                match part {
                    Part::Text(text) => {
                        fixed |= !text.trim().is_empty();
                        regex.push_str(&regex::escape(text));
                    }
                    Part::Expr(tokens) => match ternary(tokens) {
                        Some(branches) => {
                            let branches: Vec<String> = branches
                                .iter()
                                .map(|branch| regex::escape(branch))
                                .collect();
                            regex.push_str(r"(?:\{\}|");
                            regex.push_str(&branches.join("|"));
                            regex.push(')');
                        }
                        None => regex.push_str(r"(?:\{\}|.+?)"),
                    },
                }
            }
        }
        _ => return None,
    }
    regex.push_str(r"\z");
    if !fixed {
        return None;
    }
    Regex::new(&regex).ok()
}

/// The string branches of `cond ? 'a' : 'b'` (nested in the else branch too), or `None` when
/// a branch is not a literal.
fn ternary(tokens: &[Token]) -> Option<Vec<String>> {
    if let [only] = tokens {
        return match &only.tok {
            Tok::Lit(Lit::Str(text)) => Some(vec![text.clone()]),
            Tok::Template(parts) => match parts.as_slice() {
                [Part::Text(text)] => Some(vec![text.clone()]),
                _ => None,
            },
            _ => None,
        };
    }
    let mut depth = 0_usize;
    let mut question = None;
    let mut nested = 0_usize;
    for (index, token) in tokens.iter().enumerate() {
        match &token.tok {
            Tok::Punct(punct) if matches!(punct.as_str(), "(" | "[" | "{") => depth += 1,
            Tok::Punct(punct) if matches!(punct.as_str(), ")" | "]" | "}") => {
                depth = depth.saturating_sub(1);
            }
            Tok::Punct(punct) if punct == "?" && depth == 0 => {
                if question.is_none() {
                    question = Some(index);
                } else {
                    nested += 1;
                }
            }
            Tok::Punct(punct) if punct == ":" && depth == 0 && question.is_some() => {
                if nested > 0 {
                    nested -= 1;
                    continue;
                }
                let then = &tokens[question? + 1..index];
                let mut branches = ternary(then)?;
                branches.extend(ternary(&tokens[index + 1..])?);
                return Some(branches);
            }
            _ => {}
        }
    }
    None
}

/// The pinned commit, from neo-playwright-protocol's `UPSTREAM`.
pub(crate) fn read_pin(crates: &Path) -> Result<String, String> {
    let path = crates.join(UPSTREAM);
    let text = fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let commit = Regex::new(r"(?m)^commit: ([0-9a-f]{40})$").map_err(|error| error.to_string())?;
    commit
        .captures(&text)
        .map(|found| found[1].to_string())
        .ok_or_else(|| format!("{UPSTREAM} names no `commit: <sha>`"))
}

/// Every copy of the pin that disagrees with `UPSTREAM`, or cannot be found.
pub(crate) fn pin_findings(crates: &Path, pin: &str) -> Vec<Finding> {
    let fix = format!(
        "every copy of the pin moves together: set it to `{pin}` ({UPSTREAM}), or upgrade them all"
    );
    let mut findings = Vec::new();
    for copy in PIN_COPIES {
        let Ok(text) = fs::read_to_string(crates.join(copy.file)) else {
            findings.push(Finding {
                file: copy.file.to_string(),
                line: 0,
                what: "missing: it holds a copy of the Playwright pin".to_string(),
                fix: fix.clone(),
            });
            continue;
        };
        let Some(found) = Regex::new(copy.pattern)
            .ok()
            .and_then(|pattern| pattern.captures(&text))
        else {
            findings.push(Finding {
                file: copy.file.to_string(),
                line: 0,
                what: format!("names no pin (looked for `{}`)", copy.pattern),
                fix: fix.clone(),
            });
            continue;
        };
        let value = &found[1];
        let agrees = if copy.prefix {
            value.len() >= 7 && pin.starts_with(value)
        } else {
            value == pin
        };
        if !agrees {
            let at = found.get(1).map_or(0, |group| group.start());
            findings.push(Finding {
                file: copy.file.to_string(),
                line: text[..at].matches('\n').count() + 1,
                what: format!("pins Playwright at `{value}`, not `{pin}`"),
                fix: fix.clone(),
            });
        }
    }
    findings
}

/// Every file a tree's `VENDOR.md` does not list as it is: edited, added or removed.
pub(crate) fn manifest_findings(crates: &Path, tree: Tree) -> Result<Vec<Finding>, String> {
    if !tree.manifest {
        return Ok(Vec::new());
    }
    let manifest = format!("{}/VENDOR.md", tree.dir);
    let fix = format!(
        "never edit `{}`; regenerate it from a checkout at the pin (neo-runtime's: \
         `NEO_PLAYWRIGHT=<checkout> cargo test -p neo-runtime --test provenance -- --ignored sync`)",
        tree.dir
    );
    let Ok(text) = fs::read_to_string(crates.join(&manifest)) else {
        return Ok(vec![Finding {
            file: manifest,
            line: 0,
            what: "missing".to_string(),
            fix,
        }]);
    };
    let row =
        Regex::new(r"^\| `([^`]+)` \| `([0-9a-f]{64})` \|$").map_err(|error| error.to_string())?;
    let mut listed = BTreeMap::new();
    for (index, line) in text.lines().enumerate() {
        if let Some(found) = row.captures(line) {
            listed.insert(found[1].to_string(), (index + 1, found[2].to_string()));
        }
    }
    let mut findings = Vec::new();
    let present: Vec<String> = walk(&crates.join(tree.dir))?
        .into_iter()
        .filter(|path| path != "VENDOR.md")
        .collect();
    for path in &present {
        let file = format!("{}/{path}", tree.dir);
        let Some((line, sha)) = listed.get(path) else {
            findings.push(Finding {
                file,
                line: 0,
                what: "not listed in VENDOR.md".to_string(),
                fix: fix.clone(),
            });
            continue;
        };
        let bytes = fs::read(crates.join(&file)).map_err(|error| format!("{file}: {error}"))?;
        if sha256(&bytes) != *sha {
            findings.push(Finding {
                file: manifest.clone(),
                line: *line,
                what: format!("`{path}` was edited: its sha256 differs"),
                fix: fix.clone(),
            });
        }
    }
    for (path, (line, _)) in &listed {
        if !present.contains(path) {
            findings.push(Finding {
                file: manifest.clone(),
                line: *line,
                what: format!("lists `{path}`, which is missing"),
                fix: fix.clone(),
            });
        }
    }
    Ok(findings)
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// neo-runtime's `VENDOR.md`: the commit, then every file with its sha256.
pub(crate) fn render_manifest(pin: &str, rows: &[(String, String)]) -> String {
    let mut text = format!(
        "# Vendored Playwright\n\n\
         Copied unchanged from https://github.com/microsoft/playwright at commit `{pin}` by\n\
         `NEO_PLAYWRIGHT=<checkout> cargo test -p neo-runtime --test provenance -- --ignored sync`,\n\
         which generated this file. Do not edit anything here: `cargo test -p neo-runtime --test\n\
         provenance` fails on any edit, addition or removal.\n\n\
         The files are the Playwright sources that the neo crates' provenance lines cite and no\n\
         other neo vendor tree holds, at their paths in Playwright's repository, plus its LICENSE\n\
         and NOTICE.\n\n\
         | File | sha256 |\n\
         | --- | --- |\n"
    );
    text.extend(
        rows.iter()
            .map(|(path, sha)| format!("| `{path}` | `{sha}` |\n")),
    );
    text
}

/// Writes neo-runtime's tree afresh: each wanted file (repository path → where it is cited),
/// read from the pinned commit's objects in `checkout` so local edits there never leak in,
/// plus Playwright's LICENSE and NOTICE and the `VENDOR.md` that lists them. Returns how many
/// cited files it wrote.
pub(crate) fn sync(
    crates: &Path,
    pin: &str,
    wanted: &BTreeMap<String, String>,
    checkout: &Path,
) -> Result<usize, String> {
    let head = git(checkout, &["rev-parse", "HEAD"])?;
    let head = String::from_utf8_lossy(&head);
    if head.trim() != pin {
        return Err(format!(
            "{} is at {}, not the pinned Playwright commit {pin} ({UPSTREAM})",
            checkout.display(),
            head.trim()
        ));
    }
    let mut files = Vec::new();
    let mut missing = Vec::new();
    for (repo, cited_at) in wanted {
        match git(checkout, &["cat-file", "blob", &format!("{pin}:{repo}")]) {
            Ok(bytes) => files.push((repo.clone(), bytes)),
            Err(_) => missing.push(format!("`{repo}`, cited at {cited_at}")),
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "not files at {pin}; fix these citations' paths:\n  {}",
            missing.join("\n  ")
        ));
    }
    for legal in ["LICENSE", "NOTICE"] {
        files.push((
            legal.to_string(),
            git(checkout, &["cat-file", "blob", &format!("{pin}:{legal}")])?,
        ));
    }
    let root = crates.join(RUNTIME.dir);
    if root.exists() {
        fs::remove_dir_all(&root).map_err(|error| format!("{}: {error}", root.display()))?;
    }
    let mut rows = Vec::new();
    for (path, bytes) in &files {
        let target = root.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
        }
        fs::write(&target, bytes).map_err(|error| format!("{}: {error}", target.display()))?;
        let written =
            fs::read(&target).map_err(|error| format!("{}: {error}", target.display()))?;
        if written != *bytes {
            return Err(format!(
                "{}: written bytes differ from upstream",
                target.display()
            ));
        }
        rows.push((path.clone(), sha256(&written)));
    }
    rows.sort();
    let manifest = root.join("VENDOR.md");
    fs::write(&manifest, render_manifest(pin, &rows))
        .map_err(|error| format!("{}: {error}", manifest.display()))?;
    Ok(wanted.len())
}

fn git(checkout: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(checkout)
        .args(args)
        .output()
        .map_err(|error| format!("git {}: {error}", args.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}
