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
        FileChooser,
        Load,
        DomContentLoaded,
        FrameAttached,
        FrameNavigated,
        FrameDetached,
        /// A page's call of a binding the run exposed (`exposeBinding`). Not one of
        /// Playwright's events: only the run's own binding ops listen for it.
        Binding,
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
            Self::FileChooser => "filechooser",
            Self::Binding => "binding",
            Self::Load => "load",
            Self::DomContentLoaded => "domcontentloaded",
            Self::FrameAttached => "frameattached",
            Self::FrameNavigated => "framenavigated",
            Self::FrameDetached => "framedetached",
        }
    }

    /// Context mirrors in Playwright 1.64.1; `DOMContentLoaded` is page-only.
    pub(crate) fn context(name: &str) -> Option<Self> {
        match name {
            "pageload" => Some(Self::Load),
            "frameattached" => Some(Self::FrameAttached),
            "framenavigated" => Some(Self::FrameNavigated),
            "framedetached" => Some(Self::FrameDetached),
            _ => None,
        }
    }

    /// The kind Playwright calls `name`. A page event Neo does not report gets a hint, so the
    /// script learns what to use instead of waiting for nothing.
    pub(crate) fn parse(name: &str) -> Result<Self, Unknown> {
        if let Some(kind) = Self::ALL
            .iter()
            .copied()
            .find(|kind| *kind != Self::Binding && kind.name() == name)
        {
            return Ok(kind);
        }
        let hint = match name {
            "request" | "requestfailed" | "requestfinished" | "response" | "websocket" => Some(
                format!("page.on('{name}') is not available in Neo: network events are not in v1"),
            ),
            "crash" | "worker" => Some(format!("page.on('{name}') is not available in Neo")),
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
    /// A lifecycle event forwarded from every page in the context.
    Context(PageEventKind),
}

/// What `page.off` and `context.off` name: a subscription without its target, which may have
/// changed since the script called `on`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ListenerKey {
    /// A page event on one page.
    Page(TabId, PageEventKind),
    /// The context's `page` event.
    ContextPage,
    /// A lifecycle event forwarded from every page in the context.
    Context(PageEventKind),
}

impl Subscription {
    /// The listener this subscription is, without its target.
    pub(crate) fn key(&self) -> ListenerKey {
        match self {
            Self::Page { tab, event, .. } => ListenerKey::Page(*tab, *event),
            Self::ContextPage => ListenerKey::ContextPage,
            Self::Context(event) => ListenerKey::Context(*event),
        }
    }

    /// Whether `event` is one this subscription receives.
    pub(crate) fn matches(&self, event: &PageEvent) -> bool {
        match (self, event) {
            (Self::ContextPage, PageEvent::Opened(_)) => true,
            (Self::Context(event), PageEvent::FrameLifecycle(frame)) => frame.kind == *event,
            (Self::Context(event), PageEvent::Load { kind, .. }) => kind == event,
            (Self::ContextPage | Self::Context(_), _) => false,
            (Self::Page { target, event, .. }, recorded) => recorded.concerns(*event, target),
        }
    }

    /// The page event kind, for a page subscription.
    pub(crate) fn page_event(&self) -> Option<(&TargetId, PageEventKind)> {
        match self {
            Self::Page { target, event, .. } => Some((target, *event)),
            Self::ContextPage | Self::Context(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PageEventKind, Unknown};

    #[test]
    fn tests_that_every_kind_parses_from_its_playwright_name() {
        for kind in PageEventKind::ALL {
            if *kind == PageEventKind::Binding {
                assert_eq!(
                    PageEventKind::parse(kind.name()),
                    Err(Unknown::NotAnEvent),
                    "binding calls are no event a script listens for"
                );
            } else {
                assert_eq!(PageEventKind::parse(kind.name()), Ok(*kind));
            }
        }
    }

    #[test]
    fn tests_that_a_playwright_event_neo_lacks_says_what_to_use() {
        let Err(Unknown::Unsupported(hint)) = PageEventKind::parse("request") else {
            panic!("request is a Playwright event Neo lacks");
        };
        assert!(hint.contains("network events"), "{hint}");
        assert_eq!(PageEventKind::parse("clicked"), Err(Unknown::NotAnEvent));
    }
}
