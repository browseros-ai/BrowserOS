//! Links locally proven installation UUIDs to BrowserClaw's canonical analytics
//! UUID. This is the only event allowed to request person processing; it carries
//! no profile properties. Delivery belongs to the consent-gated client lifetime.

use super::installation::valid_identity;
use posthog_rs::{Client, Event};
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tokio::task::JoinHandle;

pub(crate) const EVENT_NAME: &str = "$create_alias";

/// Owned by the active analytics client; dropping or stopping it cancels all
/// direct delivery and retries so they cannot survive shutdown or consent loss.
pub(crate) struct AliasDelivery(JoinHandle<()>);

impl AliasDelivery {
    pub(crate) fn start(
        client: Arc<Client>,
        canonical_id: &str,
        aliases: &[String],
    ) -> Option<Arc<Self>> {
        let events: Vec<_> = aliases
            .iter()
            .filter_map(|alias| alias_event(canonical_id, alias))
            .collect();
        if events.is_empty() {
            return None;
        }
        Some(Arc::new(Self(tokio::spawn(deliver(client, events)))))
    }
    pub(crate) fn stop(&self) {
        self.0.abort();
    }
}

impl Drop for AliasDelivery {
    fn drop(&mut self) {
        self.stop();
    }
}

fn alias_event(canonical_id: &str, alias: &str) -> Option<Event> {
    if !valid_identity(canonical_id) || !valid_identity(alias) || canonical_id == alias {
        return None;
    }
    let mut event = Event::new(EVENT_NAME, canonical_id);
    for (key, value) in [
        ("alias", Value::String(alias.to_owned())),
        ("$process_person_profile", Value::Bool(true)),
        ("$is_server", Value::Bool(true)),
        ("$geoip_disable", Value::Bool(true)),
    ] {
        event.insert_prop(key, value).ok()?;
    }
    Some(event)
}

pub(crate) fn allowlist(mut event: Event) -> Option<Event> {
    let alias = event.properties().get("alias")?.as_str()?;
    if event.event_name() != EVENT_NAME
        || !valid_identity(event.distinct_id())
        || !valid_identity(alias)
        || alias == event.distinct_id()
        || event.properties().get("$process_person_profile") != Some(&Value::Bool(true))
        || event.properties().get("$is_server") != Some(&Value::Bool(true))
        || event.properties().get("$geoip_disable") != Some(&Value::Bool(true))
    {
        return None;
    }
    let remove: Vec<_> = event
        .properties()
        .keys()
        .filter(|key| {
            !matches!(
                key.as_str(),
                "alias" | "$process_person_profile" | "$is_server" | "$geoip_disable"
            )
        })
        .cloned()
        .collect();
    for key in remove {
        event.remove_prop(&key);
    }
    Some(event)
}

async fn deliver(client: Arc<Client>, mut pending: Vec<Event>) {
    let mut delay = Duration::from_secs(1);
    while !pending.is_empty() {
        let mut failed = Vec::new();
        for event in pending {
            // Queue insertion/flush does not acknowledge delivery. Keep the same
            // UUID across retries and bound attempts independently of the SDK's
            // retry budget. Opt-out aborts this future, including in-flight I/O.
            let delivered = matches!(
                tokio::time::timeout(Duration::from_secs(5), client.capture_immediate(event.clone())).await,
                Ok(Ok(summary)) if summary.submitted() == 1 && summary.all_persisted()
            );
            if !delivered {
                failed.push(event);
            }
        }
        pending = failed;
        if !pending.is_empty() {
            tracing::warn!(
                count = pending.len(),
                "analytics identity links not delivered; will retry while consent remains enabled"
            );
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(Duration::from_secs(60));
        }
    }
    // HTTP acceptance is not proof of a successful PostHog merge. Never mark a
    // pair permanently complete: replay it on future startups, and verify
    // ingestion warnings/resolved person IDs when deploying this migration.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alias_boundary_rejects_invalid_pairs_and_strips_profile_properties() -> anyhow::Result<()> {
        let a = "2e087632-1f4e-4ee7-b8bb-cf8ad53e91a8";
        let b = "31eca9ca-566d-4373-8a1c-2f29b32dbed1";
        assert!(alias_event(a, a).is_none());
        assert!(alias_event(a, "not-an-id").is_none());
        assert!(alias_event("", b).is_none());
        let mut event = alias_event(a, b).ok_or_else(|| anyhow::anyhow!("valid pair rejected"))?;
        event.insert_prop("$set", serde_json::json!({"email": "private@example.com"}))?;
        event.insert_prop("$lib", "sdk")?;
        let event = allowlist(event).ok_or_else(|| anyhow::anyhow!("valid alias rejected"))?;
        assert_eq!(event.properties().len(), 4);
        assert!(!event.properties().contains_key("$set"));
        assert!(!event.properties().contains_key("$lib"));
        let mut personless = event;
        personless.insert_prop("$process_person_profile", false)?;
        assert!(allowlist(personless).is_none());
        Ok(())
    }
}
