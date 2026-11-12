//! What a run can listen for, parsed from Playwright's event names once, at the op boundary.

use browseros_core::{TabId, TargetId};

use crate::events::page_event::PageEvent;
use crate::listed_enum::listed_enum;

listed_enum! {
    /// A page event a script listens for (`page.on(name)`) or waits on
    /// (`page.waitForEvent(name)`).
    pub(crate) enum PageEventKind {
        Popup,
        Worker,
        WorkerClosed,
        WorkerState,
        Close,
        Dialog,
        DialogClosed,
        Console,
        PageError,
        Download,
        FileChooser,
        Load,
        DomContentLoaded,
        FrameAttached,
        FrameNavigated,
        FrameDetached,
        /// The page's renderer crashed (`Target.targetCrashed`); the page stays open.
        Crash,
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
            Self::Worker => "worker",
            Self::WorkerClosed => "workerclosed",
            Self::WorkerState => "workerstate",
            Self::Close => "close",
            Self::Dialog => "dialog",
            Self::DialogClosed => "dialogclosed",
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
            Self::Crash => "crash",
        }
    }

    /// Context mirrors in Playwright 1.64.1; `DOMContentLoaded` is page-only.
    pub(crate) fn context(name: &str) -> Option<Self> {
        match name {
            "pageload" => Some(Self::Load),
            "dialog" => Some(Self::Dialog),
            "dialogclosed" => Some(Self::DialogClosed),
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
    /// A lifecycle or dialog event forwarded from every page in the context.
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
    /// A lifecycle or dialog event forwarded from every page in the context.
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
            (Self::ContextPage, PageEvent::Opened(_))
            | (Self::Context(PageEventKind::Dialog), PageEvent::Dialog(_))
            | (Self::Context(PageEventKind::DialogClosed), PageEvent::DialogClosed(_)) => true,
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

    /// Whether this subscription asks for `event` on the tab whose target is `target`: a page
    /// subscription for that tab, or the context's for every tab. The watcher holds a dialog
    /// open for the runs this says want it, so `context.waitForEvent('dialog')` holds one as
    /// `page.on('dialog')` does.
    pub(crate) fn wants(&self, target: &TargetId, event: PageEventKind) -> bool {
        match self {
            Self::Page {
                target: listened,
                event: listened_for,
                ..
            } => listened == target && *listened_for == event,
            Self::Context(listened_for) => *listened_for == event,
            Self::ContextPage => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use browseros_core::{TabId, TargetId};

    use super::{PageEventKind, Subscription, Unknown};
    use crate::events::page_event::{DialogId, DialogKind, DialogOpened, PageEvent};

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

    /// Playwright 1.64.1's context emits `dialog` for every page; Neo's context subscription
    /// receives each tab's dialog openings, and nothing else.
    #[test]
    fn tests_that_the_context_receives_every_tabs_dialogs() {
        let dialog = PageEvent::Dialog(DialogOpened {
            id: DialogId(1),
            target: TargetId("T".to_owned()),
            kind: DialogKind::Confirm,
            message: String::new(),
            default_value: String::new(),
        });
        assert_eq!(
            PageEventKind::context("dialog"),
            Some(PageEventKind::Dialog)
        );
        assert!(Subscription::Context(PageEventKind::Dialog).matches(&dialog));
        assert!(!Subscription::Context(PageEventKind::DialogClosed).matches(&dialog));
        assert!(
            !Subscription::Context(PageEventKind::Dialog)
                .matches(&PageEvent::Closed(TargetId("T".to_owned())))
        );
    }

    #[test]
    fn tests_that_a_crash_reaches_only_its_own_pages_listener() {
        let page = |target: &str| Subscription::Page {
            tab: TabId(1),
            target: TargetId(target.to_owned()),
            event: PageEventKind::Crash,
        };
        let crashed = PageEvent::Crashed(TargetId("T".to_owned()));
        assert!(page("T").matches(&crashed));
        assert!(!page("U").matches(&crashed));
        assert!(
            !page("T").matches(&PageEvent::Closed(TargetId("T".to_owned()))),
            "a crash is no close, and a close no crash"
        );
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
