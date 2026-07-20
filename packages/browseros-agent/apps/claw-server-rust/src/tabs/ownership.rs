use super::colors::TabGroupColor;
use crate::ids::ConvoId;
use browseros_core::PageId;
use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Weak},
};
use tokio::sync::{Mutex, RwLock};

pub(crate) type GroupOperationLock = Mutex<()>;

/// Conversation-keyed BrowserOS page ownership and browser tab-group state.
/// Both ownership indexes share this lock so page reassignment cannot expose
/// mismatched views.
#[derive(Debug, Default)]
pub struct PageOwnership {
    inner: RwLock<Inner>,
    group_operation_locks: Mutex<HashMap<ConvoId, Weak<GroupOperationLock>>>,
}

#[derive(Debug, Default)]
struct Inner {
    page_owners: HashMap<PageId, ConvoId>,
    convos: HashMap<ConvoId, ConvoTabs>,
}

#[derive(Debug, Default)]
struct ConvoTabs {
    pages: BTreeSet<PageId>,
    group: Option<TabGroup>,
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct TabGroup {
    pub group_ref: Option<String>,
    pub color: Option<TabGroupColor>,
    pub collapsed: bool,
    pub title: TitleSync,
}

/// Desired browser group-title state. `Pending` survives disconnects until the
/// matching group and title are confirmed, so stale work cannot acknowledge a newer rename.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub enum TitleSync {
    #[default]
    None,
    Synced(String),
    Pending(String),
}

impl TitleSync {
    fn desired_title(&self) -> Option<&str> {
        match self {
            Self::None => None,
            Self::Synced(title) | Self::Pending(title) => Some(title),
        }
    }

    fn mark_synced(&mut self) {
        if let Self::Pending(title) = self {
            *self = Self::Synced(std::mem::take(title));
        }
    }
}

impl TabGroup {
    fn is_empty(&self) -> bool {
        self.group_ref.is_none()
            && self.color.is_none()
            && !self.collapsed
            && self.title == TitleSync::None
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct TabGroupState {
    pub group_ref: Option<String>,
    pub color: Option<TabGroupColor>,
    pub collapsed: bool,
    pub desired_title: Option<String>,
    pub title_sync_pending: bool,
}

impl PageOwnership {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /** Serializes check, create, and remember so each conversation gets one durable tab group. */
    pub(crate) async fn group_operation_lock(&self, convo_id: &ConvoId) -> Arc<GroupOperationLock> {
        let mut locks = self.group_operation_locks.lock().await;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(convo_id).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(convo_id.clone(), Arc::downgrade(&lock));
        lock
    }

    pub async fn owner_of_page(&self, page_id: &PageId) -> Option<ConvoId> {
        self.inner.read().await.page_owners.get(page_id).cloned()
    }

    pub async fn claim_page(&self, convo_id: ConvoId, page_id: PageId) {
        let mut inner = self.inner.write().await;
        if let Some(previous) = inner.page_owners.insert(page_id.clone(), convo_id.clone())
            && previous != convo_id
            && let Some(previous_convo) = inner.convos.get_mut(&previous)
        {
            previous_convo.pages.remove(&page_id);
        }
        inner
            .convos
            .entry(convo_id)
            .or_default()
            .pages
            .insert(page_id);
        inner.remove_empty_convos();
    }

    pub async fn remove_page(&self, page_id: &PageId) -> Option<ConvoId> {
        let mut inner = self.inner.write().await;
        let owner = inner.page_owners.remove(page_id)?;
        if let Some(convo) = inner.convos.get_mut(&owner) {
            convo.pages.remove(page_id);
        }
        inner.remove_empty_convos();
        Some(owner)
    }

    pub async fn prune_missing_pages(&self, live_pages: &BTreeSet<PageId>) -> Vec<PageId> {
        let mut inner = self.inner.write().await;
        let stale = inner
            .page_owners
            .keys()
            .filter(|page_id| !live_pages.contains(*page_id))
            .cloned()
            .collect::<Vec<_>>();
        for page_id in &stale {
            if let Some(owner) = inner.page_owners.remove(page_id)
                && let Some(convo) = inner.convos.get_mut(&owner)
            {
                convo.pages.remove(page_id);
            }
        }
        inner.remove_empty_convos();
        stale
    }

    pub async fn owned_pages(&self, convo_id: &ConvoId) -> BTreeSet<PageId> {
        self.inner
            .read()
            .await
            .convos
            .get(convo_id)
            .map(|convo| convo.pages.clone())
            .unwrap_or_default()
    }

    pub async fn group(&self, convo_id: &ConvoId) -> Option<TabGroup> {
        self.inner
            .read()
            .await
            .convos
            .get(convo_id)
            .and_then(|convo| convo.group.clone())
    }

    pub async fn update_group(&self, convo_id: &ConvoId, update: impl FnOnce(&mut TabGroup)) {
        let mut inner = self.inner.write().await;
        let convo = inner.convos.entry(convo_id.clone()).or_default();
        update(convo.group.get_or_insert_default());
        if convo.group.as_ref().is_some_and(TabGroup::is_empty) {
            convo.group = None;
        }
        inner.remove_empty_convos();
    }

    pub async fn tab_group_ref(&self, convo_id: &ConvoId) -> Option<String> {
        self.group(convo_id).await.and_then(|group| group.group_ref)
    }

    pub async fn set_tab_group_ref(&self, convo_id: ConvoId, value: Option<String>) {
        self.update_group(&convo_id, |group| {
            group.group_ref = value;
            if group.group_ref.is_none() {
                group.collapsed = false;
                group.title.mark_synced();
            }
        })
        .await;
    }

    /// Clears stale browser state only when it still names the completed operation's group.
    pub async fn clear_tab_group_ref_if_current(
        &self,
        convo_id: &ConvoId,
        group_ref: &str,
    ) -> bool {
        let mut inner = self.inner.write().await;
        let Some(group) = inner
            .convos
            .get_mut(convo_id)
            .and_then(|convo| convo.group.as_mut())
        else {
            return false;
        };
        if group.group_ref.as_deref() != Some(group_ref) {
            return false;
        }
        group.group_ref = None;
        group.collapsed = false;
        group.title.mark_synced();
        if group.is_empty()
            && let Some(convo) = inner.convos.get_mut(convo_id)
        {
            convo.group = None;
        }
        inner.remove_empty_convos();
        true
    }

    pub async fn tab_group_color(&self, convo_id: &ConvoId) -> Option<TabGroupColor> {
        self.group(convo_id).await.and_then(|group| group.color)
    }

    pub async fn set_tab_group_color(&self, convo_id: ConvoId, value: Option<TabGroupColor>) {
        self.update_group(&convo_id, |group| group.color = value)
            .await;
    }

    pub async fn tab_group_collapsed(&self, convo_id: &ConvoId) -> bool {
        self.group(convo_id)
            .await
            .is_some_and(|group| group.collapsed)
    }

    pub async fn set_tab_group_collapsed(&self, convo_id: ConvoId, collapsed: bool) {
        self.update_group(&convo_id, |group| {
            group.collapsed = collapsed && group.group_ref.is_some();
        })
        .await;
    }

    pub async fn set_tab_group_collapsed_if_current(
        &self,
        convo_id: &ConvoId,
        group_ref: &str,
        collapsed: bool,
    ) {
        let mut inner = self.inner.write().await;
        let Some(group) = inner
            .convos
            .get_mut(convo_id)
            .and_then(|convo| convo.group.as_mut())
        else {
            return;
        };
        if group.group_ref.as_deref() == Some(group_ref) {
            group.collapsed = collapsed;
        }
    }

    pub async fn set_tab_group(
        &self,
        convo_id: ConvoId,
        group_ref: Option<String>,
        color: Option<TabGroupColor>,
    ) {
        self.update_group(&convo_id, |group| {
            group.group_ref = group_ref;
            group.color = color;
            group.collapsed = false;
            if group.group_ref.is_none() {
                group.title.mark_synced();
            }
        })
        .await;
    }

    /// Installs a newly created group with the title already applied by Chromium.
    pub async fn set_tab_group_with_title(
        &self,
        convo_id: ConvoId,
        group_ref: String,
        color: TabGroupColor,
        title: String,
    ) {
        self.update_group(&convo_id, |group| {
            *group = TabGroup {
                group_ref: Some(group_ref),
                color: Some(color),
                collapsed: false,
                title: TitleSync::Synced(title),
            };
        })
        .await;
    }

    pub async fn tab_group_state(&self, convo_id: &ConvoId) -> Option<TabGroupState> {
        self.inner.read().await.convos.get(convo_id).map(|convo| {
            let group = convo.group.as_ref();
            TabGroupState {
                group_ref: group.and_then(|group| group.group_ref.clone()),
                color: group.and_then(|group| group.color),
                collapsed: group.is_some_and(|group| group.collapsed),
                desired_title: group
                    .and_then(|group| group.title.desired_title())
                    .map(str::to_string),
                title_sync_pending: group
                    .is_some_and(|group| matches!(&group.title, TitleSync::Pending(_))),
            }
        })
    }

    /// Records the authoritative title before any best-effort browser update.
    pub async fn set_desired_group_title(&self, convo_id: ConvoId, title: String) {
        self.update_group(&convo_id, |group| {
            group.title = if group.group_ref.is_some() {
                TitleSync::Pending(title)
            } else {
                TitleSync::Synced(title)
            };
        })
        .await;
    }

    pub async fn pending_group_title(&self, convo_id: &ConvoId) -> Option<(String, String)> {
        let group = self.group(convo_id).await?;
        let TitleSync::Pending(title) = group.title else {
            return None;
        };
        Some((group.group_ref?, title))
    }

    pub async fn mark_group_title_synced(&self, convo_id: &ConvoId, group_ref: &str, title: &str) {
        let mut inner = self.inner.write().await;
        let Some(group) = inner
            .convos
            .get_mut(convo_id)
            .and_then(|convo| convo.group.as_mut())
        else {
            return;
        };
        if group.group_ref.as_deref() == Some(group_ref)
            && matches!(&group.title, TitleSync::Pending(pending) if pending == title)
        {
            group.title = TitleSync::Synced(title.to_string());
        }
    }

    /// Removes all durable ownership and tab-group state for a conversation.
    pub async fn forget(&self, convo_id: &ConvoId) {
        let mut inner = self.inner.write().await;
        if let Some(convo) = inner.convos.remove(convo_id) {
            for page_id in convo.pages {
                if inner.page_owners.get(&page_id) == Some(convo_id) {
                    inner.page_owners.remove(&page_id);
                }
            }
        }
    }
}

impl Inner {
    fn remove_empty_convos(&mut self) {
        self.convos
            .retain(|_, convo| !convo.pages.is_empty() || convo.group.is_some());
    }
}

#[cfg(test)]
mod tests {
    use super::PageOwnership;
    use crate::{ids::ConvoId, tabs::TabGroupColor};
    use browseros_core::PageId;
    use std::collections::BTreeSet;

    #[tokio::test]
    async fn group_operation_locks_are_reused_within_one_ownership_instance() {
        let ownership = PageOwnership::new();
        let convo = ConvoId::new("codex");

        let first = ownership.group_operation_lock(&convo).await;
        let second = ownership.group_operation_lock(&convo).await;

        assert!(std::sync::Arc::ptr_eq(&first, &second));
    }

    #[tokio::test]
    async fn group_operation_locks_are_isolated_between_ownership_instances() {
        let first_ownership = PageOwnership::new();
        let second_ownership = PageOwnership::new();
        let convo = ConvoId::new("codex");

        let first = first_ownership.group_operation_lock(&convo).await;
        let second = second_ownership.group_operation_lock(&convo).await;

        assert!(!std::sync::Arc::ptr_eq(&first, &second));
    }

    #[tokio::test]
    async fn dropped_group_operation_locks_are_replaced() {
        let ownership = PageOwnership::new();
        let convo = ConvoId::new("codex");
        let first = ownership.group_operation_lock(&convo).await;
        let dropped = std::sync::Arc::downgrade(&first);
        drop(first);

        let replacement = ownership.group_operation_lock(&convo).await;

        assert!(dropped.upgrade().is_none());
        assert_eq!(std::sync::Arc::strong_count(&replacement), 1);
    }

    #[tokio::test]
    async fn page_ownership_moves_between_convo_ids() {
        let ownership = PageOwnership::new();
        let codex = ConvoId::new("codex");
        let cowork = ConvoId::new("cowork");

        ownership.claim_page(codex.clone(), PageId(1)).await;
        ownership.claim_page(cowork.clone(), PageId(1)).await;

        assert_eq!(ownership.owner_of_page(&PageId(1)).await, Some(cowork));
        assert!(ownership.owned_pages(&codex).await.is_empty());
    }

    #[tokio::test]
    async fn prune_missing_pages_removes_stale_page_owners() {
        let ownership = PageOwnership::new();
        let codex = ConvoId::new("codex");
        ownership.claim_page(codex.clone(), PageId(1)).await;
        ownership.claim_page(codex.clone(), PageId(2)).await;

        let stale = ownership
            .prune_missing_pages(&BTreeSet::from([PageId(2)]))
            .await;

        assert_eq!(stale, vec![PageId(1)]);
        assert_eq!(
            ownership.owned_pages(&codex).await,
            BTreeSet::from([PageId(2)])
        );
    }

    #[tokio::test]
    async fn tab_group_state_survives_empty_page_set() {
        let ownership = PageOwnership::new();
        let codex = ConvoId::new("codex");
        ownership.claim_page(codex.clone(), PageId(1)).await;
        ownership
            .set_tab_group(
                codex.clone(),
                Some("group-1".to_string()),
                Some(TabGroupColor::Purple),
            )
            .await;

        ownership.remove_page(&PageId(1)).await;

        assert_eq!(
            ownership.tab_group_ref(&codex).await.as_deref(),
            Some("group-1")
        );
        assert_eq!(
            ownership.tab_group_color(&codex).await,
            Some(TabGroupColor::Purple)
        );
        assert!(!ownership.tab_group_collapsed(&codex).await);
    }

    #[tokio::test]
    async fn forget_drops_pages_and_group_state() {
        let ownership = PageOwnership::new();
        let codex = ConvoId::new("codex");
        ownership.claim_page(codex.clone(), PageId(1)).await;
        ownership
            .set_tab_group(
                codex.clone(),
                Some("group-1".to_string()),
                Some(TabGroupColor::Purple),
            )
            .await;

        ownership.forget(&codex).await;

        assert_eq!(ownership.owner_of_page(&PageId(1)).await, None);
        assert!(ownership.owned_pages(&codex).await.is_empty());
        assert_eq!(ownership.tab_group_ref(&codex).await, None);
    }

    #[tokio::test]
    async fn collapsed_state_requires_a_live_group_ref() {
        let ownership = PageOwnership::new();
        let codex = ConvoId::new("codex");
        ownership
            .set_tab_group_ref(codex.clone(), Some("group-1".to_string()))
            .await;
        ownership.set_tab_group_collapsed(codex.clone(), true).await;
        assert!(ownership.tab_group_collapsed(&codex).await);

        ownership.set_tab_group_ref(codex.clone(), None).await;

        assert!(!ownership.tab_group_collapsed(&codex).await);
    }

    #[tokio::test]
    async fn matching_group_clear_preserves_replacement_identity() {
        let ownership = PageOwnership::new();
        let codex = ConvoId::new("codex");
        ownership
            .set_tab_group_with_title(
                codex.clone(),
                "group-1".to_string(),
                TabGroupColor::Purple,
                "codex/first".to_string(),
            )
            .await;
        ownership.set_tab_group_collapsed(codex.clone(), true).await;
        ownership
            .set_desired_group_title(codex.clone(), "codex/renamed".to_string())
            .await;

        assert!(
            ownership
                .clear_tab_group_ref_if_current(&codex, "group-1")
                .await
        );
        let cleared = ownership
            .tab_group_state(&codex)
            .await
            .unwrap_or_else(|| unreachable!());
        assert_eq!(cleared.group_ref, None);
        assert!(!cleared.collapsed);
        assert_eq!(cleared.desired_title.as_deref(), Some("codex/renamed"));
        assert!(!cleared.title_sync_pending);

        ownership
            .set_tab_group_with_title(
                codex.clone(),
                "group-2".to_string(),
                TabGroupColor::Blue,
                "codex/replacement".to_string(),
            )
            .await;

        assert!(
            !ownership
                .clear_tab_group_ref_if_current(&codex, "group-1")
                .await
        );
        let replacement = ownership
            .tab_group_state(&codex)
            .await
            .unwrap_or_else(|| unreachable!());
        assert_eq!(replacement.group_ref.as_deref(), Some("group-2"));
        assert_eq!(replacement.color, Some(TabGroupColor::Blue));
        assert_eq!(
            replacement.desired_title.as_deref(),
            Some("codex/replacement")
        );
    }

    #[tokio::test]
    async fn desired_title_stays_pending_until_matching_group_sync() {
        let ownership = PageOwnership::new();
        let codex = ConvoId::new("codex");
        ownership
            .set_desired_group_title(codex.clone(), "codex/first".to_string())
            .await;
        assert_eq!(ownership.pending_group_title(&codex).await, None);
        ownership
            .set_tab_group_with_title(
                codex.clone(),
                "group-1".to_string(),
                TabGroupColor::Purple,
                "codex/first".to_string(),
            )
            .await;
        ownership
            .set_desired_group_title(codex.clone(), "codex/second".to_string())
            .await;
        assert_eq!(
            ownership.pending_group_title(&codex).await,
            Some(("group-1".to_string(), "codex/second".to_string()))
        );

        ownership
            .mark_group_title_synced(&codex, "group-1", "codex/first")
            .await;
        assert!(
            ownership
                .tab_group_state(&codex)
                .await
                .is_some_and(|state| state.title_sync_pending)
        );
        ownership
            .mark_group_title_synced(&codex, "group-1", "codex/second")
            .await;
        assert!(
            ownership
                .tab_group_state(&codex)
                .await
                .is_some_and(|state| !state.title_sync_pending)
        );
    }
}
