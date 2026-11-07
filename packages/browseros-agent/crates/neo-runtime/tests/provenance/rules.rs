//! The rules of `crates/NEO_STYLE.md` "Provenance", each one check over the scanned sources
//! and the vendored trees, each problem naming the file, the line and the fix.
//!
//! Truth applies to every line written anywhere: the grammar, the citation (R4), a
//! `Playwright:` constant's value against its quote (R5), and a non-empty reason (R6).
//! Coverage applies to the files in `provenance-enforced.txt` only: every constant tagged (R1),
//! a header that classifies the file (R2), every item of a port of several tagged (R3), no
//! Playwright path in prose (R7), no `Duration` literal outside a constant (R8), and
//! Playwright's wording in the call logs and errors of ported code (R11). The vendor (R9), the
//! pin (R9b) and the register (R12) are whole-tree checks.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::lex::{self, Lit, Part, Tok, Token};
use crate::playwright::{self, PlaywrightFile, RUNTIME, TREES, Unresolved, Vendors};
use crate::source::{Citation, FileKind, ItemKind, Lang, Source, Tag, TagKind};

/// The neo production sources: each tree, its language and its file extension.
const SOURCE_TREES: [(&str, Lang, &str); 3] = [
    ("neo-runtime/src", Lang::Rust, ".rs"),
    ("neo-runtime-injected/src", Lang::Ts, ".ts"),
    ("neo-sdk/src", Lang::Ts, ".ts"),
];

pub(crate) const ENFORCED: &str = "neo-runtime/tests/provenance-enforced.txt";
pub(crate) const REGISTER: &str = "NEO_DEVIATIONS.md";

/// Path roots that are Neo's own, so `neo-sdk/src/page.ts` in prose is not Playwright's
/// `page.ts`.
const NEO_ROOTS: [&str; 16] = [
    "crates",
    "neo-runtime",
    "neo-runtime-injected",
    "neo-runtime-spec",
    "neo-sdk",
    "src",
    "tests",
    "scripts",
    "spec",
    "dist",
    "vendor",
    "codegen",
    "examples",
    "integration-tests",
    "apps",
    "node_modules",
];

const SYNC: &str =
    "`NEO_PLAYWRIGHT=<checkout> cargo test -p neo-runtime --test provenance -- --ignored sync`";

/// A rule broken at one place, and how to fix it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Problem {
    /// Relative to `crates/`.
    pub(crate) file: String,
    /// 1-based; 0 for the whole file.
    pub(crate) line: usize,
    pub(crate) what: String,
    pub(crate) fix: String,
}

impl Problem {
    fn new(file: &str, line: usize, what: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            file: file.to_string(),
            line,
            what: what.into(),
            fix: fix.into(),
        }
    }

    /// `crates/<file>:<line>  <what>`, then the fix, indented.
    pub(crate) fn show(&self) -> String {
        let location = if self.line == 0 {
            format!("crates/{}", self.file)
        } else {
            format!("crates/{}:{}", self.file, self.line)
        };
        format!("{location}  {}\n      {}", self.what, self.fix)
    }
}

impl From<playwright::Finding> for Problem {
    fn from(finding: playwright::Finding) -> Self {
        Self::new(&finding.file, finding.line, finding.what, finding.fix)
    }
}

pub(crate) fn report(problems: &[Problem]) -> String {
    let mut text = format!(
        "provenance: {} problem{} (crates/NEO_STYLE.md \"Provenance\")\n",
        problems.len(),
        if problems.len() == 1 { "" } else { "s" }
    );
    for problem in problems {
        text.push_str("  ");
        text.push_str(&problem.show());
        text.push('\n');
    }
    text
}

/// The neo crates as the gate reads them, from one `crates/` directory: the real one, or a
/// fixture shaped like it.
pub(crate) struct Gate {
    crates: PathBuf,
    pin: String,
    sources: Vec<Source>,
    vendors: Vendors,
    /// Every cited file that is vendored, by repository path.
    playwright: BTreeMap<String, PlaywrightFile>,
    /// The enforced list: each path with its line.
    enforced: Vec<(usize, String)>,
}

impl Gate {
    pub(crate) fn load(crates: &Path) -> Result<Self, String> {
        let pin = playwright::read_pin(crates)?;
        let mut sources = Vec::new();
        for (dir, lang, extension) in SOURCE_TREES {
            for relative in playwright::walk(&crates.join(dir))? {
                if !relative.ends_with(extension) {
                    continue;
                }
                let path = format!("{dir}/{relative}");
                let text = fs::read_to_string(crates.join(&path))
                    .map_err(|error| format!("{path}: {error}"))?;
                sources.push(Source::scan(path, lang, &text));
            }
        }
        let vendors = Vendors::load(crates)?;
        let mut files = BTreeMap::new();
        for source in &sources {
            for citation in source.live_tags().filter_map(|tag| tag.kind.citation()) {
                if files.contains_key(&citation.repo) {
                    continue;
                }
                if let Some(path) = vendors.locate(&citation.repo) {
                    let file = PlaywrightFile::read(&crates.join(path))?;
                    files.insert(citation.repo.clone(), file);
                }
            }
        }
        // A missing list is an error, not an empty one: deleting it must not switch coverage off.
        let list = fs::read_to_string(crates.join(ENFORCED))
            .map_err(|error| format!("{ENFORCED}: {error}"))?;
        let enforced = list
            .lines()
            .enumerate()
            .map(|(index, line)| (index + 1, line.trim().to_string()))
            .filter(|(_, path)| !path.is_empty() && !path.starts_with('#'))
            .collect();
        Ok(Self {
            crates: crates.to_path_buf(),
            pin,
            sources,
            vendors,
            playwright: files,
            enforced,
        })
    }

    /// Every rule over every source: truth everywhere, coverage in enforced files.
    pub(crate) fn source_problems(&self) -> Vec<Problem> {
        let mut problems = self.enforced_list_problems();
        for source in &self.sources {
            self.truth(source, &mut problems);
            if self.enforced.iter().any(|(_, path)| *path == source.path) {
                self.coverage(source, &mut problems);
            }
        }
        problems.sort();
        problems
    }

    /// R9 and R9b: neo-runtime's tree holds exactly the cited files no other tree holds, every
    /// `VENDOR.md` matches its files, and every copy of the pin agrees.
    pub(crate) fn vendor_problems(&self) -> Result<Vec<Problem>, String> {
        let plan = self.vendor_plan();
        let mut problems = Vec::new();
        for (repo, (file, line, path)) in &plan {
            if !self.vendors.in_runtime(repo) {
                problems.push(Problem::new(
                    file,
                    *line,
                    format!("`{path}` is not vendored"),
                    format!(
                        "if it is a Playwright file at the pin, vendor it: {SYNC} (sync names a \
                         path that is not); never edit `vendor/`"
                    ),
                ));
            }
        }
        for repo in self.vendors.runtime_files() {
            if !plan.contains_key(repo) {
                problems.push(Problem::new(
                    &format!("{}/{repo}", RUNTIME.dir),
                    0,
                    "vendored, but no provenance line cites it, or another neo crate vendors it",
                    format!("rerun {SYNC}"),
                ));
            }
        }
        for tree in TREES {
            let findings = playwright::manifest_findings(&self.crates, tree)?;
            problems.extend(findings.into_iter().map(Problem::from));
        }
        let pins = playwright::pin_findings(&self.crates, &self.pin);
        problems.extend(pins.into_iter().map(Problem::from));
        problems.sort();
        Ok(problems)
    }

    /// The cited files only neo-runtime's tree can hold, each with its first citation.
    fn vendor_plan(&self) -> BTreeMap<String, (String, usize, String)> {
        let mut plan = BTreeMap::new();
        for source in &self.sources {
            for tag in source.live_tags() {
                let Some(citation) = tag.kind.citation() else {
                    continue;
                };
                if !self.vendors.elsewhere(&citation.repo) {
                    plan.entry(citation.repo.clone())
                        .or_insert_with(|| (source.path.clone(), tag.line, citation.path.clone()));
                }
            }
        }
        plan
    }

    /// Vendors the cited files from a Playwright checkout at the pin.
    pub(crate) fn sync(&self, checkout: &Path) -> Result<usize, String> {
        let wanted = self
            .vendor_plan()
            .into_iter()
            .map(|(repo, (file, line, _))| (repo, format!("crates/{file}:{line}")))
            .collect();
        playwright::sync(&self.crates, &self.pin, &wanted, checkout)
    }

    /// R12: the register in the tree equals what the tags produce.
    pub(crate) fn register_problems(&self) -> Vec<Problem> {
        let generated = self.register();
        let what = match fs::read_to_string(self.crates.join(REGISTER)) {
            Ok(committed) if committed == generated => return Vec::new(),
            Ok(committed) => stale_rows(&generated, &committed),
            Err(_) => "missing".to_string(),
        };
        vec![Problem::new(
            REGISTER,
            0,
            what,
            "rerun `NEO_RUNTIME_WRITE_FIXTURES=1 cargo test -p neo-runtime --test provenance` and \
             commit it; on a rebase conflict, regenerate it rather than merge it",
        )]
    }

    pub(crate) fn write_register(&self) -> Result<(), String> {
        let path = self.crates.join(REGISTER);
        fs::write(&path, self.register()).map_err(|error| format!("{}: {error}", path.display()))
    }

    /// `crates/NEO_DEVIATIONS.md`: every `Deviation:` and `Unexplained:` line outside test
    /// code, by file and item, without line numbers, so it changes only when one does.
    pub(crate) fn register(&self) -> String {
        let cell = |text: &str| {
            text.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .replace('|', "\\|")
        };
        let mut deviations = BTreeSet::new();
        let mut unknowns = BTreeSet::new();
        for source in &self.sources {
            for tag in source.live_tags() {
                let owner = source.owner(tag);
                let item = if owner == "(file)" {
                    owner
                } else {
                    format!("`{owner}`")
                };
                match &tag.kind {
                    TagKind::Deviation(citation, why) => {
                        deviations.insert(format!(
                            "| {} | {item} | {} | {} |",
                            source.path,
                            citation.show(),
                            cell(why)
                        ));
                    }
                    TagKind::Unexplained(what) => {
                        unknowns.insert(format!("| {} | {item} | {} |", source.path, cell(what)));
                    }
                    _ => {}
                }
            }
        }
        let short = &self.pin[..self.pin.len().min(9)];
        let lines = |rows: &BTreeSet<String>| {
            let mut text = String::new();
            for row in rows {
                text.push_str(row);
                text.push('\n');
            }
            text
        };
        format!(
            "<!-- @generated by NEO_RUNTIME_WRITE_FIXTURES=1 cargo test -p neo-runtime --test \
             provenance. Never edit. -->\n\
             # Where Neo differs from Playwright (pin {short})\n\n\
             Every `Deviation:` and `Unexplained:` line in the neo crates' sources, by file and \
             item (`crates/NEO_STYLE.md`, Provenance). Regenerate it on a rebase conflict; never \
             merge it by hand.\n\n\
             ## Deviations ({})\n\n\
             | File | Item | Playwright | Why |\n\
             | --- | --- | --- | --- |\n\
             {}\n\
             ## Unexplained: owner questions ({})\n\n\
             | File | Item | What is unknown |\n\
             | --- | --- | --- |\n\
             {}",
            deviations.len(),
            lines(&deviations),
            unknowns.len(),
            lines(&unknowns)
        )
    }

    fn enforced_list_problems(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        for (index, (line, path)) in self.enforced.iter().enumerate() {
            if !self.sources.iter().any(|source| source.path == *path) {
                let what = if self.crates.join(path).exists() {
                    format!("`{path}` is not a neo source the gate reads")
                } else {
                    format!("`{path}` does not exist")
                };
                problems.push(Problem::new(
                    ENFORCED,
                    *line,
                    what,
                    "list a file under neo-runtime/src, neo-runtime-injected/src or neo-sdk/src, \
                     relative to crates/, or remove the line",
                ));
            }
            if let Some((_, previous)) = index.checked_sub(1).map(|before| &self.enforced[before])
                && previous >= path
            {
                problems.push(Problem::new(
                    ENFORCED,
                    *line,
                    format!("`{path}` is listed out of order or twice"),
                    "keep the list sorted, one path per line, so two additions rarely touch the \
                     same line",
                ));
            }
        }
        problems
    }

    /// Grammar, R4, R5 and R6, for every line outside test code.
    fn truth(&self, source: &Source, out: &mut Vec<Problem>) {
        for tag in source.live_tags() {
            let reason = match &tag.kind {
                TagKind::Malformed(fix) => {
                    out.push(Problem::new(
                        &source.path,
                        tag.line,
                        "malformed provenance line",
                        fix,
                    ));
                    continue;
                }
                TagKind::Playwright(citation) => {
                    self.check_citation(source, tag, citation, out);
                    continue;
                }
                TagKind::Deviation(citation, why) => {
                    self.check_citation(source, tag, citation, out);
                    (
                        why,
                        "say why Neo differs, after the citation or on the lines below it",
                    )
                }
                TagKind::Neo(why) => (why, "say why Neo needs it and why no script can see it"),
                TagKind::Platform(fact) => (fact, "state the platform fact and its source"),
                TagKind::Unexplained(what) => {
                    (what, "say what is unknown: it is an owner question")
                }
            };
            if reason.0.is_empty() {
                out.push(Problem::new(
                    &source.path,
                    tag.line,
                    format!("`{}:` gives no reason", tag.kind.name()),
                    reason.1,
                ));
            }
        }
        for index in 0..source.items.len() {
            let item = &source.items[index];
            let constant = matches!(
                item.kind,
                ItemKind::Const | ItemKind::Static | ItemKind::TsLiteralConst
            );
            if constant && !item.test {
                value_tie(source, index, out);
            }
        }
    }

    /// R4: the cited symbol is defined in the vendored file, and holds the quote. A file that
    /// is not vendored is R9's to report.
    fn check_citation(
        &self,
        source: &Source,
        tag: &Tag,
        citation: &Citation,
        out: &mut Vec<Problem>,
    ) {
        let Some(file) = self.playwright.get(&citation.repo) else {
            return;
        };
        match playwright::resolve(file, &citation.symbol, citation.quote.as_deref()) {
            Ok(_) => {}
            Err(Unresolved::Symbol(segment)) => out.push(Problem::new(
                &source.path,
                tag.line,
                format!("{} does not resolve: no `{segment}` there", citation.show()),
                "cite a symbol the file defines: `Class.member` (one level inside the class), \
                 `function`, `CONSTANT`, or `outer/inner` for a closure",
            )),
            Err(Unresolved::Quote(block)) => {
                let (first, last) = block.lines();
                out.push(Problem::new(
                    &source.path,
                    tag.line,
                    format!(
                        "quote not in {} (lines {first}-{last} at {})",
                        citation.show(),
                        &self.pin[..9]
                    ),
                    "copy Playwright's code exactly, on one line, from inside that symbol",
                ));
            }
        }
    }

    /// R1, R2, R3, R7, R8 and R11, for an enforced file.
    fn coverage(&self, source: &Source, out: &mut Vec<Problem>) {
        let lead = match source.lang {
            Lang::Rust => "///",
            Lang::Ts => "*",
        };
        match source.header_tags().next().map(|tag| (tag.line, &tag.kind)) {
            None => out.push(Problem::new(
                &source.path,
                1,
                "the file header has no provenance line",
                match source.lang {
                    Lang::Rust => {
                        "start the `//!` header with `//! Neo: <why>` for Neo's own module, or \
                         `//! Playwright: `<path>` `<symbol>`` for a port"
                    }
                    Lang::Ts => {
                        "start the first `/** */` with `Neo: <why>` for Neo's own module, or \
                         `Playwright: `<path>` `<symbol>`` for a port"
                    }
                },
            )),
            Some((line, kind))
                if !matches!(
                    kind,
                    TagKind::Neo(_) | TagKind::Playwright(_) | TagKind::Malformed(_)
                ) =>
            {
                out.push(Problem::new(
                    &source.path,
                    line,
                    format!("the header's first provenance line is `{}:`", kind.name()),
                    "start the header with `Neo:` or `Playwright:`: its first line says what the \
                     file is",
                ));
            }
            Some(_) => {}
        }
        for (index, item) in source.items.iter().enumerate() {
            if item.test {
                continue;
            }
            let constant = match item.kind {
                ItemKind::Const | ItemKind::Static => item.name != "_",
                ItemKind::TsLiteralConst => true,
                _ => false,
            };
            if constant && item.doc.is_empty() {
                out.push(Problem::new(
                    &source.path,
                    item.line,
                    format!("untagged constant `{}`", item.name),
                    format!(
                        "give it a provenance line in its doc comment: `{lead} Playwright: …`, \
                         `{lead} Deviation: …`, `{lead} Neo: …`, `{lead} Platform: …` or \
                         `{lead} Unexplained: …`"
                    ),
                ));
            }
            let ported_item = match item.kind {
                ItemKind::Fn
                | ItemKind::Impl
                | ItemKind::Trait
                | ItemKind::TsFunction
                | ItemKind::TsClass => item.parent.is_none(),
                ItemKind::TsMember => item
                    .parent
                    .is_some_and(|parent| source.items[parent].kind == ItemKind::TsTable),
                _ => false,
            };
            if source.kind == FileKind::PortOfMany && ported_item && item.doc.is_empty() {
                out.push(Problem::new(
                    &source.path,
                    item.line,
                    format!(
                        "untagged `{}` in a port of several Playwright sources",
                        source.display(index)
                    ),
                    format!(
                        "tag it `{lead} Playwright: …`, `{lead} Deviation: …` or `{lead} Neo: …`; \
                         a tag on an `impl`, trait, class or table covers its members"
                    ),
                ));
            }
        }
        self.prose_paths(source, out);
        if source.lang == Lang::Rust {
            duration_literals(source, out);
        }
        self.wording(source, out);
    }

    /// R7: a Playwright source path in a comment outside a provenance line.
    fn prose_paths(&self, source: &Source, out: &mut Vec<Problem>) {
        let Ok(paths) = Regex::new(
            r"(?:^|[^A-Za-z0-9_./@$-])((?:[A-Za-z0-9_@$.-]+/)*[A-Za-z0-9_$.-]+\.(?:ts|tsx|js|mjs|cjs|yml|yaml|css|json))\b",
        ) else {
            return;
        };
        let basenames = self.vendors.basenames();
        let package_dirs = self.vendors.package_dirs();
        for (line, text) in &source.prose {
            for found in paths.captures_iter(text) {
                let path = &found[1];
                let first = path.split('/').next().unwrap_or(path);
                let base = path.rsplit('/').next().unwrap_or(path);
                let playwright = if path.contains("vendor/playwright/") {
                    false
                } else if path.contains('/') && ["server", "client"].contains(&first) {
                    true
                } else if let Some(package) = path.strip_prefix("packages/") {
                    !package.starts_with("browseros-agent/")
                } else if path.contains('/') && NEO_ROOTS.contains(&first) {
                    false
                } else {
                    (path.contains('/') && package_dirs.contains(first)) || basenames.contains(base)
                };
                if playwright {
                    out.push(Problem::new(
                        &source.path,
                        *line,
                        format!("names Playwright's `{path}` in prose"),
                        "cite it in a `Playwright:` or `Deviation:` line, or write a Neo path in \
                         full (`neo-sdk/src/page.ts`, `vendor/playwright/…`)",
                    ));
                }
            }
        }
    }

    /// R11: each message literal a Playwright-tagged item sends to the call log or raises as
    /// an error matches a string or template literal of the symbols it cites.
    fn wording(&self, source: &Source, out: &mut Vec<Problem>) {
        for sink in sinks(source) {
            if source.in_test(sink.token) {
                continue;
            }
            let chain = source.chain(sink.token);
            let governing = chain
                .iter()
                .copied()
                .find(|&item| !source.items[item].doc.is_empty());
            let ported = governing.map_or(source.kind == FileKind::PortOfOne, |item| {
                source.items[item]
                    .doc
                    .iter()
                    .any(|&tag| matches!(source.tags[tag].kind, TagKind::Playwright(_)))
            });
            let exempt = source.live_tags().any(|tag| {
                matches!(tag.kind, TagKind::Deviation(..))
                    && (sink.line..=sink.literal_line).contains(&tag.attach_line)
            });
            if !ported || exempt {
                continue;
            }
            let span = governing.map(|item| {
                let item = &source.items[item];
                source.tokens[item.first].line..=source.tokens[item.last].line
            });
            let citations: Vec<&Citation> = source
                .live_tags()
                .filter(|tag| {
                    tag.header
                        || tag.doc_of.is_some_and(|item| chain.contains(&item))
                        || span.as_ref().is_none_or(|span| span.contains(&tag.line))
                })
                .filter_map(|tag| tag.kind.citation())
                .collect();
            let mut patterns = Vec::new();
            for citation in &citations {
                let Some(file) = self.playwright.get(&citation.repo) else {
                    continue;
                };
                if let Ok(blocks) = playwright::resolve(file, &citation.symbol, None) {
                    patterns.extend(playwright::message_patterns(file, &blocks));
                }
            }
            if patterns.is_empty()
                || patterns
                    .iter()
                    .any(|pattern| pattern.is_match(&sink.message))
            {
                continue;
            }
            let cited: BTreeSet<String> =
                citations.iter().map(|citation| citation.show()).collect();
            out.push(Problem::new(
                &source.path,
                sink.literal_line,
                format!(
                    "{:?} matches no message of {}",
                    sink.message,
                    cited.into_iter().collect::<Vec<_>>().join(", ")
                ),
                "copy Playwright's wording from the cited code (cite the symbol that has it), or \
                 tag the line `// Deviation: …` with why Neo words it differently; a message \
                 Playwright never sends is Neo's own",
            ));
        }
    }
}

/// R5: a `Playwright:` constant's literals equal its quotes' literals, in order. One with
/// a one-line value must quote it; one with no literals (enum variants) or a multi-line
/// value and no quote (a table, which needs a drift test) is not tied.
fn value_tie(source: &Source, index: usize, out: &mut Vec<Problem>) {
    let item = &source.items[index];
    let citations: Vec<&Citation> = item
        .doc
        .iter()
        .filter_map(|&tag| match &source.tags[tag].kind {
            TagKind::Playwright(citation) => Some(citation),
            _ => None,
        })
        .collect();
    let Some(init) = item.init.clone() else {
        return;
    };
    if citations.is_empty() || init.is_empty() {
        return;
    }
    let value = &source.tokens[init];
    let one_line = value.first().map(|token| token.line) == value.last().map(|token| token.line);
    let quotes: Vec<&str> = citations
        .iter()
        .filter_map(|citation| citation.quote.as_deref())
        .collect();
    if quotes.is_empty() {
        if one_line {
            out.push(Problem::new(
                &source.path,
                item.line,
                format!(
                    "`{}` is tagged `Playwright:` without a quote of its value",
                    item.name
                ),
                "quote Playwright's value: `Playwright: `<path>` `<symbol>`: `<code>``",
            ));
        }
        return;
    }
    let ours = lex::literals(value);
    if ours.is_empty() {
        return;
    }
    let theirs: Vec<Lit> = quotes
        .iter()
        .flat_map(|quote| lex::literals(&lex::js(quote).tokens))
        .collect();
    if ours != theirs {
        out.push(Problem::new(
            &source.path,
            item.line,
            format!(
                "`{}` is {}; its quote says {}",
                item.name,
                lex::show(&ours),
                lex::show(&theirs)
            ),
            "a `Playwright:` value is Playwright's: make them equal, or tag a value Neo \
             chose `Deviation:` and compute it from the Playwright one",
        ));
    }
}

/// How a committed register differs from the generated one, by row, with the first few.
fn stale_rows(generated: &str, committed: &str) -> String {
    let rows = |text: &str| -> BTreeSet<String> {
        text.lines()
            .filter(|line| line.starts_with("| ") && !line.starts_with("| File"))
            .filter(|line| !line.starts_with("| ---"))
            .map(str::to_string)
            .collect()
    };
    let (want, have) = (rows(generated), rows(committed));
    let added: Vec<&String> = want.difference(&have).collect();
    let removed: Vec<&String> = have.difference(&want).collect();
    if added.is_empty() && removed.is_empty() {
        return "stale: its rows match, its other text does not".to_string();
    }
    let mut what = format!(
        "stale: {} row{} to add, {} to remove",
        added.len(),
        if added.len() == 1 { "" } else { "s" },
        removed.len()
    );
    what.extend(added.iter().take(3).map(|row| format!("\n      + {row}")));
    what.extend(removed.iter().take(3).map(|row| format!("\n      - {row}")));
    what
}

/// R8: a `Duration` built from a literal amount outside a constant's value.
fn duration_literals(source: &Source, out: &mut Vec<Problem>) {
    let tokens = &source.tokens;
    for (index, token) in tokens.iter().enumerate() {
        let constructor = tokens.get(index + 2).and_then(Token::ident).filter(|name| {
            name.starts_with("from_") || name.starts_with("try_from_") || *name == "new"
        });
        let call = token.is("Duration")
            && tokens
                .get(index + 1)
                .is_some_and(|next| next.is_punct("::"))
            && tokens.get(index + 3).is_some_and(|next| next.is_punct("("));
        let Some(constructor) = constructor.filter(|_| call) else {
            continue;
        };
        let args: Vec<&Token> = tokens[index + 4..]
            .iter()
            .scan(1_usize, |depth, token| {
                if token.is_punct("(") {
                    *depth += 1;
                } else if token.is_punct(")") {
                    *depth -= 1;
                }
                (*depth > 0).then_some(token)
            })
            .collect();
        let literal = !args.is_empty()
            && args.iter().all(|arg| {
                matches!(arg.tok, Tok::Lit(Lit::Num(_)))
                    || ["*", "+", "-", "/", "(", ")", ","]
                        .iter()
                        .any(|punct| arg.is_punct(punct))
            });
        let in_constant = source.items.iter().any(|item| {
            matches!(item.kind, ItemKind::Const | ItemKind::Static)
                && item.init.as_ref().is_some_and(|init| init.contains(&index))
        });
        if literal && !in_constant && !source.in_test(index) {
            out.push(Problem::new(
                &source.path,
                token.line,
                format!("`Duration::{constructor}` of a literal outside a constant"),
                "name the duration as a `const` with a provenance line (`Playwright:` with its \
                 quote, `Neo:`, `Platform:` or `Unexplained:`)",
            ));
        }
    }
}

/// A message literal sent to a call-log or error sink, with each placeholder as `{}`.
struct Sink {
    token: usize,
    line: usize,
    literal_line: usize,
    message: String,
}

/// The call-log and Playwright-error sinks: `progress.log`, `progress.name_locator`,
/// `PlaywrightError::error` (and `WireError::error`/`protocol` once the error model lands);
/// in TypeScript `createStacklessError` and `new Error`. A message built without a literal is
/// not seen.
fn sinks(source: &Source) -> Vec<Sink> {
    let tokens = &source.tokens;
    let mut found = Vec::new();
    for index in 0..tokens.len() {
        let argument = match source.lang {
            Lang::Rust => rust_sink(tokens, index),
            Lang::Ts => ts_sink(tokens, index),
        };
        let Some((argument, format)) = argument else {
            continue;
        };
        let message = match &tokens[argument].tok {
            Tok::Lit(Lit::Str(text)) if format && source.lang == Lang::Rust => holes(text),
            Tok::Lit(Lit::Str(text)) => text.clone(),
            Tok::Template(parts) => parts
                .iter()
                .map(|part| match part {
                    Part::Text(text) => text.as_str(),
                    Part::Expr(_) => "{}",
                })
                .collect(),
            _ => continue,
        };
        if message.replace("{}", "").trim().is_empty() {
            continue;
        }
        found.push(Sink {
            token: index,
            line: tokens[index].line,
            literal_line: tokens[argument].line,
            message,
        });
    }
    found
}

/// The first argument of a Rust sink call at `index`, and whether it is a `format!` string.
fn rust_sink(tokens: &[Token], index: usize) -> Option<(usize, bool)> {
    let at = |offset: usize| tokens.get(index + offset);
    let log = tokens[index].is("progress")
        && at(1)?.is_punct(".")
        && matches!(at(2)?.ident(), Some("log" | "name_locator"));
    let error = matches!(tokens[index].ident(), Some("PlaywrightError" | "WireError"))
        && at(1)?.is_punct("::")
        && matches!(at(2)?.ident(), Some("error" | "protocol"));
    if !(log || error) || !at(3)?.is_punct("(") {
        return None;
    }
    let format = at(4)?.is("format") && at(5)?.is_punct("!") && at(6)?.is_punct("(");
    Some((index + if format { 7 } else { 4 }, format))
}

fn ts_sink(tokens: &[Token], index: usize) -> Option<(usize, bool)> {
    let open = if tokens[index].is("createStacklessError") {
        index + 1
    } else if tokens[index].is("new") && tokens.get(index + 1)?.is("Error") {
        index + 2
    } else {
        return None;
    };
    tokens.get(open)?.is_punct("(").then_some((open + 1, false))
}

/// A Rust format string with each `{…}` placeholder written `{}`, and `{{`/`}}` as braces.
fn holes(format: &str) -> String {
    let mut out = String::with_capacity(format.len());
    let mut chars = format.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            '{' => {
                for inner in chars.by_ref() {
                    if inner == '}' {
                        break;
                    }
                }
                out.push_str("{}");
            }
            _ => out.push(ch),
        }
    }
    out
}
