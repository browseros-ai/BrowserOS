use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand};
use serde::Serialize;
use toml_edit::{DocumentMut, Item, Table, value};

use crate::git::GitAdapter;

/// Alias command wrapper.
#[derive(Debug, Args)]
pub struct AliasArgs {
    /// Alias subcommand.
    #[command(subcommand)]
    pub command: AliasCommand,
}

/// Alias subcommands.
#[derive(Debug, Subcommand)]
pub enum AliasCommand {
    /// Add or update a checkout alias.
    Add(AliasAddArgs),
    /// List configured checkout aliases.
    List,
    /// Remove a checkout alias.
    Remove(AliasRemoveArgs),
}

/// Alias add flags.
#[derive(Debug, Args)]
pub struct AliasAddArgs {
    /// Alias name.
    pub name: String,
    /// Existing checkout path.
    pub path: PathBuf,
}

/// Alias remove flags.
#[derive(Debug, Args)]
pub struct AliasRemoveArgs {
    /// Alias name.
    pub name: String,
}

/// Serializable alias management result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum AliasReport {
    /// Alias was added or updated.
    Added {
        /// Alias name.
        alias: String,
        /// Canonical checkout path.
        path: PathBuf,
        /// Process exit code.
        exit: i32,
    },
    /// Alias was removed.
    Removed {
        /// Alias name.
        alias: String,
        /// Former checkout path.
        path: PathBuf,
        /// Process exit code.
        exit: i32,
    },
    /// Configured aliases were listed.
    Listed {
        /// Alias mapping ordered by alias.
        checkouts: BTreeMap<String, PathBuf>,
        /// Process exit code.
        exit: i32,
    },
}

impl AliasReport {
    /// Returns the process exit code represented by the report.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Added { exit, .. } | Self::Removed { exit, .. } | Self::Listed { exit, .. } => {
                *exit
            }
        }
    }
}

/// Runs checkout alias management without requiring a checkout or store.
pub fn run(args: &AliasArgs, json: bool) -> Result<i32> {
    let report = match &args.command {
        AliasCommand::Add(args) => add(&args.name, &args.path)?,
        AliasCommand::List => list()?,
        AliasCommand::Remove(args) => remove(&args.name)?,
    };
    super::write_output(json, &render_json(&report)?, &render_human(&report))?;
    Ok(report.exit_code())
}

fn add(name: &str, path: &Path) -> Result<AliasReport> {
    let checkout = super::canonical_checkout_path(path)?;
    GitAdapter::new(&checkout).preflight()?;

    let path = super::config_path();
    let mut doc = load_document()?;
    checkouts_table_mut(&mut doc)?.insert(name, value(checkout.display().to_string()));
    write_document(&path, &doc)?;

    Ok(AliasReport::Added {
        alias: name.to_string(),
        path: checkout,
        exit: 0,
    })
}

fn list() -> Result<AliasReport> {
    let path = super::config_path();
    let config = super::load_config(&path)?.unwrap_or_default();
    Ok(AliasReport::Listed {
        checkouts: config.checkouts,
        exit: 0,
    })
}

fn remove(name: &str) -> Result<AliasReport> {
    let path = super::config_path();
    let mut doc = load_document()?;
    let (removed, remove_table) = {
        let table = doc
            .as_table_mut()
            .get_mut("checkouts")
            .and_then(Item::as_table_mut)
            .ok_or_else(|| anyhow!("unknown checkout alias `{}`", name))?;
        let removed = table
            .get(name)
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .map(PathBuf::from)
            .ok_or_else(|| anyhow!("unknown checkout alias `{}`", name))?;
        table.remove(name);
        (removed, table.is_empty())
    };
    if remove_table {
        doc.as_table_mut().remove("checkouts");
    }
    write_document(&path, &doc)?;

    Ok(AliasReport::Removed {
        alias: name.to_string(),
        path: removed,
        exit: 0,
    })
}

fn load_document() -> Result<DocumentMut> {
    let path = super::config_path();
    if !path.exists() {
        return Ok(DocumentMut::new());
    }
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    text.parse::<DocumentMut>()
        .with_context(|| format!("parsing {}", path.display()))
}

fn checkouts_table_mut(doc: &mut DocumentMut) -> Result<&mut Table> {
    if !doc.as_table().contains_key("checkouts") {
        doc["checkouts"] = Item::Table(Table::new());
    }
    doc["checkouts"]
        .as_table_mut()
        .ok_or_else(|| anyhow!("config key `checkouts` must be a table"))
}

fn write_document(path: &Path, doc: &DocumentMut) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    } else {
        bail!("config path has no parent: {}", path.display());
    }
    fs::write(path, doc.to_string()).with_context(|| format!("writing {}", path.display()))
}

fn render_human(report: &AliasReport) -> String {
    match report {
        AliasReport::Added { alias, path, .. } => {
            format!("alias added: {alias} -> {}\n", path.display())
        }
        AliasReport::Removed { alias, path, .. } => {
            format!("alias removed: {alias} -> {}\n", path.display())
        }
        AliasReport::Listed { checkouts, .. } => {
            if checkouts.is_empty() {
                return "no checkout aliases configured\n".to_string();
            }
            let mut out = String::from("checkout aliases:\n");
            for (alias, path) in checkouts {
                out.push_str(&format!("  {:<16} {}\n", alias, path.display()));
            }
            out
        }
    }
}

fn render_json(report: &AliasReport) -> Result<String> {
    Ok(serde_json::to_string(report)?)
}
