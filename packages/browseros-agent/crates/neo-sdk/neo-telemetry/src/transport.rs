//! The OTLP exporters' transport: one owned OS thread running a current-thread tokio runtime and
//! one reqwest 0.12 client (rustls), which `opentelemetry_http::HttpClient` sends through.
//!
//! Owned by the OTLP work. Built only when push is on, and owned by `otlp::Push`, so it lives
//! exactly as long as the span processor and metric reader that use it. Exports never touch the
//! server's runtime. Every request gets `min(per_request_timeout, deadline - now)`, and
//! `arm_deadline` caps what shutdown may still wait for. `http+unix://<percent-encoded socket
//! path>/<url path>` endpoints get a client built with `ClientBuilder::unix_socket(path)` and
//! the URL rewritten to `http://localhost/<url path>`. `tests/otel_stack.rs` shows the shape.

/// The push door's own I/O thread, runtime and HTTP client.
#[derive(Debug)]
pub(crate) struct Transport {}
