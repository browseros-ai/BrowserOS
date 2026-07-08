use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use toml_edit::{DocumentMut, value};

use super::InitArgs;

/// Result of writing the user's bpatch config.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct InitReport {
    pub result: InitResult,
    pub store: PathBuf,
    pub config: PathBuf,
    pub exit: i32,
}

/// Init result discriminator.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InitResult {
    Initialized,
}

/// Validates a patch store path and records it in the user config.
pub fn run(args: &InitArgs, config_path: &Path) -> Result<InitReport> {
    let store = resolve_store_dir(args.store_dir.as_deref())?;
    write_config(config_path, &store)?;
    Ok(InitReport {
        result: InitResult::Initialized,
        store,
        config: config_path.to_path_buf(),
        exit: 0,
    })
}

/// Renders init output for human terminals.
pub fn render_human(report: &InitReport) -> String {
    format!(
        "initialized store {}\nconfig      {}\n",
        report.store.display(),
        report.config.display()
    )
}

/// Renders init output as one JSON object.
pub fn render_json(report: &InitReport) -> Result<String> {
    Ok(serde_json::to_string(report)?)
}

fn resolve_store_dir(store_dir: Option<&Path>) -> Result<PathBuf> {
    let candidate = match store_dir {
        Some(store_dir) => store_dir.to_path_buf(),
        None => {
            let cwd = env::current_dir().context("reading current directory")?;
            if !cwd.join("store.yaml").exists() {
                bail!(
                    "init requires <STORE_DIR>; run from a patch store containing store.yaml or pass `bpatch init <dir>`"
                );
            }
            cwd
        }
    };
    let store = candidate
        .canonicalize()
        .with_context(|| format!("canonicalizing {}", candidate.display()))?;
    if !store.join("store.yaml").exists() {
        bail!(
            "patch store {} is missing store.yaml; pass a chromium_patches directory containing store.yaml",
            store.display()
        );
    }
    Ok(store)
}

fn write_config(config_path: &Path, store: &Path) -> Result<()> {
    let store = store
        .to_str()
        .ok_or_else(|| anyhow!("store path {} is not valid UTF-8", store.display()))?;
    let mut doc = load_config_document(config_path)?;
    let mut store_value = value(store);
    if let Some(existing) = doc.as_table_mut().get_mut("store") {
        if let Some(existing_value) = existing.as_value()
            && let Some(next) = store_value.as_value_mut()
        {
            *next.decor_mut() = existing_value.decor().clone();
        }
        *existing = store_value;
    } else {
        doc.as_table_mut().insert("store", store_value);
    }

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(config_path, doc.to_string())
        .with_context(|| format!("writing {}", config_path.display()))
}

fn load_config_document(config_path: &Path) -> Result<DocumentMut> {
    if !config_path.exists() {
        return Ok(DocumentMut::new());
    }
    let text = fs::read_to_string(config_path)
        .with_context(|| format!("reading {}", config_path.display()))?;
    text.parse::<DocumentMut>()
        .with_context(|| format!("parsing {}", config_path.display()))
}
