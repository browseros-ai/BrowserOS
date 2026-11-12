//! Renders the page-side probe contract from the Rust types that encode requests and decode
//! replies, plus value payloads decoded by individual families. This renderer is separate
//! from the op-family generator: only `probe-gen` writes
//! `src/probe/generated.ts`, while the stale-file test compares without modifying sources.

use std::any::{self, TypeId};
use std::collections::{BTreeMap, HashSet};
use std::error::Error;

use ts_rs::{Config, TS, TypeVisitor};

use crate::action::Checkable;
use crate::read::{FrameOrigin, Located, PinnedMatch};

use super::probe_reply::ProbeReply;
use super::probe_request::{BootOptions, ProbeRequest};

const HEADER: &str = "\
// Generated from Rust's probe request/reply types and family-decoded value payloads by ts-rs.
// Do not edit: change Rust's probe types, then run `bun run gen:probe` in neo-playwright-js.
// PROTOCOL stays hand-bumped in protocol.ts; generating types never changes the wire version.
";

/// The complete probe declarations as one module. Keeping this pure lets the generation
/// command and the compare-only test use exactly the same rendering path.
pub fn render() -> Result<String, Box<dyn Error>> {
    let mut declarations = Declarations::new();
    declarations.visit::<ProbeRequest>();
    declarations.visit::<ProbeReply>();
    declarations.visit::<BootOptions<'static>>();
    // `value` replies carry JSON that the owning runner decodes after the envelope. Those
    // payloads are explicit roots because they are absent from ProbeReply's type graph.
    declarations.visit::<Checkable>();
    declarations.visit::<FrameOrigin>();
    declarations.visit::<Located>();
    declarations.visit::<PinnedMatch>();
    declarations.finish()
}

/// ts-rs owns every declaration; this visitor only gathers their dependency closure into
/// one module. Type identities reject name collisions instead of silently keeping one body.
struct Declarations {
    cfg: Config,
    visited: HashSet<TypeId>,
    named: BTreeMap<String, Declaration>,
    collisions: Vec<String>,
}

struct Declaration {
    identity: TypeId,
    rust: &'static str,
    text: String,
}

impl Declarations {
    fn new() -> Self {
        Self {
            // JavaScript's seqs, pins and counters are numbers, just as in the op wire.
            cfg: Config::new().with_large_int("number"),
            visited: HashSet::new(),
            named: BTreeMap::new(),
            collisions: Vec::new(),
        }
    }

    fn finish(self) -> Result<String, Box<dyn Error>> {
        if !self.collisions.is_empty() {
            return Err(self.collisions.join("\n").into());
        }
        let mut output = format!("{HEADER}\n");
        for declaration in self.named.into_values() {
            output.push_str(&declaration.text);
            output.push('\n');
        }
        Ok(output)
    }
}

impl TypeVisitor for Declarations {
    fn visit<T: TS + 'static + ?Sized>(&mut self) {
        if !self.visited.insert(TypeId::of::<T>()) {
            return;
        }
        // Primitive/container references have no standalone declaration. Their named type
        // arguments are still visited below (Option<Resolution>, Vec<SnapshotIframe>, ...).
        if T::output_path().is_some() {
            let name = T::ident(&self.cfg);
            let identity = TypeId::of::<T::WithoutGenerics>();
            let rust = any::type_name::<T::WithoutGenerics>();
            if let Some(previous) = self.named.get(&name) {
                if previous.identity != identity {
                    self.collisions.push(format!(
                        "{} and {rust} both generate `{name}`; rename one with #[ts(rename)]",
                        previous.rust
                    ));
                }
            } else {
                let docs = T::docs().unwrap_or_default();
                let text = format!("{docs}export {}\n", T::decl(&self.cfg));
                self.named.insert(
                    name,
                    Declaration {
                        identity,
                        rust,
                        text,
                    },
                );
            }
        }
        T::visit_generics(self);
        T::visit_dependencies(self);
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use serde::Deserialize;
    use ts_rs::{TS, TypeVisitor};

    use super::Declarations;

    #[derive(TS)]
    #[ts(rename = "Collision")]
    struct First;

    #[derive(TS)]
    #[ts(rename = "Collision")]
    struct Second;

    #[test]
    fn two_rust_types_cannot_silently_share_a_declaration() -> Result<(), Box<dyn Error>> {
        let mut declarations = Declarations::new();
        declarations.visit::<First>();
        declarations.visit::<Second>();
        let collision = declarations
            .finish()
            .err()
            .ok_or("two Rust types under one TypeScript name must be refused")?;
        assert!(
            collision.to_string().contains("Collision"),
            "collision diagnostics must identify the conflicting TypeScript name: {collision}"
        );
        Ok(())
    }

    #[derive(Deserialize, TS)]
    #[serde(rename_all = "camelCase")]
    #[expect(dead_code, reason = "rendered, never instantiated")]
    struct Fields {
        #[serde(default)]
        #[ts(optional)]
        stale_ref: Option<String>,
        // Required nullable fields (frameBox.origin/hit) must stay distinct from omission.
        origin: Option<f64>,
    }

    #[test]
    fn optional_fields_and_required_nulls_keep_their_serde_spelling() -> Result<(), Box<dyn Error>>
    {
        let mut declarations = Declarations::new();
        declarations.visit::<Fields>();
        let text = declarations.finish()?;
        assert!(text.contains("staleRef?: string"), "{text}");
        assert!(text.contains("origin: number | null"), "{text}");
        Ok(())
    }
}
