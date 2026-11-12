//! The frozen list of built-ins, and the type erasure that lets the door run any of them.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use serde_json::Value;
use serde_path_to_error::Error as PathError;

use super::op_ctx::OpCore;
use super::{ErrorCode, LeafError, Listing, Method, MethodSpec, OpCtx, OperationError, PageTarget};

/// The methods this process serves. Built once at startup, then shared read-only by every run:
/// agent code can never add, replace or wrap one (skeleton D7 rule 1).
pub(crate) struct Registry {
    methods: HashMap<&'static str, Registered>,
}

/// Collects methods at startup. A duplicate name fails the start,
/// so a placeholder and its real replacement can never both be registered.
#[derive(Default)]
pub(crate) struct RegistryBuilder {
    methods: HashMap<&'static str, Registered>,
}

/// A registration error, caught by the first test that builds the full set.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RegistryError {
    /// Two methods claim one name.
    #[error("method {0} is registered twice")]
    Duplicate(&'static str),
}

/// One registered method: its spec and its erased run step.
pub(super) struct Registered {
    spec: MethodSpec,
    method: Arc<dyn ErasedMethod>,
}

impl RegistryBuilder {
    /// Adds one method.
    pub(crate) fn define<M: Method>(&mut self, method: M) -> Result<(), RegistryError> {
        let spec = method.spec();
        match self.methods.entry(spec.name) {
            Entry::Occupied(_) => Err(RegistryError::Duplicate(spec.name)),
            Entry::Vacant(slot) => {
                slot.insert(Registered {
                    spec,
                    method: Arc::new(Erased(Arc::new(method))),
                });
                Ok(())
            }
        }
    }

    /// Freezes the list.
    #[must_use]
    pub(crate) fn build(self) -> Registry {
        Registry {
            methods: self.methods,
        }
    }
}

impl Registry {
    /// The specs the `run` description shows, sorted by name so the description is stable.
    #[must_use]
    pub(crate) fn listed(&self) -> Vec<MethodSpec> {
        let mut listed: Vec<MethodSpec> = self
            .methods
            .values()
            .map(|registered| registered.spec)
            .filter(|spec| match spec.listing {
                Listing::Own(_) | Listing::Grouped(_) => true,
                Listing::Hidden => false,
            })
            .collect();
        listed.sort_by_key(|spec| spec.name);
        listed
    }

    /// The method a call names. Private to the door: nothing else can reach a run step.
    pub(super) fn get(&self, name: &str) -> Option<&Registered> {
        self.methods.get(name)
    }
}

impl Registered {
    pub(super) fn spec(&self) -> MethodSpec {
        self.spec
    }

    /// Decodes `input` into the method's typed input, once. A failure names the field path,
    /// which is what a model needs to fix its call.
    pub(super) fn decode(&self, input: &Value) -> Result<Box<dyn DecodedCall>, OperationError> {
        if let Some(reason) = unencodable(input) {
            return Err(OperationError::script(
                ErrorCode::INVALID_INPUT,
                format_args!("{}: {reason}", self.spec.name),
            ));
        }
        self.method.decode(self.spec.name, input)
    }
}

/// A call whose input decoded: it knows its target and can run once.
pub(super) trait DecodedCall: Send {
    /// The page the input names, when the method has one.
    fn target(&self) -> Option<PageTarget>;

    /// Runs the method with an `OpCtx` minted for it.
    fn run(self: Box<Self>, op: OpCore) -> BoxFuture<'static, Result<Value, LeafError>>;
}

trait ErasedMethod: Send + Sync {
    fn decode(
        &self,
        name: &'static str,
        input: &Value,
    ) -> Result<Box<dyn DecodedCall>, OperationError>;
}

struct Erased<M>(Arc<M>);

struct Decoded<M: Method> {
    name: &'static str,
    method: Arc<M>,
    input: M::Input,
}

impl<M: Method> ErasedMethod for Erased<M> {
    fn decode(
        &self,
        name: &'static str,
        input: &Value,
    ) -> Result<Box<dyn DecodedCall>, OperationError> {
        match serde_path_to_error::deserialize::<_, M::Input>(input) {
            Ok(input) => Ok(Box::new(Decoded {
                name,
                method: Arc::clone(&self.0),
                input,
            })),
            Err(error) => Err(invalid_input(name, &error)),
        }
    }
}

impl<M: Method> DecodedCall for Decoded<M> {
    fn target(&self) -> Option<PageTarget> {
        M::target(&self.input).cloned()
    }

    fn run(self: Box<Self>, op: OpCore) -> BoxFuture<'static, Result<Value, LeafError>> {
        let Decoded {
            name,
            method,
            input,
        } = *self;
        Box::pin(async move {
            // The only place an `OpCtx<M>` is minted, so only the door runs `M::call`.
            let op = OpCtx::<M>::mint(op);
            let output = method.call(&op, input).await?;
            // The method returned, so whatever it did is done; only its report is lost.
            serde_json::to_value(output).map_err(|error| {
                LeafError::definite(OperationError::neo(
                    ErrorCode::INTERNAL_ERROR,
                    format_args!("{name} returned a value that is not JSON: {error}"),
                ))
            })
        })
    }
}

/// The reason the script's input never became JSON. The bootstrap's codec copies plain data
/// only, and sends `{"$neo":"invalid","reason":…}` in place of input it cannot encode (a
/// function, a cycle, a class instance), as the bridge does for text that is not JSON. The call
/// still reaches the door, so it is recorded and refused like any bad input, with the codec's
/// reason instead of a decoder error about a `$neo` field the script never wrote.
fn unencodable(input: &Value) -> Option<&str> {
    let brand = input.as_object()?;
    if brand.get("$neo")?.as_str()? != "invalid" {
        return None;
    }
    brand.get("reason")?.as_str()
}

/// `script / invalid_input`, naming the field: `page.goto: invalid type: integer, expected a
/// string (at `url`)`.
fn invalid_input(name: &str, error: &PathError<serde_json::Error>) -> OperationError {
    let path = error.path().to_string();
    let message = if path == "." {
        format!("{name}: {}", error.inner())
    } else {
        format!("{name}: {} (at `{path}`)", error.inner())
    };
    OperationError::script(ErrorCode::INVALID_INPUT, message)
}

#[cfg(test)]
mod tests {
    use std::future::{self, Future};

    use neo_trace::Effect;
    use serde_json::Value;

    use super::{RegistryBuilder, RegistryError};
    use crate::dispatch::{LeafError, Listing, Method, MethodSpec, OpCtx, PageTarget};

    /// A method that is only its spec.
    struct Named(MethodSpec);

    impl Method for Named {
        type Input = Value;
        type Output = Value;

        fn spec(&self) -> MethodSpec {
            self.0
        }

        fn target(_input: &Value) -> Option<&PageTarget> {
            None
        }

        fn call(
            &self,
            _op: &OpCtx<Self>,
            _input: Value,
        ) -> impl Future<Output = Result<Value, LeafError>> + Send {
            future::ready(Ok(Value::Null))
        }
    }

    fn named(name: &'static str, listing: Listing) -> Named {
        Named(MethodSpec {
            listing,
            ..MethodSpec::new(name, Effect::Read)
        })
    }

    #[test]
    fn the_description_lists_methods_by_name_and_skips_hidden_ones() -> Result<(), RegistryError> {
        let mut builder = RegistryBuilder::default();
        builder.define(named("page.title", Listing::Grouped("pages")))?;
        builder.define(named(
            "neo.solveCaptcha",
            Listing::Own("neo.solveCaptcha(page)"),
        ))?;
        builder.define(named("neo.internal", Listing::Hidden))?;
        let names: Vec<&str> = builder
            .build()
            .listed()
            .iter()
            .map(|spec| spec.name)
            .collect();
        assert_eq!(
            names,
            ["neo.solveCaptcha", "page.title"],
            "sorted, so the description is stable"
        );
        Ok(())
    }

    #[test]
    fn a_name_is_registered_once() -> Result<(), RegistryError> {
        let mut builder = RegistryBuilder::default();
        builder.define(named("page.title", Listing::Hidden))?;
        assert!(
            matches!(
                builder.define(named("page.title", Listing::Hidden)),
                Err(RegistryError::Duplicate("page.title"))
            ),
            "a placeholder and its replacement can never both be served"
        );
        Ok(())
    }
}
