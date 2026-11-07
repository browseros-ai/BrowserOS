//! The host side's tab directory: every page target in the browser, keyed by CDP target id, with
//! the BrowserOS tab id, URL and title it has now. The one place the two tab identities meet:
//! scripts, capture and the session ledger speak target ids (`PageId`), hosts see tab ids
//! (`TabId`). It is browser-wide and keeps flowing between runs, because a live view keeps
//! showing a session's tab after its runs end.
//!
//! [`TabTable`] is pure. The host task owns the only one and applies, in the order they happened:
//!
//! ```text
//!  host socket   Browser.getTabs reply (seed, reseed) ───────────▶ Seeded     cdp::seed
//!  (never the    Target.targetCreated / targetInfoChanged ───────▶ Created, Changed
//!   runs')       Target.targetDestroyed ─────────────────────────▶ Destroyed  cdp::directory_input
//!                a new epoch, or its event ring lagged ──────────▶ Lost
//!  run path      HostEvent::PageCreated with the create reply's tab id ─▶ Committed
//! ```
//!
//! Readers: the session ledger (which touched tabs are open, with their tab id and URL), capture
//! (a target that left the directory ends its captures even if its detach was lost), and the
//! after-call worker (the last facts of a tab that closed).
//!
//! The seed is fresh. After (re)enabling discovery the host task asks for `Browser.getTabs`, and
//! events that reach the socket before that reply describe the browser as it was before the seed
//! was taken. Applied after the seed, an older `Target.targetInfoChanged` would overwrite a newer
//! seed, as claw-server's rebuild lets it
//! (`apps/claw-server/src/services/browser/tab_registry.rs:131-133`). The table holds the rule
//! itself, so the host task only applies inputs in socket order, the seed where its reply
//! arrived: until a seed is applied (at first, and after `Lost`), `Created` and `Changed` are
//! dropped; `Destroyed` always applies, and a destroyed target never opens again (Chromium never
//! reuses a target id), so a seed that still lists it cannot bring it back.

use std::collections::HashMap;
use std::mem;

use tokio::time::Instant;

use crate::host::cdp;
use crate::host::ids::TabId;
use crate::run::PageId;

/// What a reader sees of one tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TabEntry {
    pub(crate) tab: TabId,
    /// Empty for a page the run path committed, until discovery reports it.
    pub(crate) url: String,
    pub(crate) title: String,
}

/// A page target as BrowserOS reports it, parsed where it enters (`cdp.rs`): only a `page`
/// target with a positive tab id becomes one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PageTarget {
    pub(crate) target: PageId,
    pub(crate) entry: TabEntry,
}

/// Everything that changes the directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DirectoryInput {
    /// The tab list after (re)enabling discovery on a socket incarnation: every open tab.
    Seeded(Vec<PageTarget>),
    Created(PageTarget),
    /// Its URL or title changed: a navigation committed, a `<title>` was parsed.
    Changed(PageTarget),
    Destroyed(PageId),
    /// A page the run path created, at the creation's commit point (`NewPage::keep`), with the
    /// tab id from the create reply. It maps the target before discovery reports it, so the page
    /// can become a session's current tab at once; discovery's facts win from then on.
    Committed {
        target: PageId,
        tab: TabId,
    },
    /// The host socket was replaced (a new epoch) or its event ring lagged: events may be missing
    /// until the next seed.
    Lost,
}

/// The directory's state; only [`TabTable::apply`] writes it.
#[derive(Debug, Default)]
pub(crate) struct TabTable {
    open: HashMap<PageId, TabEntry>,
    /// Targets that left `open`, kept for [`cdp::TAB_TOMBSTONE`].
    closed: HashMap<PageId, Closed>,
    /// A seed was applied since the start or the last `Lost`.
    seeded: bool,
}

#[derive(Debug)]
struct Closed {
    /// Its facts when it closed; `None` for a target destroyed before the directory knew it.
    last: Option<TabEntry>,
    /// Destroyed, so it never opens again. Otherwise a seed stopped listing it, and a later event
    /// about it reopens it: the seed may predate a page the run path just committed, or leave out
    /// a tab that discovery reports.
    destroyed: bool,
    at: Instant,
}

impl TabTable {
    /// Applies one input at `now`. Returns whether [`Self::open`] or [`Self::ready`] answers
    /// changed, which is what followers wake for.
    pub(crate) fn apply(&mut self, input: DirectoryInput, now: Instant) -> bool {
        self.closed
            .retain(|_, closed| now.saturating_duration_since(closed.at) < cdp::TAB_TOMBSTONE);
        match input {
            DirectoryInput::Seeded(tabs) => self.seed(tabs, now),
            DirectoryInput::Created(page) | DirectoryInput::Changed(page) => {
                self.seeded && self.upsert(page)
            }
            DirectoryInput::Destroyed(target) => self.destroy(target, now),
            DirectoryInput::Committed { target, tab } => self.commit(target, tab),
            DirectoryInput::Lost => mem::replace(&mut self.seeded, false),
        }
    }

    /// The tab `target` shows, if it is open.
    pub(crate) fn open(&self, target: &PageId) -> Option<&TabEntry> {
        self.open.get(target)
    }

    /// The tab `target` shows, or showed when it closed less than [`cdp::TAB_TOMBSTONE`] before
    /// `now`: the page facts of a call whose tab closed before they were read.
    pub(crate) fn recent(&self, target: &PageId, now: Instant) -> Option<&TabEntry> {
        self.open.get(target).or_else(|| {
            self.closed
                .get(target)
                .filter(|closed| now.saturating_duration_since(closed.at) < cdp::TAB_TOMBSTONE)
                .and_then(|closed| closed.last.as_ref())
        })
    }

    /// False before the first seed and between `Lost` and the next one, while the open set may be
    /// stale.
    pub(crate) fn ready(&self) -> bool {
        self.seeded
    }

    fn seed(&mut self, tabs: Vec<PageTarget>, now: Instant) -> bool {
        let mut open = HashMap::with_capacity(tabs.len());
        for PageTarget { target, entry } in tabs {
            let destroyed = self
                .closed
                .get(&target)
                .is_some_and(|closed| closed.destroyed);
            if !destroyed {
                self.closed.remove(&target);
                open.insert(target, entry);
            }
        }
        let changed = !self.seeded || open != self.open;
        for (target, entry) in mem::replace(&mut self.open, open) {
            if !self.open.contains_key(&target) {
                let closed = Closed {
                    last: Some(entry),
                    destroyed: false,
                    at: now,
                };
                self.closed.insert(target, closed);
            }
        }
        self.seeded = true;
        changed
    }

    fn upsert(&mut self, PageTarget { target, entry }: PageTarget) -> bool {
        match self.closed.get(&target).map(|closed| closed.destroyed) {
            Some(true) => return false,
            Some(false) => {
                self.closed.remove(&target);
            }
            None => {}
        }
        if self.open.get(&target) == Some(&entry) {
            return false;
        }
        self.open.insert(target, entry);
        true
    }

    fn destroy(&mut self, target: PageId, now: Instant) -> bool {
        if let Some(entry) = self.open.remove(&target) {
            let closed = Closed {
                last: Some(entry),
                destroyed: true,
                at: now,
            };
            self.closed.insert(target, closed);
            return true;
        }
        self.closed
            .entry(target)
            .and_modify(|closed| closed.destroyed = true)
            .or_insert(Closed {
                last: None,
                destroyed: true,
                at: now,
            });
        false
    }

    fn commit(&mut self, target: PageId, tab: TabId) -> bool {
        if self.open.contains_key(&target) || self.closed.contains_key(&target) {
            return false;
        }
        let entry = TabEntry {
            tab,
            url: String::new(),
            title: String::new(),
        };
        self.open.insert(target, entry);
        true
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn target(id: &str) -> PageId {
        PageId::new(id.to_string())
    }

    fn tab_id(raw: i64) -> TabId {
        TabId::parse(raw).expect("a positive tab id")
    }

    fn tab(id: &str, tab: i64, url: &str) -> PageTarget {
        PageTarget {
            target: target(id),
            entry: TabEntry {
                tab: tab_id(tab),
                url: url.to_string(),
                title: format!("{id}'s title"),
            },
        }
    }

    fn url(table: &TabTable, id: &str) -> Option<String> {
        table.open(&target(id)).map(|entry| entry.url.clone())
    }

    fn seeded(tabs: Vec<PageTarget>, now: Instant) -> TabTable {
        let mut table = TabTable::default();
        table.apply(DirectoryInput::Seeded(tabs), now);
        table
    }

    #[test]
    fn a_seed_opens_its_tabs_and_closes_the_rest() {
        let now = Instant::now();
        let mut table = TabTable::default();
        assert!(!table.ready(), "nothing is known before the first seed");
        let first = vec![
            tab("a", 1, "https://a.test/"),
            tab("b", 2, "https://b.test/"),
        ];
        assert!(
            table.apply(DirectoryInput::Seeded(first), now),
            "the first seed is a change"
        );
        assert!(table.ready(), "seeded");
        assert_eq!(
            table.open(&target("a")),
            Some(&tab("a", 1, "https://a.test/").entry),
            "a seeded tab is open with its facts"
        );

        let second = vec![
            tab("b", 2, "https://b.test/"),
            tab("c", 3, "https://c.test/"),
        ];
        assert!(
            table.apply(DirectoryInput::Seeded(second.clone()), now),
            "a reseed that differs is a change"
        );
        assert_eq!(
            table.open(&target("a")),
            None,
            "a tab the seed no longer lists closes"
        );
        assert_eq!(
            table.recent(&target("a"), now).map(|entry| entry.tab),
            Some(tab_id(1)),
            "a closed tab's facts stay readable"
        );
        assert_eq!(
            url(&table, "c").as_deref(),
            Some("https://c.test/"),
            "a newly listed tab opens"
        );
        assert!(
            !table.apply(DirectoryInput::Seeded(second), now),
            "the same seed again changes nothing"
        );
    }

    #[test]
    fn events_older_than_the_seed_never_overwrite_it() {
        let now = Instant::now();
        let mut table = TabTable::default();
        assert!(
            !table.apply(
                DirectoryInput::Changed(tab("a", 1, "https://old.test/")),
                now
            ),
            "a change before the first seed is dropped"
        );
        assert!(
            !table.apply(DirectoryInput::Created(tab("b", 2, "https://b.test/")), now),
            "a creation before the first seed is dropped"
        );
        table.apply(
            DirectoryInput::Seeded(vec![tab("a", 1, "https://new.test/")]),
            now,
        );
        assert_eq!(
            url(&table, "a").as_deref(),
            Some("https://new.test/"),
            "the seed's URL stands"
        );
        assert_eq!(
            table.open(&target("b")),
            None,
            "a tab the seed does not list stays unknown"
        );

        assert!(
            table.apply(DirectoryInput::Lost, now),
            "losing the socket is a change"
        );
        assert!(!table.ready(), "not ready until the reseed");
        assert!(
            !table.apply(DirectoryInput::Lost, now),
            "losing it again changes nothing"
        );
        assert!(
            !table.apply(
                DirectoryInput::Changed(tab("a", 1, "https://older.test/")),
                now
            ),
            "a change queued before the reseed is dropped"
        );
        assert_eq!(
            url(&table, "a").as_deref(),
            Some("https://new.test/"),
            "readers keep the last known facts meanwhile"
        );
        table.apply(
            DirectoryInput::Seeded(vec![tab("a", 1, "https://newest.test/")]),
            now,
        );
        assert_eq!(
            url(&table, "a").as_deref(),
            Some("https://newest.test/"),
            "the reseed's URL stands"
        );
        assert!(
            table.apply(
                DirectoryInput::Changed(tab("a", 1, "https://after.test/")),
                now
            ),
            "a change after the seed applies"
        );
        assert_eq!(
            url(&table, "a").as_deref(),
            Some("https://after.test/"),
            "and is what readers see"
        );
    }

    #[test]
    fn a_destroy_always_wins() {
        let now = Instant::now();
        let mut table = TabTable::default();
        table.apply(DirectoryInput::Destroyed(target("a")), now);
        table.apply(
            DirectoryInput::Seeded(vec![
                tab("a", 1, "https://a.test/"),
                tab("b", 2, "https://b.test/"),
            ]),
            now,
        );
        assert_eq!(
            table.open(&target("a")),
            None,
            "a destroy that arrived before the seed reply removes the target"
        );
        assert!(
            table.open(&target("b")).is_some(),
            "the rest of the seed opens"
        );

        assert!(
            table.apply(DirectoryInput::Destroyed(target("b")), now),
            "closing an open tab is a change"
        );
        let revivals = [
            DirectoryInput::Created(tab("b", 2, "https://b.test/")),
            DirectoryInput::Changed(tab("b", 2, "https://b.test/")),
            DirectoryInput::Committed {
                target: target("b"),
                tab: tab_id(2),
            },
            DirectoryInput::Seeded(vec![tab("b", 2, "https://b.test/")]),
        ];
        for input in revivals {
            table.apply(input, now);
            assert_eq!(
                table.open(&target("b")),
                None,
                "a destroyed target never opens again"
            );
        }
        assert!(
            !table.apply(DirectoryInput::Destroyed(target("b")), now),
            "destroying it twice changes nothing"
        );
    }

    #[test]
    fn discovery_keeps_url_and_title_current() {
        let now = Instant::now();
        let mut table = seeded(Vec::new(), now);
        assert!(
            table.apply(DirectoryInput::Created(tab("c", 3, "about:blank")), now),
            "a created tab opens"
        );
        assert!(
            table.apply(DirectoryInput::Changed(tab("c", 3, "https://c.test/")), now),
            "a navigation is a change"
        );
        assert!(
            !table.apply(DirectoryInput::Changed(tab("c", 3, "https://c.test/")), now),
            "the same facts again change nothing"
        );
        assert_eq!(
            table.open(&target("c")),
            Some(&tab("c", 3, "https://c.test/").entry),
            "readers see the newest facts"
        );
    }

    #[test]
    fn a_committed_page_is_mapped_before_discovery_reports_it() {
        let now = Instant::now();
        let mut table = seeded(Vec::new(), now);
        assert!(
            table.apply(
                DirectoryInput::Committed {
                    target: target("n"),
                    tab: tab_id(9),
                },
                now
            ),
            "the run path's creation opens the tab"
        );
        assert_eq!(
            table.open(&target("n")).map(|entry| entry.tab),
            Some(tab_id(9)),
            "with the tab id from the create reply"
        );
        assert!(
            table.apply(DirectoryInput::Created(tab("n", 9, "https://n.test/")), now),
            "discovery's facts fill it in"
        );
        assert!(
            !table.apply(
                DirectoryInput::Committed {
                    target: target("n"),
                    tab: tab_id(9),
                },
                now
            ),
            "a late commit changes nothing"
        );
        assert_eq!(
            url(&table, "n").as_deref(),
            Some("https://n.test/"),
            "discovery's facts win"
        );
    }

    #[test]
    fn a_tab_a_seed_missed_reopens_on_its_next_event() {
        let now = Instant::now();
        let mut table = seeded(vec![tab("a", 1, "https://a.test/")], now);
        table.apply(DirectoryInput::Lost, now);
        table.apply(
            DirectoryInput::Committed {
                target: target("n"),
                tab: tab_id(9),
            },
            now,
        );
        assert!(
            table.open(&target("n")).is_some(),
            "the run path's facts apply while the socket is lost"
        );
        table.apply(
            DirectoryInput::Seeded(vec![tab("a", 1, "https://a.test/")]),
            now,
        );
        assert_eq!(
            table.open(&target("n")),
            None,
            "a reseed taken before the creation closes it"
        );
        assert!(
            table.apply(DirectoryInput::Created(tab("n", 9, "https://n.test/")), now),
            "discovery's later event reopens it"
        );
        assert_eq!(
            url(&table, "n").as_deref(),
            Some("https://n.test/"),
            "with discovery's facts"
        );
    }

    #[test]
    fn a_closed_tab_stays_readable_for_the_tombstone_window() {
        let closed_at = Instant::now();
        let mut table = seeded(vec![tab("a", 1, "https://a.test/")], closed_at);
        table.apply(DirectoryInput::Destroyed(target("a")), closed_at);
        assert_eq!(
            table.open(&target("a")),
            None,
            "a destroyed tab is not open"
        );
        let just_inside = closed_at + cdp::TAB_TOMBSTONE - Duration::from_millis(1);
        assert_eq!(
            table.recent(&target("a"), just_inside),
            Some(&tab("a", 1, "https://a.test/").entry),
            "its last facts stay readable inside the window"
        );
        assert_eq!(
            table.recent(&target("a"), closed_at + cdp::TAB_TOMBSTONE),
            None,
            "and are gone once it passed"
        );
    }
}
