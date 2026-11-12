//! Agent-facing payment card contract. Native/core own all card semantics;
//! this adapter validates action combinations and formats metadata-only results.
#![allow(dead_code)] // Output-only schema types describe the wire contract.

use crate::{
    framework::{ArgIssue, ToolCtx, ToolError, ToolExecResult, ToolResult},
    response::ToolResponse,
};
use browseros_core::{
    PageId,
    payment_cards::{PaymentCardInfo, PaymentCardSource, SuppliedCard},
};
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

const DESCRIPTION: &str = "List, add, or fill payment cards in the selected browser profile. list returns saved card metadata (network, last four digits, expiry, name); card numbers and security codes never appear in results. add saves a supplied card; identical adds are unchanged and the same number with different details conflicts. fill accepts exactly one source: cardId from list/add, or a supplied card, which is filled but not saved. Filling targets the page's one card form, does not submit or pay, and reports missingFields (often cvc) so you can ask the user to finish. A form that still holds an earlier fill is refused with form_already_filled: reload the page first. Saved cards keep the browser's payment checks: verification_required and reauth_required mean the user must complete payment in the browser. Use tabs to find page ids. In run, use browser.paymentCards(options) or browser.page(id).paymentCards(options).";

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Action {
    List,
    Add,
    Fill,
}
impl Action {
    fn name(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Add => "add",
            Self::Fill => "fill",
        }
    }
}

// No Debug: error and tracing paths must not accidentally format card data.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CardArgs {
    /// Card number; spaces and dashes are ignored. Sensitive, never returned.
    number: String,
    #[schemars(range(min = 1, max = 12))]
    expiry_month: u32,
    /// Four-digit year.
    expiry_year: u32,
    /// Name on the card.
    name: Option<String>,
    /// Security code. Sensitive, never returned.
    cvc: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Args {
    action: Action,
    /// Page id from tabs. Required for fill; for list and add it only selects
    /// the page's profile (the bound profile, or Default, when omitted).
    #[schemars(range(min = 1))]
    page: Option<u32>,
    /// fill only: a saved card from list/add. Cannot be combined with card.
    card_id: Option<String>,
    /// add, or fill without saving. Cannot be combined with cardId.
    card: Option<CardArgs>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Card {
    card_id: String,
    network: String,
    last_four: String,
    expiry_month: u32,
    expiry_year: u32,
    expired: bool,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    nickname: Option<String>,
    kind: CardKind,
    cvc_saved: bool,
}
#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
enum CardKind {
    Local,
    /// A masked Google Pay card. Filling it needs the user's verification.
    GooglePay,
}
#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
enum AddStatus {
    Saved,
    Unchanged,
}
#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
enum FilledStatus {
    Filled,
}
#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
enum FieldKind {
    Number,
    Name,
    Expiry,
    Cvc,
}
#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
enum FillSource {
    Saved,
    Supplied,
}
#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ErrorStatus {
    Error,
}
#[derive(JsonSchema)]
struct Failure {
    code: String,
    message: String,
}

#[derive(JsonSchema)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum OutputVariants {
    List {
        browser_profile_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        page: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        origin: Option<String>,
        cards: Vec<Card>,
    },
    Add {
        status: AddStatus,
        browser_profile_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        page: Option<u32>,
        card: Card,
    },
    Fill {
        status: FilledStatus,
        browser_profile_id: String,
        page: u32,
        origin: String,
        source: FillSource,
        #[serde(skip_serializing_if = "Option::is_none")]
        card_id: Option<String>,
        network: String,
        last_four: String,
        filled_fields: Vec<FieldKind>,
        missing_fields: Vec<FieldKind>,
    },
    #[serde(untagged)]
    Error {
        action: Action,
        status: ErrorStatus,
        error: Failure,
    },
}

struct Output;
impl JsonSchema for Output {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "PaymentCardsOutput".into()
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let mut schema = OutputVariants::json_schema(generator);
        // MCP requires an explicit object root, including for tagged unions.
        schema.insert("type".into(), json!("object"));
        schema
    }
}

pub fn definition() -> crate::framework::ToolDef {
    super::def_with_output::<Args, Output>(
        "payment_cards",
        DESCRIPTION,
        Some(super::open_world_annotations()),
        handler,
    )
}

fn invalid(message: &str) -> ToolError {
    ToolError::InvalidArguments(vec![ArgIssue {
        path: String::new(),
        message: message.into(),
    }])
}
/// A validated request: each action with exactly the inputs it accepts.
enum Request {
    List {
        page: Option<u32>,
    },
    Add {
        page: Option<u32>,
        card: SuppliedCard,
    },
    Fill {
        page: u32,
        card_id: Option<String>,
        source: PaymentCardSource,
    },
}
impl Request {
    fn action(&self) -> &'static str {
        match self {
            Self::List { .. } => Action::List.name(),
            Self::Add { .. } => Action::Add.name(),
            Self::Fill { .. } => Action::Fill.name(),
        }
    }
}

fn parse(raw: Value) -> ToolExecResult<Request> {
    // Serde diagnostics may quote unexpected values. All malformed card
    // requests use fixed messages, including early errors.
    let obj = raw
        .as_object()
        .ok_or_else(|| invalid("Expected payment card arguments."))?;
    if obj.values().any(Value::is_null) {
        return Err(invalid("Omit optional fields instead of passing null."));
    }
    let args: Args = serde_json::from_value(raw).map_err(|_| {
        invalid("Check action and card field types; unknown fields are not accepted.")
    })?;
    if args.page == Some(0) {
        return Err(invalid("page must be a positive page id from tabs."));
    }
    match (args.action, args.page, args.card_id, args.card) {
        (Action::List, page, None, None) => Ok(Request::List { page }),
        (Action::List, ..) => Err(invalid("list accepts only page.")),
        (Action::Add, page, None, Some(card)) => Ok(Request::Add {
            page,
            card: supplied(card),
        }),
        (Action::Add, ..) => Err(invalid("add requires card and no cardId.")),
        (Action::Fill, Some(page), Some(card_id), None) if !card_id.is_empty() => {
            Ok(Request::Fill {
                page,
                card_id: Some(card_id.clone()),
                source: PaymentCardSource::Saved { card_id },
            })
        }
        (Action::Fill, Some(page), None, Some(card)) => Ok(Request::Fill {
            page,
            card_id: None,
            source: PaymentCardSource::Supplied(supplied(card)),
        }),
        (Action::Fill, ..) => Err(invalid(
            "fill requires page and exactly one source: cardId, or card.",
        )),
    }
}
fn supplied(card: CardArgs) -> SuppliedCard {
    SuppliedCard {
        number: card.number,
        expiry_month: card.expiry_month,
        expiry_year: card.expiry_year,
        name: card.name,
        cvc: card.cvc,
    }
}
fn escaped(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into())
}
fn summary(card: &PaymentCardInfo) -> String {
    format!(
        "{} — {} •••• {}, expires {:02}/{}{}{}",
        card.card_id,
        card.network,
        card.last_four,
        card.expiry_month,
        card.expiry_year,
        if card.expired { " (expired)" } else { "" },
        if card.name.is_empty() {
            String::new()
        } else {
            format!(", {}", escaped(&card.name))
        }
    )
}

fn handler<'a>(
    raw: Value,
    ctx: &'a ToolCtx,
    _response: &'a mut ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(async move {
        let request = parse(raw)?;
        let action = request.action();
        let result = tokio::select! {
            biased;
            () = ctx.cancel.cancelled() => return Err(ToolError::Cancelled),
            result = async { match request {
            Request::List { page } => ctx.browser.list_payment_cards(page.map(PageId)).await.map(|inventory| {
                let text = if inventory.cards.is_empty() {
                    "No saved cards found.".to_owned()
                } else {
                    std::iter::once(format!("{} saved card(s)", inventory.cards.len()))
                        .chain(inventory.cards.iter().map(summary))
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                let mut data = serde_json::to_value(inventory).unwrap_or(json!({}));
                data["action"] = json!(action);
                if let Some(page) = page { data["page"] = json!(page); }
                ToolResult::text(text, Some(data))
            }),
            Request::Add { page, card } => ctx.browser.add_payment_card(page.map(PageId), card).await.map(|outcome| {
                let text = format!("{} {}.", if outcome.status == "saved" { "Saved" } else { "Already saved" }, summary(&outcome.card));
                let mut data = json!({"action":action,"status":outcome.status,"browserProfileId":outcome.browser_profile_id,"card":outcome.card});
                if let Some(page) = page { data["page"] = json!(page); }
                ToolResult::text(text, Some(data))
            }),
            Request::Fill { page, card_id, source } => ctx.browser.fill_payment_card(PageId(page), source).await.map(|outcome| {
                let from = card_id.as_deref().map_or_else(
                    || format!("a supplied {} •••• {}", outcome.network, outcome.last_four),
                    |id| format!("{id} ({} •••• {})", outcome.network, outcome.last_four),
                );
                let mut text = format!("Filled {} on page {page} from {from}. Form not submitted.", outcome.filled_fields.join(", "));
                if !outcome.missing_fields.is_empty() {
                    text.push_str(&format!(" Still empty: {}.", outcome.missing_fields.join(", ")));
                }
                let mut data = json!({"action":action,"status":outcome.status,"browserProfileId":outcome.browser_profile_id,"page":page,"origin":outcome.origin,"source":outcome.source,"network":outcome.network,"lastFour":outcome.last_four,"filledFields":outcome.filled_fields,"missingFields":outcome.missing_fields});
                if let Some(id) = card_id { data["cardId"] = json!(id); }
                ToolResult::text(text, Some(data))
            }),
            } } => result,
        };
        Ok(Some(match result {
            Ok(result) => result,
            Err(failure) => {
                let mut result = ToolResult::error(failure.message);
                result.structured_content =
                    Some(json!({"action":action,"status":"error","error":failure}));
                result
            }
        }))
    })
}
