//! Playwright's lines in Rust against upstream's printer itself, in QuickJS on seeded random
//! nodes.
//!
//! The random run is small here, as the gate runs it on every change. A long run sets
//! `NEO_SNAPSHOT_DIFFERENTIAL_NODES` and `NEO_SNAPSHOT_DIFFERENTIAL_SEED` (decimal, or hex after
//! `0x`).

mod bundles;
mod generator;

use std::env;
use std::error::Error;

use self::bundles::Bundles;
use self::generator::Generator;
use super::{NodeAlone, node_text};

/// The seed of the random runs unless `NEO_SNAPSHOT_DIFFERENTIAL_SEED` names another.
const SEED: u64 = 0x5eed_7a11_0000_0002;

/// How many random nodes the gate's run draws.
const GATE_NODES: usize = 3_000;

#[test]
fn tests_that_each_generated_node_prints_upstreams_own_lines() -> Result<(), Box<dyn Error>> {
    let seed = seed()?;
    let bundles = Bundles::new()?;
    let mut generator = Generator::new(seed);
    for index in 0..count("NEO_SNAPSHOT_DIFFERENTIAL_NODES", GATE_NODES)? {
        let node = generator.node();
        let mut rust = String::new();
        node_text(&NodeAlone::from(&node), &mut rust);
        assert_eq!(
            rust,
            bundles.vendored_node_text(&node)?,
            "seed {seed:#x}, node {index} prints upstream's own text for it: {}",
            serde_json::to_string(&node)?
        );
    }
    Ok(())
}

fn seed() -> Result<u64, Box<dyn Error>> {
    let Ok(text) = env::var("NEO_SNAPSHOT_DIFFERENTIAL_SEED") else {
        return Ok(SEED);
    };
    Ok(match text.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16)?,
        None => text.parse()?,
    })
}

fn count(variable: &str, gate: usize) -> Result<usize, Box<dyn Error>> {
    match env::var(variable) {
        Ok(text) => Ok(text.parse()?),
        Err(_) => Ok(gate),
    }
}
