//! One ordered owner of the browser socket. Capture never waits on CDP.

use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError};
use std::time::{Duration, Instant};

use super::{
    paste,
    session::{Changes, Session},
};
use crate::engine::Ev;

const PASTE_BUDGET: Duration = Duration::from_millis(1200);
const CONNECT_BUDGET: Duration = Duration::from_millis(750);
const EVENT_BUDGET: Duration = Duration::from_millis(100);
const FIELD_RETRY: Duration = Duration::from_millis(50);
const FIELD_RESEND: Duration = Duration::from_secs(2);
const FIRST_RECONNECT: Duration = Duration::from_millis(250);
const MAX_RECONNECT: Duration = Duration::from_secs(5);
const QUEUED: u8 = 0;
const STARTED: u8 = 1;
const CANCELLED: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PasteOutcome {
    Pasted = 0,
    NotLanded = 1,
    NoField = 2,
    Refused = 3,
    Unconfirmed = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldBox {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fields {
    width: u16,
    height: u16,
    focused: Option<u8>,
    boxes: Vec<FieldBox>,
}

impl Fields {
    pub(crate) fn new(
        width: u16,
        height: u16,
        focused: Option<u8>,
        boxes: Vec<FieldBox>,
    ) -> Option<Self> {
        if boxes.len() > 128
            || focused.is_some_and(|i| usize::from(i) >= boxes.len())
            || boxes.iter().any(|b| {
                b.w == 0
                    || b.h == 0
                    || u32::from(b.x) + u32::from(b.w) > u32::from(width)
                    || u32::from(b.y) + u32::from(b.h) > u32::from(height)
            })
        {
            return None;
        }
        Some(Self {
            width,
            height,
            focused,
            boxes,
        })
    }

    pub fn width(&self) -> u16 {
        self.width
    }
    pub fn height(&self) -> u16 {
        self.height
    }
    pub fn focused(&self) -> Option<u8> {
        self.focused
    }
    pub fn boxes(&self) -> &[FieldBox] {
        &self.boxes
    }
}

/// Latest button-1 press successfully written to RFB, using Chrome's host clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Press {
    pub wall_ms: u64,
    pub x: u16,
    pub y: u16,
}

/// Only a failure before any page interaction permits the Ctrl+V fallback.
#[derive(Debug)]
pub enum PasteError {
    Untouched(&'static str),
}

pub struct Agent {
    requests: Sender<Request>,
    waker: UnixStream,
    owner: Arc<AtomicU64>,
}

impl Agent {
    fn spawn_with<S: Backend + Send + 'static>(
        events: SyncSender<Ev>,
        connect: impl FnMut(Instant) -> Result<S, &'static str> + Send + 'static,
    ) -> io::Result<Self> {
        let (waker, notified) = UnixStream::pair()?;
        waker.set_nonblocking(true)?;
        notified.set_nonblocking(true)?;
        let (requests, received) = mpsc::channel();
        let owner = Arc::new(AtomicU64::new(0));
        let mut worker = Worker {
            requests: received,
            notified,
            events,
            owner: owner.clone(),
            connect,
            session: None,
            active: 0,
            reconnect_at: Instant::now(),
            backoff: FIRST_RECONNECT,
            fields_dirty: true,
            fields_at: Instant::now(),
            last_fields: None,
            last_sent: None,
        };
        std::thread::Builder::new()
            .name("slimviewer-cdp".into())
            .spawn(move || {
                worker.run();
                worker.close();
            })?;
        Ok(Self {
            requests,
            waker,
            owner,
        })
    }

    pub fn spawn(events: SyncSender<Ev>) -> io::Result<Self> {
        Self::spawn_with(events, Session::open)
    }

    pub fn authorize(&self, owner: Option<u64>) {
        self.owner.store(owner.unwrap_or(0), Ordering::Release);
        self.wake();
    }

    pub fn activate(&self, owner: u64) {
        let _ = self.send(Request::Activate(owner));
    }

    pub fn deactivate(&self) {
        let _ = self.send(Request::Deactivate);
    }

    pub fn paste(&self, text: &str, press: Option<Press>) -> Result<PasteOutcome, PasteError> {
        let deadline = Instant::now() + PASTE_BUDGET;
        let (reply, received) = mpsc::sync_channel(1);
        let state = Arc::new(AtomicU8::new(QUEUED));
        self.send(Request::Paste(PasteRequest {
            text: text.to_owned(),
            press,
            reply,
            deadline,
            state: state.clone(),
        }))
        .map_err(|_| PasteError::Untouched("paste actor closed"))?;
        match received.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(result) => result,
            Err(_) => {
                if let Ok(result) = received.try_recv() {
                    return result;
                }
                if state
                    .compare_exchange(QUEUED, CANCELLED, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    Err(PasteError::Untouched("paste expired before start"))
                } else {
                    // The worker may already have written prepare. Never replay or fall back.
                    Ok(PasteOutcome::Unconfirmed)
                }
            }
        }
    }

    fn send(&self, request: Request) -> Result<(), ()> {
        self.requests.send(request).map_err(|_| ())?;
        self.wake();
        Ok(())
    }

    fn wake(&self) {
        let _ = (&self.waker).write(&[1]);
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.send(Request::Stop);
    }
}

enum Request {
    Activate(u64),
    Deactivate,
    Paste(PasteRequest),
    Stop,
}

struct PasteRequest {
    text: String,
    press: Option<Press>,
    reply: SyncSender<Result<PasteOutcome, PasteError>>,
    deadline: Instant,
    state: Arc<AtomicU8>,
}

trait Backend {
    fn raw_fd(&self) -> RawFd;
    fn next_wake(&self) -> Option<Instant> {
        None
    }
    fn pump(&mut self, until: Instant) -> Result<Changes, &'static str>;
    fn fields(&mut self, until: Instant) -> Option<Fields>;
    fn paste(
        &mut self,
        text: &str,
        press: Option<Press>,
        deadline: Instant,
    ) -> Result<PasteOutcome, PasteError>;
    fn healthy(&self) -> bool;
    fn close(self);
}

impl Backend for Session {
    fn raw_fd(&self) -> RawFd {
        self.raw_fd()
    }
    fn next_wake(&self) -> Option<Instant> {
        self.next_wake()
    }
    fn pump(&mut self, until: Instant) -> Result<Changes, &'static str> {
        self.pump(until)
    }
    fn fields(&mut self, until: Instant) -> Option<Fields> {
        self.set_deadline(until);
        self.fields()
    }
    fn paste(
        &mut self,
        text: &str,
        press: Option<Press>,
        deadline: Instant,
    ) -> Result<PasteOutcome, PasteError> {
        self.set_deadline(deadline);
        paste::run(self, text, press)
    }
    fn healthy(&self) -> bool {
        self.healthy()
    }
    fn close(mut self) {
        self.set_deadline(Instant::now() + EVENT_BUDGET);
        self.close();
    }
}

struct Worker<S, C> {
    requests: Receiver<Request>,
    notified: UnixStream,
    events: SyncSender<Ev>,
    owner: Arc<AtomicU64>,
    connect: C,
    session: Option<S>,
    active: u64,
    reconnect_at: Instant,
    backoff: Duration,
    fields_dirty: bool,
    fields_at: Instant,
    last_fields: Option<Fields>,
    last_sent: Option<Instant>,
}

impl<S: Backend, C: FnMut(Instant) -> Result<S, &'static str>> Worker<S, C> {
    fn run(&mut self) {
        loop {
            loop {
                match self.requests.try_recv() {
                    Ok(Request::Activate(owner)) => {
                        if owner != 0
                            && owner != self.active
                            && owner == self.owner.load(Ordering::Acquire)
                        {
                            self.active = owner;
                            self.last_fields = None;
                            self.last_sent = None;
                            self.fields_dirty = true;
                            self.fields_at = Instant::now();
                            self.reconnect_at = Instant::now();
                        }
                    }
                    Ok(Request::Deactivate) => {
                        self.active = 0;
                        self.close();
                    }
                    Ok(Request::Paste(request)) => self.paste(request),
                    Ok(Request::Stop) | Err(TryRecvError::Disconnected) => return,
                    Err(TryRecvError::Empty) => break,
                }
            }
            if self.watching() && self.session.is_none() && Instant::now() >= self.reconnect_at {
                let _ = self.connect(Instant::now() + CONNECT_BUDGET);
            }
            // Replies can leave events in tungstenite's buffer after the socket is drained.
            if self.watching()
                && let Some(session) = &mut self.session
            {
                match session.pump(Instant::now() + EVENT_BUDGET) {
                    Ok(changes) if changes.fields => self.fields_changed(changes.immediate),
                    Ok(_) => {}
                    Err(_) if !session.healthy() => self.disconnected(),
                    Err(_) => {}
                }
            }
            self.push_fields();
            if self.wait().is_err() {
                return;
            }
        }
    }

    fn watching(&self) -> bool {
        self.active != 0 && self.active == self.owner.load(Ordering::Acquire)
    }

    fn connect(&mut self, deadline: Instant) -> Result<(), &'static str> {
        match (self.connect)(deadline) {
            Ok(session) => {
                self.session = Some(session);
                self.backoff = FIRST_RECONNECT;
                self.fields_dirty = true;
                self.fields_at = Instant::now();
                Ok(())
            }
            Err(reason) => {
                self.disconnected();
                Err(reason)
            }
        }
    }

    fn paste(&mut self, request: PasteRequest) {
        if Instant::now() >= request.deadline {
            let _ = request
                .reply
                .try_send(Err(PasteError::Untouched("paste expired before start")));
            return;
        }
        if request
            .state
            .compare_exchange(QUEUED, STARTED, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let result = if self.session.is_none() {
            self.connect(request.deadline)
                .map_err(PasteError::Untouched)
        } else {
            Ok(())
        }
        .and_then(|()| {
            if Instant::now() >= request.deadline {
                return Err(PasteError::Untouched("paste expired before prepare"));
            }
            self.session
                .as_mut()
                .unwrap()
                .paste(&request.text, request.press, request.deadline)
        });
        self.fields_changed(false);
        if self
            .session
            .as_ref()
            .is_some_and(|session| !session.healthy())
        {
            self.disconnected();
        }
        let _ = request.reply.try_send(result);
    }

    fn disconnected(&mut self) {
        self.session = None;
        self.last_fields = None;
        self.last_sent = None;
        self.reconnect_at = Instant::now() + self.backoff;
        self.backoff = (self.backoff * 2).min(MAX_RECONNECT);
    }

    fn fields_changed(&mut self, immediate: bool) {
        self.fields_dirty = true;
        self.fields_at = self.fields_at.min(
            Instant::now()
                + if immediate {
                    Duration::ZERO
                } else {
                    FIELD_RETRY
                },
        );
    }

    fn close(&mut self) {
        if let Some(session) = self.session.take() {
            session.close();
        }
        self.last_fields = None;
        self.last_sent = None;
        self.backoff = FIRST_RECONNECT;
    }

    fn push_fields(&mut self) {
        let now = Instant::now();
        let resend = self
            .last_sent
            .is_none_or(|at| now.duration_since(at) >= FIELD_RESEND);
        if !self.watching() || now < self.fields_at || (!self.fields_dirty && !resend) {
            return;
        }
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let fields = session.fields(now + EVENT_BUDGET);
        self.fields_dirty = false;
        self.fields_at = now + FIELD_RESEND;
        if let Some(fields) = fields {
            self.fields_at = now + FIELD_RETRY;
            if self.watching() && (resend || self.last_fields.as_ref() != Some(&fields)) {
                if self
                    .events
                    .try_send(Ev::Fields(self.active, fields.clone()))
                    .is_ok()
                {
                    self.last_fields = Some(fields);
                    self.last_sent = Some(Instant::now());
                } else {
                    self.fields_dirty = true;
                }
            }
        }
        if self
            .session
            .as_ref()
            .is_some_and(|session| !session.healthy())
        {
            self.disconnected();
        }
    }

    fn wait(&mut self) -> io::Result<()> {
        let watching = self.watching();
        let session = self.session.as_ref().filter(|_| watching);
        let timer = if watching {
            if self.session.is_none() {
                Some(self.reconnect_at)
            } else if self.fields_dirty {
                Some(self.fields_at)
            } else {
                Some(
                    self.last_sent
                        .map_or(self.fields_at, |at| at + FIELD_RESEND)
                        .max(self.fields_at),
                )
            }
        } else {
            None
        };
        let timer = timer
            .into_iter()
            .chain(session.and_then(Backend::next_wake))
            .min();
        let timeout = timer.map_or(-1, |at| {
            at.saturating_duration_since(Instant::now())
                .as_millis()
                .saturating_add(1)
                .min(i32::MAX as u128) as i32
        });
        let mut fds = [
            libc::pollfd {
                fd: self.notified.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: session.map_or(-1, Backend::raw_fd),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // Both descriptors remain owned by this thread for the whole poll.
        if unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) } < 0 {
            let error = io::Error::last_os_error();
            return if error.kind() == io::ErrorKind::Interrupted {
                Ok(())
            } else {
                Err(error)
            };
        }
        if fds[0].revents != 0 {
            let mut bytes = [0; 256];
            while self.notified.read(&mut bytes).is_ok_and(|n| n != 0) {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    #[test]
    fn tests_that_fields_reject_invalid_geometry_and_focus() {
        let b = FieldBox {
            x: 10,
            y: 20,
            w: 30,
            h: 40,
        };
        let f = Fields::new(100, 100, Some(0), vec![b]).unwrap();
        assert_eq!(
            (f.width(), f.height(), f.focused(), f.boxes()),
            (100, 100, Some(0), &[b][..])
        );
        assert!(Fields::new(100, 100, None, vec![]).is_some());
        assert!(Fields::new(100, 100, None, vec![b; 128]).is_some());
        assert!(Fields::new(100, 100, None, vec![b; 129]).is_none());
        assert!(Fields::new(100, 100, Some(1), vec![b]).is_none());
        assert!(Fields::new(100, 100, Some(0), vec![]).is_none());
        for bad in [
            FieldBox { w: 0, ..b },
            FieldBox { h: 0, ..b },
            FieldBox { x: 99, ..b },
            FieldBox { y: 99, ..b },
            FieldBox { x: u16::MAX, ..b },
        ] {
            assert!(Fields::new(100, 100, None, vec![bad]).is_none());
        }
        assert!(Fields::new(40, 60, Some(0), vec![b]).is_some());
    }

    #[test]
    fn tests_that_paste_connects_lazily_without_a_field_watch_and_reuses_the_session() {
        let (events, _) = mpsc::sync_channel(4);
        let (agent, operations, _) = fake_agent(events, 0);
        assert!(operations.recv_timeout(Duration::from_millis(30)).is_err());
        assert!(matches!(
            agent.paste("first", None),
            Ok(PasteOutcome::Pasted)
        ));
        assert!(matches!(operations.recv().unwrap(), Operation::Open(_)));
        let Operation::Connected(_wake) = operations.recv().unwrap() else {
            panic!("not connected")
        };
        assert!(matches!(operations.recv().unwrap(), Operation::Paste));
        assert!(matches!(
            agent.paste("second", None),
            Ok(PasteOutcome::Pasted)
        ));
        assert!(matches!(operations.recv().unwrap(), Operation::Paste));
        agent.deactivate();
        assert!(matches!(
            operations.recv_timeout(Duration::from_secs(1)).unwrap(),
            Operation::Closed
        ));
    }

    enum Operation {
        Open(Instant),
        Connected(UnixStream),
        Paste,
        Closed,
        TapObserved,
        PumpError(Instant),
    }

    type SharedFields = Arc<Mutex<Option<Fields>>>;

    #[derive(Default)]
    struct FakeBehavior {
        disconnected: bool,
        require_tap: bool,
        tapped: bool,
        transient_error: bool,
        retry_at: Option<Instant>,
        buffer_on_fields: Option<Fields>,
        arm_fields_buffer: bool,
        buffered: Option<Fields>,
        poll_checks: Option<Arc<std::sync::atomic::AtomicUsize>>,
    }

    struct FakeBackend {
        read: UnixStream,
        fields: SharedFields,
        fields_reads: Option<mpsc::Sender<()>>,
        operations: mpsc::Sender<Operation>,
        gate: Option<mpsc::Receiver<()>>,
        behavior: FakeBehavior,
    }

    impl Backend for FakeBackend {
        fn raw_fd(&self) -> std::os::fd::RawFd {
            if let Some(checks) = &self.behavior.poll_checks {
                checks.fetch_add(1, Ordering::Relaxed);
            }
            self.read.as_raw_fd()
        }

        fn next_wake(&self) -> Option<Instant> {
            if let Some(checks) = &self.behavior.poll_checks {
                checks.fetch_add(1, Ordering::Relaxed);
            }
            if self.behavior.buffered.is_some() {
                Some(Instant::now())
            } else {
                self.behavior.retry_at
            }
        }

        fn pump(&mut self, _until: Instant) -> Result<Changes, &'static str> {
            let mut bytes = [0; 64];
            let mut immediate = false;
            let changed = match self.read.read(&mut bytes) {
                Ok(0) => {
                    self.behavior.disconnected = true;
                    return Err("peer closed");
                }
                Ok(n) => {
                    for byte in &bytes[..n] {
                        match byte {
                            b't' => {
                                self.behavior.tapped = true;
                                let _ = self.operations.send(Operation::TapObserved);
                            }
                            b'e' => self.behavior.transient_error = true,
                            b'b' => self.behavior.arm_fields_buffer = true,
                            b'n' => immediate = true,
                            _ => {}
                        }
                    }
                    true
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => false,
                Err(_) => {
                    self.behavior.disconnected = true;
                    return Err("peer failed");
                }
            };
            if self.behavior.transient_error {
                let now = Instant::now();
                self.behavior.retry_at = Some(now + Duration::from_millis(100));
                let _ = self.operations.send(Operation::PumpError(now));
                return Err("selection temporarily unavailable");
            }
            if let Some(fields) = self.behavior.buffered.take() {
                *self.fields.lock().unwrap() = Some(fields);
                return Ok(Changes {
                    fields: true,
                    immediate,
                });
            }
            Ok(Changes {
                fields: changed,
                immediate,
            })
        }

        fn fields(&mut self, _until: Instant) -> Option<Fields> {
            let fields = self.fields.lock().unwrap().clone();
            if let Some(reads) = &self.fields_reads {
                let _ = reads.send(());
            }
            if self.behavior.arm_fields_buffer {
                self.behavior.buffered = self.behavior.buffer_on_fields.take();
                self.behavior.arm_fields_buffer = false;
            }
            fields
        }

        fn paste(
            &mut self,
            _text: &str,
            _press: Option<Press>,
            _deadline: Instant,
        ) -> Result<PasteOutcome, PasteError> {
            self.operations.send(Operation::Paste).unwrap();
            if let Some(gate) = self.gate.take() {
                gate.recv_timeout(Duration::from_secs(3)).unwrap();
            }
            Ok(if self.behavior.require_tap && !self.behavior.tapped {
                PasteOutcome::NoField
            } else {
                PasteOutcome::Pasted
            })
        }

        fn healthy(&self) -> bool {
            !self.behavior.disconnected
        }

        fn close(self) {
            let _ = self.operations.send(Operation::Closed);
        }
    }

    fn fake_agent(
        events: SyncSender<Ev>,
        failures: usize,
    ) -> (Agent, mpsc::Receiver<Operation>, SharedFields) {
        configured_agent(events, failures, FakeBehavior::default)
    }

    fn configured_agent(
        events: SyncSender<Ev>,
        mut failures: usize,
        behavior: impl Fn() -> FakeBehavior + Send + 'static,
    ) -> (Agent, mpsc::Receiver<Operation>, SharedFields) {
        let (operations, received) = mpsc::channel();
        let fields = Arc::new(Mutex::new(Fields::new(100, 100, None, vec![])));
        let values = fields.clone();
        let agent = Agent::spawn_with(events, move |_| {
            assert_eq!(std::thread::current().name(), Some("slimviewer-cdp"));
            operations.send(Operation::Open(Instant::now())).unwrap();
            if failures > 0 {
                failures -= 1;
                return Err("not available");
            }
            let (read, write) = UnixStream::pair().unwrap();
            read.set_nonblocking(true).unwrap();
            operations.send(Operation::Connected(write)).unwrap();
            Ok(FakeBackend {
                read,
                fields: values.clone(),
                fields_reads: None,
                operations: operations.clone(),
                gate: None,
                behavior: behavior(),
            })
        })
        .unwrap();
        (agent, received, fields)
    }

    fn connected(operations: &mpsc::Receiver<Operation>) -> UnixStream {
        assert!(matches!(
            operations.recv_timeout(Duration::from_secs(2)).unwrap(),
            Operation::Open(_)
        ));
        let Operation::Connected(wake) = operations.recv_timeout(Duration::from_secs(2)).unwrap()
        else {
            panic!("not connected")
        };
        wake
    }

    #[test]
    fn tests_that_fields_stop_for_revoked_owners_and_resume_only_after_activation() {
        let (events, received) = mpsc::sync_channel(8);
        let (agent, operations, fields) = fake_agent(events, 0);
        agent.authorize(Some(7));
        agent.activate(7);
        let mut wake = connected(&operations);
        assert!(matches!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            Ev::Fields(7, _)
        ));
        agent.authorize(Some(8));
        *fields.lock().unwrap() = Fields::new(200, 200, None, vec![]);
        wake.write_all(&[1]).unwrap();
        agent.activate(7);
        assert!(received.recv_timeout(Duration::from_millis(100)).is_err());
        agent.activate(8);
        assert!(
            matches!(received.recv_timeout(Duration::from_secs(1)).unwrap(), Ev::Fields(8, f) if f.width() == 200)
        );
        drop(agent);
        assert!(matches!(
            operations.recv_timeout(Duration::from_secs(1)).unwrap(),
            Operation::Closed
        ));
    }

    #[test]
    fn tests_that_a_full_engine_queue_does_not_block_paste_or_lose_the_next_fields() {
        let (events, received) = mpsc::sync_channel(1);
        events.send(Ev::Damage).unwrap();
        let (agent, operations, _) = fake_agent(events, 0);
        agent.authorize(Some(7));
        agent.activate(7);
        let _wake = connected(&operations);
        assert!(matches!(
            agent.paste("first", None),
            Ok(PasteOutcome::Pasted)
        ));
        assert!(matches!(received.recv().unwrap(), Ev::Damage));
        assert!(matches!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            Ev::Fields(7, _)
        ));
    }

    #[test]
    fn tests_that_unchanged_fields_are_resent_after_two_seconds() {
        let (events, received) = mpsc::sync_channel(4);
        let (agent, operations, _) = fake_agent(events, 0);
        agent.authorize(Some(7));
        agent.activate(7);
        let _wake = connected(&operations);
        let Ev::Fields(7, first) = received.recv_timeout(Duration::from_secs(1)).unwrap() else {
            panic!("no fields")
        };
        let started = Instant::now();
        assert!(received.recv_timeout(Duration::from_millis(100)).is_err());
        let Ev::Fields(7, second) = received.recv_timeout(Duration::from_secs(3)).unwrap() else {
            panic!("no resend")
        };
        assert_eq!(first, second);
        assert!(started.elapsed() >= Duration::from_millis(1900));
    }

    #[test]
    fn tests_that_failed_connections_back_off_and_a_dropped_connection_reopens() {
        let (events, _) = mpsc::sync_channel(4);
        let (agent, operations, _) = fake_agent(events, 2);
        agent.authorize(Some(7));
        agent.activate(7);
        let mut attempts = Vec::new();
        for _ in 0..3 {
            let Operation::Open(at) = operations.recv_timeout(Duration::from_secs(2)).unwrap()
            else {
                panic!("expected connection attempt")
            };
            attempts.push(at);
            // Repeated watch requests must not reset the reconnect timer.
            agent.activate(7);
        }
        assert!(attempts[1].duration_since(attempts[0]) >= Duration::from_millis(240));
        assert!(attempts[2].duration_since(attempts[1]) >= Duration::from_millis(490));
        let Operation::Connected(wake) = operations.recv_timeout(Duration::from_secs(1)).unwrap()
        else {
            panic!("not connected")
        };
        drop(wake);
        let wake = connected(&operations);
        assert!(matches!(
            agent.paste("after reconnect", None),
            Ok(PasteOutcome::Pasted)
        ));
        drop(wake);
    }

    #[test]
    fn tests_that_a_queued_paste_is_cancelled_before_it_can_touch_the_page() {
        let (events, _) = mpsc::sync_channel(4);
        let (opened, opening) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let (operations, received) = mpsc::channel();
        let agent = Agent::spawn_with(events, move |_| {
            opened.send(()).unwrap();
            gate.recv_timeout(Duration::from_secs(3)).unwrap();
            let (read, write) = UnixStream::pair().unwrap();
            read.set_nonblocking(true).unwrap();
            operations.send(Operation::Connected(write)).unwrap();
            Ok(FakeBackend {
                read,
                fields: Arc::new(Mutex::new(Fields::new(100, 100, None, vec![]))),
                fields_reads: None,
                operations: operations.clone(),
                gate: None,
                behavior: FakeBehavior::default(),
            })
        })
        .unwrap();
        agent.authorize(Some(7));
        agent.activate(7);
        opening.recv_timeout(Duration::from_secs(1)).unwrap();
        let start = Instant::now();
        assert!(matches!(
            agent.paste("expired", None),
            Err(PasteError::Untouched(_))
        ));
        assert!(start.elapsed() < Duration::from_millis(1400));
        release.send(()).unwrap();
        let Operation::Connected(_wake) = received.recv_timeout(Duration::from_secs(1)).unwrap()
        else {
            panic!("not connected")
        };
        assert!(received.recv_timeout(Duration::from_millis(100)).is_err());
        agent.deactivate();
        assert!(matches!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            Operation::Closed
        ));
    }

    #[test]
    fn tests_that_an_already_started_paste_times_out_without_allowing_fallback() {
        let (events, _) = mpsc::sync_channel(4);
        let (operations, received) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let mut gate = Some(gate);
        let agent = Arc::new(
            Agent::spawn_with(events, move |_| {
                let (read, write) = UnixStream::pair().unwrap();
                read.set_nonblocking(true).unwrap();
                operations.send(Operation::Connected(write)).unwrap();
                Ok(FakeBackend {
                    read,
                    fields: Arc::new(Mutex::new(Fields::new(100, 100, None, vec![]))),
                    fields_reads: None,
                    operations: operations.clone(),
                    gate: gate.take(),
                    behavior: FakeBehavior::default(),
                })
            })
            .unwrap(),
        );
        let worker = agent.clone();
        let start = Instant::now();
        let request = std::thread::spawn(move || worker.paste("late", None));
        let Operation::Connected(_wake) = received.recv_timeout(Duration::from_secs(1)).unwrap()
        else {
            panic!("not connected")
        };
        assert!(matches!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            Operation::Paste
        ));
        assert!(matches!(
            request.join().unwrap(),
            Ok(PasteOutcome::Unconfirmed)
        ));
        assert!(start.elapsed() < Duration::from_millis(1400));
        release.send(()).unwrap();
        agent.deactivate();
        assert!(matches!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            Operation::Closed
        ));
    }

    #[test]
    fn tests_that_a_connection_finishing_after_the_paste_deadline_does_not_insert() {
        let (events, _) = mpsc::sync_channel(4);
        let (opened, opening) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let (operations, received) = mpsc::channel();
        let agent = Arc::new(
            Agent::spawn_with(events, move |_| {
                opened.send(()).unwrap();
                gate.recv_timeout(Duration::from_secs(3)).unwrap();
                let (read, write) = UnixStream::pair().unwrap();
                read.set_nonblocking(true).unwrap();
                operations.send(Operation::Connected(write)).unwrap();
                Ok(FakeBackend {
                    read,
                    fields: Arc::new(Mutex::new(Fields::new(100, 100, None, vec![]))),
                    fields_reads: None,
                    operations: operations.clone(),
                    gate: None,
                    behavior: FakeBehavior::default(),
                })
            })
            .unwrap(),
        );
        let worker = agent.clone();
        let request = std::thread::spawn(move || worker.paste("late connection", None));
        opening.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(matches!(
            request.join().unwrap(),
            Ok(PasteOutcome::Unconfirmed)
        ));
        release.send(()).unwrap();
        let Operation::Connected(_wake) = received.recv_timeout(Duration::from_secs(1)).unwrap()
        else {
            panic!("not connected")
        };
        agent.deactivate();
        assert!(matches!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            Operation::Closed
        ));
    }

    #[test]
    fn tests_that_new_fields_do_not_wait_for_the_resend_timer_after_initial_unavailability() {
        let (events, received) = mpsc::sync_channel(4);
        let (operations, opened) = mpsc::channel();
        let (reads, read) = mpsc::channel();
        let fields = Arc::new(Mutex::new(None));
        let values = fields.clone();
        let agent = Agent::spawn_with(events, move |_| {
            let (read, write) = UnixStream::pair().unwrap();
            read.set_nonblocking(true).unwrap();
            operations.send(Operation::Connected(write)).unwrap();
            Ok(FakeBackend {
                read,
                fields: values.clone(),
                fields_reads: Some(reads.clone()),
                operations: operations.clone(),
                gate: None,
                behavior: FakeBehavior::default(),
            })
        })
        .unwrap();
        agent.authorize(Some(7));
        agent.activate(7);
        let Operation::Connected(mut wake) = opened.recv_timeout(Duration::from_secs(1)).unwrap()
        else {
            panic!("not connected")
        };
        read.recv_timeout(Duration::from_secs(1)).unwrap();
        *fields.lock().unwrap() = Fields::new(100, 100, None, vec![]);
        wake.write_all(&[1]).unwrap();
        assert!(matches!(
            received.recv_timeout(Duration::from_millis(400)).unwrap(),
            Ev::Fields(7, _)
        ));
    }

    #[test]
    fn tests_that_a_new_documents_first_fields_bypass_the_regular_update_throttle() {
        let (events, received) = mpsc::sync_channel(4);
        let (agent, operations, fields) = fake_agent(events, 0);
        agent.authorize(Some(7));
        agent.activate(7);
        let mut wake = connected(&operations);
        assert!(matches!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            Ev::Fields(7, _)
        ));
        *fields.lock().unwrap() = Fields::new(200, 200, None, vec![]);
        wake.write_all(b"n").unwrap();
        assert!(
            matches!(received.recv_timeout(Duration::from_millis(35)).unwrap(), Ev::Fields(7, f) if f.width() == 200)
        );
    }

    #[test]
    fn tests_that_transient_pump_errors_preserve_the_connection_and_remembered_tap() {
        let (events, received) = mpsc::sync_channel(4);
        let (agent, operations, _) = configured_agent(events, 0, || FakeBehavior {
            require_tap: true,
            ..FakeBehavior::default()
        });
        agent.authorize(Some(7));
        agent.activate(7);
        let mut wake = connected(&operations);
        assert!(matches!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            Ev::Fields(7, _)
        ));
        wake.write_all(b"t").unwrap();
        assert!(matches!(
            operations.recv_timeout(Duration::from_secs(1)).unwrap(),
            Operation::TapObserved
        ));
        wake.write_all(b"e").unwrap();
        assert!(matches!(
            operations.recv_timeout(Duration::from_secs(1)).unwrap(),
            Operation::PumpError(_)
        ));
        assert!(matches!(
            agent.paste("after transient error", None),
            Ok(PasteOutcome::Pasted)
        ));
    }

    #[test]
    fn tests_that_fields_buffered_during_a_field_read_are_delivered_without_socket_activity() {
        let (events, received) = mpsc::sync_channel(4);
        let (agent, operations, _) = configured_agent(events, 0, || FakeBehavior {
            buffer_on_fields: Fields::new(200, 200, None, vec![]),
            ..FakeBehavior::default()
        });
        agent.authorize(Some(7));
        agent.activate(7);
        let mut wake = connected(&operations);
        assert!(
            matches!(received.recv_timeout(Duration::from_secs(1)).unwrap(), Ev::Fields(7, f) if f.width() == 100)
        );
        wake.write_all(b"b").unwrap();
        assert!(
            matches!(received.recv_timeout(Duration::from_millis(400)).unwrap(), Ev::Fields(7, f) if f.width() == 200)
        );
    }

    #[test]
    fn tests_that_transient_pump_retries_follow_the_backend_timer_without_spinning() {
        let (events, received) = mpsc::sync_channel(4);
        let (agent, operations, _) = configured_agent(events, 0, FakeBehavior::default);
        agent.authorize(Some(7));
        agent.activate(7);
        let mut wake = connected(&operations);
        assert!(matches!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            Ev::Fields(7, _)
        ));
        wake.write_all(b"e").unwrap();
        let Operation::PumpError(mut previous) =
            operations.recv_timeout(Duration::from_secs(1)).unwrap()
        else {
            panic!("expected transient error")
        };
        for _ in 0..3 {
            let Operation::PumpError(at) =
                operations.recv_timeout(Duration::from_millis(400)).unwrap()
            else {
                panic!("connection was replaced")
            };
            assert!(at.duration_since(previous) >= Duration::from_millis(90));
            previous = at;
        }
    }
    #[test]
    fn tests_that_background_retries_and_socket_polling_stop_without_an_authorized_watch() {
        let polls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let checks = polls.clone();
        let (events, _received) = mpsc::sync_channel(8);
        let (agent, operations, _) = configured_agent(events, 0, move || FakeBehavior {
            transient_error: true,
            poll_checks: Some(checks.clone()),
            ..FakeBehavior::default()
        });
        assert!(matches!(
            agent.paste("explicit", None),
            Ok(PasteOutcome::Pasted)
        ));
        let mut wake = connected(&operations);
        assert!(matches!(operations.recv().unwrap(), Operation::Paste));
        wake.write_all(b"e").unwrap();
        assert!(operations.recv_timeout(Duration::from_millis(250)).is_err());
        assert_eq!(polls.load(Ordering::Relaxed), 0);

        agent.authorize(Some(7));
        agent.activate(7);
        assert!(matches!(
            operations.recv_timeout(Duration::from_secs(1)).unwrap(),
            Operation::PumpError(_)
        ));
        agent.authorize(None);
        assert!(matches!(
            agent.paste("explicit after revocation", None),
            Ok(PasteOutcome::Pasted)
        ));
        loop {
            match operations.recv_timeout(Duration::from_secs(1)).unwrap() {
                Operation::Paste => break,
                Operation::PumpError(_) => {}
                _ => panic!("session was replaced"),
            }
        }
        let before = polls.load(Ordering::Relaxed);
        wake.write_all(b"e").unwrap();
        agent.authorize(Some(8));
        assert!(operations.recv_timeout(Duration::from_millis(250)).is_err());
        assert_eq!(polls.load(Ordering::Relaxed), before);
        agent.activate(8);
        assert!(matches!(
            operations.recv_timeout(Duration::from_secs(1)).unwrap(),
            Operation::PumpError(_)
        ));
    }
}
