use std::error::Error;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::task;

use super::FocusLease;
use crate::test_env::Recording;

/// Lets the guards' spawned releases run.
async fn settle() {
    for _ in 0..20 {
        task::yield_now().await;
    }
}

fn settings(recording: &Recording) -> Vec<Value> {
    recording
        .sent()
        .into_iter()
        .filter(|(method, _)| method == "Emulation.setFocusEmulationEnabled")
        .map(|(_, params)| params["enabled"].clone())
        .collect()
}

#[tokio::test]
async fn tests_that_overlapping_holds_turn_emulation_on_once_and_off_after_the_last()
-> Result<(), Box<dyn Error>> {
    let recording = Arc::new(Recording::default());
    let session = recording.session("tab");
    let lease = Arc::new(FocusLease::default());

    let first = lease.acquire(&session).await?;
    let second = lease.acquire(&session).await?;
    assert_eq!(settings(&recording), [json!(true)], "the first turns it on");
    drop(first);
    settle().await;
    assert_eq!(settings(&recording), [json!(true)], "a holder is left");
    drop(second);
    settle().await;
    assert_eq!(
        settings(&recording),
        [json!(true), json!(false)],
        "the last release restores it"
    );

    let again = lease.acquire(&session).await?;
    drop(again);
    settle().await;
    assert_eq!(
        settings(&recording),
        [json!(true), json!(false), json!(true), json!(false)],
        "a later op leases it afresh"
    );
    Ok(())
}
