use crate::ids::ConvoId;
use tokio::sync::Mutex;

const ADJECTIVES: [&str; 46] = [
    "agile", "amber", "bold", "breezy", "bright", "brisk", "calm", "clever", "cozy", "curious",
    "dapper", "eager", "fancy", "fleet", "fuzzy", "gentle", "glad", "golden", "happy", "jolly",
    "keen", "kind", "lively", "lucky", "merry", "mighty", "nimble", "peppy", "plucky", "proud",
    "quick", "quiet", "radiant", "rapid", "ready", "silky", "snappy", "spry", "steady", "sunny",
    "swift", "tidy", "vivid", "warm", "witty", "zesty",
];
const ANIMALS: [&str; 46] = [
    "alpaca", "badger", "beaver", "bison", "bobcat", "capybara", "caribou", "cheetah", "corgi",
    "dolphin", "falcon", "ferret", "finch", "fox", "gecko", "heron", "ibis", "jaguar", "koala",
    "lemur", "leopard", "lynx", "marten", "moose", "narwhal", "ocelot", "orca", "otter", "owl",
    "panda", "parrot", "penguin", "puffin", "quokka", "rabbit", "raven", "seal", "sparrow",
    "stoat", "tamarin", "tiger", "toucan", "turtle", "walrus", "weasel", "wombat",
];
const REDRAW_ATTEMPTS: usize = 5;
const MAX_SUFFIX: usize = 999;
const RENAME_NUDGE_LIMIT: u8 = 5;

#[derive(Debug, Clone, Copy, Eq, PartialEq, thiserror::Error)]
#[error("unable to mint a unique session name")]
pub struct GenerateFunNameError;

/// Draws an available docker-style session name, suffixing after repeated collisions.
pub fn generate_fun_name(
    mut random: impl FnMut() -> f64,
    mut is_available: impl FnMut(&str) -> bool,
) -> Result<String, GenerateFunNameError> {
    let mut candidate = String::new();
    for _ in 0..REDRAW_ATTEMPTS {
        candidate = format!(
            "{}-{}",
            pick(&ADJECTIVES, random()),
            pick(&ANIMALS, random())
        );
        if is_available(&candidate) {
            return Ok(candidate);
        }
    }

    for suffix in 2..=MAX_SUFFIX {
        let suffixed = format!("{candidate}-{suffix}");
        if is_available(&suffixed) {
            return Ok(suffixed);
        }
    }
    Err(GenerateFunNameError)
}

fn pick<'a>(words: &'a [&str], draw: f64) -> &'a str {
    let clamped = draw.clamp(0.0, 1.0 - f64::EPSILON);
    let index = (clamped * words.len() as f64).floor() as usize;
    words.get(index).copied().unwrap_or(words[0])
}

/// Per-conversation ownership and naming identity.
/// The conversation id stays fixed when `rename` changes only the
/// operator-facing label.
#[derive(Debug)]
pub struct ConversationIdentity {
    convo_id: ConvoId,
    generated_label: String,
    naming: Mutex<SessionNamingState>,
}

#[derive(Debug)]
struct SessionNamingState {
    label: String,
    rename_nudges_left: u8,
}

impl ConversationIdentity {
    /// Creates the per-conversation identity used by MCP, ownership, and naming flows.
    #[must_use]
    pub fn new(client_slug: &str, generated_label: String) -> Self {
        let opaque_id = format!("{client_slug}-{generated_label}");
        Self {
            convo_id: ConvoId::new(opaque_id),
            generated_label: generated_label.clone(),
            naming: Mutex::new(SessionNamingState {
                label: generated_label,
                rename_nudges_left: RENAME_NUDGE_LIMIT,
            }),
        }
    }

    #[must_use]
    pub fn convo_id(&self) -> &ConvoId {
        &self.convo_id
    }

    #[must_use]
    pub fn generated_label(&self) -> &str {
        &self.generated_label
    }

    pub async fn label(&self) -> String {
        self.naming.lock().await.label.clone()
    }

    pub async fn rename(&self, new_label: String) -> String {
        let mut naming = self.naming.lock().await;
        std::mem::replace(&mut naming.label, new_label)
    }

    /// Atomically consumes one rename reminder while the generated label remains active.
    pub async fn take_rename_nudge(&self) -> Option<String> {
        let mut naming = self.naming.lock().await;
        if naming.label != self.generated_label || naming.rename_nudges_left == 0 {
            return None;
        }
        naming.rename_nudges_left -= 1;
        Some(naming.label.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::{ConversationIdentity, GenerateFunNameError, generate_fun_name};
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    fn draws(values: impl IntoIterator<Item = f64>) -> impl FnMut() -> f64 {
        let values = Mutex::new(VecDeque::from_iter(values));
        move || {
            values
                .lock()
                .ok()
                .and_then(|mut values| values.pop_front())
                .unwrap_or(0.0)
        }
    }

    #[test]
    fn selects_from_the_exact_vectors_and_clamps_random_draws() {
        assert_eq!(
            generate_fun_name(draws([0.0, 0.0]), |_| true).as_deref(),
            Ok("agile-alpaca")
        );
        assert_eq!(
            generate_fun_name(draws([1.5, -1.0]), |_| true).as_deref(),
            Ok("zesty-alpaca")
        );
    }

    #[test]
    fn redraws_collisions_before_suffixing_the_last_candidate() {
        let mut checked = Vec::new();
        let name = generate_fun_name(draws([0.0; 10]), |candidate| {
            checked.push(candidate.to_string());
            candidate == "agile-alpaca-3"
        });

        assert_eq!(name.as_deref(), Ok("agile-alpaca-3"));
        assert_eq!(checked.len(), 7);
        assert_eq!(checked[5], "agile-alpaca-2");
    }

    #[test]
    fn reports_exhaustion_after_suffix_999() {
        assert_eq!(
            generate_fun_name(draws([0.0; 10]), |_| false),
            Err(GenerateFunNameError)
        );
    }

    #[tokio::test]
    async fn parallel_nudge_attempts_succeed_exactly_five_times() -> anyhow::Result<()> {
        let identity = Arc::new(ConversationIdentity::new(
            "codex",
            "agile-alpaca".to_string(),
        ));
        let mut tasks = Vec::new();
        for _ in 0..20 {
            let identity = identity.clone();
            tasks.push(tokio::spawn(async move {
                identity.take_rename_nudge().await.is_some()
            }));
        }

        let mut successes = 0;
        for task in tasks {
            if task.await? {
                successes += 1;
            }
        }
        assert_eq!(successes, 5);
        Ok(())
    }

    #[tokio::test]
    async fn rename_returns_the_old_label_and_stops_nudges() {
        let identity = ConversationIdentity::new("codex", "agile-alpaca".to_string());
        assert_eq!(identity.convo_id().as_str(), "codex-agile-alpaca");
        assert_eq!(identity.generated_label(), "agile-alpaca");
        assert_eq!(
            identity.rename("invoice-processing".to_string()).await,
            "agile-alpaca"
        );
        assert_eq!(identity.label().await, "invoice-processing");
        assert_eq!(identity.take_rename_nudge().await, None);
    }
}
