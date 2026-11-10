//! The translator: reads `spec/` (Playwright's protocol YAML dialect, plus Neo's `serve.yml`
//! and `injected.yml`) and writes this crate's generated Rust and TypeScript.
//!
//! ```text
//! spec/playwright/*.yml ─┐                                          ┌─ rust.rs ───────▶ src/*.rs
//! spec/injected.yml ─────┼─ yaml.rs ─▶ dialect.rs ─▶ contract.rs ──┤
//! spec/serve.yml ────────┘   (YAML)    (Playwright)  (Neo's rules)  └─ typescript.rs ─▶ ts/*.ts
//! ```
//!
//! Each stage owns one body of knowledge: YAML's quirks, Playwright's dialect, Neo's rules, or
//! one target language. Both printers render the same contract and decide nothing, so the Rust
//! and the TypeScript cannot disagree. Run from `packages/browseros-agent`:
//!
//! ```text
//! NEO_RUNTIME_SPEC_CODEGEN=1 cargo run -p neo-playwright-protocol --example codegen [-- --check]
//! ```
//!
//! Without `--check` it rewrites the generated files that changed. With it, it writes nothing
//! and exits 1 when a committed file is stale; `cargo test` runs the same check
//! (`committed_output_matches_spec`), so CI needs no extra job. `NEO_RUNTIME_SPEC_CODEGEN=1`
//! builds the translator without the library code it regenerates: Cargo links a package's
//! library into its examples, and the library refuses to compile while its generated code is
//! stale. `--pin-playwright <commit>` records the vendored copy's hash in
//! `spec/playwright/UPSTREAM`; only `sync-playwright.sh` runs it.

mod contract;
mod dialect;
mod error;
mod output;
mod rust;
mod spec_files;
#[cfg(test)]
mod tests;
mod typescript;
mod yaml;

use std::env;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use contract::Contract;
use dialect::{DialectFile, Grammar, Protocol};
use error::{At, SpecError};
use output::{Drift, Mode, REGENERATE, Rendered};
use spec_files::SpecFile;

/// The pin of the vendored Playwright files, relative to `spec/`.
const UPSTREAM: &str = "playwright/UPSTREAM";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let outcome = crate_dir().and_then(|crate_dir| match args.as_slice() {
        [] => translate(&crate_dir, Mode::Write),
        ["--check"] => translate(&crate_dir, Mode::Check),
        ["--pin-playwright", commit] => pin_playwright(&crate_dir, commit),
        _ => Err(Error::Usage),
    });
    report(outcome)
}

/// This crate's directory, which `cargo run` and `cargo test` set when they start the binary.
///
/// Read at run time, never with `env!`. Every worktree shares one Cargo build dir (`build-dir`
/// in the repository's `.cargo/config.toml`), so Cargo can reuse a binary compiled in another
/// worktree. A path fixed at compile time would then read that worktree's spec and, in write
/// mode, overwrite its generated files.
fn crate_dir() -> Result<PathBuf, Error> {
    env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .ok_or(Error::NoCrateDir)
}

/// Renders the outputs from the spec on disk, then writes or checks them.
fn translate(crate_dir: &Path, mode: Mode) -> Result<Outcome, Error> {
    let spec_dir = crate_dir.join("spec");
    // One read: the hash stamped into the output covers exactly the bytes parsed.
    let files = spec_files::read(&spec_dir).map_err(Error::Io)?;
    let upstream = fs::read_to_string(spec_dir.join(UPSTREAM)).map_err(Error::Io)?;
    let rendered = generate(&files, &upstream).map_err(Error::Spec)?;
    let drift = output::apply(crate_dir, &rendered, mode).map_err(Error::Io)?;
    Ok(Outcome::Translated { mode, drift })
}

/// Spec bytes in, generated files out: everything but I/O, so tests can feed it small specs.
fn generate(files: &[SpecFile], upstream: &str) -> Result<Vec<Rendered>, SpecError> {
    check_upstream(files, upstream)?;
    let mut dialect = Vec::new();
    let mut injected = None;
    let mut serve = None;
    for file in files {
        let root = yaml::parse(file)?;
        let path = file.path.clone();
        match file.path.as_str() {
            "serve.yml" => serve = Some(root),
            "injected.yml" => {
                injected = Some(DialectFile {
                    path,
                    root,
                    grammar: Grammar::Extended,
                });
            }
            other if is_playwright(other) => {
                dialect.push(DialectFile {
                    path,
                    root,
                    grammar: Grammar::Playwright,
                });
            }
            other => {
                return Err(At::file(other).error(
                    "is not a spec file: expected playwright/*.yml, serve.yml or injected.yml"
                        .to_owned(),
                ));
            }
        }
    }
    let serve = serve.ok_or_else(|| At::file("serve.yml").error("is missing".to_owned()))?;
    // Playwright's files load first, in name order, as Playwright's own loader reads them.
    dialect.push(injected.ok_or_else(|| At::file("injected.yml").error("is missing".to_owned()))?);
    let protocol = Protocol::load(&dialect)?;
    let contract = Contract::build(&protocol, &serve)?;
    let sha256 = spec_files::sha256(files);
    Ok(vec![
        Rendered {
            path: "src/serve.rs",
            contents: rust::serve(&contract.serve, &sha256),
        },
        Rendered {
            path: "src/injected.rs",
            contents: rust::injected(&contract.injected, &sha256),
        },
        Rendered {
            path: "src/spec_hash.rs",
            contents: rust::spec_hash(&sha256),
        },
        Rendered {
            path: "ts/serve.ts",
            contents: typescript::serve(&contract.serve, &sha256),
        },
        Rendered {
            path: "ts/injected.ts",
            contents: typescript::injected(&contract.injected, &sha256),
        },
        Rendered {
            path: "ts/spec-hash.ts",
            contents: typescript::spec_hash(&sha256),
        },
    ])
}

fn is_playwright(path: &str) -> bool {
    path.strip_prefix("playwright/")
        .is_some_and(|name| !name.contains('/'))
}

/// Refuses an edited copy of Playwright's files: they must hash to what `sync-playwright.sh`
/// pinned, so the dialect Neo reads is the dialect of the recorded commit.
fn check_upstream(files: &[SpecFile], upstream: &str) -> Result<(), SpecError> {
    let at = At::file(UPSTREAM);
    let pinned = upstream
        .lines()
        .find_map(|line| line.strip_prefix("sha256: "))
        .ok_or_else(|| at.error("has no `sha256:` line; rerun sync-playwright.sh".to_owned()))?;
    let actual = spec_files::sha256(files.iter().filter(|file| is_playwright(&file.path)));
    if actual == pinned {
        return Ok(());
    }
    Err(at.error(format!(
        "pins sha256 {pinned}, but spec/playwright/*.yml hash to {actual}: the vendored copy was \
         edited. Never edit Playwright's files; restore them, or upgrade with \
         `sync-playwright.sh <playwright-checkout>`"
    )))
}

/// Records the commit and hash of the vendored Playwright files in `spec/playwright/UPSTREAM`.
fn pin_playwright(crate_dir: &Path, commit: &str) -> Result<Outcome, Error> {
    let is_commit = commit.len() == 40 && commit.bytes().all(|byte| byte.is_ascii_hexdigit());
    if !is_commit {
        return Err(Error::Usage);
    }
    let spec_dir = crate_dir.join("spec");
    let files = spec_files::read(&spec_dir).map_err(Error::Io)?;
    let sha256 = spec_files::sha256(files.iter().filter(|file| is_playwright(&file.path)));
    fs::write(spec_dir.join(UPSTREAM), upstream_text(commit, &sha256)).map_err(Error::Io)?;
    Ok(Outcome::Pinned { sha256 })
}

fn upstream_text(commit: &str, sha256: &str) -> String {
    format!(
        "# Byte-for-byte copy of microsoft/playwright packages/protocol/spec/*.yml, with its LICENSE and\n\
         # NOTICE. Never edit these files. To upgrade, run ../../sync-playwright.sh <playwright-checkout>,\n\
         # the only writer of this file. The translator refuses to run when the *.yml files here no\n\
         # longer hash to `sha256` (the framing in codegen/spec_files.rs, paths relative to spec/).\n\
         commit: {commit}\n\
         sha256: {sha256}\n"
    )
}

enum Outcome {
    Translated { mode: Mode, drift: Drift },
    Pinned { sha256: String },
}

enum Error {
    Usage,
    NoCrateDir,
    Io(io::Error),
    Spec(SpecError),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Usage => write!(
                formatter,
                "usage: {REGENERATE} [-- --check | -- --pin-playwright <commit>]"
            ),
            Error::NoCrateDir => write!(
                formatter,
                "CARGO_MANIFEST_DIR is not set; run the translator through Cargo: {REGENERATE}"
            ),
            Error::Io(error) => write!(formatter, "{error}"),
            Error::Spec(error) => write!(formatter, "{error}"),
        }
    }
}

/// Reports the outcome on stderr and picks the exit code.
#[expect(
    clippy::print_stderr,
    reason = "the translator is a developer tool that reports on stderr"
)]
fn report(outcome: Result<Outcome, Error>) -> ExitCode {
    match outcome {
        Ok(Outcome::Translated { drift, .. }) if drift.is_empty() => ExitCode::SUCCESS,
        Ok(Outcome::Translated {
            mode: Mode::Write,
            drift,
        }) => {
            for line in drift.lines(Mode::Write) {
                eprintln!("{line}");
            }
            ExitCode::SUCCESS
        }
        Ok(Outcome::Translated {
            mode: Mode::Check,
            drift,
        }) => {
            for line in drift.lines(Mode::Check) {
                eprintln!("{line}");
            }
            eprintln!("run `{REGENERATE}` in packages/browseros-agent");
            ExitCode::FAILURE
        }
        Ok(Outcome::Pinned { sha256 }) => {
            eprintln!("pinned spec/playwright at sha256 {sha256}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
