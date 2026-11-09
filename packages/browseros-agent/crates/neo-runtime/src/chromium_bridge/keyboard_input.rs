//! Playwright's keyboard for Chromium: `server/input.ts` (`Keyboard`, `buildLayoutClosure`,
//! `resolveSmartModifierString`), `usKeyboardLayout.ts`, `macEditingCommands.ts` and
//! `chromium/crInput.ts` (`RawKeyboardImpl`), as of Playwright 26137face; none of the four changed
//! since v1.64.1. It plans key input into a [`Plan`] from the tab's keyboard state
//! (`mouse_and_touch_input.rs`), which the one input writer sends.
//!
//! ```text
//!  plan_press / plan_key_down / plan_key_up / plan_type / plan_insert_text
//!    pure: Playwright's `+` split, the US layout, the held modifiers and keys of the tab's
//!    keyboard, crInput's `Input.dispatchKeyEvent` params, each keyUp naming the keyDown it
//!    releases, `delay` before a keyUp right after its keyDown
//!  plan_ensure_modifiers / plan_restore_modifiers
//!    Playwright's `ensureModifiers` around an action's input, and its restore
//!  KeyboardTab::write(plan)        the detached input writer (`mouse_and_touch_input::start_writer`)
//!  KeyboardTab::platform()         `Browser.getVersion`'s userAgent → mac, win or linux
//! ```
//!
//! Policy (press, fill, the page's keyboard) plans the keys and decides retries; nothing here
//! writes a call-log line or names a locator. Neo-only behaviour, each deliberate:
//! - A press is planned whole before anything is written, so an unknown token writes nothing.
//!   Playwright writes the keyDowns before the bad token, then throws and leaves them held:
//!   `Shift+Foo` leaves Shift down in the tab, corrupting every later keystroke there.
//! - After cancellation or a CDP error the writer still sends the keyUp of every keyDown that
//!   may have been written, newest first. Playwright's press stops at the error, keys held.
//! - `ControlOrMeta` follows the browser's platform, as the mac `commands` do; Playwright reads
//!   its own process's platform. Neo's server runs on the browser's machine, so they agree.
//! - `Escape` never tries to cancel a drag first: Neo has no drag manager yet.

use std::collections::HashMap;
use std::future::Future;
use std::mem;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use crate::browser_representations::page::{FrameTab, InputPermit};
use crate::chromium_bridge::cdp::input::{self, KeyDown, KeyEvent, KeyUp};
use crate::chromium_bridge::mouse_and_touch_input::{
    self, DragStep, Effect, InputDevices, KeyboardState, Owner, Plan, Planned,
};
use crate::chromium_bridge::playwright_injected_methods;
use crate::playwright_methods::click::{PageError, PressReport, Tab};
use crate::progress::InputTicket;

// ---------------------------------------------------------------------------------------------
// The seam: what key input needs from a tab, beyond click's.
// ---------------------------------------------------------------------------------------------

/// The keyboard of the run's tab. Each method is one Playwright step; every returned future is
/// cancel-safe, as [`Tab`]'s are. Implemented by `FrameTab` here and by click's `FakeTab` in the
/// tests, whose writes go through the real writer.
pub(crate) trait KeyboardTab: Tab {
    /// The browser's platform: `Browser.getVersion`'s userAgent, read as Playwright's
    /// `crBrowser.ts` `_platform` reads it. It decides `ControlOrMeta` and the mac `commands`.
    fn platform(&self) -> impl Future<Output = Result<Platform, PageError>> + Send;

    /// Starts writing `plan` on a detached writer task *before it returns*, and gives back a
    /// future of the writer's report (`mouse_and_touch_input::start_writer`): the ticket and the turn
    /// move into the writer, which marks the ticket and records each event in the tab's devices
    /// however the caller ends. Dropping the returned future (the call stopped) cuts a `delay`
    /// short and starts no further keyDown, while every key the writer may have pressed is still
    /// released. The writer holds `input` until its last event is answered, so the tab's next
    /// input waits for the release.
    fn write(
        &self,
        plan: Plan,
        ticket: InputTicket,
        input: Self::Input,
    ) -> impl Future<Output = PressReport> + Send;
}

impl KeyboardTab for FrameTab {
    async fn platform(&self) -> Result<Platform, PageError> {
        let user_agent = self
            .user_agent()
            .await
            .map_err(|error| playwright_injected_methods::classify(&error))?;
        Ok(Platform::from_user_agent(&user_agent))
    }

    fn write(
        &self,
        plan: Plan,
        ticket: InputTicket,
        input: InputPermit,
    ) -> impl Future<Output = PressReport> + Send {
        let written = mouse_and_touch_input::start_writer(
            |task| self.spawn(task),
            Arc::clone(&self.page),
            plan,
            ticket,
            input,
            (),
        );
        async move { written.await.0 }
    }
}

// ---------------------------------------------------------------------------------------------
// Planning: Playwright's keyboard, from the tab's keyboard state.
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

/// Playwright's `KeyboardModifier`, with the smart `ControlOrMeta`: what an action's
/// `modifiers` option names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyboardModifier {
    Alt,
    Control,
    ControlOrMeta,
    Meta,
    Shift,
}

/// Playwright: `server/input.ts` `Keyboard.press`
///
/// The key split on `+`, a keyDown per token in order, then a keyUp per token in reverse,
/// starting from the held modifiers and keys of `state` (a token already held repeats).
/// Every token is looked up before any event exists, so an unknown one fails the whole press
/// and nothing is planned; the error names the first unknown token, the one at whose keyDown
/// Playwright throws.
pub(crate) fn press_events(
    key: &str,
    platform: Platform,
    state: &KeyboardState,
) -> Result<Vec<KeyEvent>, UnknownKey> {
    let tokens = split(key);
    let mut keyboard = Keyboard::from_state(state);
    let mut events = Vec::with_capacity(tokens.len() * 2);
    for token in &tokens {
        events.push(KeyEvent::Down(keyboard.down(token, platform)?));
    }
    for token in tokens.iter().rev() {
        events.push(KeyEvent::Up(keyboard.up(token, platform)?));
    }
    Ok(events)
}

/// Playwright: `server/input.ts` `Keyboard.press`: `if (options.delay)`
///
/// One press as plan commands, from the tab's keyboard: each keyUp undoes the latest keyDown of
/// its code in this press, so it is owed once that keyDown may have gone out, and `delay` is
/// slept before the keyUp right after the last keyDown.
pub(crate) fn plan_press(
    plan: &mut Plan,
    simulated: &mut InputDevices,
    key: &str,
    platform: Platform,
    owner: &Owner,
    delay: Option<Duration>,
) -> Result<(), UnknownKey> {
    let events = press_events(key, platform, &simulated.keyboard)?;
    let mut downs: Vec<(&'static str, usize)> = Vec::with_capacity(events.len() / 2);
    let mut after_down = false;
    for event in events {
        match event {
            KeyEvent::Down(down) => {
                let code = down.code;
                let at = plan.push(simulated, key_command(&KeyEvent::Down(down), owner));
                downs.push((code, at));
                after_down = true;
            }
            KeyEvent::Up(up) => {
                let undoes = downs
                    .iter()
                    .rposition(|(code, _)| *code == up.code)
                    .map(|found| downs.remove(found).1);
                let pause = if after_down { delay } else { None };
                after_down = false;
                plan.push(
                    simulated,
                    Planned {
                        undoes,
                        pause,
                        ..key_command(&KeyEvent::Up(up), owner)
                    },
                );
            }
        }
    }
    Ok(())
}

/// Playwright: `server/input.ts` `Keyboard.down`: `this._pressedKeys.add(description.code);`
///
/// One keyDown from the tab's keyboard, held until a later keyUp or the run's end. Returns its
/// index.
pub(crate) fn plan_key_down(
    plan: &mut Plan,
    simulated: &mut InputDevices,
    key: &str,
    platform: Platform,
    owner: &Owner,
) -> Result<usize, UnknownKey> {
    let down = Keyboard::from_state(&simulated.keyboard).down(key, platform)?;
    Ok(plan.push(simulated, key_command(&KeyEvent::Down(down), owner)))
}

/// Playwright: `server/input.ts` `Keyboard.up`: `this._pressedKeys.delete(description.code);`
///
/// One keyUp from the tab's keyboard, whether or not the key is held (Playwright sends it
/// either way). `undoes` is the keyDown of this plan it releases, if any.
pub(crate) fn plan_key_up(
    plan: &mut Plan,
    simulated: &mut InputDevices,
    key: &str,
    platform: Platform,
    undoes: Option<usize>,
) -> Result<usize, UnknownKey> {
    let up = Keyboard::from_state(&simulated.keyboard).up(key, platform)?;
    // Owners matter only on keyDowns; a keyUp names none.
    let owner = Owner::default();
    let command = Planned {
        undoes,
        ..key_command(&KeyEvent::Up(up), &owner)
    };
    Ok(plan.push(simulated, command))
}

/// Neo: how many characters of a `keyboard.type` are planned at once. The text arrives whole
/// (bounded by the call's request size), but its commands, a few per character, are planned a
/// chunk at a time as the writer reaches them, so typing a long text holds a bounded number of
/// them.
const TYPE_CHUNK: usize = 64;

/// Neo: the text a `keyboard.type` has still to type, with the devices its commands so far
/// leave, planned [`TYPE_CHUNK`] characters at a time by the writer (`Plan::tail`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TextTail {
    text: String,
    platform: Platform,
    owner: Owner,
    delay: Option<Duration>,
    simulated: InputDevices,
}

/// Neo: planning a long text a chunk at a time.
impl TextTail {
    pub(crate) fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The next chunk's commands, or `None` once the whole text is planned.
    pub(crate) fn next_chunk(&mut self) -> Option<Plan> {
        if self.text.is_empty() {
            return None;
        }
        let rest = self.text.split_off(chunk_end(&self.text));
        let chunk = mem::replace(&mut self.text, rest);
        let mut plan = Plan::default();
        plan_characters(
            &mut plan,
            &mut self.simulated,
            &chunk,
            self.platform,
            &self.owner,
            self.delay,
        );
        Some(plan)
    }
}

/// Neo: the byte offset that ends the text's first [`TYPE_CHUNK`] characters.
fn chunk_end(text: &str) -> usize {
    text.char_indices()
        .nth(TYPE_CHUNK)
        .map_or(text.len(), |(at, _)| at)
}

/// Playwright: `server/input.ts` `Keyboard.type`: `for (const char of text) {`
///
/// The first [`TYPE_CHUNK`] characters now, the rest as the plan's tail.
pub(crate) fn plan_type(
    plan: &mut Plan,
    simulated: &mut InputDevices,
    text: &str,
    platform: Platform,
    owner: &Owner,
    delay: Option<Duration>,
) {
    let end = chunk_end(text);
    plan_characters(plan, simulated, &text[..end], platform, owner, delay);
    if end < text.len() {
        plan.tail = Some(TextTail {
            text: text[end..].to_string(),
            platform,
            owner: owner.clone(),
            delay,
            simulated: simulated.clone(),
        });
    }
}

/// Playwright: `server/input.ts` `Keyboard.type`: `if (usKeyboardLayout.has(char)) {`
///
/// Each character a press when the US layout knows it, else an `Input.insertText` of it after
/// `delay`.
fn plan_characters(
    plan: &mut Plan,
    simulated: &mut InputDevices,
    text: &str,
    platform: Platform,
    owner: &Owner,
    delay: Option<Duration>,
) {
    for character in text.chars() {
        let mut buffer = [0; 4];
        let character: &str = character.encode_utf8(&mut buffer);
        let pressed = LAYOUT.contains_key(character)
            && plan_press(plan, simulated, character, platform, owner, delay).is_ok();
        if !pressed {
            let at = plan_insert_text(plan, simulated, character);
            plan.commands[at].pause = delay;
        }
    }
}

/// Playwright: `server/chromium/crInput.ts` `RawKeyboardImpl.sendText`: `await progress.race(this._client.send('Input.insertText', { text }));`
pub(crate) fn plan_insert_text(plan: &mut Plan, simulated: &mut InputDevices, text: &str) -> usize {
    plan.push(
        simulated,
        Planned::write(&input::InsertText { text }, Effect::None),
    )
}

/// One modifier change `plan_ensure_modifiers` made, for its restore.
#[derive(Debug, Clone)]
pub(crate) struct ModifierChange {
    key: &'static str,
    /// The change pressed the key (and its restore releases it), or released it.
    pressed: bool,
    /// The change's index in the plan.
    at: usize,
    /// For a released key, every run that held it, which gets it back on the restore.
    held_by: Vec<Owner>,
}

/// Playwright: `server/input.ts` `Keyboard.ensureModifiers`: `const restore: types.KeyboardModifier[] = Array.from(this._pressedModifiers);`
///
/// Presses each modifier of `target` not held and releases each held one it lacks, in
/// Playwright's `kModifiers` order, and returns the changes for [`plan_restore_modifiers`].
pub(crate) fn plan_ensure_modifiers(
    plan: &mut Plan,
    simulated: &mut InputDevices,
    target: u8,
    platform: Platform,
    owner: &Owner,
) -> Vec<ModifierChange> {
    let mut changes = Vec::new();
    for (bit, key) in K_MODIFIERS {
        let need_down = target & bit != 0;
        let is_down = simulated.keyboard.modifiers & bit != 0;
        if need_down == is_down {
            continue;
        }
        let held_by = holders_of(&simulated.keyboard, key);
        let planned = if need_down {
            plan_key_down(plan, simulated, key, platform, owner)
        } else {
            plan_key_up(plan, simulated, key, platform, None)
        };
        // Every name in `K_MODIFIERS` is in the layout.
        if let Ok(at) = planned {
            changes.push(ModifierChange {
                key,
                pressed: need_down,
                at,
                held_by,
            });
        }
    }
    changes
}

/// Playwright: `server/dom.ts` `ElementHandle._performPointerAction`: `await this._page.keyboard.ensureModifiers(progress, restoreModifiers);`
///
/// Undoes every change `plan_ensure_modifiers` made, in its order: a key it pressed is released
/// and a key it released is pressed again for the run that held it. Each restore undoes its
/// change, so it is written whenever the change may have been, a stopped call included.
pub(crate) fn plan_restore_modifiers(
    plan: &mut Plan,
    simulated: &mut InputDevices,
    changes: &[ModifierChange],
    platform: Platform,
    owner: &Owner,
) {
    for change in changes {
        let planned = if change.pressed {
            plan_key_up(plan, simulated, change.key, platform, Some(change.at))
        } else {
            let (first, others) = change
                .held_by
                .split_first()
                .map_or((owner, &[][..]), |(first, others)| (first, others));
            plan_key_down(plan, simulated, change.key, platform, first).inspect(|&at| {
                plan.commands[at].undoes = Some(change.at);
                // Every other run that held the key holds it again, as written as the keyDown.
                if let Effect::KeyDown { code, .. } = plan.commands[at].effect {
                    for holder in others {
                        let held = plan.push(
                            simulated,
                            Planned::state_only(Effect::KeyHolder {
                                code,
                                owner: holder.clone(),
                            }),
                        );
                        plan.commands[held].undoes = Some(change.at);
                    }
                }
            })
        };
        debug_assert!(
            planned.is_ok(),
            "every name in `K_MODIFIERS` is in the layout"
        );
    }
}

/// Deviation: `server/input.ts` `Keyboard.up`: `this._pressedKeys.delete(description.code);` Playwright keeps a page's keys held until its script releases them; Neo releases at a run's end every key the run left held, newest first, because the tab is the user's and outlives the run. A held code is a layout name, so the release needs no platform.
pub(crate) fn plan_key_releases(plan: &mut Plan, simulated: &mut InputDevices, owner: &Owner) {
    // First, before any release: whatever this run still holds once its end is over (a release
    // that is refused, fails or never goes out) is held by no live run.
    mouse_and_touch_input::plan_retire(plan, simulated, owner);
    // What this run holds, newest first, and what only ended runs hold (a release that failed
    // at an earlier run's end).
    let mut held: Vec<&'static str> = Vec::new();
    for (code, holder) in simulated.keyboard.pressed.iter().rev() {
        if simulated.releases(holder, owner) && !held.contains(code) {
            held.push(code);
        }
    }
    for code in held {
        // A key another live run still holds stays down; this run only lets go of it.
        let shared = simulated
            .keyboard
            .pressed
            .iter()
            .any(|(held, holder)| *held == code && simulated.live_other(holder, owner));
        let at = if shared {
            if !simulated
                .keyboard
                .pressed
                .iter()
                .any(|(held, holder)| *held == code && holder == owner)
            {
                continue;
            }
            Some(plan.push(
                simulated,
                Planned::state_only(Effect::KeyLetGo {
                    code,
                    owner: owner.clone(),
                }),
            ))
        } else {
            // A code is always in the layout; the platform only decides `ControlOrMeta`.
            plan_key_up(plan, simulated, code, Platform::Linux, None).ok()
        };
        // Each release is independent: one that fails does not keep the others held.
        if let Some(at) = at {
            plan.commands[at].always = true;
        }
    }
}

/// Playwright: `server/input.ts` `kModifiers`: `['Alt', 'Control', 'Meta', 'Shift']`
const K_MODIFIERS: [(u8, &str); 4] = [
    (ALT, "Alt"),
    (CONTROL, "Control"),
    (META, "Meta"),
    (SHIFT, "Shift"),
];

/// Playwright: `server/input.ts` `resolveSmartModifierString`: `return process.platform === 'darwin' ? 'Meta' : 'Control';`
///
/// An action's `modifiers` option as the mask `ensureModifiers` aims at.
pub(crate) fn modifiers_mask(modifiers: &[KeyboardModifier], platform: Platform) -> u8 {
    modifiers
        .iter()
        .map(|modifier| match (modifier, platform) {
            (KeyboardModifier::Alt, _) => ALT,
            (KeyboardModifier::Control, _)
            | (KeyboardModifier::ControlOrMeta, Platform::Linux | Platform::Win) => CONTROL,
            (KeyboardModifier::Meta, _) | (KeyboardModifier::ControlOrMeta, Platform::Mac) => META,
            (KeyboardModifier::Shift, _) => SHIFT,
        })
        .fold(0, |mask, bit| mask | bit)
}

/// The runs that hold the modifier `key` (`Shift`, …), oldest first: every holder of a held
/// code the layout names that key, as Playwright's modifier set holds a key by name, whichever
/// side pressed it.
fn holders_of(state: &KeyboardState, key: &str) -> Vec<Owner> {
    let mut holders: Vec<Owner> = Vec::new();
    for (code, owner) in &state.pressed {
        if LAYOUT
            .get(code)
            .is_some_and(|entry| entry.description.key == key)
            && !holders.contains(owner)
        {
            holders.push(owner.clone());
        }
    }
    holders
}

/// Neo: one key event as a plan command, with what it does to the tab's keyboard.
fn key_command(event: &KeyEvent, owner: &Owner) -> Planned {
    let effect = match event {
        KeyEvent::Down(down) => Effect::KeyDown {
            code: down.code,
            modifier: modifier_bit(down.key).unwrap_or(0),
            owner: owner.clone(),
        },
        KeyEvent::Up(up) => Effect::KeyUp {
            code: up.code,
            modifier: modifier_bit(up.key).unwrap_or(0),
        },
    };
    let drag = match event {
        // Playwright: `server/chromium/crInput.ts` `RawKeyboardImpl.keydown`: `if (code === 'Escape' && await progress.race(this._dragManger.cancelDrag()))`
        KeyEvent::Down(down) if down.code == "Escape" => DragStep::Escape,
        _ => DragStep::None,
    };
    Planned {
        drag,
        ..Planned::write(event, effect)
    }
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
pub(crate) const ALT: u8 = 1;
pub(crate) const CONTROL: u8 = 2;
pub(crate) const META: u8 = 4;
pub(crate) const SHIFT: u8 = 8;

/// `kModifiers`: the keys that are modifiers, by their `key`.
pub(crate) fn modifier_bit(key: &str) -> Option<u8> {
    match key {
        "Alt" => Some(ALT),
        "Control" => Some(CONTROL),
        "Meta" => Some(META),
        "Shift" => Some(SHIFT),
        _ => None,
    }
}

/// Playwright's `Keyboard` state while a press is planned: the modifiers held (as the mask)
/// and the codes of the keys held, which make a second keyDown of the same code an
/// `autoRepeat`.
#[derive(Debug, Default)]
struct Keyboard {
    modifiers: u8,
    pressed: Vec<&'static str>,
}

impl Keyboard {
    /// The tab's keyboard, as the planning copy starts from it: each held code once, whatever
    /// runs hold it.
    fn from_state(state: &KeyboardState) -> Self {
        let mut pressed: Vec<&'static str> = Vec::new();
        for (code, _) in &state.pressed {
            if !pressed.contains(code) {
                pressed.push(code);
            }
        }
        Self {
            modifiers: state.modifiers,
            pressed,
        }
    }

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
    use std::iter;
    use std::sync::Mutex;

    use browseros_cdp::CdpError;
    use serde_json::Value;
    use tokio::time;
    use tokio::time::Instant;

    use super::*;
    use crate::chromium_bridge::cdp;
    use crate::chromium_bridge::mouse_and_touch_input::{Devices, EffectSink, InputSender};
    use crate::playwright_methods::click::tests::{FakeInput, FakeTab, Step, Writes, fake_owner};
    use crate::progress::DeliveryCell;
    use crate::run::Delivery;

    const ROUND_TRIP: Duration = Duration::from_millis(1);
    const DELAY: Duration = Duration::from_millis(50);

    fn press(key: &str) -> Vec<KeyEvent> {
        press_on(key, Platform::Mac)
    }

    fn press_on(key: &str, platform: Platform) -> Vec<KeyEvent> {
        press_events(key, platform, &KeyboardState::default()).expect("a known key")
    }

    /// The presses of `keys`, one after another, planned on mac from a keyboard with nothing held.
    fn keys_plan(keys: &[&str], delay: Option<Duration>) -> Plan {
        let mut simulated = InputDevices::default();
        let mut plan = Plan::default();
        for key in keys {
            plan_press(
                &mut plan,
                &mut simulated,
                key,
                Platform::Mac,
                &fake_owner(),
                delay,
            )
            .expect("a known key");
        }
        plan
    }

    fn text_plan(text: &str) -> Plan {
        let mut plan = Plan::default();
        plan_insert_text(&mut plan, &mut InputDevices::default(), text);
        plan
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
            let error = press_events(key, Platform::Mac, &KeyboardState::default()).expect_err(key);
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
                _release: Arc::new(TurnRelease {
                    start: self.start,
                    released: Arc::clone(&released),
                }),
                devices: Arc::default(),
            };
            (turn, released)
        }
    }

    /// One sent event as the tests compare it: `keyDown a`, `insertText hi`, and for the device
    /// tests a mouse or touch event's type with the button state and modifiers it carries
    /// (`mousePressed button=left buttons=1 clickCount=1 modifiers=8`), never coordinates.
    fn describe(sent: &Sent) -> String {
        let params: Value = serde_json::from_str(&sent.params).expect("params are JSON");
        let field = |name: &str| params[name].as_str().unwrap_or_default().to_string();
        match sent.method {
            "Input.insertText" => format!("insertText {}", field("text")),
            "Input.dispatchMouseEvent" | "Input.dispatchTouchEvent" => {
                let state = ["button", "buttons", "clickCount", "modifiers"]
                    .into_iter()
                    .filter_map(|name| {
                        let value = params.get(name)?;
                        let value = value
                            .as_str()
                            .map_or_else(|| value.to_string(), str::to_string);
                        Some(format!("{name}={value}"))
                    });
                iter::once(field("type"))
                    .chain(state)
                    .collect::<Vec<_>>()
                    .join(" ")
            }
            _ => format!("{} {}", field("type"), field("key")),
        }
    }

    impl InputSender for FakeSender {
        async fn send_input(
            &self,
            input: &cdp::Input,
            _delivery: &InputTicket,
        ) -> Result<(), CdpError> {
            let sent = Sent {
                at: self.start.elapsed(),
                method: input.method(),
                params: input.params().to_string(),
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

    /// The test's input turn: records when its last copy goes, and carries devices of its own.
    #[derive(Clone)]
    struct Turn {
        /// Held for its `Drop`, which records when the last copy of the turn went.
        _release: Arc<TurnRelease>,
        devices: Arc<Mutex<InputDevices>>,
    }

    struct TurnRelease {
        start: Instant,
        released: Arc<Mutex<Option<Duration>>>,
    }

    impl Drop for TurnRelease {
        fn drop(&mut self) {
            *self.released.lock().expect("released") = Some(self.start.elapsed());
        }
    }

    impl EffectSink for Turn {
        fn apply(&self, effect: &Effect, answered: bool) {
            // Each write is its own turn here.
            let mut devices = self.devices.lock().expect("devices");
            devices.apply(effect, answered);
            devices.forget_ended();
        }

        fn devices(&self) -> InputDevices {
            self.devices.lock().expect("devices").clone()
        }
    }

    impl Devices for Turn {
        fn snapshot(&self) -> Result<InputDevices, PageError> {
            Ok(self.devices.lock().expect("devices").clone())
        }

        fn owner(&self) -> Owner {
            fake_owner()
        }
    }

    fn released_at(released: &Mutex<Option<Duration>>) -> Option<u128> {
        released.lock().expect("released").map(|at| at.as_millis())
    }

    /// The real input writer over `sender` with the presses of `keys`, started as
    /// `KeyboardTab::write` starts it.
    fn start_keys(
        sender: &FakeSender,
        keys: &[&str],
        delay: Option<Duration>,
        ticket: InputTicket,
        turn: Turn,
    ) -> impl Future<Output = PressReport> + Send {
        start_plan(sender, keys_plan(keys, delay), ticket, turn)
    }

    fn start_text(
        sender: &FakeSender,
        text: &str,
        ticket: InputTicket,
        turn: Turn,
    ) -> impl Future<Output = PressReport> + Send {
        start_plan(sender, text_plan(text), ticket, turn)
    }

    fn start_plan(
        sender: &FakeSender,
        plan: Plan,
        ticket: InputTicket,
        turn: Turn,
    ) -> impl Future<Output = PressReport> + Send {
        let written = mouse_and_touch_input::start_writer(
            |task| {
                tokio::spawn(task);
            },
            sender.clone(),
            plan,
            ticket,
            turn,
            (),
        );
        async move { written.await.0 }
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
        let report = start_keys(&sender, &["Shift+A"], None, ticket, turn).await;
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
        let report = start_keys(&sender, &["Shift+a"], Some(DELAY), ticket, turn).await;
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
        let report = start_keys(&sender, &["Shift+a"], Some(DELAY), ticket, turn);
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
        drop(start_keys(&sender, &["Enter"], None, ticket, turn));
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
        let report = start_keys(&sender, &["a", "b"], Some(DELAY), ticket, turn);
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
        let report = start_keys(&sender, &["Control+Shift+a"], None, ticket, turn).await;
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
        let report = start_keys(&sender, &["Shift+A"], None, ticket, turn).await;
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
        let report = start_keys(&sender, &["Shift+A"], None, ticket, turn).await;
        assert!(matches!(report, PressReport::NothingWritten), "{report:?}");
        assert_eq!(sender.sent(), ["0 rawKeyDown Shift"], "nothing after it");
        assert_eq!(cell.delivery(), Delivery::NotSent);
    }

    #[tokio::test(start_paused = true)]
    async fn a_socket_replaced_after_a_key_down_leaves_delivery_unknown() {
        let sender = FakeSender::answering([Ok(()), Err(epoch())]);
        let (cell, ticket) = fresh();
        let (turn, _released) = sender.turn();
        let report = start_keys(&sender, &["Shift+A"], None, ticket, turn).await;
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
        async fn send_input(
            &self,
            input: &cdp::Input,
            _delivery: &InputTicket,
        ) -> Result<(), CdpError> {
            let sent = Sent {
                at: Duration::ZERO,
                method: input.method(),
                params: input.params().to_string(),
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

        fn write(
            &self,
            plan: Plan,
            ticket: InputTicket,
            input: FakeInput,
        ) -> impl Future<Output = PressReport> + Send {
            let written = mouse_and_touch_input::start_writer(
                |task| {
                    tokio::spawn(task);
                },
                TabSender::for_tab(self),
                plan,
                ticket,
                input,
                (),
            );
            async move { written.await.0 }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_fake_tab_types_through_the_real_writer() {
        let tab = FakeTab::ready();
        assert_eq!(tab.platform().await, Ok(Platform::Mac));
        let (cell, ticket) = fresh();
        let input = tab.input().await;
        let report = tab
            .write(keys_plan(&["Shift+A"], None), ticket, input)
            .await;
        assert!(matches!(report, PressReport::Acked(_)), "{report:?}");
        assert_eq!(cell.delivery(), Delivery::Done);
        let (_cell, ticket) = fresh();
        let input = tab.input().await;
        let _report = tab.write(text_plan("hi"), ticket, input).await;
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
        let report = tab
            .write(keys_plan(&["Shift+A"], None), ticket, input)
            .await;
        assert!(matches!(report, PressReport::NothingWritten), "{report:?}");
        assert_eq!(cell.delivery(), Delivery::NotSent);
        let (cell, ticket) = fresh();
        let input = tab.input().await;
        let report = tab
            .write(keys_plan(&["Shift+A"], None), ticket, input)
            .await;
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
