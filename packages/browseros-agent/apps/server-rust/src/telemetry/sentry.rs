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

pub fn init() -> SentryGuard {
    let Some(dsn) = BUILD_DSN
        .filter(|dsn| !dsn.is_empty())
        .and_then(|dsn| dsn.parse().ok())
    else {
        return SentryGuard::default();
    };
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
    if !client.is_enabled() {
        return SentryGuard::default();
    }
    *PANIC_CLIENT.write().unwrap_or_else(|e| e.into_inner()) = Arc::downgrade(&client);
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
                client.capture_event(event("panic", message, None), None);
            }
            // Preserve the application's previous hook; ownership is never taken back on
            // guard drop, which could overwrite a hook installed by another module later.
            previous(info);
        }));
    });
    SentryGuard {
        client: Some(client),
    }
}

impl SentryGuard {
    pub fn is_enabled(&self) -> bool {
        self.client.is_some()
    }

    pub fn capture_startup_error(&self, message: &str) {
        if let Some(client) = &self.client {
            client.capture_event(event("startup", message, None), None);
        }
    }

    /// A scheduled chat failure is explicitly tagged without inheriting prompts, URLs or
    /// user identity from an ambient hub. Callers pass only the failure message.
    pub fn capture_scheduled_error(&self, message: &str) {
        if let Some(client) = &self.client {
            client.capture_event(event("chat", message, Some("schedule")), None);
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

fn event(
    kind: &str,
    message: &str,
    request_type: Option<&str>,
) -> sentry::protocol::Event<'static> {
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
    if let Some(request_type) = request_type {
        event
            .tags
            .insert("request-type".to_owned(), request_type.to_owned());
    }
    event
}
