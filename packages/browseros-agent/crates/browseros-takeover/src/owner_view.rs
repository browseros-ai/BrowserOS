//! What the owner's page needs from a fitted tab, read every 750 ms: the boxes of the fields a
//! phone keyboard opens for, the browser bar, and the pages the tab opened, which on a phone
//! are pulled into the tab. zuzu's `ownerViewScript` and `ownerTabJS`, tuned against real
//! phones, ported.
//!
//! Geometry only: the script reads where fields are, never their values, labels or text. It
//! runs in BrowserOS's isolated world (core's `PageJs::isolated_only`), so the page can neither
//! see it nor forge what it reports, and it never changes zoom, the window or the viewport.

use crate::fit_rule::FitMode;
use crate::limits::{MAX_KEYBOARD_BOXES, MAX_SCANNED_FIELDS, POPUP_CLOSE_POLL, POPUP_CLOSE_WAIT};
use crate::{BrowserBar, BrowserTabs, Keyboard, PartError, ScreenSize};
use browseros_core::pages::PageInfo;
use browseros_core::{PageJs, ProtocolSession, TabId, TargetId};
use browseros_trace::{EpochMillis, SecretSet};
use serde::Deserialize;
use serde_json::json;
use tokio::time::Instant;

/// One reading of a fitted tab.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OwnerView {
    /// `None` when the page did not show the fit's screen size and touch, as after a resize it
    /// has not caught up with: boxes measured at another size would open the keyboard in the
    /// wrong place.
    pub(crate) keyboard: Option<Keyboard>,
    /// The tab's place; phones only, since a desktop window shows the browser's own toolbar.
    pub(crate) bar: Option<BrowserBar>,
    /// The pages the tab opened that stay where they are.
    pub(crate) popups: Vec<TargetId>,
}

/// Reads `tab` as fitted at `screen` in `mode`. `known` lists the pages from the last read that
/// stay where they are; `None` on a fit's first read, which pulls nothing in, so what the tab
/// opened before the owner arrived stays where it is.
pub(crate) async fn read(
    browser: &BrowserTabs,
    secrets: &SecretSet,
    tab: TabId,
    mode: FitMode,
    screen: ScreenSize,
    known: Option<&[TargetId]>,
) -> Result<OwnerView, PartError> {
    let pages = browser.pages().await?;
    let page = pages
        .iter()
        .find(|page| page.tab_id == tab)
        .ok_or(PartError::TabNotFound(tab))?;
    let session = browser.session_for(page).await?;
    let mut reading: PageReading = PageJs::isolated_only(&session)
        .eval(&script(mode))
        .await
        .map_err(PartError::Browser)?;
    reading.boxes.truncate(MAX_KEYBOARD_BOXES);
    let shown = reading.screen == [u32::from(screen.width), u32::from(screen.height)]
        && reading.touch == (mode == FitMode::Mobile);
    let keyboard = shown.then(|| Keyboard {
        screen,
        boxes: reading.boxes,
        at: EpochMillis::now(),
    });
    let bar = match mode {
        FitMode::Mobile => Some(bar(&session, secrets).await?),
        FitMode::Desktop => None,
    };
    let popups = popups(browser, secrets, &pages, page, mode, known).await?;
    Ok(OwnerView {
        keyboard,
        bar,
        popups,
    })
}

/// What the script reports.
#[derive(Deserialize)]
struct PageReading {
    boxes: Vec<[f64; 4]>,
    /// `screen.width` and `screen.height`: the emulated phone's, or Neo's screen on a desktop.
    screen: [u32; 2],
    touch: bool,
}

/// The visible text-like fields that pass a hit test, while the page has focus, in screen px.
/// Screen px are page px times the pinch zoom and the browser zoom (`devicePixelRatio`, over
/// the mode's own device scale: 2 on a phone). A phone's page fills the fullscreen window from
/// the screen's top-left; a desktop's starts below the browser's toolbar, whose height is the
/// window's outer height less the page's.
fn script(mode: FitMode) -> String {
    let (device_scale, desktop) = match mode {
        FitMode::Mobile => (2, false),
        FitMode::Desktop => (1, true),
    };
    format!(
        "(() => {{
  const v = visualViewport, scale = v.scale * devicePixelRatio / {device_scale}, boxes = [];
  const left = {desktop} ? screenX : 0, top = {desktop} ? screenY + outerHeight - innerHeight : 0;
  let scanned = 0;
  if (document.hasFocus()) for (const e of document.querySelectorAll('input,textarea,[contenteditable]')) {{
    if (boxes.length >= {MAX_KEYBOARD_BOXES} || ++scanned > {MAX_SCANNED_FIELDS}) break;
    if (e.disabled || e.readOnly || !(e.isContentEditable || e.tagName === 'TEXTAREA' ||
        ['text', 'search', 'url', 'tel', 'email', 'password', 'number'].includes(e.type))) continue;
    const r = e.getBoundingClientRect();
    const x = Math.max(r.left, v.offsetLeft), y = Math.max(r.top, v.offsetTop);
    const w = Math.min(r.right, v.offsetLeft + v.width) - x, h = Math.min(r.bottom, v.offsetTop + v.height) - y;
    if (w <= 0 || h <= 0) continue;
    const hit = document.elementFromPoint(x + w / 2, y + h / 2);
    if (hit !== e && !e.contains(hit)) continue;
    boxes.push([left + (x - v.offsetLeft) * scale, top + (y - v.offsetTop) * scale, w * scale, h * scale]);
  }}
  return {{ boxes, screen: [screen.width, screen.height], touch: navigator.maxTouchPoints > 0 }};
}})()"
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct History {
    current_index: i64,
    entries: Vec<HistoryEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryEntry {
    url: String,
    #[serde(default)]
    transition_type: String,
}

impl History {
    fn current(&self) -> Option<&HistoryEntry> {
        usize::try_from(self.current_index)
            .ok()
            .and_then(|index| self.entries.get(index))
    }
}

async fn history(session: &ProtocolSession) -> Result<History, PartError> {
    session
        .send("Page.getNavigationHistory", json!({}))
        .await
        .map_err(PartError::Browser)
}

/// The tab's place for the owner page's bar. Its URL goes through the known secrets mask, as
/// every other read a host gets does: an address can carry what the owner typed into a form.
async fn bar(session: &ProtocolSession, secrets: &SecretSet) -> Result<BrowserBar, PartError> {
    let history = history(session).await?;
    let mut url = history
        .current()
        .map(|entry| entry.url.clone())
        .unwrap_or_default();
    secrets.mask_str(&mut url);
    let next = history.current_index.saturating_add(1);
    Ok(BrowserBar {
        url,
        back: history.current_index > 0,
        forward: usize::try_from(next).is_ok_and(|next| next < history.entries.len()),
    })
}

/// What became of one page the fitted tab opened.
enum PullIn {
    /// Its address loaded in the fitted tab, and then it closed.
    Moved,
    /// Still loading; the next read looks again.
    Later,
    /// It stays where it is.
    Stays,
}

/// The pages `fitted` opened that stay where they are. On a phone, the fullscreen tab is the
/// owner's whole browser, so each page it opened since the last read is pulled into it. A page
/// that could not be pulled in stays, and is not tried again.
async fn popups(
    browser: &BrowserTabs,
    secrets: &SecretSet,
    pages: &[PageInfo],
    fitted: &PageInfo,
    mode: FitMode,
    known: Option<&[TargetId]>,
) -> Result<Vec<TargetId>, PartError> {
    let pull = mode == FitMode::Mobile && known.is_some();
    let known = known.unwrap_or_default();
    let mut stay = Vec::new();
    for opened in browser.opened_by(fitted).await? {
        let popup = pages.iter().find(|page| page.target_id == opened.target);
        let pulled = match popup {
            Some(popup) if pull && !known.contains(&opened.target) => {
                pull_in(browser, fitted, popup, &opened.url).await
            }
            _ => Ok(PullIn::Stays),
        };
        match pulled {
            Ok(PullIn::Moved | PullIn::Later) => {}
            Ok(PullIn::Stays) => stay.push(opened.target),
            Err(error) => {
                // A failed load's error names the address, which can carry a known secret.
                let mut error = error.to_string();
                secrets.mask_str(&mut error);
                tracing::debug!(%error, "a page the fitted tab opened could not be pulled in");
                stay.push(opened.target);
            }
        }
    }
    Ok(stay)
}

/// Moves `popup` into `fitted`, if it can load again from its address: an http(s) page that
/// has loaded there, and not a form's response, which only its POST could load again.
///
/// The fitted tab loads the address first, and the popup closes only once it has, so a load
/// that fails, or turns out to be a download, leaves the owner the popup.
async fn pull_in(
    browser: &BrowserTabs,
    fitted: &PageInfo,
    popup: &PageInfo,
    url: &str,
) -> Result<PullIn, PartError> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Ok(PullIn::Later);
    }
    let history = history(&browser.session_for(popup).await?).await?;
    let Some(entry) = history.current().filter(|entry| entry.url == url) else {
        return Ok(PullIn::Later);
    };
    if entry.transition_type == "form_submit" {
        return Ok(PullIn::Stays);
    }
    let loaded = browser
        .navigation(fitted)
        .goto(url)
        .await
        .map_err(PartError::Browser)?;
    if loaded.download {
        return Ok(PullIn::Stays);
    }
    browser.close(popup).await?;
    let deadline = Instant::now() + POPUP_CLOSE_WAIT;
    while browser
        .pages()
        .await?
        .iter()
        .any(|page| page.target_id == popup.target_id)
    {
        // A page that asks "Leave site?" never closes; it stays where it is.
        if Instant::now() >= deadline {
            return Ok(PullIn::Stays);
        }
        tokio::time::sleep(POPUP_CLOSE_POLL).await;
    }
    Ok(PullIn::Moved)
}
