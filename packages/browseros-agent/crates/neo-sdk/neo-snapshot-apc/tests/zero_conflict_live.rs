//! The zero-conflict fuzzer's live half (design 10.2 and 10.3): in a browser carrying BrowserOS's
//! APC contract, every line with a ref that a snapshot prints reaches its element through the
//! `getByRole` an agent writes from it, two captures of an unchanged page agree, every printed ref
//! hydrates in its frame's session, and the tree's role sets are compared with the vendored
//! Playwright engine's (recall).
//!
//! Run from `packages/browseros-agent`, in a heavy slot:
//! ```sh
//! NEO_E2E_BROWSER="/Users/shadowfax/code/chromium-1/src/out/Default_browseros_arm64/BrowserOS Dev.app/Contents/MacOS/BrowserOS Dev" \
//!   bun run test:browser -- cargo test -p neo-snapshot-apc --test zero_conflict_live -- --ignored --nocapture
//! ```
//! That runs the edge pack, every recorded fixture on the fixture server. The 50-site list runs
//! through `crates/neo-sdk/tools/zero-conflict/run.sh`, which names the list and the private
//! folder its pages' captures go to; without them the site test says it skipped. A site
//! capture saved there replays offline with `NEO_ZERO_CONFLICT_REPLAY=<folder>`.

#[path = "support/fixture_pages.rs"]
#[expect(
    dead_code,
    reason = "the by-name lookup, the typed canaries, the capture_apc method and the \
              capture_fixture_* helpers serve live_capture and the recorder"
)]
mod fixture_pages;
#[path = "support/fixture_server.rs"]
mod fixture_server;
#[path = "support/oracle.rs"]
mod oracle;
#[path = "support/oracle_capture.rs"]
mod oracle_capture;
#[path = "support/oracle_join.rs"]
mod oracle_join;
#[path = "support/zero_conflict.rs"]
#[expect(
    dead_code,
    reason = "the script-options form of a query and PageCheck's own counts serve the offline \
              gate; this test counts lines itself"
)]
mod zero_conflict;
#[path = "support/zero_conflict_live.rs"]
mod zero_conflict_live;

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use std::{env, fs};

use browseros_cdp::{CdpClient, SessionId};
use fixture_pages::{ApcContract, FIXTURES, Fixture};
use fixture_server::FixtureServer;
use oracle::Oracle;
use oracle_capture::OracleEngine;
use serde_json::{Value, json};
use tokio::time::{self, Instant};
use zero_conflict::{Format, LineCheck, PageCheck, Verdict};
use zero_conflict_live::{Instability, MeasureError, Measured, Recall};

/// The depth of `C4` in `nested-cross-site-frames` (Top, B1, C2, A3, C4): the edge pack must
/// hydrate a ref this many documents down.
const DEEPEST_FRAME: usize = 4;

/// The site gate (design 10.3): at least 99.5% of role lines reached, as a fraction in
/// thousandths so the comparison is exact.
const SITE_GATE_PER_MILLE: usize = 995;

/// Bound on a site's `document.readyState` reaching `complete`. A page still loading then is
/// measured as it stands, its state reported.
const SITE_READY_BOUND: Duration = Duration::from_secs(30);

/// Settle time after a site is ready, for scripts that render after `load`.
const SITE_SETTLE: Duration = Duration::from_secs(3);

/// Bound on one site's oracle read as a whole. The measurement after it is not bounded so: its
/// CDP steps are, and its check is CPU work no timer could cut short.
const SITE_ORACLE_BOUND: Duration = Duration::from_mins(5);

/// How often the ready state is read while waiting.
const SITE_POLL: Duration = Duration::from_millis(250);

/// How many missing backendNodeIds a report lists per frame and role.
const LISTED_MISSING: usize = 20;

/// Writes one line of a test's report, which `--nocapture` shows.
#[expect(
    clippy::print_stdout,
    reason = "the rows are the run's evidence; the test harness has no other channel"
)]
fn report(line: &str) {
    println!("{line}");
}

/// `part` of `whole` as a percentage with two decimals, rounded down, so 100.00% means every one.
struct Percent {
    part: usize,
    whole: usize,
}

impl fmt::Display for Percent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.part * 10_000).checked_div(self.whole) {
            None => f.write_str("n/a"),
            Some(hundredths) => write!(f, "{}.{:02}%", hundredths / 100, hundredths % 100),
        }
    }
}

/// Lines with a ref in one capture, or one format of it, by what their queries came to.
#[derive(Clone, Copy, Debug, Default)]
struct LineCounts {
    lines: usize,
    reached: usize,
    labels: usize,
    misses: usize,
}

impl LineCounts {
    fn of<'a>(lines: impl IntoIterator<Item = &'a LineCheck>) -> Self {
        let mut counts = Self::default();
        for line in lines {
            counts.lines += 1;
            counts.reached += usize::from(line.verdict.reached());
            counts.labels += usize::from(matches!(line.verdict, Verdict::Label(_)));
            counts.misses += usize::from(line.verdict.is_miss());
        }
        counts
    }

    fn of_format(check: &PageCheck, format: Format) -> Self {
        Self::of(check.lines.iter().filter(|line| line.format == format))
    }

    /// The lines a role query names: every line but the labels. A page's percentage is over
    /// these.
    fn role_lines(&self) -> usize {
        self.lines - self.labels
    }

    fn merge(&mut self, other: Self) {
        self.lines += other.lines;
        self.reached += other.reached;
        self.labels += other.labels;
        self.misses += other.misses;
    }
}

/// What hydrating one page's printed refs came to.
#[derive(Clone, Copy, Debug, Default)]
struct HydrationCounts {
    refs: usize,
    hydrated: usize,
    max_depth: usize,
    closed_shadow: usize,
}

impl HydrationCounts {
    fn of(measured: &Measured) -> Self {
        let mut counts = Self {
            refs: measured.hydrated.len(),
            ..Self::default()
        };
        for hydrated in measured
            .hydrated
            .iter()
            .filter_map(|r| r.outcome.as_ref().ok())
        {
            counts.hydrated += 1;
            counts.max_depth = counts.max_depth.max(hydrated.depth);
            counts.closed_shadow += usize::from(hydrated.closed_shadow);
        }
        counts
    }

    fn failed(&self) -> usize {
        self.refs - self.hydrated
    }

    fn merge(&mut self, other: Self) {
        self.refs += other.refs;
        self.hydrated += other.hydrated;
        self.max_depth = self.max_depth.max(other.max_depth);
        self.closed_shadow += other.closed_shadow;
    }
}

/// Recall over one page or many: the page's role-set elements and how many Rust found.
#[derive(Clone, Copy, Debug, Default)]
struct RecallCounts {
    page: usize,
    found: usize,
    extra: usize,
}

impl RecallCounts {
    fn of(measured: &Measured) -> Option<Self> {
        measured.recall.as_ref().map(|recall| Self {
            page: recall.page(),
            found: recall.found(),
            extra: recall.extra(),
        })
    }

    fn merge(&mut self, other: Self) {
        self.page += other.page;
        self.found += other.found;
        self.extra += other.extra;
    }

    fn recall(&self) -> Percent {
        Percent {
            part: self.found,
            whole: self.page,
        }
    }
}

/// Every page's counts added up, for a run's total row.
#[derive(Debug, Default)]
struct Totals {
    lines: LineCounts,
    hydration: HydrationCounts,
    recall: RecallCounts,
    unstable_pages: usize,
    contracts: BTreeSet<u32>,
}

impl Totals {
    /// Adds `measured`, its recall when it was measured against an oracle.
    fn add(&mut self, measured: &Measured) {
        self.lines
            .merge(LineCounts::of(&measured.first.check.lines));
        self.hydration.merge(HydrationCounts::of(measured));
        if let Some(counts) = RecallCounts::of(measured) {
            self.recall.merge(counts);
        }
        self.unstable_pages += usize::from(!measured.unstable.is_empty());
        self.contracts.insert(measured.contract_version);
    }
}

/// Loads `fixture` as the recorder does (contract fields typed into), reads its oracle and
/// measures it; the tab is closed again, on failure too.
async fn measure_fixture(
    client: &CdpClient,
    server: &FixtureServer,
    engine: &OracleEngine,
    browser: &str,
    fixture: &'static Fixture,
) -> Result<Measured, Box<dyn Error>> {
    let loaded = fixture_pages::load_fixture(client, server, fixture, ApcContract::Present).await?;
    let measured = async {
        let oracle = oracle_capture::read_oracle(
            client,
            fixture.name,
            &loaded.session,
            &loaded.target_id,
            engine,
            browser,
        )
        .await?;
        let measured = zero_conflict_live::measure(
            client,
            fixture.name,
            &loaded.session,
            &loaded.target_id,
            Some(&oracle),
            loaded.before_capture(client),
        )
        .await?;
        Ok::<_, Box<dyn Error>>(measured)
    }
    .await;
    loaded.close(client).await;
    measured
}

/// Every way `measured` breaks the edge pack's rules: a line of either capture neither reached
/// nor a label (with its line number, verdict, text and query), a difference between the
/// captures, a ref that did not hydrate.
fn edge_failures(page: &str, measured: &Measured) -> Vec<String> {
    let mut failures = Vec::new();
    for (capture, check) in [(1, &measured.first.check), (2, &measured.second.check)] {
        failures.extend(check.misses().map(|line| {
            format!(
                "{page} capture #{capture} {} line {}: {} | {:?} | {}",
                line.format, line.line.number, line.verdict, line.line.text, line.query
            )
        }));
    }
    failures.extend(
        measured
            .unstable
            .iter()
            .map(|instability| format!("{page}: unstable: {instability:?}")),
    );
    for hydrated in &measured.hydrated {
        if let Err(failure) = &hydrated.outcome {
            failures.push(format!(
                "{page} ref {}: did not hydrate: {failure}",
                hydrated.r#ref
            ));
        }
    }
    failures
}

/// One fixture's row of the edge-pack table.
fn edge_row(page: &str, measured: &Measured) -> String {
    let check = &measured.first.check;
    let counts = LineCounts::of(&check.lines);
    let hydration = HydrationCounts::of(measured);
    let recall = RecallCounts::of(measured).unwrap_or_default();
    format!(
        "{page} | {} | {} | {} | {} | {} | {}/{} | {} | {} | {} | {} ({}/{}) | {}",
        LineCounts::of_format(check, Format::Neo).lines,
        LineCounts::of_format(check, Format::Playwright).lines,
        counts.reached,
        counts.labels,
        counts.misses,
        hydration.hydrated,
        hydration.refs,
        hydration.max_depth,
        hydration.closed_shadow,
        if measured.unstable.is_empty() {
            "yes"
        } else {
            "no"
        },
        recall.recall(),
        recall.found,
        recall.page,
        recall.extra,
    )
}

/// Each frame and role whose page set Rust did not wholly find, with the missing backendNodeIds
/// and why Rust's set was unread, if it was. Recall is reported, not gated (design 10.2).
fn recall_gaps(page: &str, measured: &Measured) -> Vec<String> {
    let Some(recall) = &measured.recall else {
        return Vec::new();
    };
    let mut gaps: Vec<String> = recall
        .roles
        .iter()
        .filter(|role| role.found < role.page || role.extra > 0)
        .map(|role| {
            format!(
                "  recall: {page} frame {} role {}: {}/{} found, {} extra, missing {:?}{}",
                role.frame,
                role.role,
                role.found,
                role.page,
                role.extra,
                role.missing,
                role.unread
                    .as_ref()
                    .map_or_else(String::new, |why| format!(" (unread: {why})"))
            )
        })
        .collect();
    if recall.frames_without_document > 0 {
        gaps.push(format!(
            "  recall: {page}: {} oracle frames have no document in the tree",
            recall.frames_without_document
        ));
    }
    gaps
}

#[tokio::test]
#[ignore = "needs a BrowserOS build carrying the APC contract on BROWSEROS_CDP_PORT; run it through bun run test:browser"]
async fn edge_pack_lines_reach_their_elements_and_refs_hydrate() -> Result<(), Box<dyn Error>> {
    let engine = OracleEngine::new()?;
    let server = FixtureServer::start(fixture_pages::all_fixture_pages()).await?;
    let client = fixture_pages::connect_cdp().await?;
    let result = async {
        let browser = oracle_capture::browser_product(&client).await?;
        report(&format!("zero-conflict edge pack: browser {browser}"));
        report(
            "page | neo lines | playwright lines | reached | labels | misses | refs hydrated | \
             max depth | closed-root refs | stable | recall | extra",
        );
        let mut failures = Vec::new();
        let mut totals = Totals::default();
        for fixture in FIXTURES {
            let measured = measure_fixture(&client, &server, &engine, &browser, fixture).await?;
            failures.extend(edge_failures(fixture.name, &measured));
            report(&edge_row(fixture.name, &measured));
            for gap in recall_gaps(fixture.name, &measured) {
                report(&gap);
            }
            totals.add(&measured);
        }
        let Totals {
            lines,
            hydration,
            recall,
            unstable_pages,
            contracts,
        } = &totals;
        report(&format!(
            "total | {} lines | {} reached | {} labels | {} misses | {}/{} refs hydrated | max \
             depth {} | {} closed-root refs | {unstable_pages} unstable pages | recall {} ({}/{}) \
             | {} extra | contract_version {contracts:?}",
            lines.lines,
            lines.reached,
            lines.labels,
            lines.misses,
            hydration.hydrated,
            hydration.refs,
            hydration.max_depth,
            hydration.closed_shadow,
            recall.recall(),
            recall.found,
            recall.page,
            recall.extra,
        ));

        assert!(
            failures.is_empty(),
            "the edge pack holds no miss of any kind, prints stably and every ref hydrates; {} \
             failures:\n{}",
            failures.len(),
            failures.join("\n")
        );
        assert!(
            hydration.max_depth >= DEEPEST_FRAME,
            "nested-cross-site-frames prints refs in C4, {DEEPEST_FRAME} documents down, so one \
             hydrates there; the deepest was {}",
            hydration.max_depth
        );
        assert!(
            hydration.closed_shadow > 0,
            "shadow-dom and shadow-roots print refs inside closed shadow roots, so one hydrates \
             there"
        );
        Ok::<(), Box<dyn Error>>(())
    }
    .await;
    client.disconnect().await;
    result
}

/// One entry of the site list: its id (1-based position and host, `07-mui.com`) and URL.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Site {
    id: String,
    url: String,
}

/// Reads the site list: one URL per line; blank lines and `#` lines are skipped.
fn read_sites(text: &str) -> Result<Vec<Site>, Box<dyn Error>> {
    let mut sites = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let authority = line
            .split_once("://")
            .map(|(_, rest)| rest)
            .ok_or_else(|| format!("site list entry {line:?} is no absolute URL"))?
            .split(['/', '?', '#'])
            .next()
            .unwrap_or_default();
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        let host = match host.rsplit_once(':') {
            Some((name, port)) if port.bytes().all(|byte| byte.is_ascii_digit()) => name,
            _ => host,
        };
        if host.is_empty() {
            return Err(format!("site list entry {line:?} has no host").into());
        }
        sites.push(Site {
            id: format!("{:02}-{host}", sites.len() + 1),
            url: line.to_owned(),
        });
    }
    Ok(sites)
}

#[test]
fn site_ids_are_position_and_host() -> Result<(), Box<dyn Error>> {
    let sites = read_sites(
        "# a comment\n\nhttps://news.ycombinator.com/\n  https://user@mui.com:8443/x?y#z  \n\
         # another\nhttps://news.ycombinator.com/item?id=1\n",
    )?;
    let ids: Vec<&str> = sites.iter().map(|site| site.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "01-news.ycombinator.com",
            "02-mui.com",
            "03-news.ycombinator.com"
        ],
        "comments and blank lines take no position, and a repeated host is told apart by it"
    );
    let committed = include_str!("../../tools/zero-conflict/sites.txt");
    assert_eq!(
        read_sites(committed)?.len(),
        50,
        "the committed list is the 50-site corpus of design 10.3"
    );
    Ok(())
}

/// A site loaded in a tab of its own, which its owner must close, on failure too.
struct OpenedSite {
    target_id: String,
    session: SessionId,
    /// `document.readyState` when the wait ended: `complete`, or what it was at the bound.
    ready: String,
}

/// `Runtime.evaluate`'s by-value string, or `None` when the page answered something else.
async fn evaluate_string(
    client: &CdpClient,
    session: &SessionId,
    expression: &str,
) -> Result<Option<String>, Box<dyn Error>> {
    let reply = client
        .send(
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true }),
            Some(session),
        )
        .await?;
    Ok(reply
        .pointer("/result/value")
        .and_then(Value::as_str)
        .map(str::to_owned))
}

/// Opens `url` in a fresh tab as `load_fixture` opens a fixture (focus emulation first, so
/// `active` does not depend on which tab the browser last raised), waits for it to be ready
/// within [`SITE_READY_BOUND`], then [`SITE_SETTLE`]. A navigation the browser reports failed is
/// an error naming the browser's reason. Every CDP call is bounded by the client's request
/// timeout ([`fixture_pages::connect_cdp`]).
async fn open_site(client: &CdpClient, url: &str) -> Result<OpenedSite, Box<dyn Error>> {
    let created = client
        .send("Target.createTarget", json!({ "url": "about:blank" }), None)
        .await?;
    let target_id = created
        .get("targetId")
        .and_then(Value::as_str)
        .ok_or("Target.createTarget reply missing targetId")?
        .to_owned();
    let opened = async {
        let attached = client
            .send(
                "Target.attachToTarget",
                json!({ "targetId": target_id, "flatten": true }),
                None,
            )
            .await?;
        let session = SessionId::from(
            attached
                .get("sessionId")
                .and_then(Value::as_str)
                .ok_or("Target.attachToTarget reply missing sessionId")?
                .to_owned(),
        );
        client
            .send("Page.enable", json!({}), Some(&session))
            .await?;
        client
            .send(
                "Emulation.setFocusEmulationEnabled",
                json!({ "enabled": true }),
                Some(&session),
            )
            .await?;
        let navigated = client
            .send("Page.navigate", json!({ "url": url }), Some(&session))
            .await?;
        if let Some(reason) = navigated
            .get("errorText")
            .and_then(Value::as_str)
            .filter(|reason| !reason.is_empty())
        {
            return Err(format!("navigation failed: {reason}").into());
        }
        let deadline = Instant::now() + SITE_READY_BOUND;
        let ready = loop {
            let state = evaluate_string(client, &session, "document.readyState").await?;
            let state = state.unwrap_or_else(|| "unknown".to_owned());
            if state == "complete" || Instant::now() >= deadline {
                break state;
            }
            time::sleep(SITE_POLL).await;
        };
        time::sleep(SITE_SETTLE).await;
        Ok::<_, Box<dyn Error>>(OpenedSite {
            target_id: target_id.clone(),
            session,
            ready,
        })
    }
    .await;
    if opened.is_err() {
        close_tab(client, &target_id).await;
    }
    opened
}

/// Closes a tab, ignoring errors so cleanup never masks the measurement's own result.
async fn close_tab(client: &CdpClient, target_id: &str) {
    let _closed = client
        .send("Target.closeTarget", json!({ "targetId": target_id }), None)
        .await;
}

/// A site the run measured, or why it did not.
enum SiteOutcome {
    /// Loaded, captured twice and checked.
    Measured {
        /// `document.readyState` when measuring began.
        ready: String,
        /// The page's oracle, or why it could not be read: recall is then not measured.
        oracle: Result<Box<Oracle>, String>,
        measured: Box<Measured>,
    },
    /// Not measured, for the browser's or the page's sake: the reason a report outside the
    /// private folder may print, and the full detail for `report.md`.
    NotMeasured { reason: String, detail: String },
    /// Captured, and a capture did not build or check: the engine's or the checker's failure,
    /// which fails the gate. The captures, and the oracle when it was read, are kept to replay it.
    CheckFailed {
        detail: String,
        oracle: Option<Box<Oracle>>,
        captures: [Vec<u8>; 2],
    },
}

/// Loads `site`, reads its oracle and measures it; each tab is closed again, on failure too.
///
/// When the oracle fails, the site is loaded again in a fresh tab and measured there without it.
/// An oracle step that overran its bound stops only its own wait: the renderer can stay inside it
/// for minutes (a `DOMSnapshot` step is the browser's own code, which no script stop ends), and
/// every later call to that renderer queues behind it. The fresh tab costs the site its recall,
/// not its lines.
async fn run_site(
    client: &CdpClient,
    engine: &OracleEngine,
    browser: &str,
    site: &Site,
) -> SiteOutcome {
    let opened = match open_site(client, &site.url).await {
        Ok(opened) => opened,
        Err(error) => {
            let detail = error.to_string();
            // A browser's navigation error is a net error code; anything else stays private.
            let reason = if detail.starts_with("navigation failed: ") {
                detail.clone()
            } else {
                "load failed (detail in report.md)".to_owned()
            };
            return SiteOutcome::NotMeasured { reason, detail };
        }
    };
    // The oracle is bounded as a whole too: a page that keeps its renderer busy slows each of its
    // thousands of per-element steps without tripping any one step's bound. Cut off, it leaves
    // its child-frame sessions attached; closing the tab ends them.
    let oracle = time::timeout(
        SITE_ORACLE_BOUND,
        oracle_capture::read_oracle(
            client,
            &site.id,
            &opened.session,
            &opened.target_id,
            engine,
            browser,
        ),
    )
    .await
    .map_err(|_elapsed| format!("the oracle did not finish within {SITE_ORACLE_BOUND:?}"))
    .and_then(|read| read.map(Box::new).map_err(|error| error.to_string()));
    let opened = if oracle.is_ok() {
        opened
    } else {
        close_tab(client, &opened.target_id).await;
        match open_site(client, &site.url).await {
            Ok(reopened) => reopened,
            Err(error) => {
                return SiteOutcome::NotMeasured {
                    reason: "the fresh tab after the oracle failed did not load (detail in \
                             report.md)"
                        .to_owned(),
                    detail: oracle_note(&oracle, &error.to_string()),
                };
            }
        }
    };
    let outcome = measure_site(client, site, &opened, oracle).await;
    close_tab(client, &opened.target_id).await;
    outcome
}

/// Measures the site loaded in `opened`, with `oracle` for its recall when it was read on this
/// load.
async fn measure_site(
    client: &CdpClient,
    site: &Site,
    opened: &OpenedSite,
    oracle: Result<Box<Oracle>, String>,
) -> SiteOutcome {
    let measured = zero_conflict_live::measure(
        client,
        &site.id,
        &opened.session,
        &opened.target_id,
        oracle.as_deref().ok(),
        async { Ok(()) },
    )
    .await;
    match measured {
        Ok(measured) => SiteOutcome::Measured {
            ready: opened.ready.clone(),
            oracle,
            measured: Box::new(measured),
        },
        Err(MeasureError::Browser(error)) => SiteOutcome::NotMeasured {
            reason: "a capture step failed (detail in report.md)".to_owned(),
            detail: oracle_note(&oracle, &error.to_string()),
        },
        Err(MeasureError::Check { error, captures }) => SiteOutcome::CheckFailed {
            detail: error.to_string(),
            oracle: oracle.ok(),
            captures,
        },
    }
}

/// `error`, with the oracle's own failure when there was one.
fn oracle_note(oracle: &Result<Box<Oracle>, String>, error: &str) -> String {
    match oracle {
        Ok(_) => error.to_owned(),
        Err(oracle) => format!("{error} (after the oracle failed: {oracle})"),
    }
}

/// The private folder `NEO_ZERO_CONFLICT_PRIVATE` names, once it is known to exist outside every
/// work tree of this repository: a site's capture is a real page's text, which must never be
/// committed or reach the remote pool, which uploads `packages/browseros-agent` from whichever
/// work tree it is run in. `run.sh` refuses the same folders.
fn private_folder() -> Result<PathBuf, Box<dyn Error>> {
    let named = env::var_os("NEO_ZERO_CONFLICT_PRIVATE")
        .ok_or("NEO_ZERO_CONFLICT_PRIVATE names no folder; run.sh names and makes one")?;
    let folder = Path::new(&named).canonicalize().map_err(|error| {
        format!(
            "NEO_ZERO_CONFLICT_PRIVATE {}: {error}",
            Path::new(&named).display()
        )
    })?;
    for tree in work_trees()? {
        if folder.starts_with(&tree) {
            return Err(format!(
                "NEO_ZERO_CONFLICT_PRIVATE {} is inside the git work tree {}; real pages' \
                 captures never go there",
                folder.display(),
                tree.display()
            )
            .into());
        }
    }
    Ok(folder)
}

/// Every work tree of the repository holding this crate, as `git worktree list` names them: the
/// main checkout and each linked worktree, wherever it sits. An error when git cannot say, so
/// the privacy check never passes for want of an answer.
fn work_trees() -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .map_err(|error| format!("git worktree list: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git worktree list: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    // A worktree whose folder is gone cannot hold the private folder, which exists.
    Ok(String::from_utf8(output.stdout)?
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .filter_map(|tree| Path::new(tree).canonicalize().ok())
        .collect())
}

/// `site`'s section of `report.md`: everything the stdout rows leave out because it is the
/// page's text.
fn site_section(site: &Site, outcome: &SiteOutcome) -> String {
    let mut out = format!("## {}\n\n{}\n\n", site.id, site.url);
    match outcome {
        SiteOutcome::NotMeasured { detail, .. } => {
            let _ = writeln!(out, "Not measured: {detail}\n");
        }
        SiteOutcome::CheckFailed { detail, .. } => {
            let _ = writeln!(
                out,
                "Check failed: {detail}\n\nReplay `{0}.pb` and `{0}.2.pb` with `check_apc`.\n",
                site.id
            );
        }
        SiteOutcome::Measured {
            ready,
            oracle,
            measured,
        } => {
            let _ = writeln!(
                out,
                "readyState {ready}; contract_version {}\n",
                measured.contract_version
            );
            for (capture, check) in [(1, &measured.first.check), (2, &measured.second.check)] {
                for format in Format::ALL {
                    let counts = LineCounts::of_format(check, format);
                    let _ = writeln!(
                        out,
                        "- capture #{capture} {format}: {} lines, {} reached, {} labels, {} \
                         misses",
                        counts.lines, counts.reached, counts.labels, counts.misses
                    );
                }
            }
            for (capture, check) in [(1, &measured.first.check), (2, &measured.second.check)] {
                let _ = writeln!(out, "\n### Misses, capture #{capture}\n");
                for line in check.misses() {
                    let target = line.target.as_ref().map_or_else(
                        || "none".to_owned(),
                        |key| format!("{}/{}", key.doc, key.node),
                    );
                    let _ = writeln!(
                        out,
                        "- {} line {} ({}): `{}` | `{}` | {} | target {target}",
                        line.format,
                        line.line.number,
                        line.line.role,
                        line.line.text.trim_start(),
                        line.query,
                        line.verdict
                    );
                }
            }
            out.push_str("\n### Instability\n\n");
            for instability in &measured.unstable {
                let _ = writeln!(out, "- {instability:?}");
            }
            out.push_str("\n### Hydration failures\n\n");
            for hydrated in &measured.hydrated {
                if let Err(failure) = &hydrated.outcome {
                    let _ = writeln!(out, "- {}: {failure}", hydrated.r#ref);
                }
            }
            out.push_str("\n### Recall\n\n");
            match (&measured.recall, oracle) {
                (_, Err(error)) => {
                    let _ = writeln!(out, "Not measured: the oracle failed: {error}");
                }
                (None, Ok(_)) => out.push_str("Not measured.\n"),
                (Some(recall), Ok(_)) => out.push_str(&recall_table(recall)),
            }
            out.push('\n');
        }
    }
    out
}

/// `recall` as `report.md` tables it: per frame and role, the page's elements, what Rust found
/// of them, and the backendNodeIds it missed.
fn recall_table(recall: &Recall) -> String {
    let mut out = format!(
        "{} frames without a document in the tree.\n\n\
         | frame | role | page | found | extra | missing backendNodeIds | unread |\n\
         | ---: | --- | ---: | ---: | ---: | --- | --- |\n",
        recall.frames_without_document
    );
    for role in &recall.roles {
        let listed: Vec<String> = role
            .missing
            .iter()
            .take(LISTED_MISSING)
            .map(u64::to_string)
            .collect();
        let more = role.missing.len().saturating_sub(LISTED_MISSING);
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {}{} | {} |",
            role.frame,
            role.role,
            role.page,
            role.found,
            role.extra,
            listed.join(" "),
            if more > 0 {
                format!(" (+{more})")
            } else {
                String::new()
            },
            role.unread.as_deref().unwrap_or("")
        );
    }
    out
}

/// One measured site's stdout row and the misses it lists, of both captures: counts, line
/// numbers, roles and verdict kinds, never a name or other page text. The row's counts are
/// capture #1's, whose bytes replay offline.
fn site_row(site: &Site, ready: &str, measured: &Measured) -> Vec<String> {
    let counts = LineCounts::of(&measured.first.check.lines);
    let hydration = HydrationCounts::of(measured);
    let recall = match RecallCounts::of(measured) {
        Some(recall) => format!(
            "{} ({}/{}) | {}",
            recall.recall(),
            recall.found,
            recall.page,
            recall.extra
        ),
        None => "n/a (oracle failed) | n/a".to_owned(),
    };
    let mut rows = vec![format!(
        "{} | {} | {} | {} | {} | {} | {}/{} ({} failed) | {} | {recall}{}",
        site.id,
        counts.role_lines(),
        counts.reached,
        counts.labels,
        counts.misses,
        Percent {
            part: counts.reached,
            whole: counts.role_lines()
        },
        hydration.hydrated,
        hydration.refs,
        hydration.failed(),
        if measured.unstable.is_empty() {
            "yes"
        } else {
            "no"
        },
        if ready == "complete" {
            String::new()
        } else {
            format!(" | readyState {ready} at the bound")
        },
    )];
    for (capture, check) in [(1, &measured.first.check), (2, &measured.second.check)] {
        for line in check.misses() {
            rows.push(format!(
                "  miss: {} capture #{capture} {} line {}: role {} | {}",
                site.id,
                line.format,
                line.line.number,
                line.line.role,
                zero_conflict_live::verdict_kind(&line.verdict)
            ));
        }
    }
    for instability in &measured.unstable {
        rows.push(match instability {
            Instability::Text(format) => format!("  unstable: {} {format} text differs", site.id),
            Instability::Lines {
                format,
                first,
                second,
            } => format!(
                "  unstable: {} {format} lines with a ref {first} then {second}",
                site.id
            ),
            Instability::Verdict {
                format,
                line,
                first,
                second,
            } => format!(
                "  unstable: {} {format} line {line}: {} then {}",
                site.id,
                zero_conflict_live::verdict_brief(first),
                zero_conflict_live::verdict_brief(second)
            ),
        });
    }
    let mut failure_kinds: Vec<&str> = measured
        .hydrated
        .iter()
        .filter_map(|hydrated| hydrated.outcome.as_ref().err())
        .map(zero_conflict_live::HydrationFailure::kind)
        .collect();
    failure_kinds.sort_unstable();
    failure_kinds.dedup();
    if !failure_kinds.is_empty() {
        rows.push(format!(
            "  hydration failures: {}: {}",
            site.id,
            failure_kinds.join(", ")
        ));
    }
    rows
}

/// Writes a site's captures and oracle to the private folder, so a miss or a failed check
/// replays offline: capture #1 as `check_apc` reads it, and capture #2 for a line only it missed.
fn save_site(private: &Path, site: &Site, outcome: &SiteOutcome) -> Result<(), Box<dyn Error>> {
    let (captures, oracle) = match outcome {
        SiteOutcome::NotMeasured { .. } => return Ok(()),
        SiteOutcome::Measured {
            oracle, measured, ..
        } => (
            [&measured.first.pb, &measured.second.pb],
            oracle.as_ref().ok(),
        ),
        SiteOutcome::CheckFailed {
            oracle, captures, ..
        } => ([&captures[0], &captures[1]], oracle.as_ref()),
    };
    let [first, second] = captures;
    fs::write(private.join(format!("{}.pb", site.id)), first)?;
    fs::write(private.join(format!("{}.2.pb", site.id)), second)?;
    if let Some(oracle) = oracle {
        let mut json = serde_json::to_vec_pretty(oracle)?;
        json.push(b'\n');
        fs::write(private.join(format!("{}.oracle.json", site.id)), json)?;
    }
    Ok(())
}

/// The site run's total rows: the gate's percentage, each site not measured, and each site whose
/// check failed.
fn report_site_totals(
    sites: usize,
    totals: &Totals,
    not_measured: &[String],
    check_failed: &[String],
) {
    let Totals {
        lines,
        hydration,
        recall,
        unstable_pages,
        ..
    } = totals;
    report(&format!(
        "total | {} of {sites} sites measured | {} role lines | {} reached | {} labels | {} \
         misses | {} | {unstable_pages} unstable sites | {} hydration failures | recall {} \
         ({}/{}) | {} extra",
        sites - not_measured.len() - check_failed.len(),
        lines.role_lines(),
        lines.reached,
        lines.labels,
        lines.misses,
        Percent {
            part: lines.reached,
            whole: lines.role_lines()
        },
        hydration.failed(),
        recall.recall(),
        recall.found,
        recall.page,
        recall.extra,
    ));
    for reason in not_measured {
        report(&format!("not measured: {reason}"));
    }
    for id in check_failed {
        report(&format!("check failed: {id} (detail in report.md)"));
    }
}

#[tokio::test]
#[ignore = "needs the network and a BrowserOS build carrying the APC contract; run it through tools/zero-conflict/run.sh"]
async fn site_lines_reach_their_elements() -> Result<(), Box<dyn Error>> {
    let Some(list) = env::var_os("NEO_ZERO_CONFLICT_SITES") else {
        report("sites: SKIPPED (NEO_ZERO_CONFLICT_SITES names no list)");
        return Ok(());
    };
    let sites = read_sites(&fs::read_to_string(&list)?)?;
    let private = private_folder()?;
    let engine = OracleEngine::new()?;
    let client = fixture_pages::connect_cdp().await?;
    let result = async {
        let browser = oracle_capture::browser_product(&client).await?;
        report(&format!(
            "zero-conflict sites: {} sites; browser {browser}; private folder {}",
            sites.len(),
            private.display()
        ));
        report(
            "site | role lines | reached | labels | misses | % | refs hydrated | stable | recall \
             | extra",
        );
        let mut document = format!(
            "# Zero-conflict site run\n\nbrowser {browser}; {} sites from {}\n\n",
            sites.len(),
            Path::new(&list).display()
        );
        let mut totals = Totals::default();
        let mut not_measured = Vec::new();
        let mut check_failed = Vec::new();
        for site in &sites {
            let started = Instant::now();
            let outcome = run_site(&client, &engine, &browser, site).await;
            save_site(&private, site, &outcome)?;
            document.push_str(&site_section(site, &outcome));
            fs::write(private.join("report.md"), &document)?;
            match &outcome {
                SiteOutcome::NotMeasured { reason, .. } => {
                    report(&format!("{} | not measured: {reason}", site.id));
                    not_measured.push(format!("{}: {reason}", site.id));
                }
                SiteOutcome::CheckFailed { .. } => {
                    report(&format!("{} | check failed (detail in report.md)", site.id));
                    check_failed.push(site.id.clone());
                }
                SiteOutcome::Measured {
                    ready, measured, ..
                } => {
                    for row in site_row(site, ready, measured) {
                        report(&row);
                    }
                    totals.add(measured);
                }
            }
            report(&format!(
                "  time: {} {:.1} s",
                site.id,
                started.elapsed().as_secs_f64()
            ));
        }
        report_site_totals(sites.len(), &totals, &not_measured, &check_failed);
        assert!(
            check_failed.is_empty(),
            "a capture that does not build or check is the engine's or the checker's failure, \
             whatever the page did, and its saved captures replay it: {check_failed:?}"
        );
        let lines = &totals.lines;
        assert!(
            lines.role_lines() > 0,
            "a site run that measured no role line proves nothing"
        );
        assert!(
            lines.reached * 1000 >= lines.role_lines() * SITE_GATE_PER_MILLE,
            "design 10.3 gates the sites at 99.5% of role lines reached; {} of {} is {}",
            lines.reached,
            lines.role_lines(),
            Percent {
                part: lines.reached,
                whole: lines.role_lines()
            }
        );
        Ok::<(), Box<dyn Error>>(())
    }
    .await;
    client.disconnect().await;
    result
}

/// Replays a site run's saved captures offline: each `<id>.pb` in the folder
/// `NEO_ZERO_CONFLICT_REPLAY` names, checked by `check_apc` as capture #1 was checked live, so a
/// miss can be worked on without the browser or the network. Without the variable it passes and
/// says it skipped. Its rows hold counts, line numbers, roles and verdict kinds; the lines'
/// text is in the run's `report.md`.
#[test]
fn private_site_captures_replay_offline() -> Result<(), Box<dyn Error>> {
    let Some(folder) = env::var_os("NEO_ZERO_CONFLICT_REPLAY") else {
        report("replay: SKIPPED (NEO_ZERO_CONFLICT_REPLAY names no folder)");
        return Ok(());
    };
    let mut captures = Vec::new();
    for entry in fs::read_dir(&folder)? {
        let path = entry?.path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if let Some(id) = name.strip_suffix(".pb").filter(|id| !id.ends_with(".2")) {
            captures.push((id.to_owned(), path));
        }
    }
    captures.sort();
    assert!(
        !captures.is_empty(),
        "{} holds no <id>.pb, so the replay would judge nothing",
        Path::new(&folder).display()
    );
    let mut totals = LineCounts::default();
    for (id, path) in &captures {
        let check =
            zero_conflict::check_apc(id, neo_snapshot_apc::decode_protobuf(&fs::read(path)?)?)?;
        let counts = LineCounts::of(&check.lines);
        report(&format!(
            "{id} | {} role lines | {} reached | {} labels | {} misses | {}",
            counts.role_lines(),
            counts.reached,
            counts.labels,
            counts.misses,
            Percent {
                part: counts.reached,
                whole: counts.role_lines()
            }
        ));
        for line in check.misses() {
            report(&format!(
                "  miss: {id} {} line {}: role {} | {}",
                line.format,
                line.line.number,
                line.line.role,
                zero_conflict_live::verdict_kind(&line.verdict)
            ));
        }
        totals.merge(counts);
    }
    report(&format!(
        "replay total | {} captures | {} role lines | {} reached | {} misses | {}",
        captures.len(),
        totals.role_lines(),
        totals.reached,
        totals.misses,
        Percent {
            part: totals.reached,
            whole: totals.role_lines()
        }
    ));
    Ok(())
}
