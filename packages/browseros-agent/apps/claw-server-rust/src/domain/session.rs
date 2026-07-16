use crate::domain::{AgentRef, DispatchId, SessionId};
use browseros_core::PageId;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
    time::Duration,
};
use tokio::{
    sync::{Mutex, RwLock},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

const TAB_GROUP_COLORS: [TabGroupColor; 9] = [
    TabGroupColor::Grey,
    TabGroupColor::Blue,
    TabGroupColor::Red,
    TabGroupColor::Yellow,
    TabGroupColor::Green,
    TabGroupColor::Pink,
    TabGroupColor::Purple,
    TabGroupColor::Cyan,
    TabGroupColor::Orange,
];

/// Chrome tab-group colour names accepted by the BrowserOS tab_groups tool.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TabGroupColor {
    Grey,
    Blue,
    Red,
    Yellow,
    Green,
    Pink,
    Purple,
    Cyan,
    Orange,
}

impl TabGroupColor {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Grey => "grey",
            Self::Blue => "blue",
            Self::Red => "red",
            Self::Yellow => "yellow",
            Self::Green => "green",
            Self::Pink => "pink",
            Self::Purple => "purple",
            Self::Cyan => "cyan",
            Self::Orange => "orange",
        }
    }

    /// Hex twin of the browser tab-group colour (TS TAB_GROUP_HEX) so the
    /// cockpit card border matches the tab strip.
    #[must_use]
    pub fn hex(self) -> &'static str {
        match self {
            Self::Grey => "#6B7280",
            Self::Blue => "#2F6FE0",
            Self::Red => "#DC2626",
            Self::Yellow => "#F59E0B",
            Self::Green => "#10A37F",
            Self::Pink => "#DB2777",
            Self::Purple => "#7A5AF8",
            Self::Cyan => "#0EA5E9",
            Self::Orange => "#F26B2A",
        }
    }
}

/// Hex colour for an agent slug; stable across processes and the TS server.
#[must_use]
pub fn hex_for_slug(slug: &str) -> &'static str {
    color_for_slug(slug).hex()
}

impl fmt::Display for TabGroupColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Selects the deterministic tab-group colour shared with the TS Claw server.
#[must_use]
pub fn color_for_slug(slug: &str) -> TabGroupColor {
    let idx = usize::try_from(fnv1a(slug) % u32::try_from(TAB_GROUP_COLORS.len()).unwrap_or(1))
        .unwrap_or(0);
    TAB_GROUP_COLORS
        .get(idx)
        .copied()
        .unwrap_or(TabGroupColor::Grey)
}

fn fnv1a(input: &str) -> u32 {
    let mut hash = 0x811c9dc5_u32;
    for byte in input.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_add(
            (hash << 1)
                .wrapping_add(hash << 4)
                .wrapping_add(hash << 7)
                .wrapping_add(hash << 8)
                .wrapping_add(hash << 24),
        );
    }
    hash
}

pub struct Session {
    id: SessionId,
    agent: AgentRef,
    first_captures: RwLock<BTreeSet<PageId>>,
    active_dispatches: Mutex<BTreeMap<DispatchId, CancellationToken>>,
    cancel: CancellationToken,
    replay_handle: Mutex<Option<String>>,
    last_activity: Mutex<Instant>,
    /// User-facing session name gathered via MCP elicitation. In-memory only,
    /// mirroring the TS identity map's sessionLabel (never persisted to audit).
    session_label: RwLock<Option<String>>,
}

impl Session {
    #[must_use]
    pub fn new(id: SessionId, agent: AgentRef, now: Instant) -> Arc<Self> {
        Arc::new(Self {
            id,
            agent,
            first_captures: RwLock::new(BTreeSet::new()),
            active_dispatches: Mutex::new(BTreeMap::new()),
            cancel: CancellationToken::new(),
            replay_handle: Mutex::new(None),
            last_activity: Mutex::new(now),
            session_label: RwLock::new(None),
        })
    }

    #[must_use]
    pub fn id(&self) -> &SessionId {
        &self.id
    }

    #[must_use]
    pub fn agent(&self) -> &AgentRef {
        &self.agent
    }

    pub async fn touch(&self, now: Instant) {
        *self.last_activity.lock().await = now;
    }

    pub async fn idle_for(&self, now: Instant) -> Duration {
        now.saturating_duration_since(*self.last_activity.lock().await)
    }

    pub async fn has_first_capture(&self, page_id: &PageId) -> bool {
        self.first_captures.read().await.contains(page_id)
    }

    pub async fn mark_first_capture_done(&self, page_id: PageId) {
        self.first_captures.write().await.insert(page_id);
    }

    pub async fn forget_first_capture(&self, page_id: &PageId) {
        self.first_captures.write().await.remove(page_id);
    }

    pub async fn set_replay_handle(&self, value: Option<String>) {
        *self.replay_handle.lock().await = value;
    }

    pub async fn session_label(&self) -> Option<String> {
        self.session_label.read().await.clone()
    }

    pub async fn set_session_label(&self, label: String) {
        *self.session_label.write().await = Some(label);
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub async fn register_dispatch(&self, dispatch_id: DispatchId, token: CancellationToken) {
        self.active_dispatches
            .lock()
            .await
            .insert(dispatch_id, token);
    }

    pub async fn unregister_dispatch(&self, dispatch_id: &DispatchId) {
        self.active_dispatches.lock().await.remove(dispatch_id);
    }

    pub async fn cancel_active_dispatches(&self) -> usize {
        let tokens = self
            .active_dispatches
            .lock()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for token in &tokens {
            token.cancel();
        }
        tokens.len()
    }

    #[must_use]
    pub fn child_token(&self) -> CancellationToken {
        self.cancel.child_token()
    }
}

#[cfg(test)]
mod tests {
    use super::{TabGroupColor, color_for_slug};

    #[test]
    fn color_for_slug_matches_tab_group_palette() {
        assert_eq!(color_for_slug("codex"), TabGroupColor::Purple);
        assert_eq!(color_for_slug("finance-ops"), TabGroupColor::Grey);
    }

    #[test]
    fn hex_for_slug_matches_ts_tab_group_hex() {
        assert_eq!(super::hex_for_slug("codex"), "#7A5AF8");
        assert_eq!(super::hex_for_slug("finance-ops"), "#6B7280");
    }
}
