pub mod abort;
pub mod alias;
pub mod annotate;
pub mod apply;
mod checkout_guard;
pub mod continue_cmd;
pub mod diff;
pub mod extract;
pub mod feature;
pub mod init;
pub mod ls;
pub mod render;
pub mod status;

use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
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
use crate::store::{self, Store};

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
  Run bpatch ls from any directory to show configured machine paths.

GLOBAL FLAGS:
  --store <STORE>  Overrides the config file for store-reading commands.
  -C, --checkout   Targets a checkout alias or path instead of discovering from cwd.
  --json           Emits a single JSON object and suppresses progress and prompts.

EXAMPLES:
  Setup:
    bpatch init /abs/path/to/chromium_patches
    bpatch ls

  Daily loop:
    bpatch status
    bpatch diff
    bpatch apply
    bpatch annotate

  Extract checkout commits into the store:
    bpatch extract <rev1>..<rev2> --feature <name>

  Base upgrade:
    bpatch apply -> bpatch continue --materialize -> resolve markers -> bpatch continue -> bpatch extract --repin

EXIT CODES:
  0  Initialized, converged, applied, annotated, extracted, repinned, listed, added, aborted, or completed.
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
    /// Target a checkout alias or path instead of discovering from cwd.
    #[arg(
        id = "checkout_flag",
        short = 'C',
        long = "checkout",
        global = true,
        value_name = "CHECKOUT"
    )]
    pub checkout: Option<String>,
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
  bpatch status ch1
"#
    )]
    Status(CheckoutArgs),
    /// Show what apply would touch.
    #[command(
        long_about = "Show what apply would touch, grouped by feature, with a rebuild-scope hint.",
        after_long_help = r#"EXAMPLE:
  bpatch diff
  bpatch diff ch1
"#
    )]
    Diff(CheckoutArgs),
    /// Converge the checkout to the store.
    #[command(
        long_about = "Optionally fast-forward the store repo with --pull, then converge the checkout to the store. Exit 2 means conflicts are pending; exit 3 means drift or refusal blocked the write.",
        after_long_help = r#"EXAMPLE:
  bpatch apply --pull
"#
    )]
    Apply(ApplyArgs),
    /// Commit dirty bos_build output into feature/resource commits.
    #[command(
        long_about = "Commit a dirty bos_build-patched checkout into feature commits, one resource commit for store:false groups, and optionally one wip commit for leftovers. Exit 3 means unclaimed paths were left in the working tree.",
        after_long_help = r#"EXAMPLES:
  bpatch annotate
  bpatch annotate --rest wip-frame-experiments
  bpatch annotate --triage --json
"#
    )]
    Annotate(AnnotateArgs),
    /// Extract commits into the store or repin the store base.
    #[command(
        long_about = "Extract <rev> or <rev1>..<rev2> into the store, or repin existing store patches to the checkout base. Use --feature <FEATURE> to route unmatched files, --commit to commit store repo changes, and --repin without a spec for base upgrades.",
        after_long_help = r#"EXAMPLE:
  bpatch extract <rev1>..<rev2> --feature <name>
"#
    )]
    Extract(ExtractArgs),
    /// Manage .features.yaml entries.
    #[command(
        long_about = "Manage .features.yaml entries. List the feature inventory or append a new feature block with an owned path.",
        after_long_help = r#"EXAMPLES:
  bpatch feature list
  bpatch feature add wallet --path chrome/browser/browseros/wallet/
"#
    )]
    Feature(FeatureArgs),
    /// Manage checkout aliases.
    #[command(
        long_about = "Manage checkout aliases in ~/.config/bpatch/config.toml. Aliases let checkout-scoped commands target a Chromium checkout from any directory.",
        after_long_help = r#"EXAMPLES:
  bpatch alias add ch1 /Users/shadowfax/ch1-src
  bpatch alias list
  bpatch alias remove ch1
"#
    )]
    Alias(alias::AliasArgs),
    /// List configured patch store and checkout aliases.
    #[command(
        long_about = "List the configured chromium_patches store and registered Chromium checkout aliases without requiring checkout discovery.",
        after_long_help = r#"EXAMPLE:
  bpatch ls
  bpatch --store /abs/path/to/chromium_patches ls --json

NOTE:
  bpatch ls does not accept -C/--checkout because it lists machine config, not a checkout target.
"#
    )]
    Ls,
    /// Write the patch store path to the user config.
    #[command(
        long_about = "Canonicalize a chromium_patches store directory, validate that it contains bpatch metadata, and write it to ~/.config/bpatch/config.toml while preserving other config keys and comments.",
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
    Abort(CheckoutArgs),
    /// Continue a conflict session.
    #[command(
        long_about = "Use continue --materialize first to write conflict marker files, then resolve markers and run bare continue to finish convergence.",
        after_long_help = r#"EXAMPLE:
  bpatch continue --materialize -> resolve markers -> bpatch continue
"#
    )]
    Continue(ContinueArgs),
}

/// Positional checkout selector shared by checkout-scoped commands.
#[derive(Debug, Args)]
pub struct CheckoutArgs {
    /// Checkout alias or path.
    pub checkout: Option<String>,
}

/// Apply command flags.
#[derive(Debug, Args)]
pub struct ApplyArgs {
    /// Fast-forward the store repository before applying.
    #[arg(long)]
    pub pull: bool,
    /// Checkout alias or path.
    pub checkout: Option<String>,
}

/// Annotate command flags.
#[derive(Debug, Args)]
pub struct AnnotateArgs {
    /// Commit unclaimed leftovers into one `wip: <NAME>` commit.
    #[arg(long)]
    pub rest: Option<String>,
    /// Emit a triage plan instead of committing; JSON mode never opens an editor.
    #[arg(long)]
    pub triage: bool,
    /// Checkout alias or path.
    pub checkout: Option<String>,
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
    /// Add paths to a feature block.
    #[command(
        long_about = "Append paths to .features.yaml. Provide explicit paths, or use --from-dirty to claim currently unclaimed dirty paths. Use --store=false for resource groups that annotate commits but extract/apply ignore.",
        after_long_help = r#"EXAMPLE:
  bpatch feature add wallet chrome/browser/browseros/wallet/ --desc "feat: wallet"
  bpatch feature add build-resources --store=false --from-dirty
"#
    )]
    Add(FeatureAddArgs),
}

/// Feature add flags.
#[derive(Debug, Args)]
pub struct FeatureAddArgs {
    /// Feature name.
    pub name: String,
    /// Path or prefixes owned by the feature.
    pub paths: Vec<String>,
    /// Path or prefix owned by the feature. Kept for older scripts.
    #[arg(long)]
    pub path: Vec<String>,
    /// Feature description.
    #[arg(long, alias = "desc")]
    pub description: Option<String>,
    /// Claim currently unclaimed dirty checkout paths.
    #[arg(long)]
    pub from_dirty: bool,
    /// Commit the store repo after updating .features.yaml.
    #[arg(long)]
    pub commit: bool,
    /// Hidden normalized spelling for `feature add ... --store=false`.
    #[arg(id = "feature_store", long = "feature-store", hide = true)]
    pub feature_store: Option<bool>,
}

/// Continue command flags.
#[derive(Debug, Args)]
pub struct ContinueArgs {
    /// Write conflict marker files instead of finishing convergence.
    #[arg(long)]
    pub materialize: bool,
    /// Checkout alias or path.
    pub checkout: Option<String>,
}

impl Command {
    fn positional_checkout(&self) -> Option<&str> {
        match self {
            Self::Status(args) | Self::Diff(args) | Self::Abort(args) => args.checkout.as_deref(),
            Self::Apply(args) => args.checkout.as_deref(),
            Self::Annotate(args) => args.checkout.as_deref(),
            Self::Continue(args) => args.checkout.as_deref(),
            Self::Extract(_) | Self::Feature(_) | Self::Alias(_) | Self::Ls | Self::Init(_) => None,
        }
    }
}

/// Init command arguments.
#[derive(Debug, Args)]
pub struct InitArgs {
    /// chromium_patches store directory. Defaults to cwd when cwd has bpatch metadata.
    pub store_dir: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct Config {
    store: Option<PathBuf>,
    #[serde(default)]
    checkouts: BTreeMap<String, PathBuf>,
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
    if let Command::Alias(args) = &cli.command {
        if cli.checkout.is_some() {
            bail!(
                "alias commands do not accept -C/--checkout; aliases manage the global checkout map"
            );
        }
        return alias::run(args, cli.json);
    }
    if matches!(&cli.command, Command::Ls) {
        if cli.checkout.is_some() {
            bail!(
                "ls does not accept -C/--checkout; it lists machine config and does not target a checkout"
            );
        }
        let config_path = config_path();
        let report = ls::run(cli.store.as_deref(), &config_path)?;
        write_output(
            cli.json,
            &ls::render_json(&report)?,
            &ls::render_human(&report),
        )?;
        return Ok(report.exit_code());
    }
    if let Command::Init(args) = &cli.command {
        let report = init::run(args, &config_path())?;
        write_output(
            cli.json,
            &init::render_json(&report)?,
            &init::render_human(&report),
        )?;
        return Ok(0);
    }

    let checkout = resolve_checkout(cli)?;
    GitAdapter::new(&checkout).preflight()?;
    let store_dir = discover_store(cli.store.as_deref())?;
    if needs_checkout_store_guard(&cli.command) {
        checkout_guard::ensure_matches_store(&checkout, &store_dir)?;
    }
    let state_ctx = StateContext::new(&checkout, &store_dir);

    match &cli.command {
        Command::Status(_) => {
            let report = status::run(&state_ctx)?;
            write_output(
                cli.json,
                &status::render_json(&report)?,
                &status::render_human(&report),
            )?;
            Ok(0)
        }
        Command::Diff(_) => {
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
        Command::Annotate(args) => {
            let report = if args.triage {
                if render::is_interactive(cli.json) {
                    let mut progress = render::progress_sink(cli.json);
                    annotate::triage_editor(&state_ctx, &mut progress)?
                } else {
                    annotate::triage(&state_ctx)?
                }
            } else {
                let mut progress = render::progress_sink(cli.json);
                annotate::run(
                    &state_ctx,
                    &crate::engine::annotate::AnnotateOptions {
                        rest: args.rest.clone(),
                    },
                    &mut progress,
                )?
            };
            write_output(
                cli.json,
                &annotate::render_json(&report)?,
                &annotate::render_human(&report),
            )?;
            Ok(report.exit_code())
        }
        Command::Extract(args) => run_extract(cli, args, &checkout, &store_dir),
        Command::Feature(args) => run_feature(cli, args, &state_ctx, &store_dir),
        Command::Alias(_) => unreachable!("alias dispatches before checkout/store discovery"),
        Command::Ls => unreachable!("ls dispatches before checkout/store discovery"),
        Command::Init(_) => unreachable!("init dispatches before checkout/store discovery"),
        Command::Abort(_) => {
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

/// Normalizes feature-add's `--store=false` before clap sees the global store flag.
pub fn normalize_args(args: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    let mut out = Vec::new();
    let mut saw_feature = false;
    let mut in_feature_add = false;
    let mut iter = args.into_iter().peekable();
    while let Some(arg) = iter.next() {
        let text = arg.to_string_lossy();
        if text == "feature" {
            saw_feature = true;
            out.push(arg);
            continue;
        }
        if saw_feature && text == "add" {
            in_feature_add = true;
            out.push(arg);
            continue;
        }
        if in_feature_add && (text == "--store=false" || text == "--store=true") {
            out.push(OsString::from(text.replacen(
                "--store",
                "--feature-store",
                1,
            )));
            continue;
        }
        if in_feature_add
            && text == "--store"
            && let Some(next) = iter.peek()
            && matches!(next.to_string_lossy().as_ref(), "false" | "true")
        {
            out.push(OsString::from("--feature-store"));
            out.push(iter.next().expect("peeked"));
            continue;
        }
        out.push(arg);
    }
    out
}

/// Resolves `-C` or positional checkout selectors into the checkout used by all checkout verbs.
fn resolve_checkout(cli: &Cli) -> Result<PathBuf> {
    match (cli.checkout.as_deref(), cli.command.positional_checkout()) {
        (None, None) => discover_checkout(&env::current_dir()?),
        (Some(selector), None) | (None, Some(selector)) => {
            let config_path = config_path();
            let config = load_config(&config_path)?;
            resolve_checkout_selector(selector, config.as_ref())
        }
        (Some(flag), Some(positional)) => {
            let config_path = config_path();
            let config = load_config(&config_path)?;
            let flag_checkout = resolve_checkout_selector(flag, config.as_ref())?;
            let positional_checkout = resolve_checkout_selector(positional, config.as_ref())?;
            if flag_checkout != positional_checkout {
                bail!(
                    "-C/--checkout `{}` and positional checkout `{}` resolve to different checkouts ({} != {})",
                    flag,
                    positional,
                    flag_checkout.display(),
                    positional_checkout.display()
                );
            }
            Ok(flag_checkout)
        }
    }
}

fn resolve_checkout_selector(selector: &str, config: Option<&Config>) -> Result<PathBuf> {
    if let Some(path) = config.and_then(|config| config.checkouts.get(selector)) {
        return canonical_checkout_path(path).with_context(|| {
            format!(
                "checkout alias `{}` points to invalid path {}",
                selector,
                path.display()
            )
        });
    }

    let path = PathBuf::from(selector);
    if path.is_dir() {
        return canonical_checkout_path(&path);
    }

    bail!(
        "unknown checkout `{}`; known aliases: {}",
        selector,
        format_known_aliases(config)
    )
}

fn canonical_checkout_path(path: &Path) -> Result<PathBuf> {
    let path = path
        .canonicalize()
        .with_context(|| format!("resolving {}", path.display()))?;
    discover_checkout(&path)?
        .canonicalize()
        .with_context(|| format!("resolving checkout root from {}", path.display()))
}

fn format_known_aliases(config: Option<&Config>) -> String {
    let Some(config) = config else {
        return "none".to_string();
    };
    if config.checkouts.is_empty() {
        "none".to_string()
    } else {
        config
            .checkouts
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
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
            state_ctx,
            store_dir,
            &args.name,
            feature::FeatureAddOptions {
                paths: args.paths.iter().chain(args.path.iter()).cloned().collect(),
                description: args.description.clone(),
                store: args.feature_store.unwrap_or(true),
                from_dirty: args.from_dirty,
                commit: args.commit,
            },
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
    matches!(
        command,
        Command::Status(_) | Command::Diff(_) | Command::Apply(_) | Command::Annotate(_)
    )
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

    store::validate_metadata_layout(&store).with_context(|| {
        format!(
            "invalid patch store {}; pass --store <dir>, run `bpatch init <dir>`, or set `store = \"/abs/path\"` in {}",
            store.display(),
            config_path.display()
        )
    })?;
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
