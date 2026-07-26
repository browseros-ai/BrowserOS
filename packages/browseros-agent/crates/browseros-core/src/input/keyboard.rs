use crate::{CoreError, ProtocolSession};
use serde_json::{Value, json};

#[cfg(target_os = "macos")]
const PLATFORM_MODIFIER: i64 = 4;
#[cfg(not(target_os = "macos"))]
const PLATFORM_MODIFIER: i64 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyInfo {
    pub code: String,
    pub key_code: Option<i64>,
}

pub fn normalize_key(key: &str) -> String {
    if key_info_named(key).is_some() {
        return key.to_string();
    }
    let lower = key.to_ascii_lowercase();
    match lower.as_str() {
        "return" => "Enter",
        "esc" => "Escape",
        "del" => "Delete",
        "ctrl" => "Control",
        "cmd" | "command" => "Meta",
        "option" => "Alt",
        "left" => "ArrowLeft",
        "right" => "ArrowRight",
        "up" => "ArrowUp",
        "down" => "ArrowDown",
        _ => key,
    }
    .to_string()
}

pub fn get_key_info(key: &str) -> KeyInfo {
    if let Some((code, key_code)) = key_info_named(key) {
        return KeyInfo {
            code: code.to_string(),
            key_code: Some(key_code),
        };
    }
    let mut chars = key.chars();
    let first = chars.next();
    if let Some(ch) = first
        && chars.next().is_none()
    {
        if ch.is_ascii_lowercase() {
            return KeyInfo {
                code: format!("Key{}", ch.to_ascii_uppercase()),
                key_code: Some(ch.to_ascii_uppercase() as i64),
            };
        }
        if ch.is_ascii_uppercase() {
            return KeyInfo {
                code: format!("Key{ch}"),
                key_code: Some(ch as i64),
            };
        }
        if ch.is_ascii_digit() {
            return KeyInfo {
                code: format!("Digit{ch}"),
                key_code: Some(ch as i64),
            };
        }
    }
    KeyInfo {
        code: key.to_string(),
        key_code: None,
    }
}

pub fn modifier_bitmask(modifiers: &[String]) -> i64 {
    modifiers.iter().fold(0, |mask, modifier| {
        mask | match modifier.as_str() {
            "Alt" => 1,
            "Control" => 2,
            "Meta" => 4,
            "Shift" => 8,
            _ => 0,
        }
    })
}

pub async fn type_text(session: &ProtocolSession, text: &str) -> Result<(), CoreError> {
    for ch in text.chars() {
        let is_newline = ch == '\n';
        let key = if is_newline {
            "Enter".to_string()
        } else {
            ch.to_string()
        };
        let info = get_key_info(&key);
        dispatch_key(
            session,
            json!({
                "type": "keyDown",
                "key": key,
                "code": info.code,
                "windowsVirtualKeyCode": info.key_code
            }),
        )
        .await?;
        dispatch_key(
            session,
            json!({
                "type": "char",
                "text": if is_newline { "\r".to_string() } else { ch.to_string() },
                "key": key
            }),
        )
        .await?;
        dispatch_key(
            session,
            json!({
                "type": "keyUp",
                "key": key,
                "code": info.code,
                "windowsVirtualKeyCode": info.key_code
            }),
        )
        .await?;
    }
    Ok(())
}

pub async fn clear_field(session: &ProtocolSession) -> Result<(), CoreError> {
    dispatch_key(
        session,
        json!({
            "type": "keyDown",
            "key": "a",
            "code": "KeyA",
            "modifiers": PLATFORM_MODIFIER,
            "windowsVirtualKeyCode": 65
        }),
    )
    .await?;
    dispatch_key(
        session,
        json!({
            "type": "keyUp",
            "key": "a",
            "code": "KeyA",
            "modifiers": PLATFORM_MODIFIER,
            "windowsVirtualKeyCode": 65
        }),
    )
    .await?;
    dispatch_key(
        session,
        json!({
            "type": "keyDown",
            "key": "Backspace",
            "code": "Backspace",
            "windowsVirtualKeyCode": 8
        }),
    )
    .await?;
    dispatch_key(
        session,
        json!({
            "type": "keyUp",
            "key": "Backspace",
            "code": "Backspace",
            "windowsVirtualKeyCode": 8
        }),
    )
    .await
}

pub async fn press_combo(session: &ProtocolSession, key: &str) -> Result<(), CoreError> {
    let parsed = parse_key_combo(key)?;
    let main_key = normalize_key(&parsed.key);
    let modifiers = parsed
        .modifiers
        .iter()
        .map(|modifier| normalize_key(modifier))
        .collect::<Vec<_>>();
    validate_key(&main_key)?;
    for modifier in &modifiers {
        validate_key(modifier)?;
    }

    let bitmask = modifier_bitmask(&modifiers);
    for modifier in &modifiers {
        let info = get_key_info(modifier);
        dispatch_key(
            session,
            json!({
                "type": "keyDown",
                "key": modifier,
                "code": info.code,
                "windowsVirtualKeyCode": info.key_code
            }),
        )
        .await?;
    }

    let main_info = get_key_info(&main_key);
    let suppress_char = modifiers
        .iter()
        .any(|modifier| matches!(modifier.as_str(), "Control" | "Alt" | "Meta"));
    let text = if suppress_char {
        String::new()
    } else {
        get_char_text(&main_key)
    };
    let mut key_down = serde_json::Map::new();
    key_down.insert("type".to_string(), Value::String("keyDown".to_string()));
    key_down.insert("key".to_string(), Value::String(main_key.clone()));
    key_down.insert("code".to_string(), Value::String(main_info.code.clone()));
    key_down.insert("modifiers".to_string(), Value::from(bitmask));
    if let Some(key_code) = main_info.key_code {
        key_down.insert("windowsVirtualKeyCode".to_string(), Value::from(key_code));
    }
    if !text.is_empty() {
        key_down.insert("text".to_string(), Value::String(text));
    }
    dispatch_key(session, Value::Object(key_down)).await?;

    dispatch_key(
        session,
        json!({
            "type": "keyUp",
            "key": main_key,
            "code": main_info.code,
            "modifiers": bitmask,
            "windowsVirtualKeyCode": main_info.key_code
        }),
    )
    .await?;

    for modifier in modifiers.iter().rev() {
        let info = get_key_info(modifier);
        dispatch_key(
            session,
            json!({
                "type": "keyUp",
                "key": modifier,
                "code": info.code,
                "windowsVirtualKeyCode": info.key_code
            }),
        )
        .await?;
    }
    Ok(())
}

#[derive(Debug)]
struct ParsedCombo {
    key: String,
    modifiers: Vec<String>,
}

fn parse_key_combo(input: &str) -> Result<ParsedCombo, CoreError> {
    let mut parts = Vec::new();
    let mut current = String::new();
    for ch in input.chars() {
        if ch == '+' && !current.is_empty() {
            parts.push(current);
            current = String::new();
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    if parts.is_empty() {
        return Err(CoreError::Message("Empty key input".to_string()));
    }
    let key = parts.pop().unwrap_or_default();
    Ok(ParsedCombo {
        key,
        modifiers: parts,
    })
}

fn validate_key(key: &str) -> Result<(), CoreError> {
    if key_info_named(key).is_some() || key.chars().count() == 1 {
        return Ok(());
    }
    Err(CoreError::Message(format!(
        "Unknown key: \"{key}\". Valid keys: Backspace, Tab, Enter, Escape, Space, PageUp, PageDown, End, Home, ArrowLeft, ArrowUp, ArrowRight, ArrowDown, Insert, Delete, Shift, Control, Alt, Meta, F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12, single characters (a-z, A-Z, 0-9, symbols). Aliases: Return → Enter, Esc → Escape, Del → Delete, Ctrl → Control, Cmd → Meta, Command → Meta, Option → Alt, Left → ArrowLeft, Right → ArrowRight, Up → ArrowUp, Down → ArrowDown"
    )))
}

fn get_char_text(key: &str) -> String {
    match key {
        "Enter" => "\r".to_string(),
        "Tab" => "\t".to_string(),
        "Space" | " " => " ".to_string(),
        _ if key.chars().count() == 1 => key.to_string(),
        _ => String::new(),
    }
}

async fn dispatch_key(session: &ProtocolSession, params: Value) -> Result<(), CoreError> {
    let params = strip_nulls(params);
    let _: Value = session.send("Input.dispatchKeyEvent", params).await?;
    Ok(())
}

fn strip_nulls(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter_map(|(key, value)| (!value.is_null()).then_some((key, value)))
                .collect(),
        ),
        other => other,
    }
}

fn key_info_named(key: &str) -> Option<(&'static str, i64)> {
    Some(match key {
        "Backspace" => ("Backspace", 8),
        "Tab" => ("Tab", 9),
        "Enter" => ("Enter", 13),
        "Escape" => ("Escape", 27),
        "Space" | " " => ("Space", 32),
        "PageUp" => ("PageUp", 33),
        "PageDown" => ("PageDown", 34),
        "End" => ("End", 35),
        "Home" => ("Home", 36),
        "ArrowLeft" => ("ArrowLeft", 37),
        "ArrowUp" => ("ArrowUp", 38),
        "ArrowRight" => ("ArrowRight", 39),
        "ArrowDown" => ("ArrowDown", 40),
        "Insert" => ("Insert", 45),
        "Delete" => ("Delete", 46),
        "Shift" => ("ShiftLeft", 16),
        "Control" => ("ControlLeft", 17),
        "Alt" => ("AltLeft", 18),
        "Meta" => ("MetaLeft", 91),
        "F1" => ("F1", 112),
        "F2" => ("F2", 113),
        "F3" => ("F3", 114),
        "F4" => ("F4", 115),
        "F5" => ("F5", 116),
        "F6" => ("F6", 117),
        "F7" => ("F7", 118),
        "F8" => ("F8", 119),
        "F9" => ("F9", 120),
        "F10" => ("F10", 121),
        "F11" => ("F11", 122),
        "F12" => ("F12", 123),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{get_key_info, modifier_bitmask, normalize_key};

    #[test]
    fn normalizes_aliases() {
        assert_eq!(normalize_key("Cmd"), "Meta");
        assert_eq!(normalize_key("Esc"), "Escape");
        assert_eq!(normalize_key("Left"), "ArrowLeft");
    }

    #[test]
    fn maps_printable_keys() {
        let lower = get_key_info("a");
        assert_eq!(lower.code, "KeyA");
        assert_eq!(lower.key_code, Some(65));

        let digit = get_key_info("7");
        assert_eq!(digit.code, "Digit7");
        assert_eq!(digit.key_code, Some(55));
    }

    #[test]
    fn computes_modifier_bitmask() {
        let modifiers = vec!["Alt".to_string(), "Meta".to_string(), "Shift".to_string()];
        assert_eq!(modifier_bitmask(&modifiers), 13);
    }
}
