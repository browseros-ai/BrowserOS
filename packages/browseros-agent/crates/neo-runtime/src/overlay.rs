//! Neo: `Screencast.showOverlay`, `showChapter`, `showOverlays` and `hideOverlays` (Playwright's
//! `server/overlay.ts`): user overlays in the injected Highlight's closed-shadow popover of the
//! tab's main frame, the overlay `locator.highlight` draws in, so they stay out of selectors and
//! snapshots and need no running screencast. They show over the BrowserOS user's tab, so each run
//! removes its own when it ends.
//!
//! ```text
//!  Page.screencastShowOverlay {html, duration?}  → an id; addUserOverlay(id, html) in the main
//!    frame (a failure still answers the id); with duration: wait, then remove → answers after
//!  Page.screencastRemoveOverlay {id}             → removeUserOverlay(id); a failure is ignored
//!  Page.screencastChapter {title, description?, duration?}
//!    → show(escaped card), wait duration ?? 2000, fade-out (300 ms), remove
//!  Page.screencastSetOverlayVisible {visible}    → setUserOverlaysVisible; none shown → nothing
//!  a main-frame navigation to a new document     → every shown overlay is added again
//!  run end (`Dispatcher::finish`)                → every overlay the run shows goes
//! ```

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use neo_runtime_spec::injected::{
    InjectedAddUserOverlayParams, InjectedFadeOutUserOverlayParams,
    InjectedRemoveUserOverlayParams, InjectedSetUserOverlaysVisibleParams,
};
use tokio_util::sync::CancellationToken;

use crate::navigate::Observation;
use crate::page::PageSession;
use crate::progress::{CallError, PlaywrightError, Progress};

/// Playwright: `server/overlay.ts` `Overlay.chapter`: `const duration = options.duration ?? 2000;`
const CHAPTER_MS: f64 = 2000.0;

/// Playwright: `server/overlay.ts` `Overlay.chapter`: `const fadeDuration = 300;`
const FADE_MS: u64 = 300;

/// Neo: the overlays a tab shows, in the order they were added, and the watcher that adds them
/// again in each new main-frame document.
#[derive(Default)]
pub(crate) struct Overlays {
    shown: Mutex<Vec<(String, String)>>,
    next: AtomicU64,
    watcher: Mutex<Option<CancellationToken>>,
}

/// Serves `Page.screencastShowOverlay`: the overlay's id, also when the page could not show it.
///
/// Playwright: `server/overlay.ts` `Overlay.show`: `// We must return id even if this call fails.`
pub(crate) async fn show(
    page: &Arc<PageSession>,
    html: String,
    duration: Option<f64>,
    progress: &Progress,
) -> Result<String, CallError> {
    let id = format!("neo-overlay-{}", page.overlays.next.fetch_add(1, Ordering::Relaxed) + 1);
    if let Ok(mut shown) = page.overlays.shown.lock() {
        shown.push((id.clone(), html.clone()));
    }
    watch(page);
    add(page, &id, &html).await;
    if let Some(duration) = duration.filter(|duration| *duration > 0.0) {
        progress.wait(millis(duration)).await?;
        remove(page, &id).await;
    }
    Ok(id)
}

/// Serves `Page.screencastRemoveOverlay`; an overlay the page cannot reach is forgotten anyway.
///
/// Playwright: `server/overlay.ts` `Overlay.remove`: `this._overlays.delete(id);`
pub(crate) async fn remove(page: &PageSession, id: &str) {
    if let Ok(mut shown) = page.overlays.shown.lock() {
        shown.retain(|(shown, _)| shown != id);
    }
    let main = page.frames.main();
    let _removed = main
        .call(None, &InjectedRemoveUserOverlayParams { id: id.to_string() })
        .await;
}

/// Serves `Page.screencastChapter`: a centered card of `title` (and `description`) over a
/// blurred page for `duration` ms, then a 300 ms fade-out; answers once it is gone.
///
/// Playwright: `server/overlay.ts` `Overlay.chapter`: `const html = `
pub(crate) async fn chapter(
    page: &Arc<PageSession>,
    title: &str,
    description: Option<&str>,
    duration: Option<f64>,
    progress: &Progress,
) -> Result<(), CallError> {
    let id = show(page, chapter_html(title, description), None, progress).await?;
    progress
        .wait(millis(duration.unwrap_or(CHAPTER_MS)))
        .await?;
    let main = page.frames.main();
    let _faded = main
        .call(None, &InjectedFadeOutUserOverlayParams { id: id.clone() })
        .await;
    progress.wait(Duration::from_millis(FADE_MS)).await?;
    remove(page, &id).await;
    Ok(())
}

/// Serves `Page.screencastSetOverlayVisible`: shows or hides every overlay; with none shown, it
/// does nothing.
///
/// Playwright: `server/overlay.ts` `Overlay.setVisible`: `if (!this._overlays.size)`
pub(crate) async fn set_visible(page: &PageSession, visible: bool) {
    if page.overlays.shown.lock().is_ok_and(|shown| shown.is_empty()) {
        return;
    }
    let main = page.frames.main();
    let _set = main
        .call(None, &InjectedSetUserOverlaysVisibleParams { visible })
        .await;
}

/// Neo: at the run's end, every overlay the run shows goes, and its watcher stops: none may
/// outlive the run that showed it in the user's tab.
pub(crate) async fn remove_at_run_end(page: &PageSession) {
    if let Ok(mut watcher) = page.overlays.watcher.lock()
        && let Some(watcher) = watcher.take()
    {
        watcher.cancel();
    }
    let ids: Vec<String> = page
        .overlays
        .shown
        .lock()
        .map(|shown| shown.iter().map(|(id, _)| id.clone()).collect())
        .unwrap_or_default();
    for id in ids {
        remove(page, &id).await;
    }
}

/// Neo: adds overlay `id` to the main frame's current document; a failure is ignored, as
/// Playwright's `show` catches it.
///
/// Playwright: `server/overlay.ts` `Overlay._doAdd`: `return injected.addUserOverlay(id, html);`
async fn add(page: &PageSession, id: &str, html: &str) {
    let main = page.frames.main();
    let _added = main
        .call(
            None,
            &InjectedAddUserOverlayParams {
                id: id.to_string(),
                html: html.to_string(),
            },
        )
        .await;
}

/// Neo: starts the tab's watcher once: after each main-frame navigation to a new document, it
/// adds every overlay still shown again.
///
/// Playwright: `server/overlay.ts` `Overlay.constructor`: `for (const [id, html] of this._overlays)`
fn watch(page: &Arc<PageSession>) {
    let Ok(mut watcher) = page.overlays.watcher.lock() else {
        return;
    };
    if watcher.is_some() {
        return;
    }
    let stop = CancellationToken::new();
    *watcher = Some(stop.clone());
    let tab = Arc::clone(page);
    let mut navigations = page.observe_waits();
    page.session.tokio.spawn(async move {
        loop {
            let navigation = tokio::select! {
                () = stop.cancelled() => return,
                navigation = navigations.next() => navigation,
            };
            match navigation {
                Ok(Some(navigation)) if navigation.document.is_some() => {
                    let shown: Vec<(String, String)> =
                        tab.overlays.shown.lock().map(|shown| shown.clone()).unwrap_or_default();
                    for (id, html) in shown {
                        add(&tab, &id, &html).await;
                    }
                }
                Ok(_) => {}
                Err(_) => return,
            }
        }
    });
}

/// Neo: Playwright's chapter card, its title and description escaped as Playwright's `escapeHTML`
/// does (`&` and `<`).
fn chapter_html(title: &str, description: Option<&str>) -> String {
    let fade = FADE_MS;
    let description = description.map_or_else(String::new, |description| {
        format!("<div id=\"description\">{}</div>", escape_html(description))
    });
    let title = escape_html(title);
    format!(
        "<style>
      @keyframes pw-chapter-fade-in {{
        from {{ opacity: 0; }}
        to {{ opacity: 1; }}
      }}
      @keyframes pw-chapter-fade-out {{
        from {{ opacity: 1; }}
        to {{ opacity: 0; }}
      }}
      #background {{
        position: absolute;
        inset: 0;
        display: flex;
        align-items: center;
        justify-content: center;
        backdrop-filter: blur(2px);
        animation: pw-chapter-fade-in {fade}ms ease-out forwards;
      }}
      #background.fade-out {{
        animation: pw-chapter-fade-out {fade}ms ease-in forwards;
      }}
      #content {{
        background: rgba(0, 0, 0, 0.7);
        border: 1px solid rgba(255, 255, 255, 0.15);
        border-radius: 16px;
        padding: 40px 56px;
        max-width: 560px;
        box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
      }}
      #title {{
        color: white;
        font-family: system-ui, -apple-system, sans-serif;
        font-size: 28px;
        font-weight: 600;
        line-height: 1.3;
        text-align: center;
        letter-spacing: -0.01em;
      }}
      #description {{
        color: rgba(255, 255, 255, 0.7);
        font-family: system-ui, -apple-system, sans-serif;
        font-size: 15px;
        line-height: 1.5;
        margin-top: 12px;
        text-align: center;
      }}
    </style><div id=\"background\"><div id=\"content\"><div id=\"title\">{title}</div>{description}</div></div>"
    )
}

/// Playwright: `isomorphic/stringUtils.ts` `escapeHTML`: `return s.replace(/[&<]/ug, char => (escaped as any)[char]);`
fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;")
}

/// A protocol duration in ms, as a `Duration` (a negative or non-finite one is none).
fn millis(ms: f64) -> Duration {
    if ms.is_finite() && ms > 0.0 {
        Duration::from_secs_f64(ms / 1000.0)
    } else {
        Duration::ZERO
    }
}

fn _unused(_: PlaywrightError) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tests_that_a_chapter_escapes_what_playwright_escapes() {
        let html = chapter_html("A & <b>", Some("x < y > z"));
        assert!(html.contains("<div id=\"title\">A &amp; &lt;b></div>"), "{html}");
        assert!(html.contains("<div id=\"description\">x &lt; y > z</div>"), "{html}");
        assert!(html.contains("pw-chapter-fade-out 300ms"), "{html}");
        assert!(!chapter_html("t", None).contains("id=\"description\""));
    }
}
