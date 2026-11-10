//! Renders the contract as Rust: `src/serve.rs`, `src/injected.rs` and `src/spec_hash.rs`.
//!
//! `src/lib.rs` mounts these files `#[rustfmt::skip]`, so this printer owns their layout and no
//! rustfmt version can change what `--check` compares. It writes what rustfmt would anyway:
//! four-space indents, one attribute per line, trailing commas. The output must pass the
//! crate's whole clippy table with no exception for generated code, which is why every name is
//! imported (`absolute_paths`), imports are exactly what the file uses, and the output carries
//! no lint attributes, whose names drift between toolchains.

use crate::contract::{
    Body, Channel, Crossing, FieldType, HandleKindRef, InjectedModule, MemberKind, PageCall,
    ServeModule, ServedInterface, Struct, TypeDef, Union, Variant,
};
use crate::dialect::{Flag, Group, Meta};
use crate::output::{self, MAX_WIDTH};

/// Renders `src/serve.rs`.
pub fn serve(module: &ServeModule, sha256: &str) -> String {
    let mut out = output::header(sha256);
    out.push(String::new());
    for line in [
        "//! The Playwright protocol commands Neo serves (`spec/serve.yml`), with params and result",
        "//! types taken unchanged from Playwright's protocol spec (`spec/playwright`, Apache-2.0).",
        "//! Optional fields are absent on the wire, never `null`.",
    ] {
        out.push(line.to_owned());
    }

    let commands = module
        .interfaces
        .iter()
        .flat_map(|interface| &interface.commands);
    let mut call_items = vec!["CallDecodeError", "Command"];
    if commands
        .clone()
        .any(|command| !command.meta.flags.is_empty())
    {
        call_items.push("MethodFlag");
    }
    if commands.clone().any(|command| command.meta.group.is_some()) {
        call_items.push("MethodGroup");
    }
    call_items.extend(["MethodMeta", "Sealed"]);
    let mut imports = vec!["use serde::{Deserialize, Serialize};".to_owned()];
    if !module.interfaces.is_empty() || uses_json(&module.types) {
        imports.push("use serde_json::Value;".to_owned());
    }
    out.push(String::new());
    out.extend(imports);
    if !module.interfaces.is_empty() {
        out.push(String::new());
        if uses_binary(&module.types) {
            out.push("use crate::Binary;".to_owned());
        }
        out.push(format!("use crate::call::{{{}}};", call_items.join(", ")));
    }

    for interface in &module.interfaces {
        out.push(String::new());
        call_enum(&mut out, interface);
    }
    for ty in &module.types {
        out.push(String::new());
        type_def(&mut out, ty, &module.types);
        let served = module.interfaces.iter().flat_map(|interface| {
            interface
                .commands
                .iter()
                .map(move |command| (interface, command))
        });
        for (interface, command) in served.filter(|(_, command)| command.params == ty.name) {
            out.push(String::new());
            out.push(format!("impl Sealed for {} {{}}", command.params));
            out.push(String::new());
            out.push(format!("impl Command for {} {{", command.params));
            out.push(format!("    type Result = {};", command.result));
            meta(&mut out, &interface.name, &command.method, &command.meta);
            out.push(format!(
                "    const VOID_RESULT: bool = {};",
                command.void_result
            ));
            out.push("}".to_owned());
        }
    }
    lines(&out)
}

/// Renders `src/injected.rs`.
pub fn injected(module: &InjectedModule, sha256: &str) -> String {
    let mut out = output::header(sha256);
    out.push(String::new());
    for line in [
        "//! The page calls `neo-runtime` makes into `neo-playwright-injected`'s glue table",
        "//! (`spec/injected.yml`), with exactly what the page returns. A result that holds no",
        "//! handle crosses as JSON and rejects unknown fields on decode; one that holds a handle is",
        "//! read property by property and rejects unknown properties. The page bundle and this",
        "//! crate are built from one spec, so anything extra means they drifted apart.",
    ] {
        out.push(line.to_owned());
    }

    let json_unions: Vec<&Union> = module
        .types
        .iter()
        .filter_map(|ty| match &ty.body {
            Body::Union(union) if union.crossing == Crossing::Json => Some(union),
            _ => None,
        })
        .collect();
    let has_member = |wanted: fn(&MemberKind) -> bool| {
        json_unions
            .iter()
            .any(|union| union.members.iter().any(|member| wanted(&member.kind)))
    };
    let objects = has_member(|kind| matches!(kind, MemberKind::Object(_)));
    let enums = has_member(|kind| matches!(kind, MemberKind::Enum(_)));
    out.push(String::new());
    if !json_unions.is_empty() {
        out.push("use std::fmt;".to_owned());
        out.push(String::new());
        if objects {
            out.push("use serde::de::value::MapAccessDeserializer;".to_owned());
        }
        let mut de = vec!["self", "Deserializer"];
        if enums {
            de.push("IntoDeserializer");
        }
        if objects {
            de.push("MapAccess");
        }
        de.push("Visitor");
        out.push(format!("use serde::de::{{{}}};", de.join(", ")));
    }
    out.push("use serde::{Deserialize, Serialize};".to_owned());
    let property_unions = module
        .types
        .iter()
        .any(|ty| matches!(&ty.body, Body::Union(union) if union.crossing == Crossing::Properties));
    if !module.calls.is_empty() || property_unions || uses_json(&module.types) {
        out.push("use serde_json::Value;".to_owned());
    }
    let call_items = call_imports(module);
    if !call_items.is_empty() {
        out.push(String::new());
        use_list(&mut out, "crate::call", &call_items);
    }

    for ty in &module.types {
        out.push(String::new());
        type_def(&mut out, ty, &module.types);
        for call in module.calls.iter().filter(|call| call.params == ty.name) {
            injected_call(&mut out, call, ty);
        }
        for call in module.calls.iter().filter(|call| call.result == ty.name) {
            page_result(&mut out, call, ty, &module.types);
        }
    }
    lines(&out)
}

/// The names `src/injected.rs` imports from `crate::call`: exactly those it uses, because an
/// unused import fails clippy.
fn call_imports(module: &InjectedModule) -> Vec<&'static str> {
    let calls = !module.calls.is_empty();
    let mut kinds: Vec<&HandleKindRef> = Vec::new();
    let mut handle_defs = false;
    let mut property_unions = false;
    let mut primitive_properties = false;
    let mut json_results = false;
    for ty in &module.types {
        match &ty.body {
            Body::Struct(fields) => {
                for field in &fields.fields {
                    if let FieldType::Handle(kind) = &field.ty {
                        kinds.push(kind);
                    }
                }
                let result = module.calls.iter().any(|call| call.result == ty.name);
                match fields.crossing {
                    Crossing::Json => json_results |= result,
                    Crossing::Properties => {
                        primitive_properties |= fields.fields.iter().any(|field| {
                            !matches!(field.ty, FieldType::Handle(_))
                                && !is_property_union(&field.ty, &module.types)
                        });
                    }
                    Crossing::Arguments => {}
                }
            }
            Body::Union(union) => {
                for member in &union.members {
                    if let MemberKind::Handle(kind) = &member.kind {
                        kinds.push(kind);
                    }
                }
                property_unions |= union.crossing == Crossing::Properties;
            }
            Body::Handle => handle_defs = true,
            Body::Enum(_) | Body::Channel(_) => {}
        }
    }
    let mut items = Vec::new();
    if kinds.iter().any(|kind| **kind == HandleKindRef::Element) {
        items.push("Element");
    }
    if !kinds.is_empty() {
        items.push("Handle");
    }
    if calls {
        items.push("HandleId");
    }
    if handle_defs {
        items.push("HandleKind");
    }
    if calls {
        items.extend(["InjectedCall", "PageResult", "Properties"]);
    }
    if property_unions {
        items.push("Property");
    }
    if calls {
        items.push("ResultDecodeError");
    }
    if calls || handle_defs {
        items.push("Sealed");
    }
    if json_results {
        items.push("decode_json");
    }
    if primitive_properties || property_unions {
        items.push("decode_property");
    }
    if calls {
        items.push("wrong_crossing");
    }
    items
}

/// `use path::{items};`, wrapped as rustfmt wraps it when it is too long for one line.
fn use_list(out: &mut Vec<String>, path: &str, items: &[&str]) {
    let single = format!("use {path}::{{{}}};", items.join(", "));
    if single.len() <= MAX_WIDTH {
        out.push(single);
        return;
    }
    out.push(format!("use {path}::{{"));
    let mut line = String::from("   ");
    for item in items {
        if line.len() + 1 + item.len() + 1 > MAX_WIDTH {
            out.push(line);
            line = String::from("   ");
        }
        line.push(' ');
        line.push_str(item);
        line.push(',');
    }
    out.push(line);
    out.push("};".to_owned());
}

/// `impl InjectedCall` for a page call's params struct.
fn injected_call(out: &mut Vec<String>, call: &PageCall, params: &TypeDef) {
    let handles: Vec<String> = match &params.body {
        Body::Struct(fields) => fields
            .fields
            .iter()
            .filter(|field| matches!(field.ty, FieldType::Handle(_)))
            .map(|field| format!("self.{}.id()", field.ident))
            .collect(),
        _ => Vec::new(),
    };
    let handles = if handles.is_empty() {
        "Vec::new()".to_owned()
    } else {
        format!("vec![{}]", handles.join(", "))
    };
    for line in [
        String::new(),
        format!("impl Sealed for {} {{}}", call.params),
        String::new(),
        format!("impl InjectedCall for {} {{", call.params),
        format!("    const NAME: &'static str = {:?};", call.name),
        format!("    type Result = {};", call.result),
        String::new(),
        "    fn handles(&self) -> Vec<&HandleId> {".to_owned(),
        format!("        {handles}"),
        "    }".to_owned(),
        "}".to_owned(),
    ] {
        out.push(line);
    }
}

/// `impl PageResult` for a page call's result struct: a serde decode for a by-value result, or a
/// decode property by property for one that holds a handle.
fn page_result(out: &mut Vec<String>, call: &PageCall, result: &TypeDef, types: &[TypeDef]) {
    let Body::Struct(fields) = &result.body else {
        return;
    };
    let name = &call.name;
    out.push(String::new());
    out.push(format!("impl Sealed for {} {{}}", call.result));
    out.push(String::new());
    out.push(format!("impl PageResult for {} {{", call.result));
    if fields.crossing == Crossing::Properties {
        for line in [
            "    const BY_VALUE: bool = false;".to_owned(),
            String::new(),
            "    fn from_value(_: Value) -> Result<Self, ResultDecodeError> {".to_owned(),
            format!("        Err(wrong_crossing({name:?}, false))"),
            "    }".to_owned(),
            String::new(),
            "    fn from_properties(mut properties: Properties) -> Result<Self, ResultDecodeError> {"
                .to_owned(),
            "        let result = Self {".to_owned(),
        ] {
            out.push(line);
        }
        for field in &fields.fields {
            let take = if field.optional {
                "optional"
            } else {
                "required"
            };
            let decode = match &field.ty {
                FieldType::Handle(_) => "Handle::from_property".to_owned(),
                FieldType::Ref(union) if is_property_union(&field.ty, types) => {
                    format!("{union}::from_property")
                }
                _ => "decode_property".to_owned(),
            };
            let args = [format!("{name:?}"), format!("{:?}", field.wire), decode];
            let single = format!(
                "            {}: properties.{take}({})?,",
                field.ident,
                args.join(", ")
            );
            if single.len() <= MAX_WIDTH {
                out.push(single);
            } else {
                out.push(format!("            {}: properties.{take}(", field.ident));
                for arg in &args {
                    out.push(format!("                {arg},"));
                }
                out.push("            )?,".to_owned());
            }
        }
        for line in [
            "        };".to_owned(),
            format!("        properties.finish({name:?})?;"),
            "        Ok(result)".to_owned(),
            "    }".to_owned(),
        ] {
            out.push(line);
        }
    } else {
        for line in [
            "    const BY_VALUE: bool = true;".to_owned(),
            String::new(),
            "    fn from_value(value: Value) -> Result<Self, ResultDecodeError> {".to_owned(),
            format!("        decode_json({name:?}, value)"),
            "    }".to_owned(),
            String::new(),
            "    fn from_properties(_: Properties) -> Result<Self, ResultDecodeError> {".to_owned(),
            format!("        Err(wrong_crossing({name:?}, true))"),
            "    }".to_owned(),
        ] {
            out.push(line);
        }
    }
    out.push("}".to_owned());
}

/// Whether `ty` names a union with a handle member, which decodes from one property.
fn is_property_union(ty: &FieldType, types: &[TypeDef]) -> bool {
    let FieldType::Ref(name) = ty else {
        return false;
    };
    types.iter().any(|ty| {
        ty.name == *name
            && matches!(&ty.body, Body::Union(union) if union.crossing == Crossing::Properties)
    })
}

/// Renders `src/spec_hash.rs`.
pub fn spec_hash(sha256: &str) -> String {
    let mut out = output::header(sha256);
    for line in [
        "",
        "/// SHA-256 of the spec these types were translated from (`codegen/spec_files.rs` defines",
        "/// the framing). `ts/spec-hash.ts` exports the same value, so `neo-runtime` can check that",
        "/// the page bundle it embeds was built from the same spec.",
    ] {
        out.push(line.to_owned());
    }
    out.push(format!("pub const SPEC_SHA256: &str = {sha256:?};"));
    lines(&out)
}

/// The call enum of one interface, with `decode` and `meta`.
fn call_enum(out: &mut Vec<String>, interface: &ServedInterface) {
    let name = &interface.name;
    let call = &interface.call_enum;
    for line in [
        format!(
            "/// A call on a `{name}` object: one command Neo serves on `{name}`, with its decoded params."
        ),
        "///".to_owned(),
        format!(
            "/// Serving another `{name}` command in `spec/serve.yml` adds a variant, so `neo-runtime`'s"
        ),
        "/// exhaustive `match` stops compiling until the command has a handler.".to_owned(),
        format!(
            "#[derive(Clone, Debug, PartialEq{})]",
            if interface.eq { ", Eq" } else { "" }
        ),
        format!("pub enum {call} {{"),
    ] {
        out.push(line);
    }
    for command in &interface.commands {
        out.push(format!("    /// `{name}.{}`.", command.method));
        out.push(format!("    {}({}),", command.variant, command.params));
    }
    out.push("}".to_owned());
    out.push(String::new());
    for line in [
        format!("impl {call} {{"),
        format!("    /// Decodes a call from the `method` and `params` of a protocol message to a `{name}`."),
        "    ///".to_owned(),
        "    /// Absent params decode as `{}`, which Playwright's client sends as nothing for a command".to_owned(),
        "    /// that declares none.".to_owned(),
        "    ///".to_owned(),
        "    /// # Errors".to_owned(),
        "    ///".to_owned(),
        format!("    /// [`CallDecodeError::UnknownMethod`] when Neo does not serve `method` on `{name}`, and"),
        "    /// [`CallDecodeError::InvalidParams`] when `params` does not fit the command.".to_owned(),
        "    pub fn decode(method: &str, params: Option<Value>) -> Result<Self, CallDecodeError> {"
            .to_owned(),
        "        match method {".to_owned(),
    ] {
        out.push(line);
    }
    for command in &interface.commands {
        out.push(format!(
            "            {:?} => CallDecodeError::params(params).map(Self::{}),",
            command.method, command.variant
        ));
    }
    for line in [
        "            _ => Err(CallDecodeError::UnknownMethod {".to_owned(),
        format!("                interface: {name:?},"),
        "                method: method.to_owned(),".to_owned(),
        "            }),".to_owned(),
        "        }".to_owned(),
        "    }".to_owned(),
        String::new(),
        "    /// Playwright's metainfo for this call's command.".to_owned(),
        "    #[must_use]".to_owned(),
        "    pub fn meta(&self) -> &'static MethodMeta {".to_owned(),
        "        match self {".to_owned(),
    ] {
        out.push(line);
    }
    for command in &interface.commands {
        out.push(format!(
            "            Self::{}(_) => {}::META,",
            command.variant, command.params
        ));
    }
    out.push("        }".to_owned());
    out.push("    }".to_owned());
    out.push("}".to_owned());
}

/// `const META` of a served command.
fn meta(out: &mut Vec<String>, interface: &str, method: &str, meta: &Meta) {
    let text = |text: &Option<String>| {
        text.as_ref()
            .map_or_else(|| "None".to_owned(), |text| format!("Some({text:?})"))
    };
    let group = meta.group.map_or_else(
        || "None".to_owned(),
        |group| {
            let variant = match group {
                Group::Getter => "Getter",
                Group::Configuration => "Configuration",
                Group::Route => "Route",
                Group::Default => "Default",
            };
            format!("Some(MethodGroup::{variant})")
        },
    );
    let render_params: Vec<String> = meta
        .render_params
        .iter()
        .map(|param| format!("{param:?}"))
        .collect();
    let flags: Vec<String> = meta
        .flags
        .iter()
        .map(|flag| {
            let variant = match flag {
                Flag::SlowMo => "SlowMo",
                Flag::Snapshot => "Snapshot",
                Flag::Pause => "Pause",
                Flag::Input => "Input",
                Flag::IsAutoWaiting => "IsAutoWaiting",
            };
            format!("MethodFlag::{variant}")
        })
        .collect();
    for line in [
        "    const META: &'static MethodMeta = &MethodMeta {".to_owned(),
        format!("        interface: {interface:?},"),
        format!("        method: {method:?},"),
        format!("        title: {},", text(&meta.title)),
        format!("        subtitle: {},", text(&meta.subtitle)),
        format!("        group: {group},"),
        format!("        render_params: &[{}],", render_params.join(", ")),
        format!("        internal: {},", meta.internal),
        format!("        flags: &[{}],", flags.join(", ")),
        "    };".to_owned(),
    ] {
        out.push(line);
    }
}

fn type_def(out: &mut Vec<String>, ty: &TypeDef, types: &[TypeDef]) {
    doc(out, "", &ty.doc);
    match &ty.body {
        Body::Struct(fields) => struct_def(out, &ty.name, fields),
        Body::Enum(variants) => enum_def(out, &ty.name, variants),
        Body::Union(union) => union_def(out, &ty.name, union, types),
        Body::Channel(channel) => channel_def(out, &ty.name, channel),
        Body::Handle => handle_def(out, &ty.name),
    }
}

fn struct_def(out: &mut Vec<String>, name: &str, fields: &Struct) {
    let mut derives = vec!["Clone", "Debug", "PartialEq"];
    if fields.eq {
        derives.push("Eq");
    }
    match fields.crossing {
        Crossing::Json => derives.extend(["Serialize", "Deserialize"]),
        // A handle cannot come from JSON, so these params are only ever encoded.
        Crossing::Arguments => derives.push("Serialize"),
        Crossing::Properties => {}
    }
    out.push(format!("#[derive({})]", derives.join(", ")));
    if fields.deny_unknown_fields {
        out.push("#[serde(deny_unknown_fields)]".to_owned());
    }
    if fields.fields.is_empty() {
        out.push(format!("pub struct {name} {{}}"));
        return;
    }
    out.push(format!("pub struct {name} {{"));
    for field in &fields.fields {
        doc(out, "    ", &field.doc);
        let ty = rust_type(&field.ty);
        if fields.crossing == Crossing::Properties {
            if field.optional {
                out.push(format!("    pub {}: Option<{ty}>,", field.ident));
            } else {
                out.push(format!("    pub {}: {ty},", field.ident));
            }
        } else if matches!(field.ty, FieldType::Handle(_)) {
            out.push("    #[serde(skip)]".to_owned());
            out.push(format!("    pub {}: {ty},", field.ident));
        } else if field.optional {
            out.push(format!(
                "    #[serde(rename = {:?}, skip_serializing_if = \"Option::is_none\")]",
                field.wire
            ));
            out.push(format!("    pub {}: Option<{ty}>,", field.ident));
        } else {
            out.push(format!("    #[serde(rename = {:?})]", field.wire));
            out.push(format!("    pub {}: {ty},", field.ident));
        }
    }
    out.push("}".to_owned());
}

fn enum_def(out: &mut Vec<String>, name: &str, variants: &[Variant]) {
    out.push(
        "#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]".to_owned(),
    );
    out.push(format!("pub enum {name} {{"));
    for variant in variants {
        out.push(format!("    #[serde(rename = {:?})]", variant.literal));
        out.push(format!("    {},", variant.ident));
    }
    out.push("}".to_owned());
}

fn channel_def(out: &mut Vec<String>, name: &str, channel: &Channel) {
    if channel.implemented {
        out.push("#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]".to_owned());
        out.push(format!("pub struct {name} {{"));
        out.push("    #[serde(rename = \"guid\")]".to_owned());
        out.push("    pub guid: String,".to_owned());
        out.push("}".to_owned());
    } else {
        out.push(
            "#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]".to_owned(),
        );
        out.push(format!("pub enum {name} {{}}"));
    }
}

/// A `type: handle` definition: an uninhabited marker that only names a kind of page object.
fn handle_def(out: &mut Vec<String>, name: &str) {
    for line in [
        "#[derive(Debug)]".to_owned(),
        format!("pub enum {name} {{}}"),
        String::new(),
        format!("impl Sealed for {name} {{}}"),
        String::new(),
        format!("impl HandleKind for {name} {{"),
        format!("    const NAME: &'static str = {name:?};"),
        "    // A plain JavaScript object, for which CDP reports no `RemoteObject.subtype`."
            .to_owned(),
        "    const SUBTYPE: Option<&'static str> = None;".to_owned(),
        "}".to_owned(),
    ] {
        out.push(line);
    }
}

/// An untagged union: `Serialize` writes the member's own JSON, and a generated visitor decodes
/// it in one pass by JSON kind. A union with a handle member instead decodes from one property
/// of a page result.
fn union_def(out: &mut Vec<String>, name: &str, union: &Union, types: &[TypeDef]) {
    if union.crossing == Crossing::Properties {
        property_union_def(out, name, union, types);
        return;
    }
    let mut derives = vec!["Clone"];
    if union.copy_hash {
        derives.push("Copy");
    }
    derives.extend(["Debug", "PartialEq"]);
    if union.eq {
        derives.push("Eq");
    }
    if union.copy_hash {
        derives.push("Hash");
    }
    derives.push("Serialize");
    out.push(format!("#[derive({})]", derives.join(", ")));
    out.push("#[serde(untagged)]".to_owned());
    out.push(format!("pub enum {name} {{"));
    for member in &union.members {
        doc(out, "    ", &member.doc);
        let payload = match &member.kind {
            MemberKind::Bool => "bool".to_owned(),
            MemberKind::String => "String".to_owned(),
            MemberKind::Enum(enum_name) => enum_name.clone(),
            // Boxed: an object member would make every value as large as the object.
            MemberKind::Object(object) => format!("Box<{object}>"),
            MemberKind::Handle(kind) => handle_type(kind),
        };
        out.push(format!("    {}({payload}),", member.ident));
    }
    out.push("}".to_owned());
    union_visitor(out, name, union, types);
}

/// A union with a handle member. `Runtime.getProperties` hands the handle back as a page object
/// and every other member as a value, told apart by JSON kind as in a JSON union.
fn property_union_def(out: &mut Vec<String>, name: &str, union: &Union, types: &[TypeDef]) {
    let mut derives = vec!["Clone", "Debug", "PartialEq"];
    if union.eq {
        derives.push("Eq");
    }
    out.push(format!("#[derive({})]", derives.join(", ")));
    out.push(format!("pub enum {name} {{"));
    for member in &union.members {
        doc(out, "    ", &member.doc);
        let payload = match &member.kind {
            MemberKind::Bool => "bool".to_owned(),
            MemberKind::String => "String".to_owned(),
            MemberKind::Enum(enum_name) => enum_name.clone(),
            MemberKind::Object(object) => format!("Box<{object}>"),
            MemberKind::Handle(kind) => handle_type(kind),
        };
        out.push(format!("    {}({payload}),", member.ident));
    }
    out.push("}".to_owned());
    for line in [
        String::new(),
        format!("impl {name} {{"),
        "    /// Decodes the union from one property of a page result.".to_owned(),
        "    fn from_property(property: Property) -> Result<Self, String> {".to_owned(),
        "        match property {".to_owned(),
    ] {
        out.push(line);
    }
    for member in &union.members {
        let variant = &member.ident;
        let arm = match &member.kind {
            MemberKind::Handle(_) => {
                format!(
                    "Property::Object(..) => Handle::from_property(property).map(Self::{variant}),"
                )
            }
            MemberKind::String | MemberKind::Enum(_) => format!(
                "Property::Value(Value::String(_)) => decode_property(property).map(Self::{variant}),"
            ),
            MemberKind::Bool => format!(
                "Property::Value(Value::Bool(_)) => decode_property(property).map(Self::{variant}),"
            ),
            // The dialect gives a handle member the object kind, so no object member shares its
            // union.
            MemberKind::Object(_) => continue,
        };
        out.push(format!("            {arm}"));
    }
    let expected = expecting(union, types)
        .replace('{', "{{")
        .replace('}', "}}");
    let message = format!("the value {{value}}, not {expected}");
    out.push(format!(
        "            Property::Value(value) => Err(format!({message:?})),"
    ));
    out.push("        }".to_owned());
    out.push("    }".to_owned());
    out.push("}".to_owned());
}

/// `Deserialize` for a union, through a visitor that picks the member by JSON kind.
fn union_visitor(out: &mut Vec<String>, name: &str, union: &Union, types: &[TypeDef]) {
    let visitor = format!("{name}Visitor");
    let objects = union
        .members
        .iter()
        .any(|member| matches!(member.kind, MemberKind::Object(_)));
    let expecting = expecting(union, types);

    for line in [
        String::new(),
        format!("impl<'de> Deserialize<'de> for {name} {{"),
        "    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {"
            .to_owned(),
        format!("        deserializer.deserialize_any({visitor})"),
        "    }".to_owned(),
        "}".to_owned(),
        String::new(),
        format!(
            "/// Decodes [`{name}`] in one pass by JSON kind. `serde(untagged)` would buffer every"
        ),
        "/// value before trying each member, at every level of a nested tree.".to_owned(),
        format!("struct {visitor};"),
        String::new(),
        if objects {
            format!("impl<'de> Visitor<'de> for {visitor} {{")
        } else {
            format!("impl Visitor<'_> for {visitor} {{")
        },
        format!("    type Value = {name};"),
        String::new(),
        "    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {".to_owned(),
        format!("        formatter.write_str({expecting:?})"),
        "    }".to_owned(),
    ] {
        out.push(line);
    }
    for member in &union.members {
        let variant = &member.ident;
        out.push(String::new());
        match &member.kind {
            MemberKind::Bool => {
                out.push(format!(
                    "    fn visit_bool<E: de::Error>(self, value: bool) -> Result<{name}, E> {{"
                ));
                out.push(format!("        Ok({name}::{variant}(value))"));
                out.push("    }".to_owned());
            }
            MemberKind::String => {
                out.push(format!(
                    "    fn visit_str<E: de::Error>(self, value: &str) -> Result<{name}, E> {{"
                ));
                out.push(format!("        Ok({name}::{variant}(value.to_owned()))"));
                out.push("    }".to_owned());
                out.push(String::new());
                out.push(format!(
                    "    fn visit_string<E: de::Error>(self, value: String) -> Result<{name}, E> {{"
                ));
                out.push(format!("        Ok({name}::{variant}(value))"));
                out.push("    }".to_owned());
            }
            MemberKind::Enum(enum_name) => {
                out.push(format!(
                    "    fn visit_str<E: de::Error>(self, value: &str) -> Result<{name}, E> {{"
                ));
                out.push(format!("        {enum_name}::deserialize(value.into_deserializer()).map({name}::{variant})"));
                out.push("    }".to_owned());
            }
            MemberKind::Object(object) => {
                out.push(format!("    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<{name}, A::Error> {{"));
                out.push(format!(
                    "        let value = {object}::deserialize(MapAccessDeserializer::new(map))?;"
                ));
                out.push(format!("        Ok({name}::{variant}(Box::new(value)))"));
                out.push("    }".to_owned());
            }
            // A union with a handle member never crosses as JSON (`property_union_def`).
            MemberKind::Handle(_) => {}
        }
    }
    out.push("}".to_owned());
}

/// What a union's decode error says it expected, such as
/// `a boolean or the string "mixed"`.
fn expecting(union: &Union, types: &[TypeDef]) -> String {
    let kinds: Vec<String> = union
        .members
        .iter()
        .map(|member| match &member.kind {
            MemberKind::Bool => "a boolean".to_owned(),
            MemberKind::String => "a string".to_owned(),
            MemberKind::Enum(enum_name) => match types
                .iter()
                .find(|ty| &ty.name == enum_name)
                .map(|ty| &ty.body)
            {
                Some(Body::Enum(variants)) if variants.len() == 1 => {
                    format!("the string \"{}\"", variants[0].literal)
                }
                Some(Body::Enum(variants)) => {
                    let literals: Vec<String> = variants
                        .iter()
                        .map(|variant| format!("\"{}\"", variant.literal))
                        .collect();
                    format!("one of {}", literals.join(", "))
                }
                _ => "a string".to_owned(),
            },
            MemberKind::Object(object) => {
                let vowel = object.starts_with(['A', 'E', 'I', 'O', 'U']);
                format!("{} {object} object", if vowel { "an" } else { "a" })
            }
            MemberKind::Handle(kind) => {
                let name = match kind {
                    HandleKindRef::Element => "Element",
                    HandleKindRef::Named(name) => name,
                };
                format!("a `{name}` page object")
            }
        })
        .collect();
    match kinds.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        _ => kinds.concat(),
    }
}

fn rust_type(ty: &FieldType) -> String {
    match ty {
        FieldType::String => "String".to_owned(),
        FieldType::Base64 => "Binary".to_owned(),
        FieldType::Bool => "bool".to_owned(),
        FieldType::Int => "i64".to_owned(),
        FieldType::Float => "f64".to_owned(),
        FieldType::Json => "Value".to_owned(),
        FieldType::Ref(name) => name.clone(),
        FieldType::Boxed(name) => format!("Box<{name}>"),
        FieldType::Array(items) => format!("Vec<{}>", rust_type(items)),
        FieldType::Handle(kind) => handle_type(kind),
    }
}

fn handle_type(kind: &HandleKindRef) -> String {
    match kind {
        HandleKindRef::Element => "Handle<Element>".to_owned(),
        HandleKindRef::Named(name) => format!("Handle<{name}>"),
    }
}

/// Whether a struct field anywhere is `binary`, which `serve.rs` then imports `Binary` for.
fn uses_binary(types: &[TypeDef]) -> bool {
    fn binary(ty: &FieldType) -> bool {
        match ty {
            FieldType::Base64 => true,
            FieldType::Array(items) => binary(items),
            _ => false,
        }
    }
    types.iter().any(|ty| match &ty.body {
        Body::Struct(fields) => fields.fields.iter().any(|field| binary(&field.ty)),
        Body::Enum(_) | Body::Union(_) | Body::Channel(_) | Body::Handle => false,
    })
}

fn uses_json(types: &[TypeDef]) -> bool {
    fn json(ty: &FieldType) -> bool {
        match ty {
            FieldType::Json => true,
            FieldType::Array(items) => json(items),
            _ => false,
        }
    }
    types.iter().any(|ty| match &ty.body {
        Body::Struct(fields) => fields.fields.iter().any(|field| json(&field.ty)),
        Body::Enum(_) | Body::Union(_) | Body::Channel(_) | Body::Handle => false,
    })
}

fn doc(out: &mut Vec<String>, indent: &str, lines: &[String]) {
    let width = output::MAX_WIDTH - indent.len() - "/// ".len();
    for line in lines {
        if line.is_empty() {
            out.push(format!("{indent}///"));
        }
        let line = angle_urls(line);
        for part in output::wrap(&line, width)
            .iter()
            .filter(|part| !part.is_empty())
        {
            out.push(format!("{indent}/// {part}"));
        }
    }
}

/// `line` with each bare `http(s)://` word in angle brackets, as rustdoc links it and clippy's
/// `doc_markdown` asks: Playwright's spec comments carry plain URLs.
fn angle_urls(line: &str) -> String {
    line.split(' ')
        .map(|word| {
            if word.starts_with("http://") || word.starts_with("https://") {
                format!("<{word}>")
            } else {
                word.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The file text: one line per entry, ending in a newline.
fn lines(out: &[String]) -> String {
    let mut text = out.join("\n");
    text.push('\n');
    text
}
