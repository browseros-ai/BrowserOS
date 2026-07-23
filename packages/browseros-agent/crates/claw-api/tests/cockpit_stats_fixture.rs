use claw_api::CockpitStats;

const FIXTURE: &str = include_str!("../../../contracts/claw-api/fixtures/cockpit-stats.json");

#[test]
fn cockpit_stats_fixture_preserves_signed_values() -> Result<(), serde_json::Error> {
    let stats: CockpitStats = serde_json::from_str(FIXTURE)?;

    assert!(stats.has_measured_stats);
    assert_eq!(stats.all_time.raw_token_savings_estimate, -10_000);
    assert_eq!(stats.last30_days.session_count, 5);
    assert_eq!(stats.last7_days.browser_claw_token_estimate, 0);
    assert_eq!(stats.last7_days.tool_call_count, 0);

    let encoded = serde_json::to_string(&stats)?;
    let round_tripped: CockpitStats = serde_json::from_str(&encoded)?;
    assert_eq!(round_tripped, stats);
    Ok(())
}
