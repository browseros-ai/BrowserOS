//! What the owner's page needs from a fitted tab, read every 750 ms: the boxes of the fields a
//! phone keyboard opens for, the browser bar, and the pages the tab opened, which on a phone
//! are pulled into the tab. zuzu's `ownerViewScript` and `ownerTabJS`, tuned against real
//! phones, ported.
//!
//! Geometry only: the script reads where fields and the page's drawings of them are, never
//! their values, labels' words or text. It runs in BrowserOS's isolated world (core's
//! `PageJs::isolated_only`), so the page can neither see it nor forge what it reports, and it
//! writes nothing into the page: it never changes focus, zoom, the window or the viewport.

use crate::fit_rule::FitMode;
use crate::limits::{MAX_KEYBOARD_BOXES, MAX_SCANNED_FIELDS, POPUP_CLOSE_POLL, POPUP_CLOSE_WAIT};
use crate::{BrowserBar, BrowserTabs, Keyboard, PartError, ScreenSize};
use browseros_core::pages::PageInfo;
use browseros_core::{PageJs, ProtocolSession, TabId, TargetId};
use neo_trace::EpochMillis;
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
        FitMode::Mobile => Some(bar(&session).await?),
        FitMode::Desktop => None,
    };
    let popups = popups(browser, &pages, page, mode, known).await?;
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

/// The editable text-like fields a tap can reach, while the page has focus, in screen px: a
/// field that passes a hit test at its middle, and a field the page draws with its own elements
/// ([`DRAWN_BY`]), such as six digit squares over a code input with no size.
///
/// Screen px are page px times the pinch zoom and the browser zoom (`devicePixelRatio`, over
/// the mode's own device scale: 2 on a phone). A phone's page fills the fullscreen window from
/// the screen's top-left; a desktop's starts below the browser's toolbar, whose height is the
/// window's outer height less the page's.
///
/// A field with no size is hit-tested at its top-left corner, where a page lays its drawing, and
/// its box is the drawing ([`DRAWING`]), since a tap must land on what the owner sees. A drawn
/// field with a size keeps its own box: the drawing lies over it.
fn script(mode: FitMode) -> String {
    let (device_scale, desktop) = match mode {
        FitMode::Mobile => (2, false),
        FitMode::Desktop => (1, true),
    };
    format!(
        "(() => {{
  const v = visualViewport, scale = v.scale * devicePixelRatio / {device_scale}, boxes = [];
  const left = {desktop} ? screenX : 0, top = {desktop} ? screenY + outerHeight - innerHeight : 0;
  const clip = r => {{
    const x = Math.max(r.left, v.offsetLeft), y = Math.max(r.top, v.offsetTop);
    return [x, y, Math.min(r.right, v.offsetLeft + v.width) - x, Math.min(r.bottom, v.offsetTop + v.height) - y];
  }};
  const drawnBy = {DRAWN_BY}, drawing = {DRAWING};
  let scanned = 0;
  if (document.hasFocus()) for (const e of document.querySelectorAll('input,textarea,[contenteditable]')) {{
    if (boxes.length >= {MAX_KEYBOARD_BOXES} || ++scanned > {MAX_SCANNED_FIELDS}) break;
    if (e.disabled || e.readOnly || !(e.isContentEditable || e.tagName === 'TEXTAREA' ||
        ['text', 'search', 'url', 'tel', 'email', 'password', 'number'].includes(e.type))) continue;
    const r = e.getBoundingClientRect(), empty = r.width === 0 || r.height === 0;
    let [x, y, w, h] = clip(r);
    const hit = empty ? document.elementFromPoint(r.left, r.top)
      : w > 0 && h > 0 ? document.elementFromPoint(x + w / 2, y + h / 2) : null;
    if (!hit) continue;
    if (hit !== e && !e.contains(hit)) {{
      // A form's named controls shadow its own DOM methods in every world (`<input
      // name=contains>`), so the rule can throw on a page's form: that field is then not drawn,
      // and the other fields keep their boxes.
      try {{
        if (!drawnBy(e, hit)) continue;
        if (empty) [x, y, w, h] = clip(drawing(e, hit).getBoundingClientRect());
      }} catch {{ continue; }}
    }}
    if (w <= 0 || h <= 0) continue;
    boxes.push([left + (x - v.offsetLeft) * scale, top + (y - v.offsetTop) * scale, w * scale, h * scale]);
  }}
  return {{ boxes, screen: [screen.width, screen.height], touch: navigator.maxTouchPoints > 0 }};
}})()"
    )
}

/// `(field, top) => bool`: whether the page draws `field` with its own elements, given `top`,
/// the element the browser finds on top at the field's place (its middle, or for a field with
/// no size, its top-left corner) when that is neither the field nor inside it. zuzu's
/// `neo.DrawnBy` (ZUZU-282), ported, but for its own-label exception (below). The page draws
/// the field when all hold:
///
/// - the page renders the field: `display:none`, which has no client rects, never counts;
/// - `top` is inside the field's scope but is not the scope itself, the page's body, or the
///   field or what it holds. The scope is the field's parent, or for a field with no size its
///   form (else its parent), so a drawing elsewhere on the page never claims a field;
/// - `top` is not inside a control (a link, button, field, select or label, or a
///   `role=button`/`role=link`), which draws itself. Only a field with no size may be drawn by
///   its own label, as a digit row inside the label that holds the code input. The nearest
///   control must be that label: a "Resend code" link inside it is the link's, and a tap there
///   follows the link. zuzu exempted everything inside the label, links and buttons included;
/// - a `top` that holds the field counts only for a field with no size, and only when it draws
///   something else beside the field: a rendered child that does not hold the field, as a digit
///   row holding the input and its digits. A wrapper that holds nothing but the field, or the
///   form's line of text, is not a drawing.
///
/// So a hidden input with no stand-in gets no box: off screen (no element on top), alone in the
/// body or a wrapper, next to text or a button, or `display:none`.
const DRAWN_BY: &str = "(field, top) => {
    if (!field.getClientRects().length) return false;
    const r = field.getBoundingClientRect(), empty = r.width === 0 || r.height === 0;
    const scope = empty ? field.form || field.parentElement : field.parentElement;
    if (!scope || scope === field.ownerDocument.body || top === scope || !scope.contains(top) ||
        field.contains(top)) return false;
    const control = top.closest('a,button,input,select,textarea,label,[role=button],[role=link]');
    if (control && !(empty && [...(field.labels || [])].includes(control))) return false;
    return !top.contains(field) ||
      empty && [...top.children].some(c => !c.contains(field) && c.getClientRects().length > 0);
  }";

/// `(field, top) => Element`: what a tap on a drawn field with no size must land on, the
/// drawing as a whole rather than the one digit at the field's corner. That is `top` when it
/// holds the field; else the branch of the scope that holds `top`, or the field's parent when
/// that branch sits beside the field there, unless the parent is the field's whole form. zuzu's
/// `ownerViewScript`, ported unchanged. [`DRAWN_BY`] has checked that the scope holds `top`, so
/// the climb ends inside it.
const DRAWING: &str = "(field, top) => {
    if (top.contains(field)) return top;
    let branch = top;
    while (!branch.parentElement.contains(field)) branch = branch.parentElement;
    return branch.parentElement === field.parentElement && field.parentElement !== field.form
      ? field.parentElement : branch;
  }";

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

/// The tab's current URL and available history for the owner page's browser bar.
async fn bar(session: &ProtocolSession) -> Result<BrowserBar, PartError> {
    let history = history(session).await?;
    let url = history
        .current()
        .map(|entry| entry.url.clone())
        .unwrap_or_default();
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

/// The owner view's script on real BrowserOS pages at a phone's size: which fields get a
/// keyboard box, and where. Needs a throwaway BrowserOS with CDP on `BROWSEROS_CDP_PORT`
/// (default 9005), for example
/// `BROWSEROS_CDP_PORT=9371 cargo test -p neo-takeover owner_view -- --ignored`.
///
/// The fits loop and the host API around the script are the fits tests' (over a fake browser,
/// which cannot lay out a page); these read real layout and hit tests.
#[cfg(test)]
mod live_tests {
    use super::PageReading;
    use crate::BrowserTabs;
    use crate::fit_rule::FitMode;
    use browseros_cdp::{CdpClient, ConnectOptions};
    use browseros_core::pages::PageInfo;
    use browseros_core::viewport::Viewports;
    use browseros_core::{BrowserSession, BrowserSessionHooks, PageJs, ProtocolSession, TargetId};
    use percent_encoding::NON_ALPHANUMERIC;
    use serde_json::{Value, json};
    use std::sync::Arc;
    use std::time::Duration;

    type BoxError = Box<dyn std::error::Error + Send + Sync>;
    type TestResult = Result<(), BoxError>;

    /// A sign-in code the page draws as six digit squares over one input with no size, beside
    /// an ordinary field. A tap on the squares reaches the page's own focus of the input.
    const CODE_ENTRY: &str = r#"<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1">
<style>body{margin:16px}#email{display:block;width:260px;height:32px;margin-bottom:24px}
.otp{position:relative}.otp input{position:absolute;width:0;height:0;padding:0;border:0;opacity:0}
.digits{display:flex;gap:8px}.digits span{width:36px;height:44px;border:1px solid}</style>
<input id="email" type="email" aria-label="Email">
<div class="otp" onclick="document.getElementById('otp').focus()">
<input id="otp" inputmode="numeric" autocomplete="one-time-code" maxlength="6" aria-label="Code">
<div class="digits"><span></span><span></span><span></span><span></span><span></span><span></span></div></div>"#;

    /// zuzu's scenario 89 page (`cell/harness/test/e2e-gateway.mjs`, `/local-browser-fixture?keyboard=1`),
    /// unchanged but for its script: one visible field and one drawn code box get a keyboard
    /// box; a disabled field, a read-only one, and a field with no size that nothing draws
    /// (alone in a bordered wrapper, before a button, beside text, `display:none`, alone in the
    /// body) do not.
    const ZUZU_SCENARIO_89: &str = concat!(
        r#"<title>Local pilot fixture</title><meta name="viewport" content="width=device-width"><body>"#,
        r#"<input id="mobile-text" value="keyboard-private-fixture"><input disabled value="disabled"><textarea readonly>read only</textarea>"#,
        r#"<form><input id="drawn-code" aria-label="Code" style="position:absolute;width:0;height:0;padding:0;border:0;opacity:0">"#,
        r#"<div onclick="this.previousElementSibling.focus()" style="display:flex;gap:4px">"#,
        r#"<div style="width:24px;height:32px;border:1px solid"></div><div style="width:24px;height:32px;border:1px solid"></div>"#,
        r#"<div style="width:24px;height:32px;border:1px solid"></div><div style="width:24px;height:32px;border:1px solid"></div>"#,
        r#"<div style="width:24px;height:32px;border:1px solid"></div><div style="width:24px;height:32px;border:1px solid"></div></div></form>"#,
        r#"<form><div style="padding:6px;border:1px solid"><input id="wrapped-zero" style="width:0;height:0;padding:0;border:0"></div>"#,
        r#"<div><input id="form-zero" style="width:0;height:0;padding:0;border:0"><button type="button">Send</button></div></form>"#,
        r#"<form><input id="form-only" style="width:0;height:0;padding:0;border:0"> Pay later <button type="button">Go</button></form>"#,
        r#"<form style="position:fixed;left:0;top:0"><input id="gone" style="display:none"><span style="display:block;width:10px;height:10px"></span></form>"#,
        r#"<input name="zero_size" style="width:0;height:0;padding:0;border:0"><div id="result">Before JavaScript</div>"#,
        r#"<a href="/local-browser-next">Next page</a></body>"#,
    );

    /// The rule's edges, one form each so no two share a scope.
    const EDGES: &str = r##"<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1">
<style>body{margin:16px}form{margin:12px 0}.cell{width:30px;height:40px;border:1px solid}
.zero{width:0;height:0;padding:0;border:0}</style>
<form><input id="plain" aria-label="Name"></form>
<form><div style="position:relative;width:228px;height:42px"><input id="under" style="position:absolute;inset:0;opacity:0"><div style="position:absolute;inset:0;display:flex;gap:8px"><div class="cell"></div><div class="cell"></div><div class="cell"></div><div class="cell"></div><div class="cell"></div><div class="cell"></div></div></div></form>
<form><label style="position:relative;display:block;width:200px;height:40px"><input id="labelled" style="position:absolute;inset:0;opacity:0"><span style="position:absolute;inset:0">Labelled</span></label></form>
<form><label id="row-label" style="display:block"><input id="in-label" class="zero" style="position:absolute;opacity:0"><div style="display:flex;gap:4px"><div class="cell"></div><div class="cell"></div><div class="cell"></div><div class="cell"></div></div></label></form>
<form><input id="off-screen" name="website" tabindex="-1" autocomplete="off" style="position:absolute;left:-9999px"></form>
<form><input id="before-button" class="zero"><button type="button">Sign up</button></form>
<form><input id="alone" class="zero" style="opacity:0"></form>
<form><input id="by-link" class="zero" style="position:absolute"><a href="#" style="display:block;width:200px;height:30px">Forgot code?</a></form>
<form><div style="display:flex;gap:4px"><input id="shadowed" class="zero" style="position:absolute"><div class="cell"></div><div class="cell"></div></div><input id="named" name="contains"></form>
<form><label><input id="under-link" class="zero" style="position:absolute"><a href="#resend" style="display:block;width:200px;height:30px">Resend code</a></label></form>"##;

    /// The keyboard boxes the script reports on `html` at a phone's size, and the boxes of the
    /// elements `expected` selects, measured the same way: a phone's page fills the screen from
    /// its top-left at one screen px per page px (DPR 2 over the phone's device scale of 2).
    async fn boxes_on(html: &str, expected: &[&str]) -> Result<[Vec<[f64; 4]>; 2], BoxError> {
        let port: u16 = match std::env::var("BROWSEROS_CDP_PORT") {
            Ok(port) => port.parse()?,
            Err(_) => 9005,
        };
        let client = CdpClient::connect(ConnectOptions::new(port)).await?;
        let session = BrowserSession::new(Arc::new(client), BrowserSessionHooks::default());
        let url = format!(
            "data:text/html,{}",
            percent_encoding::utf8_percent_encode(html, NON_ALPHANUMERIC)
        );
        let created = session
            .cdp("Target.createTarget", json!({ "url": url }), None)
            .await?;
        let target = TargetId(
            created["targetId"]
                .as_str()
                .ok_or("a target id")?
                .to_string(),
        );
        let tabs = BrowserTabs::new(Arc::clone(&session), Arc::new(Viewports::default()));
        let read = read_page(&tabs, &target, expected).await;
        let close = json!({ "targetId": target.as_str() });
        session.cdp("Target.closeTarget", close, None).await?;
        read
    }

    async fn read_page(
        tabs: &BrowserTabs,
        target: &TargetId,
        expected: &[&str],
    ) -> Result<[Vec<[f64; 4]>; 2], BoxError> {
        let session = tabs.session_for(&listed(tabs, target).await?).await?;
        as_a_phone(&session).await?;
        let reading: PageReading = PageJs::isolated_only(&session)
            .eval(&super::script(FitMode::Mobile))
            .await?;
        let want = PageJs::isolated_only(&session)
            .eval(&format!(
                "{}.map(s => {{ const r = document.querySelector(s).getBoundingClientRect(); \
                 return [r.left, r.top, r.width, r.height]; }})",
                serde_json::to_string(expected)?
            ))
            .await?;
        Ok([reading.boxes, want])
    }

    /// The page of the tab `target` names, once the browser lists it.
    async fn listed(tabs: &BrowserTabs, target: &TargetId) -> Result<PageInfo, BoxError> {
        for _ in 0..50 {
            let pages = tabs.pages().await?;
            if let Some(page) = pages.into_iter().find(|page| &page.target_id == target) {
                return Ok(page);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err("the new tab is never listed".into())
    }

    /// Shows the loaded page as a fit's phone does: mobile emulation at its size, DPR 2. The
    /// headless window never has the focus a real one has, which the script requires.
    async fn as_a_phone(session: &ProtocolSession) -> TestResult {
        let phone = json!({ "width": 390, "height": 844, "deviceScaleFactor": 2, "mobile": true });
        let _: Value = session
            .send("Emulation.setDeviceMetricsOverride", phone)
            .await?;
        let focus = json!({ "enabled": true });
        let _: Value = session
            .send("Emulation.setFocusEmulationEnabled", focus)
            .await?;
        for _ in 0..50 {
            let loaded = PageJs::isolated_only(session)
                .eval::<bool>("document.readyState === 'complete'")
                .await;
            if loaded.unwrap_or(false) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err("the page never loads".into())
    }

    #[tokio::test]
    #[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
    async fn a_code_the_page_draws_over_a_field_with_no_size_gets_the_drawings_box() -> TestResult {
        let [boxes, want] = boxes_on(CODE_ENTRY, &["#email", ".otp"]).await?;
        assert_eq!(boxes, want);
        Ok(())
    }

    #[tokio::test]
    #[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
    async fn zuzus_scenario_89_page_gets_its_visible_field_and_its_drawn_code() -> TestResult {
        let [boxes, want] =
            boxes_on(ZUZU_SCENARIO_89, &["#mobile-text", "#drawn-code + div"]).await?;
        assert_eq!(boxes, want);
        Ok(())
    }

    /// A field with a size under the page's drawing keeps its own box; one under its own label's
    /// text gets none, as zuzu's rule has it. A field with no size inside a label that draws it
    /// gets the label's box, but a link inside that label at the field's place is the link's, not
    /// a drawing of the field. No honeypot gets a box: off screen, before a button, alone, beside
    /// only a link. A form whose control is named `contains` shadows `form.contains`, so its drawn
    /// field gets none, and every other field keeps its box.
    #[tokio::test]
    #[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
    async fn the_rules_edges_and_honeypots() -> TestResult {
        let [boxes, want] = boxes_on(EDGES, &["#plain", "#under", "#row-label", "#named"]).await?;
        assert_eq!(boxes, want);
        Ok(())
    }
}
