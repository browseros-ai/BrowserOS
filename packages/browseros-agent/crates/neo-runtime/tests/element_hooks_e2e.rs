//! Neo: element action hooks (plan F4; design D5, D6, D11) on an isolated real BrowserOS neo
//! profile, through `Runtime::run` with a real hook file. `beforeAction` fires once Neo has found
//! the element, with its locator and a pinned `ElementHandle`, so a hook can swap a script's
//! `fill('{{password}}')` for the real password: the box receives it, and it reaches nothing the
//! model reads (the script's variables and console, the outcome, every call record), a failing
//! fill included. A hook that checks the pinned element's type blocks the swap into a text box;
//! the fill types into the element the hook checked even when the page then moves the selector,
//! and a retry that finds another element asks the hook again; pinning keeps the script's
//! snapshot refs; an element action no hook wants is unchanged. The page's state is read over the
//! test's own session. Values and effects only, never wording.

#[path = "support/browser.rs"]
#[expect(
    dead_code,
    reason = "the shared real-browser harness serves other suites too"
)]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::{
    BeforeHookResult, CallOrigin, CallRecord, CallResult, Delivery, PageId, RunHooks, RunOutcome,
    RunRequest, RunStatus,
};
use serde::Serialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

/// Neo: a login form: a text box, a password box, a disabled password box no fill can reach, and
/// a submit button whose submits the page counts instead of navigating.
const LOGIN: &str = "<!doctype html><title>Login</title><form id=login>\
    <label for=user>Username</label><input id=user type=text>\
    <label for=pw>Password</label><input id=pw type=password>\
    <label for=pw2>Old password</label><input id=pw2 type=password disabled>\
    <button id=submit type=submit>Sign in</button></form><script>window.__submitted = 0;\
    document.getElementById('login').addEventListener('submit', (event) => {\
    event.preventDefault(); window.__submitted += 1 })</script>";
/// Neo: what the script types, standing for a password it never holds.
const PLACEHOLDER: &str = "{{password}}";
/// Neo: the real password; only the hook file holds it. Letters, digits and dashes only, so JSON
/// escaping cannot hide it from a search of serialized text.
const SECRET: &str = "Zq9vX7-real-password";
/// Neo: the secret's first characters, which a leak that cuts the secret short still holds.
const SECRET_MARK: &str = "Zq9vX7";
/// Neo: why the swap hook refuses a box that is not a password box; hook files read it as
/// `REASON`.
const BLOCK_REASON: &str = "a secret only goes into a password box";
/// Neo: the bound of every run here; none comes near it.
const RUN_TIMEOUT: Duration = Duration::from_secs(30);

/// Neo: the swap: a fill of the placeholder into a password box types `SECRET` instead; into any
/// other box it is blocked. It asks the pinned element, never the page again by selector.
const SWAP: &str = "
neo.on('beforeAction', { kinds: ['fill'] }, async (event) => {
  if (event.args.value !== '{{password}}') return;
  const type = await event.element.evaluate((e) => e.type);
  if (type !== 'password') return { block: REASON };
  return { replaceArgs: { ...event.args, value: SECRET } };
});
";
/// Neo: the login form, whose password box mirrors its value into its `value` attribute on every
/// input, as React's controlled inputs do: the attribute then shows in every preview of the box.
const MIRROR: &str = "<!doctype html><title>Login</title><form id=login>\
    <label for=user>Username</label><input id=user type=text>\
    <label for=pw>Password</label><input id=pw type=password>\
    <button id=submit type=submit>Sign in</button></form><script>window.__submitted = 0;\
    const pw = document.getElementById('pw');\
    pw.addEventListener('input', () => pw.setAttribute('value', pw.value));\
    document.getElementById('login').addEventListener('submit', (event) => {\
    event.preventDefault(); window.__submitted += 1 })</script>";
/// Neo: two buttons side by side; each counts its clicks.
const BUTTONS: &str = "<!doctype html><title>Pay</title>\
    <style>button{position:absolute;top:20px;width:100px;height:40px}\
    #safe{left:20px}#pay{left:200px}</style>\
    <button id=safe onclick='window.__safe=(window.__safe||0)+1'>Safe</button>\
    <button id=pay onclick='window.__pay=(window.__pay||0)+1'>Pay</button>";
/// Neo: one button whose label names the site: "Next" on 127.0.0.1, "Pay" on any other host.
/// The same markup on both; each counts its clicks.
const NEXT_OR_PAY: &str = "<!doctype html><title>Checkout</title>\
    <button id=b onclick='window.__clicks=(window.__clicks||0)+1'>?</button>\
    <script>document.getElementById('b').textContent = \
    location.hostname === '127.0.0.1' ? 'Next' : 'Pay';</script>";
/// Neo: two checkboxes, the terms and a newsletter.
const CHECKBOXES: &str = "<!doctype html><title>Sign up</title>\
    <label><input id=terms type=checkbox> Terms</label>\
    <label><input id=news type=checkbox> News</label>";
/// Neo: a click policy that asks about each clicked element by its page and label, and holds
/// "Next" for a while (as a hook asking the owner would); it blocks "Pay".
const ASK_PAY: &str = "
neo.on('beforeAction', { kinds: ['click'] }, async (event, ctx) => {
  const info = await event.element.evaluate((e) => location.host + ' ' + e.textContent);
  (ctx.state.asked ??= []).push(info);
  if (info.endsWith('Pay')) return { block: 'paying needs the owner' };
  await event.page.waitForTimeout(2500);
});
neo.on('afterRun', (event, ctx) => ({ note: JSON.stringify(ctx.state.asked ?? []) }));
";
/// Neo: the script clicks `#b`; half a second later, while the click's hook holds it, it
/// navigates the tab to `OTHER`; it returns how the click ended.
const NAVIGATE_DURING_CLICK: &str = "
const clicking = page.click('#b', { timeout: 15000 })
  .then(() => 'clicked', (e) => e.name);
await page.waitForTimeout(500);
await page.goto(OTHER + '/login');
return { click: await clicking };
";
/// Neo: the swap for `type`, on the bank's password box only (`BANK`), held a while as a vault
/// lookup would be.
const BANK_TYPE: &str = "
neo.on('beforeAction', { kinds: ['type'] }, async (event, ctx) => {
  if (event.args.text !== '{{password}}') return;
  const info = await event.element.evaluate((e) => [location.origin, e.type]);
  (ctx.state.asked ??= []).push(info.join(' '));
  if (info[1] !== 'password') return { block: REASON };
  if (info[0] !== BANK) return { block: 'not the bank' };
  await event.page.waitForTimeout(2500);
  return { replaceArgs: { ...event.args, text: SECRET } };
});
neo.on('afterRun', (event, ctx) => ({ note: JSON.stringify(ctx.state.asked ?? []) }));
";
/// Neo: the swap for `type` and `press`: a placeholder typed into a password box becomes the
/// real password; any other key goes as written.
const TYPE_SWAP: &str = "
neo.on('beforeAction', { kinds: ['type', 'press'] }, async (event, ctx) => {
  (ctx.state.kinds ??= []).push(event.kind);
  if (event.args.text !== '{{password}}') return;
  const type = await event.element.evaluate((e) => e.type);
  if (type !== 'password') return { block: REASON };
  return { replaceArgs: { ...event.args, text: SECRET } };
});
neo.on('afterRun', (event, ctx) => ({ note: JSON.stringify(ctx.state.kinds ?? []) }));
";
/// Neo: refuses checking the terms, by the pinned element; any other box may be checked.
const NO_TERMS: &str = "
neo.on('beforeAction', { kinds: ['check'] }, async (event) => {
  if ((await event.element.evaluate((e) => e.id)) === 'terms') return { block: 'the owner reads the terms' };
});
";
/// Neo: the swap's script: it fills `#pw` with the placeholder, logs what it holds and returns
/// its own variables.
const SWAP_SCRIPT: &str = "
const typed = '{{password}}';
const filled = await page.fill('#pw', typed);
console.log('typed', typed, 'filled', String(filled));
return { typed, filled: String(filled) };
";
/// Neo: a swapped fill into the disabled box, which times out; the script keeps every text of
/// the error it catches, and logs it.
const DISABLED_SCRIPT: &str = "
try {
  await page.fill('#pw2', '{{password}}', { timeout: 2000 });
  return { filled: true };
} catch (error) {
  console.log('caught', String(error), error.stack);
  return { filled: false, name: error.name, message: error.message, stack: String(error.stack) };
}
";
/// Neo: the placeholder into the text box, where the swap refuses it.
const TEXT_BOX_SCRIPT: &str = "
try {
  await page.fill('#user', '{{password}}', { timeout: 5000 });
  return { filled: true };
} catch (error) {
  return { filled: false, name: error.name, message: error.message };
}
";
/// Neo: records each fill's pinned element id and locator text, and notes them as the run ends.
const PINNED: &str = "
neo.on('beforeAction', { kinds: ['fill'] }, async (event, ctx) => {
  (ctx.state.pinned ??= []).push({
    element: await event.element.evaluate((e) => e.id),
    locator: String(event.locator),
  });
});
neo.on('afterRun', (event, ctx) => ({ note: JSON.stringify(ctx.state.pinned ?? []) }));
";
/// Neo: two fills whose selectors name their boxes differently: by id, and as the first of the
/// two password boxes.
const PINNED_SCRIPT: &str = "
const user = page.locator('#user');
const password = page.locator('input[type=password]').first();
await user.fill('alice');
await password.fill('not-a-secret');
return { user: String(user), password: String(password) };
";
/// Neo: records each click's pinned element id, and notes it as the run ends.
const CLICKED: &str = "
neo.on('beforeAction', { kinds: ['click'] }, async (event, ctx) => {
  ctx.state.clicked = await event.element.evaluate((e) => e.id);
});
neo.on('afterRun', (event, ctx) => ({ note: `clicked ${ctx.state.clicked}` }));
";
/// Neo: one AI snapshot, then a fill and a click by the refs it gave, with no snapshot between;
/// it returns early, with the snapshot, if a ref is missing.
const REFS_SCRIPT: &str = r#"
const snapshot = await page.locator('body').ariaSnapshot({ mode: 'ai' });
const ref = (label) =>
  snapshot.split('\n').find((line) => line.includes(label))?.match(/\[ref=(e\d+)\]/)?.[1] ?? null;
const password = ref('textbox "Password"');
const submit = ref('button "Sign in"');
if (!password || !submit) return { password, submit, snapshot };
await page.locator(`aria-ref=${password}`).fill('{{password}}');
await page.locator(`aria-ref=${submit}`).click({ timeout: 5000 });
return { password, submit };
"#;
/// Neo: a hook file that wants only navigations, and allows them.
const GOTO_ONLY: &str = "neo.on('beforeAction', { kinds: ['goto'] }, () => {});";
/// Neo: the swap, counting its checks, that then renames the boxes once, as the page's own
/// script could between a check and the action: the checked box becomes `#checked`, and `#pw`
/// comes to name the text box. A check of the text box blocks.
const MOVING: &str = "
neo.on('beforeAction', { kinds: ['fill'] }, async (event, ctx) => {
  if (event.args.value !== '{{password}}') return;
  ctx.state.checks = (ctx.state.checks ?? 0) + 1;
  const type = await event.element.evaluate((e) => e.type);
  if (type !== 'password') return { block: REASON };
  if (!ctx.state.moved) {
    ctx.state.moved = true;
    await event.element.evaluate((e) => {
      const user = document.getElementById('user');
      e.id = 'checked';
      user.id = 'pw';
    });
  }
  return { replaceArgs: { ...event.args, value: SECRET } };
});
neo.on('afterRun', (event, ctx) => ({ note: `checks ${ctx.state.checks ?? 0}` }));
";
/// Neo: the swap, noting which box each check saw, that then replaces the checked box once with a
/// fresh copy (same id and type, marked `data-copy`), as a page that re-renders its form could
/// between a check and the action.
const REPLACING: &str = "
neo.on('beforeAction', { kinds: ['fill'] }, async (event, ctx) => {
  if (event.args.value !== '{{password}}') return;
  const box = await event.element.evaluate((e) => ({ type: e.type, copy: e.dataset.copy === 'yes' }));
  (ctx.state.seen ??= []).push(box.copy ? 'copy' : 'original');
  if (box.type !== 'password') return { block: REASON };
  if (!box.copy) {
    await event.element.evaluate((e) => {
      const copy = e.cloneNode();
      copy.dataset.copy = 'yes';
      e.replaceWith(copy);
    });
  }
  return { replaceArgs: { ...event.args, value: SECRET } };
});
neo.on('afterRun', (event, ctx) => ({ note: JSON.stringify(ctx.state.seen ?? []) }));
";

async fn open() -> Option<Tab> {
    open_page(LOGIN).await
}

/// A fresh browser on `page`, served at `/login` on the fixture's origin, under any host name.
async fn open_page(page: &str) -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    Some(
        Tab::open(
            binary,
            Fixture {
                slow_response: Duration::ZERO,
                pages: vec![("/login".to_string(), page.to_string())],
            },
            "/login",
        )
        .await,
    )
}

/// Runs `code` on `tab` with a hook file of `bodies` after `SECRET` and `REASON`, and checks the
/// run itself ended well.
async fn run(tab: &Tab, code: &str, bodies: &[&str]) -> RunOutcome {
    let hooks = format!(
        "const SECRET = {SECRET:?};\nconst REASON = {BLOCK_REASON:?};\n{}",
        bodies.concat()
    );
    let outcome = tab
        .runtime()
        .await
        .run(RunRequest {
            code: code.to_string(),
            page: PageId::new(tab.target_id.clone()),
            timeout: RUN_TIMEOUT,
            cancel: CancellationToken::new(),
            artifacts: None,
            hooks: Some(RunHooks::new(hooks)),
        })
        .await;
    assert_eq!(
        outcome.status,
        RunStatus::Ok,
        "{:?} {outcome:?}",
        outcome.hook_trace
    );
    outcome
}

/// The value of the box whose id is `id`, read over the test's own session; `null` if no element
/// has that id.
async fn value_of(tab: &Tab, id: &str) -> Value {
    tab.evaluate(&format!("document.getElementById({id:?})?.value ?? null"))
        .await
}

/// The script's records of calls named `method`, in order; never a hook's own.
fn agent_calls<'a>(outcome: &'a RunOutcome, method: &str) -> Vec<&'a CallRecord> {
    outcome
        .calls
        .iter()
        .filter(|call| call.origin == CallOrigin::Agent && call.method == method)
        .collect()
}

/// The script's one record of a call named `method`.
fn only_agent_call<'a>(outcome: &'a RunOutcome, method: &str) -> &'a CallRecord {
    let calls = agent_calls(outcome, method);
    assert_eq!(calls.len(), 1, "one {method} by the script: {calls:?}");
    calls[0]
}

fn before(call: &CallRecord) -> Option<BeforeHookResult> {
    call.hook.and_then(|hook| hook.before)
}

/// `value` as the JSON text a host hands the model.
fn json_text<T: Serialize + ?Sized>(value: &T) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|error| panic!("a run's outcome serializes: {error}"))
}

/// Each place of `outcome` the model reads, named, as text: the whole serialized outcome, then
/// its value, error, logs and notes, and each call record's params, log, result and hook.
fn model_visible(outcome: &RunOutcome) -> Vec<(String, String)> {
    let mut places = vec![
        ("outcome".to_string(), json_text(outcome)),
        ("value".to_string(), json_text(&outcome.value)),
        ("error".to_string(), json_text(&outcome.error)),
        ("logs".to_string(), outcome.logs.join("\n")),
        ("notes".to_string(), outcome.notes.join("\n")),
    ];
    for call in &outcome.calls {
        let name = format!("call {} {}", call.call_id, call.method);
        places.extend([
            (format!("{name} params"), call.params.to_string()),
            (format!("{name} log"), call.log.join("\n")),
            (format!("{name} result"), json_text(&call.result)),
            (format!("{name} hook"), json_text(&call.hook)),
        ]);
    }
    places
}

/// Fails the test naming every place of `outcome` the model reads that holds the secret, or the
/// start of it.
fn assert_no_secret(outcome: &RunOutcome) {
    let leaks: Vec<(String, String)> = model_visible(outcome)
        .into_iter()
        .filter(|(_, text)| text.contains(SECRET_MARK))
        .collect();
    assert!(leaks.is_empty(), "the real password reached {leaks:#?}");
}

/// Item 1, the swap: the script fills `#pw` with the placeholder; the hook, having asked the
/// pinned element whether it is a password box, swaps in the real password, and the box holds it.
/// The script's own variables still hold only the placeholder.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_hook_swaps_the_placeholder_for_the_real_password_in_the_box() {
    let Some(tab) = open().await else { return };
    let outcome = run(&tab, SWAP_SCRIPT, &[SWAP]).await;
    assert_eq!(
        value_of(&tab, "pw").await,
        json!(SECRET),
        "the password box holds the real password"
    );
    assert_eq!(
        outcome.value,
        Some(json!({ "typed": PLACEHOLDER, "filled": "undefined" })),
        "the script's variables hold only the placeholder"
    );
    tab.close().await;
}

/// Item 2, the leak checks on that run: the real password is nowhere the model reads: not in the
/// script's returned variables or its console, nor anywhere in the serialized outcome (value,
/// error, logs, notes, and every call record's params, log, result and hook). The fill's record
/// keeps the script's placeholder and says the hook replaced its arguments.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_the_real_password_reaches_nothing_the_model_reads() {
    let Some(tab) = open().await else { return };
    let outcome = run(&tab, SWAP_SCRIPT, &[SWAP]).await;
    assert_no_secret(&outcome);
    assert!(
        outcome.logs.iter().any(|line| line.contains(PLACEHOLDER)),
        "the script's console reached the outcome, so its check means something: {:?}",
        outcome.logs
    );
    let fill = only_agent_call(&outcome, "fill");
    assert_eq!(
        fill.params["value"],
        json!(PLACEHOLDER),
        "the record keeps the script's value: {fill:?}"
    );
    assert_eq!(
        before(fill),
        Some(BeforeHookResult::ArgsReplaced),
        "the record says the hook replaced the arguments: {fill:?}"
    );
    assert_eq!(fill.result, CallResult::Ok, "{fill:?}");
    assert_eq!(
        value_of(&tab, "pw").await,
        json!(SECRET),
        "the swap happened, so the leak checks above were not vacuous"
    );
    tab.close().await;
}

/// Item 3, a failing fill: the swap into the disabled `#pw2` times out; the error the script
/// catches (name, message, stack, and its console line), the record's result and the whole
/// outcome hold no trace of the real password, and the box stays empty.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_swapped_fill_that_fails_leaks_nothing_through_its_error() {
    let Some(tab) = open().await else { return };
    let outcome = run(&tab, DISABLED_SCRIPT, &[SWAP]).await;
    let caught = outcome
        .value
        .clone()
        .expect("the script returns what it caught");
    assert_eq!(
        (&caught["filled"], &caught["name"]),
        (&json!(false), &json!("TimeoutError")),
        "the fill into the disabled box timed out: {caught}"
    );
    assert_no_secret(&outcome);
    let fill = only_agent_call(&outcome, "fill");
    assert!(
        matches!(&fill.result, CallResult::Error { name, .. } if name == "TimeoutError"),
        "{:?}",
        fill.result
    );
    assert_eq!(
        before(fill),
        Some(BeforeHookResult::ArgsReplaced),
        "the hook swapped the secret in, so the leak checks above were not vacuous: {fill:?}"
    );
    assert_eq!(
        value_of(&tab, "pw2").await,
        json!(""),
        "nothing reached the disabled box"
    );
    tab.close().await;
}

/// Item 4, the type check blocks: the hook asks the pinned element its type and refuses to put
/// the secret into the text box `#user`: the script's fill rejects with the hook's reason, the
/// box stays empty (neither the placeholder nor the secret), and the record says blocked, its
/// input never sent.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_the_type_check_blocks_the_swap_into_a_text_box() {
    let Some(tab) = open().await else { return };
    let outcome = run(&tab, TEXT_BOX_SCRIPT, &[SWAP]).await;
    let ended = outcome
        .value
        .clone()
        .expect("the script returns how its fill ended");
    assert_eq!(
        (&ended["filled"], &ended["name"]),
        (&json!(false), &json!("Error")),
        "the fill rejected: {ended}"
    );
    assert!(
        ended["message"]
            .as_str()
            .is_some_and(|message| message.contains(BLOCK_REASON)),
        "with the hook's reason: {ended}"
    );
    assert_eq!(
        value_of(&tab, "user").await,
        json!(""),
        "the text box got neither the placeholder nor the secret"
    );
    let fill = only_agent_call(&outcome, "fill");
    assert_eq!(before(fill), Some(BeforeHookResult::Blocked), "{fill:?}");
    assert_eq!(fill.delivery, Some(Delivery::NotSent), "{fill:?}");
    assert_no_secret(&outcome);
    tab.close().await;
}

/// Item 5, the pinned element: for each fill, the element the hook got pinned and its locator
/// name the box the fill then filled: the element by its id, the locator as the script's own.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_the_hook_gets_the_element_and_locator_the_fill_acts_on() {
    let Some(tab) = open().await else { return };
    let outcome = run(&tab, PINNED_SCRIPT, &[PINNED]).await;
    let locators = outcome
        .value
        .clone()
        .expect("the script returns its locators' text");
    assert!(
        locators["user"]
            .as_str()
            .is_some_and(|text| text.contains("#user"))
            && locators["password"]
                .as_str()
                .is_some_and(|text| text.contains("password")),
        "a locator's text names its selector: {locators}"
    );
    let noted: Vec<Value> = outcome
        .notes
        .iter()
        .map(|note| serde_json::from_str(note).expect("the hook notes JSON"))
        .collect();
    assert_eq!(
        noted,
        [json!([
            { "element": "user", "locator": locators["user"] },
            { "element": "pw", "locator": locators["password"] },
        ])],
        "what each fill's hook was handed"
    );
    assert_eq!(
        (value_of(&tab, "user").await, value_of(&tab, "pw").await),
        (json!("alice"), json!("not-a-secret")),
        "each fill filled the box its hook was handed"
    );
    let fills = agent_calls(&outcome, "fill");
    assert!(
        fills.len() == 2
            && fills
                .iter()
                .all(|call| before(call) == Some(BeforeHookResult::Allowed)),
        "{fills:?}"
    );
    tab.close().await;
}

/// Item 6, snapshot refs survive pinning: after one AI snapshot, a hooked fill by the password
/// box's `aria-ref` (which pins it) and then a hooked click by the submit button's earlier ref
/// both work: the box holds the secret, the form counted one submit, and the click's pinned
/// element was the button.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_pinning_keeps_the_scripts_snapshot_refs() {
    let Some(tab) = open().await else { return };
    let outcome = run(&tab, REFS_SCRIPT, &[SWAP, CLICKED]).await;
    let refs = outcome.value.clone().expect("the script returns its refs");
    assert!(
        refs["password"].is_string()
            && refs["submit"].is_string()
            && refs.get("snapshot").is_none(),
        "the snapshot gave both refs: {refs}"
    );
    assert_eq!(
        value_of(&tab, "pw").await,
        json!(SECRET),
        "the fill by its ref was swapped"
    );
    assert_eq!(
        tab.evaluate("window.__submitted").await,
        json!(1),
        "the click by its earlier ref submitted the form"
    );
    assert_eq!(
        outcome.notes,
        ["clicked submit"],
        "the click's pinned element was the button"
    );
    let fill = only_agent_call(&outcome, "fill");
    assert_eq!(
        before(fill),
        Some(BeforeHookResult::ArgsReplaced),
        "{fill:?}"
    );
    let click = only_agent_call(&outcome, "click");
    assert_eq!(
        (before(click), &click.result),
        (Some(BeforeHookResult::Allowed), &CallResult::Ok),
        "{click:?}"
    );
    assert_no_secret(&outcome);
    tab.close().await;
}

/// Item 7, no hook wants it: with a hook file that wants only `goto`, a fill works as it does
/// without hooks and its record has no `hook`, while the run's `goto` was hooked.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_fill_no_hook_wants_runs_unchanged() {
    let Some(tab) = open().await else { return };
    let url = format!("{}/login", tab.origin);
    let code =
        format!("await page.goto({url:?}); await page.fill('#user', 'alice'); return 'filled'");
    let outcome = run(&tab, &code, &[GOTO_ONLY]).await;
    assert_eq!(outcome.value, Some(json!("filled")), "{outcome:?}");
    assert_eq!(
        value_of(&tab, "user").await,
        json!("alice"),
        "the fill filled the box"
    );
    let goto = only_agent_call(&outcome, "goto");
    assert_eq!(
        before(goto),
        Some(BeforeHookResult::Allowed),
        "the run's hooks were live: {goto:?}"
    );
    let fill = only_agent_call(&outcome, "fill");
    assert_eq!(
        (fill.hook, &fill.result, fill.delivery),
        (None, &CallResult::Ok, Some(Delivery::Done)),
        "{fill:?}"
    );
    tab.close().await;
}

/// D6's reason, beyond the brief's list: the hook's check and the action hit the same box. Once
/// the hook has checked the pinned password box, the page renames the boxes so that `#pw` names
/// the text box; the fill still types the secret into the box the hook checked, the text box
/// gets nothing, and the hook ran once, since the element never changed.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_the_fill_hits_the_box_the_hook_checked_after_the_page_moves_its_selector() {
    let Some(tab) = open().await else { return };
    let code = "const filled = await page.fill('#pw', '{{password}}'); return String(filled)";
    let outcome = run(&tab, code, &[MOVING]).await;
    assert_eq!(
        value_of(&tab, "checked").await,
        json!(SECRET),
        "the box the hook checked holds the secret"
    );
    assert_eq!(
        value_of(&tab, "pw").await,
        json!(""),
        "the text box that `#pw` came to name got nothing"
    );
    assert_eq!(
        outcome.notes,
        ["checks 1"],
        "the element never changed, so the hook ran once"
    );
    let fill = only_agent_call(&outcome, "fill");
    assert_eq!(
        before(fill),
        Some(BeforeHookResult::ArgsReplaced),
        "{fill:?}"
    );
    assert_no_secret(&outcome);
    tab.close().await;
}

/// D6's other half, beyond the brief's list: a retry that finds a different element asks the
/// hook again. Once the hook has checked the box, the page replaces it with a fresh copy; the
/// fill's retry finds the copy, the hook checks the copy too, and the copy receives the secret.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_retry_that_finds_another_element_asks_the_hook_again() {
    let Some(tab) = open().await else { return };
    let code = "const filled = await page.fill('#pw', '{{password}}'); return String(filled)";
    let outcome = run(&tab, code, &[REPLACING]).await;
    let seen: Vec<Value> = outcome
        .notes
        .iter()
        .map(|note| serde_json::from_str(note).expect("the hook notes JSON"))
        .collect();
    assert_eq!(
        seen,
        [json!(["original", "copy"])],
        "the hook checked the box, then the copy the retry found"
    );
    assert_eq!(
        tab.evaluate(
            "[document.getElementById('pw').dataset.copy, document.getElementById('pw').value]"
        )
        .await,
        json!(["yes", SECRET]),
        "the copy, now the page's `#pw`, holds the secret"
    );
    let fill = only_agent_call(&outcome, "fill");
    assert_eq!(
        (before(fill), &fill.result),
        (Some(BeforeHookResult::ArgsReplaced), &CallResult::Ok),
        "{fill:?}"
    );
    assert_no_secret(&outcome);
    tab.close().await;
}

/// Review P1-1: the page mirrors the box's value into its `value` attribute, so after the swapped
/// fill every preview of the box shows the real password: the next call's log (a `press`, which
/// no hook touches) and a strict-mode error the script keeps. None of it may hold the secret.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_page_mirroring_the_secret_cannot_leak_it_through_later_logs_and_errors() {
    let Some(tab) = open_page(MIRROR).await else {
        return;
    };
    let code = "
await page.fill('#pw', '{{password}}');
await page.press('#pw', 'Enter');
const strict = await page.locator('input').click({ timeout: 1000 }).then(() => 'clicked', (e) => e.message);
return { strict };
";
    let outcome = run(&tab, code, &[SWAP]).await;
    assert_eq!(
        tab.evaluate("document.getElementById('pw').getAttribute('value')")
            .await,
        json!(SECRET),
        "the page mirrored the secret, so the checks below mean something"
    );
    let press = only_agent_call(&outcome, "press");
    assert!(
        press.log.iter().any(|line| line.contains("<input")),
        "the press's log names the box: {press:?}"
    );
    let strict = outcome.value.as_ref().map(|value| value["strict"].clone());
    assert!(
        strict
            .as_ref()
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains("strict mode")),
        "the script kept a strict-mode error naming the boxes: {strict:?}"
    );
    assert_no_secret(&outcome);
    tab.close().await;
}

/// Review P1-2: the click's hook is asked about "Next" on 127.0.0.1; while it holds the click, the
/// script navigates the tab to another site (another renderer process), where `#b` is "Pay". The
/// click's retry finds that node: the hook is asked again, and blocks it.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_retry_on_another_site_asks_the_hook_again() {
    let Some(tab) = open_page(NEXT_OR_PAY).await else {
        return;
    };
    let other = tab.origin.replace("127.0.0.1", "localhost");
    let code = format!("const OTHER = {other:?};\n{NAVIGATE_DURING_CLICK}");
    let outcome = run(&tab, &code, &[ASK_PAY]).await;
    let page_now = tab
        .evaluate(
            "[location.hostname, document.getElementById('b')?.textContent, window.__clicks ?? 0]",
        )
        .await;
    assert_eq!(
        page_now,
        json!(["localhost", "Pay", 0]),
        "Pay was never clicked"
    );
    let asked: Vec<Value> = outcome
        .notes
        .iter()
        .map(|note| serde_json::from_str(note).expect("the hook notes JSON"))
        .collect();
    let [Value::Array(asked)] = &asked[..] else {
        panic!("one note: {asked:?}");
    };
    assert_eq!(
        asked.len(),
        2,
        "asked about Next, then about Pay: {asked:?}"
    );
    assert!(asked[1].as_str().is_some_and(|info| info.ends_with("Pay")));
    let click = only_agent_call(&outcome, "click");
    assert_eq!(before(click), Some(BeforeHookResult::Blocked), "{click:?}");
    assert_eq!(click.delivery, Some(Delivery::NotSent), "{click:?}");
    tab.close().await;
}

/// Review P1-2's control: the same flow, navigating within the site (the same renderer process).
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_retry_on_the_same_site_asks_the_hook_again() {
    let Some(tab) = open_page(NEXT_OR_PAY).await else {
        return;
    };
    let code = format!("const OTHER = {:?};\n{NAVIGATE_DURING_CLICK}", tab.origin);
    let second_blocks = "
neo.on('beforeAction', { kinds: ['click'] }, async (event, ctx) => {
  ctx.state.asked = (ctx.state.asked ?? 0) + 1;
  if (ctx.state.asked > 1) return { block: 'second question' };
  await event.page.waitForTimeout(2500);
});
neo.on('afterRun', (event, ctx) => ({ note: String(ctx.state.asked ?? 0) }));
";
    let outcome = run(&tab, &code, &[second_blocks]).await;
    assert_eq!(
        outcome.notes,
        ["2"],
        "asked again about the new document's node"
    );
    assert_eq!(
        tab.evaluate("window.__clicks ?? 0").await,
        json!(0),
        "nothing was clicked"
    );
    tab.close().await;
}

/// Review P1-2 with a secret: `type` swaps the placeholder into the bank's box only. While the hook
/// waits for the vault, the script navigates to another site; the retry finds `#pw` there, the
/// hook is asked again and refuses, and the other site's box never gets the password.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_swap_decided_on_one_site_never_types_into_another() {
    let Some(tab) = open().await else { return };
    let bank = tab.origin.clone();
    let other = bank.replace("127.0.0.1", "localhost");
    let code = format!(
        "const OTHER = {other:?};
const typing = page.type('#pw', '{{{{password}}}}', {{ timeout: 15000 }})
  .then(() => 'typed', (e) => e.name);
await page.waitForTimeout(500);
await page.goto(OTHER + '/login');
return {{ typed: await typing }};
"
    );
    let hooks = format!("const BANK = {bank:?};\n{BANK_TYPE}");
    let outcome = run(&tab, &code, &[&hooks]).await;
    let page_now = tab
        .evaluate("[location.hostname, document.getElementById('pw')?.value ?? null]")
        .await;
    assert_eq!(page_now[0], json!("localhost"));
    assert_ne!(
        page_now[1],
        json!(SECRET),
        "the bank's password went to the other site"
    );
    let asked: Vec<Value> = outcome
        .notes
        .iter()
        .map(|note| serde_json::from_str(note).expect("the hook notes JSON"))
        .collect();
    let [Value::Array(asked)] = &asked[..] else {
        panic!("one note: {asked:?}");
    };
    assert_eq!(
        asked.len(),
        2,
        "asked on the bank, then on the other site: {asked:?}"
    );
    assert_no_secret(&outcome);
    tab.close().await;
}

/// Review P2-1: the hook refuses clicks on `#pay`, by the pinned element. A forced click on `#safe`
/// at a position that lands on `#pay` keeps its hit check: it never clicks `#pay`.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_forced_click_cannot_land_on_another_element_than_the_hooks() {
    let Some(tab) = open_page(BUTTONS).await else {
        return;
    };
    let hooks = "
neo.on('beforeAction', { kinds: ['click'] }, async (event) => {
  if ((await event.element.evaluate((e) => e.id)) === 'pay') return { block: 'paying needs the owner' };
});
";
    let code = "
const direct = await page.click('#pay', { timeout: 2000 }).then(() => 'clicked', (e) => e.name);
const forced = await page.click('#safe', { force: true, position: { x: 230, y: 20 }, timeout: 2000 })
  .then(() => 'clicked', (e) => e.name);
const plain = await page.click('#safe', { force: true, timeout: 2000 }).then(() => 'clicked', (e) => e.name);
return { direct, forced, plain };
";
    let outcome = run(&tab, code, &[hooks]).await;
    assert_eq!(
        tab.evaluate("[window.__safe ?? 0, window.__pay ?? 0]")
            .await,
        json!([1, 0]),
        "only the forced click that stays on #safe landed: {:?}",
        outcome.value
    );
    assert_eq!(
        outcome.value,
        Some(json!({ "direct": "Error", "forced": "TimeoutError", "plain": "clicked" }))
    );
    tab.close().await;
}

/// Review tests 3: a swapped `type` types the real password into the box, and a `press` after it
/// is asked about too; nothing the model reads holds the secret.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_swapped_type_types_the_real_password() {
    let Some(tab) = open().await else { return };
    let code = "
await page.type('#pw', '{{password}}');
await page.press('#pw', 'Enter');
return 'typed';
";
    let outcome = run(&tab, code, &[TYPE_SWAP]).await;
    assert_eq!(value_of(&tab, "pw").await, json!(SECRET));
    assert_eq!(
        outcome.notes,
        [r#"["type","press"]"#],
        "{:?}",
        outcome.notes
    );
    let typed = only_agent_call(&outcome, "type");
    assert_eq!(typed.params["text"], json!(PLACEHOLDER), "{typed:?}");
    assert_eq!(
        before(typed),
        Some(BeforeHookResult::ArgsReplaced),
        "{typed:?}"
    );
    assert_eq!(typed.delivery, Some(Delivery::Done), "{typed:?}");
    assert_no_secret(&outcome);
    tab.close().await;
}

/// Review tests 3: an element-keyed `check` policy: the hook refuses checking the terms by the
/// pinned box; the newsletter box is checked, the terms box stays unchecked and its record says
/// blocked with nothing sent.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_check_policy_reads_the_pinned_box() {
    let Some(tab) = open_page(CHECKBOXES).await else {
        return;
    };
    let code = "
await page.check('#news');
const terms = await page.check('#terms', { timeout: 2000 }).then(() => 'checked', (e) => e.name);
return { terms };
";
    let outcome = run(&tab, code, &[NO_TERMS]).await;
    assert_eq!(outcome.value, Some(json!({ "terms": "Error" })));
    assert_eq!(
        tab.evaluate(
            "[document.getElementById('news').checked, document.getElementById('terms').checked]"
        )
        .await,
        json!([true, false])
    );
    let checks = agent_calls(&outcome, "check");
    assert_eq!(checks.len(), 2, "{checks:?}");
    assert_eq!(
        (before(checks[0]), checks[0].delivery),
        (Some(BeforeHookResult::Allowed), Some(Delivery::Done))
    );
    assert_eq!(
        (before(checks[1]), checks[1].delivery),
        (Some(BeforeHookResult::Blocked), Some(Delivery::NotSent))
    );
    tab.close().await;
}
