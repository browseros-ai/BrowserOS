//! `page.pdf()`. The paper formats and the unit conversion
//! follow Playwright's `server/chromium/crPdf.ts` at v1.64.1 (Apache-2.0, Copyright Microsoft
//! Corporation), so a size means what it means upstream.

use browseros_cdp::CdpError;
use browseros_core::CoreError;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::op_output::Bytes;
use crate::op_scope::OpScope;
use crate::page::document_watch::decode;
use crate::target::PageRef;
use crate::{ErrorCode, Playwright, PlaywrightError};
use ts_rs::TS;

/// `page.pdf(options)` with CSS lengths normalized by the client; Neo returns bytes inline.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct PdfInput {
    page: PageRef,
    #[serde(default)]
    #[ts(optional)]
    scale: Option<f64>,
    #[serde(default)]
    #[ts(optional)]
    display_header_footer: Option<bool>,
    #[serde(default)]
    #[ts(optional)]
    header_template: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    footer_template: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    print_background: Option<bool>,
    #[serde(default)]
    #[ts(optional)]
    landscape: Option<bool>,
    #[serde(default)]
    #[ts(optional)]
    page_ranges: Option<String>,
    /// `Letter`, `A4`, ... (case-insensitive); wins over `width` and `height`.
    #[serde(default)]
    #[ts(optional)]
    format: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    width: Option<Length>,
    #[serde(default)]
    #[ts(optional)]
    height: Option<Length>,
    #[serde(default, rename = "preferCSSPageSize")]
    #[ts(optional)]
    prefer_css_page_size: Option<bool>,
    #[serde(default)]
    #[ts(optional, as = "Option<Margin>")]
    margin: Margin,
    #[serde(default)]
    #[ts(optional)]
    tagged: Option<bool>,
    #[serde(default)]
    #[ts(optional)]
    outline: Option<bool>,
}

crate::wire::options!(PdfInput, flat);

/// The channel accepts only CSS length strings. The prelude converts numeric script
/// options to `px` before the call, as upstream's client does.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(untagged)]
// Retain the untagged decoder so invalid non-length script values keep their existing error.
enum Length {
    Text(String),
}

/// Page margins; each side defaults to 0.
#[derive(Clone, Debug, Default, Deserialize, TS)]
#[serde(deny_unknown_fields)]
struct Margin {
    #[serde(default)]
    #[ts(optional)]
    top: Option<Length>,
    #[serde(default)]
    #[ts(optional)]
    right: Option<Length>,
    #[serde(default)]
    #[ts(optional)]
    bottom: Option<Length>,
    #[serde(default)]
    #[ts(optional)]
    left: Option<Length>,
}

/// Paper sizes in inches, as upstream names them (lower case).
const PAPER_FORMATS: [(&str, f64, f64); 11] = [
    ("letter", 8.5, 11.0),
    ("legal", 8.5, 14.0),
    ("tabloid", 11.0, 17.0),
    ("ledger", 17.0, 11.0),
    ("a0", 33.1, 46.8),
    ("a1", 23.4, 33.1),
    ("a2", 16.54, 23.4),
    ("a3", 11.7, 16.54),
    ("a4", 8.27, 11.7),
    ("a5", 5.83, 8.27),
    ("a6", 4.13, 5.83),
];

/// CSS pixels per unit.
const UNIT_PIXELS: [(&str, f64); 4] = [("px", 1.0), ("in", 96.0), ("cm", 37.8), ("mm", 3.78)];

impl Length {
    /// Upstream's `convertPrintParameterToInches`.
    fn inches(&self) -> Result<f64, String> {
        let Self::Text(text) = self;
        let unit_at = text.len().saturating_sub(2);
        let unit = text.get(unit_at..).map(str::to_ascii_lowercase);
        let known = UNIT_PIXELS
            .iter()
            .find(|(name, _)| unit.as_deref() == Some(*name));
        let (number, per_unit) = match known {
            Some((_, per_unit)) => (text.get(..unit_at).unwrap_or_default(), *per_unit),
            None => (text.as_str(), 1.0),
        };
        let value: f64 = number
            .trim()
            .parse()
            .map_err(|_error| format!("Failed to parse parameter value: {text}"))?;
        let pixels = value * per_unit;
        Ok(pixels / 96.0)
    }
}

impl PdfInput {
    /// `Page.printToPDF`'s parameters, with upstream's defaults.
    fn params(&self) -> Result<Value, String> {
        let (paper_width, paper_height) = match &self.format {
            Some(format) => PAPER_FORMATS
                .iter()
                .find(|(name, ..)| name.eq_ignore_ascii_case(format))
                .map(|(_, width, height)| (*width, *height))
                .ok_or_else(|| format!("Unknown paper format: {format}"))?,
            None => (
                non_zero(self.width.as_ref())?.unwrap_or(8.5),
                non_zero(self.height.as_ref())?.unwrap_or(11.0),
            ),
        };
        let side = |length: Option<&Length>| non_zero(length).map(|inches| inches.unwrap_or(0.0));
        Ok(json!({
            "landscape": self.landscape.unwrap_or(false),
            "displayHeaderFooter": self.display_header_footer.unwrap_or(false),
            "headerTemplate": self.header_template.as_deref().unwrap_or_default(),
            "footerTemplate": self.footer_template.as_deref().unwrap_or_default(),
            "printBackground": self.print_background.unwrap_or(false),
            "scale": self.scale.unwrap_or(1.0),
            "paperWidth": paper_width,
            "paperHeight": paper_height,
            "marginTop": side(self.margin.top.as_ref())?,
            "marginBottom": side(self.margin.bottom.as_ref())?,
            "marginLeft": side(self.margin.left.as_ref())?,
            "marginRight": side(self.margin.right.as_ref())?,
            "pageRanges": self.page_ranges.as_deref().unwrap_or_default(),
            "preferCSSPageSize": self.prefer_css_page_size.unwrap_or(false),
            "generateTaggedPDF": self.tagged.unwrap_or(false),
            "generateDocumentOutline": self.outline.unwrap_or(false),
        }))
    }
}

/// A length in inches; upstream treats 0 like an absent value (`|| default`).
fn non_zero(length: Option<&Length>) -> Result<Option<f64>, String> {
    Ok(length
        .map(Length::inches)
        .transpose()?
        .filter(|inches| *inches != 0.0))
}

/// `Page.printToPDF`'s reply, the PDF inline as base64.
#[derive(Deserialize)]
struct PrintReply {
    data: String,
}

/// Prints the page as `input` asks.
pub(crate) async fn print(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: PdfInput,
) -> Result<Bytes, PlaywrightError> {
    let params = input
        .params()
        .map_err(|message| scope.fail(ErrorCode::InvalidInput, &message))?;
    let page = input.page.tab();
    let admitted = scope.admit(pw.browser(), page).await?;
    let session = pw
        .browser()
        .protocol_session(admitted.session.session_id.clone());
    let reply: PrintReply = session
        .send_value("Page.printToPDF", params)
        .await
        .and_then(decode)
        .map_err(|error| match &error {
            // The browser itself refused to print (headed Chrome cannot): not Neo's bug.
            CoreError::Cdp(CdpError::Protocol { message, .. }) => scope.fail(
                ErrorCode::Unsupported,
                &format!("printing to PDF is not available in this browser ({message})"),
            ),
            _ => scope.core_error(&error),
        })?;
    Ok(Bytes::from_base64(reply.data))
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use serde_json::json;

    use super::PdfInput;

    fn params(options: &serde_json::Value) -> Result<serde_json::Value, Box<dyn Error>> {
        let mut input = json!({ "page": { "$neo": "page", "tab": 1 } });
        if let (Some(input), Some(options)) = (input.as_object_mut(), options.as_object()) {
            input.extend(options.clone());
        }
        let input: PdfInput = serde_json::from_value(input)?;
        Ok(input.params()?)
    }

    #[test]
    fn tests_that_the_defaults_are_upstreams_letter_page() -> Result<(), Box<dyn Error>> {
        let sent = params(&json!({}))?;
        assert_eq!(sent["paperWidth"], json!(8.5));
        assert_eq!(sent["paperHeight"], json!(11.0));
        assert_eq!(sent["marginTop"], json!(0.0));
        assert_eq!(sent["scale"], json!(1.0));
        assert_eq!(sent["printBackground"], json!(false));
        Ok(())
    }

    #[test]
    fn tests_that_units_convert_to_inches_as_upstream_does() -> Result<(), Box<dyn Error>> {
        let sent = params(&json!({
            "width": "960px", "height": "10in",
            "margin": { "top": "2.54cm", "left": "48px", "right": "25.4mm", "bottom": "96" },
        }))?;
        assert_eq!(sent["paperWidth"], json!(10.0));
        assert_eq!(sent["paperHeight"], json!(10.0));
        assert_eq!(
            sent["marginLeft"],
            json!(0.5),
            "the client normalizes numeric pixels"
        );
        assert_eq!(
            sent["marginBottom"],
            json!(1.0),
            "text without a unit is pixels"
        );
        let top = sent["marginTop"].as_f64().ok_or("a number")?;
        assert!(
            (top - 2.54 * 37.8 / 96.0).abs() < 1e-9,
            "cm, by upstream's 37.8 px/cm"
        );
        Ok(())
    }

    #[test]
    fn tests_that_the_channel_requires_client_normalized_lengths() {
        for dimension in ["width", "height"] {
            assert!(params(&json!({ dimension: 96 })).is_err());
        }
        for side in ["top", "right", "bottom", "left"] {
            assert!(params(&json!({ "margin": { side: 96 } })).is_err());
        }
    }

    #[test]
    fn tests_that_a_format_wins_and_bad_values_are_named() -> Result<(), Box<dyn Error>> {
        let sent = params(&json!({ "format": "A4", "width": "1in" }))?;
        assert_eq!(sent["paperWidth"], json!(8.27));
        let unknown = params(&json!({ "format": "A9" }))
            .err()
            .map(|error| error.to_string());
        assert_eq!(unknown.as_deref(), Some("Unknown paper format: A9"));
        let bad = params(&json!({ "width": "wide" }))
            .err()
            .map(|error| error.to_string());
        assert_eq!(
            bad.as_deref(),
            Some("Failed to parse parameter value: wide")
        );
        Ok(())
    }
}
