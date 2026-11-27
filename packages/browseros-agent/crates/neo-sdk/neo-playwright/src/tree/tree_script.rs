//! The scripted tab that `TabTree`'s and `ChunkWorld`'s tests share: a fake CDP connection
//! whose world installs each get their own context and global, whose captures answer queued APC
//! replies, and whose hydration answers (`DOM.resolveNode`, and the probe's `resolve`, `stage`
//! and `describePins`) each test configures; the batched `pinned` reads answer from the pins the
//! probe holds and where their elements are. Plus the encoders for those APC replies.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use browseros_core::SessionId;
use neo_snapshot_apc::proto::AnnotatedPageContent;
use neo_snapshot_apc::tree::{DocToken, DomNodeId, NodeKey};
use prost::Message as _;
use serde_json::{Value, json};

use crate::probe::PROBE_PROTOCOL;
use crate::test_support::{ProbeAnswer, SCRIPTED_CHILD, SCRIPTED_MAIN, TabScript, TestResult};
use crate::world::Pin;

/// The condition a batched `pinned` read's function applies when it answers only elements of
/// the world's own document (`probe::pinned_nodes_in_document`), as the page would run it.
const IN_DOCUMENT: &str = "node.isConnected && node.ownerDocument === document";

fn encode_varint(mut value: u64, out: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn encode_varint_field(tag: u32, value: u64, out: &mut Vec<u8>) {
    encode_varint(u64::from(tag) << 3, out);
    encode_varint(value, out);
}

fn encode_len_delimited(tag: u32, payload: &[u8], out: &mut Vec<u8>) {
    encode_varint((u64::from(tag) << 3) | 2, out);
    encode_varint(payload.len() as u64, out);
    out.extend_from_slice(payload);
}

fn encode_frame_data(doc_token: &str, url: Option<&str>, frame_token: Option<&str>) -> Vec<u8> {
    let mut frame = Vec::new();
    let mut doc_id = Vec::new();
    encode_len_delimited(1, doc_token.as_bytes(), &mut doc_id);
    encode_len_delimited(3, &doc_id, &mut frame);
    if let Some(url) = url {
        encode_len_delimited(4, url.as_bytes(), &mut frame);
    }
    if let Some(token) = frame_token {
        let mut browseros = Vec::new();
        encode_len_delimited(1, token.as_bytes(), &mut browseros);
        encode_len_delimited(1000, &browseros, &mut frame);
    }
    frame
}

fn encode_leaf_node(dom_id: i32) -> Vec<u8> {
    let mut attrs = Vec::new();
    encode_varint_field(2, u64::from(dom_id.cast_unsigned()), &mut attrs);
    encode_varint_field(3, 24, &mut attrs); // FormControl
    let mut node = Vec::new();
    encode_len_delimited(2, &attrs, &mut node);
    node
}

/// A `<button>` named `label` with a box: the ai-mode emitter gives it a ref.
pub(super) fn encode_button_node(dom_id: i32, label: &str) -> Vec<u8> {
    let mut rect = Vec::new();
    encode_varint_field(3, 80, &mut rect); // width
    encode_varint_field(4, 20, &mut rect); // height
    let mut geometry = Vec::new();
    encode_len_delimited(1, &rect, &mut geometry); // outer_bounding_box
    let mut browseros = Vec::new();
    encode_len_delimited(1, b"button", &mut browseros); // tag
    encode_len_delimited(11, label.as_bytes(), &mut browseros); // aria_label
    let mut attrs = Vec::new();
    encode_varint_field(2, u64::from(dom_id.cast_unsigned()), &mut attrs);
    encode_varint_field(3, 2, &mut attrs); // Container
    encode_len_delimited(4, &geometry, &mut attrs);
    encode_len_delimited(1000, &browseros, &mut attrs);
    let mut node = Vec::new();
    encode_len_delimited(2, &attrs, &mut node);
    node
}

pub(super) struct ChildFrameSpec<'a> {
    pub(super) iframe_dom_id: i32,
    pub(super) doc_token: &'a str,
    pub(super) frame_token: &'a str,
    pub(super) url: Option<&'a str>,
    pub(super) child_dom_ids: &'a [i32],
}

pub(super) fn encode_apc_reply(
    main_doc_token: &str,
    main_url: Option<&str>,
    main_dom_ids: &[i32],
    child: Option<&ChildFrameSpec<'_>>,
    contract_version: Option<u32>,
) -> Value {
    let main_nodes: Vec<Vec<u8>> = main_dom_ids
        .iter()
        .map(|&dom_id| encode_leaf_node(dom_id))
        .collect();
    encode_apc_reply_nodes(
        main_doc_token,
        main_url,
        &main_nodes,
        child,
        contract_version,
    )
}

pub(super) fn encode_apc_reply_nodes(
    main_doc_token: &str,
    main_url: Option<&str>,
    main_nodes: &[Vec<u8>],
    child: Option<&ChildFrameSpec<'_>>,
    contract_version: Option<u32>,
) -> Value {
    let mut root_node = Vec::new();
    for node in main_nodes {
        encode_len_delimited(1, node, &mut root_node);
    }
    if let Some(child) = child {
        let mut iframe_node = Vec::new();
        for &dom_id in child.child_dom_ids {
            let leaf = encode_leaf_node(dom_id);
            encode_len_delimited(1, &leaf, &mut iframe_node);
        }
        let child_frame_data =
            encode_frame_data(child.doc_token, child.url, Some(child.frame_token));
        let mut iframe_data = Vec::new();
        encode_len_delimited(3, &child_frame_data, &mut iframe_data);

        let mut iframe_attrs = Vec::new();
        encode_varint_field(
            2,
            u64::from(child.iframe_dom_id.cast_unsigned()),
            &mut iframe_attrs,
        );
        encode_varint_field(3, 3, &mut iframe_attrs); // Iframe
        encode_len_delimited(9, &iframe_data, &mut iframe_attrs);
        encode_len_delimited(2, &iframe_attrs, &mut iframe_node);
        encode_len_delimited(1, &iframe_node, &mut root_node);
    }

    let mut root_attrs = Vec::new();
    encode_varint_field(2, 1, &mut root_attrs);
    encode_varint_field(3, 1, &mut root_attrs); // Root
    encode_len_delimited(2, &root_attrs, &mut root_node);

    let main_frame_data = encode_frame_data(main_doc_token, main_url, None);

    let mut apc = Vec::new();
    encode_varint_field(1, 1, &mut apc); // Version10
    encode_len_delimited(2, &root_node, &mut apc);
    encode_len_delimited(3, &main_frame_data, &mut apc);
    if let Some(version) = contract_version {
        let mut browseros = Vec::new();
        encode_varint_field(1, u64::from(version), &mut browseros);
        encode_len_delimited(1000, &browseros, &mut apc);
    }
    json!({ "content": STANDARD.encode(apc) })
}

/// `apc` as the browser answers `Page.getAnnotatedPageContent`: its protobuf bytes, base64.
pub(super) fn apc_reply(apc: &AnnotatedPageContent) -> Value {
    json!({ "content": STANDARD.encode(apc.encode_to_vec()) })
}

pub(super) fn key(doc: &str, dom_id: i32) -> TestResult<NodeKey> {
    let token = DocToken::new(doc).ok_or("non-empty doc token")?;
    Ok(NodeKey::new(token, DomNodeId::new(dom_id)))
}

/// Programmable fake CDP script backing `ScriptedTab` for offline `TabTree`, `Hydrate` and
/// `ChunkWorld` tests.
///
/// Tracks distinct per-install execution context IDs and global object IDs, per-global stale
/// status (so `W_old` can go `Gone` after navigation while `W_new` succeeds), queued APC
/// replies, `DOM.resolveNode` behaviors, and probe replies.
pub(super) struct TreeScript {
    child: Mutex<ChildFrame>,
    apc_replies: Mutex<VecDeque<Value>>,
    default_apc: Mutex<Option<Value>>,
    /// Hook run right after an isolated world finishes booting, letting tests simulate a
    /// navigation occurring between design 6.2 case 3's world install and its recapture.
    after_world_install: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    /// Hook run once while a `Page.getAnnotatedPageContent` request is in flight (sent, not yet
    /// answered), letting tests act during the capture's round trip.
    during_apc: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    next_context: AtomicI64,
    context_to_global: Mutex<HashMap<i64, String>>,
    global_info: Mutex<HashMap<String, WorldStub>>,
    stale_globals: Mutex<HashMap<String, &'static str>>,
    resolve_node_error: Mutex<Option<&'static str>>,
    resolve_probe_error: Mutex<Option<&'static str>>,
    resolve_probe_reply: Mutex<Option<Value>>,
    /// The backend node id of the element under each pin the probe holds: every element a
    /// `resolve` pinned, and the pins a test hands the page ([`Self::hold_pin`]). A pin not
    /// here is one the probe evicted.
    held_pins: Mutex<HashMap<u32, i64>>,
    /// Held pins whose element the page moved out of the world's document: the probe still
    /// answers them by pin, and an in-document read answers null.
    moved_out: Mutex<HashSet<u32>>,
    next_pin: AtomicU32,
    rekey_first_main: AtomicBool,
    rekey_retired_seq: u32,
    resolve_node_calls: Mutex<Vec<Value>>,
    apc_calls: AtomicUsize,
    /// `DOM.resolveNode`'s protocol error for one backend node id, as for a node gone from its
    /// document.
    resolve_node_errors: Mutex<HashMap<i64, &'static str>>,
    /// How long `DOM.resolveNode` takes for one backend node id.
    resolve_node_delays: Mutex<HashMap<i64, Duration>>,
    /// `DOM.resolveNode` calls sent and not answered yet, and the most there ever were at once.
    resolves_in_flight: AtomicUsize,
    most_resolves_in_flight: AtomicUsize,
    /// While set, `describe` answers an error, so a capture proves no world alive and its
    /// documents reserve their seqs instead of binding.
    unproven_describe: AtomicBool,
    /// The probe's replies to `stage`, `describePins` and `privateMarks`, in the order the
    /// requests come.
    probe_replies: Mutex<VecDeque<Value>>,
    /// Every probe request a `Runtime.callFunctionOn` carried, boots aside, in order.
    probe_calls: Mutex<Vec<ProbeSent>>,
}

/// One probe request a `Runtime.callFunctionOn` carried: its op (`pinned` for the batch
/// `probe::pinned_nodes` sends, `pinnedInDocument` for `probe::pinned_nodes_in_document`'s) and
/// the call's `arguments`: the envelope, then the call's root and the nodes it hands over, when
/// it has any.
#[derive(Clone)]
pub(super) struct ProbeSent {
    pub(super) op: String,
    pub(super) arguments: Value,
}

/// Where the scripted tab's child frame, [`SCRIPTED_CHILD`], is.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ChildFrame {
    /// The tab has no child frame.
    Absent,
    /// In the main frame's process: the main frame's own tree lists it.
    InProcess,
    /// In a process of its own whose renderer does not answer: the browser lists it as an
    /// out-of-process frame, the main frame's tree does not, and attaching to it never answers,
    /// so a read of the page's frames leaves it out (`frame_tree::FrameTree::unread`).
    Hung,
}

#[derive(Clone)]
struct WorldStub {
    frame: String,
    seq: u32,
    url: String,
}

impl TreeScript {
    pub(super) fn new(default_apc: Value) -> Self {
        Self {
            child: Mutex::new(ChildFrame::Absent),
            apc_replies: Mutex::default(),
            default_apc: Mutex::new(Some(default_apc)),
            after_world_install: Mutex::default(),
            during_apc: Mutex::default(),
            next_context: AtomicI64::new(100),
            context_to_global: Mutex::default(),
            global_info: Mutex::default(),
            stale_globals: Mutex::default(),
            resolve_node_error: Mutex::default(),
            resolve_probe_error: Mutex::default(),
            resolve_probe_reply: Mutex::default(),
            held_pins: Mutex::default(),
            moved_out: Mutex::default(),
            next_pin: AtomicU32::new(1),
            rekey_first_main: AtomicBool::new(false),
            rekey_retired_seq: 0,
            resolve_node_calls: Mutex::default(),
            apc_calls: AtomicUsize::new(0),
            resolve_node_errors: Mutex::default(),
            resolve_node_delays: Mutex::default(),
            resolves_in_flight: AtomicUsize::new(0),
            most_resolves_in_flight: AtomicUsize::new(0),
            unproven_describe: AtomicBool::new(false),
            probe_replies: Mutex::default(),
            probe_calls: Mutex::default(),
        }
    }

    pub(super) fn with_child(self) -> Self {
        self.set_child(ChildFrame::InProcess);
        self
    }

    /// Where [`SCRIPTED_CHILD`] is from now on, as the page's frame tree read finds it.
    pub(super) fn set_child(&self, child: ChildFrame) {
        *self.child.lock().unwrap_or_else(PoisonError::into_inner) = child;
    }

    fn child_frame(&self) -> ChildFrame {
        *self.child.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `Target.getTargets`: no pre-existing targets at Playwright start, so the scripted tab
    /// models a newly opened tab starting at FrameSeq(0) (bare `e1`, then `f1e1`). A hung child
    /// is the one out-of-process frame listed.
    fn targets(&self) -> Value {
        let hung = (self.child_frame() == ChildFrame::Hung).then(|| {
            json!({ "targetId": SCRIPTED_CHILD, "type": "iframe",
                "url": "https://a.test/child", "parentFrameId": SCRIPTED_MAIN })
        });
        json!({ "targetInfos": hung.into_iter().collect::<Vec<_>>() })
    }

    pub(super) fn with_rekey_retired_seq(mut self, retired: u32) -> Self {
        self.rekey_first_main = AtomicBool::new(true);
        self.rekey_retired_seq = retired;
        self
    }

    pub(super) fn push_apc(&self, reply: Value) {
        self.apc_replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    pub(super) fn set_default_apc(&self, reply: Value) {
        *self
            .default_apc
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(reply);
    }

    pub(super) fn mark_global_stale(&self, global: &str, message: &'static str) {
        self.stale_globals
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(global.to_owned(), message);
    }

    pub(super) fn set_resolve_node_error(&self, message: Option<&'static str>) {
        *self
            .resolve_node_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = message;
    }

    pub(super) fn set_resolve_probe_error(&self, message: Option<&'static str>) {
        *self
            .resolve_probe_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = message;
    }

    pub(super) fn set_resolve_probe_reply(&self, reply: Option<Value>) {
        *self
            .resolve_probe_reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = reply;
    }

    /// The probe holds `pin` for the element `backend_node_id`, as for a pin a `stage` reply
    /// names.
    pub(super) fn hold_pin(&self, pin: Pin, backend_node_id: i32) {
        self.held_pins
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(pin.0, i64::from(backend_node_id));
    }

    /// The page moves `pin`'s element out of the world's document (into another document, or
    /// out of the page) while the probe still holds it.
    pub(super) fn move_out_of_document(&self, pin: Pin) {
        self.moved_out
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(pin.0);
    }

    pub(super) fn set_after_world_install(&self, hook: impl Fn() + Send + Sync + 'static) {
        *self
            .after_world_install
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Box::new(hook));
    }

    pub(super) fn set_during_apc(&self, hook: impl Fn() + Send + Sync + 'static) {
        *self
            .during_apc
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Box::new(hook));
    }

    pub(super) fn apc_count(&self) -> usize {
        self.apc_calls.load(Ordering::SeqCst)
    }

    pub(super) fn resolve_node_calls(&self) -> Vec<Value> {
        self.resolve_node_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// `DOM.resolveNode` of `backend_node_id` fails with `message` from now on.
    pub(super) fn fail_resolve_node(&self, backend_node_id: i32, message: &'static str) {
        self.resolve_node_errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(i64::from(backend_node_id), message);
    }

    /// `DOM.resolveNode` of `backend_node_id` answers only after `delay` on the tokio clock.
    pub(super) fn delay_resolve_node(&self, backend_node_id: i32, delay: Duration) {
        self.resolve_node_delays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(i64::from(backend_node_id), delay);
    }

    pub(super) fn most_resolves_in_flight(&self) -> usize {
        self.most_resolves_in_flight.load(Ordering::SeqCst)
    }

    pub(super) fn set_unproven_describe(&self, unproven: bool) {
        self.unproven_describe.store(unproven, Ordering::SeqCst);
    }

    /// Queues the probe's reply to the next `stage`, `describePins` or `privateMarks`.
    pub(super) fn push_probe_reply(&self, reply: Value) {
        self.probe_replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    pub(super) fn probe_calls(&self) -> Vec<ProbeSent> {
        self.probe_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record_probe(&self, op: &str, params: &Value) {
        self.probe_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ProbeSent {
                op: op.to_owned(),
                arguments: params["arguments"].clone(),
            });
    }

    fn resolve_answered(&self) {
        self.resolves_in_flight.fetch_sub(1, Ordering::SeqCst);
    }

    /// The protocol error `DOM.resolveNode` fails with: every node's, one node's, or a stale
    /// context's.
    fn resolve_node_error(&self, params: &Value) -> Option<&'static str> {
        if let Some(error) = *self
            .resolve_node_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
        {
            return Some(error);
        }
        let backend_id = params["backendNodeId"].as_i64().unwrap_or_default();
        if let Some(error) = self
            .resolve_node_errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&backend_id)
            .copied()
        {
            return Some(error);
        }
        let ctx = params["executionContextId"].as_i64()?;
        let global = self
            .context_to_global
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&ctx)
            .cloned()?;
        self.stale_globals
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&global)
            .copied()
    }

    fn call_function_on_reply(&self, params: &Value) -> Option<Value> {
        let object_id = params["objectId"].as_str().unwrap_or_default();
        let first = &params["arguments"][0]["value"];

        // A batched `pinned` read (an array of envelopes): mark_private_pins, a chunk run's keys
        // and a hydration's document check.
        if let Some(calls) = first.as_array() {
            let in_document = params["functionDeclaration"]
                .as_str()
                .is_some_and(|function| function.contains(IN_DOCUMENT));
            let op = if in_document {
                "pinnedInDocument"
            } else {
                "pinned"
            };
            self.record_probe(op, params);
            return Some(self.pinned_batch(calls, in_document));
        }

        if let Some(seq) = first["frameSeq"].as_u64() {
            return Some(self.boot_reply(object_id, u32::try_from(seq).unwrap_or(0)));
        }

        let request = first.get("request")?;
        let op = request["op"].as_str()?;
        self.record_probe(op, params);
        let reply = match op {
            "describe" if self.unproven_describe.load(Ordering::SeqCst) => json!({
                "kind": "error",
                "class": "internal",
                "message": "the scripted world proves nothing",
            }),
            "stage" | "describePins" | "privateMarks" => self
                .probe_replies
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()?,
            "describe" => {
                let stub = self
                    .global_info
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(object_id)
                    .cloned();
                let (seq, url) = stub.map_or_else(
                    || (0, "https://a.test/".to_owned()),
                    |stub| (stub.seq, stub.url),
                );
                json!({
                    "kind": "described",
                    "protocol": PROBE_PROTOCOL,
                    "seq": seq,
                    "url": url,
                })
            }
            "resolve" if request["want"] == "frame" => {
                let pin = self.next_pin.fetch_add(1, Ordering::SeqCst);
                json!({
                    "kind": "frame",
                    "pin": pin,
                    "key": format!("#{pin}"),
                    "visible": true,
                    "src": "https://a.test/child",
                })
            }
            "resolve" => {
                let root_obj = params["arguments"][1]["objectId"]
                    .as_str()
                    .unwrap_or("node");
                let reply = self
                    .resolve_probe_reply
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone()
                    .unwrap_or_else(|| {
                        let pin = self.next_pin.fetch_add(1, Ordering::SeqCst);
                        json!({
                            "kind": "element",
                            "pin": pin,
                            "preview": format!("<button data-obj=\"{root_obj}\">Go</button>"),
                        })
                    });
                if reply["kind"] == "element"
                    && let Some(pin) = reply["pin"]
                        .as_u64()
                        .and_then(|pin| u32::try_from(pin).ok())
                    && let Some(backend) = resolved_backend_id(root_obj)
                {
                    self.held_pins
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .insert(pin, backend);
                }
                reply
            }
            _ => return None,
        };
        Some(json!({
            "result": { "type": "string", "value": reply.to_string() }
        }))
    }

    /// The deep-serialized answer to a batched `pinned` read of `calls`: each held pin's
    /// element by backend id, null for an evicted pin, and with `in_document` null too for an
    /// element the page moved out of the world's document.
    fn pinned_batch(&self, calls: &[Value], in_document: bool) -> Value {
        let held = self
            .held_pins
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let moved_out = self
            .moved_out
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let nodes: Vec<Value> = calls
            .iter()
            .map(|call| {
                let pin = call["request"]["pin"]
                    .as_u64()
                    .and_then(|pin| u32::try_from(pin).ok());
                let element = pin
                    .filter(|pin| !(in_document && moved_out.contains(pin)))
                    .and_then(|pin| held.get(&pin));
                match element {
                    Some(id) => json!({ "type": "node", "value": { "backendNodeId": id } }),
                    None => json!({ "type": "null" }),
                }
            })
            .collect();
        json!({ "result": { "type": "object",
            "deepSerializedValue": { "type": "array", "value": nodes } } })
    }

    fn boot_reply(&self, object_id: &str, seq: u32) -> Value {
        let mut retired = None;
        let url = {
            let mut info = self
                .global_info
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(stub) = info.get_mut(object_id) {
                stub.seq = seq;
                if stub.frame == SCRIPTED_MAIN
                    && self.rekey_first_main.swap(false, Ordering::SeqCst)
                {
                    retired = Some(self.rekey_retired_seq);
                }
                stub.url.clone()
            } else {
                "https://a.test/".to_owned()
            }
        };
        let hook = self
            .after_world_install
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(hook) = hook {
            hook();
        }
        let mut reply = json!({
            "kind": "described",
            "protocol": PROBE_PROTOCOL,
            "seq": seq,
            "url": url,
        });
        if let Some(retired_seq) = retired {
            reply["retired"] = json!(retired_seq);
        }
        json!({ "result": { "type": "string", "value": reply.to_string() } })
    }
}

/// The backend node id in an object id this script's `DOM.resolveNode` hands out
/// (`node-{context}-{backend id}`).
fn resolved_backend_id(object_id: &str) -> Option<i64> {
    object_id
        .strip_prefix("node-")?
        .rsplit_once('-')?
        .1
        .parse()
        .ok()
}

impl TabScript for TreeScript {
    fn probe(&self, _frame: &str, _request: &Value) -> ProbeAnswer {
        // `Runtime.callFunctionOn` is handled directly in `cdp_reply` / `cdp_error` below so
        // we can inspect `params["objectId"]` per world install.
        ProbeAnswer::unexpected()
    }

    fn child(&self) -> bool {
        self.child_frame() == ChildFrame::InProcess
    }

    fn seen(&self, method: &str, _params: &Value, _session: Option<&SessionId>) {
        if method == "DOM.resolveNode" {
            let now = self.resolves_in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.most_resolves_in_flight
                .fetch_max(now, Ordering::SeqCst);
        }
    }

    fn delay(&self, method: &str, params: &Value) -> Duration {
        if method == "Target.attachToTarget"
            && params["targetId"] == SCRIPTED_CHILD
            && self.child_frame() == ChildFrame::Hung
        {
            return Duration::from_secs(60);
        }
        if method != "DOM.resolveNode" {
            return Duration::ZERO;
        }
        let backend_id = params["backendNodeId"].as_i64().unwrap_or_default();
        self.resolve_node_delays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&backend_id)
            .copied()
            .unwrap_or_default()
    }

    fn cdp_error(&self, method: &str, params: &Value) -> Option<&'static str> {
        match method {
            "DOM.resolveNode" => {
                let error = self.resolve_node_error(params);
                if error.is_some() {
                    self.resolve_answered();
                }
                error
            }
            "Runtime.callFunctionOn" => {
                if let Some(object_id) = params["objectId"].as_str()
                    && let Some(msg) = self
                        .stale_globals
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .get(object_id)
                        .copied()
                {
                    return Some(msg);
                }
                let op = params["arguments"][0]["value"]["request"]["op"].as_str();
                if op == Some("resolve") {
                    return *self
                        .resolve_probe_error
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner);
                }
                None
            }
            _ => None,
        }
    }

    fn cdp_reply(&self, method: &str, params: &Value) -> Option<Value> {
        match method {
            "Target.getTargets" => Some(self.targets()),
            "Page.getAnnotatedPageContent" => {
                self.apc_calls.fetch_add(1, Ordering::SeqCst);
                let hook = self
                    .during_apc
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
                if let Some(hook) = hook {
                    hook();
                }
                let queued = self
                    .apc_replies
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .pop_front();
                queued.or_else(|| {
                    self.default_apc
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .clone()
                })
            }
            "Page.createIsolatedWorld" => {
                let frame = params["frameId"]
                    .as_str()
                    .unwrap_or(SCRIPTED_MAIN)
                    .to_owned();
                let ctx = self.next_context.fetch_add(1, Ordering::SeqCst);
                let global = format!("g-{frame}-{ctx}");
                self.context_to_global
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(ctx, global.clone());
                let url = if frame == SCRIPTED_CHILD {
                    "https://a.test/child".to_owned()
                } else {
                    "https://a.test/".to_owned()
                };
                self.global_info
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(global, WorldStub { frame, seq: 0, url });
                Some(json!({ "executionContextId": ctx }))
            }
            "Runtime.evaluate" => {
                let ctx = params["contextId"].as_i64()?;
                let global = self
                    .context_to_global
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(&ctx)
                    .cloned()?;
                Some(json!({ "result": { "type": "object", "objectId": global } }))
            }
            "DOM.getFrameOwner" => {
                if params["frameId"].as_str() == Some(SCRIPTED_CHILD) {
                    Some(json!({ "backendNodeId": 500 }))
                } else {
                    None
                }
            }
            "DOM.resolveNode" => {
                self.resolve_answered();
                let backend_id = params["backendNodeId"].as_i64().unwrap_or_default();
                let ctx = params["executionContextId"].as_i64().unwrap_or_default();
                self.resolve_node_calls
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(params.clone());
                Some(json!({
                    "object": {
                        "type": "object",
                        "subtype": "node",
                        "objectId": format!("node-{ctx}-{backend_id}"),
                    }
                }))
            }
            "DOM.describeNode" => {
                let object_id = params["objectId"].as_str().unwrap_or_default();
                if object_id.ends_with("-500") {
                    return Some(json!({
                        "node": {
                            "backendNodeId": 500,
                            "nodeName": "IFRAME",
                            "frameId": SCRIPTED_CHILD,
                            "contentDocument": { "nodeId": 2, "backendNodeId": 2, "nodeName": "#document" },
                        }
                    }));
                }
                None
            }
            "Runtime.callFunctionOn" => self.call_function_on_reply(params),
            _ => None,
        }
    }
}
