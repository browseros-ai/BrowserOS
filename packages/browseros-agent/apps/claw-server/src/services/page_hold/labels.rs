//! What the working overlay says neo is doing, in plain words for the person watching.
//!
//! Labels name the element neo acts on and never what it types: values can be secrets.

use serde_json::Value;

/// Shown while neo is between actions on a held page.
pub const BETWEEN: &str = "Working out the next step";

const GENERIC: &str = "Working on this page";
const NAME_LIMIT: usize = 40;

/// The label for a granular tool call. `target` is the accessible name of the ref it acts on.
#[must_use]
pub fn for_tool(tool: &str, args: &Value, target: Option<&str>) -> String {
    let named = |verb: &str, fallback: &str| match target.map(trim_name).filter(|n| !n.is_empty()) {
        Some(name) => format!("{verb} \"{name}\""),
        None => fallback.to_string(),
    };
    match tool {
        "act" => match args["kind"].as_str().unwrap_or_default() {
            "click" | "click_at" => named("Clicking", "Clicking on the page"),
            "fill"
                if args["fields"]
                    .as_array()
                    .is_some_and(|fields| fields.len() > 1) =>
            {
                "Filling in the form".to_string()
            }
            "fill" | "type" | "type_at" => named("Typing in", "Typing on the page"),
            "select" => named("Choosing in", "Choosing an option"),
            "press" => match args["key"]
                .as_str()
                .map(str::trim)
                .filter(|k| !k.is_empty())
            {
                Some(key) => format!("Pressing {}", trim_name(key)),
                None => "Pressing a key".to_string(),
            },
            "scroll" => "Scrolling the page".to_string(),
            "hover" | "hover_at" => named("Pointing at", "Pointing on the page"),
            "focus" => named("Moving to", "Moving on the page"),
            "check" => named("Ticking", "Ticking a box"),
            "uncheck" => named("Unticking", "Unticking a box"),
            "drag" | "drag_at" => named("Dragging", "Dragging on the page"),
            "dialog_accept" | "dialog_dismiss" => "Answering a dialog".to_string(),
            _ => GENERIC.to_string(),
        },
        "navigate" => match args["action"].as_str().unwrap_or("url") {
            "back" => "Going back".to_string(),
            "forward" => "Going forward".to_string(),
            "reload" => "Reloading the page".to_string(),
            _ => match args["url"].as_str().and_then(host_of) {
                Some(host) => format!("Opening {host}"),
                None => "Opening a page".to_string(),
            },
        },
        "upload" => "Uploading a file".to_string(),
        "download" => "Downloading a file".to_string(),
        "solve_captcha" => "Solving a check on this page".to_string(),
        _ => GENERIC.to_string(),
    }
}

/// The label for one primitive of a `run` script, from its method name (`input.click`).
#[must_use]
pub fn for_script(method: &str) -> String {
    let method = method.strip_prefix("tool:").unwrap_or(method);
    let action = method.rsplit('.').next().unwrap_or(method);
    match (method.split('.').next().unwrap_or_default(), action) {
        ("nav", "goto") => "Opening a page",
        ("nav", "back") => "Going back",
        ("nav", "forward") => "Going forward",
        ("nav", "reload") => "Reloading the page",
        (_, "click" | "clickAt" | "click_at") => "Clicking on the page",
        (_, "fill" | "type" | "typeAt" | "type_at" | "typeText") => "Typing on the page",
        (_, "press") => "Pressing a key",
        (_, "scroll") => "Scrolling the page",
        (_, "hover" | "hoverAt" | "hover_at") => "Pointing on the page",
        (_, "select" | "selectOption") => "Choosing an option",
        (_, "check") => "Ticking a box",
        (_, "uncheck") => "Unticking a box",
        (_, "drag" | "dragAt" | "drag_at") => "Dragging on the page",
        _ => GENERIC,
    }
    .to_string()
}

fn trim_name(name: &str) -> String {
    let single_line = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if single_line.chars().count() <= NAME_LIMIT {
        return single_line;
    }
    let mut short: String = single_line.chars().take(NAME_LIMIT - 1).collect();
    short.push('…');
    short
}

fn host_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    Some(host.strip_prefix("www.").unwrap_or(host).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn act_labels_name_the_target_and_never_the_value() {
        assert_eq!(
            for_tool(
                "act",
                &json!({ "kind": "click", "ref": "e1" }),
                Some("Add to cart")
            ),
            "Clicking \"Add to cart\""
        );
        let typed = for_tool(
            "act",
            &json!({ "kind": "fill", "ref": "e2", "value": "hunter2" }),
            Some("Password"),
        );
        assert_eq!(typed, "Typing in \"Password\"");
        assert!(!typed.contains("hunter2"));
        assert_eq!(
            for_tool("act", &json!({ "kind": "fill", "fields": [{}, {}] }), None),
            "Filling in the form"
        );
        assert_eq!(
            for_tool("act", &json!({ "kind": "press", "key": "Enter" }), None),
            "Pressing Enter"
        );
        assert_eq!(
            for_tool("act", &json!({ "kind": "click_at", "x": 1, "y": 2 }), None),
            "Clicking on the page"
        );
    }

    #[test]
    fn long_or_multiline_names_are_shortened() {
        let label = for_tool(
            "act",
            &json!({ "kind": "click" }),
            Some("A very long\nbutton name that keeps going and going and going"),
        );
        assert!(label.starts_with("Clicking \"A very long button name"));
        assert!(label.ends_with("…\""));
    }

    #[test]
    fn navigation_names_the_site() {
        assert_eq!(
            for_tool(
                "navigate",
                &json!({ "action": "url", "url": "https://www.example.com/a?b=c" }),
                None
            ),
            "Opening example.com"
        );
        assert_eq!(
            for_tool("navigate", &json!({ "action": "back" }), None),
            "Going back"
        );
    }

    #[test]
    fn script_primitives_read_as_plain_actions() {
        assert_eq!(for_script("input.click"), "Clicking on the page");
        assert_eq!(for_script("nav.goto"), "Opening a page");
        assert_eq!(for_script("tool:act"), "Working on this page");
    }
}
