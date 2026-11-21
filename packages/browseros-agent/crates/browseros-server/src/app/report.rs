//! Error reporting: which failures leave the machine, and in what form (design section 3.7).
//!
//! Three things are reported, and only when a DSN is set (`BROWSEROS_SENTRY_DSN` in the
//! environment, else the value baked in at build time; a development build has neither): a
//! panic, a failed start, and an unhandled `500`. Each report is built only from text this file
//! chooses (a source location, a start-failure kind, a method and a route template), never from
//! an error message, a path or a request, because those carry the user's folders, page text and
//! keys. The client is claw-server's hardened one: no ambient environment, no integrations, no
//! PII, no breadcrumbs.

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{MatchedPath, Request};
use axum::middleware::Next;
use axum::response::Response;

/// The DSN a release build may bake in; absent in development.
const BUILD_DSN: Option<&str> = option_env!("BROWSEROS_SENTRY_DSN");

/// The process's reporter. Cheap to clone; a no-op without a DSN.
#[derive(Clone, Default)]
pub(super) struct Reporter {
    client: Option<Arc<sentry::Client>>,
}

impl Reporter {
    /// From the environment, then the build. A blank or invalid DSN means off.
    pub(super) fn from_env() -> Self {
        let dsn = std::env::var("BROWSEROS_SENTRY_DSN")
            .ok()
            .or_else(|| BUILD_DSN.map(str::to_owned))
            .filter(|dsn| !dsn.trim().is_empty());
        Self {
            client: dsn.as_deref().and_then(client).map(Arc::new),
        }
    }

    /// `event` is one of the fixed texts below; the caller never passes user data.
    fn send(&self, event: &str) {
        if let Some(client) = &self.client {
            let hub = sentry::Hub::new(Some(client.clone()), Arc::new(sentry::Scope::default()));
            hub.capture_message(event, sentry::Level::Error);
        }
    }

    pub(super) fn panic(&self, location: Option<&std::panic::Location<'_>>) {
        self.send(&panic_event(location));
    }

    pub(super) fn failed_start(&self, kind: &'static str) {
        self.send(&format!("start failed: {kind}"));
    }

    /// Waits for queued reports to leave, bounded: called at the end of shutdown and after a
    /// failed start, inside the time the launcher allows.
    pub(super) fn flush(&self, within: Duration) {
        if let Some(client) = &self.client {
            client.flush(Some(within));
        }
    }

    /// Middleware: reports a `500` by method and route template (`/providers/{providerId}`),
    /// which names no user data.
    pub(super) async fn watch_500s(self, request: Request, next: Next) -> Response {
        let method = request.method().clone();
        let route = request
            .extensions()
            .get::<MatchedPath>()
            .map_or_else(|| "unmatched".to_owned(), |path| path.as_str().to_owned());
        let response = next.run(request).await;
        if response.status() == axum::http::StatusCode::INTERNAL_SERVER_ERROR {
            self.send(&format!("500 {method} {route}"));
        }
        response
    }
}

fn panic_event(location: Option<&std::panic::Location<'_>>) -> String {
    match location {
        Some(location) => format!("panic at {}:{}", location.file(), location.line()),
        None => "panic".to_owned(),
    }
}

fn client(dsn: &str) -> Option<sentry::Client> {
    let options = sentry::ClientOptions {
        dsn: dsn.parse().ok(),
        // `Client::from_config` installs no transport of its own; without this factory the
        // client holds a valid DSN and sends nothing.
        transport: Some(Arc::new(sentry::transports::DefaultTransportFactory)),
        release: Some(Cow::Borrowed(crate::VERSION)),
        environment: Some(Cow::Borrowed(if cfg!(debug_assertions) {
            "development"
        } else {
            "production"
        })),
        send_default_pii: false,
        attach_stacktrace: false,
        max_breadcrumbs: 0,
        ..Default::default()
    };
    let client = sentry::Client::from_config(options);
    client.is_enabled().then_some(client)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_dsn_nothing_is_built_and_reporting_is_a_no_op() {
        let reporter = Reporter::default();
        assert!(reporter.client.is_none());
        reporter.failed_start("store");
        reporter.flush(Duration::from_millis(1));
    }

    #[test]
    fn a_panic_is_reported_by_its_location_only() {
        assert_eq!(panic_event(None), "panic");
        let location = std::panic::Location::caller();
        assert!(panic_event(Some(location)).starts_with("panic at "));
    }

    #[test]
    fn an_invalid_dsn_is_off() {
        assert!(client("not a dsn").is_none());
    }
}
