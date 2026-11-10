//! The traits generated code implements, and the metadata it carries.
//!
//! The translator emits one `impl` of these traits per command, and `neo-runtime` is generic
//! over them, so the CDP and dispatcher plumbing lives in `neo-runtime` while this crate holds
//! types only. Every trait here is sealed: only generated types can implement one, so
//! `neo-runtime` cannot serve a command, make a page call or name a kind of page object that the
//! spec does not declare.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

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
/// use neo_playwright_protocol::{Command, MethodMeta};
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
///     const VOID_RESULT: bool = true;
/// }
/// ```
pub trait Command: Serialize + DeserializeOwned + Sealed {
    /// The command's result as Playwright's wire defines it.
    type Result: Serialize;
    /// Playwright's metainfo for the command, for tracing.
    const META: &'static MethodMeta;
    /// Playwright's command declares no `returns`. Its server then replies `{id}` with no
    /// `result` (its result validator, `tOptional(tObject({}))`, yields `undefined`), and its
    /// client resolves `undefined`. Encoding the empty result struct as `{}` instead would
    /// resolve `{}` in the agent's script, so the reply asks the spec rather than guessing.
    const VOID_RESULT: bool;

    /// Encodes `result` as the wire `result` of a reply to this call, or `None` for a void
    /// command, whose reply carries no `result` key.
    ///
    /// # Errors
    ///
    /// Only when `Serialize` fails, which generated types never do.
    fn reply(&self, result: &Self::Result) -> Result<Option<Value>, serde_json::Error> {
        if Self::VOID_RESULT {
            return Ok(None);
        }
        serde_json::to_value(result).map(Some)
    }
}

/// A page call: one function in `neo-playwright-injected`'s glue table, implemented by the call's
/// params type.
///
/// The calling convention, which `neo-playwright-injected/src/call.ts` implements and
/// `neo-runtime` drives: CDP `Runtime.callFunctionOn` with `objectId` set to the glue object of
/// one world of one document, `functionDeclaration` set to the built `dist/call.js`
/// (`this[name](params, ...handles)`), `arguments` set to
/// `[{value: NAME}, {value: <params JSON>}, ...handles().map(|id| {objectId: id})]`,
/// `awaitPromise: true`, and `returnByValue` set to [`PageResult::BY_VALUE`]. Serializing the
/// params writes only their JSON fields: a handle field is `#[serde(skip)]` and listed by
/// [`InjectedCall::handles`] instead.
///
/// Failures that end the Playwright call (a strict mode violation, an invalid selector, an
/// element of the wrong kind) are thrown in the page, as in Playwright, and arrive as CDP
/// `exceptionDetails`, so results never carry errors. Other crates cannot implement it:
///
/// ```compile_fail,E0277
/// use neo_playwright_protocol::injected::InjectedTitleResult;
/// use neo_playwright_protocol::{HandleId, InjectedCall};
///
/// #[derive(serde::Serialize)]
/// struct ForgedParams {}
///
/// impl InjectedCall for ForgedParams {
///     const NAME: &'static str = "click";
///     type Result = InjectedTitleResult;
///
///     fn handles(&self) -> Vec<&HandleId> {
///         Vec::new()
///     }
/// }
/// ```
pub trait InjectedCall: Serialize + Sealed {
    /// The glue table key, as written in `spec/injected.yml`.
    const NAME: &'static str;
    /// What the glue function returns.
    type Result: PageResult;

    /// The call's handle arguments in spec order, passed after the params; empty for a call
    /// that takes none.
    fn handles(&self) -> Vec<&HandleId>;
}

/// How a page call's result crosses back. Implemented by every generated result type.
///
/// A result that holds no handle is called with `returnByValue: true` and decoded from the
/// returned JSON with unknown fields rejected, so a page bundle built from another spec fails on
/// its first call. A result that holds a handle is called with `returnByValue: false` into the
/// caller's object group and read with one `Runtime.getProperties`, whose own properties
/// arrive split into [`Properties`]. Releasing that object group frees the result and every
/// handle read from it.
pub trait PageResult: Sized + Sealed {
    /// Whether the result crosses by value.
    const BY_VALUE: bool;

    /// Decodes a by-value result.
    ///
    /// # Errors
    ///
    /// When the JSON does not match the spec, or this result holds a handle.
    fn from_value(value: Value) -> Result<Self, ResultDecodeError>;

    /// Decodes a result that holds a handle from its own properties.
    ///
    /// # Errors
    ///
    /// When a property is missing, unknown or of the wrong kind, or this result crosses by
    /// value.
    fn from_properties(properties: Properties) -> Result<Self, ResultDecodeError>;
}

/// The own properties of a returned page object, as `Runtime.getProperties` lists them: each a
/// primitive (`RemoteObject.value`) or a page object. `neo-runtime` leaves out properties whose
/// value is `undefined`, and `__proto__`.
#[derive(Debug, Default)]
pub struct Properties {
    /// Primitive properties, by name.
    pub values: Map<String, Value>,
    /// Object-valued properties, by name: the `objectId` and the `RemoteObject.subtype`, which a
    /// [`Handle`]'s decode checks against [`HandleKind::SUBTYPE`].
    pub handles: HashMap<String, (HandleId, Option<String>)>,
}

/// One property taken out of [`Properties`], for a generated decode.
pub(crate) enum Property {
    Value(Value),
    Object(HandleId, Option<String>),
}

/// How a generated decode reads one property. The error is the detail of a
/// [`ResultDecodeError`].
type DecodeProperty<T> = fn(Property) -> Result<T, String>;

impl Properties {
    /// Takes the property `key`, which the result requires.
    pub(crate) fn required<T>(
        &mut self,
        call: &'static str,
        key: &str,
        decode: DecodeProperty<T>,
    ) -> Result<T, ResultDecodeError> {
        let property = self
            .take(key)
            .ok_or_else(|| ResultDecodeError::new(call, format!("no `{key}`")))?;
        decode(property)
            .map_err(|detail| ResultDecodeError::new(call, format!("`{key}` as {detail}")))
    }

    /// Takes the property `key`, which may be absent. A `null` reads as absent, as it does for
    /// an `Option` field decoded from JSON.
    pub(crate) fn optional<T>(
        &mut self,
        call: &'static str,
        key: &str,
        decode: DecodeProperty<T>,
    ) -> Result<Option<T>, ResultDecodeError> {
        let absent =
            !self.handles.contains_key(key) && self.values.get(key).is_none_or(Value::is_null);
        if absent {
            self.values.remove(key);
            return Ok(None);
        }
        self.required(call, key, decode).map(Some)
    }

    /// Rejects whatever the decode did not take, as `deny_unknown_fields` does for JSON.
    pub(crate) fn finish(self, call: &'static str) -> Result<(), ResultDecodeError> {
        let unknown = self.values.keys().chain(self.handles.keys()).min();
        match unknown {
            None => Ok(()),
            Some(key) => Err(ResultDecodeError::new(
                call,
                format!("the unknown property `{key}`"),
            )),
        }
    }

    fn take(&mut self, key: &str) -> Option<Property> {
        if let Some((id, subtype)) = self.handles.remove(key) {
            return Some(Property::Object(id, subtype));
        }
        self.values.remove(key).map(Property::Value)
    }
}

/// Decodes a primitive property, for a generated decode.
pub(crate) fn decode_property<T: DeserializeOwned>(property: Property) -> Result<T, String> {
    match property {
        Property::Value(value) => serde_json::from_value(value).map_err(|error| error.to_string()),
        Property::Object(..) => Err("a page object, not a value".to_owned()),
    }
}

/// Decodes a by-value result, for a generated [`PageResult::from_value`].
pub(crate) fn decode_json<T: DeserializeOwned>(
    call: &'static str,
    value: Value,
) -> Result<T, ResultDecodeError> {
    serde_json::from_value(value).map_err(|error| ResultDecodeError::new(call, error.to_string()))
}

/// The error for a result that arrived the other way than its spec says.
pub(crate) fn wrong_crossing(call: &'static str, by_value: bool) -> ResultDecodeError {
    let detail = if by_value {
        "properties, but its result crosses by value"
    } else {
        "a value, but its result holds a page object"
    };
    ResultDecodeError::new(call, detail.to_owned())
}

/// A CDP `Runtime.RemoteObjectId`: a page object held for Rust in one object group of one CDP
/// session. It names nothing in another session, and nothing once its group is released.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HandleId(String);

impl HandleId {
    /// Wraps an `objectId` that CDP returned.
    #[must_use]
    pub fn new(object_id: String) -> Self {
        Self(object_id)
    }

    /// The `objectId` to pass back to CDP.
    #[must_use]
    pub fn object_id(&self) -> &str {
        &self.0
    }
}

/// What kind of page object a handle names. Implemented by [`Element`] and by one generated
/// uninhabited marker per `type: handle` definition, so a handle of one kind does not compile
/// where the spec asks for another.
pub trait HandleKind: Sealed {
    /// The spec name, for decode errors.
    const NAME: &'static str;
    /// The `RemoteObject.subtype` a handle of this kind has, checked at decode: `node` for an
    /// element, none for a plain JavaScript object.
    const SUBTYPE: Option<&'static str>;
}

/// A DOM element in the page: the spec dialect's built-in handle kind, `Element`.
#[derive(Debug)]
pub enum Element {}

impl Sealed for Element {}

impl HandleKind for Element {
    const NAME: &'static str = "Element";
    const SUBTYPE: Option<&'static str> = Some("node");
}

/// A page object of kind `K` that stays in the page while Rust holds its id.
///
/// Cloning copies the name, not the object: every copy dies when the object group holding the
/// object is released, so the holder of the group decides how long every handle lives.
///
/// The kind is part of the type, so a handle of one kind cannot reach a call that takes another:
///
/// ```compile_fail,E0308
/// use neo_playwright_protocol::injected::{HitTargetInterceptor, InjectedGetElementBorderWidthParams};
/// use neo_playwright_protocol::{Handle, HandleId};
///
/// let interceptor = Handle::<HitTargetInterceptor>::new(HandleId::new("-1.1.1".into()), None);
/// let params = InjectedGetElementBorderWidthParams {
///     element: interceptor.unwrap(),
/// };
/// ```
pub struct Handle<K: HandleKind> {
    id: HandleId,
    kind: PhantomData<K>,
}

impl<K: HandleKind> Handle<K> {
    /// Wraps the id of a remote object whose subtype is `K::SUBTYPE`; `None` for any other
    /// subtype.
    #[must_use]
    pub fn new(id: HandleId, subtype: Option<&str>) -> Option<Self> {
        (subtype == K::SUBTYPE).then_some(Self {
            id,
            kind: PhantomData,
        })
    }

    /// The object id to pass back to CDP.
    #[must_use]
    pub fn id(&self) -> &HandleId {
        &self.id
    }

    /// Decodes a handle property, for a generated decode.
    pub(crate) fn from_property(property: Property) -> Result<Self, String> {
        match property {
            Property::Object(id, subtype) => {
                let wrong = format!(
                    "a page object of subtype {subtype:?}, not a `{}` (subtype {:?})",
                    K::NAME,
                    K::SUBTYPE
                );
                Self::new(id, subtype.as_deref()).ok_or(wrong)
            }
            Property::Value(value) => Err(format!("the value {value}, not a page object")),
        }
    }
}

// By hand, so that none of these needs `K` to implement it: `K` is an uninhabited marker.
impl<K: HandleKind> Clone for Handle<K> {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            kind: PhantomData,
        }
    }
}

impl<K: HandleKind> fmt::Debug for Handle<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Handle<{}>({:?})", K::NAME, self.id.0)
    }
}

impl<K: HandleKind> PartialEq for Handle<K> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<K: HandleKind> Eq for Handle<K> {}

impl<K: HandleKind> Hash for Handle<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

/// Why a page result did not match the spec. The page bundle and this crate disagree, which the
/// spec-hash checks on both sides should have made impossible.
#[derive(Debug)]
pub struct ResultDecodeError {
    /// The glue table key of the call.
    pub call: &'static str,
    /// What came back instead.
    pub detail: String,
}

impl ResultDecodeError {
    fn new(call: &'static str, detail: String) -> Self {
        Self { call, detail }
    }
}

impl fmt::Display for ResultDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "page call {} returned {}",
            self.call, self.detail
        )
    }
}

impl Error for ResultDecodeError {}

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
