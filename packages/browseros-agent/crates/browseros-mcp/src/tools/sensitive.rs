//! What counts as secret-bearing input. Hosts consult this before a call can
//! fail, to redact audit records and skip screenshots, so it must recognise
//! every route to saved passwords and payment cards, raw CDP included.
use serde_json::Value;

/// MCP tools whose arguments can carry passwords or card data.
#[must_use]
pub fn is_sensitive_tool(name: &str) -> bool {
    matches!(name, "credentials" | "payment_cards")
}

/// Script bridge primitives that reach those tools or their CDP domains. Both
/// structured and raw CDP calls are classified before any early error.
#[must_use]
pub fn is_sensitive_primitive(method: &str, args: &[Value]) -> bool {
    if let Some(tool) = method.strip_prefix("tool:") {
        return is_sensitive_tool(tool);
    }
    let index = match method {
        "cdp" => 0,
        "cdpJsonForPage" => 1,
        _ => return false,
    };
    args.get(index)
        .and_then(Value::as_str)
        .is_some_and(is_sensitive_cdp_method)
}

/// CDP methods that carry passwords or card data: BrowserOS's Passwords and
/// Payments domains, the Autofill credential commands older browsers still
/// serve, and upstream `Autofill.trigger`, which takes a full card.
#[must_use]
pub fn is_sensitive_cdp_method(method: &str) -> bool {
    method.starts_with("Passwords.")
        || method.starts_with("Payments.")
        || method == "Autofill.trigger"
        || (method.starts_with("Autofill.") && method.to_ascii_lowercase().contains("credential"))
}

/// The page a tool call fills a card into. Unlike a password, a filled card
/// number shows in plain text, so hosts keep that page out of screenshots
/// until its document changes.
#[must_use]
pub fn card_fill_page(tool: &str, args: &Value) -> Option<u32> {
    if tool != "payment_cards" || args.get("action").and_then(Value::as_str) != Some("fill") {
        return None;
    }
    args.get("page")?.as_u64()?.try_into().ok()
}

/// CDP methods that write a card into a page.
#[must_use]
pub fn fills_card(method: &str) -> bool {
    matches!(method, "Payments.fillCard" | "Autofill.trigger")
}

/// The page a script bridge primitive fills a card into: the `payment_cards`
/// tool, or raw `Payments.fillCard` / `Autofill.trigger` on a page. Raw CDP
/// addressed by session id names no page here; the bridge resolves it.
#[must_use]
pub fn primitive_card_fill_page(method: &str, args: &[Value]) -> Option<u32> {
    match method {
        "tool:payment_cards" => card_fill_page("payment_cards", args.first()?),
        "cdpJsonForPage" if args.get(1).and_then(Value::as_str).is_some_and(fills_card) => {
            args.first()?.as_u64()?.try_into().ok()
        }
        _ => None,
    }
}

/// Conservative classification of `run` source before admission or execution:
/// a script that fails before reaching its sensitive call still contains it.
#[must_use]
pub fn source_may_hold_secrets(source: &str) -> bool {
    let source = source.to_ascii_lowercase();
    [
        "credential",
        "password",
        "paymentcard",
        "payments.",
        "autofill.trigger",
        "cvc",
    ]
    .iter()
    .any(|needle| source.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sensitive_routes_cover_tools_domains_and_legacy_names() {
        assert!(is_sensitive_tool("credentials"));
        assert!(is_sensitive_tool("payment_cards"));
        assert!(!is_sensitive_tool("tabs"));
        assert!(is_sensitive_primitive("tool:payment_cards", &[json!({})]));
        assert!(is_sensitive_primitive("cdp", &[json!("Payments.fillCard")]));
        assert!(is_sensitive_primitive(
            "cdpJsonForPage",
            &[json!(1), json!("Passwords.addCredential")]
        ));
        assert!(is_sensitive_primitive("cdp", &[json!("Autofill.trigger")]));
        assert!(is_sensitive_primitive(
            "cdp",
            &[json!("Autofill.fillCredential")]
        ));
        assert!(!is_sensitive_primitive("cdp", &[json!("Autofill.enable")]));
        assert!(!is_sensitive_primitive("tool:tabs", &[json!({})]));
    }

    #[test]
    fn card_fills_name_the_page_they_write_into() {
        assert_eq!(
            card_fill_page("payment_cards", &json!({"action":"fill","page":3})),
            Some(3)
        );
        assert_eq!(
            card_fill_page("payment_cards", &json!({"action":"list","page":3})),
            None
        );
        assert_eq!(
            card_fill_page("credentials", &json!({"action":"fill","page":3})),
            None
        );
        assert_eq!(
            primitive_card_fill_page("tool:payment_cards", &[json!({"action":"fill","page":4})]),
            Some(4)
        );
        assert_eq!(
            primitive_card_fill_page("cdpJsonForPage", &[json!(5), json!("Payments.fillCard")]),
            Some(5)
        );
        assert_eq!(
            primitive_card_fill_page("cdpJsonForPage", &[json!(6), json!("Autofill.trigger")]),
            Some(6)
        );
        assert_eq!(
            primitive_card_fill_page("cdpJsonForPage", &[json!(7), json!("Payments.listCards")]),
            None
        );
    }

    #[test]
    fn run_source_heuristic_flags_card_and_password_scripts() {
        assert!(source_may_hold_secrets(
            "await browser.paymentCards({action:'list'})"
        ));
        assert!(source_may_hold_secrets("cdp('Payments.fillCard', {})"));
        assert!(source_may_hold_secrets(
            "browser.credentials({password:'x'})"
        ));
        assert!(!source_may_hold_secrets("await browser.page(1).snapshot()"));
    }
}
