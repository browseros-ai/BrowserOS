pub mod abort;
pub mod apply;
mod checkout_guard;
pub mod continue_cmd;
pub mod diff;
pub mod extract;
pub mod feature;
pub mod init;
pub mod render;
pub mod status;

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Parser, Subcommand};
use serde::Deserialize;
use serde_json::json;
use thiserror::Error;

use crate::engine::apply::ApplyOptions;
use crate::engine::extract::{ExtractContext, ExtractSpec, FeatureDecisionPolicy};
use crate::engine::state::StateContext;
use crate::git::GitAdapter;
use crate::store::Store;

/// Top-level bpatch command-line interface.
#[derive(Debug, Parser)]
#[command(
    name = "bpatch",
    about = "Manage BrowserOS Chromium patches",
    after_long_help = r#"GETTING STARTED:
  Configure the patch store once:
    bpatch init /abs/path/to/chromium_patches

  Or run bpatch init from inside chromium_patches.
  Pass --store /abs/path/to/chromium_patches on store-reading commands.
  Run bpatch from inside a Chromium checkout.

GLOBAL FLAGS:
  --store <STORE>  Overrides the config file for store-reading commands.
  --json           Emits a single JSON object and suppresses progress and prompts.

EXAMPLES:
  Setup:
    bpatch init /abs/path/to/chromium_patches

  Daily loop:
    bpatch status
    bpatch diff
    bpatch apply

  Extract checkout commits into the store:
    bpatch extract <rev1>..<rev2> --feature <name>

  Base upgrade:
    bpatch apply -> bpatch continue --materialize -> resolve markers -> bpatch continue -> bpatch extract --repin

EXIT CODES:
  0  Initialized, converged, applied, extracted, repinned, listed, added, aborted, or completed.
  2  Conflicts are pending or conflict files remain unresolved.
  3  Drift/refusal or extract needs a feature decision.
  1  CLI, git, lock, config, or unexpected error.
"#
)]
pub struct Cli {
    /// Override the config file's chromium_patches store directory for store-reading commands.
    #[arg(long, global = true)]
    pub store: Option<PathBuf>,
    /// Emit a single JSON object and disable progress and prompts.
    #[arg(long, global = true)]
    pub json: bool,
    /// Command to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Supported bpatch verbs.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show checkout/store state.
    #[command(
        long_about = "Show checkout base, store rev, applied trailers, and drift.",
        after_long_help = r#"EXAMPLE:
  bpatch status
"#
    )]
    Status,
    /// Show what apply would touch.
    #[command(
        long_about = "Show what apply would touch, grouped by feature, with a rebuild-scope hint.",
        after_long_help = r#"EXAMPLE:
  bpatch diff
"#
    )]
    Diff,
    /// Converge the checkout to the store.
    #[command(
        long_about = "Optionally fast-forward the store repo with --pull, then converge the checkout to the store. Exit 2 means conflicts are pending; exit 3 means drift or refusal blocked the write.",
        after_long_help = r#"EXAMPLE:
  bpatch apply --pull
"#
    )]
    Apply(ApplyArgs),
    /// Extract commits into the store or repin the store base.
    #[command(
        long_about = "Extract <rev> or <rev1>..<rev2> into the store, or repin existing store patches to the checkout base. Use --feature <FEATURE> to route unmatched files, --commit to commit store repo changes, and --repin without a spec for base upgrades.",
        after_long_help = r#"EXAMPLE:
  bpatch extract <rev1>..<rev2> --feature <name>
"#
    )]
    Extract(ExtractArgs),
    /// Manage features.yaml entries.
    #[command(
        long_about = "Manage features.yaml entries. List the feature inventory or append a new feature block with an owned path.",
        after_long_help = r#"EXAMPLES:
  bpatch feature list
  bpatch feature add wallet --path chrome/browser/browseros/wallet/
"#
    )]
    Feature(FeatureArgs),
    /// Write the patch store path to the user config.
    #[command(
        long_about = "Canonicalize a chromium_patches store directory, validate that it contains store.yaml, and write it to ~/.config/bpatch/config.toml while preserving other config keys and comments.",
        after_long_help = r#"EXAMPLES:
  bpatch init /abs/path/to/chromium_patches
  cd /abs/path/to/chromium_patches && bpatch init
"#
    )]
    Init(InitArgs),
    /// Abort a conflict session.
    #[command(
        long_about = "Remove a pending conflict session. Before continue --materialize, abort only deletes the session file; the worktree has not been touched.",
        after_long_help = r#"EXAMPLE:
  bpatch abort
"#
    )]
    Abort,
    /// Continue a conflict session.
    #[command(
        long_about = "Use continue --materialize first to write conflict marker files, then resolve markers and run bare continue to finish convergence.",
        after_long_help = r#"EXAMPLE:
  bpatch continue --materialize -> resolve markers -> bpatch continue
"#
    )]
    Continue(ContinueArgs),
}

/// Apply command flags.
#[derive(Debug, Args)]
pub struct ApplyArgs {
    /// Fast-forward the store repository before applying.
    #[arg(long)]
    pub pull: bool,
}

/// Extract command flags.
#[derive(Debug, Args)]
pub struct ExtractArgs {
    /// Revision or rev1..rev2 range to extract.
    pub spec: Option<String>,
    /// Route unmatched files to this feature.
    #[arg(long)]
    pub feature: Option<String>,
    /// Commit store repo changes after writing them.
    #[arg(long)]
    pub commit: bool,
    /// Re-diff existing store patches against the checkout's current base.
    #[arg(long)]
    pub repin: bool,
    /// Accept nearest feature suggestions without prompting.
    #[arg(long, hide = true)]
    pub accept_suggestions: bool,
}

/// Feature command wrapper.
#[derive(Debug, Args)]
pub struct FeatureArgs {
    /// Feature subcommand.
    #[command(subcommand)]
    pub command: FeatureCommand,
}

/// Feature subcommands.
#[derive(Debug, Subcommand)]
pub enum FeatureCommand {
    /// List features, patch counts, and last applied sequence.
    #[command(
        long_about = "List features, owned patch counts, and last applied sequence numbers.",
        after_long_help = r#"EXAMPLE:
  bpatch feature list
"#
    )]
    List,
    /// Add a feature path block.
    #[command(
        long_about = "Append a new feature block to features.yaml. Provide a feature name and an exact path or directory prefix with --path.",
        after_long_help = r#"EXAMPLE:
  bpatch feature add wallet --path chrome/browser/browseros/wallet/ --description "Wallet UI"
"#
    )]
    Add(FeatureAddArgs),
}

/// Feature add flags.
#[derive(Debug, Args)]
pub struct FeatureAddArgs {
    /// Feature name.
    pub name: String,
    /// Path or prefix owned by the feature.
    #[arg(long)]
    pub path: String,
    /// Feature description.
    #[arg(long)]
    pub description: Option<String>,
}

/// Continue command flags.
#[derive(Debug, Args)]
pub struct ContinueArgs {
    /// Write conflict marker files instead of finishing convergence.
    #[arg(long)]
    pub materialize: bool,
}

/// Init command arguments.
#[derive(Debug, Args)]
pub struct InitArgs {
    /// chromium_patches store directory. Defaults to cwd when cwd contains store.yaml.
    pub store_dir: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct Config {
    store: Option<PathBuf>,
}

/// Runs the parsed CLI and returns the process exit code.
pub fn run(cli: Cli) -> i32 {
    match run_inner(&cli) {
        Ok(code) => code,
        Err(err) => {
            let exit = error_exit(&err);
            write_error(cli.json, &err, exit);
            exit
        }
    }
}

fn run_inner(cli: &Cli) -> Result<i32> {
    if let Command::Init(args) = &cli.command {
        let report = init::run(args, &config_path())?;
        write_output(
            cli.json,
            &init::render_json(&report)?,
            &init::render_human(&report),
        )?;
        return Ok(0);
    }

    let checkout = discover_checkout(&env::current_dir()?)?;
    GitAdapter::new(&checkout).preflight()?;
    let store_dir = discover_store(cli.store.as_deref())?;
    if needs_checkout_store_guard(&cli.command) {
        checkout_guard::ensure_matches_store(&checkout, &store_dir)?;
    }
    let state_ctx = StateContext::new(&checkout, &store_dir);

    match &cli.command {
        Command::Status => {
            let report = status::run(&state_ctx)?;
            write_output(
                cli.json,
                &status::render_json(&report)?,
                &status::render_human(&report),
            )?;
            Ok(0)
        }
        Command::Diff => {
            let report = diff::run(&state_ctx)?;
            write_output(
                cli.json,
                &diff::render_json(&report)?,
                &diff::render_human(&report),
            )?;
            Ok(0)
        }
        Command::Apply(args) => {
            let mut progress = render::progress_sink(cli.json);
            let report = apply::run(&state_ctx, ApplyOptions { pull: args.pull }, &mut progress);
            write_output(
                cli.json,
                &apply::render_json(&report)?,
                &apply::render_human(&report),
            )?;
            Ok(report.exit_code())
        }
        Command::Extract(args) => run_extract(cli, args, &checkout, &store_dir),
        Command::Feature(args) => run_feature(cli, args, &state_ctx, &store_dir),
        Command::Init(_) => unreachable!("init dispatches before checkout/store discovery"),
        Command::Abort => {
            let report = abort::run(&state_ctx);
            write_output(
                cli.json,
                &abort::render_json(&report)?,
                &abort::render_human(&report),
            )?;
            Ok(report.exit_code())
        }
        Command::Continue(args) => {
            let mut progress = render::progress_sink(cli.json);
            let report = continue_cmd::run(
                &state_ctx,
                continue_cmd::ContinueOptions {
                    materialize: args.materialize,
                },
                &mut progress,
            );
            write_output(
                cli.json,
                &continue_cmd::render_json(&report)?,
                &continue_cmd::render_human(&report),
            )?;
            Ok(report.exit_code())
        }
    }
}

fn run_extract(cli: &Cli, args: &ExtractArgs, checkout: &Path, store_dir: &Path) -> Result<i32> {
    let ctx = ExtractContext::new(checkout, store_dir);
    let mode = if args.repin {
        if args.spec.is_some() {
            bail!("extract --repin does not accept a revision argument");
        }
        extract::ExtractMode::Repin
    } else {
        let spec = args
            .spec
            .as_ref()
            .ok_or_else(|| anyhow!("extract requires <rev | rev1..rev2> unless --repin is set"))?;
        let policy = extract_policy(args);
        extract::ExtractMode::Revs {
            spec: ExtractSpec::parse(spec)?,
            policy,
        }
    };

    let mut progress = render::progress_sink(cli.json);
    let options = extract::ExtractOptions {
        mode,
        commit: args.commit,
    };
    let mut report = extract::run(&ctx, &options, &mut progress)?;

    if matches!(report.result, extract::ExtractReportResult::NeedsFeature)
        && args.feature.is_none()
        && !args.accept_suggestions
        && render::is_interactive(cli.json)
    {
        let suggestion = report.suggestion.clone().unwrap_or_default();
        let count = report.unmatched.len();
        let policy = if store_has_feature(store_dir, &suggestion)? {
            if render::prompt_accept_suggestion(count, &suggestion)? {
                FeatureDecisionPolicy::AcceptSuggestions
            } else {
                FeatureDecisionPolicy::RequireExplicit
            }
        } else {
            FeatureDecisionPolicy::Named(render::prompt_feature_name(count, &suggestion)?)
        };
        if !matches!(policy, FeatureDecisionPolicy::RequireExplicit) {
            let spec = args.spec.as_ref().expect("checked above");
            let retry = extract::ExtractOptions {
                mode: extract::ExtractMode::Revs {
                    spec: ExtractSpec::parse(spec)?,
                    policy,
                },
                commit: args.commit,
            };
            let mut retry_progress = render::progress_sink(cli.json);
            report = extract::run(&ctx, &retry, &mut retry_progress)?;
        }
    }

    write_output(
        cli.json,
        &extract::render_json(&report)?,
        &extract::render_human(&report),
    )?;
    Ok(report.exit)
}

fn run_feature(
    cli: &Cli,
    args: &FeatureArgs,
    state_ctx: &StateContext,
    store_dir: &Path,
) -> Result<i32> {
    let report = match &args.command {
        FeatureCommand::List => feature::list(state_ctx)?,
        FeatureCommand::Add(args) => feature::add(
            store_dir,
            &args.name,
            &args.path,
            args.description.as_deref(),
        )?,
    };
    write_output(
        cli.json,
        &feature::render_json(&report)?,
        &feature::render_human(&report),
    )?;
    Ok(report.exit_code())
}

fn extract_policy(args: &ExtractArgs) -> FeatureDecisionPolicy {
    if let Some(feature) = &args.feature {
        FeatureDecisionPolicy::Named(feature.clone())
    } else if args.accept_suggestions {
        FeatureDecisionPolicy::AcceptSuggestions
    } else {
        FeatureDecisionPolicy::RequireExplicit
    }
}

fn needs_checkout_store_guard(command: &Command) -> bool {
    matches!(command, Command::Status | Command::Diff | Command::Apply(_))
}

#[derive(Debug, Error)]
#[error("{reason}")]
struct CliFailure {
    reason: String,
    exit: i32,
}

fn refusal(reason: impl Into<String>) -> anyhow::Error {
    CliFailure {
        reason: reason.into(),
        exit: 3,
    }
    .into()
}

fn error_exit(err: &anyhow::Error) -> i32 {
    err.downcast_ref::<CliFailure>()
        .map(|failure| failure.exit)
        .unwrap_or(1)
}

fn discover_checkout(cwd: &Path) -> Result<PathBuf> {
    for dir in cwd.ancestors() {
        if dir.join(".git").exists() {
            return Ok(dir.to_path_buf());
        }
    }
    bail!(
        "could not find a git checkout from {}; run bpatch inside a Chromium checkout",
        cwd.display()
    )
}

fn discover_store(flag: Option<&Path>) -> Result<PathBuf> {
    let config_path = config_path();
    let store = if let Some(store) = flag {
        store.to_path_buf()
    } else {
        let Some(config) = load_config(&config_path)? else {
            bail!("{}", missing_store_message(&config_path));
        };
        config
            .store
            .ok_or_else(|| anyhow!("{}", missing_store_message(&config_path)))?
    };

    if !store.join("store.yaml").exists() {
        bail!(
            "patch store {} is missing store.yaml; pass --store <dir>, run `bpatch init <dir>`, or set `store = \"/abs/path\"` in {}",
            store.display(),
            config_path.display()
        );
    }
    Ok(store)
}

fn load_config(path: &Path) -> Result<Option<Config>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(Some(
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?,
    ))
}

fn config_path() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/bpatch/config.toml")
}

fn missing_store_message(config_path: &Path) -> String {
    format!(
        "missing patch store; pass --store <dir>, run `bpatch init <dir>`, or set `store = \"/abs/path\"` in {}",
        config_path.display()
    )
}

fn store_has_feature(store_dir: &Path, feature: &str) -> Result<bool> {
    Ok(Store::load(store_dir)?
        .features()
        .features
        .contains_key(feature))
}

fn write_output(json: bool, json_text: &str, human: &str) -> Result<()> {
    render::clear_live_progress(json);
    if json {
        println!("{json_text}");
    } else {
        print!("{human}");
        io::stdout().flush()?;
    }
    Ok(())
}

fn write_error(json_mode: bool, err: &anyhow::Error, exit: i32) {
    render::clear_live_progress(json_mode);
    let reason = format!("{err:#}");
    if json_mode {
        println!(
            "{}",
            json!({ "result": "error", "reason": reason, "exit": exit })
        );
    } else {
        eprintln!("error: {reason}");
    }
}
