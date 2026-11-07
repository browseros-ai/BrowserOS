//! Playwright's keyboard for Chromium, and the writer that sends it to the run's tab:
//! `server/input.ts` (`Keyboard`, `buildLayoutClosure`, `resolveSmartModifierString`),
//! `usKeyboardLayout.ts`, `macEditingCommands.ts` and `chromium/crInput.ts` (`RawKeyboardImpl`),
//! as of Playwright 26137face; none of the four changed since v1.64.1.
//!
//! ```text
//!  plan_press(key, platform)         pure: Playwright's `+` split, the US layout, one press's
//!    └─ Vec<KeyEvent>                modifier state, crInput's `Input.dispatchKeyEvent` params
//!  KeyboardTab::write_keys(events)   detached writer on the run's tracker: one event at a time,
//!    └─ PressReport                  `delay` before the keyUp, and a keyUp for every keyDown
//!                                    it may have written, however the call ends
//!  KeyboardTab::insert_text(text)    one `Input.insertText`, the same way (fill's typing)
//!  KeyboardTab::platform()           `Browser.getVersion`'s userAgent → mac, win or linux
//! ```
//!
//! Policy (press, fill) plans the keys and decides retries; nothing here writes a call-log line
//! or names a locator. Neo-only behaviour, each deliberate:
//! - Every press starts and ends with no key held. Playwright keeps one keyboard per page, and a
//!   key it pressed stays down until something releases it; Neo has no `page.keyboard` yet, so
//!   nothing may leave a key down between calls.
//! - A press is planned whole before anything is written, so an unknown token writes nothing.
//!   Playwright writes the keyDowns before the bad token, then throws and leaves them held:
//!   `Shift+Foo` leaves Shift down in the tab, corrupting every later keystroke there.
//! - After cancellation or a CDP error the writer still sends the keyUp of every keyDown that
//!   may have been written, newest first. Playwright's press stops at the error, keys held.
//! - `ControlOrMeta` follows the browser's platform, as the mac `commands` do; Playwright reads
//!   its own process's platform. Neo's server runs on the browser's machine, so they agree.
//! - `Escape` never tries to cancel a drag first: Neo has no drag manager.

use std::collections::HashMap;
use std::future::Future;
use std::mem;
use std::pin::Pin;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use browseros_cdp::CdpError;
use serde::Serialize;
use serde_json::json;
use tokio::sync::oneshot;
use tokio::time;
use tokio_util::sync::CancellationToken;

use crate::click::{PageError, PressReport, Tab};
use crate::page::{FrameTab, InputPermit, PageSession};
use crate::progress::InputTicket;
use crate::world;

// ---------------------------------------------------------------------------------------------
// The seam: what a press or a fill's typing needs from a tab, beyond click's.
// ---------------------------------------------------------------------------------------------

/// The keyboard of the run's tab. Each method is one Playwright step; every returned future is
/// cancel-safe, as [`Tab`]'s are. Implemented by `Arc<PageSession>` here and by click's
/// `FakeTab` in the tests, whose writes go through the real writer.
pub(crate) trait KeyboardTab: Tab {
    /// The browser's platform: `Browser.getVersion`'s userAgent, read as Playwright's
    /// `crBrowser.ts` `_platform` reads it. It decides `ControlOrMeta` and the mac `commands`.
    fn platform(&self) -> impl Future<Output = Result<Platform, PageError>> + Send;

    /// Starts writing `events` on a detached writer task *before it returns*, and gives back a
    /// future of the writer's report: [`Tab::press`]'s contract, for keys. `events` are one
    /// [`plan_press`], or several in a row; `delay` is slept between a keyDown and the keyUp
    /// right after it (`Keyboard.press`'s `delay`).
    ///
    /// The ticket moves into the writer, which marks it however the caller ends, so dropping the
    /// returned future (the call stopped) stops the wait, never the writes: it cuts a `delay`
    /// short and starts no further keyDown, and every key the writer may have pressed is still
    /// released, newest first. Starting eagerly matters: a future that started the writer only
    /// when polled could be dropped unpolled by a `Progress::race` that had already lost, leaving
    /// a begun ticket nobody marks. The writer holds `input` until its last event is answered,
    /// so the tab's next input waits for the release.
    fn write_keys(
        &self,
        events: Vec<KeyEvent>,
        delay: Option<Duration>,
        ticket: InputTicket,
        input: Self::Input,
    ) -> impl Future<Output = PressReport> + Send;

    /// `Input.insertText {text}` (`crInput.ts` `sendText`), started the same way, with the same
    /// report. A call that stopped before the writer ran writes nothing.
    fn insert_text(
        &self,
        text: &str,
        ticket: InputTicket,
        input: Self::Input,
    ) -> impl Future<Output = PressReport> + Send;
}

impl KeyboardTab for FrameTab {
    async fn platform(&self) -> Result<Platform, PageError> {
        let user_agent = self
            .user_agent()
            .await
            .map_err(|error| world::classify(&error))?;
        Ok(Platform::from_user_agent(&user_agent))
    }

    fn write_keys(
        &self,
        events: Vec<KeyEvent>,
        delay: Option<Duration>,
        ticket: InputTicket,
        input: InputPermit,
    ) -> impl Future<Output = PressReport> + Send {
        let cut_short = CancellationToken::new();
        let writer = write_keys(Arc::clone(self), events, delay, ticket, cut_short.clone());
        detach(|task| self.spawn(task), writer, input, cut_short)
    }

    fn insert_text(
        &self,
        text: &str,
        ticket: InputTicket,
        input: InputPermit,
    ) -> impl Future<Output = PressReport> + Send {
        let cut_short = CancellationToken::new();
        let writer = insert_text(
            Arc::clone(self),
            text.to_string(),
            ticket,
            cut_short.clone(),
        );
        detach(|task| self.spawn(task), writer, input, cut_short)
    }
}

/// Where a writer's commands go, keys here and the mouse and touch writers in `page.rs`: the
/// run's session (`page.rs` `send_input_json`, at the session's socket incarnation), or a
/// recording fake under paused time in the tests.
pub(crate) trait InputSender: Send + Sync + 'static {
    /// Sends one `Input.*` command, its params already serialized.
    fn send_input(
        &self,
        method: &'static str,
        params: String,
    ) -> impl Future<Output = Result<(), CdpError>> + Send;
}

impl InputSender for Arc<PageSession> {
    async fn send_input(&self, method: &'static str, params: String) -> Result<(), CdpError> {
        self.send_input_json(method, &params).await
    }
}

// ---------------------------------------------------------------------------------------------
// The writer.
// ---------------------------------------------------------------------------------------------

/// A writer, boxed so one spawning seam serves the run's tracker and the tests' runtime.
type WriterTask = Pin<Box<dyn Future<Output = ()> + Send>>;

/// Spawns `writer` now through `spawn`, holding `input` until the writer has reported, and
/// returns a future of the report holding a drop guard on the writer's `cut_short` token.
fn detach(
    spawn: impl FnOnce(WriterTask),
    writer: impl Future<Output = PressReport> + Send + 'static,
    input: impl Send + 'static,
    cut_short: CancellationToken,
) -> impl Future<Output = PressReport> + Send {
    let (report, receiver) = oneshot::channel();
    spawn(Box::pin(async move {
        let written = writer.await;
        // Every event is answered: the tab's next input may start.
        drop(input);
        // Nobody may be waiting any more; the ticket already holds the outcome.
        let _waiting = report.send(written);
    }));
    let cut_short = cut_short.drop_guard();
    async move {
        let written = receiver.await.unwrap_or_else(|_| PressReport::Failed {
            closed: true,
            message: "the key writer stopped before reporting".to_string(),
        });
        drop(cut_short);
        written
    }
}

/// The key writer. It owns its events and the ticket that records them, and holds the sender
/// (for the real tab, the page and so the session) until its last event is answered.
async fn write_keys(
    sender: impl InputSender,
    events: Vec<KeyEvent>,
    delay: Option<Duration>,
    ticket: InputTicket,
    cut_short: CancellationToken,
) -> PressReport {
    let outcomes = send_keys(&sender, &events, delay, &cut_short).await;
    // Every planned event goes out unless the writer stopped early: then the press is partial.
    let complete = outcomes.len() == events.len();
    report(&outcomes, complete, ticket)
}

/// Playwright's sequential `Keyboard.press`: each event is answered before the next is sent,
/// with `delay` between a keyDown and the keyUp right after it. After `cut_short` or a failed
/// event no further keyDown is sent, but every keyUp whose key a sent keyDown may have pressed
/// still is: a failed keyDown counts as pressed, since it may have reached the browser. A
/// refused send (`ConnectionEpochChanged`, raised before writing) ends everything: the socket
/// that carried the session is gone, so nothing later can be written either.
async fn send_keys(
    sender: &impl InputSender,
    events: &[KeyEvent],
    delay: Option<Duration>,
    cut_short: &CancellationToken,
) -> Vec<Result<(), CdpError>> {
    let mut outcomes = Vec::with_capacity(events.len());
    // The code of every keyDown that may have been written and has no keyUp yet, oldest first.
    let mut held: Vec<&'static str> = Vec::new();
    let mut pressing = true;
    for (index, event) in events.iter().enumerate() {
        match event {
            KeyEvent::Down(down) => {
                if !pressing || cut_short.is_cancelled() {
                    pressing = false;
                    continue;
                }
                let written = send_key(sender, event).await;
                if refused(&written) {
                    outcomes.push(written);
                    break;
                }
                held.push(down.code);
                pressing = written.is_ok();
                outcomes.push(written);
                if pressing
                    && let (Some(delay), Some(KeyEvent::Up(_))) = (delay, events.get(index + 1))
                {
                    pause(delay, cut_short).await;
                }
            }
            KeyEvent::Up(up) => {
                // Only a key this writer may have pressed is released.
                let Some(at) = held.iter().rposition(|code| *code == up.code) else {
                    continue;
                };
                held.remove(at);
                let written = send_key(sender, event).await;
                let stop = refused(&written);
                pressing &= written.is_ok();
                outcomes.push(written);
                if stop {
                    break;
                }
            }
        }
    }
    outcomes
}

/// `insertText` as a writer: nothing is written once the call has stopped.
async fn insert_text(
    sender: impl InputSender,
    text: String,
    ticket: InputTicket,
    cut_short: CancellationToken,
) -> PressReport {
    let mut outcomes = Vec::with_capacity(1);
    if !cut_short.is_cancelled() {
        let params = json!({ "text": text }).to_string();
        outcomes.push(sender.send_input("Input.insertText", params).await);
    }
    let complete = !outcomes.is_empty();
    report(&outcomes, complete, ticket)
}

/// Every input writer's rule, keys here and the mouse and touch writers in `page.rs`: the whole
/// planned sequence written (`complete`) and every event acknowledged → `ticket.acked()`; nothing
/// sent, or every send refused with `ConnectionEpochChanged` → `ticket.unsent()`; otherwise the
/// ticket drops (`Unknown`, design §4.7: a call that stopped part-way, a double click cut short
/// after its first click included) and the first error, or the cut, is reported.
pub(crate) fn report(
    outcomes: &[Result<(), CdpError>],
    complete: bool,
    ticket: InputTicket,
) -> PressReport {
    match outcomes.iter().find_map(|outcome| outcome.as_ref().err()) {
        None if outcomes.is_empty() => {
            ticket.unsent();
            PressReport::NothingWritten
        }
        None if complete => PressReport::Acked(ticket.acked()),
        None => {
            // The call stopped part-way: what was written was answered, the rest never went.
            drop(ticket);
            PressReport::Failed {
                closed: false,
                message: "the input stopped before its last event".to_string(),
            }
        }
        Some(_) if outcomes.iter().all(refused) => {
            ticket.unsent();
            PressReport::NothingWritten
        }
        Some(error) => {
            drop(ticket);
            PressReport::Failed {
                closed: world::classify(error) == PageError::Closed,
                message: error.to_string(),
            }
        }
    }
}

/// One key event. Serializing these plain structs cannot fail; if it ever did, the error counts
/// as a failed event, so delivery stays `Unknown` rather than claiming nothing was written.
async fn send_key(sender: &impl InputSender, event: &KeyEvent) -> Result<(), CdpError> {
    let params = serde_json::to_string(event)?;
    sender.send_input("Input.dispatchKeyEvent", params).await
}

fn refused(written: &Result<(), CdpError>) -> bool {
    matches!(written, Err(CdpError::ConnectionEpochChanged { .. }))
}

/// A `delay` sleep that the call stopping ends early.
async fn pause(delay: Duration, cut_short: &CancellationToken) {
    tokio::select! {
        () = time::sleep(delay) => {}
        () = cut_short.cancelled() => {}
    }
}

// ---------------------------------------------------------------------------------------------
// Planning: Playwright's keyboard, one press at a time.
// ---------------------------------------------------------------------------------------------

/// The browser's platform, by Playwright's `crBrowser.ts` `_platform`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Platform {
    Mac,
    Linux,
    Win,
}

impl Platform {
    /// `Windows` in the userAgent is win, else `Macintosh` is mac, else linux.
    pub(crate) fn from_user_agent(user_agent: &str) -> Self {
        if user_agent.contains("Windows") {
            Self::Win
        } else if user_agent.contains("Macintosh") {
            Self::Mac
        } else {
            Self::Linux
        }
    }
}

/// A token the US layout does not know. The message is Playwright's
/// (`input.ts` `_keyDescriptionForString`), naming the one token, not the whole key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Unknown key: \"{key}\"")]
pub(crate) struct UnknownKey {
    pub(crate) key: String,
}

/// One `Input.dispatchKeyEvent`, serialized exactly as `crInput.ts` builds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum KeyEvent {
    Down(KeyDown),
    Up(KeyUp),
}

/// `RawKeyboardImpl.keydown`'s params, in its field order. Every field is always present:
/// `commands` is empty off mac and `text` empty for a key that types nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KeyDown {
    /// `keyDown` when the key types text, else `rawKeyDown`.
    #[serde(rename = "type")]
    pub(crate) kind: &'static str,
    pub(crate) modifiers: u8,
    pub(crate) windows_virtual_key_code: u32,
    pub(crate) code: &'static str,
    pub(crate) commands: Vec<&'static str>,
    pub(crate) key: &'static str,
    pub(crate) text: &'static str,
    pub(crate) unmodified_text: &'static str,
    pub(crate) auto_repeat: bool,
    pub(crate) location: u32,
    pub(crate) is_keypad: bool,
}

/// `RawKeyboardImpl.keyup`'s params, in its field order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KeyUp {
    /// Always `keyUp`.
    #[serde(rename = "type")]
    pub(crate) kind: &'static str,
    pub(crate) modifiers: u8,
    pub(crate) key: &'static str,
    pub(crate) windows_virtual_key_code: u32,
    pub(crate) code: &'static str,
    pub(crate) location: u32,
    pub(crate) is_keypad: bool,
}

/// Playwright's `Keyboard.press(key)` as the events it sends: the key split on `+`, a keyDown
/// per token in order, then a keyUp per token in reverse, with the modifier state Playwright
/// keeps, starting from none. Every token is looked up before any event exists, so an unknown
/// one fails the whole press and nothing is sent; the error names the first unknown token, the
/// one at whose keyDown Playwright throws.
pub(crate) fn plan_press(key: &str, platform: Platform) -> Result<Vec<KeyEvent>, UnknownKey> {
    let tokens = split(key);
    let mut keyboard = Keyboard::default();
    let mut events = Vec::with_capacity(tokens.len() * 2);
    for token in &tokens {
        events.push(KeyEvent::Down(keyboard.down(token, platform)?));
    }
    for token in tokens.iter().rev() {
        events.push(KeyEvent::Up(keyboard.up(token, platform)?));
    }
    Ok(events)
}

/// `Keyboard.press`'s `split`: a `+` ends a token only when the token so far is not empty, so a
/// leading or lone `+` is a key (`+`, `Shift++`), and a trailing one leaves an empty last token
/// (`a+` is `a` and an empty name, an unknown key).
fn split(key: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut building = String::new();
    for c in key.chars() {
        if c == '+' && !building.is_empty() {
            tokens.push(mem::take(&mut building));
        } else {
            building.push(c);
        }
    }
    tokens.push(building);
    tokens
}

/// `resolveSmartModifierString`, by the browser's platform.
fn resolve_smart_modifier(key: &str, platform: Platform) -> &str {
    match (key, platform) {
        ("ControlOrMeta", Platform::Mac) => "Meta",
        ("ControlOrMeta", Platform::Linux | Platform::Win) => "Control",
        _ => key,
    }
}

/// `toModifiersMask`'s bits (`crProtocolHelper.ts`).
const ALT: u8 = 1;
const CONTROL: u8 = 2;
const META: u8 = 4;
const SHIFT: u8 = 8;

/// `kModifiers`: the keys that are modifiers, by their `key`.
fn modifier_bit(key: &str) -> Option<u8> {
    match key {
        "Alt" => Some(ALT),
        "Control" => Some(CONTROL),
        "Meta" => Some(META),
        "Shift" => Some(SHIFT),
        _ => None,
    }
}

/// Playwright's `Keyboard` state for one press: the modifiers held (as the mask) and the codes
/// of the keys held, which make a second keyDown of the same code an `autoRepeat`.
#[derive(Debug, Default)]
struct Keyboard {
    modifiers: u8,
    pressed: Vec<&'static str>,
}

impl Keyboard {
    /// `_keyDescriptionForString`: the shifted variant while Shift is held, and no text while
    /// any other modifier is.
    fn description(&self, key: &str, platform: Platform) -> Result<KeyDescription, UnknownKey> {
        let key = resolve_smart_modifier(key, platform);
        let entry = LAYOUT.get(key).ok_or_else(|| UnknownKey {
            key: key.to_string(),
        })?;
        let description = match entry.shifted {
            Some(shifted) if self.modifiers & SHIFT != 0 => shifted,
            _ => entry.description,
        };
        Ok(if self.modifiers & !SHIFT == 0 {
            description
        } else {
            KeyDescription {
                text: "",
                ..description
            }
        })
    }

    /// `Keyboard.down` and `RawKeyboardImpl.keydown`: the key's own modifier is held before its
    /// params are built, so a Shift keyDown already carries Shift.
    fn down(&mut self, key: &str, platform: Platform) -> Result<KeyDown, UnknownKey> {
        let description = self.description(key, platform)?;
        let auto_repeat = self.pressed.contains(&description.code);
        if !auto_repeat {
            self.pressed.push(description.code);
        }
        if let Some(bit) = modifier_bit(description.key) {
            self.modifiers |= bit;
        }
        Ok(KeyDown {
            kind: if description.text.is_empty() {
                "rawKeyDown"
            } else {
                "keyDown"
            },
            modifiers: self.modifiers,
            windows_virtual_key_code: description.key_code_without_location,
            code: description.code,
            commands: commands_for_code(description.code, self.modifiers, platform),
            key: description.key,
            text: description.text,
            unmodified_text: description.text,
            auto_repeat,
            location: description.location,
            is_keypad: description.location == KEYPAD_LOCATION,
        })
    }

    /// `Keyboard.up` and `RawKeyboardImpl.keyup`: the description is read while the key is still
    /// held, its modifier released before the params are built.
    fn up(&mut self, key: &str, platform: Platform) -> Result<KeyUp, UnknownKey> {
        let description = self.description(key, platform)?;
        if let Some(bit) = modifier_bit(description.key) {
            self.modifiers &= !bit;
        }
        self.pressed.retain(|code| *code != description.code);
        Ok(KeyUp {
            kind: "keyUp",
            modifiers: self.modifiers,
            key: description.key,
            windows_virtual_key_code: description.key_code_without_location,
            code: description.code,
            location: description.location,
            is_keypad: description.location == KEYPAD_LOCATION,
        })
    }
}

/// `RawKeyboardImpl._commandsForCode`: on mac only, the editing commands of the shortcut that
/// names the held modifiers in Shift, Control, Alt, Meta order, then the code; commands that
/// insert text are dropped and the trailing `:` removed, matching Chromium's command names.
fn commands_for_code(code: &str, modifiers: u8, platform: Platform) -> Vec<&'static str> {
    if platform != Platform::Mac {
        return Vec::new();
    }
    let mut shortcut = String::new();
    for (bit, name) in [
        (SHIFT, "Shift"),
        (CONTROL, "Control"),
        (ALT, "Alt"),
        (META, "Meta"),
    ] {
        if modifiers & bit != 0 {
            shortcut.push_str(name);
            shortcut.push('+');
        }
    }
    shortcut.push_str(code);
    let commands = MAC_EDITING_COMMANDS
        .iter()
        .find(|(name, _)| *name == shortcut)
        .map_or(&[][..], |(_, commands)| *commands);
    commands
        .iter()
        .filter(|command| !command.starts_with("insert"))
        .map(|command| command.strip_suffix(':').unwrap_or(command))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// The US layout.
// ---------------------------------------------------------------------------------------------

/// `keyboardLayout.keypadLocation`.
const KEYPAD_LOCATION: u32 = 3;

/// One key as the layout describes it (`input.ts` `KeyDescription`), without `keyCode`: only
/// Playwright's Firefox and `WebKit` keyboards send it; Chromium's gets `keyCodeWithoutLocation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeyDescription {
    key: &'static str,
    key_code_without_location: u32,
    code: &'static str,
    text: &'static str,
    location: u32,
}

/// What a name in the layout closure reaches: a description and, for a code, its shifted
/// variant. Keys, shifted keys and aliases reach a description with none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LayoutEntry {
    description: KeyDescription,
    shifted: Option<KeyDescription>,
}

/// `buildLayoutClosure(USKeyboardLayout)`: every name `Keyboard.press` accepts.
static LAYOUT: LazyLock<HashMap<&'static str, LayoutEntry>> =
    LazyLock::new(|| build_layout_closure(US_KEYBOARD_LAYOUT));

/// `input.ts` `aliases`: further names for a code's unshifted description.
const ALIASES: &[(&str, &[&str])] = &[
    ("ShiftLeft", &["Shift"]),
    ("ControlLeft", &["Control"]),
    ("AltLeft", &["Alt"]),
    ("MetaLeft", &["Meta"]),
    ("Enter", &["\n", "\r"]),
];

/// Playwright's `buildLayoutClosure`, entry for entry: later entries replace earlier ones under
/// the same name, as `Map.set` does (the US layout has no such collision).
fn build_layout_closure(layout: &[KeyDefinition]) -> HashMap<&'static str, LayoutEntry> {
    let mut result = HashMap::new();
    for definition in layout {
        // Every key in the table is ASCII, so one byte is JavaScript's `key.length === 1`.
        let single = definition.key.len() == 1;
        let description = KeyDescription {
            key: definition.key,
            key_code_without_location: definition
                .key_code_without_location
                .unwrap_or(definition.key_code),
            code: definition.code,
            text: if single {
                definition.key
            } else {
                definition.text.unwrap_or("")
            },
            location: definition.location,
        };
        let shifted = definition.shift_key.map(|shift_key| KeyDescription {
            key: shift_key,
            text: shift_key,
            key_code_without_location: definition
                .shift_key_code
                .unwrap_or(description.key_code_without_location),
            ..description
        });
        result.insert(
            definition.code,
            LayoutEntry {
                description,
                shifted,
            },
        );
        let unshifted = LayoutEntry {
            description,
            shifted: None,
        };
        for (code, aliases) in ALIASES {
            if *code == definition.code {
                for alias in *aliases {
                    result.insert(alias, unshifted);
                }
            }
        }
        // Do not use numpad when converting keys to codes.
        if definition.location != 0 {
            continue;
        }
        if single {
            result.insert(definition.key, unshifted);
        }
        if let Some(shifted) = shifted {
            result.insert(
                shifted.key,
                LayoutEntry {
                    description: shifted,
                    shifted: None,
                },
            );
        }
    }
    result
}

/// One row of `usKeyboardLayout.ts` (`KeyDefinition`).
#[derive(Debug, Clone, Copy)]
struct KeyDefinition {
    code: &'static str,
    key: &'static str,
    key_code: u32,
    key_code_without_location: Option<u32>,
    shift_key: Option<&'static str>,
    shift_key_code: Option<u32>,
    text: Option<&'static str>,
    location: u32,
}

/// A row with `code`, `keyCode` and `key`; the methods below add the optional fields, so the
/// table reads like Playwright's.
const fn def(code: &'static str, key_code: u32, key: &'static str) -> KeyDefinition {
    KeyDefinition {
        code,
        key,
        key_code,
        key_code_without_location: None,
        shift_key: None,
        shift_key_code: None,
        text: None,
        location: 0,
    }
}

impl KeyDefinition {
    const fn without_location(self, key_code: u32) -> Self {
        Self {
            key_code_without_location: Some(key_code),
            ..self
        }
    }

    const fn shift(self, shift_key: &'static str) -> Self {
        Self {
            shift_key: Some(shift_key),
            ..self
        }
    }

    const fn shift_code(self, key_code: u32) -> Self {
        Self {
            shift_key_code: Some(key_code),
            ..self
        }
    }

    const fn text(self, text: &'static str) -> Self {
        Self {
            text: Some(text),
            ..self
        }
    }

    const fn at(self, location: u32) -> Self {
        Self { location, ..self }
    }
}

/// `USKeyboardLayout`, row for row, generated from `usKeyboardLayout.ts`. Unformatted, so each
/// row stays one line, as in Playwright's table.
#[rustfmt::skip]
const US_KEYBOARD_LAYOUT: &[KeyDefinition] = &[
    // Functions row
    def("Escape", 27, "Escape"),
    def("F1", 112, "F1"),
    def("F2", 113, "F2"),
    def("F3", 114, "F3"),
    def("F4", 115, "F4"),
    def("F5", 116, "F5"),
    def("F6", 117, "F6"),
    def("F7", 118, "F7"),
    def("F8", 119, "F8"),
    def("F9", 120, "F9"),
    def("F10", 121, "F10"),
    def("F11", 122, "F11"),
    def("F12", 123, "F12"),
    // Numbers row
    def("Backquote", 192, "`").shift("~"),
    def("Digit1", 49, "1").shift("!"),
    def("Digit2", 50, "2").shift("@"),
    def("Digit3", 51, "3").shift("#"),
    def("Digit4", 52, "4").shift("$"),
    def("Digit5", 53, "5").shift("%"),
    def("Digit6", 54, "6").shift("^"),
    def("Digit7", 55, "7").shift("&"),
    def("Digit8", 56, "8").shift("*"),
    def("Digit9", 57, "9").shift("("),
    def("Digit0", 48, "0").shift(")"),
    def("Minus", 189, "-").shift("_"),
    def("Equal", 187, "=").shift("+"),
    def("Backslash", 220, "\\").shift("|"),
    def("Backspace", 8, "Backspace"),
    // First row
    def("Tab", 9, "Tab"),
    def("KeyQ", 81, "q").shift("Q"),
    def("KeyW", 87, "w").shift("W"),
    def("KeyE", 69, "e").shift("E"),
    def("KeyR", 82, "r").shift("R"),
    def("KeyT", 84, "t").shift("T"),
    def("KeyY", 89, "y").shift("Y"),
    def("KeyU", 85, "u").shift("U"),
    def("KeyI", 73, "i").shift("I"),
    def("KeyO", 79, "o").shift("O"),
    def("KeyP", 80, "p").shift("P"),
    def("BracketLeft", 219, "[").shift("{"),
    def("BracketRight", 221, "]").shift("}"),
    // Second row
    def("CapsLock", 20, "CapsLock"),
    def("KeyA", 65, "a").shift("A"),
    def("KeyS", 83, "s").shift("S"),
    def("KeyD", 68, "d").shift("D"),
    def("KeyF", 70, "f").shift("F"),
    def("KeyG", 71, "g").shift("G"),
    def("KeyH", 72, "h").shift("H"),
    def("KeyJ", 74, "j").shift("J"),
    def("KeyK", 75, "k").shift("K"),
    def("KeyL", 76, "l").shift("L"),
    def("Semicolon", 186, ";").shift(":"),
    def("Quote", 222, "'").shift("\""),
    def("Enter", 13, "Enter").text("\r"),
    // Third row
    def("ShiftLeft", 160, "Shift").without_location(16).at(1),
    def("KeyZ", 90, "z").shift("Z"),
    def("KeyX", 88, "x").shift("X"),
    def("KeyC", 67, "c").shift("C"),
    def("KeyV", 86, "v").shift("V"),
    def("KeyB", 66, "b").shift("B"),
    def("KeyN", 78, "n").shift("N"),
    def("KeyM", 77, "m").shift("M"),
    def("Comma", 188, ",").shift("<"),
    def("Period", 190, ".").shift(">"),
    def("Slash", 191, "/").shift("?"),
    def("ShiftRight", 161, "Shift").without_location(16).at(2),
    // Last row
    def("ControlLeft", 162, "Control").without_location(17).at(1),
    def("MetaLeft", 91, "Meta").at(1),
    def("AltLeft", 164, "Alt").without_location(18).at(1),
    def("Space", 32, " "),
    def("AltRight", 165, "Alt").without_location(18).at(2),
    def("AltGraph", 225, "AltGraph"),
    def("MetaRight", 92, "Meta").at(2),
    def("ContextMenu", 93, "ContextMenu"),
    def("ControlRight", 163, "Control").without_location(17).at(2),
    // Center block
    def("PrintScreen", 44, "PrintScreen"),
    def("ScrollLock", 145, "ScrollLock"),
    def("Pause", 19, "Pause"),
    def("PageUp", 33, "PageUp"),
    def("PageDown", 34, "PageDown"),
    def("Insert", 45, "Insert"),
    def("Delete", 46, "Delete"),
    def("Home", 36, "Home"),
    def("End", 35, "End"),
    def("ArrowLeft", 37, "ArrowLeft"),
    def("ArrowUp", 38, "ArrowUp"),
    def("ArrowRight", 39, "ArrowRight"),
    def("ArrowDown", 40, "ArrowDown"),
    // Media keys
    def("AudioVolumeMute", 173, "AudioVolumeMute"),
    def("AudioVolumeDown", 174, "AudioVolumeDown"),
    def("AudioVolumeUp", 175, "AudioVolumeUp"),
    def("MediaTrackNext", 176, "MediaTrackNext"),
    def("MediaTrackPrevious", 177, "MediaTrackPrevious"),
    def("MediaPlayPause", 179, "MediaPlayPause"),
    // Numpad
    def("NumLock", 144, "NumLock"),
    def("NumpadDivide", 111, "/").at(3),
    def("NumpadMultiply", 106, "*").at(3),
    def("NumpadSubtract", 109, "-").at(3),
    def("Numpad7", 36, "Home").shift("7").shift_code(103).at(3),
    def("Numpad8", 38, "ArrowUp").shift("8").shift_code(104).at(3),
    def("Numpad9", 33, "PageUp").shift("9").shift_code(105).at(3),
    def("Numpad4", 37, "ArrowLeft").shift("4").shift_code(100).at(3),
    def("Numpad5", 12, "Clear").shift("5").shift_code(101).at(3),
    def("Numpad6", 39, "ArrowRight").shift("6").shift_code(102).at(3),
    def("NumpadAdd", 107, "+").at(3),
    def("Numpad1", 35, "End").shift("1").shift_code(97).at(3),
    def("Numpad2", 40, "ArrowDown").shift("2").shift_code(98).at(3),
    def("Numpad3", 34, "PageDown").shift("3").shift_code(99).at(3),
    def("Numpad0", 45, "Insert").shift("0").shift_code(96).at(3),
    def("NumpadDecimal", 46, "Delete").shift(".").shift_code(110).at(3),
    def("NumpadEnter", 13, "Enter").text("\r").at(3),
];

/// `macEditingCommands`, entry for entry (a single command as a one-element list, as
/// `_commandsForCode` reads it), generated from `macEditingCommands.ts`.
const MAC_EDITING_COMMANDS: &[(&str, &[&str])] = &[
    ("Backspace", &["deleteBackward:"]),
    ("Enter", &["insertNewline:"]),
    ("NumpadEnter", &["insertNewline:"]),
    ("Escape", &["cancelOperation:"]),
    ("ArrowUp", &["moveUp:"]),
    ("ArrowDown", &["moveDown:"]),
    ("ArrowLeft", &["moveLeft:"]),
    ("ArrowRight", &["moveRight:"]),
    ("F5", &["complete:"]),
    ("Delete", &["deleteForward:"]),
    ("Home", &["scrollToBeginningOfDocument:"]),
    ("End", &["scrollToEndOfDocument:"]),
    ("PageUp", &["scrollPageUp:"]),
    ("PageDown", &["scrollPageDown:"]),
    ("Shift+Backspace", &["deleteBackward:"]),
    ("Shift+Enter", &["insertNewline:"]),
    ("Shift+NumpadEnter", &["insertNewline:"]),
    ("Shift+Escape", &["cancelOperation:"]),
    ("Shift+ArrowUp", &["moveUpAndModifySelection:"]),
    ("Shift+ArrowDown", &["moveDownAndModifySelection:"]),
    ("Shift+ArrowLeft", &["moveLeftAndModifySelection:"]),
    ("Shift+ArrowRight", &["moveRightAndModifySelection:"]),
    ("Shift+F5", &["complete:"]),
    ("Shift+Delete", &["deleteForward:"]),
    (
        "Shift+Home",
        &["moveToBeginningOfDocumentAndModifySelection:"],
    ),
    ("Shift+End", &["moveToEndOfDocumentAndModifySelection:"]),
    ("Shift+PageUp", &["pageUpAndModifySelection:"]),
    ("Shift+PageDown", &["pageDownAndModifySelection:"]),
    ("Shift+Numpad5", &["delete:"]),
    ("Control+Tab", &["selectNextKeyView:"]),
    ("Control+Enter", &["insertLineBreak:"]),
    ("Control+NumpadEnter", &["insertLineBreak:"]),
    ("Control+Quote", &["insertSingleQuoteIgnoringSubstitution:"]),
    ("Control+KeyA", &["moveToBeginningOfParagraph:"]),
    ("Control+KeyB", &["moveBackward:"]),
    ("Control+KeyD", &["deleteForward:"]),
    ("Control+KeyE", &["moveToEndOfParagraph:"]),
    ("Control+KeyF", &["moveForward:"]),
    ("Control+KeyH", &["deleteBackward:"]),
    ("Control+KeyK", &["deleteToEndOfParagraph:"]),
    ("Control+KeyL", &["centerSelectionInVisibleArea:"]),
    ("Control+KeyN", &["moveDown:"]),
    (
        "Control+KeyO",
        &["insertNewlineIgnoringFieldEditor:", "moveBackward:"],
    ),
    ("Control+KeyP", &["moveUp:"]),
    ("Control+KeyT", &["transpose:"]),
    ("Control+KeyV", &["pageDown:"]),
    ("Control+KeyY", &["yank:"]),
    (
        "Control+Backspace",
        &["deleteBackwardByDecomposingPreviousCharacter:"],
    ),
    ("Control+ArrowUp", &["scrollPageUp:"]),
    ("Control+ArrowDown", &["scrollPageDown:"]),
    ("Control+ArrowLeft", &["moveToLeftEndOfLine:"]),
    ("Control+ArrowRight", &["moveToRightEndOfLine:"]),
    ("Shift+Control+Enter", &["insertLineBreak:"]),
    ("Shift+Control+NumpadEnter", &["insertLineBreak:"]),
    ("Shift+Control+Tab", &["selectPreviousKeyView:"]),
    (
        "Shift+Control+Quote",
        &["insertDoubleQuoteIgnoringSubstitution:"],
    ),
    (
        "Shift+Control+KeyA",
        &["moveToBeginningOfParagraphAndModifySelection:"],
    ),
    ("Shift+Control+KeyB", &["moveBackwardAndModifySelection:"]),
    (
        "Shift+Control+KeyE",
        &["moveToEndOfParagraphAndModifySelection:"],
    ),
    ("Shift+Control+KeyF", &["moveForwardAndModifySelection:"]),
    ("Shift+Control+KeyN", &["moveDownAndModifySelection:"]),
    ("Shift+Control+KeyP", &["moveUpAndModifySelection:"]),
    ("Shift+Control+KeyV", &["pageDownAndModifySelection:"]),
    (
        "Shift+Control+Backspace",
        &["deleteBackwardByDecomposingPreviousCharacter:"],
    ),
    ("Shift+Control+ArrowUp", &["scrollPageUp:"]),
    ("Shift+Control+ArrowDown", &["scrollPageDown:"]),
    (
        "Shift+Control+ArrowLeft",
        &["moveToLeftEndOfLineAndModifySelection:"],
    ),
    (
        "Shift+Control+ArrowRight",
        &["moveToRightEndOfLineAndModifySelection:"],
    ),
    ("Alt+Backspace", &["deleteWordBackward:"]),
    ("Alt+Enter", &["insertNewlineIgnoringFieldEditor:"]),
    ("Alt+NumpadEnter", &["insertNewlineIgnoringFieldEditor:"]),
    ("Alt+Escape", &["complete:"]),
    (
        "Alt+ArrowUp",
        &["moveBackward:", "moveToBeginningOfParagraph:"],
    ),
    ("Alt+ArrowDown", &["moveForward:", "moveToEndOfParagraph:"]),
    ("Alt+ArrowLeft", &["moveWordLeft:"]),
    ("Alt+ArrowRight", &["moveWordRight:"]),
    ("Alt+Delete", &["deleteWordForward:"]),
    ("Alt+PageUp", &["pageUp:"]),
    ("Alt+PageDown", &["pageDown:"]),
    ("Shift+Alt+Backspace", &["deleteWordBackward:"]),
    ("Shift+Alt+Enter", &["insertNewlineIgnoringFieldEditor:"]),
    (
        "Shift+Alt+NumpadEnter",
        &["insertNewlineIgnoringFieldEditor:"],
    ),
    ("Shift+Alt+Escape", &["complete:"]),
    (
        "Shift+Alt+ArrowUp",
        &["moveParagraphBackwardAndModifySelection:"],
    ),
    (
        "Shift+Alt+ArrowDown",
        &["moveParagraphForwardAndModifySelection:"],
    ),
    ("Shift+Alt+ArrowLeft", &["moveWordLeftAndModifySelection:"]),
    (
        "Shift+Alt+ArrowRight",
        &["moveWordRightAndModifySelection:"],
    ),
    ("Shift+Alt+Delete", &["deleteWordForward:"]),
    ("Shift+Alt+PageUp", &["pageUp:"]),
    ("Shift+Alt+PageDown", &["pageDown:"]),
    ("Control+Alt+KeyB", &["moveWordBackward:"]),
    ("Control+Alt+KeyF", &["moveWordForward:"]),
    ("Control+Alt+Backspace", &["deleteWordBackward:"]),
    (
        "Shift+Control+Alt+KeyB",
        &["moveWordBackwardAndModifySelection:"],
    ),
    (
        "Shift+Control+Alt+KeyF",
        &["moveWordForwardAndModifySelection:"],
    ),
    ("Shift+Control+Alt+Backspace", &["deleteWordBackward:"]),
    ("Meta+NumpadSubtract", &["cancel:"]),
    ("Meta+Backspace", &["deleteToBeginningOfLine:"]),
    ("Meta+ArrowUp", &["moveToBeginningOfDocument:"]),
    ("Meta+ArrowDown", &["moveToEndOfDocument:"]),
    ("Meta+ArrowLeft", &["moveToLeftEndOfLine:"]),
    ("Meta+ArrowRight", &["moveToRightEndOfLine:"]),
    ("Shift+Meta+NumpadSubtract", &["cancel:"]),
    ("Shift+Meta+Backspace", &["deleteToBeginningOfLine:"]),
    (
        "Shift+Meta+ArrowUp",
        &["moveToBeginningOfDocumentAndModifySelection:"],
    ),
    (
        "Shift+Meta+ArrowDown",
        &["moveToEndOfDocumentAndModifySelection:"],
    ),
    (
        "Shift+Meta+ArrowLeft",
        &["moveToLeftEndOfLineAndModifySelection:"],
    ),
    (
        "Shift+Meta+ArrowRight",
        &["moveToRightEndOfLineAndModifySelection:"],
    ),
    ("Meta+KeyA", &["selectAll:"]),
    ("Meta+KeyC", &["copy:"]),
    ("Meta+KeyX", &["cut:"]),
    ("Meta+KeyV", &["paste:"]),
    ("Meta+KeyZ", &["undo:"]),
    ("Shift+Meta+KeyZ", &["redo:"]),
];

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use serde_json::Value;
    use tokio::time::Instant;

    use super::*;
    use crate::click::tests::{FakeInput, FakeTab, Step, Writes};
    use crate::progress::DeliveryCell;
    use crate::run::Delivery;

    const ROUND_TRIP: Duration = Duration::from_millis(1);
    const DELAY: Duration = Duration::from_millis(50);

    fn press(key: &str) -> Vec<KeyEvent> {
        plan_press(key, Platform::Mac).expect("a known key")
    }

    fn press_on(key: &str, platform: Platform) -> Vec<KeyEvent> {
        plan_press(key, platform).expect("a known key")
    }

    /// `(type, key, modifiers)` per event: what most planning tests pin.
    fn summary(events: &[KeyEvent]) -> Vec<(&'static str, &'static str, u8)> {
        events
            .iter()
            .map(|event| match event {
                KeyEvent::Down(down) => (down.kind, down.key, down.modifiers),
                KeyEvent::Up(up) => (up.kind, up.key, up.modifiers),
            })
            .collect()
    }

    fn down(events: &[KeyEvent], index: usize) -> &KeyDown {
        match &events[index] {
            KeyEvent::Down(down) => down,
            KeyEvent::Up(up) => panic!("event {index} is the keyUp of {}", up.key),
        }
    }

    fn entry(name: &str) -> LayoutEntry {
        *LAYOUT
            .get(name)
            .unwrap_or_else(|| panic!("{name:?} is in the layout"))
    }

    fn epoch() -> CdpError {
        CdpError::ConnectionEpochChanged {
            expected: 1,
            actual: 2,
        }
    }

    fn timeout() -> CdpError {
        CdpError::Timeout {
            method: "Input.dispatchKeyEvent".to_string(),
        }
    }

    /// Spot values and the size of Playwright's own `buildLayoutClosure` run over its own table
    /// (212 names): a code carries its shifted variant, while a key, a shifted key or an alias
    /// reaches a description without one, and the numpad is reached by its codes only.
    #[test]
    fn the_layout_closure_is_playwrights() {
        assert_eq!(LAYOUT.len(), 212);
        let digit1 = KeyDescription {
            key: "1",
            key_code_without_location: 49,
            code: "Digit1",
            text: "1",
            location: 0,
        };
        let bang = KeyDescription {
            key: "!",
            text: "!",
            ..digit1
        };
        let unshifted = |description| LayoutEntry {
            description,
            shifted: None,
        };
        assert_eq!(
            entry("Digit1"),
            LayoutEntry {
                description: digit1,
                shifted: Some(bang)
            }
        );
        assert_eq!(entry("1"), unshifted(digit1));
        assert_eq!(entry("!"), unshifted(bang));
        let shift = KeyDescription {
            key: "Shift",
            key_code_without_location: 16,
            code: "ShiftLeft",
            text: "",
            location: 1,
        };
        assert_eq!(entry("Shift"), unshifted(shift));
        let enter = KeyDescription {
            key: "Enter",
            key_code_without_location: 13,
            code: "Enter",
            text: "\r",
            location: 0,
        };
        assert_eq!(entry("\n"), unshifted(enter));
        assert_eq!(entry("\r"), unshifted(enter));
        assert_eq!(
            entry("ControlRight").description,
            KeyDescription {
                key: "Control",
                key_code_without_location: 17,
                code: "ControlRight",
                text: "",
                location: 2,
            }
        );
        assert_eq!(entry(" ").description.text, " ");
        for (name, code) in [
            ("/", "Slash"),
            ("+", "Equal"),
            ("*", "Digit8"),
            ("-", "Minus"),
            (".", "Period"),
            ("Home", "Home"),
            ("Enter", "Enter"),
        ] {
            assert_eq!(
                entry(name).description.code,
                code,
                "{name:?}, not the numpad"
            );
        }
        let end = KeyDescription {
            key: "End",
            key_code_without_location: 35,
            code: "Numpad1",
            text: "",
            location: 3,
        };
        let one = KeyDescription {
            key: "1",
            key_code_without_location: 97,
            text: "1",
            ..end
        };
        assert_eq!(
            entry("Numpad1"),
            LayoutEntry {
                description: end,
                shifted: Some(one)
            }
        );
        assert!(
            !LAYOUT.contains_key("Clear"),
            "a key longer than one character is a name only as a code"
        );
    }

    #[test]
    fn a_leading_or_lone_plus_is_a_key() {
        for (key, tokens) in [
            ("+", &["+"][..]),
            ("Shift++", &["Shift", "+"]),
            ("++a", &["+", "a"]),
            ("a+", &["a", ""]),
            ("", &[""]),
            ("Control+Shift+T", &["Control", "Shift", "T"]),
        ] {
            assert_eq!(split(key), tokens, "{key:?}");
        }
    }

    /// Shift selects a code's shifted variant; a key named directly types itself.
    #[test]
    fn shift_changes_a_code_never_a_named_key() {
        let held = |key| {
            [
                ("rawKeyDown", "Shift", 8),
                ("keyDown", key, 8),
                ("keyUp", key, 8),
                ("keyUp", "Shift", 0),
            ]
        };
        assert_eq!(summary(&press("Shift+KeyA")), held("A"));
        assert_eq!(summary(&press("Shift+a")), held("a"));
        assert_eq!(summary(&press("Shift+Digit1")), held("!"));
        assert_eq!(summary(&press("Shift+1")), held("1"));
        let shift = press("Shift+a");
        let shift = down(&shift, 0);
        assert_eq!(
            (
                shift.code,
                shift.windows_virtual_key_code,
                shift.location,
                shift.text,
                shift.commands.len()
            ),
            ("ShiftLeft", 16, 1, "", 0)
        );
        let upper = press("A");
        assert_eq!(summary(&upper), [("keyDown", "A", 0), ("keyUp", "A", 0)]);
        let upper = down(&upper, 0);
        assert_eq!(
            (upper.text, upper.code, upper.windows_virtual_key_code),
            ("A", "KeyA", 65)
        );
    }

    #[test]
    fn control_or_meta_follows_the_browser_platform() {
        let mac = press_on("ControlOrMeta+a", Platform::Mac);
        assert_eq!(
            summary(&mac),
            [
                ("rawKeyDown", "Meta", 4),
                ("rawKeyDown", "a", 4),
                ("keyUp", "a", 4),
                ("keyUp", "Meta", 0),
            ]
        );
        assert_eq!(
            (down(&mac, 0).code, down(&mac, 0).windows_virtual_key_code),
            ("MetaLeft", 91)
        );
        assert_eq!(down(&mac, 1).commands, ["selectAll"]);
        assert_eq!(down(&mac, 1).text, "", "no text while Meta is held");
        for platform in [Platform::Linux, Platform::Win] {
            let other = press_on("ControlOrMeta+a", platform);
            assert_eq!(
                summary(&other),
                [
                    ("rawKeyDown", "Control", 2),
                    ("rawKeyDown", "a", 2),
                    ("keyUp", "a", 2),
                    ("keyUp", "Control", 0),
                ],
                "{platform:?}"
            );
            assert_eq!(
                (
                    down(&other, 0).code,
                    down(&other, 0).windows_virtual_key_code
                ),
                ("ControlLeft", 17)
            );
            assert!(down(&other, 1).commands.is_empty(), "{platform:?}");
        }
    }

    #[test]
    fn mac_editing_commands_follow_the_held_modifiers() {
        let commands = |key: &str, platform| {
            press_on(key, platform)
                .iter()
                .rev()
                .find_map(|event| match event {
                    KeyEvent::Down(down) => Some(down.commands.clone()),
                    KeyEvent::Up(_) => None,
                })
                .expect("a keyDown")
        };
        for (key, expected) in [
            ("Backspace", &["deleteBackward"][..]),
            ("Delete", &["deleteForward"]),
            ("Enter", &[]),
            ("ArrowLeft", &["moveLeft"]),
            ("Meta+a", &["selectAll"]),
            ("Shift+ArrowLeft", &["moveLeftAndModifySelection"]),
            ("Control+KeyO", &["moveBackward"]),
            (
                "Alt+ArrowUp",
                &["moveBackward", "moveToBeginningOfParagraph"],
            ),
            (
                "Control+Alt+Shift+KeyB",
                &["moveWordBackwardAndModifySelection"],
            ),
            ("a", &[]),
        ] {
            assert_eq!(commands(key, Platform::Mac), expected, "{key}");
        }
        for key in ["Backspace", "Meta+a", "ArrowLeft"] {
            assert!(commands(key, Platform::Linux).is_empty(), "{key} on linux");
        }
        assert!(
            down(&press("Shift+ArrowLeft"), 0).commands.is_empty(),
            "a modifier's own keyDown names itself (`Shift+ShiftLeft`)"
        );
    }

    #[test]
    fn plus_is_the_shifted_equal_key() {
        let plus = press("+");
        assert_eq!(summary(&plus), [("keyDown", "+", 0), ("keyUp", "+", 0)]);
        let plus = down(&plus, 0);
        assert_eq!(
            (plus.code, plus.windows_virtual_key_code, plus.text),
            ("Equal", 187, "+")
        );
        assert_eq!(
            summary(&press("Shift++")),
            [
                ("rawKeyDown", "Shift", 8),
                ("keyDown", "+", 8),
                ("keyUp", "+", 8),
                ("keyUp", "Shift", 0),
            ]
        );
    }

    #[test]
    fn numpad_keys_are_keypad_keys_reached_by_code() {
        let end = press("Numpad1");
        let end = down(&end, 0);
        assert_eq!(
            (
                end.kind,
                end.key,
                end.code,
                end.windows_virtual_key_code,
                end.text,
                end.location,
                end.is_keypad
            ),
            ("rawKeyDown", "End", "Numpad1", 35, "", 3, true)
        );
        let one = press("Shift+Numpad1");
        let one = down(&one, 1);
        assert_eq!(
            (one.kind, one.key, one.windows_virtual_key_code, one.text),
            ("keyDown", "1", 97, "1")
        );
        let enter = press("NumpadEnter");
        let enter_down = down(&enter, 0);
        assert_eq!(
            (enter_down.kind, enter_down.text, enter_down.is_keypad),
            ("keyDown", "\r", true)
        );
        assert!(enter_down.commands.is_empty(), "insertNewline inserts text");
        assert!(matches!(&enter[1], KeyEvent::Up(up) if up.is_keypad && up.location == 3));
    }

    /// Wherever the unknown token sits, nothing is planned, and the error names that token.
    #[test]
    fn an_unknown_token_plans_nothing() {
        for (key, token) in [
            ("Foo", "Foo"),
            ("Shift+Foo", "Foo"),
            ("ControlOrMeta+Foo", "Foo"),
            ("Foo+Shift", "Foo"),
            ("", ""),
            ("a+", ""),
            ("Shift+", ""),
        ] {
            let error = plan_press(key, Platform::Mac).expect_err(key);
            assert_eq!(error.key, token, "{key:?}");
            assert_eq!(error.to_string(), format!("Unknown key: \"{token}\""));
        }
    }

    #[test]
    fn a_held_code_pressed_again_repeats() {
        let twice = press("a+a");
        assert_eq!(
            summary(&twice),
            [
                ("keyDown", "a", 0),
                ("keyDown", "a", 0),
                ("keyUp", "a", 0),
                ("keyUp", "a", 0),
            ]
        );
        assert_eq!(
            (down(&twice, 0).auto_repeat, down(&twice, 1).auto_repeat),
            (false, true)
        );
        assert!(down(&press("a+A"), 1).auto_repeat, "`A` is `a`'s code");
    }

    /// Field for field and in order, as `crInput.ts` builds them: every keyDown field present,
    /// an empty `commands` included.
    #[test]
    fn key_events_serialize_as_playwright_builds_them() {
        let json = |key| -> Vec<String> {
            press(key)
                .iter()
                .map(|event| serde_json::to_string(event).expect("serializes"))
                .collect()
        };
        assert_eq!(
            json("Enter"),
            [
                r#"{"type":"keyDown","modifiers":0,"windowsVirtualKeyCode":13,"code":"Enter","commands":[],"key":"Enter","text":"\r","unmodifiedText":"\r","autoRepeat":false,"location":0,"isKeypad":false}"#,
                r#"{"type":"keyUp","modifiers":0,"key":"Enter","windowsVirtualKeyCode":13,"code":"Enter","location":0,"isKeypad":false}"#,
            ]
        );
        assert_eq!(
            json("Meta+a")[1],
            r#"{"type":"rawKeyDown","modifiers":4,"windowsVirtualKeyCode":65,"code":"KeyA","commands":["selectAll"],"key":"a","text":"","unmodifiedText":"","autoRepeat":false,"location":0,"isKeypad":false}"#
        );
    }

    #[test]
    fn the_platform_is_read_from_the_user_agent_as_playwright_reads_it() {
        for (user_agent, platform) in [
            (
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like \
                 Gecko) HeadlessChrome/155.0.0.0 Safari/537.36",
                Platform::Mac,
            ),
            (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) \
                 Chrome/155.0.0.0 Safari/537.36",
                Platform::Win,
            ),
            (
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) \
                 Chrome/155.0.0.0 Safari/537.36",
                Platform::Linux,
            ),
            ("", Platform::Linux),
        ] {
            assert_eq!(
                Platform::from_user_agent(user_agent),
                platform,
                "{user_agent}"
            );
        }
    }

    // -----------------------------------------------------------------------------------------
    // The writer, over a recording sender under paused time.
    // -----------------------------------------------------------------------------------------

    /// One send the fake was asked for: when, in ms since the test began, and what.
    struct Sent {
        at: Duration,
        method: &'static str,
        params: String,
    }

    /// A sender that records every send it is asked for, refused ones included, and answers
    /// each after [`ROUND_TRIP`] with the next scripted answer, then `Ok`.
    #[derive(Clone)]
    struct FakeSender {
        start: Instant,
        sent: Arc<Mutex<Vec<Sent>>>,
        answers: Arc<Mutex<VecDeque<Result<(), CdpError>>>>,
    }

    impl FakeSender {
        fn answering(answers: impl IntoIterator<Item = Result<(), CdpError>>) -> Self {
            Self {
                start: Instant::now(),
                sent: Arc::default(),
                answers: Arc::new(Mutex::new(answers.into_iter().collect())),
            }
        }

        /// `<ms> <type> <key>` per key event, `<ms> insertText <text>` per text.
        fn sent(&self) -> Vec<String> {
            self.sent
                .lock()
                .expect("sent")
                .iter()
                .map(|sent| format!("{} {}", sent.at.as_millis(), describe(sent)))
                .collect()
        }

        fn params(&self) -> Vec<String> {
            let sent = self.sent.lock().expect("sent");
            sent.iter().map(|sent| sent.params.clone()).collect()
        }

        /// The test's input turn: records when the writer lets go of it.
        fn turn(&self) -> (Turn, Arc<Mutex<Option<Duration>>>) {
            let released = Arc::default();
            let turn = Turn {
                start: self.start,
                released: Arc::clone(&released),
            };
            (turn, released)
        }
    }

    fn describe(sent: &Sent) -> String {
        let params: Value = serde_json::from_str(&sent.params).expect("params are JSON");
        let field = |name: &str| params[name].as_str().unwrap_or_default().to_string();
        match sent.method {
            "Input.insertText" => format!("insertText {}", field("text")),
            _ => format!("{} {}", field("type"), field("key")),
        }
    }

    impl InputSender for FakeSender {
        async fn send_input(&self, method: &'static str, params: String) -> Result<(), CdpError> {
            let sent = Sent {
                at: self.start.elapsed(),
                method,
                params,
            };
            self.sent.lock().expect("sent").push(sent);
            let answer = self
                .answers
                .lock()
                .expect("answers")
                .pop_front()
                .unwrap_or(Ok(()));
            time::sleep(ROUND_TRIP).await;
            answer
        }
    }

    struct Turn {
        start: Instant,
        released: Arc<Mutex<Option<Duration>>>,
    }

    impl Drop for Turn {
        fn drop(&mut self) {
            *self.released.lock().expect("released") = Some(self.start.elapsed());
        }
    }

    fn released_at(released: &Mutex<Option<Duration>>) -> Option<u128> {
        released.lock().expect("released").map(|at| at.as_millis())
    }

    /// The real key writer over `sender`, started as `KeyboardTab::write_keys` starts it.
    fn start_keys(
        sender: &FakeSender,
        events: Vec<KeyEvent>,
        delay: Option<Duration>,
        ticket: InputTicket,
        turn: Turn,
    ) -> impl Future<Output = PressReport> + Send {
        let cut_short = CancellationToken::new();
        let writer = write_keys(sender.clone(), events, delay, ticket, cut_short.clone());
        detach(
            |task| {
                tokio::spawn(task);
            },
            writer,
            turn,
            cut_short,
        )
    }

    fn start_text(
        sender: &FakeSender,
        text: &str,
        ticket: InputTicket,
        turn: Turn,
    ) -> impl Future<Output = PressReport> + Send {
        let cut_short = CancellationToken::new();
        let writer = insert_text(sender.clone(), text.to_string(), ticket, cut_short.clone());
        detach(
            |task| {
                tokio::spawn(task);
            },
            writer,
            turn,
            cut_short,
        )
    }

    fn fresh() -> (Arc<DeliveryCell>, InputTicket) {
        let cell = Arc::new(DeliveryCell::default());
        let ticket = cell.begin().expect("a fresh cell");
        (cell, ticket)
    }

    #[tokio::test(start_paused = true)]
    async fn a_press_writes_each_event_once_the_last_is_answered() {
        let sender = FakeSender::answering([]);
        let (cell, ticket) = fresh();
        let (turn, released) = sender.turn();
        let report = start_keys(&sender, press("Shift+A"), None, ticket, turn).await;
        assert!(matches!(report, PressReport::Acked(_)), "{report:?}");
        assert_eq!(
            sender.sent(),
            [
                "0 rawKeyDown Shift",
                "1 keyDown A",
                "2 keyUp A",
                "3 keyUp Shift"
            ]
        );
        assert_eq!(cell.delivery(), Delivery::Done);
        assert_eq!(released_at(&released), Some(4), "held to the last answer");
    }

    #[tokio::test(start_paused = true)]
    async fn the_delay_sits_between_the_last_key_down_and_its_key_up() {
        let sender = FakeSender::answering([]);
        let (_cell, ticket) = fresh();
        let (turn, released) = sender.turn();
        let report = start_keys(&sender, press("Shift+a"), Some(DELAY), ticket, turn).await;
        assert!(matches!(report, PressReport::Acked(_)), "{report:?}");
        assert_eq!(
            sender.sent(),
            [
                "0 rawKeyDown Shift",
                "1 keyDown a",
                "52 keyUp a",
                "53 keyUp Shift"
            ]
        );
        assert_eq!(released_at(&released), Some(54));
    }

    #[tokio::test(start_paused = true)]
    async fn a_press_stopped_mid_delay_still_releases_every_key() {
        let sender = FakeSender::answering([]);
        let (cell, ticket) = fresh();
        let (turn, released) = sender.turn();
        let report = start_keys(&sender, press("Shift+a"), Some(DELAY), ticket, turn);
        let stopped = time::timeout(Duration::from_millis(10), report).await;
        assert!(stopped.is_err(), "the call stops at 10 ms");
        time::sleep(Duration::from_secs(1)).await;
        assert_eq!(
            sender.sent(),
            [
                "0 rawKeyDown Shift",
                "1 keyDown a",
                "10 keyUp a",
                "11 keyUp Shift"
            ],
            "the delay is cut short, never the release"
        );
        assert_eq!(cell.delivery(), Delivery::Done);
        assert_eq!(released_at(&released), Some(12));
    }

    #[tokio::test(start_paused = true)]
    async fn a_press_stopped_before_its_writer_runs_writes_nothing() {
        let sender = FakeSender::answering([]);
        let (cell, ticket) = fresh();
        let (turn, released) = sender.turn();
        drop(start_keys(&sender, press("Enter"), None, ticket, turn));
        time::sleep(Duration::from_secs(1)).await;
        assert!(sender.sent().is_empty());
        assert_eq!(cell.delivery(), Delivery::NotSent, "the writer still ran");
        assert_eq!(released_at(&released), Some(0));
    }

    #[tokio::test(start_paused = true)]
    async fn after_the_call_stops_no_further_key_goes_down() {
        let sender = FakeSender::answering([]);
        let (cell, ticket) = fresh();
        let (turn, _released) = sender.turn();
        let mut events = press("a");
        events.extend(press("b"));
        let report = start_keys(&sender, events, Some(DELAY), ticket, turn);
        assert!(
            time::timeout(Duration::from_millis(10), report)
                .await
                .is_err()
        );
        time::sleep(Duration::from_secs(1)).await;
        assert_eq!(sender.sent(), ["0 keyDown a", "10 keyUp a"]);
        assert_eq!(
            cell.delivery(),
            Delivery::Unknown,
            "the second press never went out: partial input (design section 4.7), not Done"
        );
    }

    /// A keyDown whose send failed may still have reached the browser, so it is released too;
    /// the key after it is never pressed.
    #[tokio::test(start_paused = true)]
    async fn a_failed_key_down_still_releases_every_key_it_may_have_pressed() {
        let sender = FakeSender::answering([Ok(()), Err(timeout())]);
        let (cell, ticket) = fresh();
        let (turn, _released) = sender.turn();
        let report = start_keys(&sender, press("Control+Shift+a"), None, ticket, turn).await;
        let PressReport::Failed { closed, message } = report else {
            panic!("{report:?}");
        };
        assert!(!closed);
        assert_eq!(message, "CDP request timed out: Input.dispatchKeyEvent");
        assert_eq!(
            sender.sent(),
            [
                "0 rawKeyDown Control",
                "1 rawKeyDown Shift",
                "2 keyUp Shift",
                "3 keyUp Control"
            ]
        );
        assert_eq!(cell.delivery(), Delivery::Unknown);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_key_up_still_releases_the_other_keys() {
        let protocol = CdpError::Protocol {
            code: -32000,
            message: "busy".to_string(),
        };
        let sender = FakeSender::answering([Ok(()), Ok(()), Err(protocol), Ok(())]);
        let (cell, ticket) = fresh();
        let (turn, _released) = sender.turn();
        let report = start_keys(&sender, press("Shift+A"), None, ticket, turn).await;
        assert!(
            matches!(report, PressReport::Failed { closed: false, .. }),
            "{report:?}"
        );
        assert_eq!(sender.sent().len(), 4);
        assert_eq!(cell.delivery(), Delivery::Unknown);
    }

    #[tokio::test(start_paused = true)]
    async fn a_refused_first_key_proves_nothing_was_written() {
        let sender = FakeSender::answering([Err(epoch())]);
        let (cell, ticket) = fresh();
        let (turn, _released) = sender.turn();
        let report = start_keys(&sender, press("Shift+A"), None, ticket, turn).await;
        assert!(matches!(report, PressReport::NothingWritten), "{report:?}");
        assert_eq!(sender.sent(), ["0 rawKeyDown Shift"], "nothing after it");
        assert_eq!(cell.delivery(), Delivery::NotSent);
    }

    #[tokio::test(start_paused = true)]
    async fn a_socket_replaced_after_a_key_down_leaves_delivery_unknown() {
        let sender = FakeSender::answering([Ok(()), Err(epoch())]);
        let (cell, ticket) = fresh();
        let (turn, _released) = sender.turn();
        let report = start_keys(&sender, press("Shift+A"), None, ticket, turn).await;
        assert!(
            matches!(report, PressReport::Failed { closed: true, .. }),
            "{report:?}"
        );
        assert_eq!(sender.sent(), ["0 rawKeyDown Shift", "1 keyDown A"]);
        assert_eq!(cell.delivery(), Delivery::Unknown);
    }

    #[tokio::test(start_paused = true)]
    async fn insert_text_reports_like_the_key_writer() {
        for (answers, delivery) in [
            (vec![], Delivery::Done),
            (vec![Err(epoch())], Delivery::NotSent),
            (vec![Err(timeout())], Delivery::Unknown),
        ] {
            let sender = FakeSender::answering(answers);
            let (cell, ticket) = fresh();
            let (turn, released) = sender.turn();
            let _report = start_text(&sender, "héllo\n", ticket, turn).await;
            assert_eq!(sender.params(), [r#"{"text":"héllo\n"}"#]);
            assert_eq!(cell.delivery(), delivery);
            assert_eq!(released_at(&released), Some(1));
        }
        let sender = FakeSender::answering([]);
        let (cell, ticket) = fresh();
        let (turn, _released) = sender.turn();
        drop(start_text(&sender, "late", ticket, turn));
        time::sleep(Duration::from_secs(1)).await;
        assert!(sender.sent().is_empty(), "a stopped call types nothing");
        assert_eq!(cell.delivery(), Delivery::NotSent);
    }

    // -----------------------------------------------------------------------------------------
    // Click's FakeTab as a keyboard, for the policies' tests.
    // -----------------------------------------------------------------------------------------

    /// The sender behind `FakeTab`'s keyboard: it records each event in the tab's `calls`
    /// (`keyDown A`, `insertText hi`) and answers it after the `press` step's duration with the
    /// writer's scripted [`Writes`]: `Ack` answers every event, `Nothing` refuses each
    /// (`ConnectionEpochChanged`), `Fail` times each out.
    struct TabSender {
        calls: Arc<Mutex<Vec<String>>>,
        writes: Writes,
        duration: Duration,
    }

    impl TabSender {
        fn for_tab(tab: &FakeTab) -> Self {
            Self {
                calls: Arc::clone(&tab.calls),
                writes: tab.press.next(),
                duration: tab.press.duration,
            }
        }
    }

    impl InputSender for TabSender {
        async fn send_input(&self, method: &'static str, params: String) -> Result<(), CdpError> {
            let sent = Sent {
                at: Duration::ZERO,
                method,
                params,
            };
            self.calls.lock().expect("calls").push(describe(&sent));
            time::sleep(self.duration).await;
            match self.writes {
                Writes::Ack => Ok(()),
                Writes::Nothing => Err(epoch()),
                Writes::Fail => Err(timeout()),
            }
        }
    }

    /// The real writer over [`TabSender`], so a policy test sees Playwright's events in `calls`
    /// and the writer's own release and report rules. The platform is mac, as in the goldens.
    impl KeyboardTab for FakeTab {
        async fn platform(&self) -> Result<Platform, PageError> {
            self.record("platform");
            self.platform.answer().await
        }

        fn write_keys(
            &self,
            events: Vec<KeyEvent>,
            delay: Option<Duration>,
            ticket: InputTicket,
            input: FakeInput,
        ) -> impl Future<Output = PressReport> + Send {
            let cut_short = CancellationToken::new();
            let sender = TabSender::for_tab(self);
            let writer = write_keys(sender, events, delay, ticket, cut_short.clone());
            detach(
                |task| {
                    tokio::spawn(task);
                },
                writer,
                input,
                cut_short,
            )
        }

        fn insert_text(
            &self,
            text: &str,
            ticket: InputTicket,
            input: FakeInput,
        ) -> impl Future<Output = PressReport> + Send {
            let cut_short = CancellationToken::new();
            let sender = TabSender::for_tab(self);
            let writer = insert_text(sender, text.to_string(), ticket, cut_short.clone());
            detach(
                |task| {
                    tokio::spawn(task);
                },
                writer,
                input,
                cut_short,
            )
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_fake_tab_types_through_the_real_writer() {
        let tab = FakeTab::ready();
        assert_eq!(tab.platform().await, Ok(Platform::Mac));
        let (cell, ticket) = fresh();
        let input = tab.input().await;
        let report = tab.write_keys(press("Shift+A"), None, ticket, input).await;
        assert!(matches!(report, PressReport::Acked(_)), "{report:?}");
        assert_eq!(cell.delivery(), Delivery::Done);
        let (_cell, ticket) = fresh();
        let input = tab.input().await;
        let _report = tab.insert_text("hi", ticket, input).await;
        assert_eq!(
            tab.calls(),
            [
                "platform",
                "input",
                "rawKeyDown Shift",
                "keyDown A",
                "keyUp A",
                "keyUp Shift",
                "input released",
                "input",
                "insertText hi",
                "input released",
            ]
        );

        let tab = FakeTab {
            press: Step::new(ROUND_TRIP, [Writes::Nothing, Writes::Fail]),
            ..FakeTab::ready()
        };
        let (cell, ticket) = fresh();
        let input = tab.input().await;
        let report = tab.write_keys(press("Shift+A"), None, ticket, input).await;
        assert!(matches!(report, PressReport::NothingWritten), "{report:?}");
        assert_eq!(cell.delivery(), Delivery::NotSent);
        let (cell, ticket) = fresh();
        let input = tab.input().await;
        let report = tab.write_keys(press("Shift+A"), None, ticket, input).await;
        assert!(matches!(report, PressReport::Failed { .. }), "{report:?}");
        assert_eq!(cell.delivery(), Delivery::Unknown);
        assert_eq!(
            tab.calls(),
            [
                "input",
                "rawKeyDown Shift",
                "input released",
                "input",
                "rawKeyDown Shift",
                "keyUp Shift",
                "input released",
            ],
            "a refused key ends the writer; a failed one is still released"
        );
    }
}
