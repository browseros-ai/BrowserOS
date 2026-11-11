//! What a run can listen for, parsed from Playwright's event names once, at the op boundary.

use browseros_core::{TabId, TargetId};

use crate::events::page_event::PageEvent;
use crate::listed_enum::listed_enum;

listed_enum! {
    /// A page event a script listens for (`page.on(name)`) or waits on
    /// (`page.waitForEvent(name)`).
    pub(crate) enum PageEventKind {
        Popup,
        Close,
        Dialog,
        Console,
        PageError,
        Download,
    }
}

impl PageEventKind {
    /// Playwright's name for the event.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Popup => "popup",
            Self::Close => "close",
            Self::Dialog => "dialog",
            Self::Console => "console",
            Self::PageError => "pageerror",
            Self::Download => "download",
        }
    }

    /// The kind Playwright calls `name`. A page event Neo does not report gets a hint, so the
    /// script learns what to use instead of waiting for nothing.
    pub(crate) fn parse(name: &str) -> Result<Self, Unknown> {
        if let Some(kind) = Self::ALL.iter().copied().find(|kind| kind.name() == name) {
            return Ok(kind);
        }
        let hint = match name {
            "load" | "domcontentloaded" => Some(format!(
                "page.on('{name}') is not available in Neo; use page.waitForLoadState('{name}')"
            )),
            "framenavigated" | "frameattached" | "framedetached" => Some(format!(
                "page.on('{name}') is not available in Neo; read page.frames() or use \
                 page.waitForURL()"
            )),
            "request" | "requestfailed" | "requestfinished" | "response" | "websocket" => Some(
                format!("page.on('{name}') is not available in Neo: network events are not in v1"),
            ),
            "crash" | "filechooser" | "worker" => {
                Some(format!("page.on('{name}') is not available in Neo"))
            }
            _ => None,
        };
        Err(hint.map_or(Unknown::NotAnEvent, Unknown::Unsupported))
    }
}

/// Why an event name is not one Neo reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Unknown {
    /// A Playwright event Neo does not have, with what to use instead.
    Unsupported(String),
    /// Not a Playwright page event at all.
    NotAnEvent,
}

/// One thing a run listens for or waits on. Page events are keyed by the page's target id,
/// which is what the watcher's raw records carry; the tab id rides along for the output.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Subscription {
    /// A page event on one page.
    Page {
        /// The tab the listener's page handle named, for the handles its events carry.
        tab: TabId,
        target: TargetId,
        event: PageEventKind,
    },
    /// `context.on('page')`: every new tab of the context.
    ContextPage,
}

/// What `page.off` and `context.off` name: a subscription without its target, which may have
/// changed since the script called `on`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ListenerKey {
    /// A page event on one page.
    Page(TabId, PageEventKind),
    /// The context's `page` event.
    ContextPage,
}

impl Subscription {
    /// The listener this subscription is, without its target.
    pub(crate) fn key(&self) -> ListenerKey {
        match self {
            Self::Page { tab, event, .. } => ListenerKey::Page(*tab, *event),
            Self::ContextPage => ListenerKey::ContextPage,
        }
    }

    /// Whether `event` is one this subscription receives.
    pub(crate) fn matches(&self, event: &PageEvent) -> bool {
        match (self, event) {
            (Self::ContextPage, PageEvent::Opened(_)) => true,
            (Self::ContextPage, _) => false,
            (Self::Page { target, event, .. }, recorded) => recorded.concerns(*event, target),
        }
    }

    /// The page event kind, for a page subscription.
    pub(crate) fn page_event(&self) -> Option<(&TargetId, PageEventKind)> {
        match self {
            Self::Page { target, event, .. } => Some((target, *event)),
            Self::ContextPage => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PageEventKind, Unknown};

    #[test]
    fn tests_that_every_kind_parses_from_its_playwright_name() {
        for kind in PageEventKind::ALL {
            assert_eq!(PageEventKind::parse(kind.name()), Ok(*kind));
        }
    }

    #[test]
    fn tests_that_a_playwright_event_neo_lacks_says_what_to_use() {
        let Err(Unknown::Unsupported(hint)) = PageEventKind::parse("load") else {
            panic!("load is a Playwright event Neo lacks");
        };
        assert!(hint.contains("page.waitForLoadState('load')"), "{hint}");
        assert_eq!(PageEventKind::parse("clicked"), Err(Unknown::NotAnEvent));
    }
}
