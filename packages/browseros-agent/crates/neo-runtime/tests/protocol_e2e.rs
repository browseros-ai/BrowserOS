//! Whether the BrowserOS neo under test still speaks the protocol neo-runtime decodes with.
//!
//! The door (`src/chromium_bridge/cdp.rs`) declares every CDP command and event the crate names
//! on browseros-cdp's types, which `build.rs` generates from the pinned
//! `crates/browseros-cdp/protocol/protocol.json`, and its events and replies decode strictly: a
//! field the pinned protocol requires must be there. The door's own tests check each declaration
//! against that pinned file; this one checks the pinned file against the browser. It reads the
//! browser's protocol from its CDP HTTP endpoint (`/json/protocol`) and fails when a
//! declared method is missing there, or when its required params, returns or event fields differ
//! from the pinned ones, followed through the types they name. The fix is to re-pin the protocol
//! (`crates/browseros-cdp/sync-protocol.sh`) and rebuild, not to loosen the decoding.
//!
//! Runs only when `NEO_E2E_BROWSER` names the browser binary (`support/browser.rs`); otherwise
//! the browser test passes as a skip. The tests of the comparison itself always run.

#[path = "support/browser.rs"]
#[expect(
    dead_code,
    reason = "shared browser harness also drives runs, navigation and downloads"
)]
mod browser;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use browser::{Fixture, Tab, e2e_browser};
use regex::Regex;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time;

/// How long the browser has to serve its protocol, a few megabytes, on a loaded machine.
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// What a declaration is: a command neo-runtime sends, or an event it decodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Command,
    Event,
}

/// One declared method: its kind, domain and name within the domain (`Page`, `navigate`).
type Declared = (Kind, String, String);

/// A method's required fields, each by its path (`params.url`, `returns.frameId`, `frame.id`,
/// `items[]`) and its type (`string`, `Page.Frame: object`).
type Shape = BTreeMap<String, String>;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every method the door's domain files declare, read from their `command!` and `event!` lists:
/// the lists are the door's whole surface (`cdp.rs`), and an integration test cannot see the
/// `#[cfg(test)]` lists the macros build inside the crate.
fn declared() -> BTreeSet<Declared> {
    let line = Regex::new(r#"^\s*\w+\s*=>\s*"(\w+)\.(\w+)""#)
        .unwrap_or_else(|error| panic!("the declaration pattern: {error}"));
    let folder = crate_dir().join("src/chromium_bridge/cdp");
    let entries =
        fs::read_dir(&folder).unwrap_or_else(|error| panic!("read {}: {error}", folder.display()));
    let mut found = BTreeSet::new();
    for entry in entries {
        let path = entry
            .unwrap_or_else(|error| panic!("list {}: {error}", folder.display()))
            .path();
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let mut list = None;
        for text in source.lines() {
            match text.trim() {
                "command! {" => list = Some(Kind::Command),
                "event! {" => list = Some(Kind::Event),
                "}" => list = None,
                _ => {
                    if let (Some(kind), Some(captures)) = (list, line.captures(text)) {
                        found.insert((kind, captures[1].to_string(), captures[2].to_string()));
                    }
                }
            }
        }
    }
    found
}

fn read_protocol(path: &Path) -> Value {
    let text =
        fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

/// browseros-cdp's pinned protocol, the one its generated types come from.
fn pinned() -> Value {
    read_protocol(&crate_dir().join("../browseros-cdp/protocol/protocol.json"))
}

/// The browser's own protocol, as its CDP HTTP endpoint serves it.
async fn live(port: u16) -> Value {
    let body = time::timeout(FETCH_TIMEOUT, get(port, "/json/protocol"))
        .await
        .unwrap_or_else(|_| panic!("/json/protocol: no answer within {FETCH_TIMEOUT:?}"))
        .unwrap_or_else(|failure| panic!("/json/protocol: {failure}"));
    serde_json::from_slice(&body).unwrap_or_else(|error| panic!("parse /json/protocol: {error}"))
}

/// The body of `GET path` on the browser's CDP HTTP endpoint. Chromium's HTTP server
/// keeps the connection open after its answer, so the body ends at its `Content-Length`, not at
/// the connection's close.
async fn get(port: u16, path: &str) -> Result<Vec<u8>, String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .map_err(|error| format!("connect: {error}"))?;
    let request = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|error| format!("send: {error}"))?;
    let mut response = Vec::new();
    let mut chunk = vec![0_u8; 64 * 1024];
    loop {
        if let Some(end) = response.windows(4).position(|window| window == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&response[..end]).into_owned();
            if !head.starts_with("HTTP/1.1 200") {
                return Err(format!("answered {head}"));
            }
            let length = head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.trim()
                        .eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())?
                })
                .ok_or_else(|| format!("no Content-Length in {head}"))?;
            let body = end + 4;
            if response.len() >= body + length {
                return Ok(response[body..body + length].to_vec());
            }
        }
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|error| format!("read: {error}"))?;
        if read == 0 {
            return Err(format!("closed after {} bytes", response.len()));
        }
        response.extend_from_slice(&chunk[..read]);
    }
}

/// `domain`'s member `name` in `list` (`commands`, `events` or `types`, keyed by `key`).
fn member<'a>(
    protocol: &'a Value,
    domain: &str,
    list: &str,
    key: &str,
    name: &str,
) -> Option<&'a Value> {
    protocol["domains"]
        .as_array()?
        .iter()
        .find(|entry| entry["domain"] == domain)?[list]
        .as_array()?
        .iter()
        .find(|entry| entry[key] == name)
}

/// The required fields of a declared method in `protocol`, or `None` when it has no such
/// method: a command's required params and returns, an event's required fields.
fn shape(protocol: &Value, (kind, domain, name): &Declared) -> Option<Shape> {
    let list = match kind {
        Kind::Command => "commands",
        Kind::Event => "events",
    };
    let method = member(protocol, domain, list, "name", name)?;
    let mut fields = Shape::new();
    let mut named = Vec::new();
    match kind {
        Kind::Command => {
            required(
                protocol,
                domain,
                &method["parameters"],
                "params.",
                &mut named,
                &mut fields,
            );
            required(
                protocol,
                domain,
                &method["returns"],
                "returns.",
                &mut named,
                &mut fields,
            );
        }
        Kind::Event => required(
            protocol,
            domain,
            &method["parameters"],
            "",
            &mut named,
            &mut fields,
        ),
    }
    Some(fields)
}

/// Records each required member of `members` (a parameter or property list of `domain`) under
/// `prefix`. An optional member is left out: the generated types decode without it, and a
/// browser that starts requiring it shows up as a field the pinned shape lacks.
fn required(
    protocol: &Value,
    domain: &str,
    members: &Value,
    prefix: &str,
    named: &mut Vec<String>,
    fields: &mut Shape,
) {
    for field in members.as_array().into_iter().flatten() {
        if field["optional"].as_bool() == Some(true) {
            continue;
        }
        let path = format!("{prefix}{}", field["name"].as_str().unwrap_or_default());
        walk(protocol, domain, field, &path, named, fields);
    }
}

/// Records `path`'s type, and the required fields under it: an array's items, an object's
/// properties, and a named type's own definition, whose name joins the type (`Page.Frame:
/// object`). `named` holds the named types being expanded, so a type that contains itself
/// stops at its name.
fn walk(
    protocol: &Value,
    domain: &str,
    schema: &Value,
    path: &str,
    named: &mut Vec<String>,
    fields: &mut Shape,
) {
    if let Some(reference) = schema["$ref"].as_str() {
        let (owner, id) = reference.split_once('.').unwrap_or((domain, reference));
        let qualified = format!("{owner}.{id}");
        let Some(definition) = member(protocol, owner, "types", "id", id) else {
            fields.insert(path.to_string(), format!("{qualified} (undefined)"));
            return;
        };
        if named.contains(&qualified) {
            fields.insert(path.to_string(), qualified);
            return;
        }
        named.push(qualified.clone());
        walk(protocol, owner, definition, path, named, fields);
        named.pop();
        if let Some(kind) = fields.get_mut(path) {
            *kind = format!("{qualified}: {kind}");
        }
        return;
    }
    let kind = schema["type"].as_str().unwrap_or("untyped");
    fields.insert(path.to_string(), kind.to_string());
    match kind {
        "array" => walk(
            protocol,
            domain,
            &schema["items"],
            &format!("{path}[]"),
            named,
            fields,
        ),
        "object" => required(
            protocol,
            domain,
            &schema["properties"],
            &format!("{path}."),
            named,
            fields,
        ),
        _ => {}
    }
}

/// Each way `browser` disagrees with `pinned` on a declared method, one line apiece.
fn disagreements(declared: &BTreeSet<Declared>, pinned: &Value, browser: &Value) -> Vec<String> {
    let mut found = Vec::new();
    for method in declared {
        let (_, domain, name) = method;
        let Some(expected) = shape(pinned, method) else {
            found.push(format!("{domain}.{name}: not in the pinned protocol"));
            continue;
        };
        let Some(actual) = shape(browser, method) else {
            found.push(format!("{domain}.{name}: not in the browser's protocol"));
            continue;
        };
        let paths: BTreeSet<&String> = expected.keys().chain(actual.keys()).collect();
        for path in paths {
            let (want, have) = (expected.get(path), actual.get(path));
            if want != have {
                let absent = String::from("absent or optional");
                found.push(format!(
                    "{domain}.{name}: {path}: pinned {}, browser {}",
                    want.unwrap_or(&absent),
                    have.unwrap_or(&absent),
                ));
            }
        }
    }
    found
}

#[test]
fn the_door_declares_commands_and_events() {
    let declared = declared();
    let has = |kind: Kind, domain: &str, name: &str| {
        declared.contains(&(kind, domain.to_string(), name.to_string()))
    };
    assert!(has(Kind::Command, "Page", "navigate"), "{declared:?}");
    assert!(has(Kind::Event, "Page", "frameNavigated"), "{declared:?}");
    assert!(
        !has(Kind::Command, "Page", "frameNavigated"),
        "{declared:?}"
    );
}

#[test]
fn a_shape_follows_required_fields_through_the_types_they_name() {
    let pinned = pinned();
    let event = (
        Kind::Event,
        "Page".to_string(),
        "frameNavigated".to_string(),
    );
    let fields = shape(&pinned, &event).unwrap_or_default();
    assert_eq!(
        fields.get("frame").map(String::as_str),
        Some("Page.Frame: object")
    );
    assert_eq!(
        fields.get("frame.loaderId").map(String::as_str),
        Some("Network.LoaderId: string")
    );
    assert_eq!(
        fields.get("frame.gatedAPIFeatures[]").map(String::as_str),
        Some("Page.GatedAPIFeatures: string")
    );
    assert!(
        !fields.contains_key("frame.parentId"),
        "an optional field is left out: {fields:?}"
    );
    let command = (Kind::Command, "Page".to_string(), "navigate".to_string());
    let fields = shape(&pinned, &command).unwrap_or_default();
    assert_eq!(fields.get("params.url").map(String::as_str), Some("string"));
    assert_eq!(
        fields.get("returns.frameId").map(String::as_str),
        Some("Page.FrameId: string")
    );
}

#[test]
fn a_browser_that_drops_or_loosens_a_required_field_disagrees() {
    let pinned = pinned();
    let declared = BTreeSet::from([(
        Kind::Event,
        "Page".to_string(),
        "frameNavigated".to_string(),
    )]);
    assert_eq!(
        disagreements(&declared, &pinned, &pinned),
        Vec::<String>::new()
    );
    let mut browser = pinned.clone();
    let loader = browser["domains"]
        .as_array_mut()
        .and_then(|domains| domains.iter_mut().find(|domain| domain["domain"] == "Page"))
        .and_then(|page| page["types"].as_array_mut())
        .and_then(|types| types.iter_mut().find(|kind| kind["id"] == "Frame"))
        .and_then(|frame| frame["properties"].as_array_mut())
        .and_then(|properties| {
            properties
                .iter_mut()
                .find(|property| property["name"] == "loaderId")
        });
    if let Some(loader) = loader {
        loader["optional"] = Value::Bool(true);
    }
    assert_eq!(
        disagreements(&declared, &pinned, &browser),
        vec![
            "Page.frameNavigated: frame.loaderId: pinned Network.LoaderId: string, browser absent \
             or optional"
                .to_string()
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_browser_speaks_the_pinned_protocol_for_every_declared_method() {
    let Some(binary) = e2e_browser() else {
        return;
    };
    let tab = Tab::open(binary, Fixture::default(), "/protocol").await;
    let browser = live(tab.port()).await;
    tab.close().await;
    let declared = declared();
    let found = disagreements(&declared, &pinned(), &browser);
    assert!(
        found.is_empty(),
        "the browser's protocol differs from crates/browseros-cdp/protocol/protocol.json on \
         the {} methods neo-runtime declares; re-pin it with \
         crates/browseros-cdp/sync-protocol.sh:\n{}",
        declared.len(),
        found.join("\n")
    );
}
