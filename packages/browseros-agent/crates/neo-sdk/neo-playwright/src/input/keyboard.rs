//! Playwright's keyboard as data and state, ported from upstream's `Keyboard` (server/input.ts)
//! and Chromium's `RawKeyboardImpl` (server/chromium/crInput.ts) at v1.64.1: which keys and
//! modifiers are held, and the exact `Input.dispatchKeyEvent` and `Input.insertText` events
//! each call sends.
//!
//! One deliberate difference: a call checks every key name before anything changes. Upstream
//! sends Shift down and only then fails on `Foo` in `press('Shift+Foo')`, which leaves Shift
//! held; here that call fails with nothing held and nothing to send.

use std::collections::HashMap;
use std::error::Error;
use std::sync::LazyLock;
use std::time::Duration;
use std::{fmt, mem};

use serde_json::{Value, json};

use super::input_devices::{Held, Hold, InputEvent, InputStep};
use super::mac_editing_commands::MAC_EDITING_COMMANDS;
use super::us_keyboard_layout::{KEYPAD_LOCATION, US_KEYBOARD_LAYOUT};

/// The operating system the browser runs on. It decides what `ControlOrMeta` means and whether
/// key downs carry macOS editing commands. Upstream resolves `ControlOrMeta` by the machine
/// Playwright itself runs on; Neo's agent code has no machine of its own, so both follow the
/// browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Platform {
    Mac,
    Linux,
    Windows,
}

impl Platform {
    /// Upstream's `CRBrowser._platform()`, from `Browser.getVersion`'s user agent.
    pub(crate) fn from_user_agent(user_agent: &str) -> Self {
        if user_agent.contains("Windows") {
            Self::Windows
        } else if user_agent.contains("Macintosh") {
            Self::Mac
        } else {
            Self::Linux
        }
    }

    /// The key `ControlOrMeta` names here.
    fn control_or_meta(self) -> &'static str {
        match self {
            Self::Mac => "Meta",
            Self::Linux | Self::Windows => "Control",
        }
    }
}

/// A modifier key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Modifier {
    Alt,
    Control,
    Meta,
    Shift,
}

impl Modifier {
    /// Upstream's `kModifiers`, the order `ensure_modifiers` presses and releases them in.
    const ALL: [Self; 4] = [Self::Alt, Self::Control, Self::Meta, Self::Shift];

    /// The order upstream names held modifiers in a macOS shortcut (`Shift+Control+KeyA`).
    const SHORTCUT_ORDER: [Self; 4] = [Self::Shift, Self::Control, Self::Alt, Self::Meta];

    /// The key that holds it, by its `key` (both Shift keys are `Shift`).
    fn key(self) -> &'static str {
        match self {
            Self::Alt => "Alt",
            Self::Control => "Control",
            Self::Meta => "Meta",
            Self::Shift => "Shift",
        }
    }

    fn of_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|modifier| modifier.key() == key)
    }

    const fn bit(self) -> u8 {
        match self {
            Self::Alt => 1,
            Self::Control => 2,
            Self::Meta => 4,
            Self::Shift => 8,
        }
    }
}

/// Held modifiers as CDP's `modifiers` bitmask (Alt 1, Control 2, Meta 4, Shift 8), which every
/// key and mouse event carries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Modifiers(u8);

impl Modifiers {
    pub(crate) fn mask(self) -> u8 {
        self.0
    }

    pub(crate) fn contains(self, modifier: Modifier) -> bool {
        self.0 & modifier.bit() != 0
    }

    #[must_use]
    pub(crate) fn with(self, modifier: Modifier) -> Self {
        Self(self.0 | modifier.bit())
    }

    #[must_use]
    fn without(self, modifier: Modifier) -> Self {
        Self(self.0 & !modifier.bit())
    }

    /// A modifier other than Shift is held: upstream then sends key events without text.
    fn suppress_text(self) -> bool {
        self.0 & !Modifier::Shift.bit() != 0
    }
}

/// A key name the US layout does not have.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct UnknownKey(String);

impl fmt::Display for UnknownKey {
    /// Upstream's message; the leaf puts it after `keyboard.press: `.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Unknown key: \"{}\"", self.0)
    }
}

impl Error for UnknownKey {}

/// What a key name presses: upstream's `KeyDescription`, without the `keyCode` that Chromium is
/// never sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct KeyDescription {
    key: &'static str,
    code: &'static str,
    /// What the key types; empty for keys that type nothing (`Shift`, `ArrowLeft`).
    text: &'static str,
    /// CDP's `windowsVirtualKeyCode`: upstream's `keyCodeWithoutLocation`.
    key_code: u32,
    location: u32,
    /// The key with Shift held. Only a key's code carries it: `KeyA` with Shift is `A`, while
    /// `a` stays `a`.
    shifted: Option<Shifted>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Shifted {
    key: &'static str,
    key_code: u32,
}

impl KeyDescription {
    fn shift(self, shifted: Shifted) -> Self {
        Self {
            key: shifted.key,
            text: shifted.key,
            key_code: shifted.key_code,
            shifted: None,
            ..self
        }
    }
}

/// Upstream's `aliases`: more names for a code.
const ALIASES: [(&str, &[&str]); 5] = [
    ("ShiftLeft", &["Shift"]),
    ("ControlLeft", &["Control"]),
    ("AltLeft", &["Alt"]),
    ("MetaLeft", &["Meta"]),
    ("Enter", &["\n", "\r"]),
];

/// Upstream's `buildLayoutClosure`: every name a key can be pressed by. That is its code and
/// aliases and, for keys without a location, its one-character key and shifted key (`1` is
/// `Digit1`, never `Numpad1`). Built in the layout's order, so a later key wins a shared name
/// as it does upstream.
static KEYS: LazyLock<HashMap<&'static str, KeyDescription>> = LazyLock::new(|| {
    let mut keys = HashMap::new();
    for definition in US_KEYBOARD_LAYOUT {
        // Upstream's `||` defaults, where 0 counts as absent.
        let key_code = definition
            .key_code_without_location
            .filter(|code| *code != 0)
            .unwrap_or(definition.key_code);
        let description = KeyDescription {
            key: definition.key,
            code: definition.code,
            text: if js_length(definition.key) == 1 {
                definition.key
            } else {
                definition.text.unwrap_or("")
            },
            key_code,
            location: definition.location.unwrap_or(0),
            shifted: None,
        };
        let shifted = definition.shift_key.map(|key| Shifted {
            key,
            key_code: definition
                .shift_key_code
                .filter(|code| *code != 0)
                .unwrap_or(key_code),
        });
        keys.insert(
            definition.code,
            KeyDescription {
                shifted,
                ..description
            },
        );
        for (code, aliases) in ALIASES {
            if code == definition.code {
                for alias in aliases {
                    keys.insert(*alias, description);
                }
            }
        }
        if description.location != 0 {
            continue;
        }
        if js_length(description.key) == 1 {
            keys.insert(description.key, description);
        }
        if let Some(shifted) = shifted {
            keys.insert(shifted.key, description.shift(shifted));
        }
    }
    keys
});

/// `string.length` in JavaScript, which counts UTF-16 units: upstream's "one character" test.
fn js_length(text: &str) -> usize {
    text.encode_utf16().count()
}

/// Upstream's `split` in `press`: `+` separates keys except where it is the key itself
/// (`+`, `Shift++`).
fn split_chord(chord: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut building = String::new();
    for character in chord.chars() {
        if character == '+' && !building.is_empty() {
            keys.push(mem::take(&mut building));
        } else {
            building.push(character);
        }
    }
    keys.push(building);
    keys
}

/// One `Input.dispatchKeyEvent`, as `RawKeyboardImpl` builds it.
pub(crate) enum KeyEvent {
    Down {
        key: KeyDescription,
        /// Held after this key went down, so a modifier's own bit is set.
        modifiers: Modifiers,
        /// The key was already down.
        auto_repeat: bool,
        /// macOS editing commands, without their trailing `:`.
        commands: Vec<&'static str>,
    },
    Up {
        key: KeyDescription,
        /// Held after this key went up.
        modifiers: Modifiers,
    },
}

impl KeyEvent {
    /// The key this event presses or lets go, by its code.
    pub(crate) fn hold(&self) -> Hold {
        match self {
            Self::Down { key, .. } => Hold::Press(Held::Key(key.code)),
            Self::Up { key, .. } => Hold::Release(Held::Key(key.code)),
        }
    }

    pub(crate) fn params(&self) -> Value {
        match self {
            Self::Down {
                key,
                modifiers,
                auto_repeat,
                commands,
            } => json!({
                "type": if key.text.is_empty() { "rawKeyDown" } else { "keyDown" },
                "modifiers": modifiers.mask(),
                "windowsVirtualKeyCode": key.key_code,
                "code": key.code,
                "commands": commands,
                "key": key.key,
                "text": key.text,
                "unmodifiedText": key.text,
                "autoRepeat": auto_repeat,
                "location": key.location,
                "isKeypad": key.location == KEYPAD_LOCATION,
            }),
            Self::Up { key, modifiers } => json!({
                "type": "keyUp",
                "modifiers": modifiers.mask(),
                "key": key.key,
                "windowsVirtualKeyCode": key.key_code,
                "code": key.code,
                "location": key.location,
                "isKeypad": key.location == KEYPAD_LOCATION,
            }),
        }
    }
}

/// One page's keyboard: the held modifiers and keys, and the browser's platform. `Clone`, so a
/// burst can be built on a copy that the tab keeps only once the burst goes out
/// (`PageInput::plan`).
#[derive(Clone, Debug)]
pub(crate) struct Keyboard {
    platform: Platform,
    modifiers: Modifiers,
    /// Codes held now. A second `down` of one is an auto-repeat.
    pressed: Vec<&'static str>,
}

impl Keyboard {
    pub(crate) fn new(platform: Platform) -> Self {
        Self {
            platform,
            modifiers: Modifiers::default(),
            pressed: Vec::new(),
        }
    }

    /// The modifiers held now. Mouse events carry them too.
    pub(crate) fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// Upstream's `resolveSmartModifier` for an action's `modifiers` option: `ControlOrMeta` by
    /// platform. `None` for a name that is not a modifier (upstream's `Unknown modifier`).
    pub(crate) fn modifier_named(&self, name: &str) -> Option<Modifier> {
        Modifier::of_key(self.resolve(name))
    }

    /// `keyboard.down(key)`.
    pub(crate) fn down(&mut self, key: &str) -> Result<Vec<InputStep>, UnknownKey> {
        let key = self.layout_key(key)?;
        Ok(vec![self.key_down(key)])
    }

    /// `keyboard.up(key)`.
    pub(crate) fn up(&mut self, key: &str) -> Result<Vec<InputStep>, UnknownKey> {
        let key = self.layout_key(key)?;
        Ok(vec![self.key_up(key)])
    }

    /// `keyboard.press(key, { delay })` and `locator.press`: each key of a `Shift+ArrowLeft`
    /// chord down in order, `delay`, then up in reverse order.
    pub(crate) fn press(
        &mut self,
        key: &str,
        delay: Duration,
    ) -> Result<Vec<InputStep>, UnknownKey> {
        let chord = split_chord(key)
            .iter()
            .map(|name| self.layout_key(name))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(self.press_chord(&chord, delay))
    }

    /// `keyboard.type(text, { delay })` and `locator.pressSequentially`: a press for each
    /// character the layout has, `Input.insertText` for any other (`é`, `👋`). As upstream,
    /// `delay` comes inside each press and before each inserted character.
    pub(crate) fn type_text(&mut self, text: &str, delay: Duration) -> Vec<InputStep> {
        let mut steps = Vec::new();
        let mut buffer = [0; 4];
        for character in text.chars() {
            let name: &str = character.encode_utf8(&mut buffer);
            if let Some(key) = KEYS.get(name) {
                steps.extend(self.press_chord(&[*key], delay));
            } else {
                if !delay.is_zero() {
                    steps.push(InputStep::Wait(delay));
                }
                steps.push(InputStep::Dispatch(InputEvent::InsertText(name.to_owned())));
            }
        }
        steps
    }

    /// `keyboard.insertText(text)`: one `Input.insertText`, no key events, no state.
    pub(crate) fn insert_text(text: &str) -> Vec<InputStep> {
        vec![InputStep::Dispatch(InputEvent::InsertText(text.to_owned()))]
    }

    /// Upstream's `ensureModifiers`, for actions with a `modifiers` option: holds exactly
    /// `wanted`, pressing and releasing in upstream's order, and returns the modifiers held
    /// before, which a second call restores.
    pub(crate) fn ensure_modifiers(
        &mut self,
        wanted: Modifiers,
    ) -> Result<(Modifiers, Vec<InputStep>), UnknownKey> {
        let restore = self.modifiers;
        let mut steps = Vec::new();
        for modifier in Modifier::ALL {
            let held = self.modifiers.contains(modifier);
            if wanted.contains(modifier) != held {
                let key = self.layout_key(modifier.key())?;
                steps.push(if held {
                    self.key_up(key)
                } else {
                    self.key_down(key)
                });
            }
        }
        Ok((restore, steps))
    }

    fn resolve<'a>(&self, name: &'a str) -> &'a str {
        if name == "ControlOrMeta" {
            self.platform.control_or_meta()
        } else {
            name
        }
    }

    fn layout_key(&self, name: &str) -> Result<KeyDescription, UnknownKey> {
        let name = self.resolve(name);
        KEYS.get(name)
            .copied()
            .ok_or_else(|| UnknownKey(name.to_owned()))
    }

    fn press_chord(&mut self, chord: &[KeyDescription], delay: Duration) -> Vec<InputStep> {
        let Some((key, held)) = chord.split_last() else {
            return Vec::new();
        };
        let mut steps = Vec::new();
        for modifier in held {
            steps.push(self.key_down(*modifier));
        }
        steps.push(self.key_down(*key));
        if !delay.is_zero() {
            steps.push(InputStep::Wait(delay));
        }
        steps.push(self.key_up(*key));
        for modifier in held.iter().rev() {
            steps.push(self.key_up(*modifier));
        }
        steps
    }

    /// The rest of upstream's `_keyDescriptionForString`: held Shift picks the shifted key, and
    /// any other held modifier types nothing.
    fn as_held(&self, key: KeyDescription) -> KeyDescription {
        let key = match key.shifted {
            Some(shifted) if self.modifiers.contains(Modifier::Shift) => key.shift(shifted),
            _ => key,
        };
        if self.modifiers.suppress_text() {
            KeyDescription { text: "", ..key }
        } else {
            key
        }
    }

    fn key_down(&mut self, key: KeyDescription) -> InputStep {
        let key = self.as_held(key);
        let auto_repeat = self.pressed.contains(&key.code);
        if !auto_repeat {
            self.pressed.push(key.code);
        }
        if let Some(modifier) = Modifier::of_key(key.key) {
            self.modifiers = self.modifiers.with(modifier);
        }
        let commands = self.commands(key.code);
        InputStep::Dispatch(InputEvent::Key(KeyEvent::Down {
            key,
            modifiers: self.modifiers,
            auto_repeat,
            commands,
        }))
    }

    fn key_up(&mut self, key: KeyDescription) -> InputStep {
        let key = self.as_held(key);
        if let Some(modifier) = Modifier::of_key(key.key) {
            self.modifiers = self.modifiers.without(modifier);
        }
        self.pressed.retain(|code| *code != key.code);
        InputStep::Dispatch(InputEvent::Key(KeyEvent::Up {
            key,
            modifiers: self.modifiers,
        }))
    }

    /// `_commandsForCode`: the macOS editing commands for `code` under the held modifiers,
    /// minus the ones that insert text, without the trailing `:`. None off a Mac.
    fn commands(&self, code: &str) -> Vec<&'static str> {
        if self.platform != Platform::Mac {
            return Vec::new();
        }
        let mut shortcut = String::new();
        for modifier in Modifier::SHORTCUT_ORDER {
            if self.modifiers.contains(modifier) {
                shortcut.push_str(modifier.key());
                shortcut.push('+');
            }
        }
        shortcut.push_str(code);
        MAC_EDITING_COMMANDS
            .iter()
            .find(|entry| entry.shortcut == shortcut)
            .map(|entry| {
                entry
                    .commands
                    .iter()
                    .filter(|command| !command.starts_with("insert"))
                    .filter_map(|command| command.strip_suffix(':'))
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::time::Duration;

    use serde_json::Value;

    use super::{KEYS, Keyboard, Modifier, Modifiers, Platform, UnknownKey};
    use crate::input::input_devices::InputStep;
    use crate::input::input_devices::tests::{by_value, sent, timeline};

    /// Recorded from playwright-core 1.64.1 by scripts/capture-goldens.mjs.
    const GOLDENS: &str = include_str!("../../tests/data/key-events.json");

    /// The calls a recorded case makes, in order, as `(method, argument)`: every
    /// `.method('argument')` in `page.keyboard.down('Shift').then(() => ...)` or
    /// `page.locator('#field').fill('')`, apart from the `locator` that names the field.
    fn calls(case: &str) -> Vec<(&str, &str)> {
        let mut calls = Vec::new();
        let mut rest = case;
        while let Some((head, tail)) = rest.split_once("('") {
            let Some((argument, after)) = tail.split_once("')") else {
                break;
            };
            let method = head.rsplit_once('.').map_or(head, |(_, method)| method);
            if method != "locator" {
                calls.push((method, argument));
            }
            rest = after;
        }
        calls
    }

    /// One recorded call through the keyboard, as upstream's server makes it.
    fn replay(
        keyboard: &mut Keyboard,
        method: &str,
        argument: &str,
    ) -> Result<Vec<InputStep>, UnknownKey> {
        match method {
            "press" => keyboard.press(argument, Duration::ZERO),
            "down" => keyboard.down(argument),
            "up" => keyboard.up(argument),
            "type" | "pressSequentially" => Ok(keyboard.type_text(argument, Duration::ZERO)),
            // dom.ts `_fill`: an input that needs typing takes the whole value as one
            // `Input.insertText`, and an empty value is a press of Delete.
            "fill" if argument.is_empty() => keyboard.press("Delete", Duration::ZERO),
            "insertText" | "fill" => Ok(Keyboard::insert_text(argument)),
            other => panic!("the goldens call {other}, which this replay does not know"),
        }
    }

    #[test]
    fn tests_that_every_recorded_key_call_sends_what_playwright_sent() -> Result<(), Box<dyn Error>>
    {
        let goldens: Value = serde_json::from_str(GOLDENS)?;
        let mut mismatches = Vec::new();
        for (profile, platform) in [("linux", Platform::Linux), ("mac", Platform::Mac)] {
            let Some(cases) = goldens["profiles"][profile].as_array() else {
                panic!("the goldens have no {profile} profile");
            };
            assert!(!cases.is_empty(), "the {profile} profile has cases");
            for case in cases {
                let call = case["call"].as_str().unwrap_or_default();
                let replayed = calls(call);
                assert!(!replayed.is_empty(), "{call} names a keyboard call");
                let mut keyboard = Keyboard::new(platform);
                let mut steps = Vec::new();
                for (method, argument) in replayed {
                    steps.extend(replay(&mut keyboard, method, argument)?);
                }
                let expected = by_value(case["messages"].clone());
                if Value::Array(sent(&steps)) != expected {
                    mismatches.push(format!("{profile}: {call}"));
                }
            }
        }
        assert!(
            mismatches.is_empty(),
            "sent other events than Playwright: {mismatches:#?}"
        );
        Ok(())
    }

    #[test]
    fn tests_that_control_or_meta_follows_the_browser_platform() -> Result<(), Box<dyn Error>> {
        for (platform, meant) in [
            (Platform::Linux, "Control+A"),
            (Platform::Windows, "Control+A"),
            (Platform::Mac, "Meta+A"),
        ] {
            let smart = Keyboard::new(platform).press("ControlOrMeta+A", Duration::ZERO)?;
            let plain = Keyboard::new(platform).press(meant, Duration::ZERO)?;
            assert_eq!(
                sent(&smart),
                sent(&plain),
                "ControlOrMeta is {meant} on {platform:?}"
            );
        }
        let linux = Keyboard::new(Platform::Linux);
        let mac = Keyboard::new(Platform::Mac);
        assert_eq!(
            linux.modifier_named("ControlOrMeta"),
            Some(Modifier::Control)
        );
        assert_eq!(mac.modifier_named("ControlOrMeta"), Some(Modifier::Meta));
        assert_eq!(mac.modifier_named("Shift"), Some(Modifier::Shift));
        assert_eq!(
            mac.modifier_named("KeyA"),
            None,
            "KeyA is a key, not a modifier"
        );
        Ok(())
    }

    #[test]
    fn tests_that_the_platform_comes_from_the_user_agent_as_upstream_reads_it() {
        let agents = [
            (
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Chrome/146.0.0.0",
                Platform::Mac,
            ),
            (
                "Mozilla/5.0 (X11; Linux x86_64) Chrome/146.0.0.0",
                Platform::Linux,
            ),
            (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/146.0.0.0",
                Platform::Windows,
            ),
        ];
        for (agent, platform) in agents {
            assert_eq!(Platform::from_user_agent(agent), platform, "{agent}");
        }
    }

    #[test]
    fn tests_that_an_unknown_key_fails_the_whole_call_with_nothing_held()
    -> Result<(), Box<dyn Error>> {
        let mut keyboard = Keyboard::new(Platform::Linux);
        let failed = keyboard.press("Shift+Foo", Duration::ZERO);
        assert_eq!(
            failed.err().map(|error| error.to_string()),
            Some("Unknown key: \"Foo\"".to_owned())
        );
        let after = sent(&keyboard.press("a", Duration::ZERO)?);
        assert_eq!(after[0]["params"]["modifiers"], 0.0, "Shift was never held");
        assert_eq!(after[0]["params"]["text"], "a");
        let empty = keyboard.press("Shift+", Duration::ZERO);
        assert_eq!(
            empty.err().map(|error| error.to_string()),
            Some("Unknown key: \"\"".to_owned())
        );
        Ok(())
    }

    #[test]
    fn tests_that_a_plus_is_a_key_where_it_cannot_be_a_separator() -> Result<(), Box<dyn Error>> {
        let steps = Keyboard::new(Platform::Linux).press("Shift++", Duration::ZERO)?;
        let shifted_plus = sent(&steps);
        let keys: Vec<&str> = shifted_plus
            .iter()
            .filter_map(|message| message["params"]["key"].as_str())
            .collect();
        assert_eq!(
            keys,
            ["Shift", "+", "+", "Shift"],
            "Shift down, + down, + up, Shift up"
        );
        assert_eq!(
            shifted_plus[1]["params"]["code"], "Equal",
            "+ is Shift+Equal on the US layout"
        );
        Ok(())
    }

    /// Upstream's `press` waits between the last key's down and up; `type` passes the delay
    /// to each press and waits before each inserted character.
    #[test]
    fn tests_that_delays_fall_where_upstream_waits() -> Result<(), Box<dyn Error>> {
        let delay = Duration::from_millis(50);
        let press = Keyboard::new(Platform::Linux).press("Shift+a", delay)?;
        assert_eq!(
            timeline(&press),
            [
                "Input.dispatchKeyEvent rawKeyDown",
                "Input.dispatchKeyEvent keyDown",
                "wait 50ms",
                "Input.dispatchKeyEvent keyUp",
                "Input.dispatchKeyEvent keyUp",
            ]
        );
        let typed = Keyboard::new(Platform::Linux).type_text("aé", delay);
        assert_eq!(
            timeline(&typed),
            [
                "Input.dispatchKeyEvent keyDown",
                "wait 50ms",
                "Input.dispatchKeyEvent keyUp",
                "wait 50ms",
                "Input.insertText text",
            ]
        );
        let quick = Keyboard::new(Platform::Linux).type_text("aé", Duration::ZERO);
        assert!(
            !timeline(&quick).iter().any(|step| step.starts_with("wait")),
            "no delay, no waits"
        );
        Ok(())
    }

    /// Upstream's `ensureModifiers` walks Alt, Control, Meta, Shift, and the restore it returns
    /// undoes exactly what it changed.
    #[test]
    fn tests_that_ensure_modifiers_holds_exactly_the_wanted_set_and_restores()
    -> Result<(), Box<dyn Error>> {
        let mut keyboard = Keyboard::new(Platform::Linux);
        keyboard.down("Alt")?;
        let wanted = Modifiers::default()
            .with(Modifier::Shift)
            .with(Modifier::Control);
        let (restore, steps) = keyboard.ensure_modifiers(wanted)?;
        let changed: Vec<String> = sent(&steps)
            .iter()
            .map(|message| {
                format!(
                    "{} {}",
                    message["params"]["type"].as_str().unwrap_or_default(),
                    message["params"]["key"].as_str().unwrap_or_default()
                )
            })
            .collect();
        assert_eq!(
            changed,
            ["keyUp Alt", "rawKeyDown Control", "rawKeyDown Shift"]
        );
        assert_eq!(keyboard.modifiers(), wanted);
        let (_, back) = keyboard.ensure_modifiers(restore)?;
        assert_eq!(back.len(), 3, "Alt down again, Control and Shift up");
        assert_eq!(
            keyboard.modifiers(),
            Modifiers::default().with(Modifier::Alt)
        );
        Ok(())
    }

    #[test]
    fn tests_that_modifiers_and_aliases_name_keys_and_numpad_keys_never_take_a_character() {
        for name in ["Shift", "Control", "Alt", "Meta", "\n", "\r"] {
            assert!(
                KEYS.contains_key(name),
                "{name:?} names a key, as upstream's aliases make it"
            );
        }
        assert_eq!(
            KEYS.get("1").map(|key| key.code),
            Some("Digit1"),
            "1 is Digit1, never Numpad1"
        );
    }
}
