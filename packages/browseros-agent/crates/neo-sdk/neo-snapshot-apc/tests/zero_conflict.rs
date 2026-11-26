//! The zero-conflict gate (design 10.1): every line with a ref that a snapshot prints, in Neo's
//! format and in Playwright's, must reach its element through the `getByRole` an agent writes
//! from it, over every recorded fixture and 10,000 generated pages. A sibling test proves the
//! selector escaping that query relies on equal to the bundled prelude's, run in QuickJS.
//!
//! `NEO_ZERO_CONFLICT_TREES=<n>` changes how many pages are generated, and
//! `NEO_ZERO_CONFLICT_SEED=<seed>` (decimal or `0x…`) generates that one page and prints its two
//! texts, to replay a miss. The default run is fixed and deterministic.

#[path = "support/prelude_chunks.rs"]
#[expect(
    dead_code,
    reason = "the escaping test compares chunks, so the helper's locator text goes unused"
)]
mod prelude_chunks;
#[path = "support/zero_conflict.rs"]
mod zero_conflict;
#[path = "support/zero_conflict_gen.rs"]
mod zero_conflict_gen;

use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::thread;
use std::time::Instant;

use neo_snapshot::Tristate;
use neo_snapshot_apc::proto::FormControlType;
use neo_snapshot_apc::select::SelectorBody;
use neo_snapshot_apc::tree::{ApcBuilder, NodeBuilder};
use prelude_chunks::Prelude;
use zero_conflict::{ByRole, ByRoleOptions, Format, LineCheck, PageCheck, Verdict};
use zero_conflict_gen::{BASE_SEED, Coverage};

/// The fixtures recorded today. A new recording raises the count; a lost one fails the gate.
const MIN_FIXTURES: usize = 16;

/// The generated pages of the default run.
const DEFAULT_TREES: u64 = 10_000;

/// How many misses a failure lists; the total is always given.
const LISTED_MISSES: usize = 50;

/// The default run's lines of the one known miss class, [`Verdict::NeoDroppedAriaHidden`],
/// pinned exactly until Neo's printer carries `[aria-hidden]` onto the lines it flattens (the
/// captain's decision of 2026-11-25; its own pull request). Any other count fails, so the fix
/// landing fails this gate too: then this pin goes and the gate is 100%. The fixtures hold none.
const KNOWN_NEO_DROPPED_ARIA_HIDDEN: usize = 17_776;

/// Writes one line of the gate's report, which `--nocapture` shows.
#[expect(
    clippy::print_stdout,
    reason = "the summary lines are the gate's report; the test harness has no other channel"
)]
fn report(line: &str) {
    println!("{line}");
}

/// One miss and the page it was on.
struct Miss {
    page: String,
    check: LineCheck,
}

impl Miss {
    fn line(&self) -> String {
        let check = &self.check;
        let target = check.target.as_ref().map_or_else(
            || "none".to_owned(),
            |key| format!("{}/{}", key.doc, key.node),
        );
        format!(
            "{} | {} line {} | {:?} | {} | {} | target {target}",
            self.page, check.format, check.line.number, check.line.text, check.query, check.verdict
        )
    }
}

/// Lines with a ref in one format: how many were reached, and how many print a label no role
/// query names (`generic` for an element with no role, `iframe`). Every other line is a miss.
#[derive(Clone, Copy, Default)]
struct Counts {
    lines: usize,
    reached: usize,
    labels: usize,
}

impl Counts {
    fn merge(&mut self, other: Self) {
        self.lines += other.lines;
        self.reached += other.reached;
        self.labels += other.labels;
    }
}

/// One corpus part's counts and misses.
#[derive(Default)]
struct Tally {
    pages: usize,
    neo: Counts,
    playwright: Counts,
    /// Each miss with the position of its page in the part, which orders the listing.
    misses: Vec<(usize, Miss)>,
}

impl Tally {
    fn add(&mut self, order: usize, check: &PageCheck) {
        self.pages += 1;
        for line in &check.lines {
            let counts = match line.format {
                Format::Neo => &mut self.neo,
                Format::Playwright => &mut self.playwright,
            };
            counts.lines += 1;
            counts.reached += usize::from(line.verdict.reached());
            counts.labels += usize::from(matches!(line.verdict, Verdict::Label(_)));
        }
        self.misses.extend(check.misses().map(|line| {
            let miss = Miss {
                page: check.page.clone(),
                check: line.clone(),
            };
            (order, miss)
        }));
    }

    fn merge(&mut self, other: Self) {
        self.pages += other.pages;
        self.neo.merge(other.neo);
        self.playwright.merge(other.playwright);
        self.misses.extend(other.misses);
    }

    fn summary(&self, part: &str, unit: &str) -> String {
        format!(
            "zero-conflict {part}: {} {unit}; neo {} lines, {} reached, {} labels; playwright {} \
             lines, {} reached, {} labels; {} misses",
            self.pages,
            self.neo.lines,
            self.neo.reached,
            self.neo.labels,
            self.playwright.lines,
            self.playwright.reached,
            self.playwright.labels,
            self.misses.len()
        )
    }
}

/// Every recorded fixture, checked as a fresh tab would capture it, in file-name order.
fn fixture_checks() -> Result<Vec<PageCheck>, Box<dyn Error>> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut paths = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "pb") {
            paths.push(path);
        }
    }
    paths.sort();
    let mut checks = Vec::new();
    for path in paths {
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| format!("{} has no UTF-8 stem", path.display()))?;
        let apc = neo_snapshot_apc::decode_protobuf(&fs::read(&path)?)?;
        checks.push(zero_conflict::check_apc(stem, apc)?);
    }
    Ok(checks)
}

/// Which generated pages a run checks.
#[derive(Clone, Copy)]
enum Run {
    /// `DEFAULT_TREES` pages from `BASE_SEED`, and the coverage gate.
    Default,
    /// `NEO_ZERO_CONFLICT_TREES` pages from `BASE_SEED`.
    Trees(u64),
    /// `NEO_ZERO_CONFLICT_SEED`'s page alone, its texts printed.
    Seed(u64),
}

fn run_choice() -> Result<Run, Box<dyn Error>> {
    if let Some(seed) = env_var("NEO_ZERO_CONFLICT_SEED")? {
        let seed = match seed.strip_prefix("0x") {
            Some(hex) => u64::from_str_radix(hex, 16)?,
            None => seed.parse()?,
        };
        return Ok(Run::Seed(seed));
    }
    Ok(match env_var("NEO_ZERO_CONFLICT_TREES")? {
        Some(trees) => Run::Trees(trees.parse()?),
        None => Run::Default,
    })
}

fn env_var(name: &str) -> Result<Option<String>, Box<dyn Error>> {
    match env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(format!("{name}: {error}").into()),
    }
}

/// What one worker found over its share of the seeds.
#[derive(Default)]
struct Outcome {
    tally: Tally,
    coverage: Coverage,
}

/// Generates and checks `seeds[worker]`, `seeds[worker + workers]`, and so on.
fn run_share(seeds: &[u64], worker: usize, workers: usize) -> Result<Outcome, String> {
    let mut outcome = Outcome::default();
    for (order, &seed) in seeds.iter().enumerate().skip(worker).step_by(workers) {
        let page = zero_conflict_gen::generate(seed)?;
        let check = zero_conflict::check_apc(&format!("seed {seed:#x}"), page.apc)
            .map_err(|error| format!("seed {seed:#x}: {error}"))?;
        outcome.coverage.count(&page.facts, &check);
        outcome.tally.add(order, &check);
    }
    Ok(outcome)
}

/// Every seed's page, spread over the machine's threads. Each thread owns its share's results
/// until the join; they merge in seed order, so the listing is the same however the threads
/// interleave.
fn generated(seeds: &[u64]) -> Result<Outcome, Box<dyn Error>> {
    let workers = thread::available_parallelism().map_or(1, usize::from);
    let shares: Vec<Result<Outcome, String>> = thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|worker| scope.spawn(move || run_share(seeds, worker, workers)))
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|_| Err("a generator worker panicked".to_owned()))
            })
            .collect()
    });
    let mut merged = Outcome::default();
    for share in shares {
        let share = share?;
        merged.coverage.merge(&share.coverage);
        merged.tally.merge(share.tally);
    }
    merged.tally.misses.sort_by_key(|(order, _)| *order);
    Ok(merged)
}

/// Prints one seed's two texts and its count, to replay a miss by hand.
fn replay(seed: u64) -> Result<(), Box<dyn Error>> {
    let page = zero_conflict_gen::generate(seed)?;
    let check = zero_conflict::check_apc(&format!("seed {seed:#x}"), page.apc)?;
    for (format, text) in Format::ALL.iter().zip(&check.texts) {
        report(&format!("--- seed {seed:#x}, {format} text ---\n{text}"));
    }
    report(&format!(
        "--- seed {seed:#x}: {} of {} lines reached, {} labels ---",
        check.reached(),
        check.total(),
        check.labels()
    ));
    Ok(())
}

#[test]
fn every_printed_ref_reaches_its_element_by_role() -> Result<(), Box<dyn Error>> {
    let started = Instant::now();
    let fixtures = fixture_checks()?;
    assert!(
        fixtures.len() >= MIN_FIXTURES,
        "found {} fixtures; every recorded page is part of the corpus, so one went missing",
        fixtures.len()
    );
    let mut recorded = Tally::default();
    for (order, check) in fixtures.iter().enumerate() {
        recorded.add(order, check);
    }
    report(&recorded.summary("fixtures", "pages"));

    let run = run_choice()?;
    let seeds: Vec<u64> = match run {
        Run::Default => (0..DEFAULT_TREES).map(|i| BASE_SEED + i).collect(),
        Run::Trees(trees) => (0..trees).map(|i| BASE_SEED + i).collect(),
        Run::Seed(seed) => vec![seed],
    };
    if let Run::Seed(seed) = run {
        replay(seed)?;
    }
    let outcome = generated(&seeds)?;
    report(&outcome.tally.summary("generated", "trees"));
    let counters = outcome.coverage.counters();
    let mut line = String::from("zero-conflict coverage:");
    for (label, count) in &counters {
        let _ = write!(line, " {label}={count};");
    }
    report(&line);
    report(&format!(
        "zero-conflict wall time: {:.1} s",
        started.elapsed().as_secs_f64()
    ));

    // A shorter or single-seed run cannot be expected to reach every shape.
    let gaps: Vec<&str> = if matches!(run, Run::Default) {
        counters
            .iter()
            .filter(|(_, count)| *count == 0)
            .map(|(label, _)| label.as_str())
            .collect()
    } else {
        Vec::new()
    };
    // The known class is pinned on the generated corpus alone; on a fixture it is a miss.
    let known = |miss: &Miss| miss.check.verdict == Verdict::NeoDroppedAriaHidden;
    let known_generated = outcome
        .tally
        .misses
        .iter()
        .filter(|(_, miss)| known(miss))
        .count();
    report(&format!(
        "zero-conflict known class (Neo's format dropped the [aria-hidden] line): {known_generated} \
         generated lines, pinned at {KNOWN_NEO_DROPPED_ARIA_HIDDEN} for the default run"
    ));
    let misses: Vec<&Miss> = recorded
        .misses
        .iter()
        .map(|(_, miss)| miss)
        .chain(
            outcome
                .tally
                .misses
                .iter()
                .map(|(_, miss)| miss)
                .filter(|miss| !known(miss)),
        )
        .collect();
    let mut message = String::new();
    if !gaps.is_empty() {
        let _ = writeln!(
            message,
            "the generator claims these shapes and no printed ref line carried one: {gaps:?}"
        );
    }
    if matches!(run, Run::Default) && known_generated != KNOWN_NEO_DROPPED_ARIA_HIDDEN {
        let _ = writeln!(
            message,
            "{known_generated} generated lines are Neo's dropped [aria-hidden] class, pinned at \
             {KNOWN_NEO_DROPPED_ARIA_HIDDEN}: if Neo's printer now carries the mark, remove the \
             pin and the class so the gate is 100%; otherwise the generator changed, so re-pin"
        );
    }
    if !misses.is_empty() {
        let _ = writeln!(
            message,
            "{} printed ref lines were not reached by their getByRole ({} listed):",
            misses.len(),
            misses.len().min(LISTED_MISSES)
        );
        for miss in misses.iter().take(LISTED_MISSES) {
            let _ = writeln!(message, "  {}", miss.line());
        }
    }
    assert!(
        message.is_empty(),
        "an agent must reach every element it is shown by the getByRole its line spells \
         (design 10.1):\n{message}"
    );
    Ok(())
}

/// The known class on its smallest page: a box-visible `aria-hidden="true"` `<div>` around one
/// button, as a carousel's off-screen slide is. Playwright's format nests the button under the
/// wrapper's `[aria-hidden]` line, so the agent's query asks `includeHidden` and reaches it.
/// Neo's flat format prints the button with no sign of the wrapper, so the agent's query skips
/// it. Once Neo's printer carries the mark, the Neo line is reached and this test fails beside
/// the corpus pin: both go, and the gate is 100%.
#[test]
fn neo_format_drops_the_aria_hidden_line_that_hides_a_button() -> Result<(), Box<dyn Error>> {
    let button = NodeBuilder::form_control(FormControlType::ButtonButton)
        .dom_id(5)
        .tag("button")
        .bounding_box(0, 40, 120, 24)
        .aria_hidden(true)
        .child(NodeBuilder::text("Next slide").aria_hidden(true));
    let wrapper = NodeBuilder::container()
        .dom_id(4)
        .bounding_box(0, 32, 800, 40)
        .aria_hidden(true)
        .child(button);
    let body = NodeBuilder::element("body")
        .dom_id(3)
        .bounding_box(0, 0, 800, 600)
        .child(wrapper);
    let html = NodeBuilder::element("html")
        .dom_id(2)
        .bounding_box(0, 0, 800, 600)
        .child(body);
    let apc = ApcBuilder::new("doc-known-class")
        .contract_version(6)
        .viewport(0, 0, 800, 600)
        .root(
            NodeBuilder::root()
                .dom_id(1)
                .bounding_box(0, 0, 800, 600)
                .child(html),
        )
        .build();
    let check = zero_conflict::check_apc("known class", apc)?;
    let [neo_text, playwright_text] = &check.texts;
    let button_line = |format: Format| {
        check
            .lines
            .iter()
            .find(|line| line.format == format && line.line.role == "button")
    };
    let neo = button_line(Format::Neo)
        .ok_or_else(|| format!("Neo's format prints the button with its ref:\n{neo_text}"))?;
    let playwright = button_line(Format::Playwright).ok_or_else(|| {
        format!("Playwright's format prints the button with its ref:\n{playwright_text}")
    })?;
    assert_eq!(
        neo.verdict,
        Verdict::NeoDroppedAriaHidden,
        "Neo's flat format prints the hidden button with no sign of its [aria-hidden] wrapper, \
         so the agent's getByRole skips it:\n{neo_text}"
    );
    assert!(
        playwright.verdict.reached() && playwright.line.under_aria_hidden,
        "Playwright's format nests the button under the [aria-hidden] line, so the agent asks \
         includeHidden and reaches it ({}):\n{playwright_text}",
        playwright.verdict
    );
    report(&format!(
        "known class, minimal page\n--- neo ---\n{neo_text}\n--- playwright ---\n{playwright_text}\n\
         neo line {}: {} -> {}\nplaywright line {}: {} -> {}",
        neo.line.number,
        neo.query,
        neo.verdict,
        playwright.line.number,
        playwright.query,
        playwright.verdict
    ));
    Ok(())
}

/// Option sets that together pass every `getByRole` option, both forms of `checked` and
/// `pressed`, and `false` for each boolean, each under a role.
fn option_sets(name: &str) -> Vec<ByRole> {
    let named = |exact: Option<bool>| ByRoleOptions {
        name: Some(name.to_owned()),
        exact,
        ..ByRoleOptions::default()
    };
    let with = |role: &str, options: ByRoleOptions| ByRole {
        role: role.to_owned(),
        options,
    };
    vec![
        with("button", named(Some(true))),
        with("link", named(Some(false))),
        with("heading", named(None)),
        with(
            "checkbox",
            ByRoleOptions {
                checked: Some(Tristate::True),
                ..named(Some(true))
            },
        ),
        with(
            "menuitemcheckbox",
            ByRoleOptions {
                checked: Some(Tristate::Mixed),
                pressed: Some(Tristate::Mixed),
                ..named(Some(true))
            },
        ),
        with(
            "treeitem",
            ByRoleOptions {
                disabled: Some(true),
                selected: Some(true),
                expanded: Some(true),
                include_hidden: Some(true),
                level: Some(3),
                pressed: Some(Tristate::True),
                ..named(Some(true))
            },
        ),
        with(
            "tab",
            ByRoleOptions {
                checked: Some(Tristate::False),
                disabled: Some(false),
                selected: Some(false),
                expanded: Some(false),
                include_hidden: Some(false),
                level: Some(1),
                pressed: Some(Tristate::False),
                ..named(Some(true))
            },
        ),
    ]
}

#[test]
fn by_role_selector_and_chunk_equal_the_preludes() -> Result<(), Box<dyn Error>> {
    let prelude = Prelude::load()?;
    let mut cases: BTreeMap<String, ByRole> = BTreeMap::new();
    let mut add = |by_role: ByRole| {
        let key = format!("{}\u{0}{}", by_role.role, by_role.script_options());
        cases.entry(key).or_insert(by_role);
    };
    for name in zero_conflict_gen::palette_names() {
        option_sets(&name).into_iter().for_each(&mut add);
    }
    add(ByRole {
        role: "generic".to_owned(),
        options: ByRoleOptions::default(),
    });
    for check in fixture_checks()? {
        for line in &check.lines {
            add(ByRole::from_line(&line.line));
        }
    }
    let mut mismatches = Vec::new();
    for by_role in cases.values() {
        let call = format!(
            "page.getByRole({}, {})",
            serde_json::to_string(&by_role.role)?,
            serde_json::to_string(&by_role.script_options())?
        );
        // `_selector` leaves QuickJS inside a chunk: as a JSON string literal, the body of an
        // `internal:describe` part, which neither `>>` nor a quote in the selector can split.
        let carrier = prelude.chunk(&format!(
            "page.locator('internal:describe=' + JSON.stringify({call}._selector))"
        ))?;
        let Some(SelectorBody::Text(literal)) = carrier.parts.first().map(|part| &part.body) else {
            return Err(format!("{call}: the carrier chunk has no text body").into());
        };
        let selector: String = serde_json::from_str(literal)?;
        if by_role.selector() != selector {
            mismatches.push(format!(
                "{call}: selector {:?}, prelude {selector:?}",
                by_role.selector()
            ));
        }
        let chunk = prelude.chunk(&call)?;
        if by_role.chunk() != chunk {
            mismatches.push(format!(
                "{call}: chunk {:?}, prelude {chunk:?}",
                by_role.chunk()
            ));
        }
    }
    report(&format!(
        "zero-conflict escaping: {} getByRole cases, {} mismatches",
        cases.len(),
        mismatches.len()
    ));
    assert!(
        mismatches.is_empty(),
        "the gate's queries must be the ones a script's getByRole builds, byte for byte:\n{}",
        mismatches.join("\n")
    );
    Ok(())
}
