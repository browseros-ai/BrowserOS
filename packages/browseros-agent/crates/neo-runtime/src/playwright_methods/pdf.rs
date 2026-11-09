//! Playwright: `server/chromium/crPdf.ts` `CRPDF`
//! Playwright: `server/chromium/crProtocolHelper.ts` `readProtocolStream`
//!
//! `Page.pdf`: Playwright's option mapping onto one `Page.printToPDF` with
//! `transferMode: ReturnAsStream`, then the stream read in `IO.read` chunks until `eof` and
//! closed. Option errors come before any CDP call. The client's unit conversion (numbers to
//! `px`) and `path` are the SDK's and `capture.rs`'s.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use neo_runtime_spec::serve::PagePdfParams;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tokio::time;
use tracing::debug;

use crate::browser_representations::page::PageSession;
use crate::playwright_methods::capture;
use crate::progress::{self, CallError, PlaywrightError, Progress};

/// Playwright: `server/chromium/crPdf.ts` `CRPDF.generate`: `let paperWidth = 8.5;`
const DEFAULT_PAPER_WIDTH: f64 = 8.5;
/// Playwright: `server/chromium/crPdf.ts` `CRPDF.generate`: `let paperHeight = 11;`
const DEFAULT_PAPER_HEIGHT: f64 = 11.0;
/// Playwright: `server/chromium/crPdf.ts` `convertPrintParameterToInches`: `return pixels / 96;`
const CSS_PIXELS_PER_INCH: f64 = 96.0;
/// Playwright: `server/chromium/crPdf.ts` `CRPDF.generate`: `'Unknown paper format: ' + options.format`
const UNKNOWN_PAPER_FORMAT: &str = "Unknown paper format: ";
/// Playwright: `server/chromium/crPdf.ts` `convertPrintParameterToInches`: `'Failed to parse parameter value: ' + text`
const UNPARSABLE_VALUE: &str = "Failed to parse parameter value: ";

/// `Page.printToPDF`'s params as Playwright's `CRPDF.generate` builds them, defaults filled.
#[derive(Debug, Clone, PartialEq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Playwright's boolean PDF options, one field each"
)]
pub(crate) struct PdfOptions {
    landscape: bool,
    display_header_footer: bool,
    header_template: String,
    footer_template: String,
    print_background: bool,
    scale: f64,
    paper_width: f64,
    paper_height: f64,
    margin_top: f64,
    margin_bottom: f64,
    margin_left: f64,
    margin_right: f64,
    page_ranges: String,
    prefer_css_page_size: bool,
    tagged: bool,
    outline: bool,
}

/// Playwright: `server/chromium/crPdf.ts` `CRPDF.generate`
impl PdfOptions {
    /// Playwright's `CRPDF.generate` up to its `Page.printToPDF`: the destructuring defaults,
    /// the paper size from `format` or `width`/`height`, the margins in inches.
    pub(crate) fn from_params(params: &PagePdfParams) -> Result<Self, PlaywrightError> {
        // Playwright: `server/chromium/crPdf.ts` `CRPDF.generate`: `if (options.format) {`
        let (paper_width, paper_height) = match &params.format {
            Some(format) if !format.is_empty() => paper_format(&format.to_lowercase())
                .ok_or_else(|| PlaywrightError::error(format!("{UNKNOWN_PAPER_FORMAT}{format}")))?,
            _ => (
                inches_or(params.width.as_deref(), DEFAULT_PAPER_WIDTH)?,
                inches_or(params.height.as_deref(), DEFAULT_PAPER_HEIGHT)?,
            ),
        };
        let margin = params.margin.as_ref();
        let side =
            |value: Option<&Option<String>>| inches_or(value.and_then(Option::as_deref), 0.0);
        Ok(Self {
            landscape: params.landscape.unwrap_or(false),
            display_header_footer: params.display_header_footer.unwrap_or(false),
            header_template: params.header_template.clone().unwrap_or_default(),
            footer_template: params.footer_template.clone().unwrap_or_default(),
            print_background: params.print_background.unwrap_or(false),
            scale: params.scale.unwrap_or(1.0),
            paper_width,
            paper_height,
            margin_top: side(margin.map(|margin| &margin.top))?,
            margin_bottom: side(margin.map(|margin| &margin.bottom))?,
            margin_left: side(margin.map(|margin| &margin.left))?,
            margin_right: side(margin.map(|margin| &margin.right))?,
            page_ranges: params.page_ranges.clone().unwrap_or_default(),
            prefer_css_page_size: params.prefer_css_page_size.unwrap_or(false),
            tagged: params.tagged.unwrap_or(false),
            outline: params.outline.unwrap_or(false),
        })
    }

    fn to_cdp(&self) -> Value {
        json!({
            "transferMode": "ReturnAsStream",
            "landscape": self.landscape,
            "displayHeaderFooter": self.display_header_footer,
            "headerTemplate": self.header_template,
            "footerTemplate": self.footer_template,
            "printBackground": self.print_background,
            "scale": self.scale,
            "paperWidth": self.paper_width,
            "paperHeight": self.paper_height,
            "marginTop": self.margin_top,
            "marginBottom": self.margin_bottom,
            "marginLeft": self.margin_left,
            "marginRight": self.margin_right,
            "pageRanges": self.page_ranges,
            "preferCSSPageSize": self.prefer_css_page_size,
            "generateTaggedPDF": self.tagged,
            "generateDocumentOutline": self.outline,
        })
    }
}

/// Playwright: `server/chromium/crPdf.ts` `PagePaperFormats`
fn paper_format(name: &str) -> Option<(f64, f64)> {
    Some(match name {
        "letter" => (8.5, 11.0),
        "legal" => (8.5, 14.0),
        "tabloid" => (11.0, 17.0),
        "ledger" => (17.0, 11.0),
        "a0" => (33.1, 46.8),
        "a1" => (23.4, 33.1),
        "a2" => (16.54, 23.4),
        "a3" => (11.7, 16.54),
        "a4" => (8.27, 11.7),
        "a5" => (5.83, 8.27),
        "a6" => (4.13, 5.83),
        _ => return None,
    })
}

/// Playwright: `server/chromium/crPdf.ts` `unitToPixels`
fn unit_to_pixels(unit: &str) -> Option<f64> {
    Some(match unit {
        "px" => 1.0,
        "in" => 96.0,
        "cm" => 37.8,
        "mm" => 3.78,
        _ => return None,
    })
}

/// Playwright's `convertPrintParameterToInches(text) || fallback`: absent, and also a value of
/// zero, take `fallback`.
/// Playwright: `server/chromium/crPdf.ts` `CRPDF.generate`: `const marginTop = convertPrintParameterToInches(margin.top) || 0;`
fn inches_or(text: Option<&str>, fallback: f64) -> Result<f64, PlaywrightError> {
    let Some(text) = text else {
        return Ok(fallback);
    };
    let inches = to_inches(text)?;
    Ok(if inches == 0.0 { fallback } else { inches })
}

/// Playwright: `server/chromium/crPdf.ts` `convertPrintParameterToInches`: `assert(!isNaN(value), 'Failed to parse parameter value: ' + text);`
fn to_inches(text: &str) -> Result<f64, PlaywrightError> {
    // `text.substring(text.length - 2)`, in UTF-16 units as JavaScript counts them.
    let units: Vec<u16> = text.encode_utf16().collect();
    let tail = String::from_utf16_lossy(&units[units.len().saturating_sub(2)..]).to_lowercase();
    let (pixels_per_unit, value_text) = match unit_to_pixels(&tail) {
        Some(pixels) => (
            pixels,
            String::from_utf16_lossy(&units[..units.len().saturating_sub(2)]),
        ),
        // In case of unknown unit try to parse the whole parameter as number of pixels.
        None => (1.0, text.to_string()),
    };
    let value = js_number(&value_text);
    if value.is_nan() {
        return Err(PlaywrightError::error(format!("{UNPARSABLE_VALUE}{text}")));
    }
    Ok(value * pixels_per_unit / CSS_PIXELS_PER_INCH)
}

/// Platform: JavaScript `Number(string)` (ECMA-262 StringToNumber): surrounding white space
/// ignored, empty is 0, `Infinity` with an optional sign, unsigned `0x`/`0o`/`0b` integers, and
/// decimal literals; anything else is NaN.
fn js_number(text: &str) -> f64 {
    let trimmed = text.trim_matches(progress::is_js_whitespace);
    if trimmed.is_empty() {
        return 0.0;
    }
    let (sign, unsigned) = match trimmed.as_bytes()[0] {
        b'+' => (1.0, &trimmed[1..]),
        b'-' => (-1.0, &trimmed[1..]),
        _ => (1.0, trimmed),
    };
    if unsigned == "Infinity" {
        return sign * f64::INFINITY;
    }
    let radix = match unsigned.get(..2) {
        Some("0x" | "0X") => Some(16),
        Some("0o" | "0O") => Some(8),
        Some("0b" | "0B") => Some(2),
        _ => None,
    };
    if let Some(radix) = radix {
        let digits = &unsigned[2..];
        // A sign before a radix prefix is NaN in JavaScript.
        if trimmed.len() != unsigned.len()
            || digits.is_empty()
            || !digits.chars().all(|digit| digit.is_digit(radix))
        {
            return f64::NAN;
        }
        return digits.chars().fold(0.0, |value, digit| {
            value * f64::from(radix) + f64::from(digit.to_digit(radix).unwrap_or(0))
        });
    }
    if !is_decimal_literal(unsigned) {
        return f64::NAN;
    }
    unsigned
        .parse::<f64>()
        .map_or(f64::NAN, |value| sign * value)
}

/// `StrUnsignedDecimalLiteral` without `Infinity`: digits with an optional fraction, or a
/// fraction alone, then an optional exponent.
/// Platform: ECMA-262 `StringNumericLiteral` grammar, `StrUnsignedDecimalLiteral`.
fn is_decimal_literal(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut at = 0;
    let digits = |at: &mut usize| {
        let start = *at;
        while bytes.get(*at).is_some_and(u8::is_ascii_digit) {
            *at += 1;
        }
        *at - start
    };
    let whole = digits(&mut at);
    let mut fraction = 0;
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        fraction = digits(&mut at);
    }
    if whole == 0 && fraction == 0 {
        return false;
    }
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        at += 1;
        if matches!(bytes.get(at), Some(b'+' | b'-')) {
            at += 1;
        }
        if digits(&mut at) == 0 {
            return false;
        }
    }
    at == bytes.len()
}

/// Neo: how long an abandoned stream's `IO.close` may hold the run's tracker, and with it a
/// host's `Runtime::drain`: the 5 s the run's other browser cleanups get (`Tabs::finish_close`,
/// `Tabs::close_failed_creation`). The browser serves `IO` itself, so the close is answered
/// after the tab's renderer is gone too; this bounds only a browser that stopped answering.
const ABANDONED_STREAM_CLOSE: Duration = Duration::from_secs(5);

/// `IO.read`'s answer.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IoChunk {
    #[serde(default)]
    base64_encoded: bool,
    data: String,
    eof: bool,
}

/// Serves `Page.pdf`: the document's PDF bytes.
/// Playwright: `server/chromium/crPdf.ts` `CRPDF.generate`: `return await readProtocolStream(this._client, result.stream!);`
pub(crate) async fn pdf(
    page: &Arc<PageSession>,
    params: &PagePdfParams,
    progress: &Progress,
) -> Result<Vec<u8>, CallError> {
    let options = PdfOptions::from_params(params)?;
    // The print runs on the run's tracker, not in this call: a call that stops before the answer
    // still gets its stream closed when the answer arrives (`Stream`'s `Drop`), however late.
    // Not only while the tab is open (`PageSession::spawn_while_open`): a stream outlives its
    // tab's renderer while any debugger session stays on the tab (measured on BrowserOS neo,
    // `capture-pdf-crash-review`), so an answer after a crash still has a stream to close. The
    // wait is bounded by browseros-cdp's request timeout, as any command's.
    let (sent, printed) = oneshot::channel();
    let session = Arc::clone(page);
    page.spawn(async move {
        let answer = session.send("Page.printToPDF", options.to_cdp()).await;
        let stream = match answer {
            Ok(answer) => match answer.get("stream").and_then(Value::as_str) {
                Some(handle) => Ok(Stream {
                    page: Arc::clone(&session),
                    handle: handle.to_string(),
                    closed: false,
                }),
                None => Err(capture::malformed("Page.printToPDF", "no stream")),
            },
            Err(error) => Err(capture::protocol_error("Page.printToPDF", &error)),
        };
        // A call that stopped waiting drops the stream here, which closes it.
        let _unread = sent.send(stream);
    });
    let stream = progress
        .race(printed)
        .await?
        .map_err(|_| PlaywrightError::target_closed())??;
    stream.read(progress).await
}

/// An open `IO` stream. Read to its end, then closed; dropped anywhere else (the call stopped
/// before, while or after reading it, before its close was answered), it is closed on the run's
/// tracker, because Chromium keeps a stream after the session that made it goes away.
struct Stream {
    page: Arc<PageSession>,
    handle: String,
    closed: bool,
}

/// Playwright: `server/chromium/crProtocolHelper.ts` `readProtocolStream`
impl Stream {
    /// Playwright: `server/chromium/crProtocolHelper.ts` `readProtocolStream`: `const buf = Buffer.from(response.data, response.base64Encoded ? 'base64' : undefined);`
    async fn read(mut self, progress: &Progress) -> Result<Vec<u8>, CallError> {
        let mut bytes = Vec::new();
        loop {
            let reply = progress
                .race(self.page.send("IO.read", json!({ "handle": self.handle })))
                .await?
                .map_err(|error| capture::protocol_error("IO.read", &error))?;
            let chunk: IoChunk = serde_json::from_value(reply)
                .map_err(|error| capture::malformed("IO.read", error))?;
            if chunk.base64_encoded {
                let decoded = STANDARD
                    .decode(chunk.data.as_bytes())
                    .map_err(|error| capture::malformed("IO.read", error))?;
                bytes.extend_from_slice(&decoded);
            } else {
                bytes.extend_from_slice(chunk.data.as_bytes());
            }
            if chunk.eof {
                break;
            }
        }
        // A call that stops before the close is answered leaves the stream to `Drop`, which
        // sends the close again: it may never have gone out.
        let closed = progress
            .race(self.page.send("IO.close", json!({ "handle": self.handle })))
            .await?;
        self.closed = true;
        if let Err(error) = closed {
            debug!(%error, "closing a PDF stream failed");
        }
        Ok(bytes)
    }
}

/// Neo: a stream the call stopped reading is closed in the background, whether or not its tab
/// is still there, within [`ABANDONED_STREAM_CLOSE`].
impl Drop for Stream {
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        let page = Arc::clone(&self.page);
        let handle = self.handle.clone();
        self.page.spawn(async move {
            let close = page.send("IO.close", json!({ "handle": handle }));
            match time::timeout(ABANDONED_STREAM_CLOSE, close).await {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => debug!(%error, "closing an abandoned PDF stream failed"),
                Err(_) => debug!("closing an abandoned PDF stream went unanswered"),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    use neo_runtime_spec::serve::PagePdfParamsMargin;
    use tokio::runtime::Handle;
    use tokio::sync::Mutex;
    use tokio_util::sync::CancellationToken;

    use crate::chromium_bridge::cdp;
    use tokio_util::task::TaskTracker;

    use super::*;
    use crate::browser_representations::page::{PageSetup, Session};
    use crate::events::EventHub;
    use crate::fake_browser::{FakeBrowser, PAGE, Replies};
    use crate::run::PageId;

    /// A session on the fake's tab, with its own tracker and closed token.
    async fn tracked_page(
        browser: &FakeBrowser,
        tasks: &TaskTracker,
        closed: &CancellationToken,
    ) -> Arc<PageSession> {
        let id = PageId::new(PAGE.to_string());
        let session = Session::attach(
            cdp::Browser::new(browser.cdp().await),
            id.clone(),
            Handle::current(),
        )
        .await
        .expect("attach");
        Arc::new(PageSession::new(
            session,
            &id,
            PageSetup {
                tasks: tasks.clone(),
                input: Arc::default(),
                screenshots: Arc::new(Mutex::new(())),
                closed: closed.clone(),
                hub: EventHub::default(),
                test_id: Arc::default(),
                context_routes: Arc::default(),
                discovered: false,
            },
        ))
    }

    /// The handles of the `IO.close` commands the fake received.
    fn closes(browser: &FakeBrowser) -> Vec<Value> {
        browser
            .exchanges()
            .iter()
            .filter(|exchange| exchange.method() == "IO.close")
            .map(|exchange| exchange.command["params"]["handle"].clone())
            .collect()
    }

    /// A stream dropped after its tab closed or crashed is still closed: the browser keeps it
    /// while any debugger session stays on the tab, and serves `IO.close` without the renderer.
    #[tokio::test]
    async fn tests_that_an_abandoned_stream_is_closed_after_its_tab_went() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let tasks = TaskTracker::new();
        let closed = CancellationToken::new();
        let page = tracked_page(&browser, &tasks, &closed).await;
        closed.cancel();
        drop(Stream {
            page: Arc::clone(&page),
            handle: "stream-1".to_string(),
            closed: false,
        });
        tasks.close();
        time::timeout(Duration::from_secs(5), tasks.wait())
            .await
            .expect("the close settles");
        assert_eq!(closes(&browser), vec![json!("stream-1")]);
    }

    /// A read that fails leaves its stream to `Drop`, which closes it once.
    #[tokio::test]
    async fn tests_that_a_stream_whose_read_failed_is_closed() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let tasks = TaskTracker::new();
        let closed = CancellationToken::new();
        let page = tracked_page(&browser, &tasks, &closed).await;
        let stream = Stream {
            page: Arc::clone(&page),
            handle: "stream-2".to_string(),
            closed: false,
        };
        // The fake knows no `IO.read`, so the first read fails.
        let read = stream.read(&Progress::for_test(5_000.0)).await;
        assert!(read.is_err(), "{read:?}");
        tasks.close();
        time::timeout(Duration::from_secs(5), tasks.wait())
            .await
            .expect("the close settles");
        assert_eq!(closes(&browser), vec![json!("stream-2")]);
    }

    fn params(edit: impl FnOnce(&mut PagePdfParams)) -> PagePdfParams {
        let mut params: PagePdfParams = serde_json::from_value(json!({})).expect("no options");
        edit(&mut params);
        params
    }

    #[test]
    fn tests_that_defaults_are_playwrights() {
        let options = PdfOptions::from_params(&params(|_| {})).expect("defaults");
        assert_eq!(
            options.to_cdp(),
            json!({
                "transferMode": "ReturnAsStream", "landscape": false,
                "displayHeaderFooter": false, "headerTemplate": "", "footerTemplate": "",
                "printBackground": false, "scale": 1.0, "paperWidth": 8.5, "paperHeight": 11.0,
                "marginTop": 0.0, "marginBottom": 0.0, "marginLeft": 0.0, "marginRight": 0.0,
                "pageRanges": "", "preferCSSPageSize": false, "generateTaggedPDF": false,
                "generateDocumentOutline": false,
            })
        );
    }

    #[test]
    fn tests_that_format_wins_over_width_and_is_case_insensitive() {
        let options = PdfOptions::from_params(&params(|params| {
            params.format = Some("A4".to_string());
            params.width = Some("100px".to_string());
        }))
        .expect("a4");
        assert_eq!((options.paper_width, options.paper_height), (8.27, 11.7));
        let error = PdfOptions::from_params(&params(|params| {
            params.format = Some("A7".to_string());
        }))
        .expect_err("unknown");
        assert_eq!(error.message, "Unknown paper format: A7");
    }

    #[test]
    fn tests_that_units_convert_as_playwright_converts_them() {
        let cases = [
            ("96px", 1.0),
            ("2in", 2.0),
            ("2IN", 2.0),
            ("2.54cm", 2.54 * 37.8 / 96.0),
            ("10mm", 10.0 * 3.78 / 96.0),
            ("192", 2.0),
            (" 192 ", 2.0),
            ("0x60", 1.0),
            ("1e2px", 100.0 / 96.0),
        ];
        for (text, inches) in cases {
            let converted = to_inches(text).expect(text);
            assert!((converted - inches).abs() < 1e-9, "{text}: {converted}");
        }
        for text in ["abc", "10pt", "1,5in", "-0x10", "px1"] {
            let error = to_inches(text).expect_err(text);
            assert_eq!(
                error.message,
                format!("Failed to parse parameter value: {text}")
            );
        }
    }

    #[test]
    fn tests_that_zero_sizes_and_margins_fall_back_as_in_playwright() {
        let options = PdfOptions::from_params(&params(|params| {
            params.width = Some("0".to_string());
            params.height = Some("480px".to_string());
            params.margin = Some(PagePdfParamsMargin {
                top: Some("1in".to_string()),
                bottom: None,
                left: Some("0px".to_string()),
                right: Some("48px".to_string()),
            });
        }))
        .expect("sizes");
        assert_eq!(
            (options.paper_width, options.paper_height),
            (DEFAULT_PAPER_WIDTH, 5.0)
        );
        assert_eq!(
            (
                options.margin_top,
                options.margin_bottom,
                options.margin_left,
                options.margin_right
            ),
            (1.0, 0.0, 0.0, 0.5)
        );
    }

    #[test]
    fn tests_that_js_number_follows_string_to_number() {
        for (text, value) in [
            ("", 0.0_f64),
            ("  ", 0.0),
            ("12", 12.0),
            ("+1.5", 1.5),
            ("-.5", -0.5),
            ("5.", 5.0),
            ("1E3", 1000.0),
            ("0b101", 5.0),
            ("0o17", 15.0),
        ] {
            assert_eq!(js_number(text).to_bits(), value.to_bits(), "{text:?}");
        }
        assert_eq!(
            js_number("-Infinity").to_bits(),
            f64::NEG_INFINITY.to_bits()
        );
        for text in ["inf", "NaN", "1_000", ".", "e5", "1e", "0x", "12abc", "+-1"] {
            assert!(js_number(text).is_nan(), "{text:?}");
        }
    }

    /// The paper and unit tables are copies of Playwright's, so this compares them with the
    /// vendored source line by line.
    #[test]
    fn tests_that_the_paper_and_unit_tables_match_playwright() {
        let path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets it"))
            .join("vendor/playwright/packages/playwright-core/src/server/chromium/crPdf.ts");
        let source = fs::read_to_string(&path).expect("crPdf.ts is vendored");
        let mut formats = 0;
        let mut units = 0;
        for line in source.lines().map(str::trim) {
            // The table's rows, not its type annotation: `letter: { width: 8.5, height: 11 },`.
            if let Some((name, size)) = line
                .split_once(": { width: ")
                .filter(|(name, _)| name.chars().all(|c| c.is_ascii_alphanumeric()))
            {
                let (width, height) = size
                    .trim_end_matches(" },")
                    .split_once(", height: ")
                    .expect("width and height");
                let expected = (
                    width.parse::<f64>().expect("width"),
                    height.parse::<f64>().expect("height"),
                );
                assert_eq!(paper_format(name), Some(expected), "{line}");
                formats += 1;
            } else if let Some((unit, pixels)) = line
                .strip_prefix('\'')
                .and_then(|line| line.split_once("': "))
            {
                let pixels = pixels.trim_end_matches(',').parse::<f64>().expect(line);
                assert_eq!(unit_to_pixels(unit), Some(pixels), "{line}");
                units += 1;
            }
        }
        assert_eq!((formats, units), (11, 4), "every table row was compared");
    }
}
