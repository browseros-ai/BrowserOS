//! A stand-in for sdk-skeleton's `dispatch` and `run` modules, so `adapter/playwright.rs`
//! compiles against the skeleton's documented contract before apps/claw-sdk exists.
//!
//! Every item the adapter uses is spelled and typed as sdk-skeleton/design.md (Interfaces,
//! rev 3) documents it. Three adjustments, each marked where it is:
//!
//! - Rev 3's `pub` is `pub(crate)` here: this is a test crate, where `unreachable_pub` reads
//!   every `pub` as a mistake.
//! - Rev 3 names `OpName`, `ErrorCode` and `Schema` but not how to build or read them. The
//!   four conversions marked "assumed" are the only names the adapter relies on that rev 3 does
//!   not spell out; when the skeleton merges, the adapter follows whatever it chose.
//! - Items marked "test harness" exist only so tests can build and inspect these types.

/// `apps/claw-sdk/src/dispatch.rs` and the files under it.
pub(crate) mod dispatch {
    use std::borrow::Cow;
    use std::error::Error;
    use std::fmt;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, PoisonError};

    use browseros_trace::{Effect, ErrorOwner, RedactionPolicy};
    use futures_util::future::BoxFuture;
    use serde_json::Value;

    use super::run::RunCtx;

    // dispatch/method_spec.rs

    /// A method's name.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct OpName(Arc<str>);

    /// Who may call a method.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum Visibility {
        Script,
        #[expect(
            dead_code,
            reason = "rev 3 declares it; every Playwright op is a script method"
        )]
        Internal,
    }

    /// Which input field is the target page.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum Target {
        None,
        Field(&'static str),
    }

    /// How the `run` tool's description lists a method.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum Listing {
        #[expect(
            dead_code,
            reason = "rev 3 declares it; Playwright ops are listed as one line"
        )]
        Full,
        Summarized,
    }

    /// How long a method's op may live.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum Lifetime {
        Operation,
        Background,
    }

    /// A method's input or output JSON Schema. Rev 3 names the type only.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct Schema(Value);

    /// One method, as the registry keeps it.
    pub(crate) struct MethodSpec {
        pub(crate) name: OpName,
        pub(crate) doc: Cow<'static, str>,
        pub(crate) input: Schema,
        pub(crate) output: Schema,
        pub(crate) effect: Effect,
        pub(crate) redaction: Arc<RedactionPolicy>,
        pub(crate) visibility: Visibility,
        pub(crate) target: Target,
        pub(crate) listing: Listing,
        pub(crate) lifetime: Lifetime,
        pub(crate) handler: Arc<dyn Handler>,
    }

    /// A method's body.
    pub(crate) trait Handler: Send + Sync {
        fn call<'a>(
            &'a self,
            op: &'a OpCtx,
            input: Value,
        ) -> BoxFuture<'a, Result<Value, LeafError>>;
    }

    /// A handler's failure.
    pub(crate) struct LeafError {
        pub(crate) error: OperationError,
        pub(crate) answer: Answer,
    }

    /// Whether the handler knows what happened.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum Answer {
        Definite,
        Ambiguous,
    }

    // dispatch/registry.rs

    /// The method list while it is being built. Test harness: rev 3 does not say how the app
    /// creates one, so it is `Default` here.
    #[derive(Default)]
    pub(crate) struct RegistryBuilder {
        methods: Vec<MethodSpec>,
    }

    impl RegistryBuilder {
        pub(crate) fn define(&mut self, spec: MethodSpec) -> Result<(), RegistryError> {
            if self.methods.iter().any(|method| method.name == spec.name) {
                return Err(RegistryError(format!("{} is defined twice", spec.name.0)));
            }
            self.methods.push(spec);
            Ok(())
        }

        pub(crate) fn build(self) -> Registry {
            Registry {
                methods: self.methods,
            }
        }
    }

    /// The frozen method list.
    pub(crate) struct Registry {
        methods: Vec<MethodSpec>,
    }

    impl Registry {
        pub(crate) fn method(&self, name: &str) -> Option<&MethodSpec> {
            self.methods.iter().find(|method| &*method.name.0 == name)
        }

        pub(crate) fn script_methods(&self) -> impl Iterator<Item = &MethodSpec> {
            self.methods
                .iter()
                .filter(|method| method.visibility == Visibility::Script)
        }
    }

    /// A method the registry refused. Rev 3 names the type only.
    #[derive(Debug)]
    pub(crate) struct RegistryError(String);

    impl fmt::Display for RegistryError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl Error for RegistryError {}

    // dispatch/operation_error.rs

    /// A stable error code.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct ErrorCode(Cow<'static, str>);

    /// Whether a failure is a timeout.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum TimedOut {
        No,
        Yes,
    }

    /// A failure with its owner.
    #[derive(Clone, Debug)]
    pub(crate) struct OperationError {
        pub(crate) owner: ErrorOwner,
        pub(crate) code: ErrorCode,
        pub(crate) message: String,
        pub(crate) host_code: Option<String>,
        pub(crate) timed_out: TimedOut,
    }

    // dispatch/op_ctx.rs

    /// One op's context. Only the methods the adapter calls are reproduced.
    pub(crate) struct OpCtx {
        run: Arc<RunCtx>,
        refusal: Option<OperationError>,
        sent: AtomicBool,
        sensitive: AtomicBool,
        notes: Mutex<Vec<String>>,
    }

    impl OpCtx {
        pub(crate) fn run(&self) -> &Arc<RunCtx> {
            &self.run
        }

        pub(crate) fn recheck(&self) -> Result<(), OperationError> {
            self.refusal.clone().map_or(Ok(()), Err)
        }

        pub(crate) fn mark_sent(&self) {
            self.sent.store(true, Ordering::SeqCst);
        }

        pub(crate) fn mark_sensitive_target(&self) {
            self.sensitive.store(true, Ordering::SeqCst);
        }

        pub(crate) fn note(&self, line: impl Into<String>) {
            self.notes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(line.into());
        }
    }

    // Assumed: how the adapter builds an `OpName`, a `Schema` and an `ErrorCode`, and reads a
    // refusal's code.

    impl From<&str> for OpName {
        fn from(name: &str) -> Self {
            Self(Arc::from(name))
        }
    }

    impl From<Value> for Schema {
        fn from(schema: Value) -> Self {
            Self(schema)
        }
    }

    impl From<String> for ErrorCode {
        fn from(code: String) -> Self {
            Self(Cow::Owned(code))
        }
    }

    impl ErrorCode {
        pub(crate) fn as_str(&self) -> &str {
            &self.0
        }
    }

    // Test harness.

    impl Schema {
        pub(crate) fn json(&self) -> &Value {
            &self.0
        }
    }

    impl OpCtx {
        /// A context whose `recheck` passes.
        pub(crate) fn new(run: Arc<RunCtx>) -> Self {
            Self {
                run,
                refusal: None,
                sent: AtomicBool::new(false),
                sensitive: AtomicBool::new(false),
                notes: Mutex::default(),
            }
        }

        /// A context whose `recheck` refuses with `error`.
        pub(crate) fn refusing(run: Arc<RunCtx>, error: OperationError) -> Self {
            Self {
                refusal: Some(error),
                ..Self::new(run)
            }
        }

        pub(crate) fn was_sent(&self) -> bool {
            self.sent.load(Ordering::SeqCst)
        }

        pub(crate) fn target_was_sensitive(&self) -> bool {
            self.sensitive.load(Ordering::SeqCst)
        }

        pub(crate) fn notes(&self) -> Vec<String> {
            self.notes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }
    }
}

/// `apps/claw-sdk/src/run.rs` and the files under it.
pub(crate) mod run {
    use std::any::{Any, TypeId};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, PoisonError};

    use browseros_core::BrowserView;

    /// One run: its browser view and its per-feature slots. Only the methods the adapter calls
    /// are reproduced.
    pub(crate) struct RunCtx {
        view: BrowserView,
        slots: Mutex<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
    }

    impl RunCtx {
        pub(crate) fn view(&self) -> &BrowserView {
            &self.view
        }

        /// The run's one `T`, created on first use.
        pub(crate) fn slot<T: Default + Send + Sync + 'static>(&self) -> Arc<T> {
            let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
            let slot = slots
                .entry(TypeId::of::<T>())
                .or_insert_with(|| Arc::new(T::default()));
            // Keyed by `T`'s type id, so the downcast cannot miss.
            Arc::clone(slot)
                .downcast::<T>()
                .unwrap_or_else(|_other| Arc::new(T::default()))
        }
    }

    // Test harness.

    impl RunCtx {
        pub(crate) fn new(view: BrowserView) -> Arc<Self> {
            Arc::new(Self {
                view,
                slots: Mutex::default(),
            })
        }
    }
}
