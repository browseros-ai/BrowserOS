//! `GET /v1/metrics` as the host API contract (`neo-sdk-api/openapi.yaml`) describes it, against the live pipeline's first scrape: the families it lists, in its order, the series it says start at zero, and nothing else before Neo records.

use std::error::Error;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use neo_telemetry::{OpenTabCounts, Telemetry, TelemetryConfig};

#[test]
fn the_contract_lists_what_a_first_scrape_writes() -> Result<(), Box<dyn Error>> {
    let (telemetry, _probe) = Telemetry::in_memory(&TelemetryConfig::default());
    let exposition = telemetry.render_prometheus();
    assert_eq!(
        listed(&contract()?)?,
        typed(&exposition),
        "openapi.yaml's /v1/metrics lists every family a scrape writes, in its order, and no other"
    );
    assert_eq!(
        samples(&exposition),
        ["neo_run_active 0", "neo_renderer_crashes_total 0"],
        "before Neo records anything only the two unlabelled series have a value, both zero: the \
         body is never empty"
    );
    // The app sets the tab registry's source before the browser links, so before the host API
    // serves its first scrape.
    telemetry.set_open_tabs(Arc::new(|| OpenTabCounts {
        agent: 0,
        found: 2,
        neo: 1,
    }));
    let origins = telemetry
        .render_prometheus()
        .lines()
        .filter(|line| line.starts_with("neo_tab_count{"))
        .count();
    assert_eq!(origins, 3, "`neo_tab_count` has a series for each origin");
    Ok(())
}

/// The `/v1/metrics` path of the host API contract, as one line of text.
fn contract() -> Result<String, Box<dyn Error>> {
    let spec = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("neo-telemetry sits in crates/neo-sdk")?
        .join("neo-sdk-api/openapi.yaml");
    let yaml = fs::read_to_string(spec)?;
    // The path's own lines: deeper than a path key, up to the next path or `components:`.
    let route: Vec<&str> = yaml
        .lines()
        .skip_while(|line| *line != "  /v1/metrics:")
        .skip(1)
        .take_while(|line| line.is_empty() || line.starts_with("   "))
        .flat_map(str::split_whitespace)
        .collect();
    if route.is_empty() {
        return Err("openapi.yaml has no /v1/metrics path".into());
    }
    Ok(route.join(" "))
}

/// The families the contract lists, in its order: each quoted name followed by its OpenTelemetry
/// name in parentheses.
fn listed(route: &str) -> Result<Vec<&str>, Box<dyn Error>> {
    let (_, list) = route
        .split_once("with their labels:")
        .ok_or("the /v1/metrics description lists its families")?;
    let (list, _) = list
        .split_once("Every one has")
        .ok_or("the list is followed by what a first scrape holds")?;
    let pieces: Vec<&str> = list.split("` (`").collect();
    Ok(pieces
        .iter()
        .take(pieces.len().saturating_sub(1))
        .filter_map(|piece| piece.rsplit('`').next())
        .collect())
}

/// The family of each `# TYPE` line, in the order the scrape writes them.
fn typed(exposition: &str) -> Vec<&str> {
    exposition
        .lines()
        .filter_map(|line| line.strip_prefix("# TYPE "))
        .filter_map(|rest| rest.split(' ').next())
        .collect()
}

/// Every sample line, in order.
fn samples(exposition: &str) -> Vec<&str> {
    exposition
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
}
