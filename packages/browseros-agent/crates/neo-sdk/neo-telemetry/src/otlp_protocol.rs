//! The OTLP/HTTP body encoding.

/// The OTLP/HTTP body encoding (`OTEL_EXPORTER_OTLP_PROTOCOL`). gRPC is not supported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum OtlpProtocol {
    /// Protobuf bodies, `Content-Type: application/x-protobuf`. The default.
    #[default]
    HttpProtobuf,
    /// The OTLP JSON encoding, `Content-Type: application/json`: what Neo Viewer reads without
    /// a protobuf library.
    HttpJson,
}

impl OtlpProtocol {
    /// The spelling `OTEL_EXPORTER_OTLP_PROTOCOL` takes.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HttpProtobuf => "http/protobuf",
            Self::HttpJson => "http/json",
        }
    }
}
