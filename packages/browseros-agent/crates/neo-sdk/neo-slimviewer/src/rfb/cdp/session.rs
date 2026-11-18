//! Persistent tab observation. Only this session's actor reads the browser socket.
use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use tungstenite::Message;

use super::agent::{Fields, PasteError, Press};
use super::paste::{Attempt, AttemptResult, PageOps, Path, Reply, Target};
use super::protocol::{Evidence, Prepared, Report};
use super::{Client, DeliveryError, FocusEvidence, FocusedPage, fields, geometry, protocol};

pub(super) const WORLD: &str = "slimviewer-input";
pub(super) const BINDING: &str = "slimviewerAgent";
pub(super) const SCRIPT: &str = include_str!("agent.js");
const MAX_FRAMES: usize = 256;
const CALL_WAIT: Duration = Duration::from_millis(200);
const TAP_WAIT: Duration = Duration::from_millis(200);
const SELECTION_DEBOUNCE: Duration = Duration::from_millis(50);
const METADATA_RETRY: Duration = Duration::from_millis(250);
const MAX_METADATA_RETRY: Duration = Duration::from_secs(5);
const DRAIN_BUDGET: Duration = Duration::from_millis(5);
const DRAIN_MESSAGES: usize = 64;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(super) struct SessionId(pub(super) String);

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(super) struct FrameKey {
    pub(super) session: SessionId,
    pub(super) frame: String,
}

pub(super) struct Session {
    client: Client,
    page: Option<Page>,
    candidates: HashMap<String, FocusedPage>,
    install: VecDeque<SessionId>,
    pending_scripts: HashMap<u64, SessionId>,
    selected_dirty: bool,
    selection: Option<Selection>,
    selection_at: Instant,
    selection_retry: MetadataRetry,
    selection_epoch: u64,
    known_pages: HashSet<String>,
    install_at: Instant,
    install_retry: MetadataRetry,
    buffered: bool,
    fields_dirty: bool,
    fields_immediate: bool,
    healthy: bool,
    method_unavailable: bool,
    input_serial: u64,
    generation: u64,
    transaction: u32,
    transaction_context: Option<(FrameKey, String)>,
}

struct MetadataRetry {
    delay: Duration,
}

impl Default for MetadataRetry {
    fn default() -> Self {
        Self {
            delay: METADATA_RETRY,
        }
    }
}

impl MetadataRetry {
    fn failed(&mut self) -> Instant {
        let at = Instant::now() + self.delay;
        self.delay = (self.delay * 2).min(MAX_METADATA_RETRY);
        at
    }

    fn reset(&mut self) {
        self.delay = METADATA_RETRY;
    }
}

struct Page {
    target: String,
    session: SessionId,
    root: String,
    oopifs: HashMap<String, SessionId>,
    frames: HashMap<FrameKey, Frame>,
    scripts: HashMap<SessionId, String>,
    installation: HashMap<SessionId, InstallStage>,
    window: Option<geometry::Window>,
    complete: bool,
}

struct Selection {
    pending: VecDeque<String>,
    visible: Vec<(String, bool)>,
    epoch: u64,
    chosen: Option<FocusedPage>,
    focus_checked: bool,
    native: NativeFront,
    recheck: NativeFront,
}

#[derive(Default)]
enum NativeFront {
    #[default]
    Window,
    Tab(i64),
    Done(Option<String>),
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum InstallStage {
    #[default]
    Runtime,
    Page,
    Binding,
    Script,
    AutoAttach,
    FrameTree,
    Frames,
    Complete,
}

#[derive(Default)]
struct Frame {
    context: Option<u64>,
    unique_context: Option<String>,
    parent: Option<FrameKey>,
    tap: Option<protocol::Tap>,
    focus: Option<protocol::Focus>,
    boxes: Option<protocol::Boxes>,
    observed: bool,
    last_input_ms: u64,
}

impl Frame {
    fn document(&self) -> Option<(&str, u64)> {
        Some((self.unique_context.as_deref()?, self.boxes.as_ref()?.doc))
    }
}

impl Page {
    fn reset_document(&mut self, key: &FrameKey) {
        let children: Vec<_> = self
            .frames
            .iter()
            .filter(|(_, frame)| frame.parent.as_ref() == Some(key))
            .map(|(key, _)| key.frame.clone())
            .collect();
        for child in children {
            self.invalidate_subtree(&child, true);
        }
        if let Some(frame) = self.frames.get_mut(key) {
            *frame = Frame {
                parent: frame.parent.clone(),
                ..Frame::default()
            };
        }
    }

    fn claim_frame(&mut self, key: FrameKey) -> &mut Frame {
        let previous: Vec<_> = self
            .frames
            .keys()
            .filter(|old| old.frame == key.frame && **old != key)
            .cloned()
            .collect();
        for old in previous {
            let previous = self.frames.remove(&old).unwrap();
            let frame = self.frames.entry(key.clone()).or_default();
            if frame.parent.is_none() {
                frame.parent = previous.parent;
            }
            for child in self.frames.values_mut() {
                if child.parent.as_ref() == Some(&old) {
                    child.parent = Some(key.clone());
                }
            }
        }
        self.frames.entry(key).or_default()
    }

    // A detached renderer can leave its iframe on screen. Keep unknown placeholders
    // until a frame-removal event proves that this subtree no longer exists.
    fn invalidate_subtree(&mut self, id: &str, remove: bool) {
        let mut ids = HashSet::from([id.to_owned()]);
        loop {
            let before = ids.len();
            for (key, frame) in &self.frames {
                if frame
                    .parent
                    .as_ref()
                    .is_some_and(|p| ids.contains(&p.frame))
                {
                    ids.insert(key.frame.clone());
                }
            }
            if before == ids.len() {
                break;
            }
        }
        let sessions: HashSet<_> = self
            .oopifs
            .iter()
            .filter_map(|(id, session)| ids.contains(id).then_some(session.clone()))
            .collect();
        self.oopifs.retain(|id, _| !ids.contains(id));
        self.scripts
            .retain(|session, _| !sessions.contains(session));
        self.installation
            .retain(|session, _| !sessions.contains(session));
        if remove {
            self.frames.retain(|key, _| !ids.contains(&key.frame));
        } else {
            for (key, frame) in &mut self.frames {
                if ids.contains(&key.frame) {
                    *frame = Frame {
                        parent: frame.parent.clone(),
                        ..Frame::default()
                    };
                }
            }
        }
    }
}

#[derive(Debug)]
pub(super) enum CallError {
    NotSent(&'static str),
    Uncertain,
}

pub(super) struct Changes {
    pub fields: bool,
    pub immediate: bool,
}

impl Session {
    pub(super) fn open(deadline: Instant) -> Result<Self, &'static str> {
        Self::open_at(super::CDP_PORT, deadline)
    }

    fn open_at(port: u16, deadline: Instant) -> Result<Self, &'static str> {
        let mut session = Self {
            client: Client::connect(port, deadline)?,
            page: None,
            candidates: HashMap::new(),
            install: VecDeque::new(),
            pending_scripts: HashMap::new(),
            selected_dirty: true,
            selection: None,
            selection_at: Instant::now(),
            selection_retry: MetadataRetry::default(),
            selection_epoch: 0,
            known_pages: HashSet::new(),
            install_at: Instant::now(),
            install_retry: MetadataRetry::default(),
            buffered: false,
            fields_dirty: true,
            fields_immediate: false,
            healthy: true,
            method_unavailable: false,
            input_serial: 0,
            generation: 0,
            transaction: 0,
            transaction_context: None,
        };
        session.command("Target.setDiscoverTargets", json!({"discover":true}), None)?;
        if let Err(error) = session.ensure_selected(deadline)
            && !session.healthy
        {
            return Err(error);
        }
        Ok(session)
    }

    pub(super) fn set_deadline(&mut self, deadline: Instant) {
        self.client.deadline = deadline;
        self.client.socket.get_mut().deadline = deadline;
    }

    pub(super) fn healthy(&self) -> bool {
        self.healthy
    }

    pub(super) fn next_wake(&self) -> Option<Instant> {
        if self.buffered || self.fields_dirty {
            return Some(Instant::now());
        }
        self.selected_dirty
            .then_some(self.selection_at)
            .into_iter()
            .chain((!self.install.is_empty()).then_some(self.install_at))
            .min()
    }

    pub(super) fn raw_fd(&self) -> RawFd {
        self.client.socket.get_ref().inner.as_raw_fd()
    }

    fn command(
        &mut self,
        method: &str,
        params: Value,
        session: Option<&SessionId>,
    ) -> Result<Value, &'static str> {
        let result = self.request(method, params, session, CALL_WAIT);
        result.map_err(|e| match e {
            _ if self.method_unavailable => "CDP method unavailable",
            CallError::NotSent(reason) => reason,
            CallError::Uncertain => "CDP reply unavailable",
        })
    }

    // Once send starts, even a transport error may follow a page mutation.
    fn request(
        &mut self,
        method: &str,
        params: Value,
        session: Option<&SessionId>,
        wait: Duration,
    ) -> Result<Value, CallError> {
        self.method_unavailable = false;
        let until = self.client.deadline.min(Instant::now() + wait);
        super::remaining(until).map_err(CallError::NotSent)?;
        if !self.healthy {
            return Err(CallError::NotSent("CDP disconnected"));
        }
        self.client.socket.get_mut().deadline = until;
        self.client.next_id += 1;
        let id = self.client.next_id;
        let mut request = json!({"id":id,"method":method,"params":params});
        if let Some(session) = session {
            request["sessionId"] = json!(session.0);
            if method == "Page.addScriptToEvaluateOnNewDocument" {
                self.pending_scripts.insert(id, session.clone());
            }
        }
        self.buffered = true;
        if let Err(error) = self
            .client
            .socket
            .send(Message::Text(request.to_string().into()))
        {
            if !matches!(error, tungstenite::Error::Io(ref e) if timed_out(e)) {
                self.healthy = false;
            }
            return Err(CallError::Uncertain);
        }
        loop {
            if Instant::now() >= until {
                return Err(CallError::Uncertain);
            }
            let message = match self.client.socket.read() {
                Ok(message) => message,
                Err(tungstenite::Error::Io(e)) if timed_out(&e) => {
                    return Err(CallError::Uncertain);
                }
                Err(_) => {
                    self.healthy = false;
                    return Err(CallError::Uncertain);
                }
            };
            match message {
                Message::Text(text) => {
                    let Ok(value) = serde_json::from_str::<Value>(&text) else {
                        self.healthy = false;
                        return Err(CallError::Uncertain);
                    };
                    self.event(&value);
                    if value["id"].as_u64() == Some(id) {
                        if value.get("error").is_some() {
                            self.method_unavailable = value["error"]["code"] == -32601;
                            // A rejected callFunctionOn can already have run JavaScript.
                            return Err(CallError::Uncertain);
                        }
                        return value.get("result").cloned().ok_or(CallError::Uncertain);
                    }
                }
                Message::Ping(_) | Message::Pong(_) => {}
                _ => {
                    self.healthy = false;
                    return Err(CallError::Uncertain);
                }
            }
        }
    }

    fn probe(&mut self, page: &FocusedPage) -> Result<(bool, bool), &'static str> {
        let result = self.command("Runtime.evaluate", json!({"expression":super::FOCUS,"contextId":page.context,"returnByValue":true,"timeout":100}), Some(&SessionId(page.session.clone())))?;
        if result.get("exceptionDetails").is_some() {
            return Err("focus probe failed");
        }
        let value = &result["result"]["value"];
        Ok((
            value["focused"].as_bool().ok_or("invalid focus")?,
            value["visible"].as_bool().ok_or("invalid visibility")?,
        ))
    }

    fn native_front(&mut self, state: &mut NativeFront) -> Result<Option<String>, &'static str> {
        loop {
            match state {
                NativeFront::Window => {
                    // Ordinary Chromium falls back to document/visibility evidence.
                    let windows = match self.command("Browser.getWindows", json!({}), None) {
                        Ok(windows) => windows,
                        Err("CDP method unavailable") => {
                            *state = NativeFront::Done(None);
                            continue;
                        }
                        Err(error) => return Err(error),
                    };
                    let mut visible = Vec::new();
                    for window in windows["windows"].as_array().ok_or("missing windows")? {
                        let active = window["isActive"].as_bool().ok_or("invalid window focus")?;
                        if window["isVisible"]
                            .as_bool()
                            .ok_or("invalid window visibility")?
                        {
                            visible.push((window, active));
                        }
                    }
                    let active: Vec<_> = visible.iter().filter(|(_, active)| *active).collect();
                    let window = match active.as_slice() {
                        [window] => window.0,
                        [] if visible.len() == 1 => visible[0].0,
                        _ => return Err("no unique front window"),
                    };
                    *state =
                        NativeFront::Tab(window["windowId"].as_i64().ok_or("missing window id")?);
                }
                NativeFront::Tab(id) => {
                    let result =
                        match self.command("Browser.getActiveTab", json!({"windowId":id}), None) {
                            Ok(result) => result,
                            Err("CDP method unavailable") => {
                                *state = NativeFront::Done(None);
                                continue;
                            }
                            Err(error) => return Err(error),
                        };
                    let tab = &result["tab"];
                    if tab["windowId"] != *id || tab["isActive"] != true || tab["isHidden"] != false
                    {
                        return Err("no visible front tab");
                    }
                    *state = NativeFront::Done(Some(
                        tab["targetId"]
                            .as_str()
                            .ok_or("missing front target")?
                            .to_owned(),
                    ));
                }
                NativeFront::Done(target) => return Ok(target.clone()),
            }
        }
    }

    pub(super) fn ensure_selected(&mut self, deadline: Instant) -> Result<(), &'static str> {
        // A paste always checks every tab again; background scans may resume between pumps.
        self.selection = None;
        self.refresh_selected(deadline)
    }

    fn refresh_selected(&mut self, deadline: Instant) -> Result<(), &'static str> {
        self.set_deadline(deadline);
        let epoch = self.selection_epoch;
        if let Err(error) = self.select_page() {
            self.selected_dirty = true;
            self.selection_at = self.selection_retry.failed();
            return Err(error);
        }
        self.selection_retry.reset();
        self.selected_dirty = self.selection_epoch != epoch;
        self.fields_dirty = true;
        self.install_pending()
    }

    fn select_page(&mut self) -> Result<(), &'static str> {
        let resumed = self
            .selection
            .take()
            .filter(|scan| scan.epoch == self.selection_epoch);
        let mut scan = if let Some(scan) = resumed {
            scan
        } else {
            let epoch = self.selection_epoch;
            let result = self.command("Target.getTargets", json!({}), None)?;
            let targets = result["targetInfos"].as_array().ok_or("missing targets")?;
            let pending: VecDeque<_> = targets
                .iter()
                .filter(|t| t["type"] == "page" && t.get("subtype").is_none())
                .map(|t| {
                    t["targetId"]
                        .as_str()
                        .map(str::to_owned)
                        .ok_or("missing target")
                })
                .collect::<Result<_, _>>()?;
            if pending.len() > super::MAX_PAGES {
                return Err("too many pages");
            }
            self.known_pages = pending.iter().cloned().collect();
            self.candidates
                .retain(|id, _| self.known_pages.contains(id));
            Selection {
                pending,
                visible: Vec::new(),
                epoch,
                chosen: None,
                focus_checked: false,
                native: NativeFront::default(),
                recheck: NativeFront::default(),
            }
        };
        let result = self.scan_page(&mut scan);
        if result.is_err() && self.healthy && Instant::now() >= self.client.deadline {
            self.selection = Some(scan);
        }
        result
    }

    fn scan_page(&mut self, scan: &mut Selection) -> Result<(), &'static str> {
        while let Some(target) = scan.pending.front().cloned() {
            let mut page = if let Some(cached) = self.candidates.get(&target) {
                cached.clone()
            } else {
                let attached = self.command(
                    "Target.attachToTarget",
                    json!({"targetId":target,"flatten":true}),
                    None,
                )?;
                let page = FocusedPage {
                    target: target.to_owned(),
                    session: attached["sessionId"]
                        .as_str()
                        .ok_or("missing session")?
                        .to_owned(),
                    context: 0,
                    evidence: FocusEvidence::Document,
                };
                self.candidates.insert(target.to_owned(), page.clone());
                page
            };
            let focus = match (page.context != 0).then(|| self.probe(&page)).transpose() {
                Ok(Some(focus)) => focus,
                Err(error) if Instant::now() >= self.client.deadline => return Err(error),
                _ => {
                    let sid = SessionId(page.session.clone());
                    let tree = self.command("Page.getFrameTree", json!({}), Some(&sid))?;
                    let frame = tree["frameTree"]["frame"]["id"]
                        .as_str()
                        .ok_or("missing frame")?;
                    let world = self.command(
                        "Page.createIsolatedWorld",
                        json!({"frameId":frame,"worldName":WORLD}),
                        Some(&sid),
                    )?;
                    page.context = world["executionContextId"]
                        .as_u64()
                        .ok_or("missing context")?;
                    self.candidates.insert(target.to_owned(), page.clone());
                    self.probe(&page)?
                }
            };
            if focus.1 {
                scan.visible.push((target.clone(), focus.0));
            }
            self.candidates.insert(target, page);
            scan.pending.pop_front();
        }
        if scan.chosen.is_none() {
            let candidates = &scan.visible;
            let focused: Vec<_> = candidates.iter().filter(|(_, focused)| *focused).collect();
            let (target, evidence) = if let [chosen] = focused.as_slice() {
                (chosen.0.clone(), FocusEvidence::Document)
            } else if let Some(native) = self.native_front(&mut scan.native)? {
                if !candidates.iter().any(|(id, _)| *id == native) {
                    return Err("front page is not visible");
                }
                (native, FocusEvidence::Native)
            } else if candidates.len() == 1 {
                (candidates[0].0.clone(), FocusEvidence::Visible)
            } else {
                return Err("no unique visible page");
            };
            let mut selected = self
                .candidates
                .get(&target)
                .cloned()
                .ok_or("missing candidate")?;
            selected.evidence = evidence;
            scan.chosen = Some(selected);
        }
        let selected = scan.chosen.as_ref().unwrap();
        if !scan.focus_checked {
            let focus = self.probe(selected)?;
            if !focus.1 || (matches!(selected.evidence, FocusEvidence::Document) && !focus.0) {
                return Err("focus changed");
            }
            scan.focus_checked = true;
        }
        if matches!(selected.evidence, FocusEvidence::Native)
            && self.native_front(&mut scan.recheck)?.as_deref() != Some(&selected.target)
        {
            return Err("focus changed");
        }
        let target = selected.target.clone();
        let sid = SessionId(selected.session.clone());
        self.candidates.insert(target.clone(), selected.clone());
        if self.page.as_ref().is_none_or(|p| p.target != target) {
            self.stop_page();
            self.page = Some(Page {
                target,
                session: sid.clone(),
                root: String::new(),
                oopifs: HashMap::new(),
                frames: HashMap::new(),
                scripts: HashMap::new(),
                installation: HashMap::new(),
                window: None,
                complete: true,
            });
            self.install.push_back(sid);
            self.fields_dirty = true;
            self.generation += 1;
        }
        Ok(())
    }

    fn owns(&self, session: &SessionId) -> bool {
        self.page
            .as_ref()
            .is_some_and(|p| p.session == *session || p.oopifs.values().any(|s| s == session))
    }

    fn install_pending(&mut self) -> Result<(), &'static str> {
        while let Some(session) = self.install.pop_front() {
            if !self.owns(&session)
                || self
                    .page
                    .as_ref()
                    .is_some_and(|p| p.installation.get(&session) == Some(&InstallStage::Complete))
            {
                continue;
            }
            if let Err(error) = self.install_one(&session) {
                if self.owns(&session) {
                    self.install.push_front(session);
                }
                self.install_at = self.install_retry.failed();
                return Err(error);
            }
            self.page
                .as_mut()
                .ok_or("page detached")?
                .installation
                .insert(session, InstallStage::Complete);
            self.install_retry.reset();
        }
        self.install_retry.reset();
        Ok(())
    }

    fn install_one(&mut self, session: &SessionId) -> Result<(), &'static str> {
        loop {
            let stage = self
                .page
                .as_ref()
                .ok_or("page detached")?
                .installation
                .get(session)
                .copied()
                .unwrap_or_default();
            let next = match stage {
                InstallStage::Runtime => {
                    self.command("Runtime.enable", json!({}), Some(session))?;
                    InstallStage::Page
                }
                InstallStage::Page => {
                    self.command("Page.enable", json!({}), Some(session))?;
                    InstallStage::Binding
                }
                InstallStage::Binding => {
                    self.command(
                        "Runtime.addBinding",
                        json!({"name":BINDING,"executionContextName":WORLD}),
                        Some(session),
                    )?;
                    InstallStage::Script
                }
                InstallStage::Script => {
                    if self
                        .pending_scripts
                        .values()
                        .any(|pending| pending == session)
                    {
                        return Err("script registration pending");
                    }
                    let result = self.command(
                        "Page.addScriptToEvaluateOnNewDocument",
                        json!({"source":SCRIPT,"worldName":WORLD,"runImmediately":true}),
                        Some(session),
                    )?;
                    let script = result["identifier"]
                        .as_str()
                        .ok_or("missing script identifier")?
                        .to_owned();
                    self.page
                        .as_mut()
                        .ok_or("page detached")?
                        .scripts
                        .insert(session.clone(), script);
                    InstallStage::AutoAttach
                }
                InstallStage::AutoAttach => {
                    self.command("Target.setAutoAttach", json!({"autoAttach":true,"flatten":true,"waitForDebuggerOnStart":false,"filter":[{"type":"iframe"}]}), Some(session))?;
                    InstallStage::FrameTree
                }
                InstallStage::FrameTree => {
                    let tree = self.command("Page.getFrameTree", json!({}), Some(session))?;
                    self.frame_tree(session, &tree["frameTree"], None);
                    InstallStage::Frames
                }
                InstallStage::Frames | InstallStage::Complete => break,
            };
            if !self.owns(session) {
                return Err("page detached");
            }
            self.page
                .as_mut()
                .unwrap()
                .installation
                .insert(session.clone(), next);
        }
        let frames: Vec<_> = self
            .page
            .as_ref()
            .ok_or("page detached")?
            .frames
            .keys()
            .filter(|key| key.session == *session)
            .cloned()
            .collect();
        for key in frames {
            let frame = self.page.as_ref().and_then(|p| p.frames.get(&key));
            if frame.is_some_and(|f| f.observed) {
                continue;
            }
            let context = if let Some(context) = frame.and_then(|f| f.context) {
                context
            } else {
                let world = self.command(
                    "Page.createIsolatedWorld",
                    json!({"frameId":key.frame,"worldName":WORLD}),
                    Some(session),
                )?;
                let context = world["executionContextId"]
                    .as_u64()
                    .ok_or("missing context")?;
                if let Some(frame) = self.page.as_mut().and_then(|p| p.frames.get_mut(&key)) {
                    frame.context = Some(context);
                }
                context
            };
            // Older Chrome builds may accept runImmediately without creating existing worlds.
            self.command(
                    "Runtime.evaluate",
                    json!({"expression":format!("if (!globalThis.__sv) {{ {SCRIPT}\n }}"),"contextId":context,"returnByValue":true}),
                    Some(session),
                )?;
        }
        Ok(())
    }

    fn frame_tree(&mut self, session: &SessionId, tree: &Value, parent: Option<FrameKey>) {
        let Some(id) = tree["frame"]["id"].as_str() else {
            return;
        };
        let Some(page) = self.page.as_mut() else {
            return;
        };
        let owner = page.oopifs.get(id).unwrap_or(session).clone();
        let key = FrameKey {
            session: owner.clone(),
            frame: id.to_owned(),
        };
        if page.frames.len() >= MAX_FRAMES && !page.frames.contains_key(&key) {
            page.complete = false;
            return;
        }
        if parent.is_none() && *session == page.session {
            page.root = id.to_owned();
        }
        let frame = page.claim_frame(key.clone());
        if parent.is_some() {
            frame.parent = parent;
        }
        if let Some(children) = tree["childFrames"].as_array() {
            for child in children {
                self.frame_tree(&owner, child, Some(key.clone()));
            }
        }
    }

    fn event(&mut self, event: &Value) {
        if let Some(session) = event["id"]
            .as_u64()
            .and_then(|id| self.pending_scripts.remove(&id))
        {
            if self.owns(&session)
                && let Some(script) = event["result"]["identifier"].as_str()
            {
                let page = self.page.as_mut().unwrap();
                page.scripts.insert(session.clone(), script.to_owned());
                page.installation.insert(session, InstallStage::AutoAttach);
                self.install_at = Instant::now();
            }
            return;
        }

        let method = event["method"].as_str().unwrap_or("");
        let params = &event["params"];
        if matches!(
            method,
            "Target.targetCreated" | "Target.targetDestroyed" | "Target.targetInfoChanged"
        ) {
            let relevant = if method == "Target.targetDestroyed" {
                params["targetId"].as_str().is_some_and(|id| {
                    self.known_pages.remove(id) || self.candidates.contains_key(id)
                })
            } else {
                let info = &params["targetInfo"];
                if info["type"] == "page" && info.get("subtype").is_none() {
                    if let Some(id) = info["targetId"].as_str() {
                        self.known_pages.insert(id.to_owned());
                    }
                    true
                } else {
                    false
                }
            };
            if !relevant {
                return;
            }
            self.selection_epoch += 1;
            if !self.selected_dirty {
                self.selection_at = Instant::now() + SELECTION_DEBOUNCE;
            }
            self.selected_dirty = true;
            if method == "Target.targetDestroyed"
                && let Some(id) = params["targetId"].as_str()
            {
                self.candidates.remove(id);
                if self.page.as_ref().is_some_and(|p| p.target == id) {
                    self.page = None;
                    self.generation += 1;
                    self.fields_dirty = true;
                }
            }
            return;
        }
        let session = SessionId(event["sessionId"].as_str().unwrap_or("").to_owned());
        if method == "Target.detachedFromTarget" {
            let detached = SessionId(params["sessionId"].as_str().unwrap_or("").to_owned());
            if self
                .page
                .as_ref()
                .is_some_and(|page| page.session == detached)
            {
                let page = self.page.take().unwrap();
                self.candidates.remove(&page.target);
                self.selected_dirty = true;
                self.selection_at = Instant::now() + SELECTION_DEBOUNCE;
                self.fields_dirty = true;
                self.generation += 1;
                return;
            }
        }
        if !self.owns(&session) {
            return;
        }
        let page = self.page.as_mut().unwrap();
        match method {
            "Target.attachedToTarget" if params["targetInfo"]["type"] == "iframe" => {
                let (Some(id), Some(sid)) = (
                    params["targetInfo"]["targetId"].as_str(),
                    params["sessionId"].as_str(),
                ) else {
                    return;
                };
                let child = SessionId(sid.to_owned());
                if page.oopifs.len() >= MAX_FRAMES {
                    page.complete = false;
                    return;
                }
                page.oopifs.insert(id.to_owned(), child.clone());
                let old = page.frames.keys().find(|key| key.frame == id).cloned();
                let parent = old
                    .as_ref()
                    .and_then(|key| page.frames.remove(key))
                    .and_then(|f| f.parent)
                    .or_else(|| {
                        page.frames
                            .keys()
                            .find(|key| {
                                key.session == session && page.frames[*key].parent.is_none()
                            })
                            .cloned()
                    });
                let key = FrameKey {
                    session: child.clone(),
                    frame: id.to_owned(),
                };
                for frame in page.frames.values_mut() {
                    if old.is_some() && frame.parent == old {
                        frame.parent = Some(key.clone());
                    }
                }
                page.frames.insert(
                    key,
                    Frame {
                        parent,
                        ..Frame::default()
                    },
                );
                self.install.push_back(child);
                self.install_at = Instant::now();
                self.generation += 1;
            }
            "Target.detachedFromTarget" => {
                let detached = SessionId(params["sessionId"].as_str().unwrap_or("").to_owned());
                if let Some(id) = page
                    .oopifs
                    .iter()
                    .find_map(|(id, session)| (*session == detached).then_some(id.clone()))
                {
                    page.invalidate_subtree(&id, false);
                }
                self.generation += 1;
                self.fields_dirty = true;
            }
            "Runtime.executionContextCreated" if params["context"]["name"] == WORLD => {
                let context = &params["context"];
                let (Some(id), Some(frame_id)) = (
                    context["id"].as_u64(),
                    context["auxData"]["frameId"].as_str(),
                ) else {
                    return;
                };
                let key = FrameKey {
                    session,
                    frame: frame_id.to_owned(),
                };
                let unique_context = context["uniqueId"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned);
                if page
                    .oopifs
                    .get(frame_id)
                    .is_some_and(|owner| *owner != key.session)
                    && let Some(previous) = page.oopifs.remove(frame_id)
                {
                    page.scripts.remove(&previous);
                    page.installation.remove(&previous);
                }
                if page.frames.len() >= MAX_FRAMES && !page.frames.contains_key(&key) {
                    page.complete = false;
                    return;
                }
                let frame = page.claim_frame(key.clone());
                if frame.context != Some(id) || frame.unique_context != unique_context {
                    if frame.context.is_some() {
                        page.reset_document(&key);
                    }
                    let frame = page.frames.get_mut(&key).unwrap();
                    *frame = Frame {
                        context: Some(id),
                        unique_context,
                        parent: frame.parent.clone(),
                        ..Frame::default()
                    };
                    self.generation += 1;
                    self.fields_dirty = true;
                }
            }
            "Runtime.executionContextDestroyed" | "Runtime.executionContextsCleared" => {
                let id = params["executionContextId"].as_u64();
                let destroyed: Vec<_> = page
                    .frames
                    .iter()
                    .filter(|(key, frame)| {
                        key.session == session
                            && (method.ends_with("Cleared") || frame.context == id)
                    })
                    .map(|(key, _)| key.clone())
                    .collect();
                for key in destroyed {
                    page.reset_document(&key);
                    self.generation += 1;
                    self.fields_dirty = true;
                }
            }
            "Page.frameAttached" | "Page.frameNavigated" => {
                let frame = if method == "Page.frameNavigated" {
                    &params["frame"]
                } else {
                    params
                };
                let id = frame["id"].as_str().or_else(|| frame["frameId"].as_str());
                let Some(id) = id else { return };
                let key = FrameKey {
                    session: session.clone(),
                    frame: id.to_owned(),
                };
                if page.frames.len() >= MAX_FRAMES && !page.frames.contains_key(&key) {
                    page.complete = false;
                    return;
                }
                let parent_id = frame["parentId"]
                    .as_str()
                    .or_else(|| frame["parentFrameId"].as_str());
                let parent = parent_id
                    .and_then(|id| page.frames.keys().find(|k| k.frame == id).cloned())
                    .or_else(|| page.frames.get(&key).and_then(|frame| frame.parent.clone()));
                if method == "Page.frameNavigated" && params["type"] != "BackForwardCacheRestore" {
                    page.reset_document(&key);
                    *page.claim_frame(key) = Frame {
                        parent,
                        ..Frame::default()
                    };
                    if parent_id.is_none() && session == page.session {
                        page.root = id.to_owned();
                    }
                } else {
                    page.claim_frame(key).parent = parent;
                }
                self.generation += 1;
                self.fields_dirty = true;
            }
            "Page.frameDetached" => {
                if let Some(id) = params["frameId"].as_str() {
                    if params["reason"] == "swap"
                        && page
                            .frames
                            .keys()
                            .any(|key| key.frame == id && key.session != session)
                    {
                        return;
                    }
                    page.invalidate_subtree(id, params["reason"] != "swap");
                    self.generation += 1;
                    self.fields_dirty = true;
                }
            }
            "Runtime.bindingCalled" if params["name"] == BINDING => {
                let Some(id) = params["executionContextId"].as_u64() else {
                    return;
                };
                let Some((key, frame)) = page.frames.iter_mut().find_map(|(key, frame)| {
                    (key.session == session && frame.context == Some(id)).then_some((key, frame))
                }) else {
                    return;
                };
                let Some(report) = params["payload"].as_str().and_then(protocol::parse_report)
                else {
                    return;
                };
                frame.observed = true;
                match report {
                    Report::Tap(tap) => frame.tap = Some(tap),
                    Report::Focus(focus) => {
                        frame.focus = Some(focus);
                        self.fields_dirty = true;
                    }
                    Report::Input { at } => {
                        frame.last_input_ms = at;
                        self.input_serial += 1;
                    }
                    Report::Boxes(boxes) => {
                        self.fields_immediate |= frame.boxes.is_none()
                            || (key.session == page.session && key.frame == page.root);
                        frame.boxes = Some(boxes);
                        self.fields_dirty = true;
                    }
                }
            }
            _ => {}
        }
    }

    // Nonblocking reads drain tungstenite's buffer too; raw socket readiness alone misses it.
    fn drain_events(&mut self, until: Instant) -> Result<(), &'static str> {
        let until = until.min(Instant::now() + DRAIN_BUDGET);
        self.client.socket.get_mut().deadline = until;
        if self
            .client
            .socket
            .get_ref()
            .inner
            .set_nonblocking(true)
            .is_err()
        {
            self.healthy = false;
            return Err("socket option failed");
        }
        self.buffered = true;
        let result = (|| {
            for _ in 0..DRAIN_MESSAGES {
                if Instant::now() >= until {
                    break;
                }
                match self.client.socket.read() {
                    Ok(Message::Text(text)) => {
                        let value = serde_json::from_str(&text).map_err(|_| "invalid CDP JSON")?;
                        self.event(&value);
                    }
                    Ok(Message::Ping(_) | Message::Pong(_)) => {}
                    Err(tungstenite::Error::Io(e)) if timed_out(&e) => {
                        self.buffered = Instant::now() >= until;
                        break;
                    }
                    _ => return Err("CDP disconnected"),
                }
            }
            Ok(())
        })();
        if self
            .client
            .socket
            .get_ref()
            .inner
            .set_nonblocking(false)
            .is_err()
        {
            self.healthy = false;
            return Err("socket option failed");
        }
        if result.is_err() {
            self.healthy = false;
        }
        result
    }

    pub(super) fn pump(&mut self, until: Instant) -> Result<Changes, &'static str> {
        self.set_deadline(until);
        self.drain_events(until)?;
        if self.selected_dirty
            && Instant::now() >= self.selection_at
            && let Err(error) = self.refresh_selected(until)
            && !self.healthy
        {
            return Err(error);
        }
        if !self.install.is_empty()
            && Instant::now() >= self.install_at
            && let Err(error) = self.install_pending()
            && !self.healthy
        {
            return Err(error);
        }
        self.drain_events(until)?;
        Ok(Changes {
            fields: std::mem::take(&mut self.fields_dirty),
            immediate: std::mem::take(&mut self.fields_immediate),
        })
    }

    pub(super) fn call(
        &mut self,
        frame: &FrameKey,
        name: &str,
        arg: Value,
        await_promise: bool,
    ) -> Result<Value, CallError> {
        let context = self
            .page
            .as_ref()
            .and_then(|p| p.frames.get(frame))
            .and_then(|f| f.unique_context.clone())
            .ok_or(CallError::NotSent("frame unobserved"))?;
        if !matches!(name, "prepare" | "verify" | "stop") {
            return Err(CallError::NotSent("invalid agent call"));
        }
        let wait = if await_promise {
            Duration::from_millis(arg["windowMs"].as_u64().unwrap_or(0) + 75)
        } else {
            CALL_WAIT
        };
        let result = self.request("Runtime.callFunctionOn", json!({"uniqueContextId":context,"functionDeclaration":"function(name, arg) { return globalThis.__sv[name](arg); }","arguments":[{"value":name},{"value":arg}],"returnByValue":true,"awaitPromise":await_promise}), Some(&frame.session), wait)?;
        if result.get("exceptionDetails").is_some() {
            return Err(CallError::Uncertain);
        }
        if self
            .page
            .as_ref()
            .and_then(|p| p.frames.get(frame))
            .and_then(|f| f.unique_context.clone())
            != Some(context)
        {
            return Err(CallError::Uncertain);
        }
        result["result"]
            .get("value")
            .cloned()
            .ok_or(CallError::Uncertain)
    }

    pub(super) fn insert_text(&mut self, text: &str) -> Result<(), DeliveryError> {
        let session = self
            .page
            .as_ref()
            .map(|p| p.session.clone())
            .ok_or(DeliveryError::NotSent("no selected page"))?;
        self.request(
            "Input.insertText",
            json!({"text":text}),
            Some(&session),
            Duration::from_millis(150),
        )
        .map(|_| ())
        .map_err(|error| match error {
            CallError::NotSent(reason) => DeliveryError::NotSent(reason),
            CallError::Uncertain => DeliveryError::Uncertain,
        })
    }

    pub(super) fn flush(&mut self) -> Result<(), &'static str> {
        let page = self.page.as_ref().ok_or("no selected page")?;
        let sessions: Vec<_> = std::iter::once(page.session.clone())
            .chain(page.oopifs.values().cloned())
            .collect();
        for session in sessions {
            self.command(
                "Runtime.evaluate",
                json!({"expression":"0","returnByValue":true}),
                Some(&session),
            )?;
        }
        Ok(())
    }

    fn all_frames_observed(&self) -> bool {
        self.page.as_ref().is_some_and(|p| {
            p.complete
                && !p.frames.is_empty()
                && self.install.is_empty()
                && p.frames
                    .values()
                    .all(|f| f.unique_context.is_some() && f.observed)
        })
    }

    fn input_since(&self, serial: u64, wall_ms: Option<u64>) -> bool {
        self.input_serial != serial
            || wall_ms.is_none_or(|at| {
                self.page.as_ref().is_none_or(|p| {
                    p.frames.values().any(|f| {
                        // A preceding report can suppress another report for up to 50 ms.
                        f.last_input_ms != 0 && f.last_input_ms >= at.saturating_sub(50)
                    })
                })
            })
    }

    fn stop_page(&mut self) {
        let keys: Vec<_> = self
            .page
            .as_ref()
            .map(|p| p.frames.keys().cloned().collect())
            .unwrap_or_default();
        for key in keys {
            let _ = self.call(&key, "stop", Value::Null, false);
        }
        if let Some(page) = self.page.take() {
            for (session, identifier) in page.scripts {
                let _ = self.command(
                    "Page.removeScriptToEvaluateOnNewDocument",
                    json!({"identifier":identifier}),
                    Some(&session),
                );
                let _ = self.command(
                    "Runtime.removeBinding",
                    json!({"name":BINDING}),
                    Some(&session),
                );
                let _ = self.command(
                    "Target.setAutoAttach",
                    json!({"autoAttach":false,"flatten":true,"waitForDebuggerOnStart":false}),
                    Some(&session),
                );
            }
        }
        self.install.clear();
    }

    pub(super) fn close(mut self) {
        self.set_deadline(Instant::now() + CALL_WAIT);
        self.stop_page();
    }

    pub(super) fn fields(&mut self) -> Option<Fields> {
        if self.selected_dirty {
            return None;
        }
        let page = self.page.as_ref()?;
        let session = page.session.clone();
        let target = page.target.clone();
        let root = FrameKey {
            session: session.clone(),
            frame: page.root.clone(),
        };
        // An unreported child cannot contribute boxes and must not delay the root.
        let documents: HashMap<_, _> = page
            .frames
            .iter()
            .filter_map(|(key, frame)| {
                let (context, doc) = frame.document()?;
                Some((key.clone(), (context.to_owned(), doc)))
            })
            .collect();
        let (root_context, root_doc) = documents.get(&root)?;
        let bounds = self
            .command(
                "Browser.getWindowForTarget",
                json!({"targetId":target}),
                None,
            )
            .ok()?;
        let bounds = &bounds["bounds"];
        let window = geometry::Window {
            left: bounds["left"].as_f64()?,
            top: bounds["top"].as_f64()?,
            width: bounds["width"].as_f64()?,
            height: bounds["height"].as_f64()?,
            fullscreen: bounds["windowState"] == "fullscreen",
        };
        let metrics = self
            .command("Page.getLayoutMetrics", json!({}), Some(&session))
            .ok()?;
        let zoom = metrics["cssVisualViewport"]["zoom"]
            .as_f64()
            .filter(|v| v.is_finite() && *v > 0.)
            .unwrap_or(1.);
        let page = self.page.as_mut()?;
        page.window = Some(window);
        page.frames.get_mut(&root)?.boxes.as_mut()?.vp.zoom = zoom;
        let keys: Vec<_> = page
            .frames
            .keys()
            .filter(|key| **key != root && documents.contains_key(*key))
            .cloned()
            .collect();
        let mut offsets = HashMap::from([(root.clone(), [0., 0., f64::INFINITY, f64::INFINITY])]);
        for _ in 0..keys.len() {
            let mut progress = false;
            for key in &keys {
                if offsets.contains_key(key) {
                    continue;
                }
                let (context, doc) = &documents[key];
                if self
                    .page
                    .as_ref()?
                    .frames
                    .get(key)
                    .and_then(Frame::document)
                    != Some((context.as_str(), *doc))
                {
                    continue;
                }
                let Some(parent) = self
                    .page
                    .as_ref()?
                    .frames
                    .get(key)
                    .and_then(|frame| frame.parent.clone())
                else {
                    continue;
                };
                let Some(base) = offsets.get(&parent).copied() else {
                    continue;
                };
                if let Some(mut rect) = self.frame_rect(key, &parent) {
                    rect[0] += base[0];
                    rect[1] += base[1];
                    offsets.insert(key.clone(), rect);
                    progress = true;
                }
            }
            if !progress {
                break;
            }
        }
        self.drain_events(self.client.deadline).ok()?;
        if self.selected_dirty {
            return None;
        }
        let page = self.page.as_ref()?;
        if page.frames.get(&root)?.document() != Some((root_context.as_str(), *root_doc)) {
            return None;
        }
        let top = page.frames.get(&root)?.boxes.as_ref()?;
        let map = geometry::content_map(&top.vp, page.window.as_ref()?)?;
        // Drop only changed subtrees; unrelated attachment cannot invalidate root boxes.
        let mut valid = HashSet::from([root.clone()]);
        for _ in 0..keys.len() {
            let before = valid.len();
            for key in &keys {
                let Some(frame) = page.frames.get(key) else {
                    continue;
                };
                let (context, doc) = &documents[key];
                if frame.document() == Some((context.as_str(), *doc))
                    && frame
                        .parent
                        .as_ref()
                        .is_some_and(|parent| valid.contains(parent))
                    && offsets.contains_key(key)
                {
                    valid.insert(key.clone());
                }
            }
            if valid.len() == before {
                break;
            }
        }
        let children: Vec<_> = keys
            .iter()
            .filter(|key| valid.contains(key))
            .filter_map(|key| {
                Some(fields::Child {
                    offset: *offsets.get(key)?,
                    boxes: page.frames.get(key)?.boxes.as_ref()?,
                })
            })
            .collect();
        Some(fields::compose(top, &children, &map))
    }

    fn frame_rect(&mut self, child: &FrameKey, parent: &FrameKey) -> Option<[f64; 4]> {
        let context = self.page.as_ref()?.frames.get(parent)?.context?;
        let owner = self
            .command(
                "DOM.getFrameOwner",
                json!({"frameId":child.frame}),
                Some(&parent.session),
            )
            .ok()?;
        let node = self.command("DOM.resolveNode", json!({"backendNodeId":owner["backendNodeId"],"executionContextId":context,"objectGroup":"slimviewer-fields"}), Some(&parent.session)).ok()?;
        let object = node["object"]["objectId"].as_str()?;
        let result = self.command("Runtime.callFunctionOn", json!({"objectId":object,"functionDeclaration":"function() { return globalThis.__sv.frameRect(this); }","returnByValue":true}), Some(&parent.session));
        let _ = self.command(
            "Runtime.releaseObjectGroup",
            json!({"objectGroup":"slimviewer-fields"}),
            Some(&parent.session),
        );
        let result = result.ok()?;
        if result.get("exceptionDetails").is_some() {
            return None;
        }
        let values = result["result"]["value"].as_array()?;
        if values.len() != 4 {
            return None;
        }
        let mut rect = [0.; 4];
        for (out, value) in rect.iter_mut().zip(values) {
            *out = value.as_f64().filter(|v| v.is_finite())?;
        }
        (rect[2] > 0. && rect[3] > 0.).then_some(rect)
    }
}

impl PageOps for Session {
    fn resolve(&mut self, press: Option<Press>) -> Result<Target, PasteError> {
        self.ensure_selected(self.client.deadline)
            .map_err(PasteError::Untouched)?;
        self.drain_events(self.client.deadline)
            .map_err(PasteError::Untouched)?;
        if let Some(press) = press.filter(|press| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|d| u64::try_from(d.as_millis()).ok())
                .and_then(|now| now.checked_sub(press.wall_ms))
                .is_some_and(|age| age <= 1000)
        }) {
            let until = self.client.deadline.min(Instant::now() + TAP_WAIT);
            loop {
                let seen = self.page.as_ref().is_some_and(|p| {
                    p.frames.values().any(|f| {
                        f.tap
                            .as_ref()
                            .is_some_and(|tap| tap.at >= press.wall_ms.saturating_sub(2))
                    })
                });
                if seen {
                    break;
                }
                if Instant::now() >= until {
                    // RFB Ctrl+V stays ordered after the owner's tap when Chrome's report is late.
                    return Err(PasteError::Untouched("tap not observed"));
                }
                let mut poll = libc::pollfd {
                    fd: self.raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                let timeout = until
                    .saturating_duration_since(Instant::now())
                    .as_millis()
                    .max(1) as i32;
                // poll borrows one valid descriptor for this call; the actor owns its lifetime.
                if !self.buffered
                    && unsafe { libc::poll(&mut poll, 1, timeout) } < 0
                    && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted
                {
                    return Err(PasteError::Untouched("CDP poll failed"));
                }
                self.drain_events(until).map_err(PasteError::Untouched)?;
            }
        }
        self.install_pending().map_err(PasteError::Untouched)?;
        let page = self
            .page
            .as_ref()
            .ok_or(PasteError::Untouched("no selected page"))?;
        let newest_tap = page
            .frames
            .iter()
            .filter_map(|(key, f)| f.tap.as_ref().map(|tap| (key, tap)))
            .max_by_key(|(_, tap)| tap.at);
        let focus = page
            .frames
            .iter()
            .filter_map(|(key, f)| {
                f.focus
                    .as_ref()
                    .filter(|focus| focus.inside && focus.editable)
                    .map(|focus| (key, focus))
            })
            .max_by_key(|(_, focus)| focus.at);
        let (frame, refocus) = if let Some((key, _)) = focus {
            (key.clone(), false)
        } else if let Some((key, _)) = newest_tap.filter(|(_, tap)| {
            tap.editable && press.is_some_and(|press| tap.at >= press.wall_ms.saturating_sub(2))
        }) {
            (key.clone(), true)
        } else {
            (
                FrameKey {
                    session: page.session.clone(),
                    frame: page.root.clone(),
                },
                false,
            )
        };
        let observed = page
            .frames
            .get(&frame)
            .filter(|f| f.observed)
            .and_then(|f| f.unique_context.clone())
            .ok_or(PasteError::Untouched("frame unobserved"))?;
        self.transaction = self.transaction.wrapping_add(1).max(1);
        self.transaction_context = Some((frame.clone(), observed));
        Ok(Target { frame, refocus })
    }

    fn attempt(
        &mut self,
        target: &Target,
        text: &str,
        attempt: Attempt,
    ) -> Result<AttemptResult, PasteError> {
        let retry = matches!(attempt, Attempt::Retry);
        let uncertain = || AttemptResult {
            path: Path::Inserted {
                reply: Reply::Uncertain,
            },
            evidence: None,
            unobserved_input: false,
            all_frames_observed: false,
        };
        let context = self
            .page
            .as_ref()
            .and_then(|p| p.frames.get(&target.frame))
            .and_then(|f| f.unique_context.clone());
        if self
            .transaction_context
            .as_ref()
            .is_none_or(|(key, id)| *key != target.frame || Some(id) != context.as_ref())
        {
            return if retry {
                Ok(uncertain())
            } else {
                Err(PasteError::Untouched("frame changed before paste"))
            };
        }
        let input_serial = self.input_serial;
        let started_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|d| u64::try_from(d.as_millis()).ok());
        let generation = self.generation;
        let prepared = self.call(
            &target.frame,
            "prepare",
            protocol::PrepareArg {
                id: self.transaction,
                text,
                refocus: target.refocus || retry,
                retry,
            }
            .to_value(),
            false,
        );
        let mut wait = 500;
        let path = match prepared {
            Err(CallError::NotSent(reason)) if !retry => return Err(PasteError::Untouched(reason)),
            Err(_) => Path::Inserted {
                reply: Reply::Uncertain,
            },
            Ok(value) => match Prepared::parse(&value) {
                Some(Prepared::Ready) => {
                    let selected = self.page.as_ref().map(|p| p.target.clone());
                    let valid = selected
                        .as_ref()
                        .and_then(|id| self.candidates.remove(id))
                        .map(|page| {
                            let focus = self.probe(&page);
                            let valid = focus.is_ok_and(|(focused, visible)| {
                                visible
                                    && match page.evidence {
                                        FocusEvidence::Document => focused,
                                        FocusEvidence::Native => self
                                            .native_front(&mut NativeFront::default())
                                            .is_ok_and(|target| {
                                                target.as_deref() == Some(&page.target)
                                            }),
                                        FocusEvidence::Visible => true,
                                    }
                            });
                            self.candidates.insert(page.target.clone(), page);
                            valid
                        })
                        .unwrap_or(false);
                    if !valid
                        || self
                            .page
                            .as_ref()
                            .and_then(|p| p.frames.get(&target.frame))
                            .and_then(|f| f.unique_context.clone())
                            != context
                    {
                        Path::Inserted {
                            reply: Reply::Uncertain,
                        }
                    } else {
                        let reply = match self.insert_text(text) {
                            Ok(()) => {
                                wait = 50;
                                Reply::Ok
                            }
                            Err(_) => Reply::Uncertain,
                        };
                        Path::Inserted { reply }
                    }
                }
                Some(Prepared::Handled) => {
                    wait = 300;
                    Path::Handled
                }
                Some(state @ (Prepared::NoField { .. } | Prepared::NotFocused)) => {
                    let handled = matches!(state, Prepared::NoField { handled: true });
                    let flushed = self.flush().is_ok();
                    let same = self
                        .page
                        .as_ref()
                        .and_then(|p| p.frames.get(&target.frame))
                        .and_then(|f| f.unique_context.clone())
                        == context;
                    if !flushed || !same {
                        return Ok(uncertain());
                    }
                    return Ok(AttemptResult {
                        path: Path::NoField { handled },
                        evidence: None,
                        unobserved_input: self.input_since(input_serial, started_ms),
                        all_frames_observed: false,
                    });
                }
                None => Path::Inserted {
                    reply: Reply::Uncertain,
                },
            },
        };
        let mut evidence = self
            .call(
                &target.frame,
                "verify",
                protocol::VerifyArg {
                    id: self.transaction,
                    window_ms: wait,
                }
                .to_value(),
                true,
            )
            .ok()
            .and_then(|value| Evidence::parse(&value));
        let flushed = self.flush().is_ok();
        if self
            .page
            .as_ref()
            .and_then(|p| p.frames.get(&target.frame))
            .and_then(|f| f.unique_context.clone())
            != context
            && let Some(evidence) = &mut evidence
        {
            evidence.gone = true;
        }
        Ok(AttemptResult {
            path,
            evidence,
            unobserved_input: !flushed || self.input_since(input_serial, started_ms),
            all_frames_observed: flushed
                && self.generation == generation
                && self.all_frames_observed(),
        })
    }
}

fn timed_out(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::{Ipv4Addr, TcpListener};
    use std::thread;
    use std::time::Duration;
    use tungstenite::Message;

    fn peer() -> (u16, thread::JoinHandle<Vec<Value>>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = thread::spawn(move || {
            let (mut http, _) = listener.accept().unwrap();
            http.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                http.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            let body = json!({"webSocketDebuggerUrl":"ws://ignored.invalid/devtools/browser/test-session"}).to_string();
            write!(
                http,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            let (socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut ws = tungstenite::accept(socket).unwrap();
            let mut log = Vec::new();
            while let Ok(Message::Text(raw)) = ws.read() {
                let request: Value = serde_json::from_str(&raw).unwrap();
                log.push(request.clone());
                let result = match request["method"].as_str().unwrap() {
                    "Target.getTargets" => {
                        json!({"targetInfos":[{"type":"page","targetId":"page"}]})
                    }
                    "Target.attachToTarget" => json!({"sessionId":"page-session"}),
                    "Page.getFrameTree" => json!({"frameTree":{"frame":{"id":"main"}}}),
                    "Page.createIsolatedWorld" => json!({"executionContextId":1}),
                    "Page.addScriptToEvaluateOnNewDocument" => json!({"identifier":"script"}),
                    "Runtime.evaluate"
                        if request["params"]["expression"] == super::super::FOCUS =>
                    {
                        json!({"result":{"value":{"focused":true,"visible":true}}})
                    }
                    "Browser.getWindowForTarget" => {
                        json!({"bounds":{"left":0,"top":0,"width":390,"height":844,"windowState":"fullscreen"}})
                    }
                    "Runtime.callFunctionOn" => json!({"result":{"value":{"state":"ready"}}}),
                    _ => json!({"result":{"value":0}}),
                };
                ws.send(Message::Text(
                    json!({"id":request["id"],"result":result})
                        .to_string()
                        .into(),
                ))
                .unwrap();
            }
            log
        });
        (port, task)
    }

    #[test]
    fn tests_that_selected_page_installs_a_world_binding_and_script() {
        let (port, server) = peer();
        let session = Session::open_at(port, Instant::now() + Duration::from_secs(2));
        assert!(session.is_ok());
        drop(session);
        let log = server.join().unwrap();
        let binding = log
            .iter()
            .find(|r| r["method"] == "Runtime.addBinding")
            .unwrap();
        assert_eq!(
            binding["params"],
            json!({"name":BINDING,"executionContextName":WORLD})
        );
        let script = log
            .iter()
            .find(|r| r["method"] == "Page.addScriptToEvaluateOnNewDocument")
            .unwrap();
        assert_eq!(script["params"]["source"], SCRIPT);
        assert_eq!(script["params"]["worldName"], WORLD);
        assert_eq!(script["params"]["runImmediately"], true);
        let attach = log
            .iter()
            .find(|r| r["method"] == "Target.setAutoAttach")
            .unwrap();
        assert_eq!(
            attach["params"],
            json!({"autoAttach":true,"flatten":true,"waitForDebuggerOnStart":false,"filter":[{"type":"iframe"}]})
        );
    }
    #[test]
    fn tests_that_frame_tree_keeps_one_owner_for_a_cross_process_frame() {
        let (port, server) = peer();
        let mut session = Session::open_at(port, Instant::now() + Duration::from_secs(2)).unwrap();
        session.event(&json!({"sessionId":"page-session","method":"Page.frameAttached","params":{"frameId":"child","parentFrameId":"main"}}));
        session.event(&json!({"sessionId":"page-session","method":"Target.attachedToTarget","params":{"sessionId":"child-session","targetInfo":{"type":"iframe","targetId":"child"}}}));
        session.frame_tree(&SessionId("page-session".into()), &json!({"frame":{"id":"main"},"childFrames":[{"frame":{"id":"child","parentId":"main"}}]}), None);
        let keys: Vec<_> = session
            .page
            .as_ref()
            .unwrap()
            .frames
            .keys()
            .filter(|key| key.frame == "child")
            .collect();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].session.0, "child-session");
        drop(session);
        server.join().unwrap();
    }

    #[test]
    fn tests_that_navigation_rejects_the_old_world_before_another_prepare() {
        let (port, server) = peer();
        let mut session = Session::open_at(port, Instant::now() + Duration::from_secs(2)).unwrap();
        let key = FrameKey {
            session: SessionId("page-session".into()),
            frame: "main".into(),
        };
        session.event(&json!({"sessionId":"page-session","method":"Page.frameNavigated","params":{"frame":{"id":"main"}}}));
        assert!(matches!(
            session.call(&key, "prepare", json!({}), false),
            Err(CallError::NotSent(_))
        ));
        drop(session);
        assert!(
            !server
                .join()
                .unwrap()
                .iter()
                .any(|r| r["method"] == "Runtime.callFunctionOn")
        );
    }
}
#[cfg(test)]
mod transaction_tests {
    use super::super::agent::PasteOutcome;
    use super::super::paste;
    use super::*;
    use std::io::{Read, Write};
    use std::net::{Ipv4Addr, TcpListener};
    use std::sync::mpsc::{self, Sender};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use std::thread::{self, JoinHandle};

    #[derive(Clone, Copy)]
    enum InsertReply {
        Ack,
        Lost,
        Closed,
    }

    struct Browser {
        tabs: usize,
        front: usize,
        depth: usize,
        focused_depth: usize,
        editable: bool,
        unknown_frame: bool,
        immediate: bool,
        insert_reply: InsertReply,
        changed: bool,
        prepared: &'static str,
        prepare_throws: bool,
        invalidate: Option<&'static str>,
        input_on_prepare: bool,
        fail_install: Option<&'static str>,
        focus_delay: Duration,
        install_delay: Duration,
        native_delay: Duration,
        script_reply_delay: Duration,
        create_during_discovery: bool,
        navigate_during_fields: bool,
        events_during_fields: Vec<Value>,
        field_event_method: &'static str,
        ambiguous_windows: Option<Arc<AtomicBool>>,
        persistent_install_failure: Option<Arc<AtomicBool>>,
    }

    impl Default for Browser {
        fn default() -> Self {
            Self {
                tabs: 1,
                front: 0,
                depth: 0,
                focused_depth: 0,
                editable: true,
                unknown_frame: false,
                immediate: true,
                insert_reply: InsertReply::Ack,
                changed: true,
                prepared: "ready",
                prepare_throws: false,
                invalidate: None,
                input_on_prepare: false,
                fail_install: None,
                focus_delay: Duration::ZERO,
                install_delay: Duration::ZERO,
                native_delay: Duration::ZERO,
                script_reply_delay: Duration::ZERO,
                create_during_discovery: false,
                navigate_during_fields: false,
                events_during_fields: Vec::new(),
                field_event_method: "Page.getLayoutMetrics",
                ambiguous_windows: None,
                persistent_install_failure: None,
            }
        }
    }

    fn frame(session: &str) -> String {
        match session {
            "child-session" => "child".into(),
            "grand-session" => "grand".into(),
            page => format!("main-{}", page.strip_prefix("page-").unwrap()),
        }
    }

    fn context(session: &str) -> u64 {
        match session {
            "child-session" => 20,
            "grand-session" => 30,
            page => 10 + page.strip_prefix("page-").unwrap().parse::<u64>().unwrap(),
        }
    }

    fn depth(session: &str) -> usize {
        match session {
            "child-session" => 1,
            "grand-session" => 2,
            _ => 0,
        }
    }

    fn binding(session: &str, payload: Value) -> Value {
        json!({"method":"Runtime.bindingCalled","sessionId":session,
            "params":{"name":BINDING,"executionContextId":context(session),"payload":payload.to_string()}})
    }

    fn world(session: &str) -> Value {
        json!({"method":"Runtime.executionContextCreated","sessionId":session,
            "params":{"context":{"id":context(session),"uniqueId":format!("world-{session}"),"name":WORLD,"auxData":{"frameId":frame(session),"isDefault":false}}}})
    }

    fn event(ws: &mut tungstenite::WebSocket<std::net::TcpStream>, value: Value) {
        ws.send(Message::Text(value.to_string().into())).unwrap();
    }

    fn box_report(session: &str, focused: bool) -> Value {
        json!({"k":"boxes","doc":context(session),
            "vp":{"vv":[0,0,390,844],"vv_scale":1,"inner":[390,844],"dpr":2,"screen":[390,844],"zoom":1},
            "boxes":[[10,20,100,30]],"focused":if focused { json!(0) } else { Value::Null }})
    }

    fn observe(
        ws: &mut tungstenite::WebSocket<std::net::TcpStream>,
        session: &str,
        model: &Browser,
    ) {
        event(ws, world(session));
        let focused = model.editable && depth(session) == model.focused_depth;
        event(
            ws,
            binding(
                session,
                json!({"k":"focus","at":1000 + depth(session),"editable":focused,"inside":focused}),
            ),
        );
        event(ws, binding(session, box_report(session, focused)));
    }

    fn tree(session: &str, model: &Browser) -> Value {
        let mut node = json!({"frame":{"id":frame(session)}});
        let mut children = Vec::new();
        if depth(session) < model.depth {
            children.push(tree(
                if depth(session) == 0 {
                    "child-session"
                } else {
                    "grand-session"
                },
                model,
            ));
        }
        if depth(session) == 0 && model.unknown_frame {
            children.push(json!({"frame":{"id":"unknown"}}));
        }
        if !children.is_empty() {
            node["childFrames"] = Value::Array(children);
        }
        node
    }

    fn peer(model: Browser) -> (u16, Sender<Value>, JoinHandle<Vec<Value>>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (events, incoming) = mpsc::channel::<Value>();
        let server = thread::spawn(move || {
            let (mut http, _) = listener.accept().unwrap();
            http.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                http.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            assert!(request.starts_with(b"GET /json/version HTTP/1.1\r\n"));
            let body = json!({"webSocketDebuggerUrl":"ws://untrusted.invalid:1/devtools/browser/persistent-tests"}).to_string();
            write!(
                http,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            let (socket, _) = listener.accept().unwrap();
            socket.set_nodelay(true).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut ws = tungstenite::accept(socket).unwrap();
            ws.get_mut()
                .set_read_timeout(Some(Duration::from_millis(10)))
                .unwrap();
            let mut log = Vec::new();
            let mut front = model.front;
            let mut tabs = model.tabs;
            let mut created = false;
            let mut attached = std::collections::HashSet::new();
            let mut late = None;
            let mut failed_install = false;
            let mut navigated_during_fields = false;
            let mut fields_events = model.events_during_fields.clone();
            let end = Instant::now() + Duration::from_secs(5);
            while Instant::now() < end {
                for value in incoming.try_iter() {
                    if let Some(next) = value["testFront"].as_u64() {
                        front = next as usize;
                        if value["notify"] == true {
                            event(
                                &mut ws,
                                json!({"method":"Target.targetInfoChanged","params":{"targetInfo":{"targetId":format!("tab-{front}"),"type":"page"}}}),
                            );
                        }
                    } else if let Some(n) = value["testBurst"].as_u64() {
                        for _ in 0..n {
                            event(&mut ws, binding("page-0", json!({"k":"input","at":1000})));
                        }
                    } else {
                        event(&mut ws, value);
                    }
                }
                let raw = match ws.read() {
                    Ok(Message::Text(raw)) => raw,
                    Err(tungstenite::Error::Io(error)) if timed_out(&error) => continue,
                    _ => break,
                };
                let request: Value = serde_json::from_str(&raw).unwrap();
                let session = request["sessionId"].as_str().unwrap_or("");
                log.push(request.clone());
                let method = request["method"].as_str().unwrap();
                if method == model.field_event_method {
                    for value in fields_events.drain(..) {
                        event(&mut ws, value);
                    }
                }
                if (!failed_install
                    || model
                        .persistent_install_failure
                        .as_ref()
                        .is_some_and(|fail| fail.load(Ordering::Acquire)))
                    && session == "child-session"
                    && model.fail_install == Some(method)
                {
                    failed_install = true;
                    event(
                        &mut ws,
                        json!({"id":request["id"],"error":{"code":-32000,"message":"fixture install rejection"}}),
                    );
                    continue;
                }
                if matches!(
                    method,
                    "Runtime.enable"
                        | "Page.enable"
                        | "Runtime.addBinding"
                        | "Page.addScriptToEvaluateOnNewDocument"
                        | "Target.setAutoAttach"
                        | "Page.getFrameTree"
                ) {
                    thread::sleep(model.install_delay);
                }
                let result = match method {
                    "Target.setDiscoverTargets" => {
                        assert_eq!(request["params"]["discover"], true);
                        json!({})
                    }
                    "Target.getTargets" => {
                        let snapshot = tabs;
                        if model.create_during_discovery && !created {
                            created = true;
                            event(
                                &mut ws,
                                json!({"method":"Target.targetCreated","params":{"targetInfo":{"type":"page","targetId":format!("tab-{tabs}")}}}),
                            );
                            tabs += 1;
                        }
                        json!({"targetInfos":(0..snapshot).map(|i| json!({"type":"page","targetId":format!("tab-{i}")})).collect::<Vec<_>>()})
                    }
                    "Target.attachToTarget" => {
                        let target = request["params"]["targetId"].as_str().unwrap();
                        assert_eq!(request["params"]["flatten"], true);
                        json!({"sessionId":format!("page-{}", target.strip_prefix("tab-").unwrap())})
                    }
                    "Page.getFrameTree" => json!({"frameTree":tree(session, &model)}),
                    "Page.createIsolatedWorld" => {
                        assert_eq!(request["params"]["worldName"], WORLD);
                        json!({"executionContextId":if request["params"]["frameId"] == "unknown" { 99 } else { context(session) }})
                    }
                    "Runtime.evaluate"
                        if request["params"]["expression"] == super::super::FOCUS =>
                    {
                        thread::sleep(model.focus_delay);
                        json!({"result":{"value":{"focused":true,"visible":true}}})
                    }
                    "Runtime.evaluate"
                        if request["params"]["expression"]
                            .as_str()
                            .is_some_and(|source| source.contains(SCRIPT)) =>
                    {
                        if request["params"]["contextId"] == 99 {
                            json!({"exceptionDetails":{"text":"frame agent unavailable"}})
                        } else {
                            observe(&mut ws, session, &model);
                            json!({"result":{"type":"undefined"}})
                        }
                    }
                    "Runtime.evaluate" if request["params"]["expression"] == "0" => {
                        json!({"result":{"value":0}})
                    }
                    "Browser.getWindows" => {
                        thread::sleep(model.native_delay);
                        if model
                            .ambiguous_windows
                            .as_ref()
                            .is_some_and(|ambiguous| ambiguous.load(Ordering::Acquire))
                        {
                            json!({"windows":[{"windowId":7,"isActive":false,"isVisible":true},{"windowId":8,"isActive":false,"isVisible":true}]})
                        } else {
                            json!({"windows":[{"windowId":7,"isActive":true,"isVisible":true}]})
                        }
                    }
                    "Browser.getActiveTab" => {
                        thread::sleep(model.native_delay);
                        assert_eq!(request["params"]["windowId"], 7);
                        json!({"tab":{"targetId":format!("tab-{front}"),"windowId":7,"isActive":true,"isHidden":false}})
                    }
                    "Page.addScriptToEvaluateOnNewDocument" => {
                        assert_eq!(request["params"]["worldName"], WORLD);
                        assert_eq!(request["params"]["source"], SCRIPT);
                        assert_eq!(request["params"]["runImmediately"], true);
                        if model.immediate {
                            observe(&mut ws, session, &model);
                        }
                        thread::sleep(model.script_reply_delay);
                        json!({"identifier":format!("script-{session}")})
                    }
                    "Target.setAutoAttach" => {
                        if request["params"]["autoAttach"] == true {
                            assert_eq!(request["params"]["filter"], json!([{"type":"iframe"}]));
                            assert_eq!(request["params"]["flatten"], true);
                            assert_eq!(request["params"]["waitForDebuggerOnStart"], false);
                            if depth(session) < model.depth && attached.insert(session.to_owned()) {
                                let child = if depth(session) == 0 {
                                    "child-session"
                                } else {
                                    "grand-session"
                                };
                                event(
                                    &mut ws,
                                    json!({"method":"Target.attachedToTarget","sessionId":session,
                                    "params":{"sessionId":child,"targetInfo":{"type":"iframe","targetId":frame(child)},"waitingForDebugger":false}}),
                                );
                            }
                        }
                        json!({})
                    }
                    "Browser.getWindowForTarget" => {
                        json!({"bounds":{"left":0,"top":0,"width":780,"height":1688,"windowState":"fullscreen"}})
                    }
                    "Page.getLayoutMetrics" => {
                        if model.navigate_during_fields && !navigated_during_fields {
                            navigated_during_fields = true;
                            event(
                                &mut ws,
                                json!({"method":"Page.frameNavigated","sessionId":session,
                                "params":{"frame":{"id":frame(session)}}}),
                            );
                            let mut next_world = world(session);
                            next_world["params"]["context"]["uniqueId"] =
                                json!(format!("next-world-{session}"));
                            event(&mut ws, next_world);
                            event(&mut ws, binding(session, box_report(session, true)));
                        }
                        json!({"cssVisualViewport":{"zoom":2}})
                    }
                    "DOM.getFrameOwner" => json!({"backendNodeId":42}),
                    "DOM.resolveNode" => json!({"object":{"objectId":"frame-element"}}),
                    "Runtime.releaseObjectGroup" => json!({}),
                    "Runtime.callFunctionOn"
                        if request["params"]["objectId"] == "frame-element" =>
                    {
                        assert_eq!(
                            request["params"]["functionDeclaration"],
                            "function() { return globalThis.__sv.frameRect(this); }"
                        );
                        json!({"result":{"value":[10,20,300,400]}})
                    }
                    "Runtime.callFunctionOn" => {
                        let name = request["params"]["arguments"][0]["value"].as_str().unwrap();
                        assert_eq!(
                            request["params"]["uniqueContextId"],
                            format!("world-{session}")
                        );
                        assert_eq!(request["params"]["returnByValue"], true);
                        match name {
                            "prepare" if model.prepare_throws => {
                                json!({"exceptionDetails":{"text":"not built"}})
                            }
                            "prepare" => {
                                if model.input_on_prepare {
                                    event(
                                        &mut ws,
                                        binding(session, json!({"k":"input","at":10000})),
                                    );
                                }
                                json!({"result":{"value":{"state":model.prepared,"handled":false}}})
                            }
                            "verify" => {
                                assert_eq!(request["params"]["awaitPromise"], true);
                                if let Some(id) = late.take() {
                                    event(&mut ws, json!({"id":id,"result":{}}));
                                }
                                json!({"result":{"value":{"changed":model.changed,"beforeInput":0,"input":0,"focusMoved":false,"gone":false}}})
                            }
                            "stop" => json!({"result":{"value":null}}),
                            _ => panic!("unexpected agent method"),
                        }
                    }
                    "Input.insertText" => {
                        assert_eq!(session, format!("page-{front}"));
                        if let Some(kind) = model.invalidate {
                            let target = match model.focused_depth {
                                1 => "child-session",
                                2 => "grand-session",
                                _ => session,
                            };
                            let change = match kind {
                                "context" => {
                                    json!({"method":"Runtime.executionContextDestroyed","sessionId":target,"params":{"executionContextId":context(target)}})
                                }
                                "navigation" => {
                                    json!({"method":"Page.frameNavigated","sessionId":target,"params":{"frame":{"id":frame(target)}}})
                                }
                                "detach" => {
                                    json!({"method":"Target.detachedFromTarget","sessionId":session,"params":{"sessionId":"child-session","targetId":"child"}})
                                }
                                _ => unreachable!(),
                            };
                            event(&mut ws, change);
                        }
                        match model.insert_reply {
                            InsertReply::Ack => json!({}),
                            InsertReply::Lost => {
                                late = Some(request["id"].clone());
                                continue;
                            }
                            InsertReply::Closed => break,
                        }
                    }
                    "Runtime.enable"
                    | "Page.enable"
                    | "Runtime.addBinding"
                    | "Runtime.removeBinding"
                    | "Page.removeScriptToEvaluateOnNewDocument"
                    | "Target.detachFromTarget" => json!({}),
                    other => panic!("unexpected CDP method: {other}"),
                };
                event(
                    &mut ws,
                    json!({"id":request["id"],"sessionId":session,"result":result}),
                );
            }
            log
        });
        (port, events, server)
    }

    fn open(port: u16) -> Session {
        Session::open_at(port, Instant::now() + Duration::from_secs(2)).unwrap()
    }

    fn count(log: &[Value], method: &str) -> usize {
        log.iter()
            .filter(|request| request["method"] == method)
            .count()
    }

    fn calls<'a>(log: &'a [Value], name: &str) -> Vec<&'a Value> {
        log.iter()
            .filter(|request| {
                request["method"] == "Runtime.callFunctionOn"
                    && request["params"]["arguments"][0]["value"] == name
            })
            .collect()
    }

    #[test]
    fn tests_that_recursive_oopif_paste_prepares_and_verifies_in_the_child_but_inserts_on_the_page()
    {
        let (port, _, server) = peer(Browser {
            depth: 2,
            focused_depth: 2,
            ..Browser::default()
        });
        let mut session = open(port);
        assert_eq!(
            paste::run(&mut session, "fixture", None).unwrap(),
            PasteOutcome::Pasted
        );
        drop(session);
        let log = server.join().unwrap();
        for sid in ["page-0", "child-session", "grand-session"] {
            let binding = log
                .iter()
                .find(|r| r["method"] == "Runtime.addBinding" && r["sessionId"] == sid)
                .unwrap();
            assert_eq!(
                binding["params"],
                json!({"name":BINDING,"executionContextName":WORLD})
            );
            assert!(
                log.iter()
                    .any(|r| r["method"] == "Page.addScriptToEvaluateOnNewDocument"
                        && r["sessionId"] == sid)
            );
            assert!(
                log.iter()
                    .any(|r| r["method"] == "Target.setAutoAttach" && r["sessionId"] == sid)
            );
            assert!(log.iter().any(|r| r["method"] == "Runtime.evaluate"
                && r["params"]["expression"] == "0"
                && r["sessionId"] == sid));
        }
        assert_eq!(calls(&log, "prepare").len(), 1);
        assert_eq!(calls(&log, "prepare")[0]["sessionId"], "grand-session");
        assert_eq!(calls(&log, "verify")[0]["sessionId"], "grand-session");
        assert_eq!(count(&log, "Input.insertText"), 1);
    }

    #[test]
    fn tests_that_existing_frames_get_an_agent_when_run_immediately_does_not_create_a_world() {
        let (port, _, server) = peer(Browser {
            immediate: false,
            ..Browser::default()
        });
        let mut session = open(port);
        assert_eq!(
            paste::run(&mut session, "fixture", None).unwrap(),
            PasteOutcome::Pasted
        );
        drop(session);
        let log = server.join().unwrap();
        assert!(log.iter().any(|r| {
            r["method"] == "Runtime.evaluate"
                && r["params"]["expression"]
                    .as_str()
                    .is_some_and(|source| source.contains(SCRIPT))
        }));
    }

    #[test]
    fn tests_that_a_lost_insert_reply_uses_read_back_and_ignores_the_late_ack_without_replay() {
        for changed in [false, true] {
            let (port, _, server) = peer(Browser {
                insert_reply: InsertReply::Lost,
                changed,
                ..Browser::default()
            });
            let mut session = open(port);
            session.set_deadline(Instant::now() + Duration::from_millis(1200));
            let expected = if changed {
                PasteOutcome::Pasted
            } else {
                PasteOutcome::Unconfirmed
            };
            assert_eq!(paste::run(&mut session, "fixture", None).unwrap(), expected);
            assert!(session.healthy());
            drop(session);
            let log = server.join().unwrap();
            assert_eq!(count(&log, "Input.insertText"), 1);
            assert_eq!(calls(&log, "prepare").len(), 1);
            assert_eq!(calls(&log, "verify").len(), 1);
            assert_eq!(
                calls(&log, "verify")[0]["params"]["arguments"][1]["value"]["windowMs"],
                500
            );
        }
    }

    #[test]
    fn tests_that_a_closed_socket_after_insert_returns_unconfirmed_without_fallback() {
        let (port, _, server) = peer(Browser {
            insert_reply: InsertReply::Closed,
            ..Browser::default()
        });
        let mut session = open(port);
        assert_eq!(
            paste::run(&mut session, "fixture", None).unwrap(),
            PasteOutcome::Unconfirmed
        );
        drop(session);
        assert_eq!(count(&server.join().unwrap(), "Input.insertText"), 1);
    }

    #[test]
    fn tests_that_an_unobserved_frame_prevents_retry_and_does_not_claim_absence() {
        let (port, _, server) = peer(Browser {
            unknown_frame: true,
            changed: false,
            ..Browser::default()
        });
        let mut session = open(port);
        assert_eq!(
            paste::run(&mut session, "fixture", None).unwrap(),
            PasteOutcome::Unconfirmed
        );
        drop(session);
        let log = server.join().unwrap();
        assert_eq!(count(&log, "Input.insertText"), 1);
        assert_eq!(calls(&log, "prepare").len(), 1);
    }

    #[test]
    fn tests_that_a_fully_observed_miss_retries_only_once_with_the_same_transaction() {
        let (port, _, server) = peer(Browser {
            depth: 2,
            focused_depth: 2,
            changed: false,
            ..Browser::default()
        });
        let mut session = open(port);
        assert_eq!(
            paste::run(&mut session, "fixture", None).unwrap(),
            PasteOutcome::NotLanded
        );
        drop(session);
        let log = server.join().unwrap();
        let prepared = calls(&log, "prepare");
        assert_eq!(prepared.len(), 2);
        let first = &prepared[0]["params"]["arguments"][1]["value"];
        let second = &prepared[1]["params"]["arguments"][1]["value"];
        assert_eq!(first["retry"], false);
        assert_eq!(second["retry"], true);
        assert_eq!(second["refocus"], true);
        assert_eq!(first["id"], second["id"]);
        assert_eq!(count(&log, "Input.insertText"), 2);
    }

    #[test]
    fn tests_that_handled_paste_and_script_errors_never_insert() {
        for (prepared, prepare_throws, expected) in [
            ("handled", false, PasteOutcome::Refused),
            ("ready", true, PasteOutcome::Unconfirmed),
        ] {
            let (port, _, server) = peer(Browser {
                prepared,
                prepare_throws,
                changed: false,
                ..Browser::default()
            });
            let mut session = open(port);
            assert_eq!(paste::run(&mut session, "fixture", None).unwrap(), expected);
            drop(session);
            let log = server.join().unwrap();
            assert_eq!(count(&log, "Input.insertText"), 0);
            assert_eq!(calls(&log, "prepare").len(), 1);
        }
    }

    #[test]
    fn tests_that_native_front_tab_disambiguates_multiple_visible_focused_documents() {
        let (port, _, server) = peer(Browser {
            tabs: 3,
            front: 1,
            ..Browser::default()
        });
        let mut session = open(port);
        assert_eq!(
            paste::run(&mut session, "fixture", None).unwrap(),
            PasteOutcome::Pasted
        );
        drop(session);
        let log = server.join().unwrap();
        assert!(count(&log, "Browser.getActiveTab") >= 2);
        assert_eq!(calls(&log, "prepare")[0]["sessionId"], "page-1");
        assert!(
            log.iter()
                .filter(|r| r["method"] == "Page.addScriptToEvaluateOnNewDocument")
                .all(|r| r["sessionId"] == "page-1")
        );
    }

    #[test]
    fn tests_that_navigation_context_loss_and_recursive_detach_invalidate_paste_evidence() {
        for invalidate in ["context", "navigation", "detach"] {
            let (port, _, server) = peer(Browser {
                depth: 2,
                focused_depth: 2,
                invalidate: Some(invalidate),
                ..Browser::default()
            });
            let mut session = open(port);
            assert_eq!(
                paste::run(&mut session, "fixture", None).unwrap(),
                PasteOutcome::Unconfirmed,
                "{invalidate}"
            );
            drop(session);
            let log = server.join().unwrap();
            assert_eq!(count(&log, "Input.insertText"), 1);
            assert_eq!(calls(&log, "prepare").len(), 1);
        }
    }

    #[test]
    fn tests_that_a_late_trusted_tap_unlocks_the_barrier_and_refocuses_its_frame() {
        let (port, events, server) = peer(Browser {
            depth: 1,
            editable: false,
            ..Browser::default()
        });
        let mut session = open(port);
        let press = Press {
            wall_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            x: 20,
            y: 30,
        };
        let delayed = thread::spawn(move || {
            thread::sleep(Duration::from_millis(40));
            events
                .send(binding(
                    "child-session",
                    json!({"k":"tap","at":press.wall_ms - 2,"editable":true}),
                ))
                .unwrap();
        });
        let target = session.resolve(Some(press)).unwrap();
        assert!(target.refocus);
        assert_eq!(target.frame.session.0, "child-session");
        assert_eq!(target.frame.frame, "child");
        delayed.join().unwrap();
        drop(session);
        assert!(calls(&server.join().unwrap(), "prepare").is_empty());
    }

    #[test]
    fn tests_that_stale_or_foreign_binding_reports_cannot_satisfy_the_tap_barrier() {
        let (port, events, server) = peer(Browser::default());
        let mut session = open(port);
        let press = Press {
            wall_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            x: 20,
            y: 30,
        };
        events
            .send(binding(
                "page-0",
                json!({"k":"tap","at":press.wall_ms - 3,"editable":true}),
            ))
            .unwrap();
        let mut foreign = binding(
            "page-0",
            json!({"k":"tap","at":press.wall_ms,"editable":true}),
        );
        foreign["params"]["executionContextId"] = json!(999);
        events.send(foreign).unwrap();
        let start = Instant::now();
        assert!(matches!(
            paste::run(&mut session, "fixture", Some(press)),
            Err(PasteError::Untouched("tap not observed"))
        ));
        assert!(start.elapsed() >= Duration::from_millis(150));
        assert!(start.elapsed() < Duration::from_millis(700));
        drop(session);
        let log = server.join().unwrap();
        assert!(calls(&log, "prepare").is_empty());
        assert_eq!(count(&log, "Input.insertText"), 0);
    }

    #[test]
    fn tests_that_warm_operations_reset_the_original_connection_deadline() {
        let (port, _, server) = peer(Browser::default());
        let mut session =
            Session::open_at(port, Instant::now() + Duration::from_millis(250)).unwrap();
        thread::sleep(Duration::from_millis(260));
        session.set_deadline(Instant::now() + Duration::from_secs(1));
        assert_eq!(
            paste::run(&mut session, "fixture", None).unwrap(),
            PasteOutcome::Pasted
        );
        session.set_deadline(Instant::now() + Duration::from_secs(1));
        assert_eq!(
            paste::run(&mut session, "fixture", None).unwrap(),
            PasteOutcome::Pasted
        );
        drop(session);
        let log = server.join().unwrap();
        assert_eq!(count(&log, "Target.attachToTarget"), 1);
        assert_eq!(count(&log, "Page.addScriptToEvaluateOnNewDocument"), 1);
        assert_eq!(count(&log, "Input.insertText"), 2);
    }
    #[test]
    fn tests_that_no_field_cannot_claim_absence_after_a_body_handler_fires_input() {
        let (port, _, server) = peer(Browser {
            prepared: "nofield",
            input_on_prepare: true,
            changed: false,
            ..Browser::default()
        });
        let mut session = open(port);
        assert_eq!(
            paste::run(&mut session, "fixture", None).unwrap(),
            PasteOutcome::Unconfirmed
        );
        drop(session);
        assert_eq!(count(&server.join().unwrap(), "Input.insertText"), 0);
    }

    #[test]
    fn tests_that_recent_input_cannot_hide_behind_the_report_throttle_and_allow_retry() {
        let (port, _, server) = peer(Browser {
            changed: false,
            ..Browser::default()
        });
        let mut session = open(port);
        let target = session.resolve(None).unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        session.event(&binding("page-0", json!({"k":"input","at":now})));
        let result = session.attempt(&target, "fixture", Attempt::First).unwrap();
        assert!(matches!(
            paste::decide(&result),
            paste::Next::Done(PasteOutcome::Unconfirmed)
        ));
        drop(session);
        assert_eq!(count(&server.join().unwrap(), "Input.insertText"), 1);
    }

    #[test]
    fn tests_that_a_detached_frame_stays_unobserved_until_it_is_removed_or_reinstalled() {
        let (port, _, server) = peer(Browser {
            depth: 1,
            focused_depth: 0,
            changed: false,
            ..Browser::default()
        });
        let mut session = open(port);
        session.event(&json!({"method":"Target.detachedFromTarget","sessionId":"page-0","params":{"sessionId":"child-session"}}));
        assert!(!session.all_frames_observed());
        assert!(
            session
                .page
                .as_ref()
                .unwrap()
                .frames
                .keys()
                .any(|key| key.frame == "child")
        );
        session.event(&json!({"method":"Page.frameDetached","sessionId":"page-0","params":{"frameId":"child","reason":"remove"}}));
        assert!(session.all_frames_observed());
        drop(session);
        server.join().unwrap();
    }
    #[test]
    fn tests_that_tab_changes_reselect_on_discovery_events_and_before_each_paste() {
        for notify in [false, true] {
            let (port, events, server) = peer(Browser {
                tabs: 2,
                ..Browser::default()
            });
            let mut session = open(port);
            assert_eq!(
                paste::run(&mut session, "first", None).unwrap(),
                PasteOutcome::Pasted
            );
            events.send(json!({"testFront":1,"notify":notify})).unwrap();
            thread::sleep(Duration::from_millis(25));
            let deadline = Instant::now() + Duration::from_secs(1);
            if notify {
                session.pump(deadline).unwrap();
                thread::sleep(SELECTION_DEBOUNCE + Duration::from_millis(20));
                session.pump(deadline).unwrap();
                assert_eq!(session.page.as_ref().unwrap().target, "tab-1");
            }
            session.set_deadline(deadline);
            assert_eq!(
                paste::run(&mut session, "second", None).unwrap(),
                PasteOutcome::Pasted
            );
            drop(session);
            let log = server.join().unwrap();
            let inserted: Vec<_> = log
                .iter()
                .filter(|r| r["method"] == "Input.insertText")
                .collect();
            assert_eq!(inserted.len(), 2);
            assert_eq!(inserted[0]["sessionId"], "page-0");
            assert_eq!(inserted[1]["sessionId"], "page-1");
            assert_eq!(count(&log, "Target.attachToTarget"), 2);
        }
    }

    #[test]
    fn tests_that_close_stops_the_observers_and_removes_registered_scripts() {
        let (port, _, server) = peer(Browser {
            depth: 2,
            focused_depth: 2,
            ..Browser::default()
        });
        open(port).close();
        let log = server.join().unwrap();
        assert_eq!(calls(&log, "stop").len(), 3);
        assert_eq!(count(&log, "Page.removeScriptToEvaluateOnNewDocument"), 3);
        assert_eq!(count(&log, "Runtime.removeBinding"), 3);
    }
    #[test]
    fn tests_that_frame_swaps_can_reinstall_a_child_in_the_parent_process() {
        let (port, _, server) = peer(Browser {
            depth: 1,
            ..Browser::default()
        });
        let mut session = open(port);
        session.event(&json!({"method":"Target.detachedFromTarget","sessionId":"page-0","params":{"sessionId":"child-session"}}));
        session.event(&json!({"method":"Runtime.executionContextCreated","sessionId":"page-0","params":{"context":{"id":100,"uniqueId":"local-child","name":WORLD,"auxData":{"frameId":"child"}}}}));
        session.event(&json!({"method":"Runtime.bindingCalled","sessionId":"page-0","params":{"name":BINDING,"executionContextId":100,"payload":json!({"k":"focus","at":2000,"editable":false,"inside":false}).to_string()}}));
        assert!(session.all_frames_observed());
        assert_eq!(
            session
                .page
                .as_ref()
                .unwrap()
                .frames
                .keys()
                .filter(|key| key.frame == "child")
                .count(),
            1
        );
        drop(session);
        server.join().unwrap();
    }

    #[test]
    fn tests_that_frame_swaps_from_an_old_parent_do_not_remove_the_new_child_owner() {
        let (port, _, server) = peer(Browser {
            depth: 1,
            focused_depth: 1,
            ..Browser::default()
        });
        let mut session = open(port);
        session.event(&json!({"method":"Page.frameDetached","sessionId":"page-0","params":{"frameId":"child","reason":"swap"}}));
        assert!(session.all_frames_observed());
        assert!(session.owns(&SessionId("child-session".into())));
        drop(session);
        server.join().unwrap();
    }
    #[test]
    fn tests_that_child_navigation_keeps_its_parent_for_field_mapping() {
        let (port, _, server) = peer(Browser {
            depth: 1,
            ..Browser::default()
        });
        let mut session = open(port);
        session.event(&json!({"method":"Page.frameNavigated","sessionId":"child-session","params":{"frame":{"id":"child"}}}));
        let child = FrameKey {
            session: SessionId("child-session".into()),
            frame: "child".into(),
        };
        assert_eq!(
            session.page.as_ref().unwrap().frames[&child]
                .parent
                .as_ref()
                .unwrap()
                .frame,
            "main-0"
        );
        drop(session);
        server.join().unwrap();
    }

    #[test]
    fn tests_that_a_new_local_context_supersedes_old_remote_frame_ownership() {
        let (port, _, server) = peer(Browser {
            depth: 1,
            ..Browser::default()
        });
        let mut session = open(port);
        session.event(&json!({"method":"Runtime.executionContextCreated","sessionId":"page-0","params":{"context":{"id":100,"uniqueId":"local-child","name":WORLD,"auxData":{"frameId":"child"}}}}));
        assert!(!session.owns(&SessionId("child-session".into())));
        drop(session);
        server.join().unwrap();
    }
    #[test]
    fn tests_that_field_mapping_refreshes_measured_zoom_and_resolves_nested_frame_offsets() {
        let (port, _, server) = peer(Browser {
            depth: 2,
            focused_depth: 2,
            ..Browser::default()
        });
        let mut session = open(port);
        let _ = session.fields();
        let page = session.page.as_ref().unwrap();
        let top = FrameKey {
            session: page.session.clone(),
            frame: page.root.clone(),
        };
        assert_eq!(page.frames[&top].boxes.as_ref().unwrap().vp.zoom, 2.);
        drop(session);
        let log = server.join().unwrap();
        assert_eq!(count(&log, "Page.getLayoutMetrics"), 1);
        assert_eq!(count(&log, "DOM.getFrameOwner"), 2);
        assert_eq!(count(&log, "Runtime.releaseObjectGroup"), 2);
    }
    #[test]
    fn tests_that_new_document_fields_exclude_the_previous_documents_child_frames() {
        for invalidation in [
            json!({"method":"Page.frameNavigated","params":{"frame":{"id":"main-0"}}}),
            json!({"method":"Runtime.executionContextDestroyed","params":{"executionContextId":10}}),
            json!({"method":"Runtime.executionContextsCleared","params":{}}),
        ] {
            let (port, _, server) = peer(Browser {
                depth: 2,
                ..Browser::default()
            });
            let mut session = open(port);
            assert_eq!(session.fields().unwrap().boxes().len(), 3);
            let mut invalidation = invalidation;
            invalidation["sessionId"] = json!("page-0");
            session.event(&invalidation);
            assert!(session.fields().is_none());
            session.event(&world("page-0"));
            session.event(&binding("page-0", json!({"k":"boxes","doc":11,
                "vp":{"vv":[0,0,390,844],"vv_scale":1,"inner":[390,844],"dpr":2,"screen":[390,844],"zoom":1},
                "boxes":[[30,40,100,30]],"focused":null})));
            let fields = session.fields().unwrap();
            drop(session);
            server.join().unwrap();
            assert_eq!(
                fields.boxes().len(),
                1,
                "old descendant boxes survived document loss"
            );
            assert_eq!(fields.boxes()[0].x, 60);
        }
    }

    #[test]
    fn tests_that_fields_are_discarded_when_navigation_arrives_during_composition() {
        let (port, _, server) = peer(Browser {
            navigate_during_fields: true,
            ..Browser::default()
        });
        let mut session = open(port);
        let stale = session.fields();
        let fresh = session.fields();
        drop(session);
        server.join().unwrap();
        assert!(stale.is_none(), "a report crossed document generations");
        assert_eq!(fresh.unwrap().boxes().len(), 1);
    }

    #[test]
    fn tests_that_root_fields_survive_an_iframe_attachment_during_composition() {
        let (port, _, server) = peer(Browser {
            events_during_fields: vec![
                json!({"method":"Target.attachedToTarget","sessionId":"page-0",
                "params":{"sessionId":"child-session","targetInfo":{"type":"iframe","targetId":"child"}}}),
            ],
            ..Browser::default()
        });
        let mut session = open(port);
        let fields = session.fields();
        drop(session);
        server.join().unwrap();
        let fields = fields.expect("an attaching iframe must not withhold root fields");
        assert_eq!(fields.boxes().len(), 1);
        assert_eq!(fields.boxes()[0].x, 20);
    }

    #[test]
    fn tests_that_child_navigation_only_removes_that_subtrees_fields() {
        let (port, _, server) = peer(Browser {
            depth: 2,
            events_during_fields: vec![
                json!({"method":"Page.frameNavigated","sessionId":"child-session",
                "params":{"frame":{"id":"child"}}}),
            ],
            ..Browser::default()
        });
        let mut session = open(port);
        let fields = session.fields();
        drop(session);
        server.join().unwrap();
        let fields = fields.expect("child navigation must not withhold root fields");
        assert_eq!(fields.boxes().len(), 1);
        assert_eq!(fields.boxes()[0].x, 20);
    }

    #[test]
    fn tests_that_unreported_children_do_not_delay_root_fields_and_join_immediately() {
        let (port, _, server) = peer(Browser {
            depth: 1,
            ..Browser::default()
        });
        let mut session = open(port);
        session
            .pump(Instant::now() + Duration::from_secs(1))
            .unwrap();
        session.event(
            &json!({"method":"Page.frameNavigated","sessionId":"child-session",
            "params":{"frame":{"id":"child"}}}),
        );
        let root = session.fields().expect("root fields before child reports");
        assert_eq!(root.boxes().len(), 1);
        session.event(&world("child-session"));
        session.event(&binding("child-session", json!({"k":"boxes","doc":21,
            "vp":{"vv":[0,0,300,400],"vv_scale":1,"inner":[300,400],"dpr":2,"screen":[390,844],"zoom":1},
            "boxes":[[30,40,100,30]],"focused":null})));
        let changes = session
            .pump(Instant::now() + Duration::from_secs(1))
            .unwrap();
        assert!(changes.fields && changes.immediate);
        let joined = session.fields().expect("child fields after first report");
        assert_eq!(joined.boxes().len(), 2);
        assert_eq!(joined.boxes()[0], root.boxes()[0]);
        assert_eq!(joined.boxes()[1].x, 80);
        assert_eq!(joined.boxes()[1].y, 120);
        drop(session);
        let log = server.join().unwrap();
        assert_eq!(
            count(&log, "DOM.getFrameOwner"),
            1,
            "only a reported child may make root fields wait for geometry"
        );
    }

    #[test]
    fn tests_that_child_navigation_during_geometry_never_reuses_old_subtree_offsets() {
        let mut next_world = world("child-session");
        next_world["params"]["context"]["uniqueId"] = json!("next-child-world");
        let (port, _, server) = peer(Browser {
            depth: 2,
            events_during_fields: vec![
                json!({"method":"Page.frameNavigated","sessionId":"child-session",
                    "params":{"frame":{"id":"child"}}}),
                next_world,
                binding("child-session", box_report("child-session", false)),
            ],
            field_event_method: "Runtime.releaseObjectGroup",
            ..Browser::default()
        });
        let mut session = open(port);
        let during = session.fields().expect("unchanged root stays available");
        let after = session
            .fields()
            .expect("new child joins on its next collection");
        drop(session);
        server.join().unwrap();
        assert_eq!(during.boxes().len(), 1);
        assert_eq!(after.boxes().len(), 2);
        assert_eq!(after.boxes()[0], during.boxes()[0]);
    }

    #[test]
    fn tests_that_root_geometry_updates_push_without_another_throttle() {
        let (port, _, server) = peer(Browser::default());
        let mut session = open(port);
        session
            .pump(Instant::now() + Duration::from_secs(1))
            .unwrap();
        let mut moved = box_report("page-0", true);
        moved["boxes"][0][0] = json!(50);
        session.event(&binding("page-0", moved));
        let changes = session
            .pump(Instant::now() + Duration::from_secs(1))
            .unwrap();
        let fields = session.fields().unwrap();
        drop(session);
        server.join().unwrap();
        assert_eq!(fields.boxes()[0].x, 100);
        assert!(
            changes.fields && changes.immediate,
            "root reports are already throttled in the page"
        );
    }

    #[test]
    fn tests_that_bfcache_restore_preserves_live_worlds_and_fields() {
        for restored in ["page-0", "child-session"] {
            let (port, _, server) = peer(Browser {
                depth: 2,
                ..Browser::default()
            });
            let mut session = open(port);
            let before = session.fields().unwrap();
            session.event(&json!({"method":"Page.frameNavigated","sessionId":restored,
                "params":{"type":"BackForwardCacheRestore","frame":{"id":frame(restored)}}}));
            let after = session.fields();
            let observed = session.all_frames_observed();
            drop(session);
            server.join().unwrap();
            assert!(observed, "a restored document keeps its agent world");
            assert_eq!(after.as_ref(), Some(&before));
        }
    }

    #[test]
    fn tests_that_non_page_target_events_do_not_schedule_selection() {
        let (port, _, server) = peer(Browser::default());
        let mut session = open(port);
        for kind in ["iframe", "worker", "service_worker"] {
            session.event(&json!({"method":"Target.targetCreated","params":{"targetInfo":{"type":kind,"targetId":"other"}}}));
            session.event(&json!({"method":"Target.targetInfoChanged","params":{"targetInfo":{"type":kind,"targetId":"other"}}}));
            session
                .event(&json!({"method":"Target.targetDestroyed","params":{"targetId":"other"}}));
        }
        session
            .pump(Instant::now() + Duration::from_millis(100))
            .unwrap();
        drop(session);
        assert_eq!(count(&server.join().unwrap(), "Target.getTargets"), 1);
    }

    #[test]
    fn tests_that_page_target_bursts_are_debounced_without_losing_the_connection() {
        let (port, _, server) = peer(Browser::default());
        let mut session = open(port);
        for _ in 0..100 {
            session.event(&json!({"method":"Target.targetInfoChanged","params":{"targetInfo":{"type":"page","targetId":"tab-0"}}}));
            session
                .pump(Instant::now() + Duration::from_millis(100))
                .unwrap();
        }
        thread::sleep(Duration::from_millis(80));
        session
            .pump(Instant::now() + Duration::from_millis(100))
            .unwrap();
        drop(session);
        let log = server.join().unwrap();
        assert_eq!(count(&log, "Target.getTargets"), 2);
        assert_eq!(count(&log, "Target.attachToTarget"), 1);
    }

    #[test]
    fn tests_that_a_later_old_browser_chrome_press_blocks_refocusing_a_page_tap() {
        let (port, _, server) = peer(Browser {
            depth: 1,
            editable: false,
            ..Browser::default()
        });
        let mut session = open(port);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        session.event(&binding(
            "child-session",
            json!({"k":"tap","at":now-2000,"editable":true}),
        ));
        let target = session
            .resolve(Some(Press {
                wall_ms: now - 1500,
                x: 3,
                y: 4,
            }))
            .unwrap();
        assert!(!target.refocus);
        assert_eq!(target.frame.session.0, "page-0");
        drop(session);
        server.join().unwrap();
    }

    #[test]
    fn tests_that_an_editable_focus_in_any_frame_prevents_refocusing_another_frame() {
        let (port, _, server) = peer(Browser {
            depth: 1,
            focused_depth: 0,
            ..Browser::default()
        });
        let mut session = open(port);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        session.event(&binding(
            "child-session",
            json!({"k":"tap","at":now-2000,"editable":true}),
        ));
        let target = session
            .resolve(Some(Press {
                wall_ms: now - 2000,
                x: 3,
                y: 4,
            }))
            .unwrap();
        assert!(!target.refocus);
        assert_eq!(target.frame.session.0, "page-0");
        drop(session);
        server.join().unwrap();
    }

    #[test]
    fn tests_that_refocusing_requires_the_current_owners_matching_last_press() {
        for press_delta in [None, Some(0)] {
            let (port, _, server) = peer(Browser {
                depth: 1,
                editable: false,
                ..Browser::default()
            });
            let mut session = open(port);
            let at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64
                - 2000;
            session.event(&binding(
                "child-session",
                json!({"k":"tap","at":at,"editable":true}),
            ));
            let press = press_delta.map(|delta| Press {
                wall_ms: at + delta,
                x: 3,
                y: 4,
            });
            let target = session.resolve(press).unwrap();
            assert_eq!(target.refocus, press.is_some());
            drop(session);
            server.join().unwrap();
        }
    }
    #[test]
    fn tests_that_incomplete_installs_retry_without_reopening_or_registering_duplicate_scripts() {
        for method in ["Runtime.addBinding", "Target.setAutoAttach"] {
            let (port, _, server) = peer(Browser {
                depth: 1,
                focused_depth: 1,
                fail_install: Some(method),
                ..Browser::default()
            });
            let mut session = open(port);
            let until = Instant::now() + Duration::from_secs(1);
            while !session.all_frames_observed() && Instant::now() < until {
                session
                    .pump(Instant::now() + Duration::from_millis(100))
                    .unwrap();
                thread::sleep(Duration::from_millis(20));
            }
            assert!(
                session.all_frames_observed(),
                "installation never recovered"
            );
            session.set_deadline(Instant::now() + Duration::from_secs(1));
            assert_eq!(
                paste::run(&mut session, "fixture", None).unwrap(),
                PasteOutcome::Pasted
            );
            drop(session);
            let log = server.join().unwrap();
            assert_eq!(count(&log, "Target.attachToTarget"), 1);
            assert_eq!(
                log.iter()
                    .filter(|r| r["sessionId"] == "child-session"
                        && r["method"] == "Page.addScriptToEvaluateOnNewDocument")
                    .count(),
                1
            );
        }
    }

    #[test]
    fn tests_that_slow_selection_keeps_the_session_and_retries_with_cached_contexts() {
        let (port, _, server) = peer(Browser {
            tabs: 3,
            focus_delay: Duration::from_millis(45),
            ..Browser::default()
        });
        let mut session = open(port);
        session.event(&binding(
            "page-0",
            json!({"k":"tap","at":123,"editable":true}),
        ));
        session.event(&json!({"method":"Target.targetInfoChanged","params":{"targetInfo":{"type":"page","targetId":"tab-0"}}}));
        thread::sleep(Duration::from_millis(80));
        session
            .pump(Instant::now() + Duration::from_millis(10))
            .unwrap();
        assert!(session.healthy());
        assert_eq!(
            session
                .page
                .as_ref()
                .unwrap()
                .frames
                .values()
                .filter_map(|f| f.tap.as_ref())
                .next()
                .unwrap()
                .at,
            123
        );
        let until = Instant::now() + Duration::from_secs(3);
        while session.selected_dirty && Instant::now() < until {
            session
                .pump(Instant::now() + Duration::from_millis(100))
                .unwrap();
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !session.selected_dirty,
            "selection never recovered within actor budgets"
        );
        drop(session);
        let log = server.join().unwrap();
        assert_eq!(count(&log, "Target.attachToTarget"), 3);
        assert!(count(&log, "Page.createIsolatedWorld") <= 4);
    }

    #[test]
    fn tests_that_event_floods_yield_before_consuming_the_paste_deadline() {
        let (port, events, server) = peer(Browser::default());
        let mut session = open(port);
        session
            .pump(Instant::now() + Duration::from_millis(100))
            .unwrap();
        events.send(json!({"testBurst":2048})).unwrap();
        thread::sleep(Duration::from_millis(30));
        session
            .drain_events(Instant::now() + Duration::from_millis(500))
            .unwrap();
        assert!(
            session.input_serial < 2048,
            "one drain monopolized the event stream"
        );
        assert!(session.next_wake().is_some());
        let until = Instant::now() + Duration::from_secs(2);
        while session.input_serial < 2048 && Instant::now() < until {
            session.drain_events(until).unwrap();
            thread::yield_now();
        }
        assert_eq!(session.input_serial, 2048);
        drop(session);
        server.join().unwrap();
    }
    #[test]
    fn tests_that_slow_installs_resume_across_actor_pump_budgets() {
        let (port, _, server) = peer(Browser {
            depth: 1,
            focused_depth: 1,
            install_delay: Duration::from_millis(30),
            ..Browser::default()
        });
        let mut session =
            Session::open_at(port, Instant::now() + Duration::from_millis(100)).unwrap();
        let until = Instant::now() + Duration::from_secs(3);
        while !session.all_frames_observed() && Instant::now() < until {
            session
                .pump(Instant::now() + Duration::from_millis(100))
                .unwrap();
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            session.all_frames_observed(),
            "installation never recovered within actor budgets"
        );
        drop(session);
        let log = server.join().unwrap();
        assert_eq!(count(&log, "Target.attachToTarget"), 1);
    }

    #[test]
    fn tests_that_unresolved_selection_stops_publishing_the_old_tabs_fields() {
        let (port, _, server) = peer(Browser::default());
        let mut session = open(port);
        session.event(&json!({"method":"Target.targetInfoChanged","params":{"targetInfo":{"type":"page","targetId":"tab-0"}}}));
        assert!(session.fields().is_none());
        session
            .ensure_selected(Instant::now() + Duration::from_secs(1))
            .unwrap();
        assert!(session.fields().is_some());
        drop(session);
        server.join().unwrap();
    }
    #[test]
    fn tests_that_slow_native_tab_checks_resume_across_actor_pump_budgets() {
        let (port, _, server) = peer(Browser {
            tabs: 2,
            native_delay: Duration::from_millis(45),
            ..Browser::default()
        });
        let mut session = open(port);
        session.event(&json!({"method":"Target.targetInfoChanged","params":{"targetInfo":{"type":"page","targetId":"tab-0"}}}));
        let until = Instant::now() + Duration::from_secs(3);
        while session.selected_dirty && Instant::now() < until {
            session
                .pump(Instant::now() + Duration::from_millis(100))
                .unwrap();
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !session.selected_dirty,
            "native selection never recovered within actor budgets"
        );
        drop(session);
        server.join().unwrap();
    }

    #[test]
    fn tests_that_late_script_registration_replies_do_not_reinstall_the_agent() {
        let (port, _, server) = peer(Browser {
            depth: 1,
            focused_depth: 1,
            script_reply_delay: Duration::from_millis(150),
            ..Browser::default()
        });
        let mut session =
            Session::open_at(port, Instant::now() + Duration::from_millis(100)).unwrap();
        let until = Instant::now() + Duration::from_secs(3);
        while !session.all_frames_observed() && Instant::now() < until {
            session
                .pump(Instant::now() + Duration::from_millis(100))
                .unwrap();
            thread::sleep(Duration::from_millis(20));
        }
        assert!(session.all_frames_observed());
        drop(session);
        assert_eq!(
            count(
                &server.join().unwrap(),
                "Page.addScriptToEvaluateOnNewDocument"
            ),
            2
        );
    }
    #[test]
    fn tests_that_target_events_during_discovery_invalidate_a_resumed_scan() {
        let (port, _, server) = peer(Browser {
            tabs: 2,
            focus_delay: Duration::from_millis(45),
            create_during_discovery: true,
            ..Browser::default()
        });
        let mut session =
            Session::open_at(port, Instant::now() + Duration::from_millis(100)).unwrap();
        let until = Instant::now() + Duration::from_secs(3);
        while session.selected_dirty && Instant::now() < until {
            session
                .pump(Instant::now() + Duration::from_millis(100))
                .unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!session.selected_dirty);
        assert!(session.candidates.contains_key("tab-2"));
        drop(session);
        assert!(count(&server.join().unwrap(), "Target.getTargets") >= 2);
    }
    #[test]
    fn tests_that_persistent_selection_failures_back_off_cap_and_reset_after_success() {
        let ambiguous = Arc::new(AtomicBool::new(false));
        let (port, _, server) = peer(Browser {
            tabs: 2,
            ambiguous_windows: Some(ambiguous.clone()),
            ..Browser::default()
        });
        let mut session = open(port);
        ambiguous.store(true, Ordering::Release);
        for ms in [250, 500, 1000, 2000, 4000, 5000, 5000] {
            let start = Instant::now();
            assert_eq!(
                session.ensure_selected(start + Duration::from_secs(1)),
                Err("no unique front window")
            );
            let end = Instant::now();
            let retry = session.selection_at;
            assert!(retry >= start + Duration::from_millis(ms));
            assert!(retry <= end + Duration::from_millis(ms));
            session
                .pump(Instant::now() + Duration::from_millis(100))
                .unwrap();
            assert_eq!(session.next_wake(), Some(retry));
        }
        ambiguous.store(false, Ordering::Release);
        session
            .ensure_selected(Instant::now() + Duration::from_secs(1))
            .unwrap();
        ambiguous.store(true, Ordering::Release);
        let start = Instant::now();
        assert!(
            session
                .ensure_selected(start + Duration::from_secs(1))
                .is_err()
        );
        let end = Instant::now();
        assert!(session.selection_at >= start + METADATA_RETRY);
        assert!(session.selection_at <= end + METADATA_RETRY);
        assert!(session.healthy());
        drop(session);
        assert_eq!(count(&server.join().unwrap(), "Target.getTargets"), 10);
    }

    #[test]
    fn tests_that_persistent_install_failures_back_off_and_recover() {
        let failing = Arc::new(AtomicBool::new(true));
        let (port, _, server) = peer(Browser {
            depth: 1,
            fail_install: Some("Runtime.addBinding"),
            persistent_install_failure: Some(failing.clone()),
            ..Browser::default()
        });
        let mut session = open(port);
        for ms in [500, 1000] {
            let start = Instant::now();
            session.set_deadline(start + Duration::from_secs(1));
            assert!(session.install_pending().is_err());
            let end = Instant::now();
            let retry = session.install_at;
            assert!(retry >= start + Duration::from_millis(ms));
            assert!(retry <= end + Duration::from_millis(ms));
            session
                .pump(Instant::now() + Duration::from_millis(100))
                .unwrap();
            assert_eq!(session.next_wake(), Some(retry));
        }
        failing.store(false, Ordering::Release);
        session.set_deadline(Instant::now() + Duration::from_secs(1));
        session.install_pending().unwrap();
        assert!(session.all_frames_observed());
        assert!(session.healthy());
        drop(session);
        server.join().unwrap();
    }
    #[test]
    fn tests_that_a_deadline_expiring_while_serializing_a_request_keeps_the_socket_healthy() {
        let (port, _, server) = peer(Browser::default());
        let mut session = open(port);
        let params = json!({"padding":"x".repeat(2 * 1024 * 1024)});
        session.set_deadline(Instant::now() + Duration::from_millis(5));
        assert!(matches!(
            session.request("Runtime.enable", params, None, CALL_WAIT),
            Err(CallError::Uncertain)
        ));
        assert!(session.healthy());
        session.set_deadline(Instant::now() + Duration::from_secs(1));
        session
            .command("Runtime.evaluate", json!({"expression":"0"}), None)
            .unwrap();
        drop(session);
        assert_eq!(count(&server.join().unwrap(), "Runtime.enable"), 2);
    }
}

#[cfg(test)]
mod live_tests {
    use super::super::{agent::PasteOutcome, paste};
    use super::*;

    // Run only in a disposable Neo with the cross-site agent fixture already focused.
    fn observed_session() -> Session {
        let until = Instant::now() + Duration::from_secs(3);
        let mut session = Session::open(until).unwrap();
        while Instant::now() < until {
            session
                .pump(Instant::now() + Duration::from_millis(100))
                .unwrap();
            if session.all_frames_observed() {
                session.set_deadline(Instant::now() + Duration::from_millis(100));
                if let Some(fields) = session.fields()
                    && fields.focused().is_some()
                {
                    assert!(!fields.boxes().is_empty());
                    assert!(!session.page.as_ref().unwrap().oopifs.is_empty());
                    return session;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("fixture frames or focused field boxes were not observed");
    }

    #[test]
    #[ignore = "requires disposable Neo on port 9110 with the cross-site #plain fixture focused"]
    fn tests_that_live_neo_pastes_once_in_a_cross_process_frame() {
        let mut session = observed_session();
        session.set_deadline(Instant::now() + Duration::from_millis(1200));
        assert_eq!(
            paste::run(&mut session, "Z", None).unwrap(),
            PasteOutcome::Pasted
        );
        session.close();
    }

    #[test]
    #[ignore = "requires disposable Neo on port 9110 with the cross-site #blocker fixture focused"]
    fn tests_that_live_neo_respects_a_page_paste_refusal() {
        let mut session = observed_session();
        session.set_deadline(Instant::now() + Duration::from_millis(1200));
        assert_eq!(
            paste::run(&mut session, "Z", None).unwrap(),
            PasteOutcome::Refused
        );
        session.close();
    }
}
