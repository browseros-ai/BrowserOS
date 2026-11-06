//! The traits generated code implements, and the metadata it carries.
//!
//! The translator emits one `impl` of these traits per command, and `neo-runtime` is generic
//! over them, so the CDP and dispatcher plumbing lives in `neo-runtime` while this crate holds
//! types only. Every trait here is sealed: a generated params type is the only thing that can
//! implement one, so `neo-runtime` cannot serve a command or make a page call that the spec
//! does not declare.

use std::error::Error;
use std::fmt;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

/// The seal. `Sealed` is public so it can bound the public traits, but this module is private,
/// so no other crate can name it, let alone implement it. Generated code implements it next to
/// each call trait.
mod sealed {
    pub trait Sealed {}
}

pub(crate) use sealed::Sealed;

/// A Playwright protocol command Neo serves, implemented by the command's params type.
///
/// The params type is the command: `FrameGotoParams: Command<Result = FrameGotoResult>`. A
/// handler encodes its answer with [`Command::reply`], which accepts only `Self::Result`, so
/// answering `goto` with `title`'s result does not compile. Other crates cannot implement it:
///
/// ```compile_fail,E0277
/// use neo_runtime_spec::{Command, MethodMeta};
///
/// #[derive(serde::Serialize, serde::Deserialize)]
/// struct ForgedParams {}
///
/// impl Command for ForgedParams {
///     type Result = ();
///     const META: &'static MethodMeta = &MethodMeta {
///         interface: "Frame",
///         method: "click",
///         title: Some("Click"),
///         subtitle: None,
///         group: None,
///         render_params: &[],
///         internal: false,
///         flags: &[],
///     };
/// }
/// ```
pub trait Command: Serialize + DeserializeOwned + Sealed {
    /// The command's result as Playwright's wire defines it.
    type Result: Serialize;
    /// Playwright's metainfo for the command, for tracing.
    const META: &'static MethodMeta;

    /// Encodes `result` as the wire `result` of a reply to this call.
    ///
    /// # Errors
    ///
    /// Only when `Serialize` fails, which generated types never do.
    fn reply(&self, result: &Self::Result) -> Result<Value, serde_json::Error> {
        serde_json::to_value(result)
    }
}

/// A page call: one function in `neo-runtime-injected`'s glue table, implemented by the call's
/// params type.
///
/// `neo-runtime` calls it with CDP `Runtime.callFunctionOn` on the glue object of one world of
/// one document, as `this[NAME](params)` with `awaitPromise: true`. [`ValueCall`] and
/// [`ElementCall`] say how the result crosses back; each page call implements exactly one.
/// Failures that end the Playwright call (a strict mode violation, an invalid selector, an
/// element of the wrong kind) are thrown in the page, as in Playwright, and arrive as CDP
/// `exceptionDetails`, so results never carry errors. Other crates cannot implement it:
///
/// ```compile_fail,E0277
/// use neo_runtime_spec::InjectedCall;
///
/// #[derive(serde::Serialize)]
/// struct ForgedParams {}
///
/// impl InjectedCall for ForgedParams {
///     const NAME: &'static str = "click";
/// }
/// ```
pub trait InjectedCall: Serialize + Sealed {
    /// The glue table key, as written in `spec/injected.yml`.
    const NAME: &'static str;
}

/// A page call whose result crosses by value: call with `returnByValue: true` and decode
/// [`ValueCall::Result`] from the returned JSON.
///
/// Decoding is the only validation of what the page returns, and every page result type
/// rejects unknown fields, so a page bundle built from another spec fails on its first call.
pub trait ValueCall: InjectedCall {
    /// What the glue function returns.
    type Result: DeserializeOwned;
}

/// A page call whose result is one DOM element that stays in the page: call with
/// `returnByValue: false`. The result is the returned remote object as a [`RemoteElement`], or
/// nothing when the glue function returned `undefined`.
pub trait ElementCall: InjectedCall {}

/// A DOM element left in the page by an [`ElementCall`], named by its CDP
/// `Runtime.RemoteObjectId` in the world that returned it. Whoever holds it releases it with
/// `Runtime.releaseObject`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RemoteElement(String);

impl RemoteElement {
    /// Wraps the `objectId` of a returned remote object whose subtype is `node`.
    #[must_use]
    pub fn new(object_id: String) -> Self {
        Self(object_id)
    }

    /// The element's CDP `Runtime.RemoteObjectId`.
    #[must_use]
    pub fn object_id(&self) -> &str {
        &self.0
    }
}

/// Playwright's metainfo for a served command (`packages/isomorphic/protocolMetainfo.ts`),
/// checked by the translator with Playwright's own generator rules. Tracing uses it for titles,
/// and to tell writes (`input`) from reads (`group: getter`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MethodMeta {
    /// The Playwright interface that declares the command, such as `Frame`.
    pub interface: &'static str,
    /// The wire method name, such as `textContent`.
    pub method: &'static str,
    /// The trace title, with `{param}` placeholders. `None` only for internal commands.
    pub title: Option<&'static str>,
    /// The trace subtitle, such as `{selector}`.
    pub subtitle: Option<&'static str>,
    /// The command's group, when the spec names one.
    pub group: Option<MethodGroup>,
    /// Params shown in the trace title, each as `[key=]path[:selector]`.
    pub render_params: &'static [&'static str],
    /// An internal command is not reported as an API call.
    pub internal: bool,
    /// Tracing and debugging flags, in Playwright's order.
    pub flags: &'static [MethodFlag],
}

impl MethodMeta {
    /// Whether the command carries `flag`.
    #[must_use]
    pub fn has(&self, flag: MethodFlag) -> bool {
        self.flags.contains(&flag)
    }
}

/// Playwright's command `group`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MethodGroup {
    /// `getter`: the command reads page state.
    Getter,
    Configuration,
    Route,
    Default,
}

/// Playwright's command `flags`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MethodFlag {
    /// `slowMo`: slowed down by the `slowMo` launch option.
    SlowMo,
    /// `snapshot`: tracing takes snapshots around the call.
    Snapshot,
    /// `pause`: the debugger may pause before the call.
    Pause,
    /// `input`: the command writes to the page, so its delivery is tracked as not sent, done,
    /// or unknown.
    Input,
    /// `isAutoWaiting`: the command waits for its target to be actionable.
    IsAutoWaiting,
}

/// Why a served call could not be decoded.
#[derive(Debug)]
pub enum CallDecodeError {
    /// Neo does not serve this method on this interface.
    UnknownMethod {
        /// The interface the call targeted.
        interface: &'static str,
        /// The method name from the wire.
        method: String,
    },
    /// The params do not fit the command's params type.
    InvalidParams {
        /// The command whose params failed to decode.
        meta: &'static MethodMeta,
        /// What serde reported.
        source: serde_json::Error,
    },
}

impl CallDecodeError {
    /// Decodes the params of command `C`, for the generated `*Call::decode`. Absent params
    /// decode as `{}`: Playwright's client omits them for a command that declares none, and a
    /// command that does declare params then fails on its first required field.
    pub(crate) fn params<C: Command>(params: Option<Value>) -> Result<C, Self> {
        let params = params.unwrap_or_else(|| Value::Object(Map::new()));
        serde_json::from_value(params).map_err(|source| Self::InvalidParams {
            meta: C::META,
            source,
        })
    }
}

impl fmt::Display for CallDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownMethod { interface, method } => {
                write!(formatter, "Neo does not serve {interface}.{method}")
            }
            Self::InvalidParams { meta, source } => {
                write!(
                    formatter,
                    "invalid params for {}.{}: {source}",
                    meta.interface, meta.method
                )
            }
        }
    }
}

impl Error for CallDecodeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::UnknownMethod { .. } => None,
            Self::InvalidParams { source, .. } => Some(source),
        }
    }
}
