use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

/// Machine-level bpatch configuration listing.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LsReport {
    pub result: LsResult,
    pub store: Option<PathBuf>,
    pub checkouts: BTreeMap<String, PathBuf>,
    pub config: PathBuf,
    pub exit: i32,
}

/// Listing result discriminator.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LsResult {
    Listed,
}

impl LsReport {
    /// Returns the process exit code represented by the report.
    pub fn exit_code(&self) -> i32 {
        self.exit
    }
}

/// Lists configured patch store and checkout aliases without requiring a checkout.
pub fn run(store_override: Option<&Path>, config_path: &Path) -> Result<LsReport> {
    let config = super::load_config(config_path)?.unwrap_or_default();
    let store = store_override
        .map(PathBuf::from)
        .or_else(|| config.store.clone());
    Ok(LsReport {
        result: LsResult::Listed,
        store,
        checkouts: config.checkouts,
        config: config_path.to_path_buf(),
        exit: 0,
    })
}

/// Renders listing output for human terminals.
pub fn render_human(report: &LsReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("config     {}\n", report.config.display()));
    match &report.store {
        Some(store) => out.push_str(&format!("store      {}\n", store.display())),
        None => out.push_str("store      not configured\n"),
    }
    if report.checkouts.is_empty() {
        out.push_str("checkouts  none\n");
    } else {
        out.push_str("checkouts\n");
        for (alias, path) in &report.checkouts {
            out.push_str(&format!("  {:<16} {}\n", alias, path.display()));
        }
    }
    out
}

/// Renders listing output as one JSON object.
pub fn render_json(report: &LsReport) -> Result<String> {
    Ok(serde_json::to_string(report)?)
}
