//! One ordered input queue. Network waits never run on the capture thread.

use std::collections::VecDeque;
use std::io;
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use super::{Agent, PasteError, PasteOutcome, Rfb, lock_error, validate_text};
use crate::engine::{Connection, Ev};

const MAX_PENDING: usize = 64;
const MAX_DRAINING: usize = 8;
const PRESS_WINDOW: Duration = Duration::from_secs(1);

enum Command {
    Pointer(u16, u16, u8),
    Wheel(u16, u16, f32, f32),
    Key(u32, bool),
    Paste {
        owner: u64,
        id: Option<u32>,
        text: String,
    },
    Type(String),
    ClipboardWatch(u64),
    FieldsWatch(u64),
    Release,
}

struct Pending {
    command: Command,
    close_after: Vec<Arc<Connection>>,
}

#[derive(Default)]
struct State {
    pending: VecDeque<Pending>,
    draining: usize,
    closed: bool,
}

#[derive(Default)]
struct Queue {
    state: Mutex<State>,
    ready: Condvar,
}

pub struct Input {
    queue: Arc<Queue>,
    clipboard: Option<Arc<crate::clipboard::Clipboard>>,
    agent: Option<Arc<Agent>>,
}

impl Input {
    #[cfg(test)]
    pub(crate) fn failing_for_test(events: SyncSender<Ev>) -> Self {
        Self::spawn(events, |_| Err(io::Error::other("remote input failed"))).unwrap()
    }

    pub fn connect(addr: &str, display: &str, events: SyncSender<Ev>) -> Result<Self, String> {
        let mut rfb = Rfb::connect(addr, events.clone())?;
        let clipboard = Arc::new(crate::clipboard::Clipboard::open(display, events.clone())?);
        let observer = clipboard.clone();
        let agent =
            Arc::new(Agent::spawn(events.clone()).map_err(|e| format!("paste agent: {e}"))?);
        let worker_agent = agent.clone();
        let mut input = Self::spawn(events.clone(), move |command| match command {
            Command::Pointer(x, y, buttons) => rfb.pointer(x, y, buttons),
            Command::Wheel(x, y, dx, dy) => rfb.wheel(x, y, dx, dy),
            Command::Key(sym, down) => rfb.key(sym, down),
            Command::Paste { owner, id, text } => {
                apply_paste(&mut rfb, &worker_agent, &events, owner, id, &text)
            }
            Command::Type(text) => rfb.type_text(&text),
            Command::ClipboardWatch(owner) => {
                observer.activate(owner);
                Ok(())
            }
            Command::FieldsWatch(owner) => {
                worker_agent.activate(owner);
                Ok(())
            }
            Command::Release => {
                observer.deactivate();
                worker_agent.deactivate();
                rfb.release_all()?;
                rfb.drain()
            }
        })
        .map_err(|e| format!("input worker: {e}"))?;
        input.clipboard = Some(clipboard);
        input.agent = Some(agent);
        Ok(input)
    }

    fn spawn(
        events: SyncSender<Ev>,
        mut apply: impl FnMut(Command) -> io::Result<()> + Send + 'static,
    ) -> io::Result<Self> {
        let queue = Arc::new(Queue::default());
        let worker = queue.clone();
        std::thread::Builder::new()
            .name("slimviewer-input".into())
            .spawn(move || {
                let result = (|| {
                    loop {
                        let pending = {
                            let state = worker.state.lock().map_err(lock_error)?;
                            let mut state = worker
                                .ready
                                .wait_while(state, |s| s.pending.is_empty() && !s.closed)
                                .map_err(lock_error)?;
                            match state.pending.pop_front() {
                                Some(command) => command,
                                None => return Ok::<_, io::Error>(()),
                            }
                        };
                        apply(pending.command)?;
                        let drained = pending.close_after.len();
                        for connection in &pending.close_after {
                            connection.input_drained();
                        }
                        drop(pending.close_after);
                        worker.state.lock().map_err(lock_error)?.draining -= drained;
                    }
                })();
                if let Err(error) = result {
                    if let Ok(mut state) = worker.state.lock() {
                        state.closed = true;
                        state.pending.clear();
                    }
                    let _ = events.send(Ev::Fatal(format!("RFB input failed: {error}")));
                }
            })?;
        Ok(Self {
            queue,
            clipboard: None,
            agent: None,
        })
    }

    fn send(&self, command: Command) -> io::Result<()> {
        self.send_pending(Pending {
            command,
            close_after: Vec::new(),
        })
    }

    fn send_pending(&self, mut pending: Pending) -> io::Result<()> {
        let mut state = self.queue.state.lock().map_err(lock_error)?;
        if state.closed {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "input worker closed",
            ));
        }
        // Preserve gesture order while combining adjacent trackpad ticks during slow CDP I/O.
        if let Command::Wheel(x, y, dx, dy) = &pending.command
            && let Some(Command::Wheel(px, py, pending_x, pending_y)) =
                state.pending.back_mut().map(|last| &mut last.command)
            && (x, y) == (px, py)
            && (*pending_x + dx).abs() <= 16384.0
            && (*pending_y + dy).abs() <= 16384.0
        {
            *pending_x += dx;
            *pending_y += dy;
            return Ok(());
        }
        if matches!(pending.command, Command::Release) {
            // Reserve one slot for handoff, even when a slow CDP request fills the queue.
            state.draining += pending.close_after.len();
            if let Some(last) = state.pending.back_mut()
                && matches!(last.command, Command::Release)
            {
                last.close_after.append(&mut pending.close_after);
                return Ok(());
            }
        } else if state.pending.len() >= MAX_PENDING {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "input queue full",
            ));
        }
        state.pending.push_back(pending);
        self.queue.ready.notify_one();
        Ok(())
    }

    pub fn pointer(&mut self, x: u16, y: u16, buttons: u8) -> io::Result<()> {
        self.send(Command::Pointer(x, y, buttons))
    }

    pub fn key(&mut self, sym: u32, down: bool) -> io::Result<()> {
        self.send(Command::Key(sym, down))
    }

    pub fn wheel(&mut self, x: u16, y: u16, dx: f32, dy: f32) -> io::Result<()> {
        self.send(Command::Wheel(x, y, dx, dy))
    }

    pub fn watch_clipboard(&self, owner: u64) -> io::Result<()> {
        if let Some(clipboard) = &self.clipboard {
            clipboard.authorize(Some(owner));
        }
        self.send(Command::ClipboardWatch(owner))
    }

    pub fn watch_fields(&self, owner: u64) -> io::Result<()> {
        if let Some(agent) = &self.agent {
            agent.authorize(Some(owner));
        }
        self.send(Command::FieldsWatch(owner))
    }

    pub fn paste(&mut self, owner: u64, id: Option<u32>, text: &str) -> io::Result<()> {
        validate_text(text)?;
        self.send(Command::Paste {
            owner,
            id,
            text: text.to_owned(),
        })
    }

    pub fn type_text(&mut self, text: &str) -> io::Result<()> {
        validate_text(text)?;
        self.send(Command::Type(text.to_owned()))
    }

    pub fn release_all(&mut self) -> io::Result<()> {
        if let Some(agent) = &self.agent {
            agent.authorize(None);
        }
        if let Some(clipboard) = &self.clipboard {
            clipboard.authorize(None);
        }
        self.send(Command::Release)
    }

    pub fn admits_owner(&self) -> bool {
        self.queue
            .state
            .lock()
            .is_ok_and(|state| !state.closed && state.draining < MAX_DRAINING)
    }

    pub fn release_owner(&mut self, connection: Arc<Connection>) -> io::Result<()> {
        if let Some(agent) = &self.agent {
            agent.authorize(None);
        }
        if let Some(clipboard) = &self.clipboard {
            clipboard.authorize(None);
        }
        // The last reference closes TCP. Keep it until all earlier input and this release
        // have been applied, including when video teardown has already dropped the viewer.
        connection.wait_for_input();
        self.send_pending(Pending {
            command: Command::Release,
            close_after: vec![connection],
        })
    }
}

fn apply_paste(
    rfb: &mut Rfb,
    agent: &Agent,
    events: &SyncSender<Ev>,
    owner: u64,
    id: Option<u32>,
    text: &str,
) -> io::Result<()> {
    let press = rfb.last_press;
    if rfb.recent_press(PRESS_WINDOW).is_some() {
        rfb.drain_soft()?;
    }
    let outcome = match agent.paste(text, press) {
        Ok(outcome) => outcome,
        Err(PasteError::Untouched(reason)) => {
            eprintln!("slimviewer: event=paste path=ctrl-v reason={reason}");
            rfb.paste(text)?;
            PasteOutcome::Unconfirmed
        }
    };
    if let Some(id) = id {
        // Only this worker waits; a busy engine must not silently lose the transaction reply.
        let _ = events.send(Ev::PasteResult(owner, id, outcome));
    }
    Ok(())
}

impl Drop for Input {
    fn drop(&mut self) {
        let _ = self.release_all();
        if let Ok(mut state) = self.queue.state.lock() {
            state.closed = true;
            self.queue.ready.notify_one();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn tests_that_fallback_sends_one_unconfirmed_result_after_ctrl_v() {
        let (mut rfb, mut peer, _) = super::super::tests::input_pair();
        let (events, received) = mpsc::sync_channel(4);
        let agent = Agent::spawn(events.clone()).unwrap();
        let mut input = Input::spawn(events.clone(), move |command| match command {
            Command::Paste { owner, id, text } => {
                apply_paste(&mut rfb, &agent, &events, owner, id, &text)
            }
            Command::Key(sym, down) => rfb.key(sym, down),
            Command::Release => Ok(()),
            _ => unreachable!(),
        })
        .unwrap();
        input.paste(7, Some(0x01020304), "first").unwrap();
        assert!(matches!(
            received.recv_timeout(Duration::from_secs(3)).unwrap(),
            Ev::PasteResult(7, 0x01020304, PasteOutcome::Unconfirmed)
        ));
        let mut first = [0; 44];
        peer.read_exact(&mut first).unwrap();
        assert_eq!(
            &first[..12],
            &super::super::clipboard_record(super::super::NOTIFY | super::super::UTF8, &[])
        );
        assert_eq!(
            &first[12..],
            &[
                super::super::key_record(super::super::XK_CONTROL_L, true),
                super::super::key_record(0x76, true),
                super::super::key_record(0x76, false),
                super::super::key_record(super::super::XK_CONTROL_L, false)
            ]
            .concat()
        );
        input.paste(7, None, "legacy").unwrap();
        input.key(97, true).unwrap();
        let mut rest = [0; 52];
        peer.read_exact(&mut rest).unwrap();
        assert_eq!(&rest[..44], first);
        assert_eq!(&rest[44..], super::super::key_record(97, true));
        assert!(received.try_recv().is_err());
    }

    #[test]
    fn tests_that_field_watch_keeps_its_place_in_the_input_queue() {
        let (events, _) = mpsc::sync_channel(4);
        let (applied, log) = mpsc::channel();
        let mut input = Input::spawn(events, move |command| {
            let entry = match command {
                Command::FieldsWatch(owner) => owner,
                Command::Release => 0,
                Command::Key(_, _) => 99,
                _ => unreachable!(),
            };
            let _ = applied.send(entry);
            Ok(())
        })
        .unwrap();
        input.watch_fields(1).unwrap();
        input.release_all().unwrap();
        input.watch_fields(2).unwrap();
        input.key(97, true).unwrap();
        for expected in [1, 0, 2, 99] {
            assert_eq!(log.recv_timeout(Duration::from_secs(3)).unwrap(), expected);
        }
    }

    #[test]
    fn tests_that_a_full_event_queue_does_not_drop_the_paste_result() {
        let (mut rfb, mut peer, _) = super::super::tests::input_pair();
        let (events, received) = mpsc::sync_channel(1);
        events.send(Ev::Damage).unwrap();
        let agent = Agent::spawn(events.clone()).unwrap();
        let mut input = Input::spawn(events.clone(), move |command| match command {
            Command::Paste { owner, id, text } => {
                apply_paste(&mut rfb, &agent, &events, owner, id, &text)
            }
            Command::Key(sym, down) => rfb.key(sym, down),
            Command::Release => Ok(()),
            _ => unreachable!(),
        })
        .unwrap();
        input.paste(7, Some(9), "test").unwrap();
        input.key(97, true).unwrap();
        peer.read_exact(&mut [0; 44]).unwrap();
        peer.set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        assert!(
            matches!(peer.read(&mut [0]), Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut))
        );
        assert!(matches!(received.recv().unwrap(), Ev::Damage));
        assert!(matches!(
            received.recv_timeout(Duration::from_secs(3)).unwrap(),
            Ev::PasteResult(7, 9, PasteOutcome::Unconfirmed)
        ));
        peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut key = [0; 8];
        peer.read_exact(&mut key).unwrap();
        assert_eq!(key, super::super::key_record(97, true));
        assert!(received.try_recv().is_err());
    }

    #[test]
    fn a_new_clipboard_watch_waits_for_old_input_and_its_release() {
        let (events, _) = mpsc::sync_channel(4);
        let (started, active) = mpsc::channel();
        let (resume, wait) = mpsc::channel();
        let (finished, log) = mpsc::channel();
        let mut input = Input::spawn(events, move |command| {
            let entry = match command {
                Command::ClipboardWatch(owner) => owner * 10,
                Command::Paste { .. } => {
                    started.send(()).unwrap();
                    wait.recv_timeout(Duration::from_secs(3)).unwrap();
                    1
                }
                Command::Release => 2,
                Command::Key(_, _) => 3,
                _ => unreachable!(),
            };
            let _ = finished.send(entry);
            Ok(())
        })
        .unwrap();
        input.watch_clipboard(1).unwrap();
        input.paste(1, None, "old owner clipboard").unwrap();
        active.recv_timeout(Duration::from_secs(3)).unwrap();
        input.release_all().unwrap();
        input.watch_clipboard(2).unwrap();
        input.key(97, true).unwrap();
        assert_eq!(log.try_iter().collect::<Vec<_>>(), [10]);
        resume.send(()).unwrap();
        for expected in [1, 2, 20, 3] {
            assert_eq!(log.recv_timeout(Duration::from_secs(3)).unwrap(), expected);
        }
    }

    fn connection_pair() -> (Arc<Connection>, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        peer.set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        (Arc::new(Connection::new(stream)), peer)
    }

    fn assert_open(peer: &mut TcpStream) {
        let result = peer.read(&mut [0]);
        assert!(
            matches!(result, Err(ref e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut)),
            "owner closed before input and release finished: {result:?}"
        );
    }

    #[test]
    fn tests_that_half_closed_owner_waits_for_all_input_and_the_release() {
        let (events, _) = mpsc::sync_channel(4);
        let (resume, wait) = mpsc::channel();
        let (applied, log) = mpsc::channel();
        let mut input = Input::spawn(events, move |command| {
            match command {
                Command::Type(text) => {
                    assert_eq!(text, "héllo");
                    applied.send("typing").unwrap();
                    wait.recv_timeout(Duration::from_secs(3)).unwrap();
                    applied.send("typed").unwrap();
                }
                Command::Key(_, _) => applied.send("key").unwrap(),
                Command::Release => {
                    let _ = applied.send("releasing");
                    let _ = wait.recv_timeout(Duration::from_secs(3));
                }
                _ => unreachable!(),
            }
            Ok(())
        })
        .unwrap();
        let (connection, mut peer) = connection_pair();
        input.type_text("héllo").unwrap();
        assert_eq!(log.recv_timeout(Duration::from_secs(3)).unwrap(), "typing");
        input.key(97, true).unwrap();
        peer.shutdown(Shutdown::Write).unwrap();
        input.release_owner(connection).unwrap();
        assert_open(&mut peer);
        resume.send(()).unwrap();
        for expected in ["typed", "key", "releasing"] {
            assert_eq!(log.recv_timeout(Duration::from_secs(3)).unwrap(), expected);
        }
        assert_open(&mut peer);
        resume.send(()).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        assert_eq!(peer.read(&mut [0]).unwrap(), 0);
    }

    #[test]
    fn tests_that_replaced_owners_share_a_bounded_release_without_closing_early() {
        let (events, _) = mpsc::sync_channel(4);
        let (resume, wait) = mpsc::channel();
        let (started, active) = mpsc::channel();
        let mut input = Input::spawn(events, move |command| {
            if matches!(command, Command::Type(_)) {
                started.send(()).unwrap();
                wait.recv_timeout(Duration::from_secs(3)).unwrap();
            }
            Ok(())
        })
        .unwrap();
        input.type_text("é").unwrap();
        active.recv_timeout(Duration::from_secs(3)).unwrap();
        let mut peers = Vec::new();
        for _ in 0..MAX_DRAINING {
            assert!(input.admits_owner());
            let (connection, peer) = connection_pair();
            input.release_owner(connection).unwrap();
            peers.push(peer);
        }
        assert!(!input.admits_owner());
        for peer in &mut peers {
            assert_open(peer);
        }
        resume.send(()).unwrap();
        for peer in &mut peers {
            peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            assert_eq!(peer.read(&mut [0]).unwrap(), 0);
        }
    }

    #[test]
    fn tests_that_failed_input_aborts_a_pending_handoff_instead_of_signalling_success() {
        failed_handoff(false);
        failed_handoff(true);
    }

    fn failed_handoff(fail_on_release: bool) {
        let (events, _) = mpsc::sync_channel(4);
        let (resume, wait) = mpsc::channel();
        let (started, active) = mpsc::channel();
        let mut input = Input::spawn(events, move |command| {
            if matches!(command, Command::Type(_)) {
                started.send(()).unwrap();
                wait.recv_timeout(Duration::from_secs(3)).unwrap();
                if fail_on_release {
                    return Ok(());
                }
            }
            Err(io::Error::other("remote input failed"))
        })
        .unwrap();
        let (connection, mut peer) = connection_pair();
        input.type_text("é").unwrap();
        active.recv_timeout(Duration::from_secs(3)).unwrap();
        input.release_owner(connection).unwrap();
        resume.send(()).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let result = peer.read(&mut [0]);
        assert!(
            matches!(result, Err(ref e) if e.kind() == io::ErrorKind::ConnectionReset),
            "failed input must not look like a successful handback: {result:?}"
        );
    }

    #[test]
    fn tests_that_a_blocked_commit_keeps_keys_and_handoff_in_order_without_blocking_the_caller() {
        let (events, _) = mpsc::sync_channel(4);
        let (started, active) = mpsc::channel();
        let (resume, wait) = mpsc::channel();
        let (finished, log) = mpsc::channel();
        let mut input = Input::spawn(events, move |command| {
            let entry = match command {
                Command::Pointer(..) => "click",
                Command::Type(text) => {
                    assert_eq!(text, "नमस्ते");
                    started.send(()).unwrap();
                    wait.recv_timeout(Duration::from_secs(3)).unwrap();
                    "commit"
                }
                Command::Key(0xff08, _) => "backspace",
                Command::Release => "release",
                Command::Key(_, _) => "new owner",
                Command::Paste { .. }
                | Command::Wheel(..)
                | Command::ClipboardWatch(_)
                | Command::FieldsWatch(_) => {
                    unreachable!()
                }
            };
            let _ = finished.send(entry);
            Ok(())
        })
        .unwrap();
        input.pointer(1, 2, 1).unwrap();
        input.type_text("नमस्ते").unwrap();
        active.recv_timeout(Duration::from_secs(3)).unwrap();
        input.key(0xff08, true).unwrap();
        input.release_all().unwrap();
        input.key(97, true).unwrap();
        assert_eq!(log.try_iter().collect::<Vec<_>>(), ["click"]);
        resume.send(()).unwrap();
        for expected in ["commit", "backspace", "release", "new owner"] {
            assert_eq!(log.recv_timeout(Duration::from_secs(3)).unwrap(), expected);
        }
    }

    #[test]
    fn tests_that_overflow_is_bounded_and_still_releases_held_input() {
        let (events, _) = mpsc::sync_channel(4);
        let (started, active) = mpsc::channel();
        let (resume, wait) = mpsc::channel();
        let (finished, done) = mpsc::channel();
        let mut input = Input::spawn(events, move |command| {
            if matches!(command, Command::Type(_)) {
                started.send(()).unwrap();
                wait.recv_timeout(Duration::from_secs(3)).unwrap();
            }
            if matches!(command, Command::Release) {
                let _ = finished.send(());
            }
            Ok(())
        })
        .unwrap();
        input.type_text("é").unwrap();
        active.recv_timeout(Duration::from_secs(3)).unwrap();
        for _ in 0..MAX_PENDING {
            input.key(97, true).unwrap();
        }
        assert_eq!(
            input.key(97, false).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        for _ in 0..100 {
            input.release_all().unwrap();
        }
        assert_eq!(
            input.queue.state.lock().unwrap().pending.len(),
            MAX_PENDING + 1
        );
        resume.send(()).unwrap();
        done.recv_timeout(Duration::from_secs(3)).unwrap();
    }

    #[test]
    fn tests_that_worker_failure_reaches_the_engine() {
        let (events, errors) = mpsc::sync_channel(4);
        let mut input = Input::spawn(events, |_| Err(io::Error::other("peer gone"))).unwrap();
        input.key(97, true).unwrap();
        assert!(
            matches!(errors.recv_timeout(Duration::from_secs(3)), Ok(Ev::Fatal(e)) if e.contains("peer gone"))
        );
        assert_eq!(
            input.key(97, false).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }
}
