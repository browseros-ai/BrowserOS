//! The host's private typing as the host meets it: `POST /v1/tabs/{tabId}/fill` and `/fill/check`
//! on the real host API, behind the real auth, through the real door, `host.fill`, the takeover
//! lock and the fill's request record, over the fake browser (protected-fill design, sections 4,
//! 7, 8, 9 and 10.1).
//!
//! The entry itself (protected-fill pull request 2) is a stand-in here, [`ScriptedEntry`],
//! behind the same seam production calls (`builtins::PrivateEntry`): it answers what each test
//! scripts, asks the door's rules and marks the op sent as the real entry promises to, and sees
//! only each value's length. The real browser entry is covered by neo-playwright's
//! `private_entry_live` suite.

use std::cmp::Ordering as CmpOrdering;
use std::error::Error;
use std::fs;
use std::future::{self, Future};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant as StdInstant};

use browseros_core::TabId;
use futures_util::future::BoxFuture;
use neo_http::PictureVerdict;
use neo_playwright::{
    BoxFacts, BoxLook, CoverIn, Drawing, Entry, EntryCode, EntryError, EntryOutcome, EntryRequest,
    LookRequest, OpEnv, Sending,
};
use neo_trace::{Delivery, Effect, RunId};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::UnixStream;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::{self, Instant};
use tracing::Dispatch;
use tracing::subscriber::{self, NoSubscriber};

use super::{Host, Reply, TOKEN, TestResult, request, within};
use crate::browser::TabsScope;
use crate::builtins::{ENTRY_BACKSTOP, HOST_FILL, HOST_FILL_CHECK, HOST_FILL_CODE, PrivateEntry};
use crate::dispatch::{
    Call, ErrorCode, LeafError, Method, MethodSpec, OpCtx, OpFailure, PageTarget, RegistryBuilder,
    RegistryError, RunCtx, RunEnd, RunStart,
};
use crate::host::fill_ledger::LEDGER_FILE;
use crate::host::filled_tabs::FILLED_TABS_FILE;

/// A password with characters JSON and URLs escape.
const PASSWORD: &str = "CANARY-route-pw\"7Q\\x/";
/// A card number, scanned for whole and grouped in fours as a card box shows it.
const CARD: &str = "4000056655665556";
/// A one-time code, long enough not to turn up by chance in a timestamp or an id.
const CODE: &str = "24681357";
/// The control's value: typed by a script's plain fill, which records it. The scan must find it.
const CONTROL: &str = "CONTROL-plain-fill-9Z";

/// What the stand-in entry does on its next calls.
#[derive(Clone, Copy, Debug, Default)]
pub(super) enum Script {
    /// Every box takes its value.
    #[default]
    Enter,
    /// Every character went in and the page moved after the last one.
    EnterPageMoved,
    /// Every box takes its value; the first under its field's own drawing, which drew the value
    /// (a code), the second under one that did not (rulings R55, R56).
    EnterUnderDrawings,
    /// The look refuses box `at` with `code` before any key.
    Refuse { at: usize, code: EntryCode },
    /// Box `at` got some of its characters, then `code` stopped the entry.
    Uncertain { at: usize, code: EntryCode },
    /// Marks the op sent, then waits for the test's [`ScriptedEntry::release`] and asks the rules
    /// again before its first key, as the real entry does before each field.
    Hold,
    /// Never answers: only the deadline or a shutdown ends it.
    Hang,
}

/// How each box's field drawing figured in an entry `script` answers: the first two under their
/// fields' drawings for [`Script::EnterUnderDrawings`], the first of which drew its value; none
/// otherwise.
fn drawings_of(script: Script, fields: usize) -> Vec<Drawing> {
    let mut drawings = vec![Drawing::None; fields];
    if matches!(script, Script::EnterUnderDrawings) {
        drawings[0] = Drawing::Drawn;
        drawings[1] = Drawing::Under;
    }
    drawings
}

/// The stand-in for the entry behind `builtins::PrivateEntry`.
#[derive(Default)]
pub(super) struct ScriptedEntry {
    script: Mutex<Script>,
    /// How many times the door let an entry run at all, whatever it then did.
    calls: AtomicUsize,
    /// The values' lengths each entry was handed, in call order: what typed.
    entered: Mutex<Vec<Vec<usize>>>,
    /// The tabs an entry marked sent on, for the camera: never cleared, as the real entry's.
    took: Mutex<Vec<TabId>>,
    /// The tabs whose page shows a private box's value, as the app's reading answers it.
    shows: Mutex<Vec<TabId>>,
    looks: AtomicUsize,
    holding: Notify,
    release: Notify,
}

impl ScriptedEntry {
    pub(super) fn script(&self, script: Script) {
        *self.script.lock().unwrap_or_else(PoisonError::into_inner) = script;
    }

    /// Marks the op sent, as the real entry does once every look passed, and remembers the tab
    /// as the real entry's `took_private_text` does, for the tab's life.
    fn sent(&self, env: &dyn OpEnv, tab: TabId) {
        env.mark_sent();
        self.took
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(tab);
    }

    /// Records what one entry typed into its boxes.
    fn typed(&self, lengths: &[usize]) {
        self.entered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(lengths.to_vec());
    }

    /// How many times the entry ran: zero when the door refused every fill before it.
    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// What `tab` shows, as the app's reading (`Playwright::picture_check`) answers it.
    pub(super) fn verdict(&self, tab: TabId) -> PictureVerdict {
        if self
            .shows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&tab)
        {
            PictureVerdict::Private
        } else {
            PictureVerdict::Clear {
                reading: "the page".to_owned(),
            }
        }
    }

    /// Makes `tab`'s page show a private box's value, as a script's own fill of a card box does.
    pub(super) fn show(&self, tab: TabId) {
        self.shows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(tab);
    }

    /// Whether an entry went past "sent" on `tab`: the real entry's `took_private_text`.
    pub(super) fn took(&self, tab: TabId) -> bool {
        self.took
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&tab)
    }

    /// The lengths of the values each entry was handed: one row per call that typed.
    pub(super) fn entered(&self) -> Vec<Vec<usize>> {
        self.entered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    async fn until_holding(&self) {
        self.holding.notified().await;
    }

    fn release(&self) {
        self.release.notify_one();
    }
}

impl PrivateEntry for ScriptedEntry {
    fn enter<'a>(&'a self, env: &'a dyn OpEnv, request: EntryRequest) -> BoxFuture<'a, Entry> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let lengths: Vec<usize> = request
                .fields
                .iter()
                .map(|field| field.value.char_count())
                .collect();
            let fields = lengths.len();
            let script = *self.script.lock().unwrap_or_else(PoisonError::into_inner);
            // The real entry's first step: the rules, then "sent", before any focus.
            let before_keys = |at: usize| match env.recheck() {
                Ok(()) => None,
                Err(refusal) => Some(Entry::refused_before_sending(
                    fields,
                    at,
                    EntryError::refused(refusal, Sending::NotYet),
                )),
            };
            match script {
                Script::Refuse { at, code } => {
                    Entry::refused_before_sending(fields, at, EntryError::of(code, Sending::NotYet))
                }
                Script::Hang => future::pending().await,
                Script::Enter | Script::EnterPageMoved | Script::EnterUnderDrawings => {
                    if let Some(refused) = before_keys(0) {
                        return refused;
                    }
                    self.sent(env, request.tab);
                    self.typed(&lengths);
                    Entry {
                        outcome: if matches!(script, Script::EnterPageMoved) {
                            EntryOutcome::EnteredPageMoved
                        } else {
                            EntryOutcome::Entered
                        },
                        delivery: Delivery::Done,
                        at: None,
                        typed: lengths,
                        error: None,
                        drawings: drawings_of(script, fields),
                    }
                }
                Script::Uncertain { at, code } => {
                    if let Some(refused) = before_keys(0) {
                        return refused;
                    }
                    self.sent(env, request.tab);
                    let typed: Vec<usize> = lengths
                        .iter()
                        .enumerate()
                        .map(|(index, length)| match index.cmp(&at) {
                            CmpOrdering::Less => *length,
                            CmpOrdering::Equal => length / 2,
                            CmpOrdering::Greater => 0,
                        })
                        .collect();
                    self.typed(&typed);
                    let error = EntryError::of(code, Sending::Sent);
                    Entry {
                        outcome: EntryOutcome::Uncertain,
                        delivery: neo_playwright::delivery_of(Sending::Sent, Some(&error)),
                        at: Some(at),
                        typed,
                        error: Some(error),
                        drawings: vec![Drawing::None; fields],
                    }
                }
                Script::Hold => {
                    if let Some(refused) = before_keys(0) {
                        return refused;
                    }
                    self.sent(env, request.tab);
                    self.holding.notify_one();
                    self.release.notified().await;
                    // Before the first field's focus and keys: a lock taken meanwhile stops it.
                    if let Err(refusal) = env.recheck() {
                        let error = EntryError::refused(refusal, Sending::Sent);
                        return Entry {
                            outcome: EntryOutcome::Refused,
                            delivery: neo_playwright::delivery_of(Sending::Sent, Some(&error)),
                            at: Some(0),
                            typed: vec![0; fields],
                            error: Some(error),
                            drawings: vec![Drawing::None; fields],
                        };
                    }
                    self.typed(&lengths);
                    Entry {
                        outcome: EntryOutcome::Entered,
                        delivery: Delivery::Done,
                        at: None,
                        typed: lengths,
                        error: None,
                        drawings: vec![Drawing::None; fields],
                    }
                }
            }
        })
    }

    fn look<'a>(
        &'a self,
        _env: &'a dyn OpEnv,
        request: &'a LookRequest,
    ) -> BoxFuture<'a, Result<Vec<BoxLook>, EntryError>> {
        Box::pin(async move {
            self.looks.fetch_add(1, Ordering::SeqCst);
            let script = *self.script.lock().unwrap_or_else(PoisonError::into_inner);
            Ok(request
                .fields
                .iter()
                .enumerate()
                .map(|(index, field)| BoxLook {
                    facts: Some(BoxFacts {
                        origin: "https://pay.example".to_owned(),
                        kind: "text".to_owned(),
                        autocomplete: if field.box_ref == "f2e4" {
                            "cc-number".to_owned()
                        } else {
                            String::new()
                        },
                        max_length: Some(19),
                        input_mode: "numeric".to_owned(),
                        name: "cardnumber".to_owned(),
                        id: String::new(),
                        sized: true,
                        empty: true,
                    }),
                    refusal: match script {
                        Script::Refuse { at, code } if at == index => {
                            Some(EntryError::of(code, Sending::NotYet))
                        }
                        _ => None,
                    },
                })
                .collect())
        })
    }
}

/// `test.plainFill`: a script's ordinary fill, whose value the door records as its input. The
/// canary scan must find this one, or the scan cannot fail.
struct PlainFill;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlainFillInput {
    page: PageTarget,
    value: String,
}

impl Method for PlainFill {
    type Input = PlainFillInput;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.plainFill", Effect::Write)
    }

    fn target(input: &PlainFillInput) -> Option<&PageTarget> {
        Some(&input.page)
    }

    fn call(
        &self,
        op: &OpCtx<Self>,
        input: PlainFillInput,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        // A call-log line with the value, as a careless typing helper would write: the door logs
        // it at debug for an ordinary op, which the log scan's control relies on, and never for
        // a protected one (ruling R35).
        op.note(format!("typed {}", input.value));
        let sent = op.recheck().map(|()| {
            op.mark_sent();
            Value::Null
        });
        future::ready(sent)
    }
}

/// The controls a script calls in these tests.
pub(super) fn register_controls(registry: &mut RegistryBuilder) -> Result<(), RegistryError> {
    registry.define(PlainFill)
}

/// A fill body for `tab` with the three canaries: a password, a card, a code.
fn canary_body(extra: &Value) -> Value {
    let mut body = json!({
        "taskId": "task-42",
        "url": "https://shop.example/checkout",
        "fields": [
            { "ref": "e5", "value": PASSWORD, "cut": true,
              "expect": { "origin": "https://shop.example", "type": "password" } },
            { "ref": "f2e4", "value": "4000 0566 5566 5556", "spaced": true,
              "expect": { "origin": "https://pay.example", "autocomplete": "cc-number" } },
            { "ref": "f3e2", "value": CODE, "keys": true,
              "expect": { "origin": "https://pay.example" } }
        ]
    });
    if let (Some(body), Some(extra)) = (body.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            body.insert(key.clone(), value.clone());
        }
    }
    body
}

/// The lengths the canary body's values have, as `typed` and `characters` count them.
const LENGTHS: [usize; 3] = [21, 19, 8];

async fn fill(host: &Host, tab: TabId, body: &Value) -> Result<Reply, Box<dyn Error>> {
    request(
        &host.socket,
        "POST",
        &format!("/v1/tabs/{}/fill", tab.0),
        Some(TOKEN),
        Some(body),
    )
    .await
}

async fn get(host: &Host, path: &str) -> Result<Reply, Box<dyn Error>> {
    request(&host.socket, "GET", path, Some(TOKEN), None).await
}

/// A fill-ready host: serving, with an owner tab open.
async fn ready() -> Result<(Host, TabId), Box<dyn Error>> {
    let host = Host::start(Some(TOKEN)).await?;
    let tab = host
        .fake
        .open_tab("https://shop.example/checkout", "Checkout");
    host.until_serving().await?;
    Ok((host, tab))
}

/// Every way each canary could be written down: UTF-8, UTF-16, URL-encoded, JSON-escaped, and
/// the card grouped in fours.
fn written_forms(value: &str) -> Vec<Vec<u8>> {
    let mut forms = vec![value.as_bytes().to_vec()];
    forms.push(
        value
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<u8>>(),
    );
    let url: String = value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(byte).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect();
    forms.push(url.into_bytes());
    let json = serde_json::to_string(value).unwrap_or_default();
    forms.push(json.trim_matches('"').as_bytes().to_vec());
    if value.len() == 16 && value.bytes().all(|byte| byte.is_ascii_digit()) {
        let grouped: Vec<&str> = (0..4)
            .filter_map(|group| value.get(group * 4..group * 4 + 4))
            .collect();
        forms.push(grouped.join(" ").into_bytes());
    }
    forms
}

fn holds_any(haystack: &[u8], value: &str) -> bool {
    written_forms(value).iter().any(|form| {
        !form.is_empty()
            && haystack
                .windows(form.len())
                .any(|window| window == form.as_slice())
    })
}

/// Every file under `dir` that holds `value` in any written form.
fn files_holding(dir: &Path, value: &str) -> Result<Vec<String>, Box<dyn Error>> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(path) = pending.pop() {
        if path.is_dir() {
            for entry in fs::read_dir(&path)? {
                pending.push(entry?.path());
            }
        } else if path.is_file() && holds_any(&fs::read(&path)?, value) {
            found.push(path.display().to_string());
        }
    }
    Ok(found)
}

/// Asserts no canary is in any file Neo wrote or in any of `texts`.
fn assert_no_canary(dir: &Path, texts: &[&str]) -> TestResult {
    for canary in [PASSWORD, CARD, CODE] {
        let files = files_holding(dir, canary)?;
        assert!(files.is_empty(), "{canary:?} is in {files:?}");
        for text in texts {
            assert!(
                !holds_any(text.as_bytes(), canary),
                "{canary:?} is in {text}"
            );
        }
    }
    Ok(())
}

/// A script run whose code is `code` and that calls `name` with `input`, as the bridge submits
/// it. The run's id, and how the call ended.
async fn script_call(
    host: &Host,
    code: &str,
    name: &str,
    input: Value,
) -> Result<(RunId, Result<Value, OpFailure>), Box<dyn Error>> {
    let view = host.link.view().ok_or("the link is up")?;
    let id = RunId::new();
    let (run, guard) = RunCtx::start(
        &host.dispatcher,
        RunStart {
            id,
            code: code.to_owned(),
            request_id: None,
            task_id: None,
            host_data: None,
            view,
            deadline: Instant::now() + Duration::from_secs(10),
            default_page: None,
            tabs_scope: TabsScope::Browser,
        },
    );
    let reply = host
        .dispatcher
        .submit(&run, Call::script(name.to_owned(), input, None))
        .wait()
        .await;
    let mut ops = run.close_ops();
    while ops.join_next().await.is_some() {}
    guard.finish(RunEnd {
        status: neo_trace::RunStatus::Ok,
        error: None,
        result: None,
        replay: None,
    });
    Ok((id, reply.outcome))
}

/// `GET /v1/events` held open from the start of a test, every byte it streams kept.
struct EventTap {
    seen: Arc<Mutex<Vec<u8>>>,
    reading: JoinHandle<()>,
}

impl EventTap {
    /// Opens the stream and waits for its head, so every event after this is caught.
    async fn open(socket: &Path) -> Result<Self, Box<dyn Error>> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = UnixStream::connect(socket).await?;
        stream
            .write_all(
                format!(
                    "GET /v1/events HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {TOKEN}\r\nAccept: text/event-stream\r\n\r\n"
                )
                .as_bytes(),
            )
            .await?;
        let seen: Arc<Mutex<Vec<u8>>> = Arc::default();
        let kept = Arc::clone(&seen);
        let reading = tokio::spawn(async move {
            let mut chunk = vec![0_u8; 16 * 1024];
            while let Ok(read) = stream.read(&mut chunk).await
                && read > 0
            {
                if let Some(bytes) = chunk.get(..read) {
                    kept.lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .extend_from_slice(bytes);
                }
            }
        });
        let tap = Self { seen, reading };
        within(async {
            while !tap.text().starts_with("HTTP/1.1 200") {
                time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        Ok(tap)
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.seen.lock().unwrap_or_else(PoisonError::into_inner))
            .into_owned()
    }
}

impl Drop for EventTap {
    fn drop(&mut self) {
        self.reading.abort();
    }
}

/// A log writer for one test's tracing events.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Captured {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap_or_else(PoisonError::into_inner)).into_owned()
    }
}

/// The canary suite (design section 8): values through the route on every path, then every file
/// Neo wrote, every host read, every answer and every log line searched for each value in each
/// written form. A script's plain fill of the control value proves the scan finds what is there.
#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "one suite: every path's values, then one scan of every surface, with its control"
)]
async fn the_value_is_in_no_answer_record_file_or_log_on_any_path() -> TestResult {
    let logs = Captured::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || writer.clone())
        .finish();
    // The test's runtime has one thread, so every task's events reach this subscriber.
    let _logging = subscriber::set_default(subscriber);
    // A second registered subscriber, alive for the whole test. While only one is registered,
    // tracing-core takes a new callsite's interest from the default of the thread that reaches it
    // first and caches it for the process: a line of Neo's that another test's thread reached
    // first would never reach this capture, and the scan below would pass while Neo logged a
    // value. Registering one rebuilds every known callsite against the real list, and with two
    // registered that shortcut is off for later callsites too (tracing-core 0.1.36,
    // `Rebuilder::JustOne`).
    let _second = Dispatch::new(NoSubscriber::default());
    let (host, tab) = ready().await?;
    let events = EventTap::open(&host.socket).await?;
    let mut texts = Vec::new();

    // Entered; entered and the page moved; each refusal of the look; a stop mid-entry; a repeat.
    let paths = [
        Script::Enter,
        Script::EnterPageMoved,
        Script::Refuse {
            at: 0,
            code: EntryCode::BoxCovered(CoverIn::OwnDocument),
        },
        Script::Refuse {
            at: 1,
            code: EntryCode::WrongOrigin,
        },
        Script::Refuse {
            at: 2,
            code: EntryCode::ValueTooLong,
        },
        Script::Refuse {
            at: 1,
            code: EntryCode::NotFocused,
        },
        Script::Uncertain {
            at: 1,
            code: EntryCode::FocusLost,
        },
        Script::Uncertain {
            at: 2,
            code: EntryCode::PageMoved,
        },
        Script::Uncertain {
            at: 0,
            code: EntryCode::ValueNotTaken,
        },
    ];
    for (index, script) in paths.into_iter().enumerate() {
        host.entry.script(script);
        let body = canary_body(&json!({ "requestId": format!("canary-{index}") }));
        let answer = fill(&host, tab, &body).await?;
        assert_eq!(answer.status, 200, "{script:?}: {answer:?}");
        // The repeat answers from the record.
        let repeat = fill(&host, tab, &body).await?;
        assert_eq!(
            repeat.body, answer.body,
            "{script:?}: a repeat gets the same answer"
        );
        texts.push(answer.body.to_string());
        let run = answer.body["runId"].as_str().ok_or("a runId")?;
        let record = get(&host, &format!("/v1/runs/{run}?wait=10")).await?;
        texts.push(record.body.to_string());
        if let Some(operation) = record.body["operations"][0]["id"].as_str() {
            texts.push(
                get(&host, &format!("/v1/operations/{operation}"))
                    .await?
                    .body
                    .to_string(),
            );
        }
    }
    // A lock refuses; a timeout that the backstop ends.
    let lock = host.take_lock(tab).await?;
    texts.push(
        fill(&host, tab, &canary_body(&json!({})))
            .await?
            .body
            .to_string(),
    );
    let lock_id = lock.body["id"].as_str().ok_or("a lock id")?;
    request(
        &host.socket,
        "DELETE",
        &format!("/v1/locks/{lock_id}"),
        Some(TOKEN),
        None,
    )
    .await?;
    host.entry.script(Script::Hang);
    let hung = fill(&host, tab, &canary_body(&json!({ "timeoutMs": 1000 }))).await?;
    assert_eq!(hung.body["error"]["code"], "run_timeout", "{hung:?}");
    texts.push(hung.body.to_string());
    // Refusals of the request itself: a bad body, a browser's request.
    let bad = fill(
        &host,
        tab,
        &json!({ "fields": [{ "ref": "e1", "value": CARD.parse::<u64>()? ,
        "expect": { "origin": "x" } }] }),
    )
    .await?;
    assert_eq!(bad.status, 400, "{bad:?}");
    texts.push(bad.body.to_string());
    texts.push(get(&host, "/v1/trace").await?.body.to_string());
    texts.push(get(&host, "/v1/runs").await?.body.to_string());

    // The control: a script's plain fill, which puts its value in the run's code and the op's
    // input. The scan must find it in each place the protected fill keeps clean, or it cannot
    // fail: the run's code, the operation's args, the events, and the trace's files.
    let (control_run, control) = script_call(
        &host,
        &format!("await page.locator('#pw').fill('{CONTROL}')"),
        "test.plainFill",
        json!({ "page": { "$neo": "page", "tab": tab.0 }, "value": CONTROL }),
    )
    .await?;
    assert!(control.is_ok(), "{control:?}");
    host.sink.flush().await?;
    let record = get(&host, &format!("/v1/runs/{control_run}?wait=10")).await?;
    assert!(
        holds_any(record.body["code"].to_string().as_bytes(), CONTROL),
        "in the run's code: {record:?}"
    );
    assert!(
        holds_any(
            record.body["operations"][0]["args"].to_string().as_bytes(),
            CONTROL
        ),
        "in the op's args: {record:?}"
    );
    let control_op = record.body["operations"][0]["id"]
        .as_str()
        .ok_or("the control's op")?
        .to_owned();
    let operation = get(&host, &format!("/v1/operations/{control_op}")).await?;
    assert!(
        holds_any(operation.body.to_string().as_bytes(), CONTROL),
        "in the op's own read: {operation:?}"
    );
    let trace = get(&host, "/v1/trace").await?;
    assert!(
        holds_any(trace.body.to_string().as_bytes(), CONTROL),
        "in the trace's tail"
    );
    within(async {
        while !holds_any(events.text().as_bytes(), CONTROL) {
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|_unseen| "the control is in the events")?;
    let found = files_holding(host.dir.path(), CONTROL)?;
    assert!(!found.is_empty(), "the control is in the trace's files");
    let streamed = events.text();
    assert!(
        streamed.contains("host.fill"),
        "the fills' events were streamed too"
    );
    texts.push(streamed);

    let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
    let log = logs.text();
    assert!(
        log.lines()
            .any(|line| line.contains("op note") && line.contains(CONTROL)),
        "the plain fill's call-log line is in Neo's captured log, so the log scan can fail"
    );
    let mut all = texts.clone();
    all.push(&log);
    assert_no_canary(host.dir.path(), &all)?;
    Ok(())
}

/// What the fill records instead of the value (design section 9), and that it takes no picture.
#[tokio::test]
async fn a_fill_is_one_protected_operation_with_counts_and_no_page_or_picture() -> TestResult {
    let (host, tab) = ready().await?;
    let answer = fill(&host, tab, &canary_body(&json!({}))).await?;
    assert_eq!(answer.status, 200, "{answer:?}");
    assert_eq!(answer.body["outcome"], "entered");
    assert_eq!(answer.body["delivery"], "done");
    assert_eq!(answer.body["typed"], json!(LENGTHS));
    assert!(answer.body.get("error").is_none(), "{answer:?}");
    assert_eq!(host.entry.entered(), vec![LENGTHS.to_vec()]);
    let run = answer.body["runId"].as_str().ok_or("a runId")?;
    let record = get(&host, &format!("/v1/runs/{run}?wait=10")).await?;
    let body = &record.body;
    assert_eq!(body["status"], "ok", "{body}");
    assert_eq!(body["code"], HOST_FILL_CODE, "{body}");
    assert_eq!(
        body["result"], answer.body,
        "the run's result is the whole answer"
    );
    let operations = body["operations"].as_array().ok_or("operations")?;
    assert_eq!(operations.len(), 1, "one operation: {body}");
    let operation = &operations[0];
    assert_eq!(operation["name"], HOST_FILL);
    assert_eq!(operation["kind"], "protected");
    assert_eq!(operation["effect"], "write");
    assert_eq!(operation["tabId"], tab.0);
    assert_eq!(operation["delivery"], "done");
    assert_eq!(operation["hasScreenshot"], false);
    assert!(
        operation["page"].get("url").is_none(),
        "no address: {operation}"
    );
    assert!(
        operation["page"].get("title").is_none(),
        "no title: {operation}"
    );
    let fields = operation["args"]["fields"]
        .as_array()
        .ok_or("args.fields")?;
    assert_eq!(fields.len(), 3);
    for (field, length) in fields.iter().zip(LENGTHS) {
        assert_eq!(field["characters"], length, "{field}");
        assert!(field.get("value").is_none(), "{field}");
    }
    assert_eq!(fields[1]["expect"]["autocomplete"], "cc-number");
    assert_eq!(fields[2]["keys"], true);
    let images = files_with_extension(host.dir.path(), &["jpg", "jpeg", "png", "webp"])?;
    assert!(images.is_empty(), "no picture: {images:?}");
    Ok(())
}

fn files_with_extension(dir: &Path, extensions: &[&str]) -> Result<Vec<String>, Box<dyn Error>> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(path) = pending.pop() {
        if path.is_dir() {
            for entry in fs::read_dir(&path)? {
                pending.push(entry?.path());
            }
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extensions.contains(&extension))
        {
            found.push(path.display().to_string());
        }
    }
    Ok(found)
}

/// A refusal before any key: `refused`, nothing typed, `not_sent`, `retryable` (ruling R5), and
/// a refused fill's id is spent.
#[tokio::test]
async fn a_refusal_types_nothing_and_spends_its_request_id() -> TestResult {
    let (host, tab) = ready().await?;
    host.entry.script(Script::Refuse {
        at: 1,
        code: EntryCode::BoxCovered(CoverIn::OwnDocument),
    });
    let body = canary_body(&json!({ "requestId": "card-1" }));
    let refused = fill(&host, tab, &body).await?;
    assert_eq!(refused.body["outcome"], "refused", "{refused:?}");
    assert_eq!(refused.body["delivery"], "not_sent");
    assert_eq!(refused.body["at"], 1);
    assert_eq!(refused.body["typed"], json!([0, 0, 0]));
    assert_eq!(refused.body["error"]["code"], "box_covered");
    assert_eq!(refused.body["error"]["owner"], "website");
    assert_eq!(refused.body["error"]["retryable"], true);
    // The page is free now, but the id is spent: the same refusal, nothing typed.
    host.entry.script(Script::Enter);
    let again = fill(&host, tab, &body).await?;
    assert_eq!(again.body, refused.body);
    assert!(host.entry.entered().is_empty());
    // A new id types.
    let fresh = fill(&host, tab, &canary_body(&json!({ "requestId": "card-2" }))).await?;
    assert_eq!(fresh.body["outcome"], "entered", "{fresh:?}");
    // The run of the refusal ends ok and carries the whole answer (ruling R8).
    let run = refused.body["runId"].as_str().ok_or("a runId")?;
    let record = get(&host, &format!("/v1/runs/{run}?wait=10")).await?;
    assert_eq!(record.body["status"], "ok");
    assert_eq!(record.body["result"]["at"], 1);
    assert_eq!(record.body["operations"][0]["outcome"], "error");
    assert_eq!(record.body["operations"][0]["delivery"], "not_sent");
    Ok(())
}

/// Design section 4.6: with a `requestId` a fill types once, whether the repeat comes while it
/// runs or after; without one, a repeat types again.
#[tokio::test]
async fn a_repeat_types_once_with_a_request_id_and_again_without() -> TestResult {
    let (host, tab) = ready().await?;
    host.entry.script(Script::Hold);
    let body = canary_body(&json!({ "requestId": "pw-1", "taskId": "task-7" }));
    let first = tokio::spawn({
        let socket = host.socket.clone();
        let body = body.clone();
        async move {
            request(
                &socket,
                "POST",
                &format!("/v1/tabs/{}/fill", tab.0),
                Some(TOKEN),
                Some(&body),
            )
            .await
            .map_err(|error| error.to_string())
        }
    });
    within(host.entry.until_holding()).await?;
    let second = tokio::spawn({
        let socket = host.socket.clone();
        let body = body.clone();
        async move {
            request(
                &socket,
                "POST",
                &format!("/v1/tabs/{}/fill", tab.0),
                Some(TOKEN),
                Some(&body),
            )
            .await
            .map_err(|error| error.to_string())
        }
    });
    // The repeat joined: it has not typed, and it waits for the first call's answer.
    time::sleep(Duration::from_millis(100)).await;
    assert!(
        !second.is_finished(),
        "a repeat while the first runs waits for it"
    );
    host.entry.release();
    let first = within(first).await???;
    let second = within(second).await???;
    assert_eq!(first.body["outcome"], "entered", "{first:?}");
    assert_eq!(
        second.body, first.body,
        "the repeat gets the first call's answer"
    );
    let after = fill(&host, tab, &body).await?;
    assert_eq!(
        after.body, first.body,
        "a repeat after it gets the stored answer"
    );
    assert_eq!(host.entry.entered().len(), 1, "typed once");
    // The same id under another task is its own.
    host.entry.script(Script::Enter);
    let other = fill(
        &host,
        tab,
        &canary_body(&json!({ "requestId": "pw-1", "taskId": "task-8" })),
    )
    .await?;
    assert_ne!(other.body["runId"], first.body["runId"]);
    // Without a requestId nothing stops a repeat.
    let plain = canary_body(&json!({}));
    fill(&host, tab, &plain).await?;
    fill(&host, tab, &plain).await?;
    assert_eq!(host.entry.entered().len(), 4);
    // The record holds the ids and the answers, never a value.
    let record = fs::read(host.dir.path().join(LEDGER_FILE))?;
    assert!(holds_any(&record, "pw-1"), "the record keeps the id");
    for canary in [PASSWORD, CARD, CODE] {
        assert!(!holds_any(&record, canary), "{canary:?} in the record");
    }
    Ok(())
}

/// Design section 10.1: a lock refuses the fill unless the request names it; a named lock that
/// is not held refuses the call; a lock taken during the fill stops it before its keys.
#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "the lock's cases in order on one tab: before, named, elsewhere, released, during"
)]
async fn a_lock_refuses_the_fill_unless_named_and_stops_it_when_taken_during() -> TestResult {
    let (host, tab) = ready().await?;
    let lock = host.take_lock(tab).await?;
    let lock_id = lock.body["id"].as_str().ok_or("a lock id")?.to_owned();
    let refused = fill(&host, tab, &canary_body(&json!({ "requestId": "locked" }))).await?;
    assert_eq!(refused.body["outcome"], "refused", "{refused:?}");
    assert_eq!(refused.body["delivery"], "not_sent");
    assert_eq!(refused.body["typed"], json!([0, 0, 0]));
    assert_eq!(refused.body["error"]["code"], "owner_has_control");
    assert_eq!(refused.body["error"]["owner"], "host");
    assert_eq!(
        host.entry.calls(),
        0,
        "the door refused before the entry ran"
    );
    // Naming the lock passes it.
    let passed = fill(&host, tab, &canary_body(&json!({ "lockId": lock_id }))).await?;
    assert_eq!(passed.body["outcome"], "entered", "{passed:?}");
    assert!(
        host.takeover.is_locked(tab),
        "the fill did not end the lock"
    );
    // A lock on another tab does not pass this one.
    let other = host.fake.open_tab("https://other.example/", "Other");
    let other_lock = host.take_lock(other).await?;
    let wrong = fill(
        &host,
        tab,
        &canary_body(&json!({ "lockId": other_lock.body["id"], "requestId": "wrong-lock" })),
    )
    .await?;
    assert_eq!(wrong.status, 409, "{wrong:?}");
    assert_eq!(wrong.body["code"], "lock_released");
    // A released lock refuses the call, and the request id stays unused.
    request(
        &host.socket,
        "DELETE",
        &format!("/v1/locks/{lock_id}"),
        Some(TOKEN),
        None,
    )
    .await?;
    let released = fill(
        &host,
        tab,
        &canary_body(&json!({ "lockId": lock_id, "requestId": "after-release" })),
    )
    .await?;
    assert_eq!(released.status, 409, "{released:?}");
    assert_eq!(released.body["code"], "lock_released");
    let retried = fill(
        &host,
        tab,
        &canary_body(&json!({ "requestId": "after-release" })),
    )
    .await?;
    assert_eq!(
        retried.body["outcome"], "entered",
        "a request error leaves the id free"
    );
    // A lock taken during the fill: the entry's next ask of the rules stops it.
    host.entry.script(Script::Hold);
    let entered = host.entry.entered().len();
    let during = tokio::spawn({
        let socket = host.socket.clone();
        let body = canary_body(&json!({}));
        async move {
            request(
                &socket,
                "POST",
                &format!("/v1/tabs/{}/fill", tab.0),
                Some(TOKEN),
                Some(&body),
            )
            .await
            .map_err(|error| error.to_string())
        }
    });
    within(host.entry.until_holding()).await?;
    let taking = tokio::spawn({
        let socket = host.socket.clone();
        async move {
            request(
                &socket,
                "POST",
                "/v1/locks",
                Some(TOKEN),
                Some(&json!({ "tabId": tab.0, "ttlSeconds": 60 })),
            )
            .await
            .map_err(|error| error.to_string())
        }
    });
    within(async {
        while !host.takeover.is_locked(tab) {
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    host.entry.release();
    let stopped = within(during).await???;
    assert_eq!(stopped.body["outcome"], "refused", "{stopped:?}");
    assert_eq!(stopped.body["error"]["code"], "owner_has_control");
    assert_eq!(
        stopped.body["delivery"], "done",
        "the focus step had reached the page"
    );
    assert_eq!(stopped.body["error"]["retryable"], false);
    assert_eq!(stopped.body["typed"], json!([0, 0, 0]));
    assert_eq!(host.entry.entered().len(), entered, "no key after the lock");
    let taken = within(taking).await???;
    assert_eq!(taken.status, 200, "{taken:?}");
    Ok(())
}

/// Design section 7: the host only. A web page's request is refused before anything else; no
/// token, a wrong one or one in the URL is refused; a script cannot name either method.
#[tokio::test]
async fn only_the_host_reaches_the_fill() -> TestResult {
    let (host, tab) = ready().await?;
    let body = canary_body(&json!({}));
    for header in [
        "Origin: http://127.0.0.1:9000",
        "Sec-Fetch-Site: same-origin",
    ] {
        let reply = raw_request(
            &host.socket,
            &format!("/v1/tabs/{}/fill", tab.0),
            header,
            &body,
        )
        .await?;
        assert!(reply.starts_with("HTTP/1.1 403"), "{header}: {reply}");
        assert!(reply.contains("forbidden"), "{reply}");
        assert!(!holds_any(reply.as_bytes(), PASSWORD), "{reply}");
    }
    let path = format!("/v1/tabs/{}/fill", tab.0);
    let anonymous = request(&host.socket, "POST", &path, None, Some(&body)).await?;
    assert_eq!(anonymous.status, 401, "{anonymous:?}");
    let wrong = request(
        &host.socket,
        "POST",
        &path,
        Some("not-the-token-0123456789-abcdefghij"),
        Some(&body),
    )
    .await?;
    assert_eq!(wrong.status, 401, "{wrong:?}");
    let in_url = request(
        &host.socket,
        "POST",
        &format!("{path}?token={TOKEN}"),
        Some(TOKEN),
        Some(&body),
    )
    .await?;
    assert_eq!(in_url.status, 401, "{in_url:?}");
    assert_eq!(
        host.entry.calls(),
        0,
        "no refused request reached the entry"
    );
    let runs = get(&host, "/v1/runs").await?;
    assert_eq!(
        runs.body["items"].as_array().map(Vec::len),
        Some(0),
        "no record of a refused request: {runs:?}"
    );

    // A script naming either method gets the answer an unknown name gets.
    let input = json!({ "page": { "$neo": "page", "tab": tab.0 }, "fields": [] });
    let unknown = script_call(&host, "x", "host.nothing", input.clone())
        .await?
        .1
        .err()
        .ok_or("refused")?;
    for name in [HOST_FILL, HOST_FILL_CHECK] {
        let refused = script_call(&host, "x", name, input.clone())
            .await?
            .1
            .err()
            .ok_or("refused")?;
        assert_eq!(
            refused.error.code(),
            ErrorCode::METHOD_UNAVAILABLE,
            "{name}"
        );
        assert_eq!(
            refused.error.message().replace(name, "NAME"),
            unknown.error.message().replace("host.nothing", "NAME"),
            "{name} is answered as a name that does not exist"
        );
        assert_eq!(refused.delivery, Delivery::NotSent);
    }
    assert!(host.entry.entered().is_empty());
    Ok(())
}

/// One request with an extra header line, as a browser page would send it; the raw reply.
async fn raw_request(
    socket: &Path,
    path: &str,
    header: &str,
    body: &Value,
) -> Result<String, Box<dyn Error>> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = UnixStream::connect(socket).await?;
    let body = body.to_string();
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {TOKEN}\r\n{header}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply).await?;
    Ok(reply)
}

/// Request errors leave nothing typed: a closed tab (404) and a bad body (400), whose message
/// names the field and never the value.
#[tokio::test]
async fn request_errors_type_nothing_and_quote_nothing() -> TestResult {
    let (host, tab) = ready().await?;
    let missing = fill(
        &host,
        TabId(tab.0 + 1000),
        &canary_body(&json!({ "requestId": "gone" })),
    )
    .await?;
    assert_eq!(missing.status, 404, "{missing:?}");
    assert_eq!(missing.body["code"], "tab_not_found");
    let control = fill(
        &host,
        tab,
        &json!({ "fields": [{ "ref": "e1", "value": format!("{PASSWORD}\n"),
        "expect": { "origin": "https://a.example" } }] }),
    )
    .await?;
    assert_eq!(control.status, 400, "{control:?}");
    assert_eq!(control.body["code"], "invalid_request");
    let message = control.body["message"].as_str().ok_or("a message")?;
    assert!(
        message.starts_with("body.fields[0].value: a private value holds no control character"),
        "{message}"
    );
    assert!(!holds_any(message.as_bytes(), PASSWORD), "{message}");
    assert!(host.entry.entered().is_empty());
    // The 404 left the id unused.
    let typed = fill(&host, tab, &canary_body(&json!({ "requestId": "gone" }))).await?;
    assert_eq!(typed.body["outcome"], "entered", "{typed:?}");
    Ok(())
}

/// Ruling R49 at the route: `expect.maxLength` takes -1 (a box with none), 0 and any limit; -2
/// is a malformed body, refused before anything is typed, on the fill and on the check call.
#[tokio::test]
async fn max_length_below_minus_one_is_a_malformed_body() -> TestResult {
    let (host, tab) = ready().await?;
    let body = |max_length: i64| {
        json!({ "fields": [{ "ref": "e1", "value": PASSWORD,
            "expect": { "origin": "https://a.example", "maxLength": max_length } }] })
    };
    let bad = fill(&host, tab, &body(-2)).await?;
    assert_eq!(bad.status, 400, "{bad:?}");
    assert_eq!(bad.body["code"], "invalid_request");
    let message = bad.body["message"].as_str().ok_or("a message")?;
    assert!(
        message.starts_with("body.fields[0].expect.maxLength:"),
        "{message}"
    );
    assert!(host.entry.entered().is_empty());
    let check = request(
        &host.socket,
        "POST",
        &format!("/v1/tabs/{}/fill/check", tab.0),
        Some(TOKEN),
        Some(&json!({ "fields": [{ "ref": "e1",
            "expect": { "origin": "https://a.example", "maxLength": -2 } }] })),
    )
    .await?;
    assert_eq!(check.status, 400, "{check:?}");
    let typed = fill(&host, tab, &body(-1)).await?;
    assert_eq!(typed.body["outcome"], "entered", "{typed:?}");
    Ok(())
}

/// Rulings R55 and R56 on the record: an entry whose boxes went under their fields' own drawings
/// notes each such field on its operation's result, with whether the site drew the value there;
/// a field with no drawing has no note. Fails with the notes left out of the result.
#[tokio::test]
async fn the_record_notes_fields_entered_under_their_drawing() -> TestResult {
    let (host, tab) = ready().await?;
    host.entry.script(Script::EnterUnderDrawings);
    let answer = fill(&host, tab, &canary_body(&json!({}))).await?;
    assert_eq!(answer.status, 200, "{answer:?}");
    assert_eq!(answer.body["outcome"], "entered");
    assert!(
        answer.body.get("drawings").is_none(),
        "the host's answer is unchanged: {answer:?}"
    );
    let run = answer.body["runId"].as_str().ok_or("a runId")?;
    let record = get(&host, &format!("/v1/runs/{run}?wait=10")).await?;
    assert_eq!(
        record.body["operations"][0]["result"]["drawings"],
        json!([{ "field": 0, "drawn": true }, { "field": 1, "drawn": false }]),
        "{record:?}"
    );
    Ok(())
}

/// Ruling R55 after the typing: an entry the site drew the value of answers `value_drawn`, or
/// `value_still_drawn` when the page still shows it, the website's, each with its fixed sentence,
/// and quotes no value. Fails without the route's codes (the answer reads `internal_error`).
#[tokio::test]
async fn a_drawn_value_answers_its_code() -> TestResult {
    let cases = [
        (
            EntryCode::ValueDrawn,
            "value_drawn",
            "The site drew the typed value outside the box. Neo emptied the box, and the page no \
             longer shows the value.",
        ),
        (
            EntryCode::ValueStillDrawn,
            "value_still_drawn",
            "The site drew the typed value outside the box, and the page still shows it. Reload \
             or close the page before anything reads it.",
        ),
    ];
    for (code, name, sentence) in cases {
        let (host, tab) = ready().await?;
        host.entry.script(Script::Uncertain { at: 0, code });
        let answer = fill(&host, tab, &canary_body(&json!({}))).await?;
        assert_eq!(answer.status, 200, "{answer:?}");
        assert_eq!(answer.body["outcome"], "uncertain", "{answer:?}");
        assert_eq!(answer.body["error"]["code"], name, "{answer:?}");
        assert_eq!(answer.body["error"]["owner"], "website");
        assert_eq!(answer.body["error"]["message"], sentence);
        assert!(!holds_any(answer.body.to_string().as_bytes(), PASSWORD));
    }
    Ok(())
}

/// The check (design section 4.8): each box its facts and its refusal, typed nothing, recorded
/// as one protected read with no picture; `expect` may be left out; a value is refused.
#[tokio::test]
async fn the_check_answers_facts_and_refusals_and_types_nothing() -> TestResult {
    let (host, tab) = ready().await?;
    host.entry.script(Script::Refuse {
        at: 1,
        code: EntryCode::BoxCovered(CoverIn::OwnDocument),
    });
    let body = json!({ "taskId": "task-42", "fields": [
        { "ref": "e5", "expect": { "origin": "https://pay.example" } },
        { "ref": "f2e4" }
    ] });
    let checked = request(
        &host.socket,
        "POST",
        &format!("/v1/tabs/{}/fill/check", tab.0),
        Some(TOKEN),
        Some(&body),
    )
    .await?;
    assert_eq!(checked.status, 200, "{checked:?}");
    let fields = checked.body["fields"].as_array().ok_or("fields")?;
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0]["facts"]["origin"], "https://pay.example");
    assert!(fields[0].get("refusal").is_none(), "{checked:?}");
    assert_eq!(
        fields[1]["facts"]["autocomplete"], "cc-number",
        "a covered box answers its facts"
    );
    assert_eq!(fields[1]["refusal"]["code"], "box_covered");
    assert_eq!(fields[1]["refusal"]["retryable"], true);
    assert!(host.entry.entered().is_empty());
    assert_eq!(host.entry.looks.load(Ordering::SeqCst), 1);
    let run = checked.body["runId"].as_str().ok_or("a runId")?;
    let record = get(&host, &format!("/v1/runs/{run}?wait=10")).await?;
    let operation = &record.body["operations"][0];
    assert_eq!(operation["name"], HOST_FILL_CHECK);
    assert_eq!(operation["kind"], "protected");
    assert_eq!(operation["effect"], "read");
    assert_eq!(operation["hasScreenshot"], false);
    let with_value = request(
        &host.socket,
        "POST",
        &format!("/v1/tabs/{}/fill/check", tab.0),
        Some(TOKEN),
        Some(&json!({ "fields": [{ "ref": "e5", "value": PASSWORD }] })),
    )
    .await?;
    assert_eq!(with_value.status, 400, "{with_value:?}");
    assert!(!holds_any(with_value.body.to_string().as_bytes(), PASSWORD));
    Ok(())
}

/// A fill whose entry never answers: the run's backstop stops it, nothing more is typed, and the
/// answer says it cannot tell (`unknown`).
#[tokio::test]
async fn an_entry_that_never_answers_is_stopped_by_the_backstop() -> TestResult {
    let (host, tab) = ready().await?;
    host.entry.script(Script::Hang);
    let started = StdInstant::now();
    let hung = fill(&host, tab, &canary_body(&json!({ "timeoutMs": 1000 }))).await?;
    // The backstop stops the entry at the fill's 1 s plus `ENTRY_BACKSTOP`, in real time through
    // a real socket and store, so a loaded machine runs long: the bound only catches an entry
    // the backstop never stopped.
    let stopped_by = Duration::from_secs(1) + ENTRY_BACKSTOP + Duration::from_secs(20);
    assert!(started.elapsed() < stopped_by, "{:?}", started.elapsed());
    assert_eq!(hung.status, 200, "{hung:?}");
    assert_eq!(hung.body["error"]["code"], "run_timeout");
    assert_eq!(hung.body["error"]["owner"], "neo");
    // The stand-in hangs before it marks the op sent, so Neo knows nothing went out.
    assert_eq!(hung.body["outcome"], "refused");
    assert_eq!(hung.body["delivery"], "not_sent");
    let run = hung.body["runId"].as_str().ok_or("a runId")?;
    let record = get(&host, &format!("/v1/runs/{run}?wait=10")).await?;
    assert_eq!(record.body["status"], "timeout");
    Ok(())
}

/// A fill that names its host's lock stops when that lock is released during it: the agent is no
/// longer locked out as the host believed (design section 10.1).
#[tokio::test]
async fn a_named_lock_released_during_the_fill_stops_it() -> TestResult {
    let (host, tab) = ready().await?;
    let lock = host.take_lock(tab).await?;
    let lock_id = lock.body["id"].as_str().ok_or("a lock id")?.to_owned();
    host.entry.script(Script::Hold);
    let during = tokio::spawn({
        let socket = host.socket.clone();
        let body = canary_body(&json!({ "lockId": lock_id }));
        async move {
            request(
                &socket,
                "POST",
                &format!("/v1/tabs/{}/fill", tab.0),
                Some(TOKEN),
                Some(&body),
            )
            .await
            .map_err(|error| error.to_string())
        }
    });
    within(host.entry.until_holding()).await?;
    let released = request(
        &host.socket,
        "DELETE",
        &format!("/v1/locks/{lock_id}"),
        Some(TOKEN),
        None,
    )
    .await?;
    assert_eq!(released.status, 200, "{released:?}");
    host.entry.release();
    let stopped = within(during).await???;
    assert_eq!(stopped.body["outcome"], "refused", "{stopped:?}");
    assert_eq!(stopped.body["error"]["code"], "lock_released");
    assert_eq!(stopped.body["error"]["owner"], "host");
    assert_eq!(stopped.body["delivery"], "done");
    assert_eq!(stopped.body["typed"], json!([0, 0, 0]));
    assert!(host.entry.entered().is_empty(), "no key after the release");
    Ok(())
}

/// Ruling R25: a fill passes a named lock only when the host took it. A takeover's lock is the
/// owner's: named or not, it refuses the fill with `owner_has_control`, and nothing is typed.
#[tokio::test]
async fn a_takeovers_lock_refuses_the_fill_even_when_named() -> TestResult {
    use neo_takeover::{LockBinding, LockTtl, Reads, TakeoverId};
    let (host, tab) = ready().await?;
    let locks = host
        .takeover
        .locks()
        .ok_or("the host API armed the locks")?;
    let ttl = LockTtl::new(Duration::from_mins(1)).ok_or("a minute is a valid TTL")?;
    let owners = locks
        .lock(
            tab,
            Reads::Block,
            ttl,
            LockBinding::Takeover(TakeoverId::new()),
        )
        .await;
    for body in [
        canary_body(&json!({ "lockId": owners.id.to_string() })),
        canary_body(&json!({})),
    ] {
        let refused = fill(&host, tab, &body).await?;
        assert_eq!(refused.status, 200, "{refused:?}");
        assert_eq!(refused.body["outcome"], "refused");
        assert_eq!(refused.body["error"]["code"], "owner_has_control");
        assert_eq!(refused.body["delivery"], "not_sent");
    }
    assert_eq!(
        host.entry.calls(),
        0,
        "the door refused before the entry ran"
    );
    Ok(())
}

/// Ruling R27: the idle-tab janitor counts only a script's ops as use, so the fill names its tab
/// in use for its run. A tab idle past the limit stays out of the janitor's list while a fill
/// types into it, and its idle clock restarts when the fill's run ends.
#[tokio::test]
async fn a_fill_keeps_its_tab_in_use_and_restarts_its_idle_clock() -> TestResult {
    use crate::browser::PageRef;
    use std::collections::HashMap;
    let (host, tab) = ready().await?;
    let page = PageRef { tab };
    host.tabs.opened_by_agent(&page, host.tabs.seen());
    // The tab's idle clock starts here; the fill comes well after, so a clock the fill did not
    // restart would still count from here.
    let opened = Instant::now();
    time::sleep(Duration::from_millis(1500)).await;
    let idle = Duration::from_mins(10);
    let idle_at = |at: Instant| host.tabs.idle_agent_tabs(idle, at, &HashMap::new());
    assert_eq!(
        idle_at(Instant::now() + Duration::from_mins(11)),
        vec![page.clone()],
        "the tab is a janitor's candidate once idle"
    );
    host.entry.script(Script::Hold);
    let during = tokio::spawn({
        let socket = host.socket.clone();
        let body = canary_body(&json!({}));
        async move {
            request(
                &socket,
                "POST",
                &format!("/v1/tabs/{}/fill", tab.0),
                Some(TOKEN),
                Some(&body),
            )
            .await
            .map_err(|error| error.to_string())
        }
    });
    within(host.entry.until_holding()).await?;
    assert!(
        idle_at(Instant::now() + Duration::from_hours(1)).is_empty(),
        "in use while the fill types"
    );
    host.entry.release();
    let answer = within(during).await???;
    assert_eq!(answer.body["outcome"], "entered", "{answer:?}");
    let run = answer.body["runId"].as_str().ok_or("a runId")?;
    get(&host, &format!("/v1/runs/{run}?wait=10")).await?;
    // A full period after the tab opened, but less than one after the fill's run ended: idle only
    // if the fill did not restart the clock.
    within(async {
        while !idle_at(opened + idle + Duration::from_millis(750)).is_empty() {
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_still_idle| "the fill's run restarts the tab's idle clock")?;
    assert_eq!(
        idle_at(Instant::now() + Duration::from_mins(11)),
        vec![page],
        "idle again a full period after the fill"
    );
    Ok(())
}

/// Ruling R34: the check takes `lockId` as the fill does. A host whose own lock blocks reads
/// checks its tab by naming that lock; without it every box answers `owner_has_control`; a
/// takeover's lock refuses, named or not; a named lock that holds nothing is a 409.
#[tokio::test]
async fn the_check_passes_its_hosts_own_lock_as_the_fill_does() -> TestResult {
    use neo_takeover::{LockBinding, LockTtl, Reads, TakeoverId};
    let (host, tab) = ready().await?;
    let check = |body: Value| {
        let socket = host.socket.clone();
        let path = format!("/v1/tabs/{}/fill/check", tab.0);
        async move { request(&socket, "POST", &path, Some(TOKEN), Some(&body)).await }
    };
    let lock = host.take_lock(tab).await?;
    let lock_id = lock.body["id"].as_str().ok_or("a lock id")?.to_owned();
    let boxes = json!([{ "ref": "e5" }, { "ref": "f2e4" }]);
    let blocked = check(json!({ "fields": boxes })).await?;
    assert_eq!(blocked.status, 200, "{blocked:?}");
    assert_eq!(
        blocked.body["fields"][0]["refusal"]["code"],
        "owner_has_control"
    );
    assert!(
        blocked.body["fields"][0].get("facts").is_none(),
        "{blocked:?}"
    );
    let passed = check(json!({ "lockId": lock_id, "fields": boxes })).await?;
    assert_eq!(
        passed.body["fields"][0]["facts"]["origin"], "https://pay.example",
        "{passed:?}"
    );
    request(
        &host.socket,
        "DELETE",
        &format!("/v1/locks/{lock_id}"),
        Some(TOKEN),
        None,
    )
    .await?;
    let released = check(json!({ "lockId": lock_id, "fields": boxes })).await?;
    assert_eq!(released.status, 409, "{released:?}");
    assert_eq!(released.body["code"], "lock_released");
    let locks = host
        .takeover
        .locks()
        .ok_or("the host API armed the locks")?;
    let ttl = LockTtl::new(Duration::from_mins(1)).ok_or("a minute is a valid TTL")?;
    let owners = locks
        .lock(
            tab,
            Reads::Block,
            ttl,
            LockBinding::Takeover(TakeoverId::new()),
        )
        .await;
    let takeover = check(json!({ "lockId": owners.id.to_string(), "fields": boxes })).await?;
    assert_eq!(takeover.status, 200, "{takeover:?}");
    assert_eq!(
        takeover.body["fields"][1]["refusal"]["code"],
        "owner_has_control"
    );
    assert_eq!(host.entry.calls(), 0, "a check never types");
    Ok(())
}

/// Design section 10.3 (rulings R15 and R28): once a fill typed into a tab, the camera takes no
/// picture of it for the tab's life and records `withheld` in the picture's place. A write on
/// another tab still gets a job (the fake browser cannot capture, so its job answers `Dropped`),
/// which the withheld answer is told apart from.
#[tokio::test]
async fn a_tab_that_took_a_fill_is_never_pictured_again() -> TestResult {
    let (host, tab) = ready().await?;
    set_pictures(&host, "never").await?;
    let other = host.fake.open_tab("https://other.example/", "Other");
    let answer = fill(&host, tab, &canary_body(&json!({}))).await?;
    assert_eq!(answer.body["outcome"], "entered", "{answer:?}");
    let mut operations = Vec::new();
    for on in [tab, other] {
        let (run, written) = script_call(
            &host,
            "await page.click('#pay')",
            "test.plainFill",
            json!({ "page": { "$neo": "page", "tab": on.0 }, "value": "not secret" }),
        )
        .await?;
        assert!(written.is_ok(), "{written:?}");
        operations.push(format!("{run}.1"));
    }
    host.sink.flush().await?;
    let screenshot = |operation: &str| {
        let path = format!("/v1/operations/{operation}/screenshot");
        let socket = host.socket.clone();
        async move { request(&socket, "GET", &path, Some(TOKEN), None).await }
    };
    within(async {
        loop {
            let withheld = screenshot(&operations[0]).await;
            if withheld.is_ok_and(|reply| {
                reply.body["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("withheld"))
            }) {
                return;
            }
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|_unseen| "the filled tab's later write records a withheld picture")?;
    let dropped = screenshot(&operations[1]).await?;
    assert_eq!(dropped.status, 404, "{dropped:?}");
    assert!(
        dropped.body["message"]
            .as_str()
            .is_some_and(|message| !message.contains("withheld")),
        "another tab is not withheld: {dropped:?}"
    );
    Ok(())
}

/// Rulings R45 and R58: a fill made under the host's `fill.pictures: never` writes its tab's
/// target into the record beside the trace before its first character, so a Neo restart while
/// the browser keeps the tab still takes no picture of it, and a record that cannot be written
/// refuses the fill, nothing typed. Under the default, `allowed`, a fill writes no record and is
/// not refused when the record cannot be written. Fails with the record written whatever the
/// setting (the default writes a line, or is refused).
#[tokio::test]
async fn a_fill_records_its_tab_first_only_under_never() -> TestResult {
    let (host, tab) = ready().await?;
    let answer = fill(&host, tab, &canary_body(&json!({}))).await?;
    assert_eq!(answer.body["outcome"], "entered", "{answer:?}");
    let written = fs::read_to_string(host.dir.path().join(FILLED_TABS_FILE)).unwrap_or_default();
    assert!(
        written.trim().is_empty(),
        "the default writes no record: {written:?}"
    );
    let (unwritable, unwritable_tab) = ready().await?;
    fs::create_dir(unwritable.dir.path().join(FILLED_TABS_FILE))?;
    let entered = fill(&unwritable, unwritable_tab, &canary_body(&json!({}))).await?;
    assert_eq!(
        entered.body["outcome"], "entered",
        "the default needs no record: {entered:?}"
    );

    let (host, tab) = ready().await?;
    set_pictures(&host, "never").await?;
    let answer = fill(&host, tab, &canary_body(&json!({}))).await?;
    assert_eq!(answer.body["outcome"], "entered", "{answer:?}");
    let target = host
        .link
        .page(tab)
        .await
        .map(|page| page.target_id.to_string())
        .ok_or("the tab's target")?;
    let record = fs::read_to_string(host.dir.path().join(FILLED_TABS_FILE))?;
    assert_eq!(
        record,
        format!("\n{target}\n"),
        "the tab's target, never a value"
    );

    let (other, other_tab) = ready().await?;
    set_pictures(&other, "never").await?;
    fs::create_dir(other.dir.path().join(FILLED_TABS_FILE))?;
    let refused = fill(&other, other_tab, &canary_body(&json!({}))).await?;
    assert_eq!(refused.status, 503, "{refused:?}");
    assert_eq!(refused.body["code"], "trace_unavailable", "{refused:?}");
    assert_eq!(
        refused.body["message"],
        "Neo cannot write its record of the tabs that took a fill, so it types nothing",
        "its own sentence, not the request record's: this fill names no requestId"
    );
    assert_eq!(other.entry.calls(), 0, "nothing typed");
    Ok(())
}

/// Sets the host's `fill.pictures` (ruling R58) through the settings route.
async fn set_pictures(host: &Host, value: &str) -> TestResult {
    let reply = request(
        &host.socket,
        "PATCH",
        "/v1/settings",
        Some(TOKEN),
        Some(&json!({ "fill": { "pictures": value } })),
    )
    .await?;
    assert_eq!(reply.status, 200, "{reply:?}");
    Ok(())
}

/// Ruling R45: with no host fill at all, a tab whose page shows a private box's value painted in
/// clear (a script's own fill of a card box) gets no stored picture while it does; another tab
/// is not withheld. The app's reading is the scripted entry's; the camera asks it around each
/// capture.
#[tokio::test]
async fn a_tab_that_shows_a_private_value_is_not_pictured_while_it_does() -> TestResult {
    let (host, tab) = ready().await?;
    let other = host.fake.open_tab("https://other.example/", "Other");
    host.entry.show(tab);
    let mut operations = Vec::new();
    for on in [tab, other] {
        let (run, written) = script_call(
            &host,
            "await page.click('#pay')",
            "test.plainFill",
            json!({ "page": { "$neo": "page", "tab": on.0 }, "value": "not secret" }),
        )
        .await?;
        assert!(written.is_ok(), "{written:?}");
        operations.push(format!("{run}.1"));
    }
    host.sink.flush().await?;
    let message = |operation: &str| {
        let path = format!("/v1/operations/{operation}/screenshot");
        let socket = host.socket.clone();
        async move {
            request(&socket, "GET", &path, Some(TOKEN), None)
                .await
                .map(|reply| {
                    reply.body["message"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned()
                })
        }
    };
    within(async {
        loop {
            if message(&operations[0])
                .await
                .is_ok_and(|text| text.contains("withheld"))
            {
                return;
            }
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|_unseen| "the tab that shows a private value records a withheld picture")?;
    // The other tab's picture settled before it is read: its run is complete only once every
    // picture it ordered answered. The fake browser cannot capture, so it is dropped, not
    // withheld; the real-browser test shows such a tab pictured.
    let other_run = operations[1].rsplit_once('.').map_or("", |(run, _)| run);
    let settled = request(
        &host.socket,
        "GET",
        &format!("/v1/runs/{other_run}?wait=10"),
        Some(TOKEN),
        None,
    )
    .await?;
    assert_eq!(settled.body["complete"], true, "{settled:?}");
    assert!(
        !message(&operations[1]).await?.contains("withheld"),
        "another tab is not withheld"
    );
    Ok(())
}
