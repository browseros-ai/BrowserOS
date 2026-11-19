//! slimviewer: streams an X11 screen (Neo's Xvnc display) to a phone browser as H.264 that the
//! phone's own decoder shows (WebCodecs), and takes touch, keys and paste back.
//!
//!   slimviewer --display :1 --rfb 127.0.0.1:5900 --listen 127.0.0.1:5910 --listen-view 127.0.0.1:5911
//!
//! The host chooses the viewer's role by dialing the owner or view-only port.

mod capture;
mod clipboard;
mod encoder;
mod engine;
mod picture;
mod rfb;
mod wire;
mod yuv;

use std::io::BufReader;
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::time::Duration;

use engine::{Config, Ev, Viewer};
use wire::FromPage;

const USAGE: &str =
    "usage: slimviewer [--display :1] [--rfb 127.0.0.1:5900] [--listen 127.0.0.1:5910]
       [--listen-view 127.0.0.1:5911] [--preset ultrafast] [--threads 0=auto] [--crf 24] [--fps 30]
       [--sharpen-qp 18] [--sharpen-ms 250] [--crf-range 12] [--no-cpu-guard]
       [--full-size-only] [--encoder x264|openh264] [--max-viewers 8] [--version]
Owner: the newest connection replaces the previous owner.
View-only clients: capped by --max-viewers; 0 disables them.
Owner ACK timeout: reset the stream at 3 seconds; disconnect after 15 seconds without ACK progress.
View-only ACK timeout: disconnect after 3 seconds.";

fn main() {
    if let Err(e) = run() {
        eprintln!(
            "slimviewer: event=fatal error={}",
            e.replace(['\r', '\n'], " ")
        );
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    install_sigterm()?;
    let mut a = Args(std::env::args().skip(1).collect());
    if a.flag("--help") {
        println!("{USAGE}");
        println!("Default encoder: {}", encoder::Backend::default_name());
        println!("--preset is only available with x264.");
        return Ok(());
    }
    if a.flag("--version") {
        println!("slimviewer {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let display = a.value("--display", ":1")?;
    let rfb = a.value("--rfb", "127.0.0.1:5900")?;
    let listen = a.value("--listen", "127.0.0.1:5910")?;
    let listen_view = a.value("--listen-view", "127.0.0.1:5911")?;
    let backend = a
        .value("--encoder", encoder::Backend::default_name())?
        .parse()?;
    if backend != encoder::Backend::X264 && a.0.iter().any(|arg| arg == "--preset") {
        return Err("--preset is only available with x264".into());
    }
    let cfg = Config {
        encoder: encoder::Settings {
            backend,
            preset: a.value("--preset", "ultrafast")?,
            threads: a.num("--threads", 0)?,
            crf: a.num("--crf", 24.0)?,
        },
        max_fps: a.num("--fps", 30.0)?,
        sharpen_qp: a.num("--sharpen-qp", 18)?,
        sharpen_after: Duration::from_millis(a.num("--sharpen-ms", 250)?),
        crf_range: a.num("--crf-range", 12.0)?,
        cpu_guard: !a.flag("--no-cpu-guard"),
        adaptive_size: !a.flag("--full-size-only"),
        max_viewers: a.num("--max-viewers", 8)?,
    };
    if let Some(extra) = a.0.first() {
        return Err(format!("unknown argument {extra}; use --help"));
    }
    validate(&cfg)?;
    let (tx, rx) = mpsc::sync_channel(128);
    let input = rfb::Input::connect(&rfb, &display, tx.clone())?;
    let cap = capture::Capture::open(&display, tx.clone())?;
    eprintln!(
        "slimviewer: {display} is {}x{}; owner {listen}, view-only {listen_view}",
        cap.width, cap.height
    );
    for (addr, view_only) in [(listen, false), (listen_view, true)] {
        let listener = TcpListener::bind(&addr).map_err(|e| format!("listen {addr}: {e}"))?;
        let tx = tx.clone();
        std::thread::Builder::new()
            .name(format!("accept-{addr}"))
            .spawn(move || accept(&listener, view_only, &tx))
            .map_err(|e| format!("start listener: {e}"))?;
    }
    drop(tx);
    engine::run(cfg, cap, input, rx)
}

fn validate(cfg: &Config) -> Result<(), String> {
    for (name, value, low, high) in [
        ("--fps", cfg.max_fps, 0.1, 240.0),
        ("--crf", cfg.encoder.crf, 0.0, 51.0),
        ("--crf-range", cfg.crf_range, 0.0, 51.0 - cfg.encoder.crf),
    ] {
        if !value.is_finite() || value < low || value > high {
            return Err(format!("{name} must be between {low} and {high}"));
        }
    }
    if cfg.sharpen_qp > 51 {
        return Err("--sharpen-qp must be between 0 and 51".into());
    }
    if cfg.encoder.threads > 64 {
        return Err("--threads must be between 0 and 64".into());
    }
    if cfg.max_viewers > 128 {
        return Err("--max-viewers must be between 0 and 128".into());
    }
    Ok(())
}

fn install_sigterm() -> Result<(), String> {
    extern "C" fn terminate(_: libc::c_int) {
        // _exit is async-signal-safe, including during blocked X/RFB startup. SHM is IPC_RMID.
        unsafe { libc::_exit(0) }
    }
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = terminate as *const () as libc::sighandler_t;
    unsafe {
        libc::sigemptyset(&mut action.sa_mask);
        if libc::sigaction(libc::SIGTERM, &action, std::ptr::null_mut()) != 0 {
            return Err(format!(
                "install SIGTERM handler: {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    Ok(())
}

static NEXT_VIEWER: AtomicU64 = AtomicU64::new(1);

fn accept(listener: &TcpListener, view_only: bool, tx: &SyncSender<Ev>) {
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                let _ = tx.send(Ev::Fatal(format!("accept failed: {e}")));
                return;
            }
        };
        let id = NEXT_VIEWER.fetch_add(1, Ordering::Relaxed);
        let _ = stream.set_nodelay(true);
        let Ok(reader) = stream.try_clone() else {
            continue;
        };
        let viewer = match Viewer::new(id, view_only, stream) {
            Ok(viewer) => viewer,
            Err(e) => {
                eprintln!("slimviewer: event=disconnect viewer={id} error={e}");
                continue;
            }
        };
        let (admit, admitted) = mpsc::sync_channel(1);
        if tx.send(Ev::Joined(viewer, admit)).is_err() {
            return;
        }
        // Admission precedes the reader thread, bounding idle sockets and threads too.
        if !admitted.recv().unwrap_or(false) {
            continue;
        }
        let reader_tx = tx.clone();
        if let Err(e) = std::thread::Builder::new()
            .name(format!("viewer-{id}-reader"))
            .spawn(move || {
                let mut reader = BufReader::new(reader);
                // Input and Left use one sender, so accepted input drains before disconnect.
                while let Ok(msg) = wire::read(&mut reader, view_only) {
                    let event = match msg {
                        Some(FromPage::Ack(seq)) => Ev::Ack(id, seq),
                        Some(FromPage::Keyframe) => Ev::Keyframe(id),
                        Some(input) => Ev::Input(id, input),
                        None => continue,
                    };
                    if reader_tx.send(event).is_err() {
                        return;
                    }
                }
                let _ = reader_tx.send(Ev::Left(id));
            })
        {
            let _ = tx.send(Ev::Fatal(format!("start viewer reader: {e}")));
            return;
        }
    }
}

/// `--name value` and `--flag` arguments; whatever is left over is an error.
struct Args(Vec<String>);

impl Args {
    fn flag(&mut self, name: &str) -> bool {
        let found = self.0.iter().position(|a| a == name);
        found.map(|i| self.0.remove(i)).is_some()
    }
    fn value(&mut self, name: &str, default: &str) -> Result<String, String> {
        match self.0.iter().position(|a| a == name) {
            Some(i) if i + 1 < self.0.len() && !self.0[i + 1].starts_with("--") => {
                self.0.remove(i);
                Ok(self.0.remove(i))
            }
            Some(_) => Err(format!("{name} needs a value")),
            None => Ok(default.to_string()),
        }
    }
    fn num<T: std::str::FromStr>(&mut self, name: &str, default: T) -> Result<T, String> {
        match self.0.iter().position(|a| a == name) {
            Some(_) => self
                .value(name, "")?
                .parse()
                .map_err(|_| format!("{name} needs a number")),
            None => Ok(default),
        }
    }
}
