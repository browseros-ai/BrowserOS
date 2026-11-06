//! Renders the contract as TypeScript: `ts/serve.ts`, `ts/injected.ts` and `ts/spec-hash.ts`.
//!
//! The files live in `ts/`, which Biome and Fallow skip, so this printer owns their layout. It
//! writes the repo's Biome style anyway: two-space indents, single quotes, no semicolons. The
//! package's `typecheck` runs `tsc --noEmit` over them under the root `strict` config.
//! `ts/serve.ts` and `ts/injected.ts` hold types only; `ts/spec-hash.ts` holds the one runtime
//! value, which the page bundle embeds.

use crate::contract::{
    Body, FieldType, InjectedModule, MemberKind, PageResult, ServeModule, Struct, TypeDef,
};
use crate::output::{self, MAX_WIDTH};

/// Renders `ts/serve.ts`.
pub fn serve(module: &ServeModule, sha256: &str) -> String {
    let mut out = output::header(sha256);
    for line in [
        "",
        "// The Playwright protocol commands Neo serves (spec/serve.yml), with params and result types",
        "// taken unchanged from Playwright's protocol spec (spec/playwright, Apache-2.0). Optional",
        "// fields are absent on the wire, never null.",
    ] {
        out.push(line.to_owned());
    }
    for interface in &module.interfaces {
        out.push(String::new());
        out.push(format!(
            "/** The `{}` commands Neo serves, by wire method name. */",
            interface.name
        ));
        out.push(format!("export type {}Commands = {{", interface.name));
        for command in &interface.commands {
            out.push(format!(
                "  {}: {{ params: {}; result: {} }}",
                key(&command.method),
                command.params,
                command.result
            ));
        }
        out.push("}".to_owned());
    }
    for ty in module.types.iter().filter(|ty| ty.exported) {
        out.push(String::new());
        type_def(&mut out, ty, &module.types);
    }
    lines(&out)
}

/// Renders `ts/injected.ts`.
pub fn injected(module: &InjectedModule, sha256: &str) -> String {
    let mut out = output::header(sha256);
    let commands = format!("{}Commands", module.interface);
    for line in [
        "",
        "// The page calls neo-runtime makes into neo-runtime-injected's glue table (spec/injected.yml),",
        "// with exactly what the page returns. Rust decodes each result with unknown fields rejected,",
        "// so an extra field fails the call.",
        "",
        "/** The page calls, by glue table key. */",
    ] {
        out.push(line.to_owned());
    }
    out.push(format!("export type {commands} = {{"));
    for call in &module.calls {
        let result = match &call.result {
            PageResult::Value(result) => result.clone(),
            PageResult::Element => "Element | undefined".to_owned(),
        };
        out.push(format!(
            "  {}: {{ params: {}; result: {result} }}",
            key(&call.name),
            call.params
        ));
    }
    out.push("}".to_owned());
    for line in [
        String::new(),
        "/**".to_owned(),
        " * The glue table `neo-runtime-injected` implements, checked with".to_owned(),
        format!(
            " * `export const glue = {{ ... }} satisfies {}Api<InjectedScript>`: a missing, extra or",
            module.interface
        ),
        " * mistyped glue function fails `tsc`. `neo-runtime` calls each function with".to_owned(),
        " * `awaitPromise: true`, so it may return a promise.".to_owned(),
        " */".to_owned(),
        format!("export type {}Api<Script> = {{", module.interface),
        format!("  [Name in keyof {commands}]: ("),
        "    injected: Script,".to_owned(),
        format!("    params: {commands}[Name]['params'],"),
        format!("  ) => {commands}[Name]['result'] | Promise<{commands}[Name]['result']>"),
        "}".to_owned(),
    ] {
        out.push(line);
    }
    for ty in module.types.iter().filter(|ty| ty.exported) {
        out.push(String::new());
        type_def(&mut out, ty, &module.types);
    }
    lines(&out)
}

/// Renders `ts/spec-hash.ts`.
pub fn spec_hash(sha256: &str) -> String {
    let mut out = output::header(sha256);
    for line in [
        "",
        "/**",
        " * SHA-256 of the spec these types were translated from. The page bundle embeds it, and",
        " * `neo-runtime` compares it with the Rust crate's `SPEC_SHA256` before calling the page.",
        " */",
    ] {
        out.push(line.to_owned());
    }
    out.push(format!("export const SPEC_SHA256 = {}", quote(sha256)));
    lines(&out)
}

/// An exported type alias.
fn type_def(out: &mut Vec<String>, ty: &TypeDef, types: &[TypeDef]) {
    doc(out, "", &ty.doc);
    let alias = format!("export type {} =", ty.name);
    match &ty.body {
        Body::Struct(fields) => out.push(format!("{alias} {}", object(fields, "", types))),
        Body::Enum(variants) => {
            let literals: Vec<String> = variants
                .iter()
                .map(|variant| quote(&variant.literal))
                .collect();
            union_alias(out, &alias, &literals);
        }
        Body::Union(union) => {
            let members: Vec<String> = union
                .members
                .iter()
                .map(|member| match &member.kind {
                    MemberKind::Bool => "boolean".to_owned(),
                    MemberKind::String => "string".to_owned(),
                    MemberKind::Enum(name) | MemberKind::Object(name) => reference(name, "", types),
                })
                .collect();
            union_alias(out, &alias, &members);
        }
        Body::Channel(channel) if channel.implemented => {
            out.push(format!("{alias} {{ guid: string }}"));
        }
        Body::Channel(_) => out.push(format!("{alias} never")),
    }
}

/// `alias A | B`, or one member per line when that would be too long.
fn union_alias(out: &mut Vec<String>, alias: &str, members: &[String]) {
    let single = format!("{alias} {}", members.join(" | "));
    if single.len() <= MAX_WIDTH {
        out.push(single);
        return;
    }
    out.push(alias.to_owned());
    for member in members {
        out.push(format!("  | {member}"));
    }
}

/// An object type's body, where `indent` is the indentation of the line it opens on.
fn object(fields: &Struct, indent: &str, types: &[TypeDef]) -> String {
    if fields.fields.is_empty() {
        return "Record<string, never>".to_owned();
    }
    let inner = format!("{indent}  ");
    let mut out = vec!["{".to_owned()];
    for field in &fields.fields {
        doc(&mut out, &inner, &field.doc);
        let optional = if field.optional { "?" } else { "" };
        out.push(format!(
            "{inner}{}{optional}: {}",
            key(&field.wire),
            field_type(&field.ty, &inner, types)
        ));
    }
    out.push(format!("{indent}}}"));
    out.join("\n")
}

fn field_type(ty: &FieldType, indent: &str, types: &[TypeDef]) -> String {
    match ty {
        FieldType::String | FieldType::Base64 => "string".to_owned(),
        FieldType::Bool => "boolean".to_owned(),
        FieldType::Int | FieldType::Float => "number".to_owned(),
        FieldType::Json => "unknown".to_owned(),
        FieldType::Ref(name) | FieldType::Boxed(name) => reference(name, indent, types),
        FieldType::Array(items) => {
            let items = field_type(items, indent, types);
            if items.contains(" | ") {
                format!("({items})[]")
            } else {
                format!("{items}[]")
            }
        }
    }
}

/// A type by name, or its body when it is an inline type TypeScript does not export.
fn reference(name: &str, indent: &str, types: &[TypeDef]) -> String {
    let Some(ty) = types
        .iter()
        .find(|ty| ty.name == name)
        .filter(|ty| !ty.exported)
    else {
        return name.to_owned();
    };
    match &ty.body {
        Body::Struct(fields) => object(fields, indent, types),
        Body::Enum(variants) => {
            let literals: Vec<String> = variants
                .iter()
                .map(|variant| quote(&variant.literal))
                .collect();
            literals.join(" | ")
        }
        Body::Union(union) => {
            let members: Vec<String> = union
                .members
                .iter()
                .map(|member| match &member.kind {
                    MemberKind::Bool => "boolean".to_owned(),
                    MemberKind::String => "string".to_owned(),
                    MemberKind::Enum(name) | MemberKind::Object(name) => {
                        reference(name, indent, types)
                    }
                })
                .collect();
            members.join(" | ")
        }
        // The contract exports every channel.
        Body::Channel(_) => name.to_owned(),
    }
}

/// A property key: bare when it is an identifier, quoted otherwise.
fn key(name: &str) -> String {
    let mut chars = name.chars();
    let identifier = chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_' || first == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if identifier {
        name.to_owned()
    } else {
        quote(name)
    }
}

/// A single-quoted string literal.
fn quote(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('\'');
    for c in text.chars() {
        match c {
            '\\' => quoted.push_str("\\\\"),
            '\'' => quoted.push_str("\\'"),
            '\n' => quoted.push_str("\\n"),
            _ => quoted.push(c),
        }
    }
    quoted.push('\'');
    quoted
}

fn doc(out: &mut Vec<String>, indent: &str, lines: &[String]) {
    // `*/` would end the comment early.
    let mut wrapped = Vec::new();
    for line in lines {
        let line = line.replace("*/", "*\\/");
        if line.is_empty() {
            wrapped.push(line);
        } else {
            wrapped.extend(output::wrap(&line, MAX_WIDTH - indent.len() - " * ".len()));
        }
    }
    match wrapped.as_slice() {
        [] => {}
        [line] if indent.len() + line.chars().count() + "/**  */".len() <= MAX_WIDTH => {
            out.push(format!("{indent}/** {line} */"));
        }
        _ => {
            out.push(format!("{indent}/**"));
            for line in &wrapped {
                if line.is_empty() {
                    out.push(format!("{indent} *"));
                } else {
                    out.push(format!("{indent} * {line}"));
                }
            }
            out.push(format!("{indent} */"));
        }
    }
}

/// The file text: one line per entry, ending in a newline.
fn lines(out: &[String]) -> String {
    let mut text = out.join("\n");
    text.push('\n');
    text
}
