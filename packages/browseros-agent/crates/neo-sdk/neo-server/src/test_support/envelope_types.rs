//! The run envelope's TypeScript, as ts-rs prints it from the Rust types that serialize it.
//!
//! The roots are the two texts a realm reads, the run's init ([`RunInit`]) and each op's reply
//! ([`Envelope`]); the walk adds every named type they reach through fields and type arguments
//! (page facts, observations, error fields, the core and trace ids), each once.
//! `tests/envelope_generated.rs` lays them out as the prelude's `generated/envelope.ts`.

use std::any::{self, TypeId};
use std::collections::HashSet;

use ts_rs::{Config, TS, TypeVisitor};

use crate::runtime::{Envelope, RunInit};

/// One named TypeScript type of the envelope.
#[derive(Debug, Clone)]
pub struct EnvelopeType {
    /// Its TypeScript name (`PageFact`; `WorkerInfo` for a type that says `#[ts(rename)]`).
    pub name: String,
    /// Its doc comment and `export type` declaration, as ts-rs writes them in a file of its own,
    /// without that file's imports: every type it names is in the same list.
    pub declaration: String,
    /// The Rust type, for messages.
    pub rust: &'static str,
    /// The Rust type's identity: two under one name must be refused, since a file can hold only
    /// one declaration per name.
    pub id: TypeId,
}

/// How the envelope prints. 64-bit integers are `number`, as neo-playwright prints its wire
/// (`src/wire/op_wire.rs`): the serials, counters and tab ids stay far below 2^53, and the
/// prelude reads them as numbers.
fn ts_config() -> Config {
    Config::new().with_large_int("number")
}

/// Every named type of the envelope, roots first, then each type's dependencies in the order
/// its fields name them.
#[must_use]
pub fn envelope_types() -> Vec<EnvelopeType> {
    let cfg = ts_config();
    let mut walk = NamedTypes {
        cfg: &cfg,
        found: Vec::new(),
        walked: HashSet::new(),
    };
    walk.visit::<Envelope<'static>>();
    walk.visit::<RunInit<'static>>();
    walk.found
}

/// The walk ts-rs's own `export_all` makes, keeping each named type's declaration.
struct NamedTypes<'c> {
    cfg: &'c Config,
    found: Vec<EnvelopeType>,
    /// Types already walked, by their generic-free identity.
    walked: HashSet<TypeId>,
}

impl TypeVisitor for NamedTypes<'_> {
    fn visit<T: TS + 'static + ?Sized>(&mut self) {
        if !self.walked.insert(TypeId::of::<T>()) {
            return;
        }
        // Wrappers (`Option`, `Vec`, references) have no file of their own; their arguments do.
        if T::output_path().is_some() {
            let mut declaration = T::docs().unwrap_or_default();
            declaration.push_str("export ");
            declaration.push_str(&T::decl(self.cfg));
            self.found.push(EnvelopeType {
                name: T::ident(self.cfg),
                declaration,
                rust: any::type_name::<T::WithoutGenerics>(),
                id: TypeId::of::<T::WithoutGenerics>(),
            });
        }
        T::visit_generics(self);
        T::visit_dependencies(self);
    }
}
