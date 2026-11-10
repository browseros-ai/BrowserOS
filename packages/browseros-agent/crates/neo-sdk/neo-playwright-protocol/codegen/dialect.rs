//! Playwright's protocol dialect: one flat namespace of definitions across files, read with
//! Playwright's own loader and generator rules (`utils/protocol_spec.js` and
//! `utils/generate_channels.js` at the commit in `spec/playwright/UPSTREAM`).
//!
//! This module knows the dialect and nothing about what Neo serves; `contract.rs` applies Neo's
//! rules on top. It parses every file with a closed grammar, so a Playwright upgrade that adds a
//! construct fails here instead of being mistranslated. Parsing is by position, never by
//! sniffing for a `type` key: `type` and `title` are metadata in one place and property names in
//! another (`frame.yml` has a command and a parameter named `type`).
//!
//! It is stricter than Playwright where the frozen spec never needs the slack: an undefined type
//! name (Playwright emits an unchecked `tType`), `?` on array items (Playwright ignores it), an
//! unknown key or flag (Playwright ignores them), and an enum literal that is neither a string
//! nor `null`.
//!
//! Files of the [`Grammar::Extended`] grammar may also use Neo's two extensions:
//!
//! - `type: union` with `members`: a value that is exactly one of its members. The wire carries
//!   the member's own JSON with no tag, so members must differ in JSON kind.
//! - Handles: page objects that stay in the page while Rust holds their CDP object ids. The
//!   bare name `Element` is the built-in handle type, and a `type: handle` definition names
//!   another. Playwright's files never see either. Where a handle may appear is Neo's rule, so
//!   `contract.rs` checks it.

use std::collections::HashMap;
use std::fmt;

use crate::error::{At, SpecError, did_you_mean};
use crate::yaml::{Node, Value};

/// The name an extended-grammar file uses for the built-in handle type, a DOM element.
pub const BUILT_IN_ELEMENT: &str = "Element";

/// One parsed spec file in the dialect, and the grammar it is read with.
pub struct DialectFile {
    /// Relative to `spec/`, such as `playwright/frame.yml`.
    pub path: String,
    pub root: Node,
    pub grammar: Grammar,
}

/// Which constructs a file may use, and which names it sees.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Grammar {
    /// Playwright's dialect as Playwright's generator accepts it. These files see only each
    /// other's names, as in Playwright's own namespace.
    Playwright,
    /// Playwright's dialect plus Neo's two extensions. These files see every name.
    Extended,
}

/// Every definition of every dialect file, in load order: files in the order given (Playwright
/// sorts its files by name), keys in file order. A name is defined once across all files.
pub struct Protocol {
    defs: Vec<Def>,
    by_name: HashMap<String, DefId>,
}

/// A definition's position in load order, which is also its identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DefId(usize);

/// A top-level definition.
pub struct Def {
    pub name: String,
    /// The spec file it came from, relative to `spec/`.
    pub file: String,
    pub grammar: Grammar,
    /// The comment above the definition, plus the one above an object's `properties`. A file's
    /// first definition has no doc: the comment above it is the file's header (the license, in
    /// Playwright's files).
    pub doc: String,
    pub kind: DefKind,
}

pub enum DefKind {
    Object(Vec<Prop>),
    Enum(Vec<String>),
    /// Spliced wherever a `$mixin` key names it; its own body is parsed only to check it.
    Mixin,
    Interface(Interface),
    /// Extension: a value that is exactly one of its members.
    Union(Vec<Member>),
    /// Extension: a page object other than an element, named so that the glue and Rust agree on
    /// which call produces it and which consumes it.
    Handle,
}

pub struct Interface {
    pub extends: Option<DefId>,
    /// `None` when the interface declares none (Playwright then uses its ancestor's).
    pub initializer: Option<Vec<Prop>>,
    pub initializer_doc: String,
    pub commands: Vec<Command>,
    /// Event names. Their parameters are parsed to check them but not kept: nothing Neo
    /// generates needs an event yet.
    pub events: Vec<String>,
}

pub struct Command {
    pub name: String,
    pub doc: String,
    /// `None` when the command declares no `parameters`: Playwright's scheme is then
    /// `tOptional(tObject({}))`, so the wire may omit `params` entirely.
    pub params: Option<Vec<Prop>>,
    pub returns: Returns,
    pub error_details: Option<Vec<Prop>>,
    pub meta: Meta,
}

pub enum Returns {
    /// No `returns` key: the result is empty (TypeScript `void` in Playwright).
    Nothing,
    Props(Vec<Prop>),
}

/// A command's metainfo, already checked with Playwright's generator rules.
#[derive(Clone)]
pub struct Meta {
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub group: Option<Group>,
    pub render_params: Vec<String>,
    pub internal: bool,
    /// In Playwright's order, which is [`Flag`]'s.
    pub flags: Vec<Flag>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Getter,
    Configuration,
    Route,
    Default,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Flag {
    SlowMo,
    Snapshot,
    Pause,
    Input,
    IsAutoWaiting,
}

/// A property of a property map, with every `$mixin` key already spliced in.
pub struct Prop {
    /// The JSON key.
    pub name: String,
    pub doc: String,
    pub ty: Type,
    pub presence: Presence,
}

/// A trailing `?` on a property's type: the key may be absent. Playwright never sends `null`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    Required,
    Optional,
}

/// A member of a union (extension).
pub struct Member {
    pub name: String,
    pub doc: String,
    pub ty: Type,
}

/// A type expression. Optionality belongs to the property, not the type.
pub enum Type {
    String,
    Boolean,
    Int,
    Float,
    /// Base64 text on the wire.
    Binary,
    /// Any JSON value.
    Json,
    /// Always absent.
    Undefined,
    /// `Channel`: a reference to an object of any interface.
    AnyChannel,
    /// A reference to an object of this interface; `{guid}` on Playwright's wire.
    Channel(DefId),
    /// A named object, enum or union.
    Named(DefId),
    Array(Box<Type>),
    /// An inline enum.
    Enum(Vec<String>),
    /// An inline object.
    Object(Vec<Prop>),
    /// An inline union (extension).
    Union(Vec<Member>),
    /// The built-in handle type `Element`: a DOM element that stays in the page (extension).
    Element,
    /// A `type: handle` definition (extension).
    Handle(DefId),
}

/// Where [`Protocol::command`] found a command.
pub enum Lookup<'protocol> {
    Declared(&'protocol Command),
    /// Declared by this ancestor, reached through `extends`.
    Inherited(DefId),
    Missing,
}

impl Def {
    pub fn as_interface(&self) -> Option<&Interface> {
        match &self.kind {
            DefKind::Interface(interface) => Some(interface),
            _ => None,
        }
    }
}

impl Protocol {
    /// Loads dialect files into one namespace.
    pub fn load(files: &[DialectFile]) -> Result<Protocol, SpecError> {
        let entries = index(files)?;
        let mut by_name = HashMap::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            by_name.insert(entry.name.to_owned(), DefId(index));
        }
        let reader = Reader {
            entries: &entries,
            by_name: &by_name,
        };
        let mut defs = Vec::with_capacity(entries.len());
        for index in 0..entries.len() {
            defs.push(reader.definition(DefId(index))?);
        }
        Ok(Protocol { defs, by_name })
    }

    /// The definition named `name`, if any.
    pub fn get(&self, name: &str) -> Option<DefId> {
        self.by_name.get(name).copied()
    }

    pub fn def(&self, id: DefId) -> &Def {
        &self.defs[id.0]
    }

    /// Every definition, in load order.
    pub fn defs(&self) -> impl Iterator<Item = (DefId, &Def)> {
        self.defs
            .iter()
            .enumerate()
            .map(|(index, def)| (DefId(index), def))
    }

    /// The command `name` of `interface`, looking through `extends` as Playwright's dispatcher
    /// does.
    pub fn command(&self, interface: DefId, name: &str) -> Lookup<'_> {
        let mut current = Some(interface);
        while let Some(id) = current {
            let Some(declaring) = self.def(id).as_interface() else {
                return Lookup::Missing;
            };
            if let Some(command) = declaring
                .commands
                .iter()
                .find(|command| command.name == name)
            {
                return if id == interface {
                    Lookup::Declared(command)
                } else {
                    Lookup::Inherited(id)
                };
            }
            current = declaring.extends;
        }
        Lookup::Missing
    }
}

/// A definition's kind, read in the first pass so that references resolve in the second.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Object,
    Enum,
    Mixin,
    Interface,
    Union,
    Handle,
}

/// A top-level definition as found by the first pass.
struct Entry<'source> {
    name: &'source str,
    file: &'source DialectFile,
    node: &'source Node,
    body: &'source [(String, Node)],
    kind: Kind,
    /// The first key of its file, whose comment is the file's header.
    first: bool,
}

/// Pass 1, Playwright's loader (`protocol_spec.js`): each file is a map of definitions, and a
/// name defined in an earlier file is an error.
fn index(files: &[DialectFile]) -> Result<Vec<Entry<'_>>, SpecError> {
    let mut entries = Vec::new();
    let mut seen: HashMap<&str, &str> = HashMap::new();
    for file in files {
        let at = At::file(&file.path);
        let Value::Map(definitions) = &file.root.value else {
            return Err(at.error("must be a YAML map of definitions".to_owned()));
        };
        for (position, (name, node)) in definitions.iter().enumerate() {
            let at = at.key(name);
            if let Some(other) = seen.insert(name, &file.path) {
                return Err(at.error(format!("is also defined in spec/{other}")));
            }
            let Value::Map(body) = &node.value else {
                return Err(at.error(format!("must be a map, not {}", node.value.describe())));
            };
            let kind = definition_kind(body, file.grammar, &at)?;
            entries.push(Entry {
                name,
                file,
                node,
                body,
                kind,
                first: position == 0,
            });
        }
    }
    Ok(entries)
}

fn definition_kind(body: &[(String, Node)], grammar: Grammar, at: &At) -> Result<Kind, SpecError> {
    let kind = match find(body, "type").map(|node| &node.value) {
        Some(Value::Str(kind)) => kind.as_str(),
        _ => {
            return Err(at.error("needs `type: object`, `enum`, `mixin` or `interface`".to_owned()));
        }
    };
    match kind {
        "object" => Ok(Kind::Object),
        "enum" => Ok(Kind::Enum),
        "mixin" => Ok(Kind::Mixin),
        "interface" => Ok(Kind::Interface),
        "union" => extension(grammar, "`type: union`", at).map(|()| Kind::Union),
        "handle" => extension(grammar, "`type: handle`", at).map(|()| Kind::Handle),
        other => Err(at.key("type").error(format!(
            "unknown definition type \"{other}\"; expected object, enum, mixin or interface"
        ))),
    }
}

/// Pass 2: definition bodies, now that every name resolves.
struct Reader<'source> {
    entries: &'source [Entry<'source>],
    by_name: &'source HashMap<String, DefId>,
}

/// What a node being read may use and see.
struct Scope {
    grammar: Grammar,
    /// Mixins being spliced, to reject one that includes itself.
    splicing: Vec<DefId>,
}

impl Reader<'_> {
    fn definition(&self, id: DefId) -> Result<Def, SpecError> {
        let entry = &self.entries[id.0];
        let at = At::file(&entry.file.path).key(entry.name);
        let scope = Scope {
            grammar: entry.file.grammar,
            splicing: Vec::new(),
        };
        let mut doc = if entry.first {
            String::new()
        } else {
            entry.node.doc.clone()
        };
        let kind = match entry.kind {
            Kind::Object | Kind::Mixin => {
                only_keys(entry.body, &["type", "properties"], &at)?;
                let properties = required(entry.body, "properties", &at)?;
                // Playwright describes some objects above their `properties` key.
                doc = join_docs(&doc, &properties.doc);
                let props = self.props(properties, &at.key("properties"), &scope)?;
                if entry.kind == Kind::Object {
                    DefKind::Object(props)
                } else {
                    DefKind::Mixin
                }
            }
            Kind::Enum => {
                only_keys(entry.body, &["type", "literals"], &at)?;
                DefKind::Enum(literals(
                    required(entry.body, "literals", &at)?,
                    &at.key("literals"),
                )?)
            }
            Kind::Union => {
                only_keys(entry.body, &["type", "members"], &at)?;
                let members = required(entry.body, "members", &at)?;
                DefKind::Union(self.members(members, &at.key("members"), &scope)?)
            }
            Kind::Interface => DefKind::Interface(self.interface(entry.body, &at, &scope)?),
            Kind::Handle => {
                only_keys(entry.body, &["type"], &at)?;
                DefKind::Handle
            }
        };
        Ok(Def {
            name: entry.name.to_owned(),
            file: entry.file.path.clone(),
            grammar: entry.file.grammar,
            doc,
            kind,
        })
    }

    fn interface(
        &self,
        body: &[(String, Node)],
        at: &At,
        scope: &Scope,
    ) -> Result<Interface, SpecError> {
        only_keys(
            body,
            &["type", "extends", "initializer", "commands", "events"],
            at,
        )?;
        let extends = match find(body, "extends") {
            None => None,
            Some(node) => Some(self.parent(node, &at.key("extends"), scope)?),
        };
        let initializer = self.optional_props(body, "initializer", at, scope)?;
        let initializer_doc =
            find(body, "initializer").map_or_else(String::new, |node| node.doc.clone());
        let mut commands = Vec::new();
        for (name, node) in map_entries(find(body, "commands"), &at.key("commands"))? {
            commands.push(self.command(name, node, &at.key("commands").key(name), scope)?);
        }
        let mut events = Vec::new();
        for (name, node) in map_entries(find(body, "events"), &at.key("events"))? {
            let at = at.key("events").key(name);
            let event = map_entries(Some(node), &at)?;
            only_keys(event, &["parameters"], &at)?;
            self.optional_props(event, "parameters", &at, scope)?;
            events.push(name.clone());
        }
        Ok(Interface {
            extends,
            initializer,
            initializer_doc,
            commands,
            events,
        })
    }

    fn parent(&self, node: &Node, at: &At, scope: &Scope) -> Result<DefId, SpecError> {
        let Value::Str(name) = &node.value else {
            return Err(at.error("must name an interface".to_owned()));
        };
        self.visible(name, scope.grammar)
            .filter(|id| self.entries[id.0].kind == Kind::Interface)
            .ok_or_else(|| at.error(format!("names \"{name}\", which is not an interface")))
    }

    fn command(
        &self,
        name: &str,
        node: &Node,
        at: &At,
        scope: &Scope,
    ) -> Result<Command, SpecError> {
        // Playwright counts a command written as `name:` with no body as `{}`.
        let body = map_entries(Some(node), at)?;
        only_keys(
            body,
            &[
                "title",
                "subtitle",
                "group",
                "renderParams",
                "internal",
                "flags",
                "parameters",
                "returns",
                "errorDetails",
            ],
            at,
        )?;
        let params = self.optional_props(body, "parameters", at, scope)?;
        let returns = self.returns(find(body, "returns"), &at.key("returns"), scope)?;
        let error_details = self.optional_props(body, "errorDetails", at, scope)?;
        // Playwright checks metainfo against the parameter keys as written, before splicing.
        let raw_params: Vec<&str> = match find(body, "parameters").map(|node| &node.value) {
            Some(Value::Map(entries)) => entries.iter().map(|(key, _)| key.as_str()).collect(),
            _ => Vec::new(),
        };
        let meta = meta(body, &raw_params, at)?;
        Ok(Command {
            name: name.to_owned(),
            doc: node.doc.clone(),
            params,
            returns,
            error_details,
            meta,
        })
    }

    fn returns(&self, node: Option<&Node>, at: &At, scope: &Scope) -> Result<Returns, SpecError> {
        let Some(node) = node else {
            return Ok(Returns::Nothing);
        };
        match &node.value {
            Value::Null => Ok(Returns::Nothing),
            Value::Map(_) => Ok(Returns::Props(self.props(node, at, scope)?)),
            other => Err(at.error(format!(
                "must be a map of properties, not {}",
                other.describe()
            ))),
        }
    }

    /// A property map under `key`; `None` when the key is absent or empty, which Playwright's
    /// generator treats alike.
    fn optional_props(
        &self,
        body: &[(String, Node)],
        key: &str,
        at: &At,
        scope: &Scope,
    ) -> Result<Option<Vec<Prop>>, SpecError> {
        match find(body, key) {
            None => Ok(None),
            Some(node) if matches!(node.value, Value::Null) => Ok(None),
            Some(node) => Ok(Some(self.props(node, &at.key(key), scope)?)),
        }
    }

    /// A property map. A key starting with `$mixin` splices the named mixin's properties in at
    /// its position, recursively (Playwright's `properties()`); every other key is a property
    /// name, whatever it spells.
    fn props(&self, node: &Node, at: &At, scope: &Scope) -> Result<Vec<Prop>, SpecError> {
        let mut props: Vec<Prop> = Vec::new();
        for (key, value) in map_entries(Some(node), at)? {
            let at = at.key(key);
            if key.starts_with("$mixin") {
                props.extend(self.splice(value, &at, scope)?);
            } else {
                let (ty, presence) = self.property_type(value, &at, scope)?;
                props.push(Prop {
                    name: key.clone(),
                    doc: value.doc.clone(),
                    ty,
                    presence,
                });
            }
        }
        for (index, prop) in props.iter().enumerate() {
            if props[..index]
                .iter()
                .any(|earlier| earlier.name == prop.name)
            {
                return Err(at.error(format!(
                    "has the property \"{}\" twice, counting spliced mixins",
                    prop.name
                )));
            }
        }
        Ok(props)
    }

    fn splice(&self, value: &Node, at: &At, scope: &Scope) -> Result<Vec<Prop>, SpecError> {
        let Value::Str(name) = &value.value else {
            return Err(at.error("must name a mixin".to_owned()));
        };
        let Some(id) = self
            .visible(name, scope.grammar)
            .filter(|id| self.entries[id.0].kind == Kind::Mixin)
        else {
            let mixins = self
                .entries
                .iter()
                .filter(|entry| entry.kind == Kind::Mixin);
            let suggestion = did_you_mean(name, mixins.map(|entry| entry.name));
            return Err(at.error(format!(
                "names \"{name}\", which is not a mixin{suggestion}"
            )));
        };
        if scope.splicing.contains(&id) {
            return Err(at.error(format!("splices the mixin \"{name}\" into itself")));
        }
        let mixin = &self.entries[id.0];
        let mixin_at = At::file(&mixin.file.path).key(mixin.name);
        let mut splicing = scope.splicing.clone();
        splicing.push(id);
        let inner = Scope {
            grammar: mixin.file.grammar,
            splicing,
        };
        let properties = required(mixin.body, "properties", &mixin_at)?;
        self.props(properties, &mixin_at.key("properties"), &inner)
    }

    /// A type in property position, where a trailing `?` makes the property optional.
    fn property_type(
        &self,
        node: &Node,
        at: &At,
        scope: &Scope,
    ) -> Result<(Type, Presence), SpecError> {
        match &node.value {
            Value::Str(text) => {
                let (name, presence) = split_optional(text);
                Ok((self.named_type(name, at, scope)?, presence))
            }
            Value::Map(entries) => self.type_node(entries, at, scope),
            other => Err(at.error(format!("must be a type, not {}", other.describe()))),
        }
    }

    /// A type in item position (array items, union members), which cannot be absent.
    fn item_type(&self, node: &Node, at: &At, scope: &Scope) -> Result<Type, SpecError> {
        match self.property_type(node, at, scope)? {
            (ty, Presence::Required) => Ok(ty),
            (_, Presence::Optional) => {
                Err(at.error("cannot end in `?`: only a property can be absent".to_owned()))
            }
        }
    }

    fn named_type(&self, name: &str, at: &At, scope: &Scope) -> Result<Type, SpecError> {
        let ty = match name {
            "string" => Type::String,
            "boolean" => Type::Boolean,
            "int" => Type::Int,
            "float" => Type::Float,
            "binary" => Type::Binary,
            "json" => Type::Json,
            "undefined" => Type::Undefined,
            "Channel" => Type::AnyChannel,
            // A Playwright file cannot name it, so `Element` resolves like any undefined name
            // there.
            BUILT_IN_ELEMENT if scope.grammar == Grammar::Extended => Type::Element,
            "number" => {
                return Err(
                    at.error("uses \"number\"; the dialect has \"int\" and \"float\"".to_owned())
                );
            }
            _ => {
                let Some(id) = self.visible(name, scope.grammar) else {
                    let visible = self.entries.iter().filter(|entry| {
                        scope.grammar == Grammar::Extended
                            || entry.file.grammar == Grammar::Playwright
                    });
                    let suggestion = did_you_mean(name, visible.map(|entry| entry.name));
                    return Err(at.error(format!("unknown type \"{name}\"{suggestion}")));
                };
                match self.entries[id.0].kind {
                    Kind::Interface => Type::Channel(id),
                    Kind::Object | Kind::Enum | Kind::Union => Type::Named(id),
                    Kind::Handle => Type::Handle(id),
                    Kind::Mixin => {
                        return Err(at.error(format!(
                            "uses the mixin \"{name}\" as a type; splice it with a `$mixin` key"
                        )));
                    }
                }
            }
        };
        Ok(ty)
    }

    /// `{type: array | enum | object | union, ...}`, where `type` may end in `?`.
    fn type_node(
        &self,
        entries: &[(String, Node)],
        at: &At,
        scope: &Scope,
    ) -> Result<(Type, Presence), SpecError> {
        let kind = match find(entries, "type").map(|node| &node.value) {
            Some(Value::Str(kind)) => kind.as_str(),
            _ => {
                return Err(
                    at.error("a type node needs `type: array`, `enum` or `object`".to_owned())
                );
            }
        };
        let (kind, presence) = split_optional(kind);
        let ty = match kind {
            "array" => {
                only_keys(entries, &["type", "items"], at)?;
                let items = required(entries, "items", at)?;
                Type::Array(Box::new(self.item_type(items, &at.key("items"), scope)?))
            }
            "enum" => {
                only_keys(entries, &["type", "literals"], at)?;
                Type::Enum(literals(
                    required(entries, "literals", at)?,
                    &at.key("literals"),
                )?)
            }
            "object" => {
                only_keys(entries, &["type", "properties"], at)?;
                let properties = required(entries, "properties", at)?;
                Type::Object(self.props(properties, &at.key("properties"), scope)?)
            }
            "union" => {
                extension(scope.grammar, "`type: union`", at)?;
                only_keys(entries, &["type", "members"], at)?;
                let members = required(entries, "members", at)?;
                Type::Union(self.members(members, &at.key("members"), scope)?)
            }
            other => {
                return Err(at.key("type").error(format!(
                    "unknown type node \"{other}\"; expected array, enum or object"
                )));
            }
        };
        Ok((ty, presence))
    }

    /// Extension: union members, told apart on the wire by JSON kind alone.
    fn members(&self, node: &Node, at: &At, scope: &Scope) -> Result<Vec<Member>, SpecError> {
        let entries = map_entries(Some(node), at)?;
        if entries.len() < 2 {
            return Err(at.error("needs at least two members".to_owned()));
        }
        let mut members = Vec::with_capacity(entries.len());
        let mut kinds: Vec<(JsonKind, &str)> = Vec::with_capacity(entries.len());
        for (name, value) in entries {
            let at = at.key(name);
            let ty = self.item_type(value, &at, scope)?;
            let kind = self.member_kind(&ty, &at)?;
            if let Some((_, other)) = kinds.iter().find(|(seen, _)| *seen == kind) {
                return Err(at.error(format!(
                    "is a JSON {kind}, like member \"{other}\"; members of a union carry no tag on \
                     the wire, so their JSON kinds must differ"
                )));
            }
            kinds.push((kind, name));
            members.push(Member {
                name: name.clone(),
                doc: value.doc.clone(),
                ty,
            });
        }
        Ok(members)
    }

    fn member_kind(&self, ty: &Type, at: &At) -> Result<JsonKind, SpecError> {
        match ty {
            Type::String | Type::Binary | Type::Enum(_) => Ok(JsonKind::String),
            Type::Boolean => Ok(JsonKind::Boolean),
            // A handle arrives as a remote object, as an object member would, so the two cannot
            // share a union.
            Type::Object(_)
            | Type::Channel(_)
            | Type::AnyChannel
            | Type::Element
            | Type::Handle(_) => Ok(JsonKind::Object),
            Type::Named(id) => match self.entries[id.0].kind {
                Kind::Object => Ok(JsonKind::Object),
                Kind::Enum => Ok(JsonKind::String),
                // `named_type` resolves only objects, enums and unions to `Named`.
                Kind::Union | Kind::Mixin | Kind::Interface | Kind::Handle => {
                    Err(at.error("cannot nest a union in a union".to_owned()))
                }
            },
            Type::Union(_) => Err(at.error("cannot nest a union in a union".to_owned())),
            Type::Int | Type::Float => {
                Err(at.error("is a number; numeric union members are not supported yet".to_owned()))
            }
            Type::Array(_) => {
                Err(at.error("is an array; array union members are not supported yet".to_owned()))
            }
            Type::Json => Err(at.error(
                "is `json`, which matches every JSON kind, so it cannot be a member".to_owned(),
            )),
            Type::Undefined => Err(at.error("is `undefined`, which cannot be a member".to_owned())),
        }
    }

    /// The definition `name` resolves to from a file of `grammar`.
    fn visible(&self, name: &str, grammar: Grammar) -> Option<DefId> {
        let id = *self.by_name.get(name)?;
        let defined_in = self.entries[id.0].file.grammar;
        (grammar == Grammar::Extended || defined_in == Grammar::Playwright).then_some(id)
    }
}

/// The JSON kinds a union tells its members apart by.
#[derive(Clone, Copy, PartialEq, Eq)]
enum JsonKind {
    String,
    Boolean,
    Object,
}

impl fmt::Display for JsonKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            JsonKind::String => "string",
            JsonKind::Boolean => "boolean",
            JsonKind::Object => "object",
        })
    }
}

/// A command's metainfo, checked with Playwright's generator rules (`generate_channels.js`).
fn meta(body: &[(String, Node)], raw_params: &[&str], at: &At) -> Result<Meta, SpecError> {
    let title = optional_string(body, "title", at)?;
    let subtitle = optional_string(body, "subtitle", at)?;
    let group = match optional_string(body, "group", at)?.as_deref() {
        None => None,
        Some("getter") => Some(Group::Getter),
        Some("configuration") => Some(Group::Configuration),
        Some("route") => Some(Group::Route),
        Some("default") => Some(Group::Default),
        Some(other) => return Err(at.key("group").error(format!("unknown group \"{other}\""))),
    };
    let render_params = match find(body, "renderParams") {
        None => Vec::new(),
        Some(node) => string_list(node, &at.key("renderParams"))?,
    };
    let internal = match find(body, "internal").map(|node| &node.value) {
        None => false,
        Some(Value::Bool(internal)) => *internal,
        Some(other) => {
            return Err(at
                .key("internal")
                .error(format!("must be a boolean, not {}", other.describe())));
        }
    };
    let flags = flags(find(body, "flags"), &at.key("flags"))?;

    for flag in &flags {
        if internal && matches!(flag, Flag::SlowMo | Flag::Snapshot | Flag::Pause) {
            return Err(at.error(format!(
                "has the \"{}\" flag, so it cannot be internal",
                flag_name(*flag)
            )));
        }
    }
    if title.is_none() && !internal {
        return Err(at.error("needs a title because it is not internal".to_owned()));
    }
    if internal && (group.is_some() || subtitle.is_some() || !render_params.is_empty()) {
        return Err(at.error(
            "is internal, so it cannot have a group, a subtitle or render params".to_owned(),
        ));
    }
    if subtitle.is_none() && !internal && raw_params.contains(&"selector") {
        return Err(at.error("has a \"selector\" parameter, so it needs a subtitle".to_owned()));
    }
    for entry in &render_params {
        // `[key=]path[:selector]`; the path's root must be a parameter.
        let path = entry.split(':').next().unwrap_or_default();
        let path = path.rsplit('=').next().unwrap_or_default();
        let root = path.split('.').next().unwrap_or_default();
        if !raw_params.contains(&root) {
            return Err(at
                .key("renderParams")
                .error(format!("has \"{entry}\", which does not name a parameter")));
        }
    }
    Ok(Meta {
        title,
        subtitle,
        group,
        render_params,
        internal,
        flags,
    })
}

fn flags(node: Option<&Node>, at: &At) -> Result<Vec<Flag>, SpecError> {
    let mut flags = Vec::new();
    for (name, value) in map_entries(node, at)? {
        let flag = match name.as_str() {
            "slowMo" => Flag::SlowMo,
            "snapshot" => Flag::Snapshot,
            "pause" => Flag::Pause,
            "input" => Flag::Input,
            "isAutoWaiting" => Flag::IsAutoWaiting,
            other => {
                return Err(at
                    .key(other)
                    .error("is not one of Playwright's flags".to_owned()));
            }
        };
        if !matches!(value.value, Value::Bool(true)) {
            return Err(at
                .key(name)
                .error("must be `true`; leave an unset flag out".to_owned()));
        }
        flags.push(flag);
    }
    flags.sort_unstable();
    Ok(flags)
}

/// The flag as Playwright's spec spells it.
fn flag_name(flag: Flag) -> &'static str {
    match flag {
        Flag::SlowMo => "slowMo",
        Flag::Snapshot => "snapshot",
        Flag::Pause => "pause",
        Flag::Input => "input",
        Flag::IsAutoWaiting => "isAutoWaiting",
    }
}

/// Enum literals: strings, or YAML `null`, which Playwright's generator interpolates into a
/// template string as `"null"` (`SerializedValue.v`). Anything else is rejected rather than
/// stringified, so an unquoted `-0` cannot silently become `"0"`.
fn literals(node: &Node, at: &At) -> Result<Vec<String>, SpecError> {
    let Value::Seq(items) = &node.value else {
        return Err(at.error("must be a list of literals".to_owned()));
    };
    if items.is_empty() {
        return Err(at.error("needs at least one literal".to_owned()));
    }
    let mut literals: Vec<String> = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let at = at.key(&index.to_string());
        let literal = match &item.value {
            Value::Str(text) => text.clone(),
            Value::Null => "null".to_owned(),
            other => {
                return Err(at.error(format!(
                    "must be a string; quote {} so its meaning cannot change",
                    other.describe()
                )));
            }
        };
        if literals.contains(&literal) {
            return Err(at.error(format!("repeats the literal \"{literal}\"")));
        }
        literals.push(literal);
    }
    Ok(literals)
}

/// The entries of a map node, or none for an absent or empty (`null`) node.
fn map_entries<'n>(node: Option<&'n Node>, at: &At) -> Result<&'n [(String, Node)], SpecError> {
    match node.map(|node| &node.value) {
        None | Some(Value::Null) => Ok(&[]),
        Some(Value::Map(entries)) => Ok(entries),
        Some(other) => Err(at.error(format!("must be a map, not {}", other.describe()))),
    }
}

fn find<'n>(entries: &'n [(String, Node)], key: &str) -> Option<&'n Node> {
    entries
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, node)| node)
}

fn required<'n>(entries: &'n [(String, Node)], key: &str, at: &At) -> Result<&'n Node, SpecError> {
    find(entries, key).ok_or_else(|| at.error(format!("needs a `{key}` key")))
}

fn only_keys(entries: &[(String, Node)], allowed: &[&str], at: &At) -> Result<(), SpecError> {
    match entries
        .iter()
        .find(|(key, _)| !allowed.contains(&key.as_str()))
    {
        None => Ok(()),
        Some((key, _)) => Err(at.key(key).error(format!(
            "is not part of the dialect here; expected one of {}",
            allowed.join(", ")
        ))),
    }
}

fn optional_string(
    entries: &[(String, Node)],
    key: &str,
    at: &At,
) -> Result<Option<String>, SpecError> {
    match find(entries, key).map(|node| &node.value) {
        None => Ok(None),
        Some(Value::Str(text)) => Ok(Some(text.clone())),
        Some(other) => Err(at
            .key(key)
            .error(format!("must be a string, not {}", other.describe()))),
    }
}

fn string_list(node: &Node, at: &At) -> Result<Vec<String>, SpecError> {
    let Value::Seq(items) = &node.value else {
        return Err(at.error("must be a list of strings".to_owned()));
    };
    let mut strings = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let Value::Str(text) = &item.value else {
            return Err(at
                .key(&index.to_string())
                .error(format!("must be a string, not {}", item.value.describe())));
        };
        strings.push(text.clone());
    }
    Ok(strings)
}

fn split_optional(text: &str) -> (&str, Presence) {
    match text.strip_suffix('?') {
        Some(name) => (name, Presence::Optional),
        None => (text, Presence::Required),
    }
}

/// Rejects an extension outside the extended grammar.
fn extension(grammar: Grammar, construct: &str, at: &At) -> Result<(), SpecError> {
    match grammar {
        Grammar::Extended => Ok(()),
        Grammar::Playwright => Err(at.error(format!(
            "uses {construct}, a Neo extension that only Neo's own spec files may use"
        ))),
    }
}

/// Two docs as consecutive paragraphs.
fn join_docs(first: &str, second: &str) -> String {
    match (first.is_empty(), second.is_empty()) {
        (_, true) => first.to_owned(),
        (true, false) => second.to_owned(),
        (false, false) => format!("{first}\n\n{second}"),
    }
}
