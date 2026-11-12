//! One op's wire as description data: the TypeScript of its input and result, printed by ts-rs
//! from the handler's own types, and every named type they reach.

use std::any::{self, TypeId};
use std::collections::HashSet;
use std::path::PathBuf;

use ts_rs::{Config, TS, TypeVisitor};

use super::{ADDRESSING, OptionNesting, Upstream};

/// One op's wire, as `tests/wire_generated.rs` lays it out into the prelude's generated files
/// and as the host's tests read it. Public like `OpSpec`, and like it only data: nothing decides
/// policy from it.
#[derive(Clone, Debug)]
pub struct OpWire {
    /// The TypeScript reference to the input type (`ClickInput`), from ts-rs's `TS::name`.
    pub input: String,
    /// The reference to the result type: `Clicked | null` for `Option<Clicked>`, `null` for
    /// `Nothing`.
    pub output: String,
    /// The named types `input` and `output` mention, which a file using them imports.
    pub references: Vec<String>,
    /// The script's option keys, read from serde's own field list: the fields of the input's
    /// `options` struct, or of a flat input itself minus Neo's addressing and the row's `adds`.
    /// Empty for an op without options.
    pub option_keys: Vec<&'static str>,
    /// Where the input carries those options. A flat input's upstream check compares the
    /// nesting too.
    pub option_nesting: OptionNesting,
    /// Where the op sits in Playwright's protocol, for the generated upstream check.
    pub upstream: Upstream,
    /// Every named type the input and result reach, each once, in the order the walk met them.
    pub types: Vec<WireType>,
}

/// One named TypeScript type, as ts-rs prints its file.
#[derive(Clone, Debug)]
pub struct WireType {
    /// Its TypeScript name (`ClickInput`; `PageHandle` for a type that says `#[ts(rename)]`).
    pub name: String,
    /// ts-rs's file for it, relative to the generated `types/` folder (`ClickInput.ts`).
    pub file: PathBuf,
    /// The file as ts-rs prints it: imports of the other named types, the Rust doc comment, the
    /// declaration. An error only for a type ts-rs cannot print.
    pub text: Result<String, String>,
    /// The Rust type, for messages (`neo_playwright::action::ClickInput`).
    pub rust: &'static str,
    /// The Rust type's identity (of its generic-free form). The generator refuses two
    /// identities under one name: ts-rs would keep the first file and drop the second silently.
    pub id: TypeId,
}

impl OpWire {
    /// The wire of an op whose handler takes `I` and answers `O`, with its options as
    /// `options!` declared them: `fields` are the options struct's, or a flat input's own.
    pub(crate) fn of<I: TS + 'static, O: TS + 'static>(
        option_nesting: OptionNesting,
        fields: Vec<&'static str>,
        upstream: Upstream,
    ) -> Self {
        let option_keys: Vec<&'static str> = match option_nesting {
            OptionNesting::Flat => {
                let adds = match upstream {
                    Upstream::Command(command) => command.adds,
                    Upstream::NeoOnly { .. } | Upstream::Replaces { .. } => &[],
                };
                fields
                    .into_iter()
                    .filter(|field| !ADDRESSING.contains(field))
                    .filter(|field| !adds.iter().any(|(added, _)| added == field))
                    .collect()
            }
            OptionNesting::Nested | OptionNesting::None => fields,
        };
        let cfg = ts_config();
        let mut references = References::default();
        references.add::<I>(&cfg);
        references.add::<O>(&cfg);
        let mut types = NamedTypes::new(&cfg);
        types.visit::<I>();
        types.visit::<O>();
        Self {
            input: I::name(&cfg),
            output: O::name(&cfg),
            references: references.names,
            option_keys,
            option_nesting,
            upstream,
            types: types.found,
        }
    }
}

/// How every wire type prints. 64-bit integers are `number`: the ids and counters on the wire
/// stay far below 2^53 and the prelude reads them as numbers, where ts-rs's default would print
/// `bigint`. Imports carry no extension, as the prelude's bundler resolves them.
fn ts_config() -> Config {
    Config::new().with_large_int("number")
}

/// The named types a reference mentions: a named type itself, or for a wrapper (`Option<T>`,
/// `Vec<T>`, a tuple) the named types among its type arguments, which is what its name prints.
#[derive(Default)]
struct References {
    names: Vec<String>,
}

impl References {
    fn add<T: TS + 'static + ?Sized>(&mut self, cfg: &Config) {
        struct Generics<'w> {
            references: &'w mut References,
            cfg: &'w Config,
        }
        impl TypeVisitor for Generics<'_> {
            fn visit<T: TS + 'static + ?Sized>(&mut self) {
                self.references.add::<T>(self.cfg);
            }
        }
        if T::output_path().is_some() {
            let name = T::ident(cfg);
            if !self.names.contains(&name) {
                self.names.push(name);
            }
        }
        // A wrapper's name is its arguments' names; a generic named type (`Page<Brand>`) prints
        // its arguments too.
        T::visit_generics(&mut Generics {
            references: self,
            cfg,
        });
    }
}

/// Every named type reachable from the visited types, through fields and type arguments, as
/// ts-rs's own `export_all` walks them.
struct NamedTypes<'c> {
    cfg: &'c Config,
    found: Vec<WireType>,
    /// Types already walked, so a recursive type (a tree node and its children) ends.
    walked: HashSet<TypeId>,
    /// Declarations already recorded: every instance of one generic type shares its file.
    declared: HashSet<TypeId>,
}

impl<'c> NamedTypes<'c> {
    fn new(cfg: &'c Config) -> Self {
        Self {
            cfg,
            found: Vec::new(),
            walked: HashSet::new(),
            declared: HashSet::new(),
        }
    }
}

impl TypeVisitor for NamedTypes<'_> {
    fn visit<T: TS + 'static + ?Sized>(&mut self) {
        if !self.walked.insert(TypeId::of::<T>()) {
            return;
        }
        if let Some(file) = T::output_path() {
            let id = TypeId::of::<T::WithoutGenerics>();
            if self.declared.insert(id) {
                self.found.push(WireType {
                    name: T::ident(self.cfg),
                    file,
                    text: T::export_to_string(self.cfg).map_err(|error| error.to_string()),
                    rust: any::type_name::<T::WithoutGenerics>(),
                    id,
                });
            }
        }
        T::visit_generics(self);
        T::visit_dependencies(self);
    }
}

#[cfg(test)]
mod tests {
    use browseros_core::{FrameId, TabId, TargetId};
    use serde::{Deserialize, Serialize};
    use ts_rs::TS;

    use super::OpWire;
    use crate::wire::{Nothing, OptionNesting, command};

    /// A result in the shape of `locator.click`'s, with a 64-bit counter inside.
    #[derive(Serialize, TS)]
    struct Clicked {
        navigation: Cut,
    }

    /// The frame-event cut a navigation reached.
    #[derive(Serialize, TS)]
    struct Cut {
        seq: u64,
        tab: TabId,
    }

    /// A tree like the aria snapshot's: recursive through an untagged enum.
    #[derive(Serialize, TS)]
    struct Node {
        children: Vec<Child>,
    }

    #[derive(Serialize, TS)]
    #[serde(untagged)]
    #[expect(dead_code, reason = "only printed as TypeScript, never built")]
    enum Child {
        Text(String),
        Node(Box<Node>),
    }

    #[derive(Deserialize, TS)]
    #[serde(deny_unknown_fields, rename_all = "camelCase")]
    #[ts(optional_fields)]
    #[expect(dead_code, reason = "only printed as TypeScript, never decoded")]
    struct ClickOptions {
        click_count: Option<u32>,
    }

    #[derive(Deserialize, TS)]
    #[serde(deny_unknown_fields)]
    #[ts(rename = "ClickArgs")]
    #[expect(dead_code, reason = "only printed as TypeScript, never decoded")]
    struct ClickInput {
        options: ClickOptions,
        frame: Option<FrameId>,
        target: TargetId,
    }

    fn text_of<'w>(wire: &'w OpWire, name: &str) -> &'w str {
        wire.types
            .iter()
            .find(|named| named.name == name)
            .and_then(|named| named.text.as_deref().ok())
            .unwrap_or_default()
    }

    #[test]
    fn tests_that_an_option_result_is_a_reference_and_its_types_have_files() {
        let wire = OpWire::of::<ClickInput, Option<Clicked>>(
            OptionNesting::Nested,
            vec!["clickCount"],
            command("ElementHandle.click").into(),
        );
        assert_eq!(wire.input, "ClickArgs", "#[ts(rename)] names the type");
        assert_eq!(
            wire.output, "Clicked | null",
            "Option<T> is a reference to T, never a declaration of its own"
        );
        assert_eq!(wire.references, ["ClickArgs", "Clicked"]);
        let mut names: Vec<_> = wire.types.iter().map(|named| named.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "ClickArgs",
                "ClickOptions",
                "Clicked",
                "Cut",
                "FrameId",
                "TabId",
                "TargetId"
            ],
            "every named type reached through fields and Option, once each"
        );
        let input = text_of(&wire, "ClickArgs");
        assert!(
            input.contains(r#"import type { ClickOptions } from "./ClickOptions";"#)
                && input.contains(r#"import type { FrameId } from "./FrameId";"#),
            "each file imports the named types it uses: {input}"
        );
        assert!(
            input.contains("frame: FrameId | null"),
            "an input's optional field outside `options` stays `T | null`: {input}"
        );
        assert!(
            text_of(&wire, "ClickOptions").contains("clickCount?: number"),
            "an options struct prints `key?: T` in wire spelling"
        );
    }

    #[test]
    fn tests_that_64_bit_integers_and_the_core_ids_print_as_numbers_and_strings() {
        let wire = OpWire::of::<ClickInput, Clicked>(
            OptionNesting::None,
            Vec::new(),
            command("Frame.click").into(),
        );
        assert!(
            text_of(&wire, "Cut").contains("seq: number"),
            "u64 prints as number, not bigint: {}",
            text_of(&wire, "Cut")
        );
        assert!(text_of(&wire, "TabId").contains("export type TabId = number;"));
        assert!(text_of(&wire, "FrameId").contains("export type FrameId = string;"));
        assert!(text_of(&wire, "TargetId").contains("export type TargetId = string;"));
        assert!(
            text_of(&wire, "Clicked").contains("A result in the shape of"),
            "the Rust doc comment becomes the JSDoc"
        );
    }

    #[test]
    fn tests_that_a_flat_input_keys_its_own_fields_but_addressing_and_adds() {
        let flat = OpWire::of::<ClickInput, Nothing>(
            OptionNesting::Flat,
            vec![
                "target",
                "strict",
                "publication",
                "force",
                "clickCount",
                "frame",
                "page",
            ],
            command("Frame.goto")
                .adds(&[("publication", "the prelude stamps it")])
                .into(),
        );
        assert_eq!(
            flat.option_keys,
            ["force", "clickCount"],
            "Neo's addressing and the fields the prelude computes are never the script's options"
        );
        assert_eq!(flat.option_nesting, OptionNesting::Flat);
        let nested = OpWire::of::<ClickInput, Nothing>(
            OptionNesting::Nested,
            vec!["force", "target"],
            command("Frame.click").into(),
        );
        assert_eq!(
            nested.option_keys,
            ["force", "target"],
            "an options struct's fields are all keys, whatever their names"
        );
    }

    #[test]
    fn tests_that_nothing_is_null_and_a_recursive_tree_ends() {
        let wire = OpWire::of::<ClickInput, Nothing>(
            OptionNesting::None,
            Vec::new(),
            command("Frame.hover").into(),
        );
        assert_eq!(wire.output, "null", "`Nothing` answers JSON null");
        let tree = OpWire::of::<ClickInput, Vec<Node>>(
            OptionNesting::None,
            Vec::new(),
            command("Frame.tree").into(),
        );
        assert_eq!(tree.output, "Array<Node>");
        assert_eq!(tree.references, ["ClickArgs", "Node"]);
        assert!(
            text_of(&tree, "Child").contains("export type Child = string | Node;"),
            "an untagged enum is the union of its arms: {}",
            text_of(&tree, "Child")
        );
    }
}
