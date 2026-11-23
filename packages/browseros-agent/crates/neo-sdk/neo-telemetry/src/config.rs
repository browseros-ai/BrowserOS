//! The standard `OTEL_*` settings, read once at startup: where the OTLP/HTTP push door sends spans
//! and metrics, in which encoding and with which headers, how often, and the resource and sampler
//! both providers share.
//!
//! `TelemetryConfig::from_env` calls [`OtlpConfig::from_env`] with the process environment (or a
//! test's map). Every variable is optional; a set one Neo cannot use fails startup with an error
//! that names it. Nothing here touches the network or the SDK: `otlp.rs` turns the result into
//! exporters, `transport.rs` into the client they send through.
//!
//! Headers, protocol and timeout are per signal, as the specification orders them: a signal's own
//! variable (`OTEL_EXPORTER_OTLP_TRACES_HEADERS`) when set, else the general one
//! (`OTEL_EXPORTER_OTLP_HEADERS`); a signal's headers replace the general ones, they do not add.
//!
//! The headers variables carry secrets (tokens, API keys). Their values leave [`Headers`] only
//! onto a request: `Debug` prints header names, errors name an entry by its name or its position,
//! and an endpoint may not carry `user:password@`, since headers are where credentials go.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use http::header::{HeaderMap, HeaderName, HeaderValue};
use percent_encoding::percent_decode_str;
use url::Url;

use crate::{OtlpProtocol, TelemetryError};

// The variables, as the OpenTelemetry specification spells them.
const ENDPOINT: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";
const PROTOCOL: &str = "OTEL_EXPORTER_OTLP_PROTOCOL";
const HEADERS: &str = "OTEL_EXPORTER_OTLP_HEADERS";
const TIMEOUT: &str = "OTEL_EXPORTER_OTLP_TIMEOUT";
const SERVICE_NAME: &str = "OTEL_SERVICE_NAME";
const RESOURCE_ATTRIBUTES: &str = "OTEL_RESOURCE_ATTRIBUTES";
const SAMPLER: &str = "OTEL_TRACES_SAMPLER";
const SAMPLER_ARG: &str = "OTEL_TRACES_SAMPLER_ARG";
const METRIC_INTERVAL: &str = "OTEL_METRIC_EXPORT_INTERVAL";
const BATCH_DELAY: &str = "OTEL_BSP_SCHEDULE_DELAY";

// What an unset variable means. The metric interval is Neo's (design §4), shorter than the
// specification's 60 s.
const DEFAULT_SERVICE_NAME: &str = "browseros-neo";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_METRIC_INTERVAL: Duration = Duration::from_secs(15);
const DEFAULT_BATCH_DELAY: Duration = Duration::from_secs(5);

/// One signal's variables, and its path under a base `OTEL_EXPORTER_OTLP_ENDPOINT`.
struct SignalVars {
    endpoint: &'static str,
    protocol: &'static str,
    headers: &'static str,
    timeout: &'static str,
    path: &'static str,
}

const TRACES: SignalVars = SignalVars {
    endpoint: "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
    protocol: "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL",
    headers: "OTEL_EXPORTER_OTLP_TRACES_HEADERS",
    timeout: "OTEL_EXPORTER_OTLP_TRACES_TIMEOUT",
    path: "v1/traces",
};

const METRICS: SignalVars = SignalVars {
    endpoint: "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
    protocol: "OTEL_EXPORTER_OTLP_METRICS_PROTOCOL",
    headers: "OTEL_EXPORTER_OTLP_METRICS_HEADERS",
    timeout: "OTEL_EXPORTER_OTLP_METRICS_TIMEOUT",
    path: "v1/metrics",
};

/// The standard OpenTelemetry environment, parsed. [`Default`] is push off (no exporter, thread
/// or socket), with the default resource and sampler: what an empty environment reads as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OtlpConfig {
    /// The span push. `None` turns it off, and with it every span, since spans leave Neo only
    /// over OTLP.
    pub(crate) traces: Option<SignalPush>,
    /// The metric push. `None` leaves metrics to the scrape.
    pub(crate) metrics: Option<SignalPush>,
    /// The resource's `service.name` (`OTEL_SERVICE_NAME`).
    pub(crate) service_name: String,
    /// `OTEL_RESOURCE_ATTRIBUTES`, applied after the service name, so it wins (wire contract).
    pub(crate) resource_attributes: Vec<Attribute>,
    /// Which traces are recorded (`OTEL_TRACES_SAMPLER` / `_ARG`).
    pub(crate) sampler: SamplerChoice,
    /// How often the metrics are pushed (`OTEL_METRIC_EXPORT_INTERVAL`).
    pub(crate) metric_interval: Duration,
    /// How long ended spans may wait before their batch goes out (`OTEL_BSP_SCHEDULE_DELAY`).
    pub(crate) batch_delay: Duration,
}

impl Default for OtlpConfig {
    fn default() -> Self {
        Self {
            traces: None,
            metrics: None,
            service_name: DEFAULT_SERVICE_NAME.to_owned(),
            resource_attributes: Vec::new(),
            sampler: SamplerChoice::default(),
            metric_interval: DEFAULT_METRIC_INTERVAL,
            batch_delay: DEFAULT_BATCH_DELAY,
        }
    }
}

impl OtlpConfig {
    /// Reads the `OTEL_*` variables through `lookup`. An unset or blank variable keeps its
    /// default; a malformed URL, an unknown enum value or a bad number is an error naming the
    /// variable, which fails Neo's startup. No error echoes a header value.
    pub fn from_env(lookup: &dyn Fn(&str) -> Option<String>) -> Result<Self, TelemetryError> {
        let env = Env { lookup };
        let general = General {
            endpoint: env.parse(ENDPOINT, Endpoint::parse)?,
            protocol: env.parse(PROTOCOL, parse_protocol)?.unwrap_or_default(),
            headers: env.parse(HEADERS, Headers::parse)?.unwrap_or_default(),
            timeout: env.parse(TIMEOUT, parse_millis)?.unwrap_or(DEFAULT_TIMEOUT),
        };
        Ok(Self {
            traces: SignalPush::from_env(&env, &TRACES, &general)?,
            metrics: SignalPush::from_env(&env, &METRICS, &general)?,
            service_name: env
                .get(SERVICE_NAME)
                .unwrap_or_else(|| DEFAULT_SERVICE_NAME.to_owned()),
            resource_attributes: env
                .parse(RESOURCE_ATTRIBUTES, parse_attributes)?
                .unwrap_or_default(),
            sampler: SamplerChoice::from_env(&env)?,
            metric_interval: env
                .parse(METRIC_INTERVAL, parse_millis)?
                .unwrap_or(DEFAULT_METRIC_INTERVAL),
            batch_delay: env
                .parse(BATCH_DELAY, parse_millis)?
                .unwrap_or(DEFAULT_BATCH_DELAY),
        })
    }
}

/// One signal's push: where it goes, in which encoding, with which headers, and how long one
/// export may take, retries included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SignalPush {
    /// The signal's endpoint as given, else the base endpoint with the signal's path appended.
    pub(crate) endpoint: Endpoint,
    /// The body encoding.
    pub(crate) protocol: OtlpProtocol,
    /// Sent on every export request of this signal.
    pub(crate) headers: Headers,
    /// One export's limit (`OTEL_EXPORTER_OTLP_TIMEOUT` or the signal's own).
    pub(crate) timeout: Duration,
}

/// The general variables, which a signal falls back on where its own are unset.
struct General {
    endpoint: Option<Endpoint>,
    protocol: OtlpProtocol,
    headers: Headers,
    timeout: Duration,
}

impl SignalPush {
    /// The push for the signal `vars` names, or `None` when neither its endpoint nor the base is
    /// set. Each of its variables is checked even then: a typo fails startup either way.
    fn from_env(
        env: &Env<'_>,
        vars: &SignalVars,
        general: &General,
    ) -> Result<Option<Self>, TelemetryError> {
        let endpoint = env.parse(vars.endpoint, Endpoint::parse)?;
        let protocol = env.parse(vars.protocol, parse_protocol)?;
        let headers = env.parse(vars.headers, Headers::parse)?;
        let timeout = env.parse(vars.timeout, parse_millis)?;
        let base = general.endpoint.as_ref();
        let Some(endpoint) = endpoint.or_else(|| base.map(|base| base.with_signal_path(vars.path)))
        else {
            return Ok(None);
        };
        Ok(Some(Self {
            endpoint,
            protocol: protocol.unwrap_or(general.protocol),
            headers: headers.unwrap_or_else(|| general.headers.clone()),
            timeout: timeout.unwrap_or(general.timeout),
        }))
    }
}

/// One signal's OTLP endpoint, checked at startup so a request can always be built from it.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Endpoint {
    /// The URL as configured: `http://`, `https://` or `http+unix://`.
    url: Url,
    /// `Some` for `http+unix://`: the socket the client connects to, percent-decoded from the
    /// URL's host. The request itself then goes to `http://localhost/<path>`.
    socket: Option<PathBuf>,
}

impl Endpoint {
    /// Reads one endpoint variable's value. The reason never repeats the value: a URL can carry
    /// a token in its query.
    fn parse(value: &str) -> Result<Self, String> {
        let url = Url::parse(value).map_err(|error| format!("not a URL ({error})"))?;
        if !url.username().is_empty() || url.password().is_some() {
            return Err(format!(
                "must not carry user:password@; send credentials in {HEADERS}"
            ));
        }
        let socket = match url.scheme() {
            "http" | "https" => None,
            "http+unix" => Some(unix_socket(&url)?),
            other => {
                return Err(format!(
                    "the scheme `{other}` is not one of http, https, http+unix"
                ));
            }
        };
        Ok(Self { url, socket })
    }

    /// This endpoint as a base: `signal_path` (`v1/traces`) appended to its path, the query kept.
    fn with_signal_path(&self, signal_path: &str) -> Self {
        let mut url = self.url.clone();
        let path = format!("{}/{signal_path}", url.path().trim_end_matches('/'));
        url.set_path(&path);
        Self {
            url,
            socket: self.socket.clone(),
        }
    }

    /// What the OTLP exporter posts to. For a Unix socket, `http://localhost` and the path: the
    /// socket is the client's to choose, not the URL's.
    pub(crate) fn exporter_url(&self) -> String {
        if self.socket.is_none() {
            return self.url.to_string();
        }
        match self.url.query() {
            Some(query) => format!("http://localhost{}?{query}", self.url.path()),
            None => format!("http://localhost{}", self.url.path()),
        }
    }

    /// The Unix socket to connect to, for an `http+unix://` endpoint.
    pub(crate) fn socket(&self) -> Option<&Path> {
        self.socket.as_deref()
    }
}

/// Scheme, host, port and path only: a query can carry a token, so it shows as `?<redacted>`.
impl fmt::Debug for Endpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let url = &self.url;
        write!(
            formatter,
            "{}://{}",
            url.scheme(),
            url.host_str().unwrap_or_default()
        )?;
        if let Some(port) = url.port() {
            write!(formatter, ":{port}")?;
        }
        formatter.write_str(url.path())?;
        if url.query().is_some() {
            formatter.write_str("?<redacted>")?;
        }
        Ok(())
    }
}

/// The socket path an `http+unix://` URL names: its host, percent-decoded, absolute. An
/// unencoded path (`http+unix:///run/otel.sock`) leaves the host empty, so it is refused here
/// instead of posting to a socket called `run`.
fn unix_socket(url: &Url) -> Result<PathBuf, String> {
    const EXAMPLE: &str = "http+unix://%2Frun%2Fotel.sock/v1/traces";
    let host = url.host_str().unwrap_or_default();
    if host.is_empty() {
        return Err(format!(
            "names no socket: percent-encode the socket path, as in {EXAMPLE}"
        ));
    }
    if url.port().is_some() {
        return Err("a Unix socket takes no port".to_owned());
    }
    let socket = percent_decode_str(host)
        .decode_utf8()
        .map_err(|error| format!("the socket path is not UTF-8 ({error})"))?;
    if !socket.starts_with('/') {
        return Err(format!(
            "the socket path must be absolute and percent-encoded, as in {EXAMPLE}"
        ));
    }
    Ok(PathBuf::from(socket.into_owned()))
}

/// `OTEL_EXPORTER_OTLP_HEADERS`: `name=value` pairs, values percent-decoded (OpenTelemetry's
/// format), checked and marked sensitive, so `Debug` here, in `http` and in `reqwest` shows names
/// only. The transport, not the exporter, puts them on each request.
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct Headers(HeaderMap);

impl Headers {
    /// Reads the variable's value. An error names a well-formed header, or else the entry's
    /// position, and never a value.
    fn parse(value: &str) -> Result<Self, String> {
        let mut headers = HeaderMap::new();
        for (index, entry) in value.split(',').enumerate() {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            let position = index + 1;
            let Some((name, value)) = entry.split_once('=') else {
                return Err(format!("entry {position} has no `=` (expected name=value)"));
            };
            let name = HeaderName::from_bytes(name.trim().as_bytes())
                .map_err(|error| format!("entry {position} has no valid header name ({error})"))?;
            let decoded = percent_decode_str(value.trim())
                .decode_utf8()
                .map_err(|error| {
                    format!("the value of `{name}` is not UTF-8 once percent-decoded ({error})")
                })?;
            let mut value = HeaderValue::from_str(&decoded).map_err(|error| {
                format!("the value of `{name}` is not a valid header value ({error})")
            })?;
            value.set_sensitive(true);
            headers.insert(name, value);
        }
        Ok(Self(headers))
    }

    /// Every configured header, for the transport to put on a request.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&HeaderName, &HeaderValue)> {
        self.0.iter()
    }
}

impl fmt::Debug for Headers {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_list().entries(self.0.keys()).finish()
    }
}

/// One `OTEL_RESOURCE_ATTRIBUTES` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Attribute {
    /// The attribute's key, as written.
    pub(crate) key: String,
    /// Its value, percent-decoded.
    pub(crate) value: String,
}

/// `key=value` pairs, values percent-decoded, as the specification defines the variable.
fn parse_attributes(value: &str) -> Result<Vec<Attribute>, String> {
    let mut attributes = Vec::new();
    for (index, entry) in value.split(',').enumerate() {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let position = index + 1;
        let Some((key, value)) = entry.split_once('=') else {
            return Err(format!("entry {position} has no `=` (expected key=value)"));
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(format!("entry {position} has an empty key"));
        }
        let value = percent_decode_str(value.trim())
            .decode_utf8()
            .map_err(|error| {
                format!("the value of `{key}` is not UTF-8 once percent-decoded ({error})")
            })?;
        attributes.push(Attribute {
            key: key.to_owned(),
            value: value.into_owned(),
        });
    }
    Ok(attributes)
}

/// `OTEL_TRACES_SAMPLER`, with `OTEL_TRACES_SAMPLER_ARG` for the two ratio samplers. `otlp.rs`
/// maps it onto the SDK's sampler.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SamplerChoice {
    /// `parentbased_always_on`: follow the caller's sampling decision, sample new traces.
    #[default]
    ParentBasedAlwaysOn,
    /// `parentbased_always_off`: follow the caller's decision, drop new traces.
    ParentBasedAlwaysOff,
    /// `parentbased_traceidratio`: follow the caller's decision, sample this share of new ones.
    ParentBasedTraceIdRatio(Ratio),
    /// `always_on`.
    AlwaysOn,
    /// `always_off`.
    AlwaysOff,
    /// `traceidratio`: this share of traces, whatever the caller decided.
    TraceIdRatio(Ratio),
}

impl SamplerChoice {
    fn from_env(env: &Env<'_>) -> Result<Self, TelemetryError> {
        let Some(name) = env.get(SAMPLER) else {
            return Ok(Self::default());
        };
        // The specification's default argument for the ratio samplers is 1. Other samplers
        // take no argument, so theirs is not read.
        let ratio = || -> Result<Ratio, TelemetryError> {
            Ok(env.parse(SAMPLER_ARG, Ratio::parse)?.unwrap_or(Ratio::ALL))
        };
        Ok(match name.as_str() {
            "parentbased_always_on" => Self::ParentBasedAlwaysOn,
            "parentbased_always_off" => Self::ParentBasedAlwaysOff,
            "parentbased_traceidratio" => Self::ParentBasedTraceIdRatio(ratio()?),
            "always_on" => Self::AlwaysOn,
            "always_off" => Self::AlwaysOff,
            "traceidratio" => Self::TraceIdRatio(ratio()?),
            other => {
                return Err(invalid(
                    SAMPLER,
                    format!(
                        "`{other}` is not one of parentbased_always_on, parentbased_always_off, \
                         parentbased_traceidratio, always_on, always_off, traceidratio"
                    ),
                ));
            }
        })
    }
}

/// A ratio sampler's share of traces, from 0 to 1. Held as the `f64`'s bits, so the config can
/// stay `Eq`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ratio(u64);

impl Ratio {
    /// Every trace.
    const ALL: Self = Self(1.0_f64.to_bits());

    fn parse(value: &str) -> Result<Self, String> {
        let ratio: f64 = value
            .parse()
            .map_err(|error| format!("`{value}` is not a number ({error})"))?;
        if !(0.0..=1.0).contains(&ratio) {
            return Err(format!("`{value}` is not between 0 and 1"));
        }
        Ok(Self(ratio.to_bits()))
    }

    /// The share, for the SDK's `TraceIdRatioBased`.
    pub(crate) fn get(self) -> f64 {
        f64::from_bits(self.0)
    }
}

impl fmt::Debug for Ratio {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.get())
    }
}

fn parse_protocol(value: &str) -> Result<OtlpProtocol, String> {
    [OtlpProtocol::HttpProtobuf, OtlpProtocol::HttpJson]
        .into_iter()
        .find(|protocol| protocol.as_str() == value)
        .ok_or_else(|| {
            if value == "grpc" {
                "gRPC is not supported: use http/protobuf or http/json".to_owned()
            } else {
                format!("`{value}` is not one of http/protobuf, http/json")
            }
        })
}

/// A whole number of milliseconds, at least 1: a zero batch delay would export in a busy loop,
/// and the SDK reads a zero metric interval as its own 60 s default.
fn parse_millis(value: &str) -> Result<Duration, String> {
    match value.parse::<u64>() {
        Ok(0) => Err("must be at least 1 millisecond".to_owned()),
        Ok(millis) => Ok(Duration::from_millis(millis)),
        Err(error) => Err(format!(
            "`{value}` is not a whole number of milliseconds ({error})"
        )),
    }
}

fn invalid(name: &'static str, reason: String) -> TelemetryError {
    TelemetryError::InvalidEnv { name, reason }
}

/// The environment as `from_env` reads it: trimmed, a blank value as unset.
struct Env<'a> {
    lookup: &'a dyn Fn(&str) -> Option<String>,
}

impl Env<'_> {
    fn get(&self, name: &str) -> Option<String> {
        let value = (self.lookup)(name)?;
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_owned())
    }

    /// `name`'s value through `parse`, whose reason becomes an error naming `name`.
    fn parse<T>(
        &self,
        name: &'static str,
        parse: impl FnOnce(&str) -> Result<T, String>,
    ) -> Result<Option<T>, TelemetryError> {
        self.get(name)
            .map(|value| parse(&value).map_err(|reason| invalid(name, reason)))
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::error::Error;

    use super::*;

    fn read(vars: &[(&str, &str)]) -> Result<OtlpConfig, TelemetryError> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        OtlpConfig::from_env(&|name| vars.get(name).cloned())
    }

    /// The variable an error names, and its message, for the bad-value tables.
    fn refusal(vars: &[(&str, &str)]) -> Result<(&'static str, String), Box<dyn Error>> {
        match read(vars) {
            Err(TelemetryError::InvalidEnv { name, reason }) => {
                let shown = TelemetryError::InvalidEnv { name, reason }.to_string();
                Ok((name, shown))
            }
            other => Err(format!("{vars:?} must fail startup, got {other:?}").into()),
        }
    }

    fn urls(config: &OtlpConfig) -> (Option<String>, Option<String>) {
        let url = |push: &SignalPush| push.endpoint.exporter_url();
        (
            config.traces.as_ref().map(url),
            config.metrics.as_ref().map(url),
        )
    }

    /// Both signals' push, with a base endpoint added to `vars`.
    fn pushed(vars: &[(&str, &str)]) -> Result<(SignalPush, SignalPush), Box<dyn Error>> {
        let mut vars = vars.to_vec();
        vars.push((ENDPOINT, "http://collector:4318"));
        let config = read(&vars)?;
        Ok((
            config.traces.ok_or("the base endpoint turns traces on")?,
            config.metrics.ok_or("the base endpoint turns metrics on")?,
        ))
    }

    fn header_names(push: &SignalPush) -> Vec<String> {
        push.headers
            .iter()
            .map(|(name, _)| name.to_string())
            .collect()
    }

    #[test]
    fn an_empty_environment_is_push_off_with_every_default() -> Result<(), Box<dyn Error>> {
        let config = read(&[])?;
        assert_eq!(
            config,
            OtlpConfig::default(),
            "nothing set must read as the default, so TelemetryConfig::default() stays the same"
        );
        assert_eq!(urls(&config), (None, None), "no endpoint means no push");
        assert_eq!(config.service_name, "browseros-neo");
        assert_eq!(config.sampler, SamplerChoice::ParentBasedAlwaysOn);
        assert_eq!(config.metric_interval, Duration::from_secs(15));
        assert_eq!(config.batch_delay, Duration::from_secs(5));
        assert_eq!(
            read(&[(ENDPOINT, "  "), (SERVICE_NAME, ""), (PROTOCOL, " ")])?,
            OtlpConfig::default(),
            "a blank value is unset, as every other Neo variable reads it"
        );
        for push in <[SignalPush; 2]>::from(pushed(&[])?) {
            assert_eq!(push.protocol, OtlpProtocol::HttpProtobuf);
            assert_eq!(push.timeout, Duration::from_secs(10));
            assert!(header_names(&push).is_empty());
        }
        Ok(())
    }

    #[test]
    fn a_base_endpoint_gets_each_signal_path_appended() -> Result<(), Box<dyn Error>> {
        for (base, traces, metrics) in [
            (
                "http://collector:4318",
                "http://collector:4318/v1/traces",
                "http://collector:4318/v1/metrics",
            ),
            (
                "https://collector.example/",
                "https://collector.example/v1/traces",
                "https://collector.example/v1/metrics",
            ),
            (
                "http://127.0.0.1:7310/otlp",
                "http://127.0.0.1:7310/otlp/v1/traces",
                "http://127.0.0.1:7310/otlp/v1/metrics",
            ),
            (
                "http://collector/otlp/?tenant=neo",
                "http://collector/otlp/v1/traces?tenant=neo",
                "http://collector/otlp/v1/metrics?tenant=neo",
            ),
        ] {
            assert_eq!(
                urls(&read(&[(ENDPOINT, base)])?),
                (Some(traces.to_owned()), Some(metrics.to_owned())),
                "the base `{base}` serves both signals, each under its own path"
            );
        }
        Ok(())
    }

    #[test]
    fn a_signal_endpoint_is_used_as_given_and_wins_over_the_base() -> Result<(), Box<dyn Error>> {
        let config = read(&[
            (ENDPOINT, "http://base:4318"),
            (TRACES.endpoint, "http://traces:4318/custom"),
        ])?;
        assert_eq!(
            urls(&config),
            (
                Some("http://traces:4318/custom".to_owned()),
                Some("http://base:4318/v1/metrics".to_owned())
            ),
            "a signal endpoint gets no path appended, and the other signal still uses the base"
        );
        Ok(())
    }

    #[test]
    fn each_signal_pushes_only_when_an_endpoint_covers_it() -> Result<(), Box<dyn Error>> {
        assert_eq!(
            urls(&read(&[(TRACES.endpoint, "http://traces/v1/traces")])?),
            (Some("http://traces/v1/traces".to_owned()), None),
            "a traces endpoint alone pushes no metrics"
        );
        assert_eq!(
            urls(&read(&[(METRICS.endpoint, "http://metrics/v1/metrics")])?),
            (None, Some("http://metrics/v1/metrics".to_owned())),
            "a metrics endpoint alone pushes no spans"
        );
        Ok(())
    }

    #[test]
    fn an_http_unix_endpoint_names_its_socket_and_posts_to_localhost() -> Result<(), Box<dyn Error>>
    {
        let config = read(&[
            (ENDPOINT, "http+unix://%2Frun%2Fneo%20otel.sock"),
            (
                METRICS.endpoint,
                "HTTP+UNIX://%2ftmp%2Fm.sock/custom/metrics?x=1",
            ),
        ])?;
        let traces = config.traces.as_ref().ok_or("traces must push")?;
        let metrics = config.metrics.as_ref().ok_or("metrics must push")?;
        assert_eq!(
            traces.endpoint.socket(),
            Some(Path::new("/run/neo otel.sock"))
        );
        assert_eq!(traces.endpoint.exporter_url(), "http://localhost/v1/traces");
        assert_eq!(
            metrics.endpoint.socket(),
            Some(Path::new("/tmp/m.sock")),
            "the scheme is case-blind and the encoding's hex digits too"
        );
        assert_eq!(
            metrics.endpoint.exporter_url(),
            "http://localhost/custom/metrics?x=1",
            "a signal endpoint's path and query are kept as given"
        );
        Ok(())
    }

    #[test]
    fn a_malformed_endpoint_fails_naming_its_variable_without_echoing_it()
    -> Result<(), Box<dyn Error>> {
        for name in [ENDPOINT, TRACES.endpoint, METRICS.endpoint] {
            for (value, why) in [
                ("collector:4318/secret-path", "no scheme"),
                ("ftp://collector/secret-path", "an unknown scheme"),
                ("http://", "no host"),
                (
                    "http://user:secret-pass@collector:4318",
                    "credentials in the URL",
                ),
                (
                    "http+unix:///run/secret.sock/v1/traces",
                    "an unencoded socket path",
                ),
                (
                    "http+unix://run%2Fsecret.sock/v1/traces",
                    "a relative socket path",
                ),
                (
                    "http+unix://%2Frun%2Fsecret.sock:80/v1",
                    "a port on a socket",
                ),
            ] {
                let (named, message) = refusal(&[(name, value)])?;
                assert_eq!(named, name, "{why} must fail startup naming {name}");
                assert!(
                    !message.contains("secret"),
                    "{why}: the error must not repeat the value, which can hold a token: {message}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn the_protocol_is_one_of_the_two_http_encodings() -> Result<(), Box<dyn Error>> {
        assert_eq!(
            pushed(&[(PROTOCOL, "http/protobuf")])?.0.protocol,
            OtlpProtocol::HttpProtobuf
        );
        assert_eq!(
            pushed(&[(PROTOCOL, " http/json ")])?.1.protocol,
            OtlpProtocol::HttpJson
        );
        for name in [PROTOCOL, TRACES.protocol, METRICS.protocol] {
            for value in ["grpc", "http/proto", "HTTP/JSON", "json"] {
                assert_eq!(
                    refusal(&[(name, value)])?.0,
                    name,
                    "`{value}` must fail startup: Neo speaks OTLP/HTTP only, in protobuf or JSON"
                );
            }
        }
        Ok(())
    }

    /// An operator following a vendor's docs sets only `OTEL_EXPORTER_OTLP_TRACES_HEADERS`: its
    /// spans must carry that credential, not go out without it.
    #[test]
    fn a_signals_own_variable_wins_over_the_general_one() -> Result<(), Box<dyn Error>> {
        let (traces, metrics) = pushed(&[
            (HEADERS, "x-general=yes"),
            (TRACES.headers, "authorization=Bearer%20traces-token"),
            (PROTOCOL, "http/json"),
            (METRICS.protocol, "http/protobuf"),
            (TIMEOUT, "3000"),
            (TRACES.timeout, "700"),
        ])?;
        assert_eq!(
            header_names(&traces),
            ["authorization"],
            "a signal's headers replace the general ones, as the specification has it"
        );
        assert_eq!(traces.protocol, OtlpProtocol::HttpJson);
        assert_eq!(traces.timeout, Duration::from_millis(700));
        assert_eq!(header_names(&metrics), ["x-general"]);
        assert_eq!(metrics.protocol, OtlpProtocol::HttpProtobuf);
        assert_eq!(metrics.timeout, Duration::from_secs(3));
        Ok(())
    }

    #[test]
    fn headers_are_percent_decoded_and_debug_shows_only_their_names() -> Result<(), Box<dyn Error>>
    {
        let config = read(&[
            (ENDPOINT, "http://collector:4318"),
            (
                HEADERS,
                " Authorization = Bearer%20s3cr3t-token , x-tenant=neo,,api-key=raw space ok,",
            ),
        ])?;
        let traces = config.traces.as_ref().ok_or("traces must push")?;
        let headers: Vec<(String, String)> = traces
            .headers
            .iter()
            .map(|(name, value)| -> Result<_, Box<dyn Error>> {
                Ok((name.to_string(), value.to_str()?.to_owned()))
            })
            .collect::<Result<_, _>>()?;
        assert_eq!(
            headers,
            vec![
                ("authorization".to_owned(), "Bearer s3cr3t-token".to_owned()),
                ("x-tenant".to_owned(), "neo".to_owned()),
                ("api-key".to_owned(), "raw space ok".to_owned()),
            ],
            "names are trimmed and lower-cased, values percent-decoded, empty entries skipped"
        );
        assert!(
            traces.headers.iter().all(|(_, value)| value.is_sensitive()),
            "every value is marked sensitive, so http's and reqwest's Debug hide it too"
        );
        let shown = format!("{config:?}");
        assert!(
            !shown.contains("s3cr3t") && !shown.contains("raw space"),
            "Debug must never print a header value: {shown}"
        );
        assert!(
            shown.contains("authorization"),
            "Debug still says which headers are set: {shown}"
        );
        Ok(())
    }

    #[test]
    fn a_bad_header_fails_naming_the_variable_and_never_its_value() -> Result<(), Box<dyn Error>> {
        for (value, why) in [
            ("Authorization: Bearer s3cr3t", "an entry with no `=`"),
            ("=s3cr3t", "an empty name"),
            ("bad name=s3cr3t", "an invalid name"),
            ("x-key=s3cr3t%FF", "a value that is not UTF-8 once decoded"),
            ("x-key=s3cr3t%0Abreak", "a value with a line break"),
        ] {
            for name in [HEADERS, TRACES.headers, METRICS.headers] {
                let (named, message) = refusal(&[(name, value)])?;
                assert_eq!(named, name, "{why} must fail startup naming {name}");
                assert!(
                    !message.contains("s3cr3t"),
                    "{why}: the error must not echo a header value: {message}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn an_endpoints_debug_hides_its_query() -> Result<(), Box<dyn Error>> {
        let config = read(&[
            (
                ENDPOINT,
                "https://collector.example:4318/otlp?api_key=s3cret",
            ),
            (
                METRICS.endpoint,
                "http+unix://%2Frun%2Fotel.sock/v1/metrics?token=s3cret",
            ),
        ])?;
        let shown = format!("{config:?}");
        assert!(
            !shown.contains("s3cret"),
            "a query can carry a token, so Debug never prints one: {shown}"
        );
        assert!(
            shown.contains("https://collector.example:4318/otlp/v1/traces?<redacted>"),
            "scheme, host, port and path stay readable: {shown}"
        );
        assert!(
            shown.contains("http+unix://%2Frun%2Fotel.sock/v1/metrics?<redacted>"),
            "{shown}"
        );
        assert_eq!(
            config
                .traces
                .ok_or("traces must push")?
                .endpoint
                .exporter_url(),
            "https://collector.example:4318/otlp/v1/traces?api_key=s3cret",
            "the request itself still carries the query"
        );
        Ok(())
    }

    #[test]
    fn the_service_name_and_resource_attributes_are_read() -> Result<(), Box<dyn Error>> {
        let config = read(&[
            (SERVICE_NAME, " neo-cell-7 "),
            (
                RESOURCE_ATTRIBUTES,
                "deployment.environment=prod, host.name = cell%2D7,service.name=override",
            ),
        ])?;
        assert_eq!(config.service_name, "neo-cell-7");
        let attributes: Vec<(&str, &str)> = config
            .resource_attributes
            .iter()
            .map(|attribute| (attribute.key.as_str(), attribute.value.as_str()))
            .collect();
        assert_eq!(
            attributes,
            vec![
                ("deployment.environment", "prod"),
                ("host.name", "cell-7"),
                ("service.name", "override"),
            ],
            "entries keep their order (the last one wins in the resource) and values are decoded"
        );
        for value in ["deployment.environment", "=prod", "a=%FF"] {
            assert_eq!(
                refusal(&[(RESOURCE_ATTRIBUTES, value)])?.0,
                RESOURCE_ATTRIBUTES,
                "`{value}` must fail startup naming the variable"
            );
        }
        Ok(())
    }

    #[test]
    fn every_sampler_reads_and_ratio_samplers_take_their_argument() -> Result<(), Box<dyn Error>> {
        let half = Ratio::parse("0.5")?;
        for (name, arg, sampler) in [
            (
                "parentbased_always_on",
                None,
                SamplerChoice::ParentBasedAlwaysOn,
            ),
            (
                "parentbased_always_off",
                None,
                SamplerChoice::ParentBasedAlwaysOff,
            ),
            (
                "parentbased_traceidratio",
                Some("0.5"),
                SamplerChoice::ParentBasedTraceIdRatio(half),
            ),
            ("always_on", Some("not used"), SamplerChoice::AlwaysOn),
            ("always_off", None, SamplerChoice::AlwaysOff),
            (
                "traceidratio",
                Some("0.5"),
                SamplerChoice::TraceIdRatio(half),
            ),
            (
                "traceidratio",
                None,
                SamplerChoice::TraceIdRatio(Ratio::ALL),
            ),
        ] {
            let mut vars = vec![(SAMPLER, name)];
            vars.extend(arg.map(|arg| (SAMPLER_ARG, arg)));
            assert_eq!(read(&vars)?.sampler, sampler, "{vars:?}");
        }
        assert!((half.get() - 0.5).abs() < f64::EPSILON);
        assert_eq!(
            refusal(&[(SAMPLER, "jaeger_remote")])?.0,
            SAMPLER,
            "a sampler Neo does not build must fail startup"
        );
        for arg in ["1.5", "-0.1", "half", "NaN"] {
            assert_eq!(
                refusal(&[(SAMPLER, "traceidratio"), (SAMPLER_ARG, arg)])?.0,
                SAMPLER_ARG,
                "the ratio `{arg}` must fail startup naming the argument variable"
            );
        }
        Ok(())
    }

    #[test]
    fn durations_are_whole_milliseconds_of_at_least_one() -> Result<(), Box<dyn Error>> {
        let config = read(&[(METRIC_INTERVAL, "500"), (BATCH_DELAY, "250")])?;
        assert_eq!(config.metric_interval, Duration::from_millis(500));
        assert_eq!(config.batch_delay, Duration::from_millis(250));
        assert_eq!(
            pushed(&[(TIMEOUT, "2000")])?.0.timeout,
            Duration::from_secs(2)
        );
        for name in [
            METRIC_INTERVAL,
            BATCH_DELAY,
            TIMEOUT,
            TRACES.timeout,
            METRICS.timeout,
        ] {
            for value in ["0", "-1", "1.5", "5s", "soon", "18446744073709551616"] {
                assert_eq!(
                    refusal(&[(name, value)])?.0,
                    name,
                    "`{value}` must fail startup naming {name}"
                );
            }
        }
        Ok(())
    }
}
