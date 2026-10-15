//! `suggest_schedule` / `suggest_app_connection`: tools whose output is a sentinel JSON the side
//! panel renders as a card. Omitted for scheduled runs and chat mode.

use super::{ToolSource, unimplemented_source};
use std::sync::Arc;

pub fn nudge_tools(connector_names: Vec<String>) -> Arc<dyn ToolSource> {
    let _ = connector_names;
    unimplemented_source("nudges")
}
