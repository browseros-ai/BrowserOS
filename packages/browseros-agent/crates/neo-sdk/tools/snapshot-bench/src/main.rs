//! The snapshot bench of design 4.2: neo-snapshot's `render` beside today's TypeScript renderer
//! in QuickJS, in one process, on generated trees shaped like the pages design 4.1's targets
//! were measured on. It prints a Markdown report for the switch's pull request.
//!
//! ```text
//!  shapes: a seeded tree ─▶ JSON ─┬─▶ serde decode ─▶ Snapshot ─▶ neo_snapshot::render ─▶ text
//!                                 └─▶ QuickJS: JSON.parse ─▶ todaysAnswer(call, result) ─▶ text
//!                                     both texts must be the same bytes, or the row says so
//! ```
//!
//! Nothing here asserts a time (design 4.2: no gate asserts one). Each row is timed `--runs`
//! times; the report gives the first run and the median. Memory is read by the binary's counting
//! allocator (`counting`), which QuickJS allocates through too, and by QuickJS's own count.
//!
//! Run it with `cargo run --release` from this folder, or on the pool with
//! `bun run remote run -- sh -c 'cd crates/neo-sdk/tools/snapshot-bench && cargo run --release'`.

mod counting;
mod old_engine;
mod shapes;

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use neo_snapshot::{AriaNode, Baseline, Call, Earlier, MaxTokens, Passed, Snapshot, render};

use old_engine::{OldEngine, Sample};

#[global_allocator]
static ALLOCATOR: counting::Counting = counting::Counting;

const MIB: f64 = 1024.0 * 1024.0;

/// A tree as the server holds it after its decode, and what it took to get there.
struct Shape {
    name: &'static str,
    snapshot: Snapshot,
    nodes: usize,
    json_bytes: usize,
    /// Live bytes the decoded tree holds.
    tree_bytes: usize,
    /// serde's decode of the JSON into the typed tree: the server's step before any render.
    decode: Timings,
}

/// One row's durations, in the order they ran.
struct Timings(Vec<Duration>);

impl Timings {
    fn first(&self) -> f64 {
        self.0.first().map_or(f64::NAN, ms)
    }

    fn median(&self) -> f64 {
        let mut sorted = self.0.clone();
        sorted.sort();
        sorted.get(sorted.len() / 2).map_or(f64::NAN, ms)
    }

    fn cell(&self) -> String {
        format!("{:.2} / **{:.2}**", self.first(), self.median())
    }
}

fn ms(duration: &Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn mib(bytes: usize) -> f64 {
    bytes as f64 / MIB
}

/// Rust's side of one row.
struct RustRow {
    render: Timings,
    /// The most bytes live at once above the tree during the first render, its text included.
    above_tree: usize,
    text: String,
}

/// The old engine's side of one row, or how it failed.
enum OldRow {
    Ran {
        parse: Timings,
        render: Timings,
        own_count: usize,
        peak: usize,
        same: bool,
    },
    Failed(String),
}

struct Row {
    shape: &'static str,
    call: &'static str,
    rust: RustRow,
    old: OldRow,
}

fn prepare(name: &'static str, tree: Vec<AriaNode>, runs: usize) -> Result<Shape, String> {
    let nodes = shapes::nodes(&tree);
    let json = serde_json::to_string(&tree).map_err(|error| error.to_string())?;
    drop(tree);
    let mut decode = Vec::with_capacity(runs);
    let mut decoded = None;
    let mut tree_bytes = 0;
    for _ in 0..runs {
        drop(decoded.take());
        let before = counting::live();
        let started = Instant::now();
        let tree: Vec<AriaNode> = serde_json::from_str(&json).map_err(|error| error.to_string())?;
        decode.push(started.elapsed());
        tree_bytes = counting::live().saturating_sub(before);
        decoded = Some(tree);
    }
    Ok(Shape {
        name,
        snapshot: Snapshot {
            tree: decoded.unwrap_or_default(),
            ..Snapshot::default()
        },
        nodes,
        json_bytes: json.len(),
        tree_bytes,
        decode: Timings(decode),
    })
}

/// Times `runs` renders. The first is also the one whose memory is counted, so Rust's first run
/// is a first render, as the old engine's is.
fn rust_row(snapshot: &Snapshot, call: Call<'_>, runs: usize) -> RustRow {
    let mut times = Vec::with_capacity(runs);
    let before = counting::reset_peak();
    let started = Instant::now();
    let answer = render(snapshot, call);
    times.push(started.elapsed());
    let above_tree = counting::peak().saturating_sub(before);
    let text = answer.into_text();
    for _ in 1..runs {
        let started = Instant::now();
        let answer = render(snapshot, call);
        times.push(started.elapsed());
        drop(answer);
    }
    RustRow {
        render: Timings(times),
        above_tree,
        text,
    }
}

fn old_row(snapshot: &Snapshot, call: Call<'_>, runs: usize, expected: &str) -> OldRow {
    let engine = match OldEngine::new() {
        Ok(engine) => engine,
        Err(error) => return OldRow::Failed(error),
    };
    let mut samples: Vec<Sample> = Vec::with_capacity(runs);
    for _ in 0..runs {
        match engine.answer(&call, snapshot) {
            Ok(sample) => samples.push(sample),
            Err(error) => return OldRow::Failed(error),
        }
    }
    OldRow::Ran {
        parse: Timings(samples.iter().map(|sample| sample.parse).collect()),
        render: Timings(samples.iter().map(|sample| sample.render).collect()),
        own_count: samples.first().map_or(0, |sample| sample.own_count),
        peak: samples.iter().map(|sample| sample.peak).max().unwrap_or(0),
        same: samples.iter().all(|sample| sample.text == expected),
    }
}

fn row(
    shape: &'static str,
    label: &'static str,
    snapshot: &Snapshot,
    call: Call<'_>,
    runs: usize,
) -> Row {
    eprintln!("snapshot-bench: {shape}, {label}");
    let rust = rust_row(snapshot, call, runs);
    let old = old_row(snapshot, call, runs, &rust.text);
    Row {
        shape,
        call: label,
        rust,
        old,
    }
}

/// The names `--only` takes.
const SHAPES: &[&str] = &["thread", "article", "grid", "delta"];

struct Options {
    runs: usize,
    only: Option<Vec<String>>,
    out: Option<String>,
}

fn options() -> Result<Options, String> {
    let mut options = Options {
        runs: 9,
        only: None,
        out: None,
    };
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--runs" => {
                options.runs = args
                    .next()
                    .and_then(|runs| runs.parse().ok())
                    .filter(|&runs| runs > 0)
                    .ok_or("--runs takes a count above 0")?;
            }
            "--only" => {
                let names = args
                    .next()
                    .ok_or("--only takes thread,article,grid,delta")?;
                let names: Vec<String> = names.split(',').map(str::to_owned).collect();
                if let Some(unknown) = names.iter().find(|name| !SHAPES.contains(&name.as_str())) {
                    return Err(format!(
                        "--only: no shape {unknown}; the shapes are thread,article,grid,delta"
                    ));
                }
                options.only = Some(names);
            }
            "--out" => options.out = Some(args.next().ok_or("--out takes a file")?),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(options)
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            eprintln!("snapshot-bench: {error}");
            ExitCode::from(2)
        }
    }
}

/// Runs every row and prints the report. `Ok(false)` when any row's old engine failed, or printed
/// other bytes than Rust: a row that compared nothing, or compared two different outputs,
/// measures nothing. The report still says what each row did.
fn run() -> Result<bool, String> {
    let options = options()?;
    let machine = machine();
    let wanted = |name: &str| {
        options
            .only
            .as_ref()
            .is_none_or(|only| only.iter().any(|wanted| wanted == name))
    };
    let runs = options.runs;
    let mut shapes_run = Vec::new();
    let mut rows = Vec::new();

    let default = Passed::default();
    for (name, build) in [
        ("thread", shapes::thread as fn() -> Vec<AriaNode>),
        ("article", shapes::article),
        ("grid", shapes::grid),
    ] {
        if !wanted(name) {
            continue;
        }
        let shape = prepare(name, build(), runs)?;
        rows.push(row(
            name,
            "default call",
            &shape.snapshot,
            Call::Page(&default),
            runs,
        ));
        rows.push(row(
            name,
            "mode: 'full'",
            &shape.snapshot,
            Call::Full { max_tokens: None },
            runs,
        ));
        rows.push(row(
            name,
            "Playwright's format",
            &shape.snapshot,
            Call::Playwright,
            runs,
        ));
        shapes_run.push(shape);
    }

    let mut deltas = Vec::new();
    if wanted("delta") {
        let passed = Passed {
            max_tokens: MaxTokens::new(1e9),
            ..Passed::default()
        };
        for (name, repeated) in [
            ("delta, distinct lines", false),
            ("delta, repeated lines", true),
        ] {
            let (page, changed) = shapes::delta_pair(repeated);
            let earlier_page = prepare(name, page, 1)?;
            let shape = prepare(name, changed, runs)?;
            let stored = render(&earlier_page.snapshot, Call::Page(&passed));
            let pending = stored
                .pending()
                .ok_or("a whole page in Neo's format keeps its text")?;
            let baseline = Baseline {
                text: pending.text.to_owned(),
                form: pending.form,
            };
            drop(stored);
            drop(earlier_page);
            let call = Call::Delta {
                passed: &passed,
                earlier: Earlier::Text(&baseline),
            };
            let delta = row(name, "1,000 changed lines", &shape.snapshot, call, runs);
            let head = delta
                .rust
                .text
                .lines()
                .next()
                .unwrap_or_default()
                .to_owned();
            deltas.push((name, baseline.text.lines().count(), head));
            rows.push(delta);
            shapes_run.push(shape);
        }
    }

    let report = report(&machine, &shapes_run, &rows, &deltas, runs);
    print!("{report}");
    if let Some(out) = options.out {
        fs::write(&out, &report).map_err(|error| format!("{out}: {error}"))?;
    }
    Ok(rows
        .iter()
        .all(|row| matches!(row.old, OldRow::Ran { same: true, .. })))
}

fn report(
    machine: &str,
    shapes: &[Shape],
    rows: &[Row],
    deltas: &[(&str, usize, String)],
    runs: usize,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "## Snapshot bench\n");
    let _ = writeln!(out, "{machine}\n");
    let _ = writeln!(
        out,
        "Milliseconds, first run / **median of {runs}**. Old engine: today's renderer bundle in \
         QuickJS at a run's limits (128 MiB heap, 512 KiB stack). New: `neo_snapshot::render` on \
         the decoded tree. Same bytes: every old run's text against Rust's.\n"
    );

    let _ = writeln!(out, "### The trees\n");
    let _ = writeln!(
        out,
        "| Shape | Nodes | JSON | Decoded tree in Rust | serde decode (ms) |"
    );
    let _ = writeln!(out, "|---|---:|---:|---:|---:|");
    for shape in shapes {
        let _ = writeln!(
            out,
            "| {} | {} | {:.2} MB | {:.1} MiB | {} |",
            shape.name,
            shape.nodes,
            shape.json_bytes as f64 / 1e6,
            mib(shape.tree_bytes),
            shape.decode.cell()
        );
    }

    let _ = writeln!(out, "\n### Time\n");
    let _ = writeln!(
        out,
        "| Shape | Call | Rust render | Old render | Old ÷ Rust | Old `JSON.parse` | Same bytes | Lines, chars |"
    );
    let _ = writeln!(out, "|---|---|---:|---:|---:|---:|---|---:|");
    for row in rows {
        let (old, ratio, parse, same) = match &row.old {
            OldRow::Ran {
                parse,
                render,
                same,
                ..
            } => (
                render.cell(),
                format!("{:.0}×", render.median() / row.rust.render.median()),
                parse.cell(),
                if *same { "yes" } else { "**NO**" }.to_owned(),
            ),
            OldRow::Failed(error) => (
                format!("failed: {}", one_line(error)),
                "-".to_owned(),
                "-".to_owned(),
                "-".to_owned(),
            ),
        };
        let _ = writeln!(
            out,
            "| {} | {} | {} | {old} | {ratio} | {parse} | {same} | {}, {} |",
            row.shape,
            row.call,
            row.rust.render.cell(),
            row.rust.text.lines().count(),
            row.rust.text.encode_utf16().count(),
        );
    }

    let _ = writeln!(out, "\n### Memory\n");
    let _ = writeln!(
        out,
        "Rust: the most bytes live above the decoded tree during one render, its text included. \
         Old: the most the QuickJS runtime held above its bundle during `JSON.parse` and the \
         render (the result's JSON, the tree as objects, every line), by the same counting \
         allocator; and QuickJS's own count after the render, the count its 128 MiB limit is \
         checked against.\n"
    );
    let _ = writeln!(
        out,
        "| Shape | Call | Tree | Rust above the tree | Old peak | Old own count after |"
    );
    let _ = writeln!(out, "|---|---|---:|---:|---:|---:|");
    for row in rows {
        let tree = shapes
            .iter()
            .find(|shape| shape.name == row.shape)
            .map_or(0, |shape| shape.tree_bytes);
        let (peak, own) = match &row.old {
            OldRow::Ran {
                own_count, peak, ..
            } => (
                format!("{:.1} MiB", mib(*peak)),
                format!("{:.1} MiB", mib(*own_count)),
            ),
            OldRow::Failed(_) => ("failed".to_owned(), "failed".to_owned()),
        };
        let _ = writeln!(
            out,
            "| {} | {} | {:.1} MiB | +{:.1} MiB ({:.2}× the tree) | {peak} | {own} |",
            row.shape,
            row.call,
            mib(tree),
            mib(row.rust.above_tree),
            row.rust.above_tree as f64 / tree.max(1) as f64,
        );
    }

    if !deltas.is_empty() {
        let _ = writeln!(out, "\n### The delta's pages\n");
        for (name, lines, head) in deltas {
            let _ = writeln!(
                out,
                "- {name}: the stored text has {lines} lines; the answer starts `{head}`."
            );
        }
    }

    let _ = writeln!(out, "\n### Against design 4.1's targets\n");
    let _ = writeln!(out, "| Target | Measured | Met |");
    let _ = writeln!(out, "|---|---|---|");
    let find = |shape: &str, call: &str| {
        rows.iter()
            .find(|row| row.shape == shape && row.call == call)
    };
    if let Some(row) = find("thread", "default call") {
        let rust = row.rust.render.median();
        let ratio = match &row.old {
            OldRow::Ran { render, .. } => render.median() / rust,
            OldRow::Failed(_) => f64::NAN,
        };
        let _ = writeln!(
            out,
            "| default call's render, largest shape: at most 25 ms | {rust:.2} ms | {} |",
            met(rust <= 25.0)
        );
        let _ = writeln!(
            out,
            "| the same: at least 20 times the old engine in this job | {ratio:.0}× | {} |",
            met(ratio >= 20.0)
        );
    }
    if let Some(row) = find("thread", "mode: 'full'") {
        let rust = row.rust.render.median();
        let _ = writeln!(
            out,
            "| `mode: 'full'`, largest shape: at most 60 ms | {rust:.2} ms | {} |",
            met(rust <= 60.0)
        );
    }
    if let Some(row) = find("article", "default call") {
        let rust = row.rust.render.median();
        let _ = writeln!(
            out,
            "| default call's render, article: at most 8 ms | {rust:.2} ms | {} |",
            met(rust <= 8.0)
        );
    }
    if let Some(tree) = shapes.iter().find(|shape| shape.name == "thread") {
        for (call, bound, words) in [
            ("default call", 1.0, "the tree again for the default"),
            (
                "mode: 'full'",
                1.5,
                "one and a half trees for `mode: 'full'`",
            ),
        ] {
            if let Some(row) = find("thread", call) {
                let times = row.rust.above_tree as f64 / tree.tree_bytes.max(1) as f64;
                let _ = writeln!(
                    out,
                    "| Rust's memory above the tree, largest shape: at most {words} | +{:.1} MiB over {:.1} MiB ({times:.2}×) | {} |",
                    mib(row.rust.above_tree),
                    mib(tree.tree_bytes),
                    met(times <= bound)
                );
            }
        }
    }
    out
}

fn met(yes: bool) -> &'static str {
    if yes { "yes" } else { "**no**" }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Where the numbers were taken: the CPU, its threads and the load, as Linux reports them.
fn machine() -> String {
    let threads = std::thread::available_parallelism().map_or(0, std::num::NonZero::get);
    let model = fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|info| {
            info.lines()
                .find(|line| line.starts_with("model name"))
                .and_then(|line| line.split(':').nth(1))
                .map(|model| model.trim().to_owned())
        })
        .unwrap_or_else(|| "CPU model unknown".to_owned());
    let load = fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|load| load.split_whitespace().next().map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned());
    format!(
        "Machine: {model}, {threads} threads, one-minute load {load} at the start; {} {}.",
        env::consts::OS,
        env::consts::ARCH
    )
}
