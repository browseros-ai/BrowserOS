//! Minimal process-error reporting, following claw's explicit-event Sentry setup. Ambient
//! request state, breadcrumbs and default integrations never enter an event; startup errors
//! and panics are redacted before capture. Keep the guard until process shutdown to flush.

use std::{
    sync::{Arc, OnceLock, RwLock, Weak},
    time::Duration,
};

const BUILD_DSN: Option<&str> = option_env!("SENTRY_DSN");
const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);
static PANIC_CLIENT: RwLock<Weak<sentry::Client>> = RwLock::new(Weak::new());
static PANIC_HOOK: OnceLock<()> = OnceLock::new();

/// Owns only this process's client. Constructing it does not bind a global Sentry hub, so
/// unrelated request errors cannot bypass the redaction in this module.
#[derive(Debug, Default)]
pub struct SentryGuard {
    client: Option<Arc<sentry::Client>>,
}

/// The process's one Sentry client. `main` creates it before CLI/config parsing (so startup
/// failures are reported), keeps it until the runtime has shut down (dropping it flushes), and
/// shares it as `Arc<SentryGuard>` with `http::security::ingress`, which reports every request
/// that ended on TS's unhandled-error path (`app.onError`, the `/mcp` catch).
pub fn init() -> SentryGuard {
    let Some(client) = BUILD_DSN.and_then(build_client) else {
        return SentryGuard::default();
    };
    install_panic_hook(&client);
    SentryGuard {
        client: Some(client),
    }
}

fn build_client(dsn: &str) -> Option<Arc<sentry::Client>> {
    let dsn = dsn.parse().ok()?;
    // Bypass init/apply_defaults: they read ambient env and add integrations. The explicit
    // factory is required here or Client::from_config would create a disabled transport.
    let client = Arc::new(sentry::Client::from_config(sentry::ClientOptions {
        dsn: Some(dsn),
        transport: Some(Arc::new(sentry::transports::DefaultTransportFactory)),
        default_integrations: false,
        send_default_pii: false,
        attach_stacktrace: false,
        max_breadcrumbs: 0,
        release: Some(crate::VERSION.into()),
        environment: std::env::var("NODE_ENV").ok().map(Into::into),
        ..Default::default()
    }));
    client.is_enabled().then_some(client)
}

fn install_panic_hook(client: &Arc<sentry::Client>) {
    *PANIC_CLIENT.write().unwrap_or_else(|e| e.into_inner()) = Arc::downgrade(client);
    PANIC_HOOK.get_or_init(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let client = PANIC_CLIENT
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .upgrade();
            if let Some(client) = client {
                let message = info
                    .payload()
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
                    .unwrap_or("non-string panic payload");
                client.capture_event(event("panic", message, &[]), None);
            }
            // Preserve the application's previous hook; ownership is never taken back on
            // guard drop, which could overwrite a hook installed by another module later.
            previous(info);
        }));
    });
}

impl SentryGuard {
    pub fn is_enabled(&self) -> bool {
        self.client.is_some()
    }

    pub fn capture_startup_error(&self, message: &str) {
        if let Some(client) = &self.client {
            client.capture_event(event("startup", message, &[]), None);
        }
    }

    /// A request that ended on TS's unhandled-error path, with the scope tags TS sets there
    /// (`route`, `method`, `request-type` for chat; `route=mcp`, `leased` for `/mcp`). Only
    /// the failure message and these tags are sent: no prompts, URLs or user identity from an
    /// ambient hub.
    pub fn capture_request_error(&self, message: &str, tags: &[(&str, &str)]) {
        if let Some(client) = &self.client {
            client.capture_event(event("request", message, tags), None);
        }
    }
}

impl Drop for SentryGuard {
    fn drop(&mut self) {
        if let Some(client) = &self.client {
            client.close(Some(FLUSH_TIMEOUT));
        }
    }
}

fn event(kind: &str, message: &str, tags: &[(&str, &str)]) -> sentry::protocol::Event<'static> {
    // Build by allowlist, as claw does. Exception chains, request bodies, stack locals and
    // breadcrumbs can contain secrets; omitting them is stronger than recursive scrubbing.
    let mut event = sentry::protocol::Event::new();
    event.level = if kind == "panic" {
        sentry::Level::Fatal
    } else {
        sentry::Level::Error
    };
    event.logger = Some(kind.to_owned());
    event.message = Some(browseros_agent::llm::redact(message));
    event.tags.insert("surface".to_owned(), "server".to_owned());
    for (name, value) in tags {
        event.tags.insert((*name).to_owned(), (*value).to_owned());
    }
    event
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        sync::mpsc,
    };

    #[test]
    fn invalid_dsn_disables_without_runtime_fallback() {
        assert!(build_client("not a dsn").is_none());
        assert!(build_client("").is_none());
        assert!(!SentryGuard::default().is_enabled());
    }

    #[test]
    fn explicit_events_are_redacted_and_have_no_ambient_state() -> anyhow::Result<()> {
        let event = event(
            "request",
            "bad Bearer abcdefghijklmnopqrstuvwxyz and sk-abcdefghijklmnopqrstuv",
            &[("request-type", "schedule")],
        );
        let encoded = serde_json::to_value(event)?;
        assert!(!encoded.to_string().contains("abcdefghijklmnopqrstuvwxyz"));
        assert!(!encoded.to_string().contains("sk-abcdefghijklmnopqrstuv"));
        assert_eq!(encoded["tags"]["request-type"], "schedule");
        for field in ["request", "user", "breadcrumbs", "exception", "stacktrace"] {
            assert!(
                encoded.get(field).is_none(),
                "unexpected {field}: {encoded}"
            );
        }
        Ok(())
    }

    #[test]
    fn startup_capture_reaches_transport_and_guard_drop_flushes() -> anyhow::Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let dsn = format!("http://public@{}/1", listener.local_addr()?);
        let (send, received) = mpsc::channel();
        let thread = std::thread::spawn(move || -> std::io::Result<()> {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;
            let mut reader = BufReader::new(&stream);
            let mut first = String::new();
            reader.read_line(&mut first)?;
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line)?;
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap_or_default();
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body)?;
            stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}")?;
            let _ = send.send((first, body));
            Ok(())
        });
        let guard = SentryGuard {
            client: build_client(&dsn),
        };
        assert!(guard.is_enabled());
        guard.capture_startup_error("failed with sk-abcdefghijklmnopqrstuv");
        drop(guard);
        let (first, bytes) = received.recv_timeout(Duration::from_secs(5))?;
        assert!(first.starts_with("POST /api/1/envelope/"));
        let envelope = String::from_utf8(bytes)?;
        assert!(envelope.contains("startup"));
        assert!(!envelope.contains("sk-abcdefghijklmnopqrstuv"));
        thread
            .join()
            .map_err(|_| anyhow::anyhow!("ingest thread panicked"))??;
        Ok(())
    }
}
