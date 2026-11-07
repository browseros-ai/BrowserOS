//! Neo: `Locator.highlight`, `Locator.hideHighlight` and `Page.hideHighlight` over Playwright's
//! injected overlay, which shows over the BrowserOS user's tab. Each run keeps its own
//! highlights and hides them when it ends, so a run never leaves an overlay that only a later
//! run could take down.
//!
//! ```text
//!  Frame.highlight {selector, style?}  describe (invalid → Error) → locate, not strict
//!    → the run's entry (receiver frame, selector), placed in the document the selector reaches
//!    → setHighlights(every entry of that document); the overlay follows them each frame
//!  Frame.hideHighlight {selector}      that entry goes → setHighlights(the rest); [] clears
//!  Page.hideHighlight {}               every entry goes → hideHighlight in each document the
//!                                      run put an overlay in, emptied ones included
//!  run end (`Dispatcher::finish`)      as Page.hideHighlight, on every tab the run touched
//! ```

use std::collections::HashMap;
use std::mem;
use std::sync::{Arc, Mutex};

use tokio::sync::Mutex as WriteTurn;

use neo_runtime_spec::injected::{
    HighlightSpec, InjectedHideHighlightParams, InjectedSetHighlightsParams,
};
use neo_runtime_spec::serve::{FrameHideHighlightParams, FrameHighlightParams};

use crate::click::{PageError, Tab};
use crate::frame_selector;
use crate::frames::{FrameContext, FrameScope};
use crate::page::{FrameTab, PageSession};
use crate::progress::{CallError, PlaywrightError, Progress};

/// Neo: one `Frame.highlight` or `Frame.hideHighlight`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HighlightOptions {
    pub(crate) selector: String,
    /// The overlay's CSS, as `Locator.highlight` turned its `style` into text.
    pub(crate) style: Option<String>,
}

/// Neo: decodes both commands' params.
impl HighlightOptions {
    pub(crate) fn from_highlight(params: FrameHighlightParams) -> Self {
        Self {
            selector: params.selector,
            style: params.style,
        }
    }

    pub(crate) fn from_hide(params: FrameHideHighlightParams) -> Self {
        Self {
            selector: params.selector,
            style: None,
        }
    }
}

/// Neo: the run's highlights on one tab, in the order they were added, every document the run
/// installed an overlay in, and each frame's write turn. Playwright's `hideHighlights` removes
/// the overlay of every frame; Neo reaches the documents it put one in.
///
/// Playwright resolves every highlight on one chain (`_resolutionChain`), so no older list
/// lands after a newer one. Neo writes each document under that frame's turn and reads its list
/// from the registry within the turn, so a write always carries the latest list, and a document
/// whose write is in flight holds up no other document.
#[derive(Default)]
pub(crate) struct Highlights {
    registry: Mutex<Registry>,
    turns: Mutex<HashMap<String, Arc<WriteTurn<()>>>>,
}

#[derive(Default)]
struct Registry {
    entries: Vec<Entry>,
    /// Each document context that received an overlay: a frame that changed process has a new
    /// context under the same frame id, and both may hold one.
    overlays: Vec<Arc<FrameContext>>,
}

/// One highlight: Playwright's entry key (the receiving frame and the selector) and the
/// document chunk the selector resolved to there.
struct Entry {
    receiver: FrameScope,
    selector: String,
    frame: Arc<FrameContext>,
    spec: HighlightSpec,
}

/// Serves `Frame.highlight`: the selector's highlight in the document it reaches, with every
/// other highlight of that document.
///
/// Deviation: `server/highlightController.ts` `HighlightController._resolve`: `this._resolutionTimer = setTimeout(() => this._resolveNow(), 1000);` Playwright re-resolves its highlights every second until they are hidden, so one shows up in a frame that appears later or in the next document, and keeps them until hidden or the page closes; Neo places a highlight once, in the document its selector reaches, and hides the run's highlights when the run ends.
pub(crate) async fn highlight(
    tab: &FrameTab,
    options: &HighlightOptions,
    progress: &Progress,
) -> Result<(), CallError> {
    progress
        .race(Tab::prepare(tab))
        .await?
        .map_err(page_error)?;
    // Playwright: `server/highlightController.ts` `HighlightController.addHighlight`: `this._page.browserContext.selectors().parseSelector(selector, false);`
    let facts = progress
        .race(Tab::describe(tab, &options.selector))
        .await?
        .map_err(page_error)?;
    if let Some(message) = facts.invalid {
        return Err(PlaywrightError::error(message).into());
    }
    // Playwright: `server/highlightController.ts` `HighlightController._resolve`: `const results = await entry.frame.selectors.resolveFramesForSelector({ selector: entry.selector, strict: false, anyFrame: entry.anyFrame }).catch(() => []);`
    let located = progress
        .race(frame_selector::locate(tab, &options.selector, false))
        .await?;
    let Some(Some(located)) = quiet(located)? else {
        return Ok(());
    };
    let frame = located.frame;
    let documents = tab.highlights.update(|registry| {
        if !registry
            .overlays
            .iter()
            .any(|overlay| Arc::ptr_eq(overlay, &frame))
        {
            registry.overlays.push(Arc::clone(&frame));
        }
        let entries = &mut registry.entries;
        let mut documents = vec![Arc::clone(&frame)];
        entries.retain(|entry| {
            let named = entry.names(&tab.scope, &options.selector);
            // The selector reached another document before: it loses this highlight too.
            if named && entry.frame.id != frame.id {
                documents.push(Arc::clone(&entry.frame));
            }
            !named
        });
        entries.push(Entry {
            receiver: tab.scope.clone(),
            selector: options.selector.clone(),
            frame: Arc::clone(&frame),
            spec: HighlightSpec {
                selector: located.selector,
                css_style: options.style.clone(),
            },
        });
        documents
    })?;
    show_each(&tab.highlights, documents, progress).await
}

/// Serves `Frame.hideHighlight`: that selector's highlight goes, the rest of its document stays.
///
/// Playwright: `server/highlightController.ts` `HighlightController.removeHighlight`: `this._entries.delete(this._key(frame ?? this._page.mainFrame(), selector));`
pub(crate) async fn hide_highlight(
    tab: &FrameTab,
    options: &HighlightOptions,
    progress: &Progress,
) -> Result<(), CallError> {
    let documents = tab.highlights.update(|registry| {
        let entries = &mut registry.entries;
        let mut removed = Vec::new();
        entries.retain(|entry| {
            let named = entry.names(&tab.scope, &options.selector);
            if named {
                removed.push(Arc::clone(&entry.frame));
            }
            !named
        });
        removed
    })?;
    show_each(&tab.highlights, documents, progress).await
}

/// Serves `Page.hideHighlight`: every highlight of the tab goes, and the overlay of the main
/// frame and of every document the run put one in, emptied ones included. A document that
/// cannot be reached is skipped.
///
/// Playwright: `server/highlightController.ts` `HighlightController.hideHighlights`: `await injectedScript?.evaluate(injected => injected.hideHighlight());`
pub(crate) async fn hide_all(page: &PageSession, progress: &Progress) -> Result<(), CallError> {
    for frame in page.highlights.take_documents(page)? {
        let _turn = progress
            .race(page.highlights.turn(&frame.id)?.lock_owned())
            .await?;
        // Playwright: `server/highlightController.ts` `HighlightController.hideHighlights`: `}).catch(() => {})));`
        let _hidden = progress
            .race(frame.call(None, &InjectedHideHighlightParams {}))
            .await?;
    }
    Ok(())
}

/// Neo: the run-end cleanup, without a call's progress: removes every overlay the run put on
/// one tab, as `Page.hideHighlight` does. A document that navigated away is skipped; a closed
/// tab ends it, as it needs no cleanup.
pub(crate) async fn hide_at_run_end(page: &PageSession) {
    if !page.highlights.any() {
        return;
    }
    let Ok(frames) = page.highlights.take_documents(page) else {
        return;
    };
    for frame in frames {
        let Ok(turn) = page.highlights.turn(&frame.id) else {
            return;
        };
        let _turn = turn.lock_owned().await;
        let hidden = frame.call(None, &InjectedHideHighlightParams {}).await;
        if matches!(hidden, Err(PageError::Closed)) {
            return;
        }
    }
}

/// Neo: sets each document's highlights to its specs; an empty list clears them. A document
/// that cannot be reached keeps what it had, as Playwright's `_resolve` ignores that frame.
///
/// Playwright: `server/highlightController.ts` `HighlightController._resolve`: `await injectedScript.evaluate((injected, highlights) => injected.setHighlights(highlights), highlights);`
async fn show_each(
    highlights: &Highlights,
    documents: Vec<Arc<FrameContext>>,
    progress: &Progress,
) -> Result<(), CallError> {
    let mut written: Vec<Arc<FrameContext>> = Vec::new();
    for frame in documents {
        if written.iter().any(|seen| Arc::ptr_eq(seen, &frame)) {
            continue;
        }
        let _turn = progress
            .race(highlights.turn(&frame.id)?.lock_owned())
            .await?;
        // The list as the registry holds it now, within the turn: never an older one.
        let specs = highlights.update(|registry| specs_of(&registry.entries, &frame))?;
        let shown = progress
            .race(frame.call(None, &InjectedSetHighlightsParams { highlights: specs }))
            .await?;
        quiet(shown)?;
        written.push(frame);
    }
    Ok(())
}

/// Neo: every highlight shown in `frame`'s frame: entries placed in an earlier document of the
/// same frame are re-resolved in this one, as Playwright's `_resolve` resolves each entry again.
fn specs_of(entries: &[Entry], frame: &FrameContext) -> Vec<HighlightSpec> {
    entries
        .iter()
        .filter(|entry| entry.frame.id == frame.id)
        .map(|entry| entry.spec.clone())
        .collect()
}

/// Neo: a highlight step's page error. Only a closed tab ends the call; anything else (the
/// document changed, the page threw) leaves nothing to show there, as Playwright ignores it.
fn quiet<T>(result: Result<T, PageError>) -> Result<Option<T>, CallError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(PageError::Closed) => Err(PlaywrightError::target_closed().into()),
        Err(_) => Ok(None),
    }
}

/// Neo: a selector check's page error as the highlight call ends.
fn page_error(error: PageError) -> CallError {
    match error {
        PageError::Closed => PlaywrightError::target_closed().into(),
        PageError::Threw(message) | PageError::Contract(message) => {
            PlaywrightError::error(message).into()
        }
        PageError::NotConnected => PlaywrightError::error(
            "neo-runtime: the frame's document changed while its highlight selector was checked",
        )
        .into(),
    }
}

/// Neo: one entry's key, Playwright's `frame.guid + ':' + selector`.
impl Entry {
    fn names(&self, receiver: &FrameScope, selector: &str) -> bool {
        &self.receiver == receiver && self.selector == selector
    }
}

/// Neo: the registry's lock is never held across an await; a frame's write turn is held across
/// that frame's write only.
impl Highlights {
    fn update<R>(&self, change: impl FnOnce(&mut Registry) -> R) -> Result<R, CallError> {
        let mut registry = self.registry.lock().map_err(|_| unreadable())?;
        Ok(change(&mut registry))
    }

    /// The write turn of the frame `frame`, one for all its documents.
    fn turn(&self, frame: &str) -> Result<Arc<WriteTurn<()>>, CallError> {
        let mut turns = self.turns.lock().map_err(|_| unreadable())?;
        Ok(Arc::clone(turns.entry(frame.to_string()).or_default()))
    }

    /// Whether the run left an overlay on this tab, shown or emptied.
    fn any(&self) -> bool {
        self.registry
            .lock()
            .is_ok_and(|registry| !registry.overlays.is_empty())
    }

    /// Clears the registry and answers each document the run put an overlay in, the main
    /// frame's first.
    fn take_documents(&self, page: &PageSession) -> Result<Vec<Arc<FrameContext>>, CallError> {
        let registry = self.update(mem::take)?;
        let mut frames = vec![page.frames.main()];
        for overlay in registry.overlays {
            if !frames.iter().any(|frame| Arc::ptr_eq(frame, &overlay)) {
                frames.push(overlay);
            }
        }
        Ok(frames)
    }
}

/// Neo: a poisoned registry lock, which only a panic while holding it leaves.
fn unreadable() -> CallError {
    PlaywrightError::error("neo-runtime: the run's highlights cannot be read").into()
}
