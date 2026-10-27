//! Transport-independent payment card operations. Native owns card storage,
//! validation and form filling; adapters receive card metadata and acknowledged
//! fills, never a card number or security code. Requests that may hold card
//! data intentionally have no Debug implementation.
use crate::BrowserProfileId;
use serde::{Deserialize, Serialize};

/// Saved card metadata. Never a number or security code.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PaymentCardInfo {
    pub card_id: String,
    pub network: String,
    pub last_four: String,
    pub expiry_month: u32,
    pub expiry_year: u32,
    pub expired: bool,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    /// `local`, or `google_pay` for a masked Google Pay card.
    pub kind: String,
    pub cvc_saved: bool,
}

/// Card data supplied by the caller, typically swapped in for vault tokens by a
/// partner's proxy. Sent to native once; never echoed, logged or formatted.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuppliedCard {
    pub number: String,
    pub expiry_month: u32,
    pub expiry_year: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cvc: Option<String>,
}

pub enum PaymentCardSource {
    Saved { card_id: String },
    Supplied(SuppliedCard),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PaymentCardInventory {
    pub browser_profile_id: BrowserProfileId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    pub cards: Vec<PaymentCardInfo>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PaymentCardAddOutcome {
    pub browser_profile_id: BrowserProfileId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    pub status: String,
    pub card: PaymentCardInfo,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PaymentCardFillOutcome {
    pub browser_profile_id: BrowserProfileId,
    pub origin: String,
    pub status: String,
    pub source: String,
    pub network: String,
    pub last_four: String,
    /// Card field kinds filled: number, name, expiry, cvc.
    pub filled_fields: Vec<String>,
    /// Card field kinds the form has but the fill left empty, typically cvc.
    pub missing_fields: Vec<String>,
}

/// A fixed diagnostic vocabulary: native/transport exception text and card data
/// are never reflected into a tool result or server diagnostic.
#[derive(Clone, Debug, Serialize)]
pub struct PaymentCardFailure {
    pub code: &'static str,
    pub message: &'static str,
}
impl std::fmt::Display for PaymentCardFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message)
    }
}
impl std::error::Error for PaymentCardFailure {}
impl PaymentCardFailure {
    #[must_use]
    pub fn new(code: &str) -> Self {
        let (code, message) = match code {
            "not_supported" => (
                "not_supported",
                "Payment cards require a newer native browser.",
            ),
            "page_out_of_scope" => (
                "page_out_of_scope",
                "This page is outside the selected browser target.",
            ),
            "page_changed" => (
                "page_changed",
                "The page changed. Find the current page and retry.",
            ),
            "operation_not_allowed" => (
                "operation_not_allowed",
                "The selected profile, page or payment settings do not allow this operation.",
            ),
            "card_not_found" => (
                "card_not_found",
                "Card not found in this profile. List cards and retry.",
            ),
            "card_expired" => ("card_expired", "This saved card has expired."),
            "invalid_card" => (
                "invalid_card",
                "The card number, expiry or security code is not valid.",
            ),
            "card_conflict" => (
                "card_conflict",
                "A saved card has this number with different details.",
            ),
            "verification_required" => (
                "verification_required",
                "The browser must verify this Google Pay card with the user. Ask for help to complete payment.",
            ),
            "reauth_required" => (
                "reauth_required",
                "Device verification is required to fill saved cards. Ask for help to complete payment.",
            ),
            "no_payment_form" => ("no_payment_form", "No card form was found on this page."),
            "ambiguous_payment_form" => (
                "ambiguous_payment_form",
                "This page has more than one card form.",
            ),
            "no_fillable_fields" => ("no_fillable_fields", "No card fields acknowledged filling."),
            "form_already_filled" => (
                "form_already_filled",
                "This card form still holds an earlier fill. Reload the page or clear its card fields, then fill again.",
            ),
            "busy" => (
                "busy",
                "Another payment card operation is in progress. Retry after it completes.",
            ),
            "timeout" => ("timeout", "The payment card operation timed out."),
            "operation_cancelled" => (
                "operation_cancelled",
                "The payment card operation was cancelled.",
            ),
            "save_outcome_unknown" => (
                "save_outcome_unknown",
                "The save outcome is unknown. List cards before retrying.",
            ),
            "store_unavailable" => (
                "store_unavailable",
                "The native payment data is unavailable.",
            ),
            _ => ("store_error", "The native payment card operation failed."),
        };
        Self { code, message }
    }
}
