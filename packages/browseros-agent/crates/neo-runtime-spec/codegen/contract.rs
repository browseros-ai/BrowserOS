//! What the translator emits, with every Neo decision made once.
//!
//! The Rust and TypeScript printers render a [`Contract`] and decide nothing about names,
//! shapes, derives or docs, so the two languages cannot disagree. This module owns Neo's rules
//! on top of Playwright's dialect:
//!
//! - `serve.yml` selects the Playwright commands Neo serves and the interfaces Neo implements.
//!   A reference to an interface Neo does not implement is uninhabited: no value can exist, so
//!   Rust cannot produce one.
//! - `injected.yml` declares one interface, `Injected`, whose commands are the page's glue
//!   functions. A page call crosses plain JSON and handles, never a Playwright object.
//! - A handle (`Element` or a `type: handle` definition) is a page object Rust names by CDP object
//!   id. It may be a top-level parameter, which crosses as a `Runtime.callFunctionOn` argument,
//!   or a top-level result property (alone or as one member of a union), which `neo-runtime`
//!   reads with `Runtime.getProperties`. That read hands back every object as another page
//!   object, so the other properties of such a result must be primitives. Anywhere else a
//!   handle would have to cross as JSON, which it cannot, so the builder rejects it.
//! - Only the closure of those selections is emitted.
//! - Inline types are hoisted as `{Owner}{Prop}`, with `Item` for array items. Every type name,
//!   field and variant identifier is checked unique, and a recursive reference that no array
//!   breaks is boxed.

use std::collections::{BTreeSet, HashMap};

use crate::dialect::{
    Command, Def, DefId, DefKind, Grammar, Lookup, Member, Meta, Presence, Prop, Protocol, Returns,
    Type,
};
use crate::error::{At, SpecError, did_you_mean};
use crate::yaml::{Node, Value};

const SERVE: &str = "serve.yml";
const INJECTED: &str = "injected.yml";

/// Both generated modules.
pub struct Contract {
    pub serve: ServeModule,
    pub injected: InjectedModule,
}

/// `src/serve.rs` and `ts/serve.ts`: the Playwright commands Neo serves.
pub struct ServeModule {
    /// One per interface keyed in `serve.yml` that serves a command, in `serve.yml` order.
    pub interfaces: Vec<ServedInterface>,
    pub types: Vec<TypeDef>,
}

/// The commands Neo serves on one interface.
pub struct ServedInterface {
    /// The Playwright interface, such as `Frame`.
    pub name: String,
    /// The Rust call enum, such as `FrameCall`.
    pub call_enum: String,
    /// Every params type derives `Eq`, so the call enum can too.
    pub eq: bool,
    pub commands: Vec<ServedCommand>,
}

pub struct ServedCommand {
    /// The wire method name, such as `textContent`.
    pub method: String,
    /// The call enum variant, such as `TextContent`.
    pub variant: String,
    /// The params struct in the module's types, such as `FrameTextContentParams`.
    pub params: String,
    pub result: String,
    /// Playwright's command declares no `returns`, so its server replies with no `result` at all
    /// and its client resolves `undefined`, not `{}`.
    pub void_result: bool,
    pub meta: Meta,
}

/// `src/injected.rs` and `ts/injected.ts`: the page calls `neo-runtime` makes.
pub struct InjectedModule {
    /// The glue interface, `Injected`.
    pub interface: String,
    pub calls: Vec<PageCall>,
    pub types: Vec<TypeDef>,
}

/// One glue function in the page. Its params struct lists the handle arguments as fields of
/// [`FieldType::Handle`], in spec order, and its result struct's [`Crossing`] says how the result
/// comes back.
pub struct PageCall {
    /// The glue table key, such as `textContent`.
    pub name: String,
    pub params: String,
    pub result: String,
}

/// A generated type.
pub struct TypeDef {
    pub name: String,
    /// One string per line; an empty string separates paragraphs.
    pub doc: Vec<String>,
    /// TypeScript exports it by name. A hoisted inline type is not exported: TypeScript writes
    /// its body where it is used, as Playwright's own generator does. Rust names every type.
    pub exported: bool,
    pub body: Body,
}

pub enum Body {
    Struct(Struct),
    Enum(Vec<Variant>),
    Union(Union),
    Channel(Channel),
    /// A `type: handle` definition: a marker kind in Rust, a key of `InjectedHandleKinds` in
    /// TypeScript.
    Handle,
}

pub struct Struct {
    pub fields: Vec<Field>,
    /// No `float` is reachable, so the struct derives `Eq`.
    pub eq: bool,
    /// A page call type decoded from JSON: decoding rejects unknown fields, so a page bundle built
    /// from another spec fails on its first call instead of dropping data.
    pub deny_unknown_fields: bool,
    pub crossing: Crossing,
}

/// How a struct or union crosses its seam. Served commands only ever cross as JSON; page calls
/// use all three.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Crossing {
    /// As JSON, both ways.
    Json,
    /// Page call params that hold handles: the other fields serialize as the JSON params, and
    /// each handle crosses as a `Runtime.callFunctionOn` argument. Never decoded.
    Arguments,
    /// A page call result that holds a handle, or a union with a handle member: read property by
    /// property from `Runtime.getProperties`, never as JSON.
    Properties,
}

pub struct Field {
    /// The JSON key.
    pub wire: String,
    /// The Rust identifier: snake case, raw for a keyword.
    pub ident: String,
    pub doc: Vec<String>,
    pub ty: FieldType,
    /// The key may be absent. It is never `null`.
    pub optional: bool,
}

pub enum FieldType {
    String,
    Bool,
    Int,
    Float,
    /// `binary`: base64 text, left encoded.
    Base64,
    Json,
    /// A type in the same module.
    Ref(String),
    /// A [`FieldType::Ref`] on a cycle that no array breaks, so Rust boxes it.
    Boxed(String),
    Array(Box<FieldType>),
    /// A page object of this kind, only ever a top-level field of page call params or results.
    Handle(HandleKindRef),
}

/// The kind of page object a handle names.
#[derive(Clone, PartialEq, Eq)]
pub enum HandleKindRef {
    /// The built-in `Element`.
    Element,
    /// A `type: handle` definition, by name.
    Named(String),
}

/// An enum literal.
pub struct Variant {
    /// The wire string.
    pub literal: String,
    pub ident: String,
}

/// Extension: a value that is exactly one of its members, told apart by JSON kind.
pub struct Union {
    pub members: Vec<UnionMember>,
    /// No `float` is reachable, so the union derives `Eq`.
    pub eq: bool,
    /// Every member is a boolean or an enum, so the union derives `Copy` and `Hash`.
    pub copy_hash: bool,
    /// [`Crossing::Json`], or [`Crossing::Properties`] when a member is a handle.
    pub crossing: Crossing,
}

pub struct UnionMember {
    /// The Rust variant.
    pub ident: String,
    pub doc: Vec<String>,
    pub kind: MemberKind,
}

/// A union member, by the JSON kind that selects it.
pub enum MemberKind {
    Bool,
    String,
    /// A JSON string that must be one of this enum's literals.
    Enum(String),
    /// A JSON object decoded as this struct, boxed in Rust.
    Object(String),
    /// A page object, which a result property holds instead of a primitive member.
    Handle(HandleKindRef),
}

/// A reference to an object of a Playwright interface.
pub struct Channel {
    /// The interface, such as `Response`.
    pub interface: String,
    /// Keyed in `serve.yml`: `{ guid }` on the wire. Otherwise uninhabited: an empty Rust enum
    /// and TypeScript `never`, so Neo can never send or accept one.
    pub implemented: bool,
}

impl Contract {
    /// Applies `serve.yml` and Neo's rules for `injected.yml` to `protocol`.
    pub fn build(protocol: &Protocol, serve: &Node) -> Result<Contract, SpecError> {
        let selection = read_selection(protocol, serve)?;
        let injected = find_injected(protocol)?;
        Ok(Contract {
            serve: build_serve(protocol, &selection)?,
            injected: build_injected(protocol, injected)?,
        })
    }
}

/// What `serve.yml` selects, with each entry's location for later errors.
struct Selection<'protocol> {
    interfaces: Vec<Implemented<'protocol>>,
    types: Vec<(DefId, At)>,
}

/// An interface Neo implements, and the commands it serves on it.
struct Implemented<'protocol> {
    id: DefId,
    at: At,
    commands: Vec<(&'protocol Command, At)>,
}

fn read_selection<'p>(protocol: &'p Protocol, serve: &Node) -> Result<Selection<'p>, SpecError> {
    let at = At::file(SERVE);
    let Value::Map(entries) = &serve.value else {
        return Err(at.error("must be a map with `interfaces` and `types`".to_owned()));
    };
    if let Some((key, _)) = entries
        .iter()
        .find(|(key, _)| key != "interfaces" && key != "types")
    {
        return Err(at
            .key(key)
            .error("is not a serve.yml key; expected interfaces or types".to_owned()));
    }
    let mut selection = Selection {
        interfaces: Vec::new(),
        types: Vec::new(),
    };
    let Value::Map(interfaces) = &entry(entries, "interfaces", &at)?.value else {
        return Err(at
            .key("interfaces")
            .error("must map interface names to command lists".to_owned()));
    };
    for (name, node) in interfaces {
        selection.interfaces.push(read_interface(
            protocol,
            name,
            node,
            at.key("interfaces").key(name),
        )?);
    }
    let Value::Seq(types) = &entry(entries, "types", &at)?.value else {
        return Err(at
            .key("types")
            .error("must be a list of type names".to_owned()));
    };
    for (index, item) in types.iter().enumerate() {
        let at = at.key("types").key(&index.to_string());
        let Value::Str(name) = &item.value else {
            return Err(at.error("must be a type name".to_owned()));
        };
        let is_type = |def: &Def| matches!(def.kind, DefKind::Object(_) | DefKind::Enum(_));
        let Some(id) = playwright_def(protocol, name, is_type) else {
            let types = protocol
                .defs()
                .filter(|(_, def)| def.grammar == Grammar::Playwright && is_type(def));
            let suggestion = did_you_mean(name, types.map(|(_, def)| def.name.as_str()));
            return Err(at.error(format!(
                "\"{name}\" is not a Playwright object or enum{suggestion}; list interfaces under \
                 `interfaces`"
            )));
        };
        if selection.types.iter().any(|(seen, _)| *seen == id) {
            return Err(at.error(format!("lists \"{name}\" twice")));
        }
        selection.types.push((id, at));
    }
    Ok(selection)
}

fn read_interface<'p>(
    protocol: &'p Protocol,
    name: &str,
    node: &Node,
    at: At,
) -> Result<Implemented<'p>, SpecError> {
    let Some(id) = playwright_def(protocol, name, |def| def.as_interface().is_some()) else {
        let interfaces = protocol
            .defs()
            .filter(|(_, def)| def.grammar == Grammar::Playwright && def.as_interface().is_some());
        let suggestion = did_you_mean(name, interfaces.map(|(_, def)| def.name.as_str()));
        return Err(at.error(format!("is not a Playwright interface{suggestion}")));
    };
    let Value::Seq(items) = &node.value else {
        return Err(
            at.error("must list the commands Neo serves on it; write [] for none".to_owned())
        );
    };
    let mut commands: Vec<(&Command, At)> = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let at = at.key(&index.to_string());
        let Value::Str(method) = &item.value else {
            return Err(at.error("must be a command name".to_owned()));
        };
        let command = match protocol.command(id, method) {
            Lookup::Declared(command) => command,
            Lookup::Inherited(from) => {
                return Err(at.error(format!(
                    "{name}.{method} is declared on {}, which {name} extends; serving an inherited \
                     command is not supported yet",
                    protocol.def(from).name
                )));
            }
            Lookup::Missing => {
                let declared = protocol
                    .def(id)
                    .as_interface()
                    .into_iter()
                    .flat_map(|interface| {
                        interface
                            .commands
                            .iter()
                            .map(|command| command.name.as_str())
                    });
                let suggestion = did_you_mean(method, declared);
                return Err(at.error(format!("{name} has no command \"{method}\"{suggestion}")));
            }
        };
        if commands.iter().any(|(seen, _)| seen.name == *method) {
            return Err(at.error(format!("lists \"{method}\" twice")));
        }
        commands.push((command, at));
    }
    Ok(Implemented { id, at, commands })
}

/// The definition `name` in Playwright's files, if `accept` takes it.
fn playwright_def(protocol: &Protocol, name: &str, accept: impl Fn(&Def) -> bool) -> Option<DefId> {
    protocol.get(name).filter(|id| {
        let def = protocol.def(*id);
        def.grammar == Grammar::Playwright && accept(def)
    })
}

/// The one interface of the extended file: `Injected`, the page's glue table.
fn find_injected(protocol: &Protocol) -> Result<DefId, SpecError> {
    let mut interfaces = protocol
        .defs()
        .filter(|(_, def)| def.grammar == Grammar::Extended && def.as_interface().is_some());
    match (interfaces.next(), interfaces.next()) {
        (Some((id, def)), None) if def.name == "Injected" => Ok(id),
        (None, _) => Err(At::file(INJECTED)
            .error("must declare the interface `Injected`, the page's glue table".to_owned())),
        (Some((_, def)), _) => Err(At::file(INJECTED).key(&def.name).error(
            "is an interface; injected.yml declares exactly one, named `Injected`".to_owned(),
        )),
    }
}

fn build_serve(protocol: &Protocol, selection: &Selection<'_>) -> Result<ServeModule, SpecError> {
    let implemented: Vec<DefId> = selection
        .interfaces
        .iter()
        .map(|interface| interface.id)
        .collect();
    let mut closure = Closure::default();
    for interface in &selection.interfaces {
        if let Some(initializer) = initializer(protocol, interface.id) {
            closure.props(protocol, initializer.props);
        }
        for (command, at) in &interface.commands {
            if command.error_details.is_some() {
                return Err(
                    at.error("declares errorDetails, which Neo does not serve yet".to_owned())
                );
            }
            closure.command(protocol, command);
        }
    }
    for (id, _) in &selection.types {
        closure.named(protocol, *id);
    }

    let mut builder = Builder::new(protocol, Side::Serve { implemented });
    let mut interfaces = Vec::new();
    for interface in &selection.interfaces {
        let def = protocol.def(interface.id);
        if let Some(initializer) = initializer(protocol, interface.id) {
            let head = Head {
                name: format!("{}Initializer", def.name),
                doc: paragraphs(&[
                    &format!(
                        "The initializer of a `{}` object (`spec/{}`), sent when Neo creates one.",
                        def.name, def.file
                    ),
                    initializer.doc,
                ]),
                exported: true,
            };
            builder.add_struct(head, initializer.props, &initializer.at, Place::Plain)?;
        }
        let mut commands = Vec::with_capacity(interface.commands.len());
        for (command, _) in &interface.commands {
            commands.push(builder.served_command(def, command)?);
        }
        if !commands.is_empty() {
            interfaces.push(ServedInterface {
                name: def.name.clone(),
                call_enum: format!("{}Call", def.name),
                eq: false,
                commands,
            });
        }
    }
    for id in &closure.channels {
        builder.add_channel(*id)?;
    }
    for id in &closure.named {
        builder.add_named(*id)?;
    }
    let types = builder.finish();

    check_inhabited(protocol, selection, &types)?;
    for interface in &mut interfaces {
        interface.eq = interface
            .commands
            .iter()
            .all(|command| matches!(find(&types, &command.params), Some(Body::Struct(params)) if params.eq));
    }
    Ok(ServeModule { interfaces, types })
}

fn build_injected(protocol: &Protocol, id: DefId) -> Result<InjectedModule, SpecError> {
    let def = protocol.def(id);
    let at = At::file(&def.file).key(&def.name);
    let Some(interface) = def.as_interface() else {
        return Err(at.error("must be an interface".to_owned()));
    };
    if interface.extends.is_some() {
        return Err(at
            .key("extends")
            .error("the glue table extends nothing".to_owned()));
    }
    if let Some(event) = interface.events.first() {
        return Err(at
            .key("events")
            .key(event)
            .error("the page sends no events".to_owned()));
    }
    let mut closure = Closure::default();
    if let Some(props) = &interface.initializer {
        closure.props(protocol, props);
    }
    for command in &interface.commands {
        let at = at.key("commands").key(&command.name);
        // Every page call is `internal: true`, which Playwright's generator requires of a command
        // without a title; anything more would be metainfo Neo never reads.
        if command.meta.title.is_some() || !command.meta.flags.is_empty() {
            return Err(at.error(
                "a page call carries no metainfo: declare only `internal: true`, `parameters` and \
                 `returns`"
                    .to_owned(),
            ));
        }
        if command.error_details.is_some() {
            return Err(at.key("errorDetails").error(
                "a page call reports failures by throwing, as Playwright's page code does"
                    .to_owned(),
            ));
        }
        closure.command(protocol, command);
    }
    // A type nothing reaches would be generated and never checked against the page.
    for (other, other_def) in protocol.defs() {
        let unused = other != id
            && other_def.grammar == Grammar::Extended
            && !matches!(other_def.kind, DefKind::Mixin)
            && !closure.named.contains(&other);
        if unused {
            return Err(At::file(&other_def.file).key(&other_def.name).error(
                "is not reachable from `Injected`; use it in a page call or remove it".to_owned(),
            ));
        }
    }

    let mut builder = Builder::new(protocol, Side::Page);
    if let Some(props) = &interface.initializer {
        let head = Head {
            name: format!("{}Initializer", def.name),
            doc: paragraphs(&[
                &format!(
                    "The initializer of the page's `{}` glue object (`spec/{}`).",
                    def.name, def.file
                ),
                &interface.initializer_doc,
            ]),
            exported: true,
        };
        builder.add_struct(head, props, &at.key("initializer"), Place::Plain)?;
    }
    let mut calls = Vec::with_capacity(interface.commands.len());
    for command in &interface.commands {
        calls.push(builder.page_call(def, command)?);
    }
    for id in &closure.named {
        builder.add_named(*id)?;
    }
    Ok(InjectedModule {
        interface: def.name.clone(),
        calls,
        types: builder.finish(),
    })
}

/// The interface's initializer, or the nearest ancestor's when it declares none, as Playwright
/// validates it.
struct Initializer<'protocol> {
    props: &'protocol [Prop],
    doc: &'protocol str,
    at: At,
}

fn initializer(protocol: &Protocol, id: DefId) -> Option<Initializer<'_>> {
    let mut current = Some(id);
    while let Some(id) = current {
        let def = protocol.def(id);
        let interface = def.as_interface()?;
        if let Some(props) = &interface.initializer {
            let at = At::file(&def.file).key(&def.name).key("initializer");
            return Some(Initializer {
                props,
                doc: &interface.initializer_doc,
                at,
            });
        }
        current = interface.extends;
    }
    None
}

/// The named definitions and interfaces a module's roots reach, in load order.
#[derive(Default)]
struct Closure {
    named: BTreeSet<DefId>,
    channels: BTreeSet<DefId>,
}

impl Closure {
    fn command(&mut self, protocol: &Protocol, command: &Command) {
        if let Some(params) = &command.params {
            self.props(protocol, params);
        }
        if let Returns::Props(returns) = &command.returns {
            self.props(protocol, returns);
        }
    }

    fn props(&mut self, protocol: &Protocol, props: &[Prop]) {
        for prop in props {
            self.ty(protocol, &prop.ty);
        }
    }

    fn members(&mut self, protocol: &Protocol, members: &[Member]) {
        for member in members {
            self.ty(protocol, &member.ty);
        }
    }

    fn named(&mut self, protocol: &Protocol, id: DefId) {
        if !self.named.insert(id) {
            return;
        }
        match &protocol.def(id).kind {
            DefKind::Object(props) => self.props(protocol, props),
            DefKind::Union(members) => self.members(protocol, members),
            DefKind::Enum(_) | DefKind::Mixin | DefKind::Interface(_) | DefKind::Handle => {}
        }
    }

    fn ty(&mut self, protocol: &Protocol, ty: &Type) {
        match ty {
            // A handle definition counts as reached, so an unused one is reported like any other
            // unused definition.
            Type::Named(id) | Type::Handle(id) => self.named(protocol, *id),
            Type::Channel(id) => {
                self.channels.insert(*id);
            }
            Type::Array(items) => self.ty(protocol, items),
            Type::Object(props) => self.props(protocol, props),
            Type::Union(members) => self.members(protocol, members),
            Type::String
            | Type::Boolean
            | Type::Int
            | Type::Float
            | Type::Binary
            | Type::Json
            | Type::Undefined
            | Type::AnyChannel
            | Type::Enum(_)
            | Type::Element => {}
        }
    }
}

/// The module a [`Builder`] fills, and the rules that differ between the two.
enum Side {
    /// Served commands: Playwright's wire, where a channel reference is inhabited only when its
    /// interface is keyed in `serve.yml`.
    Serve { implemented: Vec<DefId> },
    /// Page calls: plain JSON and handles, and decoding rejects unknown fields.
    Page,
}

/// Where a struct's properties sit, which decides whether one may be a handle.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    /// Anywhere but the top level of a page call: a handle cannot cross as JSON.
    Plain,
    /// A page call's `parameters`: a property may be a handle, passed as a CDP argument.
    PageParams,
    /// A page call's `returns`: a property may be a handle or a union with a handle member, read
    /// with `Runtime.getProperties`.
    PageReturns,
}

/// A type's name, docs and TypeScript visibility.
struct Head {
    name: String,
    doc: Vec<String>,
    exported: bool,
}

/// Where an inline type sits, which names it when it is hoisted.
struct Site {
    /// The hoisted name, such as `FrameAriaSnapshotParamsMode`.
    name: String,
    /// What it is the type of, such as `` `FrameAriaSnapshotParams.mode` ``.
    of: String,
}

impl Site {
    fn prop(owner: &str, prop: &str) -> Site {
        Site {
            name: format!("{owner}{}", title_case(prop)),
            of: format!("`{owner}.{prop}`"),
        }
    }

    fn member(owner: &str, member: &str) -> Site {
        Site {
            name: format!("{owner}{}", title_case(member)),
            of: format!("member `{member}` of `{owner}`"),
        }
    }

    fn items(&self) -> Site {
        Site {
            name: format!("{}Item", self.name),
            of: format!("an item of {}", self.of),
        }
    }

    fn head(&self) -> Head {
        Head {
            name: self.name.clone(),
            doc: vec![format!("The type of {}.", self.of)],
            exported: false,
        }
    }
}

/// Fills one module: lowers dialect types, hoists inline ones, and keeps names unique.
struct Builder<'protocol> {
    protocol: &'protocol Protocol,
    side: Side,
    types: Vec<TypeDef>,
    /// Each type name in the module, with where it came from, for collision errors.
    names: HashMap<String, String>,
}

impl<'protocol> Builder<'protocol> {
    fn new(protocol: &'protocol Protocol, side: Side) -> Builder<'protocol> {
        Builder {
            protocol,
            side,
            types: Vec::new(),
            names: HashMap::new(),
        }
    }

    fn served_command(&mut self, def: &Def, command: &Command) -> Result<ServedCommand, SpecError> {
        let at = At::file(&def.file)
            .key(&def.name)
            .key("commands")
            .key(&command.name);
        let method = format!("{}.{}", def.name, command.name);
        let stem = format!("{}{}", def.name, title_case(&command.name));
        let params = format!("{stem}Params");
        let head = Head {
            name: params.clone(),
            doc: paragraphs(&[
                &format!("Params of `{method}` (`spec/{}`).", def.file),
                &command.doc,
            ]),
            exported: true,
        };
        self.add_struct(
            head,
            command.params.as_deref().unwrap_or_default(),
            &at.key("parameters"),
            Place::Plain,
        )?;
        let returns: &[Prop] = match &command.returns {
            Returns::Props(props) => props,
            Returns::Nothing => &[],
        };
        let result = format!("{stem}Result");
        let head = Head {
            name: result.clone(),
            doc: vec![format!("Result of `{method}` (`spec/{}`).", def.file)],
            exported: true,
        };
        self.add_struct(head, returns, &at.key("returns"), Place::Plain)?;
        Ok(ServedCommand {
            method: command.name.clone(),
            variant: title_case(&command.name),
            params,
            result,
            void_result: matches!(command.returns, Returns::Nothing),
            meta: command.meta.clone(),
        })
    }

    fn page_call(&mut self, def: &Def, command: &Command) -> Result<PageCall, SpecError> {
        let at = At::file(&def.file)
            .key(&def.name)
            .key("commands")
            .key(&command.name);
        let call = format!("{}.{}", def.name, command.name);
        let stem = format!("{}{}", def.name, title_case(&command.name));
        let params = format!("{stem}Params");
        let head = Head {
            name: params.clone(),
            doc: paragraphs(&[
                &format!("Params of the page call `{call}` (`spec/{}`).", def.file),
                &command.doc,
            ]),
            exported: true,
        };
        self.add_struct(
            head,
            command.params.as_deref().unwrap_or_default(),
            &at.key("parameters"),
            Place::PageParams,
        )?;
        let returns: &[Prop] = match &command.returns {
            Returns::Props(props) => props,
            Returns::Nothing => &[],
        };
        let result = format!("{stem}Result");
        let head = Head {
            name: result.clone(),
            doc: vec![format!(
                "Result of the page call `{call}` (`spec/{}`).",
                def.file
            )],
            exported: true,
        };
        self.add_struct(head, returns, &at.key("returns"), Place::PageReturns)?;
        Ok(PageCall {
            name: command.name.clone(),
            params,
            result,
        })
    }

    /// A named object, enum or union, under its own name.
    fn add_named(&mut self, id: DefId) -> Result<(), SpecError> {
        let def = self.protocol.def(id);
        let at = At::file(&def.file).key(&def.name);
        let doc = match def.grammar {
            Grammar::Playwright => paragraphs(&[
                &format!("Playwright's `{}` (`spec/{}`).", def.name, def.file),
                &def.doc,
            ]),
            Grammar::Extended if def.doc.is_empty() => {
                vec![format!("`{}` (`spec/{}`).", def.name, def.file)]
            }
            Grammar::Extended => paragraphs(&[&def.doc]),
        };
        let head = Head {
            name: def.name.clone(),
            doc,
            exported: true,
        };
        match &def.kind {
            DefKind::Object(props) => self.add_struct(head, props, &at, Place::Plain),
            DefKind::Enum(literals) => self.add_enum(head, literals, &at),
            DefKind::Union(members) => self.add_union(head, members, &at),
            DefKind::Handle => self.add_handle(head, &at),
            // `Type::Named`, which the closure follows, never names these.
            DefKind::Mixin | DefKind::Interface(_) => Ok(()),
        }
    }

    /// A `type: handle` definition. TypeScript writes it as a key of `InjectedHandleKinds`, not
    /// as a type of its own, because the glue binds it to a Playwright page type.
    fn add_handle(&mut self, head: Head, at: &At) -> Result<(), SpecError> {
        self.claim(&head.name, at)?;
        self.types.push(TypeDef {
            name: head.name,
            doc: head.doc,
            exported: false,
            body: Body::Handle,
        });
        Ok(())
    }

    fn add_channel(&mut self, id: DefId) -> Result<(), SpecError> {
        let def = self.protocol.def(id);
        let implemented = match &self.side {
            Side::Serve { implemented } => implemented.contains(&id),
            Side::Page => false,
        };
        let name = format!("{}Channel", def.name);
        let doc = if implemented {
            vec![format!(
                "A reference to a `{}` object (`spec/{}`), as Playwright's wire carries it.",
                def.name, def.file
            )]
        } else {
            vec![format!(
                "A reference to a `{0}` object (`spec/{1}`). Neo creates no `{0}` objects (`{0}` is \
                 not a key of `spec/serve.yml`), so this type has no values.",
                def.name, def.file
            )]
        };
        self.claim(&name, &At::file(&def.file).key(&def.name))?;
        self.types.push(TypeDef {
            name,
            doc,
            exported: true,
            body: Body::Channel(Channel {
                interface: def.name.clone(),
                implemented,
            }),
        });
        Ok(())
    }

    fn add_struct(
        &mut self,
        head: Head,
        props: &[Prop],
        at: &At,
        place: Place,
    ) -> Result<(), SpecError> {
        self.claim(&head.name, at)?;
        // Hoisted types land right after their owner.
        let slot = self.types.len();
        let holds_handle = match place {
            Place::Plain => false,
            Place::PageParams => props
                .iter()
                .any(|prop| self.handle_kind(&prop.ty).is_some()),
            Place::PageReturns => props.iter().any(|prop| {
                self.handle_kind(&prop.ty).is_some() || self.has_handle_member(&prop.ty)
            }),
        };
        let mut fields: Vec<Field> = Vec::with_capacity(props.len());
        for prop in props {
            let at = at.key(&prop.name);
            let ident = field_ident(&prop.name)
                .ok_or_else(|| at.error("cannot be spelled as a Rust field name".to_owned()))?;
            if let Some(other) = fields.iter().find(|field| field.ident == ident) {
                return Err(at.error(format!(
                    "and \"{}\" both become the Rust field `{ident}`",
                    other.wire
                )));
            }
            let site = Site::prop(&head.name, &prop.name);
            let handle = self.handle_kind(&prop.ty);
            let ty = match (place, handle) {
                (Place::PageParams, Some(_)) if prop.presence == Presence::Optional => {
                    return Err(at.error(
                        "is an optional handle; a handle argument is passed by position, so it \
                         cannot be absent"
                            .to_owned(),
                    ));
                }
                (Place::PageParams | Place::PageReturns, Some(kind)) => FieldType::Handle(kind),
                (Place::PageReturns, None) if self.has_handle_member(&prop.ty) => {
                    self.lower_handle_union(&prop.ty, &site, &at)?
                }
                (Place::PageReturns, None) if holds_handle && !self.is_primitive(&prop.ty) => {
                    return Err(at.error(
                        "is not a primitive; a result that holds a handle is read with \
                         `Runtime.getProperties`, which hands back objects and arrays as page \
                         objects too, so its other properties must be primitives or string enums"
                            .to_owned(),
                    ));
                }
                _ => self.lower(&prop.ty, &site, &at)?,
            };
            let note = self.unimplemented(&prop.ty).map(|interface| {
                format!(
                    "Always absent: `{interface}` is not a key of `spec/serve.yml`, so Neo creates \
                     no `{interface}` objects."
                )
            });
            let doc = paragraphs(&[&prop.doc, note.as_deref().unwrap_or_default()]);
            fields.push(Field {
                wire: prop.name.clone(),
                ident,
                doc,
                ty,
                optional: prop.presence == Presence::Optional,
            });
        }
        let crossing = match place {
            Place::PageParams if holds_handle => Crossing::Arguments,
            Place::PageReturns if holds_handle => Crossing::Properties,
            Place::Plain | Place::PageParams | Place::PageReturns => Crossing::Json,
        };
        let mut doc = head.doc;
        if crossing == Crossing::Properties {
            doc.push(String::new());
            doc.push(
                "It holds a page object, so `neo-runtime` reads it with `Runtime.getProperties`, \
                 not by value."
                    .to_owned(),
            );
        }
        let body = Body::Struct(Struct {
            fields,
            eq: true,
            deny_unknown_fields: matches!(self.side, Side::Page) && crossing == Crossing::Json,
            crossing,
        });
        self.types.insert(
            slot,
            TypeDef {
                name: head.name,
                doc,
                // TypeScript writes a handle result inline where the glue's handle kinds are in
                // scope.
                exported: head.exported && crossing != Crossing::Properties,
                body,
            },
        );
        Ok(())
    }

    fn add_enum(&mut self, head: Head, literals: &[String], at: &At) -> Result<(), SpecError> {
        self.claim(&head.name, at)?;
        let mut variants: Vec<Variant> = Vec::with_capacity(literals.len());
        for literal in literals {
            let ident = variant_ident(literal).ok_or_else(|| {
                at.error(format!(
                    "has the literal \"{literal}\", which cannot be a Rust variant name"
                ))
            })?;
            if let Some(other) = variants.iter().find(|variant| variant.ident == ident) {
                return Err(at.error(format!(
                    "has the literals \"{}\" and \"{literal}\", which both become the variant `{ident}`",
                    other.literal
                )));
            }
            variants.push(Variant {
                literal: literal.clone(),
                ident,
            });
        }
        self.types.push(TypeDef {
            name: head.name,
            doc: head.doc,
            exported: head.exported,
            body: Body::Enum(variants),
        });
        Ok(())
    }

    fn add_union(&mut self, head: Head, members: &[Member], at: &At) -> Result<(), SpecError> {
        self.claim(&head.name, at)?;
        let slot = self.types.len();
        let mut union_members: Vec<UnionMember> = Vec::with_capacity(members.len());
        for member in members {
            let at = at.key(&member.name);
            let ident = variant_ident(&member.name)
                .ok_or_else(|| at.error("cannot be spelled as a Rust variant name".to_owned()))?;
            if union_members.iter().any(|other| other.ident == ident) {
                return Err(at.error(format!(
                    "collides with another member as the variant `{ident}`"
                )));
            }
            let kind = match self.handle_kind(&member.ty) {
                Some(handle) => MemberKind::Handle(handle),
                None => self.member_kind(member, &head.name, &at)?,
            };
            union_members.push(UnionMember {
                ident,
                doc: paragraphs(&[&member.doc]),
                kind,
            });
        }
        // The dialect lets a handle member share a union only with string and boolean members,
        // which `Runtime.getProperties` hands back as values, so the property's place tells the
        // members apart.
        let crossing = if union_members
            .iter()
            .any(|member| matches!(member.kind, MemberKind::Handle(_)))
        {
            Crossing::Properties
        } else {
            Crossing::Json
        };
        let body = Body::Union(Union {
            members: union_members,
            eq: true,
            copy_hash: false,
            crossing,
        });
        self.types.insert(
            slot,
            TypeDef {
                name: head.name,
                doc: head.doc,
                exported: head.exported && crossing == Crossing::Json,
                body,
            },
        );
        Ok(())
    }

    /// A union member that is not a handle. Lowering hoists an inline member and applies the
    /// module's rules; the dialect has already limited members to these kinds.
    fn member_kind(
        &mut self,
        member: &Member,
        owner: &str,
        at: &At,
    ) -> Result<MemberKind, SpecError> {
        let member_type = self.lower(&member.ty, &Site::member(owner, &member.name), at)?;
        let is_enum = match &member.ty {
            Type::Enum(_) => true,
            Type::Named(id) => matches!(self.protocol.def(*id).kind, DefKind::Enum(_)),
            _ => false,
        };
        match member_type {
            FieldType::Bool => Ok(MemberKind::Bool),
            FieldType::String | FieldType::Base64 => Ok(MemberKind::String),
            FieldType::Ref(name) if is_enum => Ok(MemberKind::Enum(name)),
            FieldType::Ref(name) => Ok(MemberKind::Object(name)),
            _ => Err(at.error("is not a kind of union member Neo supports yet".to_owned())),
        }
    }

    /// The field type for `ty`, hoisting an inline type under `site`'s name.
    fn lower(&mut self, ty: &Type, site: &Site, at: &At) -> Result<FieldType, SpecError> {
        let page = matches!(self.side, Side::Page);
        let lowered = match ty {
            Type::String => FieldType::String,
            Type::Boolean => FieldType::Bool,
            Type::Int => FieldType::Int,
            Type::Float => FieldType::Float,
            Type::Json => FieldType::Json,
            Type::Binary if page => {
                return Err(at.error(
                    "is `binary`: a page call crosses plain JSON, and base64 would be work in the \
                     page"
                        .to_owned(),
                ));
            }
            Type::Binary => FieldType::Base64,
            Type::Undefined => {
                return Err(at.error("is `undefined`, which Neo does not support yet".to_owned()));
            }
            Type::AnyChannel => {
                return Err(at.error("is `Channel`, which Neo does not support yet".to_owned()));
            }
            Type::Channel(id) if page => {
                return Err(at.error(format!(
                    "is a `{}` object; a page call crosses plain JSON and handles, never a \
                     Playwright object",
                    self.protocol.def(*id).name
                )));
            }
            Type::Element | Type::Handle(_) => {
                return Err(at.error(
                    "is a handle, which crosses only as a top-level parameter or result property \
                     of a page call"
                        .to_owned(),
                ));
            }
            Type::Named(_) | Type::Union(_) if self.has_handle_member(ty) => {
                return Err(at.error(
                    "is a union with a handle member, which crosses only as a top-level result \
                     property of a page call"
                        .to_owned(),
                ));
            }
            Type::Channel(id) => FieldType::Ref(format!("{}Channel", self.protocol.def(*id).name)),
            Type::Named(id) => FieldType::Ref(self.protocol.def(*id).name.clone()),
            Type::Array(items) => {
                FieldType::Array(Box::new(self.lower(items, &site.items(), at)?))
            }
            Type::Enum(literals) => {
                self.add_enum(site.head(), literals, at)?;
                FieldType::Ref(site.name.clone())
            }
            Type::Object(props) => {
                self.add_struct(site.head(), props, at, Place::Plain)?;
                FieldType::Ref(site.name.clone())
            }
            Type::Union(members) => {
                self.add_union(site.head(), members, at)?;
                FieldType::Ref(site.name.clone())
            }
        };
        Ok(lowered)
    }

    /// A top-level result property whose union has a handle member, which [`Builder::lower`]
    /// rejects everywhere else.
    fn lower_handle_union(
        &mut self,
        ty: &Type,
        site: &Site,
        at: &At,
    ) -> Result<FieldType, SpecError> {
        match ty {
            Type::Named(id) => Ok(FieldType::Ref(self.protocol.def(*id).name.clone())),
            Type::Union(members) => {
                self.add_union(site.head(), members, at)?;
                Ok(FieldType::Ref(site.name.clone()))
            }
            _ => self.lower(ty, site, at),
        }
    }

    /// The kind of page object `ty` names, when it is a handle.
    fn handle_kind(&self, ty: &Type) -> Option<HandleKindRef> {
        match ty {
            Type::Element => Some(HandleKindRef::Element),
            Type::Handle(id) => Some(HandleKindRef::Named(self.protocol.def(*id).name.clone())),
            _ => None,
        }
    }

    /// Whether `ty` is a union, named or inline, with a handle member.
    fn has_handle_member(&self, ty: &Type) -> bool {
        let members = match ty {
            Type::Union(members) => members,
            Type::Named(id) => match &self.protocol.def(*id).kind {
                DefKind::Union(members) => members,
                _ => return false,
            },
            _ => return false,
        };
        members
            .iter()
            .any(|member| self.handle_kind(&member.ty).is_some())
    }

    /// Whether `Runtime.getProperties` hands `ty` back as a value: a string, number, boolean or
    /// string enum.
    fn is_primitive(&self, ty: &Type) -> bool {
        match ty {
            Type::String | Type::Boolean | Type::Int | Type::Float | Type::Enum(_) => true,
            Type::Named(id) => matches!(self.protocol.def(*id).kind, DefKind::Enum(_)),
            _ => false,
        }
    }

    /// The interface of a channel reference Neo does not implement.
    fn unimplemented(&self, ty: &Type) -> Option<&str> {
        let Type::Channel(id) = ty else {
            return None;
        };
        match &self.side {
            Side::Serve { implemented } if !implemented.contains(id) => {
                Some(&self.protocol.def(*id).name)
            }
            Side::Serve { .. } | Side::Page => None,
        }
    }

    /// Reserves a type name, or reports where it was taken.
    fn claim(&mut self, name: &str, at: &At) -> Result<(), SpecError> {
        if !is_identifier(name) {
            return Err(at.error(format!(
                "would generate the type name \"{name}\", which is not an identifier"
            )));
        }
        if let Some(other) = self.names.get(name) {
            return Err(at.error(format!(
                "would generate the type `{name}`, already generated for {other}"
            )));
        }
        self.names.insert(name.to_owned(), at.describe());
        Ok(())
    }

    /// Settles what depends on the whole module: `Box` edges, `Eq`, and union `Copy`/`Hash`.
    fn finish(self) -> Vec<TypeDef> {
        let mut types = self.types;
        let edges = inline_edges(&types);
        let boxed: Vec<(usize, usize)> = edges
            .iter()
            .filter(|edge| reaches(&edges, types.len(), edge.target, edge.owner))
            .map(|edge| (edge.owner, edge.field))
            .collect();
        for (owner, field) in boxed {
            if let Body::Struct(fields) = &mut types[owner].body {
                let field = &mut fields.fields[field];
                if let FieldType::Ref(name) = &field.ty {
                    field.ty = FieldType::Boxed(name.clone());
                }
            }
        }
        let floats = reaches_float(&types);
        for (index, ty) in types.iter_mut().enumerate() {
            match &mut ty.body {
                Body::Struct(fields) => fields.eq = !floats[index],
                Body::Union(union) => {
                    union.eq = !floats[index];
                    union.copy_hash = union.members.iter().all(|member| {
                        matches!(member.kind, MemberKind::Bool | MemberKind::Enum(_))
                    });
                }
                Body::Enum(_) | Body::Channel(_) | Body::Handle => {}
            }
        }
        types
    }
}

/// A struct field that holds another struct or union inline, so the field's size includes the
/// target's. An array's items live on the heap, and a union's object members are always boxed,
/// so neither makes an edge.
struct InlineEdge {
    owner: usize,
    field: usize,
    target: usize,
}

fn inline_edges(types: &[TypeDef]) -> Vec<InlineEdge> {
    let mut edges = Vec::new();
    for (owner, ty) in types.iter().enumerate() {
        let Body::Struct(fields) = &ty.body else {
            continue;
        };
        for (field, entry) in fields.fields.iter().enumerate() {
            if let FieldType::Ref(name) = &entry.ty
                && let Some(target) = position(types, name)
                && matches!(types[target].body, Body::Struct(_) | Body::Union(_))
            {
                edges.push(InlineEdge {
                    owner,
                    field,
                    target,
                });
            }
        }
    }
    edges
}

/// Whether `to` is reachable from `from` through inline edges. An edge on such a cycle would
/// make the type infinitely large, so it is boxed.
fn reaches(edges: &[InlineEdge], len: usize, from: usize, to: usize) -> bool {
    let mut seen = vec![false; len];
    let mut stack = vec![from];
    while let Some(current) = stack.pop() {
        if current == to {
            return true;
        }
        if seen[current] {
            continue;
        }
        seen[current] = true;
        stack.extend(
            edges
                .iter()
                .filter(|edge| edge.owner == current)
                .map(|edge| edge.target),
        );
    }
    false
}

/// For each type, whether a `float` is reachable from it through any reference.
fn reaches_float(types: &[TypeDef]) -> Vec<bool> {
    let mut floats: Vec<bool> = types
        .iter()
        .map(|ty| match &ty.body {
            Body::Struct(fields) => fields.fields.iter().any(|field| holds_float(&field.ty)),
            Body::Enum(_) | Body::Union(_) | Body::Channel(_) | Body::Handle => false,
        })
        .collect();
    loop {
        let mut changed = false;
        for (index, ty) in types.iter().enumerate() {
            if floats[index] {
                continue;
            }
            let reached = references(ty)
                .iter()
                .any(|name| position(types, name).is_some_and(|target| floats[target]));
            if reached {
                floats[index] = true;
                changed = true;
            }
        }
        if !changed {
            return floats;
        }
    }
}

fn holds_float(ty: &FieldType) -> bool {
    match ty {
        FieldType::Float => true,
        FieldType::Array(items) => holds_float(items),
        _ => false,
    }
}

/// The names of the types `ty` refers to.
fn references(ty: &TypeDef) -> Vec<&str> {
    fn field_refs<'t>(ty: &'t FieldType, names: &mut Vec<&'t str>) {
        match ty {
            FieldType::Ref(name) | FieldType::Boxed(name) => names.push(name),
            FieldType::Array(items) => field_refs(items, names),
            _ => {}
        }
    }
    let mut names = Vec::new();
    match &ty.body {
        Body::Struct(fields) => {
            for field in &fields.fields {
                field_refs(&field.ty, &mut names);
            }
        }
        Body::Union(union) => {
            for member in &union.members {
                if let MemberKind::Enum(name) | MemberKind::Object(name) = &member.kind {
                    names.push(name);
                }
            }
        }
        Body::Enum(_) | Body::Channel(_) | Body::Handle => {}
    }
    names
}

fn position(types: &[TypeDef], name: &str) -> Option<usize> {
    types.iter().position(|ty| ty.name == name)
}

fn find<'t>(types: &'t [TypeDef], name: &str) -> Option<&'t Body> {
    types.iter().find(|ty| ty.name == name).map(|ty| &ty.body)
}

/// Every served params, result, initializer and listed type must be constructible: a required
/// reference to an interface Neo does not implement would make it uninhabited.
fn check_inhabited(
    protocol: &Protocol,
    selection: &Selection<'_>,
    types: &[TypeDef],
) -> Result<(), SpecError> {
    let blocked = |name: &str| blocker(types, name, &mut Vec::new());
    for interface in &selection.interfaces {
        let def = protocol.def(interface.id);
        if let Some(missing) = blocked(&format!("{}Initializer", def.name)) {
            return Err(uninhabited(
                &interface.at,
                &format!("a `{}` object's initializer", def.name),
                &missing,
            ));
        }
        for (command, at) in &interface.commands {
            let stem = format!("{}{}", def.name, title_case(&command.name));
            for (suffix, what) in [("Params", "params"), ("Result", "result")] {
                if let Some(missing) = blocked(&format!("{stem}{suffix}")) {
                    let what = format!("the {what} of `{}.{}`", def.name, command.name);
                    return Err(uninhabited(at, &what, &missing));
                }
            }
        }
    }
    for (id, at) in &selection.types {
        let name = &protocol.def(*id).name;
        if let Some(missing) = blocked(name) {
            return Err(uninhabited(at, &format!("`{name}`"), &missing));
        }
    }
    Ok(())
}

fn uninhabited(at: &At, what: &str, interface: &str) -> SpecError {
    at.error(format!(
        "{what} requires a `{interface}` object, which Neo cannot create; add `{interface}: []` to \
         `interfaces`, with the interfaces its initializer requires"
    ))
}

/// The unimplemented interface that a required field of `name` needs, if any.
fn blocker(types: &[TypeDef], name: &str, seen: &mut Vec<String>) -> Option<String> {
    if seen.iter().any(|visited| visited == name) {
        return None;
    }
    seen.push(name.to_owned());
    match find(types, name)? {
        Body::Channel(channel) if !channel.implemented => Some(channel.interface.clone()),
        Body::Struct(fields) => fields
            .fields
            .iter()
            .filter(|field| !field.optional)
            .find_map(|field| match &field.ty {
                FieldType::Ref(target) | FieldType::Boxed(target) => blocker(types, target, seen),
                _ => None,
            }),
        Body::Channel(_) | Body::Enum(_) | Body::Union(_) | Body::Handle => None,
    }
}

fn entry<'n>(entries: &'n [(String, Node)], key: &str, at: &At) -> Result<&'n Node, SpecError> {
    entries
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, node)| node)
        .ok_or_else(|| at.error(format!("needs a `{key}` key")))
}

/// Docs from texts that may each span lines, as paragraphs; empty texts are skipped.
fn paragraphs(texts: &[&str]) -> Vec<String> {
    let mut doc = Vec::new();
    for text in texts.iter().filter(|text| !text.is_empty()) {
        if !doc.is_empty() {
            doc.push(String::new());
        }
        doc.extend(text.lines().map(str::to_owned));
    }
    doc
}

/// Playwright's `titleCase`: the first character uppercased (`textContent` -> `TextContent`).
pub fn title_case(name: &str) -> String {
    let mut chars = name.chars();
    chars.next().map_or_else(String::new, |first| {
        format!("{}{}", first.to_ascii_uppercase(), chars.as_str())
    })
}

/// The Rust field identifier for a JSON key: snake case that keeps acronyms whole (`baseURL` ->
/// `base_url`, `ignoreHTTPSErrors` -> `ignore_https_errors`), separators and leading
/// underscores folded, keywords raw (`type` -> `r#type`), and the four keywords that cannot be
/// raw suffixed (`self` -> `self_`). `None` when no identifier results.
pub fn field_ident(wire: &str) -> Option<String> {
    let chars: Vec<char> = wire.chars().collect();
    let mut snake = String::new();
    for (index, &current) in chars.iter().enumerate() {
        if current.is_ascii_uppercase() {
            let previous = index.checked_sub(1).map(|before| chars[before]);
            let next = chars.get(index + 1);
            let starts_word = previous.is_some_and(|previous| {
                previous.is_ascii_lowercase()
                    || previous.is_ascii_digit()
                    || (previous.is_ascii_uppercase() && next.is_some_and(char::is_ascii_lowercase))
            });
            if starts_word && !snake.ends_with('_') {
                snake.push('_');
            }
            snake.push(current.to_ascii_lowercase());
        } else if current == '_' || current == '-' {
            if !snake.is_empty() && !snake.ends_with('_') {
                snake.push('_');
            }
        } else {
            snake.push(current);
        }
    }
    let snake = snake.trim_end_matches('_').to_owned();
    if !is_identifier(&snake) {
        return None;
    }
    Some(match snake.as_str() {
        "self" | "crate" | "super" => format!("{snake}_"),
        keyword if is_keyword(keyword) => format!("r#{snake}"),
        _ => snake,
    })
}

/// The Rust variant identifier for a wire string: words split at any non-alphanumeric
/// character and capitalized, with a leading `-` spelled `Minus` (`-Infinity` ->
/// `MinusInfinity`, `-0` -> `Minus0`, `no-preference` -> `NoPreference`). `None` when no
/// identifier results.
pub fn variant_ident(literal: &str) -> Option<String> {
    let (prefix, rest) = match literal.strip_prefix('-') {
        Some(rest) => ("Minus", rest),
        None => ("", literal),
    };
    let mut ident = String::from(prefix);
    for word in rest
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
    {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            ident.push(first.to_ascii_uppercase());
            ident.push_str(chars.as_str());
        }
    }
    (is_identifier(&ident) && ident != "Self").then_some(ident)
}

/// An ASCII identifier that Rust and TypeScript both accept.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && name != "_"
}

/// Rust 2024's strict and reserved keywords.
fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        "abstract"
            | "as"
            | "async"
            | "await"
            | "become"
            | "box"
            | "break"
            | "const"
            | "continue"
            | "do"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "final"
            | "fn"
            | "for"
            | "gen"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "macro"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "override"
            | "priv"
            | "pub"
            | "ref"
            | "return"
            | "static"
            | "struct"
            | "trait"
            | "true"
            | "try"
            | "type"
            | "typeof"
            | "unsafe"
            | "unsized"
            | "use"
            | "virtual"
            | "where"
            | "while"
            | "yield"
    )
}
