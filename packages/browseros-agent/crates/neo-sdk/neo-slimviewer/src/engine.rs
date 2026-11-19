//! One loop owns capture, encoding and each viewer's pacing state. Readers and the X event
//! thread send it events, so the loop can update that state without a lock.
//!
//! - Nothing drawn → nothing done: the loop sleeps until Damage, an ack or a timer.
//! - The ack window keeps about one frame queued beyond the pipe, and the frame sent next is
//!   always the latest screen, so a slow link drops states at the source instead of queueing them.
//!   It allows for the ack jitter seen lately (up to 150 ms): on a phone that is mostly radio
//!   jitter, not a queue.
//! - Picture policy owns quality, size and sharpening. This loop supplies measured work,
//!   input, acknowledgments and CPU counters, then applies its decisions.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::net::{Shutdown, TcpStream};
use std::os::fd::AsRawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::time::{Duration, Instant};

use crate::capture::{Capture, GrabError};
use crate::encoder::{Encoder, Settings};
#[cfg(test)]
use crate::picture::link_full;
use crate::picture::{self, Picture, Plan, Scale, Work};
use crate::rfb::{Fields, Input, PasteOutcome};
use crate::wire;
use crate::yuv::I420;

pub enum Ev {
    Damage,
    Resized,
    XGone,
    Fatal(String),
    Joined(Viewer, SyncSender<bool>),
    Left(u64),
    Ack(u64, u32),
    Keyframe(u64),
    Input(u64, wire::FromPage),
    Clipboard(u64, String),
    PasteResult(u64, u32, PasteOutcome),
    #[allow(dead_code)]
    Fields(u64, Fields),
}

pub struct Config {
    pub encoder: Settings,
    pub max_fps: f32,
    pub sharpen_qp: u8,
    pub sharpen_after: Duration,
    /// How far the quality may drop below `encoder.crf` on a slow link.
    pub crf_range: f32,
    pub cpu_guard: bool,
    pub adaptive_size: bool,
    pub max_viewers: usize,
}

/// Reset a live owner's stalled window; spectators disconnect at this deadline.
const ACK_TIMEOUT: Duration = Duration::from_secs(3);
const OWNER_ACK_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_INFLIGHT: usize = 8;
const MAX_RESET_FRAMES: usize = MAX_INFLIGHT * 5;
/// The most ack jitter the window allows for; an ack later than that is a queue.
const MAX_ACK_JITTER: Duration = Duration::from_millis(150);
const MAX_RETIRING: usize = 8;

pub struct Viewer {
    id: u64,
    view_only: bool,
    connection: Arc<Connection>,
    outgoing: SyncSender<Arc<[u8]>>,
    write_failed: Arc<AtomicBool>,
    inflight: VecDeque<Sent>,
    reset_frames: VecDeque<Sent>,
    unacked_since: Option<Instant>,
    needs_key: bool,
    burst_grace: bool,
    rtt: AckDelays,
    acked_bytes: u64,
    delays_ms: Vec<u32>,
    probes: Vec<(usize, Duration, Duration)>,
}

pub(crate) struct Connection {
    stream: TcpStream,
    drained: AtomicBool,
}

impl Connection {
    pub(crate) fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            drained: AtomicBool::new(true),
        }
    }

    pub(crate) fn wait_for_input(&self) {
        self.drained.store(false, Ordering::Release);
    }

    pub(crate) fn input_drained(&self) {
        self.drained.store(true, Ordering::Release);
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if self.drained.load(Ordering::Acquire) {
            let _ = self.stream.shutdown(Shutdown::Both);
        } else {
            // A failed barrier must not look like successful handback. Linger-zero resets
            // TCP on the final close; shutting down Write here would send a premature FIN.
            let linger = libc::linger {
                l_onoff: 1,
                l_linger: 0,
            };
            unsafe {
                libc::setsockopt(
                    self.stream.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_LINGER,
                    &linger as *const _ as *const libc::c_void,
                    std::mem::size_of_val(&linger) as libc::socklen_t,
                );
            }
            let _ = self.stream.shutdown(Shutdown::Read);
        }
    }
}

struct Sent {
    seq: u32,
    at: Instant,
    bytes: usize,
    burst: bool,
}

impl Viewer {
    pub fn new(id: u64, view_only: bool, stream: TcpStream) -> io::Result<Viewer> {
        let mut writer = stream.try_clone()?;
        writer.set_write_timeout(Some(ACK_TIMEOUT))?;
        let (outgoing, pending) = mpsc::sync_channel::<Arc<[u8]>>(MAX_INFLIGHT);
        let write_failed = Arc::new(AtomicBool::new(false));
        let failed = write_failed.clone();
        std::thread::Builder::new()
            .name(format!("viewer-{id}-writer"))
            .spawn(move || {
                while let Ok(record) = pending.recv() {
                    if writer.write_all(&record).is_err() {
                        failed.store(true, Ordering::Relaxed);
                        break;
                    }
                }
            })?;
        Ok(Viewer {
            id,
            view_only,
            connection: Arc::new(Connection::new(stream)),
            outgoing,
            write_failed,
            inflight: VecDeque::new(),
            reset_frames: VecDeque::new(),
            unacked_since: None,
            needs_key: true,
            burst_grace: true,
            rtt: AckDelays::new(),
            acked_bytes: 0,
            delays_ms: Vec::new(),
            probes: Vec::new(),
        })
    }

    /// The window: free when nothing is in flight; otherwise while the oldest unacked frame is
    /// younger than the learned round trip plus the larger of one frame interval and the ack
    /// jitter seen lately (at most 150 ms). That admits the pipe's worth of frames on a long
    /// fast link and rides out jitter, but stops once a queue grows past it.
    fn open(&self, now: Instant, interval: Duration) -> bool {
        // An owner reset releases credit, not bytes already handed to TCP. Keep
        // its key/sharp barrier across resets. Spectators retain the RTT window
        // so a slower ACK does not skip P-frames and force global resync keys.
        if !self.view_only
            && self
                .inflight
                .iter()
                .chain(&self.reset_frames)
                .any(|frame| frame.burst)
        {
            return false;
        }
        window_open(
            self.inflight.len(),
            self.inflight
                .front()
                .map(|f| now.saturating_duration_since(f.at)),
            self.rtt.get(),
            self.rtt.slack(interval),
        )
    }

    fn link_blocked(&mut self, now: Instant, interval: Duration, capacity: Option<f32>) -> bool {
        if self.open(now, interval) {
            return false;
        }
        if let Some(until) = self.burst_until(capacity) {
            if now < until {
                return false;
            }
            self.burst_grace = false;
        }
        true
    }

    fn burst_until(&self, capacity: Option<f32>) -> Option<Instant> {
        let frame = self
            .inflight
            .front()
            .filter(|frame| frame.burst && self.burst_grace)?;
        let seconds = capacity
            .filter(|cap| cap.is_finite() && *cap > 0.0)
            .map_or(1.0, |cap| {
                frame.bytes as f64 * 8.0 / (f64::from(cap) * 1000.0)
            })
            .min(1.0);
        Some(frame.at + Duration::from_secs_f64(seconds))
    }

    fn window_deadline(
        &self,
        now: Instant,
        interval: Duration,
        capacity: Option<f32>,
        pending: bool,
    ) -> Option<Instant> {
        if self.open(now, interval) {
            return None;
        }
        let mut due = self
            .inflight
            .front()
            .map(|frame| frame.at + ACK_TIMEOUT)
            .or_else(|| self.ack_deadline())?;
        if pending && let Some(until) = self.burst_until(capacity) {
            due = due.min(until);
        }
        Some(due)
    }

    fn expired(&self, now: Instant) -> bool {
        self.inflight
            .front()
            .is_some_and(|f| now.saturating_duration_since(f.at) >= ACK_TIMEOUT)
    }

    fn ack_deadline(&self) -> Option<Instant> {
        if self.view_only {
            return None;
        }
        self.unacked_since.map(|since| since + OWNER_ACK_TIMEOUT)
    }

    fn reset_window(&mut self) {
        self.reset_frames.extend(self.inflight.drain(..));
        while self.reset_frames.len() > MAX_RESET_FRAMES {
            self.reset_frames.pop_front();
        }
        self.needs_key = true;
        self.burst_grace = true;
    }

    fn record_frame(&mut self, frame: Sent, full_return: bool) {
        if full_return {
            self.burst_grace = true;
        }
        self.unacked_since.get_or_insert(frame.at);
        self.inflight.push_back(frame);
    }

    fn queue(&self, record: Arc<[u8]>) -> bool {
        if self.write_failed.load(Ordering::Relaxed) {
            return false;
        }
        match self.outgoing.try_send(record) {
            Ok(()) => true,
            Err(mpsc::TrySendError::Full(_)) => false,
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.write_failed.store(true, Ordering::Relaxed);
                false
            }
        }
    }

    fn input(&self, input: &mut Input, record: wire::FromPage) -> io::Result<()> {
        self.connection.wait_for_input();
        match record {
            wire::FromPage::Pointer { x, y, buttons } => input.pointer(x, y, buttons),
            wire::FromPage::Key { sym, down } => input.key(sym, down),
            wire::FromPage::Paste(text) => input.paste(self.id, None, &text),
            wire::FromPage::PasteTx { id, text } => input.paste(self.id, Some(id), &text),
            wire::FromPage::Type(text) => input.type_text(&text),
            wire::FromPage::ClipboardWatch => input.watch_clipboard(self.id),
            wire::FromPage::FieldsWatch => input.watch_fields(self.id),
            wire::FromPage::Wheel { x, y, dx, dy } => input.wheel(x, y, dx, dy),
            _ => Ok(()),
        }
    }

    fn on_ack(&mut self, seq: u32, now: Instant) {
        // Only an actually sent frame can grant credit; duplicate/future ACKs grant none.
        if !self.inflight.iter().any(|f| f.seq == seq) {
            if let Some(index) = self.reset_frames.iter().position(|f| f.seq == seq) {
                self.acked_bytes += self
                    .reset_frames
                    .drain(..=index)
                    .map(|f| f.bytes as u64)
                    .sum::<u64>();
                // A late ACK counts delivery and liveness, but cannot grant new-window credit.
                self.unacked_since =
                    (!self.inflight.is_empty() || !self.reset_frames.is_empty()).then_some(now);
            }
            return;
        }
        self.acked_bytes += self
            .reset_frames
            .drain(..)
            .map(|f| f.bytes as u64)
            .sum::<u64>();
        while let Some(f) = self.inflight.front() {
            if seq.wrapping_sub(f.seq) >= u32::MAX / 2 {
                break; // an older ack than this frame
            }
            self.acked_bytes += f.bytes as u64;
            if f.burst {
                self.burst_grace = false;
            }
            if f.seq == seq {
                let delay = now.saturating_duration_since(f.at);
                if f.burst {
                    self.probes.push((f.bytes, delay, self.rtt.get()));
                }
                let ms = delay.as_millis() as u32;
                self.rtt.push(ms, now);
                self.delays_ms.push(ms);
            }
            self.inflight.pop_front();
        }
        self.unacked_since = (!self.inflight.is_empty()).then_some(now);
    }
}

fn window_open(
    inflight: usize,
    oldest_age: Option<Duration>,
    rtt: Duration,
    slack: Duration,
) -> bool {
    inflight < MAX_INFLIGHT && oldest_age.is_none_or(|age| age < ACK_TIMEOUT && age < rtt + slack)
}

fn pacer_index(viewers: &[Viewer]) -> Option<usize> {
    viewers
        .iter()
        .position(|v| !v.view_only)
        .or_else(|| (!viewers.is_empty()).then_some(0))
}

fn deliver_clipboard(viewers: &[Viewer], owner: u64, text: &str) {
    if let Some(viewer) = viewers.iter().find(|v| v.id == owner && !v.view_only)
        && let Ok(record) = wire::clipboard(text)
    {
        viewer.queue(record.into());
    }
}

fn deliver_owner(viewers: &[Viewer], owner: u64, record: Vec<u8>) {
    if let Some(viewer) = viewers.iter().find(|v| v.id == owner && !v.view_only) {
        viewer.queue(record.into());
    }
}

struct Retiring {
    viewer: Viewer,
    until: Instant,
}

fn admit_viewer(
    viewers: &mut Vec<Viewer>,
    retiring: &mut Vec<Retiring>,
    viewer: Viewer,
    max_viewers: usize,
) -> bool {
    if viewer.view_only && viewers.iter().filter(|v| v.view_only).count() >= max_viewers {
        eprintln!(
            "slimviewer: event=disconnect viewer={} reason=view_limit",
            viewer.id
        );
        return false;
    }
    if !viewer.queue(wire::hello(viewer.view_only).into()) {
        return false;
    }
    if !viewer.view_only
        && let Some(index) = viewers.iter().position(|v| !v.view_only)
    {
        let old = viewers.remove(index);
        eprintln!(
            "slimviewer: event=disconnect viewer={} reason=owner_replaced",
            old.id
        );
        // Relays may discard buffered data at EOF. Let the page receive this notice and
        // close first, with a bounded grace period and a bound on retiring connections.
        if old.queue(wire::hello(true).into()) {
            if retiring.len() == MAX_RETIRING {
                retiring.remove(0);
            }
            retiring.push(Retiring {
                viewer: old,
                until: Instant::now() + ACK_TIMEOUT,
            });
        }
    }
    eprintln!(
        "slimviewer: event=connect viewer={} role={}",
        viewer.id,
        if viewer.view_only { "view" } else { "owner" }
    );
    viewers.push(viewer);
    true
}

fn expire_viewers(viewers: &mut Vec<Viewer>, now: Instant) -> bool {
    let mut reset = false;
    viewers.retain_mut(|v| {
        let reason = if v.write_failed.load(Ordering::Relaxed) {
            "write_failed"
        } else if (v.view_only && v.expired(now)) || v.ack_deadline().is_some_and(|due| now >= due)
        {
            "ack_timeout"
        } else {
            if v.expired(now) {
                v.reset_window();
                reset = true;
                eprintln!("slimviewer: event=window_reset viewer={}", v.id);
            }
            return true;
        };
        eprintln!(
            "slimviewer: event=disconnect viewer={} reason={reason}",
            v.id
        );
        false
    });
    reset
}

fn observe_input(picture: &mut Picture, record: &wire::FromPage, now: Instant) {
    if matches!(
        record,
        wire::FromPage::Pointer { .. }
            | wire::FromPage::Key { .. }
            | wire::FromPage::Paste(_)
            | wire::FromPage::PasteTx { .. }
            | wire::FromPage::Type(_)
            | wire::FromPage::Wheel { .. }
    ) {
        picture.input(now);
    }
}

fn frame_plan(
    picture: &mut Picture,
    now: Instant,
    band: Option<(usize, usize, usize)>,
    needs_key: bool,
) -> Plan {
    let plan = picture.plan(now, band);
    if needs_key {
        Plan {
            scale: picture.scale(),
            sharp_qp: None,
        }
    } else {
        plan
    }
}

fn acknowledge(viewers: &mut [Viewer], picture: &mut Picture, id: u64, seq: u32, now: Instant) {
    let pacer = pacer_index(viewers).map(|index| viewers[index].id);
    if let Some(viewer) = viewers.iter_mut().find(|viewer| viewer.id == id) {
        viewer.on_ack(seq, now);
        // This ACK may open the window for a sharp frame in the same loop pass.
        for (bytes, delay, rtt) in viewer.probes.drain(..) {
            if pacer == Some(id) {
                picture.probe(now, bytes, delay, rtt);
            }
        }
    }
}

/// ACK floor and spread over the last 5–10 s (two rotating windows).
struct AckDelays {
    cur: u32,
    prev: u32,
    cur_max: u32,
    prev_max: u32,
    since: Instant,
}

impl AckDelays {
    fn new() -> AckDelays {
        AckDelays {
            cur: u32::MAX,
            prev: u32::MAX,
            cur_max: 0,
            prev_max: 0,
            since: Instant::now(),
        }
    }
    fn push(&mut self, ms: u32, now: Instant) {
        if now - self.since > Duration::from_secs(5) {
            (self.prev, self.cur, self.since) = (self.cur, ms, now);
            (self.prev_max, self.cur_max) = (self.cur_max, ms);
        } else {
            self.cur = self.cur.min(ms);
            self.cur_max = self.cur_max.max(ms);
        }
    }
    fn get(&self) -> Duration {
        let ms = self.cur.min(self.prev);
        Duration::from_millis(if ms == u32::MAX { 50 } else { u64::from(ms) })
    }
    fn slack(&self, interval: Duration) -> Duration {
        let max = Duration::from_millis(u64::from(self.cur_max.max(self.prev_max)));
        interval.max(max.saturating_sub(self.get()).min(MAX_ACK_JITTER))
    }
}

struct Trace {
    enabled: bool,
    origin: Instant,
}

impl Trace {
    fn new(setting: Option<&str>, origin: Instant) -> Self {
        Self {
            enabled: setting == Some("1"),
            origin,
        }
    }

    fn write(&self, out: &mut impl Write, now: Instant, fields: std::fmt::Arguments<'_>) {
        if self.enabled {
            let ms = now.saturating_duration_since(self.origin).as_secs_f64() * 1000.0;
            let _ = writeln!(out, "slimviewer: trace at_ms={ms:.3} {fields}");
        }
    }

    fn log(&self, now: Instant, fields: std::fmt::Arguments<'_>) {
        if self.enabled {
            self.write(&mut io::stderr().lock(), now, fields);
        }
    }
}

enum FrameAction {
    Grab,
    Still(Plan),
    Wait(&'static str),
}

fn frame_action(
    picture: &Picture,
    now: Instant,
    last_send: Instant,
    pending: bool,
    open: bool,
) -> FrameAction {
    if !open {
        return FrameAction::Wait("window_closed");
    }
    if now < last_send + Duration::from_secs_f32(1.0 / picture.fps()) {
        return FrameAction::Wait("interval");
    }
    if pending {
        return FrameAction::Grab;
    }
    if let Some(plan) = picture.still(now) {
        FrameAction::Still(plan)
    } else {
        FrameAction::Wait(if picture.deadline(now).is_some() {
            "still_deadline"
        } else {
            "no_damage"
        })
    }
}

fn frame_deadline(
    picture: &Picture,
    now: Instant,
    last_send: Instant,
    pending: bool,
    viewer: &Viewer,
) -> Option<Instant> {
    let interval = Duration::from_secs_f32(1.0 / picture.fps());
    let ready = if pending {
        last_send + interval
    } else {
        picture.deadline(now)?
    };
    Some(
        viewer
            .window_deadline(now, interval, picture.capacity_kbps(now), pending)
            .unwrap_or(ready.max(last_send + interval)),
    )
}

struct Engine {
    cfg: Config,
    cap: Capture,
    input: Input,
    enc: Option<Encoder>,
    pic: I420,
    viewers: Vec<Viewer>,
    retiring: Vec<Retiring>,
    seq: u32,
    t0: Instant,
    trace: Trace,
    damaged: bool,
    resized: bool,
    want_key: bool,
    last_send: Instant,
    last_key: Instant,
    picture: Picture,
    /// When the oldest damage not yet in a sent frame arrived (for the damage→send log figure).
    damage_at: Option<Instant>,
    blocked: Duration,
    blocked_since: Option<Instant>,
    window_reset: bool,
    tick_at: Instant,
    last_log: Option<Instant>,
    throttled_usec: Option<u64>,
    cpu_times: Option<CpuTimes>,
    stats: Stats,
}

#[derive(Default)]
struct Stats {
    frames: u32,
    keys: u32,
    sharps: u32,
    bytes: u64,
    grabs: u32,
    empty_grabs: u32,
    grab_us: u64,
    encode_us: u64,
    lag_us: u64,
    lag_n: u32,
    delayed: u32,
}

pub fn run(cfg: Config, cap: Capture, input: Input, events: Receiver<Ev>) -> Result<(), String> {
    let now = Instant::now();
    let mut e = Engine {
        picture: Picture::new(
            picture::Config {
                base_crf: cfg.encoder.crf,
                crf_range: cfg.crf_range,
                sharpen_qp: cfg.sharpen_qp,
                sharpen_after: cfg.sharpen_after,
                max_fps: cfg.max_fps,
                adaptive_size: cfg.adaptive_size,
                cpu_guard: cfg.cpu_guard,
            },
            usize::from(cap.width & !1),
            usize::from(cap.height & !1),
            now,
        ),
        damage_at: None,
        cfg,
        pic: I420::new(usize::from(cap.width & !1), usize::from(cap.height & !1)),
        cap,
        input,
        enc: None,
        viewers: Vec::new(),
        retiring: Vec::new(),
        seq: 0,
        t0: now,
        trace: Trace::new(std::env::var("SLIMVIEWER_TRACE").ok().as_deref(), now),
        damaged: true,
        resized: false,
        want_key: true,
        last_send: now,
        last_key: now,
        blocked: Duration::ZERO,
        blocked_since: None,
        window_reset: false,
        tick_at: now,
        last_log: None,
        throttled_usec: throttled_usec(),
        cpu_times: cpu_times(),
        stats: Stats::default(),
    };
    loop {
        match events.recv_timeout(e.wait(Instant::now())) {
            Ok(ev) => e.on(ev)?,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
        // A busy input connection cannot starve capture or timeout enforcement.
        for ev in events.try_iter().take(64) {
            e.on(ev)?;
        }
        e.step()?;
    }
}

impl Engine {
    fn owner_id(&self) -> Option<u64> {
        self.viewers.iter().find(|v| !v.view_only).map(|v| v.id)
    }

    fn owner_connection(&self) -> Option<(u64, Arc<Connection>)> {
        self.viewers
            .iter()
            .find(|v| !v.view_only)
            .map(|v| (v.id, v.connection.clone()))
    }

    fn release_changed_owner(
        &mut self,
        previous: Option<(u64, Arc<Connection>)>,
    ) -> Result<(), String> {
        if let Some((id, connection)) = previous
            && Some(id) != self.owner_id()
        {
            self.input
                .release_owner(connection)
                .map_err(|e| format!("RFB release input failed: {e}"))?;
        }
        Ok(())
    }

    fn interval(&self) -> Duration {
        Duration::from_secs_f32(1.0 / self.picture.fps())
    }

    /// The viewer whose acks pace the stream: the owner, else the first spectator.
    fn pacer(&mut self) -> Option<&mut Viewer> {
        let i = pacer_index(&self.viewers)?;
        self.viewers.get_mut(i)
    }

    fn on(&mut self, ev: Ev) -> Result<(), String> {
        let previous_owner = self.owner_connection();
        let now = Instant::now();
        match &ev {
            Ev::Damage => self.trace.log(now, format_args!("event=damage")),
            Ev::Ack(id, seq) => self
                .trace
                .log(now, format_args!("event=ack viewer={id} seq={seq}")),
            Ev::Input(id, _) => self.trace.log(now, format_args!("event=input viewer={id}")),
            _ => {}
        }
        match ev {
            Ev::Damage => {
                self.damaged = true;
                self.damage_at.get_or_insert(now);
                self.picture.damage(now);
            }
            Ev::Resized => self.resized = true,
            Ev::XGone => return Err("the X server went away".into()),
            Ev::Fatal(reason) => return Err(reason),
            Ev::Joined(v, admitted) => {
                let owner = !v.view_only;
                let reset = expire_viewers(&mut self.viewers, now);
                self.want_key |= reset;
                self.window_reset |= reset;
                let accepted = if owner && !self.input.admits_owner() {
                    eprintln!(
                        "slimviewer: event=disconnect viewer={} reason=owner_drain_limit",
                        v.id
                    );
                    false
                } else {
                    admit_viewer(
                        &mut self.viewers,
                        &mut self.retiring,
                        v,
                        self.cfg.max_viewers,
                    )
                };
                let key = accepted && (owner || self.viewers.len() == 1);
                self.want_key |= key;
                let _ = admitted.send(accepted);
            }
            Ev::Left(id) => {
                if self.viewers.iter().any(|v| v.id == id) {
                    eprintln!("slimviewer: event=disconnect viewer={id} reason=reader_closed");
                }
                self.viewers.retain(|v| v.id != id);
                self.retiring.retain(|r| r.viewer.id != id);
            }
            Ev::Ack(id, seq) => acknowledge(&mut self.viewers, &mut self.picture, id, seq, now),
            Ev::Keyframe(id) => {
                if let Some(v) = self.viewers.iter_mut().find(|v| v.id == id) {
                    v.needs_key = true;
                }
            }
            Ev::Clipboard(id, text) => deliver_clipboard(&self.viewers, id, &text),
            Ev::PasteResult(owner, id, outcome) => {
                deliver_owner(&self.viewers, owner, wire::paste_result(id, outcome))
            }
            Ev::Fields(owner, fields) => deliver_owner(&self.viewers, owner, wire::fields(&fields)),
            Ev::Input(id, record) => {
                let Some(owner) = self.viewers.iter().find(|v| v.id == id && !v.view_only) else {
                    return Ok(());
                };
                observe_input(&mut self.picture, &record, now);
                match owner.input(&mut self.input, record) {
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        eprintln!(
                            "slimviewer: event=disconnect viewer={id} reason=input_queue_full"
                        );
                        self.viewers.retain(|v| v.id != id);
                    }
                    other => other.map_err(|e| format!("RFB input failed: {e}"))?,
                }
            }
        }
        self.release_changed_owner(previous_owner)
    }

    /// How long the loop may sleep before something is due. A closed window wakes for burst
    /// grace accounting, an ACK event, or the ACK timeout; it is never polled.
    fn wait(&self, now: Instant) -> Duration {
        let mut due = self.tick_at;
        for retiring in &self.retiring {
            due = due.min(retiring.until);
        }
        for viewer in &self.viewers {
            if let Some(deadline) = viewer.ack_deadline() {
                due = due.min(deadline);
            }
            if let Some(frame) = viewer.inflight.front() {
                due = due.min(frame.at + ACK_TIMEOUT);
            }
            if viewer.needs_key && viewer.open(now, self.interval()) {
                due = due.min(self.last_key + Duration::from_secs(1));
            }
        }
        if let Some(index) = pacer_index(&self.viewers)
            && let Some(ready) = frame_deadline(
                &self.picture,
                now,
                self.last_send,
                self.damaged || self.want_key || self.resized,
                &self.viewers[index],
            )
        {
            due = due.min(ready);
        }
        due.saturating_duration_since(now)
            .max(Duration::from_millis(1))
    }

    fn step(&mut self) -> Result<(), String> {
        let previous_owner = self.owner_connection();
        self.stream_step()?;
        self.release_changed_owner(previous_owner)
    }

    fn stream_step(&mut self) -> Result<(), String> {
        let now = Instant::now();
        self.retiring.retain(|r| now < r.until);
        let reset = expire_viewers(&mut self.viewers, now);
        self.want_key |= reset;
        self.window_reset |= reset;
        if now >= self.tick_at {
            self.tick(now);
        }
        if self.viewers.is_empty() {
            self.trace
                .log(now, format_args!("event=loop reason=no_viewer"));
            return Ok(());
        }
        if now.saturating_duration_since(self.last_key) >= Duration::from_secs(1)
            && self
                .viewers
                .iter()
                .any(|v| v.needs_key && v.open(now, self.interval()))
        {
            self.want_key = true;
        }
        if self.resized {
            self.resized = false;
            if self.cap.fit()? {
                eprintln!(
                    "slimviewer: screen is now {}x{}",
                    self.cap.width, self.cap.height
                );
                self.enc = None;
                self.picture.resize(
                    usize::from(self.cap.width & !1),
                    usize::from(self.cap.height & !1),
                    now,
                );
            }
        }
        if self.enc.is_none() {
            self.encoder(Scale::Full)?;
            self.cap.forget();
            self.damaged = true;
        }
        let interval = self.interval();
        let open = self.pacer().is_some_and(|p| p.open(now, interval));
        let wants = self.damaged || self.want_key;
        let capacity = self.picture.capacity_kbps(now);
        let blocked = wants
            && self
                .pacer()
                .is_some_and(|p| p.link_blocked(now, interval, capacity));
        match (blocked, self.blocked_since) {
            (true, None) => self.blocked_since = Some(now),
            (false, Some(t)) => {
                self.blocked += now - t;
                self.blocked_since = None;
            }
            _ => {}
        }
        let action = frame_action(&self.picture, now, self.last_send, wants, open);
        if self.trace.enabled {
            let reason = match action {
                FrameAction::Grab => "capture",
                FrameAction::Still(_) => "still",
                FrameAction::Wait(reason) => reason,
            };
            let viewer = &self.viewers[pacer_index(&self.viewers).unwrap()];
            let oldest_ms = viewer
                .inflight
                .front()
                .or_else(|| viewer.reset_frames.front())
                .map_or(0.0, |frame| {
                    now.saturating_duration_since(frame.at).as_secs_f64() * 1000.0
                });
            self.trace.log(now, format_args!(
                "event=loop reason={reason} damage={} key={} open={open} inflight={} reset_pending={} oldest_ms={oldest_ms:.3} since_send_ms={:.3} wait_ms={:.3}",
                self.damaged, self.want_key, viewer.inflight.len(), viewer.reset_frames.len(),
                now.saturating_duration_since(self.last_send).as_secs_f64() * 1000.0,
                self.wait(now).as_secs_f64() * 1000.0,
            ));
        }
        if let FrameAction::Grab = action {
            self.damaged = false;
            let damaged_at = self.damage_at.take();
            let t = Instant::now();
            let band = match self.cap.grab() {
                Ok(band) => band,
                // The screen changed size under the grab: refit next step, then grab again.
                Err(GrabError::Resized { first }) => {
                    if first {
                        eprintln!("slimviewer: event=grab_resized");
                    }
                    (self.resized, self.damaged, self.damage_at) = (true, true, damaged_at);
                    return Ok(());
                }
                Err(GrabError::Fatal(e)) => return Err(e),
            };
            self.stats.grabs += 1;
            let grab = t.elapsed();
            self.trace.log(
                Instant::now(),
                format_args!(
                    "event=grab changed_rows={} grab_ms={:.3}",
                    band.map_or(0, |(_, _, changed)| changed),
                    grab.as_secs_f64() * 1000.0,
                ),
            );
            let needs_key = self.pacer().is_some_and(|p| p.needs_key);
            let plan = frame_plan(&mut self.picture, now, band, needs_key);
            let convert_at = Instant::now();
            if self.encoder(plan.scale)? {
                self.convert(plan.scale, 0, usize::from(self.cap.height));
            } else if let Some((y0, y1, _)) = band {
                self.convert(plan.scale, y0, y1);
            }
            let work = Work {
                grab,
                convert: convert_at.elapsed(),
                encode: Duration::ZERO,
            };
            if band.is_none() {
                self.stats.empty_grabs += 1;
            }
            self.stats.grab_us += t.elapsed().as_micros() as u64;
            // Keep converted pixels current: fresh damage may resume with a smaller band.
            if !self.want_key && self.picture.defer_half(now, plan) {
                self.damage_at = damaged_at;
                self.trace.log(
                    Instant::now(),
                    format_args!("event=defer reason=still_deadline"),
                );
                return Ok(());
            }
            if band.is_some() || self.want_key {
                let big =
                    band.is_some_and(|(_, _, changed)| changed * 2 > usize::from(self.cap.height));
                self.send(now, plan, work, big);
                if let Some(at) = damaged_at {
                    self.stats.lag_us += at.elapsed().as_micros() as u64;
                    self.stats.lag_n += 1;
                }
            }
        } else if let FrameAction::Still(plan) = action {
            let damaged_at = self.damage_at.take();
            let t = Instant::now();
            if self.encoder(plan.scale)? {
                self.convert(plan.scale, 0, usize::from(self.cap.height));
            }
            self.send(
                now,
                plan,
                Work {
                    convert: t.elapsed(),
                    ..Work::default()
                },
                false,
            );
            if let Some(at) = damaged_at {
                self.stats.lag_us += at.elapsed().as_micros() as u64;
                self.stats.lag_n += 1;
            }
        }
        Ok(())
    }

    /// Makes sure the encoder takes full- or half-size pictures of the current screen; a change
    /// reopens it (the next frame is a keyframe at the new size). Returns whether it changed, in
    /// which case the whole picture must be converted again.
    fn encoder(&mut self, scale: Scale) -> Result<bool, String> {
        let (w, h) = scale.dims(self.cap.width, self.cap.height);
        if self
            .enc
            .as_ref()
            .is_some_and(|e| (e.width, e.height) == (w, h))
        {
            return Ok(false);
        }
        self.enc = None;
        // Half size = a short link or CPU: cap its rate. Full size: none (sharp keyframes).
        let vbv = if scale == Scale::Half {
            self.picture.link_kbps()
        } else {
            0
        };
        self.enc = Some(
            Encoder::open(w, h, &self.cfg.encoder, self.picture.crf(), vbv)
                .ok_or("encoder refused the settings")?,
        );
        self.pic = I420::new(usize::from(w), usize::from(h));
        self.want_key = true;
        Ok(true)
    }

    fn convert(&mut self, scale: Scale, y0: usize, y1: usize) {
        let stride = usize::from(self.cap.width) * 4;
        if scale == Scale::Half {
            self.pic.convert_half(self.cap.picture(), stride, y0, y1);
        } else {
            self.pic
                .convert(self.cap.picture(), stride, y0, y1.min(self.pic.height));
        }
    }

    fn send(&mut self, now: Instant, plan: Plan, mut work: Work, big: bool) {
        let qp = plan.sharp_qp;
        let full_return = self.picture.scale() == Scale::Half && plan.scale == Scale::Full;
        let owner_key = self.pacer().is_some_and(|viewer| viewer.needs_key);
        // A viewer that skipped a frame cannot decode the next ones; resync it, at most once a second.
        let resync = self.viewers.iter().any(|v| v.needs_key)
            && now - self.last_key > Duration::from_secs(1);
        let idr = self.want_key || resync;
        let Some(enc) = self.enc.as_mut() else { return };
        let t = Instant::now();
        let pts = (now - self.t0).as_millis() as i64;
        let Some(out) = enc.encode(&self.pic, idr, qp, pts) else {
            self.trace.log(
                Instant::now(),
                format_args!("event=encode reason=no_output"),
            );
            return;
        };
        self.stats.delayed += u32::from(out.delayed);
        work.encode = t.elapsed();
        self.stats.encode_us += work.encode.as_micros() as u64;
        self.seq = self.seq.wrapping_add(1);
        let record: Arc<[u8]> = wire::frame(
            self.seq,
            out.key,
            qp.is_some(),
            plan.scale == Scale::Half,
            enc.width,
            enc.height,
            &out.data,
        )
        .into();
        if out.key {
            self.want_key = false;
            self.last_key = now;
        }
        self.picture.encoded(
            now,
            picture::Encoded {
                plan,
                key: out.key,
                owner_key,
                bytes: out.data.len(),
                pixels: usize::from(enc.width) * usize::from(enc.height),
                big,
                work,
            },
        );
        let interval = self.interval();
        let seq = self.seq;
        self.viewers.retain_mut(|v| {
            // Keyframes consume credit too. Never queue for a stalled spectator.
            if !v.open(now, interval) {
                v.needs_key = true;
                return true;
            }
            if out.key {
                v.needs_key = false;
            } else if v.needs_key {
                return true;
            }
            if !v.queue(record.clone()) {
                v.needs_key = true;
                if !v.view_only && !v.write_failed.load(Ordering::Relaxed) {
                    return true;
                }
                let reason = if v.write_failed.load(Ordering::Relaxed) {
                    "write_failed"
                } else {
                    "write_queue_full"
                };
                eprintln!(
                    "slimviewer: event=disconnect viewer={} reason={reason}",
                    v.id
                );
                return false;
            }
            let queued_at = Instant::now();
            v.record_frame(
                Sent {
                    seq,
                    at: queued_at,
                    bytes: record.len(),
                    burst: out.key || qp.is_some(),
                },
                full_return && out.key,
            );
            true
        });
        self.last_send = now;
        self.stats.frames += 1;
        self.stats.keys += u32::from(out.key);
        self.stats.sharps += u32::from(qp.is_some());
        self.stats.bytes += record.len() as u64;
        self.trace.log(Instant::now(), format_args!(
            "event=send seq={seq} bytes={} requested_key={idr} key={} sharp={} scale={:?} encode_ms={:.3}",
            record.len(), out.key, qp.is_some(), plan.scale, work.encode.as_secs_f64() * 1000.0,
        ));
    }

    /// Once a second: feed link and CPU facts to policy, apply rates, and log.
    fn tick(&mut self, now: Instant) {
        let elapsed = (now - self.tick_at + Duration::from_secs(1))
            .as_secs_f32()
            .max(0.5);
        self.tick_at = now + Duration::from_secs(1);
        if let Some(t) = self.blocked_since {
            self.blocked += now - t;
            self.blocked_since = Some(now);
        }
        let blocked = std::mem::take(&mut self.blocked).as_secs_f32() / elapsed;
        let frames = self.stats.frames;
        let mut rtt_ms = 0;
        let mut link = picture::Link {
            reset: std::mem::take(&mut self.window_reset),
            blocked,
            sent_kbps: self.stats.bytes as f32 * 8.0 / 1000.0 / elapsed,
            ..picture::Link::default()
        };
        if let Some(p) = self.pacer() {
            link.delivered_kbps =
                (std::mem::take(&mut p.acked_bytes) * 8) as f32 / 1000.0 / elapsed;
            let mut d = std::mem::take(&mut p.delays_ms);
            d.sort_unstable();
            rtt_ms = p.rtt.get().as_millis() as u32;
            link.queue_ms = d.get(d.len() / 2).map_or(0, |m| m.saturating_sub(rtt_ms));
        }
        for viewer in &mut self.viewers {
            viewer.acked_bytes = 0;
            viewer.delays_ms.clear();
        }
        let throttled = throttled_usec().is_some_and(|t| {
            t.saturating_sub(self.throttled_usec.replace(t).unwrap_or(t)) > 10_000
        });
        let stolen = cpu_times().map_or(0.0, |current| {
            let share = self
                .cpu_times
                .as_ref()
                .map_or(0.0, |old| current.stolen_since(old));
            self.cpu_times = Some(current);
            share
        });
        let queue_ms = link.queue_ms;
        self.picture
            .tick(now, link, picture::Cpu { stolen, throttled });
        if let Some(enc) = self.enc.as_mut() {
            let vbv = if self.picture.scale() == Scale::Half {
                self.picture.link_kbps()
            } else {
                0
            };
            enc.set_rate(self.picture.crf(), vbv);
        }
        let s = &mut self.stats;
        if frames > 0
            || self
                .last_log
                .is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_secs(10))
        {
            self.last_log = Some(now);
            let per = |us: u64, n: u32| {
                if n == 0 {
                    0.0
                } else {
                    us as f32 / n as f32 / 1000.0
                }
            };
            eprintln!(
                "slimviewer: {frames} frames ({} key, {} sharp{}) {} kbps{} | grabs {} ({} unchanged) {:.1} ms, encode {:.1} ms, damage→sent {:.1} ms | crf {:.0}, link {} kbps, fps cap {:.0}, rtt {rtt_ms} ms, queue {queue_ms} ms, blocked {:.0}%{} | scale {}, work p75 {:.1} ms, stolen {:.0}%, sharp qp {}, cap {:.0} kbps",
                s.keys,
                s.sharps,
                if s.delayed > 0 {
                    format!(", {} delayed by x264", s.delayed)
                } else {
                    String::new()
                },
                s.bytes * 8 / 1000,
                if self.picture.scale() == Scale::Half {
                    " half-size"
                } else {
                    ""
                },
                s.grabs,
                s.empty_grabs,
                per(s.grab_us, s.grabs),
                per(s.encode_us, frames),
                per(s.lag_us, s.lag_n),
                self.picture.crf(),
                self.picture.link_kbps(),
                self.picture.fps(),
                blocked * 100.0,
                if throttled { ", cpu throttled" } else { "" },
                self.picture.scale_label(),
                self.picture.work_p75_ms,
                stolen * 100.0,
                self.picture
                    .sharp_qp(now)
                    .map_or_else(|| "skip".to_owned(), |qp| qp.to_string()),
                self.picture.capacity_kbps(now).unwrap_or(0.0),
            );
        }
        *s = Stats::default();
    }
}

/// cgroup v2 CPU throttling so far, if this container has a CPU quota.
fn throttled_usec() -> Option<u64> {
    let stat = std::fs::read_to_string("/sys/fs/cgroup/cpu.stat").ok()?;
    stat.lines()
        .find_map(|l| l.strip_prefix("throttled_usec "))
        .and_then(|n| n.trim().parse().ok())
}

struct CpuTimes {
    total: u64,
    steal: u64,
}
impl CpuTimes {
    fn parse(stat: &str) -> Option<Self> {
        let line = stat.lines().find(|line| line.starts_with("cpu "))?;
        let mut parts = line.split_whitespace().skip(1);
        let mut total = 0_u64;
        let mut steal = 0;
        // Guest and guest_nice are already included in user and nice.
        for field in 0..8 {
            let ticks: u64 = parts.next()?.parse().ok()?;
            total = total.checked_add(ticks)?;
            if field == 7 {
                steal = ticks;
            }
        }
        Some(Self { total, steal })
    }
    fn stolen_since(&self, old: &Self) -> f32 {
        let total = self.total.saturating_sub(old.total);
        if total == 0 {
            return 0.0;
        }
        (self.steal.saturating_sub(old.steal) as f32 / total as f32).clamp(0.0, 1.0)
    }
}

fn cpu_times() -> Option<CpuTimes> {
    CpuTimes::parse(&std::fs::read_to_string("/proc/stat").ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn viewer() -> Viewer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        Viewer::new(1, true, stream).unwrap()
    }

    fn viewer_pair(id: u64, view_only: bool) -> (Viewer, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (peer, _) = listener.accept().unwrap();
        (Viewer::new(id, view_only, stream).unwrap(), peer)
    }

    #[test]
    fn tests_that_owner_socket_stays_open_while_its_release_is_pending() {
        use std::io::Read;
        let (owner, mut peer) = viewer_pair(1, false);
        peer.set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let pending_release = owner.connection.clone();
        drop(owner);
        let result = peer.read(&mut [0]);
        assert!(
            matches!(result, Err(ref e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut)),
            "owner closed before its pending release: {result:?}"
        );
        drop(pending_release);
        assert_eq!(peer.read(&mut [0]).unwrap(), 0);
    }

    #[test]
    fn tests_that_a_failed_owner_drain_resets_tcp_after_writer_and_reader_clones_close() {
        use std::io::Read;
        let (owner, mut peer) = viewer_pair(1, false);
        let mut reader = owner.connection.stream.try_clone().unwrap();
        let reader_done = std::thread::spawn(move || {
            let _ = reader.read(&mut [0]);
        });
        owner.connection.wait_for_input();
        drop(owner);
        peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let result = peer.read(&mut [0]);
        assert!(
            matches!(result, Err(ref e) if e.kind() == io::ErrorKind::ConnectionReset),
            "failed drain must reset, not half-close: {result:?}"
        );
        reader_done.join().unwrap();
    }

    #[test]
    fn tests_that_input_failure_before_owner_eof_resets_the_active_connection() {
        use std::io::Read;
        let (events, errors) = mpsc::sync_channel(4);
        let mut input = Input::failing_for_test(events);
        let (owner, mut peer) = viewer_pair(1, false);
        owner
            .input(&mut input, wire::FromPage::Type("accepted".into()))
            .unwrap();
        assert!(matches!(
            errors.recv_timeout(Duration::from_secs(3)),
            Ok(Ev::Fatal(_))
        ));
        // Fatal can arrive before the owner's Left event; no release was queued yet.
        drop(owner);
        peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let result = peer.read(&mut [0]);
        assert!(
            matches!(result, Err(ref e) if e.kind() == io::ErrorKind::ConnectionReset),
            "an active owner with failed input must not see clean EOF: {result:?}"
        );
    }

    #[test]
    fn clipboard_replies_reach_only_the_current_owner_and_never_a_spectator() {
        use std::io::Read;
        let (spectator, mut view_peer) = viewer_pair(1, true);
        let (owner, mut owner_peer) = viewer_pair(2, false);
        let viewers = vec![spectator, owner];
        deliver_clipboard(&viewers, 1, "spectator secret");
        deliver_clipboard(&viewers, 99, "previous owner secret");
        deliver_clipboard(&viewers, 2, "copied sentence");
        let expected = wire::clipboard("copied sentence").unwrap();
        owner_peer
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut result = vec![0; expected.len()];
        owner_peer.read_exact(&mut result).unwrap();
        assert_eq!(result, expected);
        view_peer.set_nonblocking(true).unwrap();
        assert_eq!(
            view_peer.read(&mut [0]).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        owner_peer.set_nonblocking(true).unwrap();
        assert_eq!(
            owner_peer.read(&mut [0]).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn tests_that_paste_results_and_fields_reach_only_the_current_owner() {
        use crate::rfb::{Fields, PasteOutcome};
        use std::io::Read;
        let (spectator, mut view_peer) = viewer_pair(1, true);
        let (owner, mut owner_peer) = viewer_pair(2, false);
        let viewers = vec![spectator, owner];
        let mut expected = Vec::new();
        for record in [
            wire::paste_result(9, PasteOutcome::Unconfirmed),
            wire::fields(&Fields::new(780, 1688, None, vec![]).unwrap()),
        ] {
            deliver_owner(&viewers, 1, record.clone());
            deliver_owner(&viewers, 99, record.clone());
            deliver_owner(&viewers, 2, record.clone());
            expected.extend(record);
        }
        owner_peer
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut bytes = vec![0; expected.len()];
        owner_peer.read_exact(&mut bytes).unwrap();
        assert_eq!(bytes, expected);
        for peer in [&mut view_peer, &mut owner_peer] {
            peer.set_nonblocking(true).unwrap();
            assert_eq!(
                peer.read(&mut [0]).unwrap_err().kind(),
                io::ErrorKind::WouldBlock
            );
        }
    }

    #[test]
    fn tests_that_inflight_key_or_sharp_wait_is_not_link_blockage() {
        let mut v = viewer();
        let now = Instant::now();
        v.inflight.push_back(Sent {
            seq: 1,
            at: now - Duration::from_millis(400),
            bytes: 150_000,
            burst: true,
        });
        assert!(!v.open(now, Duration::from_millis(33)));
        assert!(!v.link_blocked(now, Duration::from_millis(33), None));
        v.inflight.front_mut().unwrap().burst = false;
        assert!(v.link_blocked(now, Duration::from_millis(33), None));
    }

    #[test]
    fn tests_that_proc_stat_uses_steal_field_and_excludes_duplicate_guest_time() {
        let before =
            CpuTimes::parse("cpu  100 0 100 700 0 0 0 100 50 0\ncpu0 50 0 50 350 0 0 0 50 25 0\n")
                .unwrap();
        let after = CpuTimes::parse("cpu  100 0 100 775 0 0 0 125 75 0\n").unwrap();
        assert_eq!(before.total, 1000);
        assert_eq!(before.steal, 100);
        assert_eq!(after.stolen_since(&before), 0.25);
        assert_eq!(before.stolen_since(&after), 0.0);
        assert_eq!(after.stolen_since(&after), 0.0);
        assert!(CpuTimes::parse("cpu0 1 2 3 4 5 6 7 8").is_none());
        assert!(CpuTimes::parse("cpu 1 2 3 invalid").is_none());
    }

    #[test]
    fn tests_that_a_key_ack_budgets_the_first_sharp_before_the_next_tick() {
        let t = Instant::now();
        let mut p = motion(t, false);
        p.encoded(
            t,
            picture::Encoded {
                plan: Plan {
                    scale: Scale::Full,
                    sharp_qp: None,
                },
                key: true,
                owner_key: true,
                bytes: 150_000,
                pixels: 780 * ROWS,
                big: false,
                work: Work::default(),
            },
        );
        let mut v = viewer();
        v.rtt.push(30, t);
        v.inflight.push_back(Sent {
            seq: 1,
            at: t,
            bytes: 150_000,
            burst: true,
        });
        let mut viewers = vec![v];
        let now = t + Duration::from_millis(430);
        p.input(now - Duration::from_millis(100));
        acknowledge(&mut viewers, &mut p, 1, 1, now);
        assert_eq!(p.capacity_kbps(now).map(|n| n.round() as u32), Some(3000));
        assert!(
            p.still(now).is_none(),
            "the first sharp must use the freshly acknowledged capacity"
        );
    }

    fn queued(v: &mut Viewer, seq: u32, now: Instant) {
        v.unacked_since.get_or_insert(now);
        v.inflight.push_back(Sent {
            seq,
            at: now,
            bytes: 100,
            burst: false,
        });
        v.needs_key = false;
    }

    #[test]
    fn tests_that_an_owner_ack_stall_resets_the_window_without_disconnecting() {
        let (mut owner, _peer) = viewer_pair(1, false);
        let t = Instant::now();
        queued(&mut owner, 1, t);
        let mut viewers = vec![owner];
        assert!(!expire_viewers(
            &mut viewers,
            t + Duration::from_millis(2999)
        ));
        let reset = expire_viewers(&mut viewers, t + Duration::from_secs(3));
        assert_eq!(viewers.len(), 1);
        assert!(reset);
        assert!(viewers[0].inflight.is_empty());
        assert!(viewers[0].needs_key);
        assert!(viewers[0].open(t + Duration::from_secs(3), Duration::from_millis(33)));
    }

    #[test]
    fn tests_that_repeated_window_resets_do_not_extend_fifteen_second_timeout() {
        let (mut owner, _peer) = viewer_pair(1, false);
        let t = Instant::now();
        queued(&mut owner, 1, t);
        let mut viewers = vec![owner];
        for seconds in [3, 6, 9, 12] {
            expire_viewers(&mut viewers, t + Duration::from_secs(seconds));
            assert_eq!(viewers.len(), 1, "removed after {seconds} seconds");
            queued(
                &mut viewers[0],
                seconds as u32,
                t + Duration::from_secs(seconds),
            );
        }
        expire_viewers(&mut viewers, t + Duration::from_millis(14999));
        assert_eq!(viewers.len(), 1);
        expire_viewers(&mut viewers, t + Duration::from_secs(15));
        assert!(viewers.is_empty());
    }

    #[test]
    fn tests_that_late_ack_proves_liveness_without_releasing_new_window_credit() {
        let (mut owner, _peer) = viewer_pair(1, false);
        let t = Instant::now();
        queued(&mut owner, 1, t);
        let mut viewers = vec![owner];
        expire_viewers(&mut viewers, t + Duration::from_secs(3));
        assert_eq!(viewers.len(), 1);
        queued(&mut viewers[0], 2, t + Duration::from_secs(3));
        viewers[0].on_ack(1, t + Duration::from_secs(4));
        assert_eq!(viewers[0].inflight.front().unwrap().seq, 2);
        assert_eq!(viewers[0].acked_bytes, 100);
        expire_viewers(&mut viewers, t + Duration::from_secs(15));
        assert_eq!(viewers.len(), 1);
        // Neither duplicate old ACKs nor a future ACK can keep the socket alive forever.
        viewers[0].on_ack(1, t + Duration::from_secs(18));
        viewers[0].on_ack(99, t + Duration::from_secs(18));
        expire_viewers(&mut viewers, t + Duration::from_secs(19));
        assert!(viewers.is_empty());
    }

    #[test]
    fn tests_that_idle_owners_get_a_fresh_ack_deadline_on_the_next_frame() {
        let (owner, _peer) = viewer_pair(1, false);
        let t = Instant::now();
        let mut viewers = vec![owner];
        expire_viewers(&mut viewers, t + Duration::from_secs(60));
        assert_eq!(viewers.len(), 1);
        queued(&mut viewers[0], 1, t + Duration::from_secs(60));
        expire_viewers(&mut viewers, t + Duration::from_secs(63));
        assert_eq!(viewers.len(), 1);
        expire_viewers(&mut viewers, t + Duration::from_millis(74999));
        assert_eq!(viewers.len(), 1);
        expire_viewers(&mut viewers, t + Duration::from_secs(75));
        assert!(viewers.is_empty());
    }

    #[test]
    fn tests_that_resync_uses_current_size_without_a_sharp_frame() {
        let t = Instant::now();
        let mut p = motion(t, true);
        assert_eq!(
            frame_plan(&mut p, t, BIG, true),
            Plan {
                scale: Scale::Full,
                sharp_qp: None
            }
        );
        changed(&mut p, t, BIG);
        assert_eq!(
            frame_plan(&mut p, t + Duration::from_secs(1), SMALL, true),
            Plan {
                scale: Scale::Half,
                sharp_qp: None
            }
        );
    }

    #[test]
    fn tests_that_checked_paste_yields_sharpening_but_field_watch_does_not() {
        let t = Instant::now();
        let mut p = motion(t, false);
        p.encoded(
            t,
            picture::Encoded {
                plan: Plan {
                    scale: Scale::Full,
                    sharp_qp: Some(18),
                },
                key: true,
                owner_key: true,
                bytes: 150_000,
                pixels: 780 * ROWS,
                big: false,
                work: Work::default(),
            },
        );
        p.probe(
            t,
            150_000,
            Duration::from_millis(430),
            Duration::from_millis(30),
        );
        changed(&mut p, t + Duration::from_secs(1), BIG);
        let input = t + Duration::from_millis(1150);
        observe_input(
            &mut p,
            &wire::FromPage::PasteTx {
                id: 1,
                text: "hello".into(),
            },
            input,
        );
        assert!(p.still(t + Duration::from_millis(1250)).is_none());
        observe_input(
            &mut p,
            &wire::FromPage::FieldsWatch,
            input + Duration::from_millis(999),
        );
        assert!(
            p.still(input + Duration::from_secs(1))
                .unwrap()
                .sharp_qp
                .is_some()
        );
    }

    const ROWS: usize = 1450;
    const BIG: Option<(usize, usize, usize)> = Some((0, ROWS, 1200));
    const SMALL: Option<(usize, usize, usize)> = Some((0, 180, 180));
    const SHARPEN: Duration = Duration::from_millis(250);
    const FULL_SIZE_AFTER: Duration = Duration::from_secs(1);

    fn motion(now: Instant, constrained: bool) -> Picture {
        let mut p = Picture::new(
            picture::Config {
                base_crf: 24.0,
                crf_range: 12.0,
                sharpen_qp: 18,
                sharpen_after: SHARPEN,
                max_fps: 30.0,
                adaptive_size: true,
                cpu_guard: true,
            },
            780,
            ROWS,
            now,
        );
        if constrained {
            for n in (1..=2).rev() {
                p.tick(
                    now - Duration::from_secs(n),
                    picture::Link {
                        blocked: 0.6,
                        sent_kbps: 9600.0,
                        delivered_kbps: 3000.0,
                        reset: false,
                        queue_ms: 200,
                    },
                    picture::Cpu::default(),
                );
            }
        }
        p
    }

    fn changed(p: &mut Picture, now: Instant, band: Option<(usize, usize, usize)>) -> Plan {
        let plan = p.plan(now, band);
        p.encoded(
            now,
            picture::Encoded {
                plan,
                key: false,
                owner_key: true,
                bytes: 2000,
                pixels: 780 * ROWS / if plan.scale == Scale::Half { 4 } else { 1 },
                big: band.is_some_and(|(_, _, n)| n * 2 > ROWS),
                work: Work::default(),
            },
        );
        plan
    }

    #[test]
    fn tests_that_small_damage_a_second_after_big_motion_leaves_half_size_sharp() {
        let t0 = Instant::now();
        let mut m = motion(t0, true);
        assert_eq!(
            changed(&mut m, t0, BIG),
            Plan {
                scale: Scale::Half,
                sharp_qp: None
            }
        );
        assert_eq!(
            changed(&mut m, t0 + FULL_SIZE_AFTER, SMALL),
            Plan {
                scale: Scale::Full,
                sharp_qp: Some(18)
            }
        );
    }

    #[test]
    fn tests_that_small_damage_inside_the_hold_keeps_half_size() {
        let t0 = Instant::now();
        let mut m = motion(t0, true);
        changed(&mut m, t0, BIG);
        let early = t0 + Duration::from_millis(250);
        assert_eq!(
            changed(&mut m, early, SMALL),
            Plan {
                scale: Scale::Half,
                sharp_qp: None
            }
        );
        // An unchanged grab never switches the size; the still path handles it.
        assert_eq!(
            m.plan(t0 + FULL_SIZE_AFTER, None),
            Plan {
                scale: Scale::Half,
                sharp_qp: None
            }
        );
    }

    #[test]
    fn tests_that_after_the_calm_sharp_frame_small_damage_waits_for_big_motion() {
        let t0 = Instant::now();
        let mut m = motion(t0, true);
        changed(&mut m, t0, BIG);
        let t1 = t0 + FULL_SIZE_AFTER;
        assert!(changed(&mut m, t1, SMALL).sharp_qp.is_some());
        for ms in [100, 2_000, 30_000] {
            assert_eq!(
                changed(&mut m, t1 + Duration::from_millis(ms), SMALL),
                Plan {
                    scale: Scale::Full,
                    sharp_qp: None
                }
            );
        }
        let t2 = t1 + Duration::from_secs(31);
        assert_eq!(
            changed(&mut m, t2, BIG),
            Plan {
                scale: Scale::Half,
                sharp_qp: None
            }
        );
        assert!(
            changed(&mut m, t2 + FULL_SIZE_AFTER, SMALL)
                .sharp_qp
                .is_some()
        );
    }

    #[test]
    fn tests_that_a_free_link_sharpens_moving_pages_after_the_short_wait() {
        let t0 = Instant::now();
        let mut m = motion(t0, false);
        assert_eq!(
            changed(&mut m, t0, BIG),
            Plan {
                scale: Scale::Full,
                sharp_qp: None
            }
        );
        assert_eq!(
            changed(&mut m, t0 + Duration::from_millis(100), SMALL),
            Plan {
                scale: Scale::Full,
                sharp_qp: None
            }
        );
        assert_eq!(
            changed(&mut m, t0 + SHARPEN, SMALL),
            Plan {
                scale: Scale::Full,
                sharp_qp: Some(18)
            }
        );
    }

    #[test]
    fn tests_that_small_motion_never_starts_half_size() {
        let t0 = Instant::now();
        let mut m = motion(t0, true);
        assert_eq!(
            changed(&mut m, t0, Some((0, ROWS * 2 / 5, ROWS * 2 / 5))),
            Plan {
                scale: Scale::Full,
                sharp_qp: None
            }
        );
    }

    /// Sends `secs` seconds of 30 fps motion through `v`'s window over a test link: one queue
    /// drained at `kbps`, then a round trip of `rtt_ms` ± `jitter_ms` (uniform; acks stay in
    /// order, as on TCP). `bytes` gives a frame's size from its send time. Returns the share of
    /// each second the window held a frame back: the engine's `blocked`.
    fn pace(
        v: &mut Viewer,
        secs: usize,
        (kbps, rtt_ms, jitter_ms): (u64, u64, u64),
        bytes: impl Fn(Duration) -> usize,
    ) -> Vec<f32> {
        let t0 = Instant::now();
        let interval = Duration::from_millis(33);
        let (mut seed, mut seq, mut last_send) = (12345u32, 0, None::<Instant>);
        let (mut link_free, mut acks) = (t0, VecDeque::<(Instant, u32)>::new());
        let mut blocked = vec![0.0; secs];
        for ms in 0..secs * 1000 {
            let now = t0 + Duration::from_millis(ms as u64);
            while let Some(&(at, acked)) = acks.front()
                && at <= now
            {
                v.on_ack(acked, at);
                acks.pop_front();
            }
            if !v.open(now, interval) {
                blocked[ms / 1000] += 0.001;
            } else if last_send.is_none_or(|t| now >= t + interval) {
                seq += 1;
                let size = bytes(now - t0);
                v.inflight.push_back(Sent {
                    seq,
                    at: now,
                    bytes: size,
                    burst: false,
                });
                link_free = link_free.max(now) + Duration::from_micros(size as u64 * 8_000 / kbps);
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let jitter = (u64::from(seed) * 2 * jitter_ms) >> 32;
                let ack = link_free + Duration::from_millis(rtt_ms - jitter_ms + jitter);
                let ack = acks.back().map_or(ack, |&(last, _)| ack.max(last));
                acks.push_back((ack, seq));
                last_send = Some(now);
            }
        }
        blocked
    }

    #[test]
    fn tests_that_jitter_alone_is_not_a_full_link() {
        // Far Wi-Fi (40 Mbps, 70 ± 40 ms) under a moving banner: the window allows for the
        // jitter it has seen, so it never holds frames back for a quarter of a second.
        let mut v = viewer();
        let blocked = pace(&mut v, 12, (40_000, 70, 40), |_| 2_000);
        assert!(blocked.iter().all(|&b| !link_full(b)), "{blocked:?}");
        let now = Instant::now();
        let mut p = motion(now, false);
        for (n, &b) in blocked.iter().enumerate() {
            let at = now + Duration::from_secs(n as u64);
            p.tick(
                at,
                picture::Link {
                    blocked: b,
                    ..picture::Link::default()
                },
                picture::Cpu::default(),
            );
            assert_eq!(p.plan(at, BIG).scale, Scale::Full);
        }
    }

    #[test]
    fn tests_that_a_true_jam_still_sets_half_size() {
        // 3 Mbps, 100 ± 20 ms: 3 s of a quiet banner, then a scroll that wants three times the
        // link (40 KB frames at 30 fps) for 15 s, longer than the 5–10 s the delays are kept.
        let mut v = viewer();
        let scroll = Duration::from_secs(3);
        let blocked = pace(&mut v, 18, (3_000, 100, 20), |t| {
            if t < scroll { 2_000 } else { 40_000 }
        });
        assert!(blocked[..3].iter().all(|&b| !link_full(b)), "{blocked:?}");
        // Every second of the jam is a full link, to the end: the allowance for jitter is
        // bounded, so a standing queue cannot pass for jitter.
        assert!(blocked[3..].iter().all(|&b| link_full(b)), "{blocked:?}");
        // Quality is spent on tick one; a second congested tick sends big motion half size.
        let t0 = Instant::now();
        let mut p = motion(t0, false);
        for n in 1..=2 {
            p.tick(
                t0 + Duration::from_secs(n),
                picture::Link {
                    blocked: blocked[15 + n as usize],
                    sent_kbps: 9600.0,
                    delivered_kbps: 3000.0,
                    reset: false,
                    queue_ms: 200,
                },
                picture::Cpu::default(),
            );
        }
        assert_eq!(p.plan(t0 + Duration::from_secs(2), BIG).scale, Scale::Half);
    }

    #[test]
    fn tests_that_pacing_tolerates_recent_ack_jitter() {
        let mut v = viewer();
        let now = Instant::now();
        for (seq, delay) in [(1, 55), (2, 70), (3, 100)] {
            v.inflight.push_back(Sent {
                seq,
                at: now - Duration::from_millis(delay),
                bytes: 200,
                burst: false,
            });
            v.on_ack(seq, now);
        }
        v.inflight.push_back(Sent {
            seq: 4,
            at: now - Duration::from_millis(95),
            bytes: 200,
            burst: false,
        });
        assert_eq!(v.rtt.get(), Duration::from_millis(55));
        assert!(v.open(now, Duration::from_millis(33)));
    }

    #[test]
    fn tests_that_jitter_slack_stays_bounded_and_preserves_the_frame_limit() {
        let mut v = viewer();
        let now = Instant::now();
        v.rtt.push(20, now);
        v.rtt.push(500, now);
        v.inflight.push_back(Sent {
            seq: 1,
            at: now - Duration::from_millis(169),
            bytes: 200,
            burst: false,
        });
        assert!(v.open(now, Duration::from_millis(33)));
        assert!(!v.open(now + Duration::from_millis(1), Duration::from_millis(33)));
        v.inflight.clear();
        for seq in 0..MAX_INFLIGHT as u32 {
            v.inflight.push_back(Sent {
                seq,
                at: now,
                bytes: 200,
                burst: false,
            });
        }
        assert!(!v.open(now, Duration::from_millis(33)));
    }

    #[test]
    fn tests_that_pacing_relearns_a_stable_link_after_old_jitter_expires() {
        let mut v = viewer();
        let now = Instant::now();
        v.rtt.push(20, now);
        v.rtt.push(120, now);
        let later = now + Duration::from_millis(5001);
        v.rtt.push(20, later);
        v.inflight.push_back(Sent {
            seq: 1,
            at: later - Duration::from_millis(90),
            bytes: 200,
            burst: false,
        });
        assert!(v.open(later, Duration::from_millis(33)));
        let recovered = later + Duration::from_millis(5001);
        v.rtt.push(20, recovered);
        v.inflight.front_mut().unwrap().at = recovered - Duration::from_millis(60);
        assert!(!v.open(recovered, Duration::from_millis(33)));
    }

    #[test]
    fn tests_that_pacing_obeys_rtt_and_inflight_limit() {
        let rtt = Duration::from_millis(50);
        let interval = Duration::from_millis(33);
        assert!(window_open(0, None, rtt, interval));
        assert!(window_open(
            1,
            Some(Duration::from_millis(82)),
            rtt,
            interval
        ));
        assert!(!window_open(
            1,
            Some(Duration::from_millis(83)),
            rtt,
            interval
        ));
        assert!(!window_open(
            MAX_INFLIGHT,
            Some(Duration::ZERO),
            rtt,
            interval
        ));
        assert!(!window_open(1, Some(ACK_TIMEOUT), ACK_TIMEOUT, interval));
    }

    #[test]
    fn tests_that_stalled_spectator_does_not_pace_owner() {
        let (mut spectator, _view_peer) = viewer_pair(1, true);
        let (owner, _owner_peer) = viewer_pair(2, false);
        let now = Instant::now();
        spectator.inflight.push_back(Sent {
            seq: 1,
            at: now - ACK_TIMEOUT,
            bytes: 100,
            burst: false,
        });
        let mut viewers = vec![spectator, owner];
        assert_eq!(pacer_index(&viewers), Some(1));
        assert!(viewers[pacer_index(&viewers).unwrap()].open(now, Duration::from_millis(33)));
        expire_viewers(&mut viewers, now);
        assert_eq!(viewers.len(), 1);
        assert_eq!(viewers[0].id, 2);
    }

    #[test]
    fn tests_that_new_owner_replaces_old_and_view_limit_is_enforced() {
        let mut viewers = Vec::new();
        let mut retiring = Vec::new();
        let (spectator, _view_peer) = viewer_pair(1, true);
        let (extra, _extra_peer) = viewer_pair(2, true);
        let (owner, _owner_peer) = viewer_pair(3, false);
        let (new_owner, _new_peer) = viewer_pair(4, false);
        assert!(admit_viewer(&mut viewers, &mut retiring, spectator, 1));
        assert!(!admit_viewer(&mut viewers, &mut retiring, extra, 1));
        assert!(admit_viewer(&mut viewers, &mut retiring, owner, 1));
        assert!(admit_viewer(&mut viewers, &mut retiring, new_owner, 1));
        assert_eq!(viewers.iter().map(|v| v.id).collect::<Vec<_>>(), [1, 4]);
        let (disabled, _disabled_peer) = viewer_pair(5, true);
        assert!(!admit_viewer(&mut Vec::new(), &mut Vec::new(), disabled, 0));
    }

    #[test]
    fn tests_that_replaced_owner_receives_view_only_notice_before_eof() {
        use std::io::Read;
        let (owner, mut peer) = viewer_pair(1, false);
        peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let mut viewers = Vec::new();
        let mut retiring = Vec::new();
        assert!(admit_viewer(&mut viewers, &mut retiring, owner, 8));
        let mut hello = [0; 7];
        peer.read_exact(&mut hello).unwrap();
        assert_eq!(&hello, &wire::hello(false)[..]);
        let (replacement, _new_peer) = viewer_pair(2, false);
        assert!(admit_viewer(&mut viewers, &mut retiring, replacement, 8));
        peer.read_exact(&mut hello).unwrap();
        assert_eq!(&hello, &wire::hello(true)[..]);
        assert_eq!(retiring.len(), 1);
        retiring.clear();
        assert_eq!(peer.read(&mut hello).unwrap(), 0);
    }

    #[test]
    fn tests_that_repeated_owner_takeovers_bound_retiring_connections() {
        use std::io::Read;
        let mut viewers = Vec::new();
        let mut retiring = Vec::new();
        let mut peers = Vec::new();
        for id in 0..(MAX_RETIRING as u64 + 2) {
            let (owner, peer) = viewer_pair(id, false);
            peers.push(peer);
            assert!(admit_viewer(&mut viewers, &mut retiring, owner, 8));
        }
        assert_eq!(viewers.len(), 1);
        assert_eq!(retiring.len(), MAX_RETIRING);
        peers[0]
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut discarded = Vec::new();
        peers[0].read_to_end(&mut discarded).unwrap();
        assert!(
            retiring
                .iter()
                .all(|r| r.until <= Instant::now() + ACK_TIMEOUT)
        );
    }

    #[test]
    fn tests_that_write_queue_is_bounded_without_reading_peer() {
        let (viewer, _peer) = viewer_pair(1, true);
        let record: Arc<[u8]> = vec![0; 4 * 1024 * 1024].into();
        let mut rejected = false;
        for _ in 0..100 {
            if !viewer.queue(record.clone()) {
                rejected = true;
                break;
            }
        }
        assert!(rejected);
    }

    #[test]
    fn tests_that_an_expired_window_stays_closed() {
        let mut v = viewer();
        let now = Instant::now();
        v.inflight.push_back(Sent {
            seq: 1,
            at: now - ACK_TIMEOUT - Duration::from_millis(1),
            bytes: 100,
            burst: false,
        });
        assert!(!v.open(now, Duration::from_millis(33)));
        assert_eq!(v.inflight.len(), 1);
    }

    #[test]
    fn tests_that_unsent_ack_cannot_release_frames() {
        let mut v = viewer();
        let now = Instant::now();
        v.inflight.push_back(Sent {
            seq: 42,
            at: now,
            bytes: 100,
            burst: false,
        });
        v.on_ack(43, now);
        assert_eq!(v.inflight.len(), 1);
        assert_eq!(v.acked_bytes, 0);
    }

    #[test]
    fn tests_that_ack_wraparound_and_duplicates_are_safe() {
        let mut v = viewer();
        let now = Instant::now();
        for seq in [u32::MAX, 0, 1] {
            v.inflight.push_back(Sent {
                seq,
                at: now,
                bytes: 100,
                burst: false,
            });
        }
        v.on_ack(0, now);
        assert_eq!(v.inflight.front().unwrap().seq, 1);
        assert_eq!(v.acked_bytes, 200);
        v.on_ack(0, now);
        assert_eq!(v.acked_bytes, 200);
    }
    #[test]
    fn tests_that_burst_wait_expires_at_expected_transfer_time() {
        for (capacity, deadline_ms) in [(Some(3000.0), 400), (None, 1000)] {
            let t = Instant::now();
            let mut v = viewer();
            v.inflight.push_back(Sent {
                seq: 1,
                at: t,
                bytes: 150_000,
                burst: true,
            });
            let interval = Duration::from_millis(33);
            assert!(!v.link_blocked(
                t + Duration::from_millis(deadline_ms - 1),
                interval,
                capacity
            ));
            assert!(v.link_blocked(t + Duration::from_millis(deadline_ms), interval, capacity));
        }
    }

    #[test]
    fn tests_that_engine_wakes_at_burst_expiry_without_input_or_ack() {
        for (capacity, expected) in [(Some(3000.0), [500, 1000]), (None, [0, 900])] {
            let t = Instant::now();
            let mut v = viewer();
            v.rtt.push(30, t);
            v.inflight.push_back(Sent {
                seq: 1,
                at: t + Duration::from_millis(100),
                bytes: 150_000,
                burst: true,
            });
            let interval = Duration::from_millis(33);
            let mut now = t + Duration::from_millis(300);
            assert_eq!(
                v.window_deadline(now, interval, capacity, false),
                Some(t + Duration::from_millis(3100))
            );
            let mut tick_at = t + Duration::from_secs(1);
            let mut blocked_since = None;
            let mut ticks = Vec::new();
            // The production loop ticks before observing blockage, then sleeps until
            // an event or the earlier of the tick/window deadlines. No periodic polling.
            while ticks.len() < 2 {
                if now >= tick_at {
                    ticks.push(blocked_since.map_or(Duration::ZERO, |at| now - at));
                    blocked_since = blocked_since.map(|_| now);
                    tick_at = now + Duration::from_secs(1);
                }
                if v.link_blocked(now, interval, capacity) {
                    blocked_since.get_or_insert(now);
                }
                now = tick_at.min(v.window_deadline(now, interval, capacity, true).unwrap());
            }
            assert_eq!(ticks, expected.map(Duration::from_millis));
        }
    }

    #[test]
    fn tests_that_late_ack_counts_delivery_once_without_releasing_new_credit() {
        let t = Instant::now();
        let mut v = viewer();
        queued(&mut v, 1, t);
        queued(&mut v, 2, t);
        v.reset_window();
        queued(&mut v, 3, t + Duration::from_secs(3));
        v.on_ack(1, t + Duration::from_secs(4));
        assert_eq!(v.acked_bytes, 100);
        assert_eq!(v.inflight.front().unwrap().seq, 3);
        v.on_ack(1, t + Duration::from_secs(5));
        assert_eq!(v.acked_bytes, 100);
        v.on_ack(3, t + Duration::from_secs(6));
        assert_eq!(v.acked_bytes, 300);
        assert!(v.inflight.is_empty());
    }

    #[derive(Debug)]
    struct PacedTick {
        crf: f32,
        scale: Scale,
        sent: f32,
        delivered: f32,
    }
    struct PacedFrame {
        at: Duration,
        scale: Scale,
        key: bool,
        bytes: usize,
    }
    #[derive(Default)]
    struct PacedTrace {
        ticks: Vec<PacedTick>,
        frames: Vec<PacedFrame>,
        resets: Vec<Duration>,
    }

    // Real Viewer pacing/ACK/timeout code and Picture policy, with a serial link and a
    // deterministic encoder-size model. An optional initial burst replays queued log facts.
    fn paced_picture(
        seconds: u64,
        kbps: u64,
        key_bytes: usize,
        motion_bytes: usize,
        initial_keys: u32,
        prior_capacity: Option<f32>,
    ) -> PacedTrace {
        let t = Instant::now();
        let mut p = motion(t, false);
        if let Some(cap) = prior_capacity {
            p.probe(
                t,
                100_000,
                Duration::from_secs_f32(800.0 / cap),
                Duration::ZERO,
            );
        }
        let (owner, _peer) = viewer_pair(1, false);
        let mut viewers = vec![owner];
        let (mut seq, mut last_send, mut sent, mut blocked, mut reset) =
            (0, None::<Instant>, 0_usize, 0.0, false);
        let (mut link_free, mut acks) = (t, VecDeque::<(Instant, u32)>::new());
        let mut trace = PacedTrace::default();
        for ms in 0..=seconds * 1000 {
            let now = t + Duration::from_millis(ms);
            while let Some(&(at, ack)) = acks.front()
                && at <= now
            {
                acknowledge(&mut viewers, &mut p, 1, ack, at);
                acks.pop_front();
            }
            if expire_viewers(&mut viewers, now) {
                reset = true;
                trace.resets.push(now - t);
            }
            assert_eq!(viewers.len(), 1, "owner disconnected at {ms} ms");
            let interval = Duration::from_secs_f32(1.0 / p.fps());
            if viewers[0].link_blocked(now, interval, p.capacity_kbps(now)) {
                blocked += 0.001;
            }
            if ms > 0 && ms % 1000 == 0 {
                let v = &mut viewers[0];
                let delivered = std::mem::take(&mut v.acked_bytes) as f32 * 8.0 / 1000.0;
                let mut delays = std::mem::take(&mut v.delays_ms);
                delays.sort_unstable();
                let queue_ms = delays
                    .get(delays.len() / 2)
                    .map_or(0, |d| d.saturating_sub(v.rtt.get().as_millis() as u32));
                let sent_kbps = sent as f32 * 8.0 / 1000.0;
                p.tick(
                    now,
                    picture::Link {
                        reset,
                        blocked,
                        sent_kbps,
                        delivered_kbps: delivered,
                        queue_ms,
                    },
                    picture::Cpu::default(),
                );
                trace.ticks.push(PacedTick {
                    crf: p.crf(),
                    scale: p.plan(now, BIG).scale,
                    sent: sent_kbps,
                    delivered,
                });
                (sent, blocked, reset) = (0, 0.0, false);
            }
            let replay = seq < initial_keys && ms < 1000;
            if (initial_keys == 0 || replay || ms >= 1000)
                && viewers[0].open(now, interval)
                && last_send.is_none_or(|last| now >= last + interval)
            {
                let owner_key = replay || viewers[0].needs_key;
                let plan = frame_plan(&mut p, now, BIG, owner_key);
                let key = owner_key || plan.scale != p.scale();
                let base = if key { key_bytes } else { motion_bytes };
                // Keep offered motion demand fixed so the 3x jam remains 3x after
                // a quality step. Keyframe estimates still shrink with quality.
                let quality = if key {
                    2.0_f32.powf((24.0 - p.crf()) / 6.0)
                } else {
                    1.0
                };
                let size = (base as f32 * quality
                    / if plan.scale == Scale::Half { 4.0 } else { 1.0 })
                .ceil() as usize;
                let full_return = p.scale() == Scale::Half && plan.scale == Scale::Full;
                p.encoded(
                    now,
                    picture::Encoded {
                        plan,
                        key,
                        owner_key,
                        bytes: size,
                        pixels: 780 * ROWS / if plan.scale == Scale::Half { 4 } else { 1 },
                        big: true,
                        work: Work {
                            encode: Duration::from_millis(10),
                            ..Work::default()
                        },
                    },
                );
                seq += 1;
                let v = &mut viewers[0];
                v.record_frame(
                    Sent {
                        seq,
                        at: now,
                        bytes: size,
                        burst: key,
                    },
                    full_return && key,
                );
                v.needs_key = false;
                link_free = link_free.max(now) + Duration::from_micros(size as u64 * 8000 / kbps);
                acks.push_back((link_free + Duration::from_millis(30), seq));
                trace.frames.push(PacedFrame {
                    at: now - t,
                    scale: plan.scale,
                    key,
                    bytes: size,
                });
                sent += size;
                last_send = Some(now);
            }
        }
        trace
    }

    #[test]
    fn tests_that_threefold_jam_uses_offered_demand_before_window_throttling() {
        let trace = paced_picture(3, 3000, 40_000, 40_000, 0, None);
        assert!(trace.ticks[0].sent < 9600.0, "{:?}", trace.ticks);
        assert!(trace.ticks[0].delivered < 9600.0);
        assert!(trace.ticks[0].crf >= 30.0, "{:?}", trace.ticks);
        assert_eq!(trace.ticks[1].scale, Scale::Half, "{:?}", trace.ticks);
    }

    #[test]
    fn tests_that_a_point_three_megabit_link_escapes_the_keyframe_reset_loop() {
        let trace = paced_picture(30, 300, 150_000, 4000, 0, None);
        assert!(trace.ticks.iter().any(|tick| tick.crf >= 32.0));
        assert!(trace.frames.iter().any(|frame| frame.scale == Scale::Half));
        assert!(
            trace
                .frames
                .iter()
                .filter(|frame| frame.key)
                .any(|frame| frame.bytes < 37_500)
        );
        assert!(
            // Accepted R3 permits a later full-size probe, but not a repeated reset loop.
            trace
                .resets
                .iter()
                .filter(|at| **at >= Duration::from_secs(20))
                .count()
                <= 1,
            "{:?}",
            trace.resets
        );
        assert!(
            trace
                .frames
                .iter()
                .filter(|frame| frame.at > Duration::from_secs(20))
                .count()
                > 30
        );
    }

    #[test]
    fn tests_that_megabyte_keyframe_jam_reaches_half_on_the_second_tick() {
        // Offer the recorded six ~1 MB keys through admission, with a stale ~859 Mb/s probe.
        let trace = paced_picture(4, 3000, 1_000_000, 1_000_000, 6, Some(859_464.0));
        assert!(trace.ticks[0].crf >= 32.0, "{:?}", trace.ticks);
        assert_eq!(trace.ticks[1].scale, Scale::Half, "{:?}", trace.ticks);
        // A 1 MB frame alone takes 2.67 s on the wire; apply the decision on its ACK.
        let half = trace
            .frames
            .iter()
            .find(|frame| frame.scale == Scale::Half)
            .unwrap();
        assert!(half.at < Duration::from_secs(3));
        assert!(half.bytes <= 125_000);
    }
    #[test]
    fn tests_that_newer_bursts_cannot_hide_an_older_stalled_frame() {
        let t = Instant::now();
        let mut v = viewer();
        queued(&mut v, 1, t);
        v.inflight.push_back(Sent {
            seq: 2,
            at: t + Duration::from_millis(300),
            bytes: 150_000,
            burst: true,
        });
        assert!(v.link_blocked(
            t + Duration::from_millis(400),
            Duration::from_millis(33),
            Some(3000.0)
        ));
    }

    #[test]
    fn tests_that_burst_grace_is_capped_and_used_only_once_per_reset() {
        for ack_ms in [100, 1100] {
            let t = Instant::now();
            let mut v = viewer();
            v.rtt.push(30, t);
            let interval = Duration::from_millis(33);
            v.inflight.push_back(Sent {
                seq: 1,
                at: t,
                bytes: 150_000,
                burst: true,
            });
            if ack_ms > 1000 {
                assert!(v.link_blocked(t + Duration::from_secs(1), interval, Some(100.0)));
            }
            let ack = t + Duration::from_millis(ack_ms);
            v.on_ack(1, ack);
            v.inflight.push_back(Sent {
                seq: 2,
                at: ack,
                bytes: 150_000,
                burst: true,
            });
            assert!(v.link_blocked(ack + Duration::from_millis(250), interval, Some(3000.0)));
            v.reset_window();
            v.inflight.push_back(Sent {
                seq: 3,
                at: ack + Duration::from_millis(500),
                bytes: 150_000,
                burst: true,
            });
            assert!(!v.link_blocked(ack + Duration::from_millis(750), interval, Some(3000.0)));
        }
    }

    #[test]
    fn tests_that_noise_keyframes_do_not_build_a_reset_cascade() {
        let t = Instant::now();
        let (mut owner, _peer) = viewer_pair(1, false);
        owner.rtt.push(2, t);
        let mut viewers = vec![owner];
        let interval = Duration::from_millis(33);
        let (mut seq, mut resets, mut max_keys) = (0, 0, 0);
        let (mut last_send, mut last_ack) = (None::<Instant>, None::<Instant>);
        let (mut link_free, mut acks) = (t, VecDeque::<(Instant, u32)>::new());
        let mut freeze = Duration::ZERO;
        for ms in 0..30_000 {
            let now = t + Duration::from_millis(ms);
            while let Some(&(at, seq)) = acks.front()
                && at <= now
            {
                viewers[0].on_ack(seq, at);
                last_ack = Some(at);
                acks.pop_front();
            }
            if let Some(at) = last_ack {
                freeze = freeze.max(now - at);
            }
            resets += usize::from(expire_viewers(&mut viewers, now));
            assert_eq!(viewers.len(), 1, "owner disconnected at {ms} ms");
            let v = &mut viewers[0];
            if v.open(now, interval) && last_send.is_none_or(|at| now >= at + interval) {
                seq += 1;
                v.record_frame(
                    Sent {
                        seq,
                        at: now,
                        bytes: 250_000,
                        burst: true,
                    },
                    false,
                );
                v.needs_key = false;
                // Full-noise half frames over a serial 3 Mb/s link, plus 2 ms RTT.
                link_free = link_free.max(now) + Duration::from_micros(666_667);
                acks.push_back((link_free + Duration::from_millis(2), seq));
                last_send = Some(now);
            }
            max_keys = max_keys.max(v.inflight.len() + v.reset_frames.len());
        }
        eprintln!(
            "noise pacing: frames={seq}, resets={resets}, max queued keys={max_keys}, longest freeze={freeze:?}"
        );
        assert!(resets <= 1, "{resets} resets");
        assert!(freeze <= Duration::from_secs(1), "{freeze:?}");
        assert_eq!(
            max_keys, 1,
            "never queue a key behind an unacknowledged key"
        );
    }

    #[test]
    fn tests_that_a_window_reset_does_not_forget_an_unacknowledged_key() {
        let t = Instant::now();
        let (mut owner, _peer) = viewer_pair(1, false);
        owner.record_frame(
            Sent {
                seq: 1,
                at: t,
                bytes: 250_000,
                burst: true,
            },
            false,
        );
        let mut viewers = vec![owner];
        let reset_at = t + ACK_TIMEOUT;
        assert!(expire_viewers(&mut viewers, reset_at));
        let v = &mut viewers[0];
        let interval = Duration::from_millis(33);
        assert!(v.needs_key);
        assert!(v.inflight.is_empty());
        assert!(!v.open(reset_at, interval));
        assert_eq!(
            v.window_deadline(reset_at, interval, Some(3000.0), true),
            v.ack_deadline()
        );
        let ack_at = reset_at + Duration::from_millis(500);
        v.on_ack(1, ack_at);
        assert!(v.open(ack_at, interval));
        assert!(v.needs_key, "the next admitted frame still must resync");
    }

    #[test]
    fn tests_that_returning_full_gets_one_keyframe_transfer_without_reentering_half() {
        let t = Instant::now();
        let mut p = motion(t, true);
        let half = p.plan(t, BIG);
        assert_eq!(half.scale, Scale::Half);
        p.encoded(
            t,
            picture::Encoded {
                plan: half,
                key: true,
                owner_key: true,
                bytes: 150_000,
                pixels: 390 * ROWS / 2,
                big: true,
                work: Work::default(),
            },
        );
        p.probe(
            t,
            600_000,
            Duration::from_millis(402),
            Duration::from_millis(2),
        );
        let returned = t + Duration::from_secs(1);
        p.tick(returned, picture::Link::default(), picture::Cpu::default());
        let full = p.plan(returned, BIG);
        assert_eq!(full.scale, Scale::Full);
        let mut v = viewer();
        v.record_frame(
            Sent {
                seq: 1,
                at: t,
                bytes: 100,
                burst: true,
            },
            false,
        );
        v.on_ack(1, t + Duration::from_millis(2));
        v.record_frame(
            Sent {
                seq: 2,
                at: returned,
                bytes: 600_000,
                burst: true,
            },
            p.scale() == Scale::Half && full.scale == Scale::Full,
        );
        p.encoded(
            returned,
            picture::Encoded {
                plan: full,
                key: true,
                owner_key: false,
                bytes: 600_000,
                pixels: 780 * ROWS,
                big: true,
                work: Work::default(),
            },
        );
        let interval = Duration::from_millis(33);
        let mut blocked = 0.0;
        for ms in 0..402 {
            if v.link_blocked(
                returned + Duration::from_millis(ms),
                interval,
                Some(12_000.0),
            ) {
                blocked += 0.001;
            }
        }
        v.on_ack(2, returned + Duration::from_millis(402));
        let now = returned + Duration::from_secs(1);
        p.tick(
            now,
            picture::Link {
                blocked,
                sent_kbps: 4800.0,
                delivered_kbps: 4800.0,
                ..picture::Link::default()
            },
            picture::Cpu::default(),
        );
        assert_eq!(p.plan(now, BIG).scale, Scale::Full);
        v.record_frame(
            Sent {
                seq: 3,
                at: now,
                bytes: 600_000,
                burst: true,
            },
            false,
        );
        assert!(
            v.link_blocked(now + Duration::from_millis(250), interval, Some(12_000.0)),
            "an ordinary later key must not renew the full-return grace"
        );
    }

    #[test]
    fn tests_that_trace_is_opt_in_and_timestamps_the_skip_reason() {
        let t = Instant::now();
        for setting in [None, Some("0"), Some("true"), Some("1")] {
            let mut log = Vec::new();
            Trace::new(setting, t).write(
                &mut log,
                t + Duration::from_millis(535),
                format_args!("event=loop reason=window_closed damage=true"),
            );
            let log = String::from_utf8(log).unwrap();
            if setting == Some("1") {
                assert_eq!(
                    log,
                    "slimviewer: trace at_ms=535.000 event=loop reason=window_closed damage=true\n"
                );
            } else {
                assert!(log.is_empty());
            }
        }
    }

    #[test]
    fn tests_that_steady_small_damage_has_no_two_hundred_ms_gap_for_sixty_seconds() {
        let t = Instant::now();
        let mut p = motion(t, false);
        // Expire this probe at 10s; ACK floor/spread rotates every 5s.
        p.probe(
            t,
            60_000,
            Duration::from_millis(110),
            Duration::from_millis(70),
        );
        let (mut v, _peer) = viewer_pair(1, false);
        v.rtt.push(30, t);
        let interval = Duration::from_secs_f32(1.0 / 30.0);
        let (mut now, mut damage_at, mut tick_at) = (t, t, t + Duration::from_secs(1));
        let mut last_send = t - interval;
        let (mut dirty, mut seq) = (false, 0);
        let mut acks = VecDeque::<(Instant, u32)>::new();
        let mut max_gap = Duration::ZERO;
        let (mut blocked, mut blocked_since) = (Duration::ZERO, None::<Instant>);
        while now < t + Duration::from_secs(60) {
            while let Some(&(at, ack)) = acks.front()
                && at <= now
            {
                v.on_ack(ack, at);
                for (bytes, delay, rtt) in v.probes.drain(..) {
                    p.probe(at, bytes, delay, rtt);
                }
                acks.pop_front();
            }
            if now >= damage_at {
                dirty = true;
                damage_at += interval;
                p.input(now);
            }
            if now >= tick_at {
                if let Some(at) = blocked_since {
                    blocked += now - at;
                    blocked_since = Some(now);
                }
                p.tick(
                    now,
                    picture::Link {
                        blocked: std::mem::take(&mut blocked).as_secs_f32(),
                        delivered_kbps: std::mem::take(&mut v.acked_bytes) as f32 * 8.0 / 1000.0,
                        ..picture::Link::default()
                    },
                    picture::Cpu::default(),
                );
                tick_at += Duration::from_secs(1);
            }
            let is_blocked = dirty && v.link_blocked(now, interval, p.capacity_kbps(now));
            match (is_blocked, blocked_since) {
                (true, None) => blocked_since = Some(now),
                (false, Some(at)) => {
                    blocked += now - at;
                    blocked_since = None;
                }
                _ => {}
            }
            match frame_action(&p, now, last_send, dirty, v.open(now, interval)) {
                FrameAction::Grab | FrameAction::Still(_) => {
                    let plan = if dirty {
                        frame_plan(&mut p, now, SMALL, v.needs_key)
                    } else {
                        p.still(now).unwrap()
                    };
                    assert_eq!(plan.scale, Scale::Full);
                    let key = v.needs_key || plan.sharp_qp.is_some();
                    let bytes = if key { 8000 } else { 2000 };
                    p.encoded(
                        now,
                        picture::Encoded {
                            plan,
                            key,
                            owner_key: v.needs_key,
                            bytes,
                            pixels: 780 * ROWS,
                            big: false,
                            work: Work {
                                encode: Duration::from_millis(12),
                                ..Work::default()
                            },
                        },
                    );
                    if seq > 0 {
                        max_gap = max_gap.max(now - last_send);
                    }
                    seq += 1;
                    last_send = now;
                    dirty = false;
                    // Capture/encode takes 12ms; ordered ACKs see 70 +/- 40ms jitter.
                    now += Duration::from_millis(12);
                    v.record_frame(
                        Sent {
                            seq,
                            at: now,
                            bytes,
                            burst: key,
                        },
                        false,
                    );
                    v.needs_key = false;
                    let ack_at =
                        now + Duration::from_millis([30, 110, 50, 90, 70][seq as usize % 5]);
                    let ack_at = acks
                        .back()
                        .map_or(ack_at, |(at, _)| ack_at.max(*at + Duration::from_millis(1)));
                    acks.push_back((ack_at, seq));
                }
                FrameAction::Wait(_) => {}
            }
            // The same send deadline as Engine::wait, interrupted by real event times.
            let mut wake = damage_at.min(tick_at);
            if let Some(&(ack_at, _)) = acks.front() {
                wake = wake.min(ack_at);
            }
            if let Some(deadline) = frame_deadline(&p, now, last_send, dirty, &v) {
                wake = wake.min(deadline);
            }
            now = wake.max(now + Duration::from_millis(1));
        }
        max_gap = max_gap.max(now - last_send);
        eprintln!(
            "steady small damage: frames={seq}, max send gap={max_gap:?}, crf={}",
            p.crf()
        );
        assert!(seq > 1500, "{seq} frames");
        assert!(max_gap <= Duration::from_millis(200), "{max_gap:?}");
        assert_eq!(p.crf(), 24.0);
    }

    #[test]
    fn tests_that_a_slower_spectator_keeps_p_frames_without_periodic_resync() {
        let t = Instant::now();
        let (mut owner, _owner_peer) = viewer_pair(1, false);
        let (mut spectator, _view_peer) = viewer_pair(2, true);
        owner.rtt.push(5, t);
        spectator.rtt.push(60, t);
        let mut viewers = [owner, spectator];
        let mut acks = [VecDeque::<(Instant, u32)>::new(), VecDeque::new()];
        let interval = Duration::from_secs_f32(1.0 / 30.0);
        let (mut last_send, mut last_key) = (t - interval, t);
        let (mut seq, mut keys, mut spectator_p_frames) = (0, 0, 0);
        for ms in 0..10_000 {
            let now = t + Duration::from_millis(ms);
            for (viewer, queue) in viewers.iter_mut().zip(&mut acks) {
                while let Some(&(at, ack)) = queue.front()
                    && at <= now
                {
                    viewer.on_ack(ack, at);
                    queue.pop_front();
                }
            }
            let pacer = pacer_index(&viewers).unwrap();
            if !viewers[pacer].open(now, interval) || now < last_send + interval {
                continue;
            }
            let key = seq == 0
                || (viewers.iter().any(|v| v.needs_key) && now > last_key + Duration::from_secs(1));
            seq += 1;
            keys += usize::from(key);
            if key {
                last_key = now;
            }
            last_send = now;
            for (index, viewer) in viewers.iter_mut().enumerate() {
                // The same admission/resync contract as Engine::send.
                if !viewer.open(now, interval) {
                    viewer.needs_key = true;
                    continue;
                }
                if key {
                    viewer.needs_key = false;
                } else if viewer.needs_key {
                    continue;
                }
                spectator_p_frames += usize::from(viewer.view_only && !key);
                viewer.record_frame(
                    Sent {
                        seq,
                        at: now,
                        bytes: 1000,
                        burst: key,
                    },
                    false,
                );
                acks[index].push_back((
                    now + Duration::from_millis(if index == pacer { 5 } else { 60 }),
                    seq,
                ));
            }
        }
        eprintln!(
            "spectator replay: owner frames={seq}, keys={keys}, spectator P-frames={spectator_p_frames}"
        );
        assert!(spectator_p_frames > 280, "{spectator_p_frames} P-frames");
        assert_eq!(keys, 1, "only the initial key; no periodic resync");
        assert!(!viewers[1].needs_key);
    }

    fn jam_then_still(clear_link: bool, stop_ms: u64, half_bytes: usize) -> Duration {
        let t = Instant::now();
        let stop = t + Duration::from_millis(stop_ms);
        let mut p = motion(t, true);
        changed(&mut p, t, BIG);
        let (mut owner, _peer) = viewer_pair(1, false);
        owner.rtt.push(2, t);
        let mut viewers = vec![owner];
        let interval = Duration::from_secs_f32(1.0 / 30.0);
        let mut wire = VecDeque::<(u32, usize, Scale)>::new();
        let mut acks = VecDeque::<(Instant, u32, Scale)>::new();
        let (mut seq, mut resets, mut queued_keys) = (0, 0, 0);
        let mut last_send = t - interval;
        let mut dirty = true;
        let (mut first_full, mut first_sharp, mut shown_full) = (None, None, None);
        for ms in 0..stop_ms + 5_000 {
            let now = t + Duration::from_millis(ms);
            let jammed = now < stop || (!clear_link && ms < stop_ms + 2_000);
            let mut credit = if jammed { 375 } else { 12_500 };
            while credit > 0 && !wire.is_empty() {
                let (_, bytes, _) = wire.front_mut().unwrap();
                let sent = credit.min(*bytes);
                credit -= sent;
                *bytes -= sent;
                if *bytes == 0 {
                    let (seq, _, scale) = wire.pop_front().unwrap();
                    acks.push_back((now + Duration::from_millis(2), seq, scale));
                }
            }
            while let Some(&(at, seq, scale)) = acks.front()
                && at <= now
            {
                acknowledge(&mut viewers, &mut p, 1, seq, at);
                if now >= stop && scale == Scale::Full {
                    shown_full.get_or_insert(now - stop);
                }
                acks.pop_front();
            }
            if ms % 1000 == 0 {
                p.tick(
                    now,
                    picture::Link {
                        blocked: if jammed { 0.8 } else { 0.0 },
                        sent_kbps: 8000.0,
                        delivered_kbps: 3000.0,
                        queue_ms: if jammed { 250 } else { 0 },
                        ..picture::Link::default()
                    },
                    picture::Cpu::default(),
                );
            }
            if ms == stop_ms + 2_000 {
                // A fresh capacity observation eventually permits sharpening too.
                p.probe(
                    now,
                    100_000,
                    Duration::from_millis(10),
                    Duration::from_millis(2),
                );
            }
            resets += usize::from(expire_viewers(&mut viewers, now));
            assert_eq!(viewers.len(), 1, "owner disconnected at {ms}ms");
            if now < stop {
                dirty = true;
                p.damage(now);
            }
            let v = &mut viewers[0];
            let pending = dirty || v.needs_key;
            let (plan, big) = match frame_action(&p, now, last_send, pending, v.open(now, interval))
            {
                FrameAction::Grab => {
                    // The final ungrabbed motion survives scroll stop.
                    let band = if dirty { BIG } else { None };
                    dirty = false;
                    let plan = frame_plan(&mut p, now, band, v.needs_key);
                    if band.is_none() && !v.needs_key {
                        continue;
                    }
                    if !v.needs_key && p.defer_half(now, plan) {
                        continue;
                    }
                    (plan, band.is_some())
                }
                FrameAction::Still(plan) => (plan, false),
                FrameAction::Wait(_) => continue,
            };
            let bytes = if plan.scale == Scale::Half {
                half_bytes
            } else {
                half_bytes * 4
            };
            let returning_full = p.scale() == Scale::Half && plan.scale == Scale::Full;
            if now >= stop && plan.scale == Scale::Full {
                first_full.get_or_insert((now - stop, plan.sharp_qp));
                if plan.sharp_qp.is_some() {
                    first_sharp.get_or_insert(now - stop);
                }
            }
            p.encoded(
                now,
                picture::Encoded {
                    plan,
                    key: true,
                    owner_key: v.needs_key,
                    bytes,
                    pixels: 780 * ROWS / if plan.scale == Scale::Half { 4 } else { 1 },
                    big,
                    work: Work::default(),
                },
            );
            seq += 1;
            v.record_frame(
                Sent {
                    seq,
                    at: now,
                    bytes,
                    burst: true,
                },
                returning_full,
            );
            v.needs_key = false;
            wire.push_back((seq, bytes, plan.scale));
            last_send = now;
            queued_keys = queued_keys.max(wire.len());
        }
        let (full, qp) = first_full.expect("still frame must return to full size");
        if stop_ms == 30_000 || full > Duration::from_millis(1200) {
            eprintln!(
                "jam then still clear={clear_link} stop={stop_ms} bytes={half_bytes}: full={first_full:?}, shown={shown_full:?}, sharp={first_sharp:?}, resets={resets}, max keys={queued_keys}"
            );
        }
        assert!(
            full <= Duration::from_millis(1200),
            "stop={stop_ms}, bytes={half_bytes}, full={full:?}"
        );
        assert!(resets <= 1, "{resets} resets");
        assert_eq!(queued_keys, 1);
        if clear_link {
            assert!(shown_full.unwrap() <= Duration::from_millis(1200));
        } else {
            assert_eq!(qp, None, "full size must not depend on sharp budget");
            assert!(first_sharp.is_some_and(|at| at >= Duration::from_secs(2)));
        }
        assert!(first_sharp.is_some_and(|at| at < Duration::from_secs(5)));
        full
    }

    #[test]
    fn tests_that_a_still_screen_returns_full_when_the_jam_clears() {
        for bytes in [250_000, 148_000] {
            let worst = (29_437..=30_105)
                .map(|stop| jam_then_still(true, stop, bytes))
                .max()
                .unwrap();
            eprintln!("cleared-link still sweep: bytes={bytes}, worst full={worst:?}");
        }
    }

    #[test]
    fn tests_that_a_still_screen_returns_full_before_sharpening_on_a_jammed_link() {
        for bytes in [250_000, 148_000] {
            let worst = (29_437..=30_105)
                .map(|stop| jam_then_still(false, stop, bytes))
                .max()
                .unwrap();
            eprintln!("constrained-link still sweep: bytes={bytes}, worst full={worst:?}");
        }
    }

    #[test]
    fn tests_that_a_still_screen_returns_full_during_cpu_backoff() {
        let t = Instant::now();
        let mut p = motion(t, false);
        let (mut returns, mut stopped) = (0, None);
        // Four failed returns grow the real calm requirement from 3 to 48 ticks.
        for second in 1..=300 {
            let now = t + Duration::from_secs(second);
            let plan = p.plan(now, BIG);
            returns += usize::from(p.scale() == Scale::Half && plan.scale == Scale::Full);
            for _ in 0..6 {
                p.encoded(
                    now,
                    picture::Encoded {
                        plan,
                        key: false,
                        owner_key: false,
                        bytes: 2000,
                        pixels: 780 * ROWS / if plan.scale == Scale::Half { 4 } else { 1 },
                        big: true,
                        work: Work {
                            grab: Duration::from_millis(2),
                            encode: Duration::from_millis(if plan.scale == Scale::Full {
                                58
                            } else {
                                9
                            }),
                            ..Work::default()
                        },
                    },
                );
            }
            p.tick(now, picture::Link::default(), picture::Cpu::default());
            if returns == 4 && plan.scale == Scale::Half {
                stopped = Some(now);
                break;
            }
        }
        let stop = stopped.expect("four failed CPU returns must reach 48-tick backoff");
        assert_eq!(p.scale_label(), "half(cpu)");
        let half = p.plan(stop, BIG);
        assert_eq!(half.scale, Scale::Half);
        p.encoded(
            stop,
            picture::Encoded {
                plan: half,
                key: true,
                owner_key: false,
                bytes: 250_000,
                pixels: 390 * ROWS / 2,
                big: true,
                work: Work::default(),
            },
        );
        let (mut owner, _peer) = viewer_pair(1, false);
        owner.rtt.push(2, stop);
        owner.record_frame(
            Sent {
                seq: 1,
                at: stop,
                bytes: 250_000,
                burst: true,
            },
            false,
        );
        owner.needs_key = false;
        let mut viewers = vec![owner];
        let interval = Duration::from_secs_f32(1.0 / 30.0);
        let (mut last_send, mut full_sent) = (stop, None);
        for ms in 0..=2100 {
            let now = stop + Duration::from_millis(ms);
            if ms == 669 {
                acknowledge(&mut viewers, &mut p, 1, 1, now);
            }
            if ms > 0 && ms % 1000 == 0 {
                p.tick(
                    now,
                    picture::Link::default(),
                    picture::Cpu {
                        throttled: true,
                        ..picture::Cpu::default()
                    },
                );
            }
            if let FrameAction::Still(plan) =
                frame_action(&p, now, last_send, false, viewers[0].open(now, interval))
            {
                assert_eq!(plan.scale, Scale::Full);
                assert_eq!(
                    plan.sharp_qp, None,
                    "full size must not depend on sharpening"
                );
                let sent = now + Duration::from_millis(60);
                p.encoded(
                    sent,
                    picture::Encoded {
                        plan,
                        key: true,
                        owner_key: false,
                        bytes: 1_000_000,
                        pixels: 780 * ROWS,
                        big: false,
                        work: Work {
                            encode: Duration::from_millis(60),
                            ..Work::default()
                        },
                    },
                );
                viewers[0].record_frame(
                    Sent {
                        seq: 2,
                        at: sent,
                        bytes: 1_000_000,
                        burst: true,
                    },
                    true,
                );
                full_sent = Some(sent - stop);
                last_send = sent;
            }
        }
        eprintln!("CPU backoff still: four failed returns, full sent={full_sent:?}");
        assert!(full_sent.is_some_and(|at| at >= Duration::from_secs(2) && at <= Duration::from_millis(2100)));
        assert_eq!(
            p.plan(stop + Duration::from_millis(2100), BIG).scale,
            Scale::Half,
            "CPU backoff must still govern resumed big motion"
        );
    }
}
