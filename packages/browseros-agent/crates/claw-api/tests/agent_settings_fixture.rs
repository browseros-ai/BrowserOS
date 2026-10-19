use claw_api::{AgentSettings, UpdateAgentSettingsRequest};
use serde_json::{Value, json};

const FIXTURE: &str = include_str!("../../../contracts/claw-api/fixtures/agent-settings.json");

#[test]
fn tests_that_settings_fixture_includes_cleanup() -> Result<(), serde_json::Error> {
    let settings: AgentSettings = serde_json::from_str(FIXTURE)?;
    let encoded = serde_json::to_value(&settings)?;
    assert_eq!(encoded["closeTabGroupsAfterTask"], false);
    assert_eq!(serde_json::from_value::<AgentSettings>(encoded)?, settings);
    Ok(())
}

#[test]
fn tests_that_cleanup_is_required_in_settings_responses() {
    assert!(
        serde_json::from_value::<AgentSettings>(json!({
            "humanHelpEnabled": true,
            "jevTokenSource": "none",
        }))
        .is_err()
    );
}

#[test]
fn tests_that_cleanup_updates_accept_both_states() -> Result<(), serde_json::Error> {
    for cleanup in [true, false] {
        let payload = json!({ "humanHelpEnabled": false, "closeTabGroupsAfterTask": cleanup });
        let update: UpdateAgentSettingsRequest = serde_json::from_value(payload.clone())?;
        assert_eq!(serde_json::to_value(update)?, payload);
    }
    Ok(())
}

#[test]
fn tests_that_older_updates_omit_cleanup() -> Result<(), serde_json::Error> {
    let payload = json!({ "humanHelpEnabled": true });
    let update: UpdateAgentSettingsRequest = serde_json::from_value(payload.clone())?;
    assert_eq!(serde_json::to_value(update)?, payload);
    Ok(())
}

#[test]
fn tests_that_update_booleans_are_type_checked() {
    for cleanup in [json!("true"), json!(1), json!([]), json!({})] {
        assert!(
            serde_json::from_value::<UpdateAgentSettingsRequest>(json!({
                "humanHelpEnabled": true,
                "closeTabGroupsAfterTask": cleanup,
            }))
            .is_err()
        );
    }
    for payload in [
        json!({ "closeTabGroupsAfterTask": true }),
        json!({ "humanHelpEnabled": Value::Null }),
    ] {
        assert!(serde_json::from_value::<UpdateAgentSettingsRequest>(payload).is_err());
    }
}
