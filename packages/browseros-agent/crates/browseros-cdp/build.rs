use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    env, fs, io,
    path::Path,
};

#[derive(Debug, Deserialize)]
struct Protocol {
    domains: Vec<Domain>,
}

#[derive(Debug, Clone, Deserialize)]
struct Domain {
    domain: String,
    #[serde(default)]
    types: Vec<TypeDef>,
    #[serde(default)]
    commands: Vec<Command>,
    #[serde(default)]
    events: Vec<Event>,
}

#[derive(Debug, Clone, Deserialize)]
struct TypeDef {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    properties: Vec<Property>,
    #[serde(default)]
    items: Option<Item>,
}

#[derive(Debug, Clone, Deserialize)]
struct Command {
    name: String,
    #[serde(default)]
    parameters: Vec<Property>,
    #[serde(default)]
    returns: Vec<Property>,
}

#[derive(Debug, Clone, Deserialize)]
struct Event {
    name: String,
    #[serde(default)]
    parameters: Vec<Property>,
}

#[derive(Debug, Clone, Deserialize)]
struct Property {
    name: String,
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(rename = "$ref")]
    ref_name: Option<String>,
    #[serde(default)]
    optional: bool,
    #[serde(default)]
    items: Option<Item>,
}

#[derive(Debug, Clone, Deserialize)]
struct Item {
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(rename = "$ref")]
    ref_name: Option<String>,
    #[serde(default)]
    items: Option<Box<Item>>,
}

#[derive(Debug, Deserialize)]
struct Surface {
    domains: Vec<SurfaceDomain>,
}

#[derive(Debug, Deserialize)]
struct SurfaceDomain {
    domain: String,
    #[serde(default)]
    types: Vec<String>,
    #[serde(default)]
    commands: Vec<String>,
    #[serde(default)]
    events: Vec<String>,
}

#[derive(Debug, Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct TypeKey {
    domain: String,
    name: String,
}

#[derive(Debug, Default)]
struct SelectedDomain {
    types: BTreeMap<String, TypeDef>,
    commands: BTreeMap<String, Command>,
    events: BTreeMap<String, Event>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR")?;
    let protocol_dir = Path::new(&manifest_dir).join("protocol");
    let protocol_path = protocol_dir.join("protocol.json");
    let surface_path = protocol_dir.join("surface.json");
    let sha_path = protocol_dir.join("protocol.sha256");
    for path in [&protocol_path, &surface_path, &sha_path] {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    let protocol_json = fs::read(&protocol_path)?;
    let expected_sha = fs::read_to_string(&sha_path)?;
    let actual_sha = format!("{:x}", Sha256::digest(&protocol_json));
    if actual_sha != expected_sha.trim() {
        return Err(io::Error::other(format!(
            "{} does not match pinned SHA-256 {}",
            protocol_path.display(),
            expected_sha.trim()
        ))
        .into());
    }

    let protocol: Protocol = serde_json::from_slice(&protocol_json)?;
    let surface: Surface = serde_json::from_slice(&fs::read(surface_path)?)?;
    let domains = select_surface(protocol, surface)?;
    let recursive_edges = recursive_type_edges(&domains);

    let mut out = String::from(
        "use serde::{Deserialize, Serialize};\n\
         use serde_json::Value;\n\
         use crate::{CdpClient, CdpError, SessionId};\n\n\
         #[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]\n\
         pub struct EmptyParams {}\n\n",
    );
    for domain in &domains {
        emit_domain(&mut out, domain, &recursive_edges);
    }

    let out_dir = env::var("OUT_DIR")?;
    fs::write(Path::new(&out_dir).join("protocol.rs"), out)?;
    Ok(())
}

fn select_surface(protocol: Protocol, surface: Surface) -> Result<Vec<Domain>, io::Error> {
    let source = protocol
        .domains
        .into_iter()
        .map(|domain| (domain.domain.clone(), domain))
        .collect::<BTreeMap<_, _>>();
    let mut selected = BTreeMap::<String, SelectedDomain>::new();
    let mut pending_types = BTreeSet::<TypeKey>::new();

    for selection in surface.domains {
        let domain = source.get(&selection.domain).ok_or_else(|| {
            io::Error::other(format!(
                "selected protocol domain {} is missing",
                selection.domain
            ))
        })?;
        let output = selected.entry(selection.domain.clone()).or_default();

        for name in selection.types {
            pending_types.insert(TypeKey {
                domain: selection.domain.clone(),
                name,
            });
        }
        for name in selection.commands {
            let command = find_command(domain, &name)?;
            collect_property_refs(
                &selection.domain,
                command.parameters.iter().chain(command.returns.iter()),
                &mut pending_types,
            );
            output.commands.insert(name, command.clone());
        }
        for name in selection.events {
            let event = find_event(domain, &name)?;
            collect_property_refs(
                &selection.domain,
                event.parameters.iter(),
                &mut pending_types,
            );
            output.events.insert(name, event.clone());
        }
    }

    while let Some(key) = pending_types.pop_first() {
        if selected
            .get(&key.domain)
            .is_some_and(|domain| domain.types.contains_key(&key.name))
        {
            continue;
        }
        let source_domain = source.get(&key.domain).ok_or_else(|| {
            io::Error::other(format!(
                "referenced protocol domain {} is missing",
                key.domain
            ))
        })?;
        let type_def = find_type(source_domain, &key.name)?;
        collect_property_refs(&key.domain, type_def.properties.iter(), &mut pending_types);
        if let Some(items) = &type_def.items {
            collect_item_refs(&key.domain, items, &mut pending_types);
        }
        selected
            .entry(key.domain)
            .or_default()
            .types
            .insert(key.name, type_def.clone());
    }

    Ok(selected
        .into_iter()
        .map(|(domain, selected)| Domain {
            domain,
            types: selected.types.into_values().collect(),
            commands: selected.commands.into_values().collect(),
            events: selected.events.into_values().collect(),
        })
        .collect())
}

fn find_type<'a>(domain: &'a Domain, name: &str) -> Result<&'a TypeDef, io::Error> {
    domain
        .types
        .iter()
        .find(|type_def| type_def.id == name)
        .ok_or_else(|| {
            io::Error::other(format!(
                "selected protocol type {}.{} is missing",
                domain.domain, name
            ))
        })
}

fn find_command<'a>(domain: &'a Domain, name: &str) -> Result<&'a Command, io::Error> {
    domain
        .commands
        .iter()
        .find(|command| command.name == name)
        .ok_or_else(|| {
            io::Error::other(format!(
                "selected protocol command {}.{} is missing",
                domain.domain, name
            ))
        })
}

fn find_event<'a>(domain: &'a Domain, name: &str) -> Result<&'a Event, io::Error> {
    domain
        .events
        .iter()
        .find(|event| event.name == name)
        .ok_or_else(|| {
            io::Error::other(format!(
                "selected protocol event {}.{} is missing",
                domain.domain, name
            ))
        })
}

fn collect_property_refs<'a>(
    domain: &str,
    properties: impl Iterator<Item = &'a Property>,
    refs: &mut BTreeSet<TypeKey>,
) {
    for property in properties {
        if let Some(ref_name) = &property.ref_name {
            refs.insert(resolve_type_key(domain, ref_name));
        }
        if let Some(items) = &property.items {
            collect_item_refs(domain, items, refs);
        }
    }
}

fn collect_item_refs(domain: &str, item: &Item, refs: &mut BTreeSet<TypeKey>) {
    if let Some(ref_name) = &item.ref_name {
        refs.insert(resolve_type_key(domain, ref_name));
    }
    if let Some(items) = &item.items {
        collect_item_refs(domain, items, refs);
    }
}

fn resolve_type_key(current_domain: &str, ref_name: &str) -> TypeKey {
    if let Some((domain, name)) = ref_name.split_once('.') {
        TypeKey {
            domain: domain.to_string(),
            name: name.to_string(),
        }
    } else {
        TypeKey {
            domain: current_domain.to_string(),
            name: ref_name.to_string(),
        }
    }
}

fn recursive_type_edges(domains: &[Domain]) -> HashSet<(TypeKey, TypeKey)> {
    let mut graph = BTreeMap::<TypeKey, BTreeSet<TypeKey>>::new();
    for domain in domains {
        for type_def in &domain.types {
            let source = TypeKey {
                domain: domain.domain.clone(),
                name: type_def.id.clone(),
            };
            let mut refs = BTreeSet::new();
            collect_property_refs(&domain.domain, type_def.properties.iter(), &mut refs);
            if let Some(items) = &type_def.items {
                collect_item_refs(&domain.domain, items, &mut refs);
            }
            graph.insert(source, refs);
        }
    }

    let mut recursive = HashSet::new();
    for (source, targets) in &graph {
        for target in targets {
            if source == target || is_reachable(target, source, &graph, &mut BTreeSet::new()) {
                recursive.insert((source.clone(), target.clone()));
            }
        }
    }
    recursive
}

fn is_reachable(
    current: &TypeKey,
    target: &TypeKey,
    graph: &BTreeMap<TypeKey, BTreeSet<TypeKey>>,
    visited: &mut BTreeSet<TypeKey>,
) -> bool {
    if current == target {
        return true;
    }
    if !visited.insert(current.clone()) {
        return false;
    }
    graph.get(current).is_some_and(|next| {
        next.iter()
            .any(|node| is_reachable(node, target, graph, visited))
    })
}

fn emit_domain(out: &mut String, domain: &Domain, recursive_edges: &HashSet<(TypeKey, TypeKey)>) {
    let module = to_snake(&domain.domain);
    out.push_str(&format!(
        "pub mod {module} {{\n#[allow(unused_imports)]\nuse super::*;\n\n"
    ));

    for type_def in &domain.types {
        emit_type(out, domain, type_def, recursive_edges);
    }
    for command in &domain.commands {
        emit_command_types(out, domain, command, recursive_edges);
    }
    for event in &domain.events {
        emit_event_type(out, domain, event, recursive_edges);
    }
    for command in &domain.commands {
        emit_command_fn(out, domain, command);
    }

    out.push_str("}\n\n");
}

fn emit_type(
    out: &mut String,
    domain: &Domain,
    type_def: &TypeDef,
    recursive_edges: &HashSet<(TypeKey, TypeKey)>,
) {
    let name = to_pascal(&type_def.id);
    match type_def.kind.as_str() {
        "object" => emit_struct(
            out,
            domain,
            &name,
            &type_def.id,
            &type_def.properties,
            recursive_edges,
        ),
        "array" => {
            let item_ty = type_def
                .items
                .as_ref()
                .map(|item| rust_item_type(&domain.domain, &type_def.id, item, recursive_edges))
                .unwrap_or_else(|| "Value".to_string());
            out.push_str(&format!("pub type {name} = Vec<{item_ty}>;\n\n"));
        }
        _ => {
            let ty = rust_primitive(&type_def.kind);
            out.push_str(&format!("pub type {name} = {ty};\n\n"));
        }
    }
}

fn emit_command_types(
    out: &mut String,
    domain: &Domain,
    command: &Command,
    recursive_edges: &HashSet<(TypeKey, TypeKey)>,
) {
    let base = to_pascal(&command.name);
    let params = format!("{base}Params");
    let result = format!("{base}Result");
    if !command.parameters.is_empty() {
        emit_struct(
            out,
            domain,
            &params,
            &params,
            &command.parameters,
            recursive_edges,
        );
    }
    emit_struct(
        out,
        domain,
        &result,
        &result,
        &command.returns,
        recursive_edges,
    );
}

fn emit_event_type(
    out: &mut String,
    domain: &Domain,
    event: &Event,
    recursive_edges: &HashSet<(TypeKey, TypeKey)>,
) {
    let name = format!("{}Event", to_pascal(&event.name));
    emit_struct(
        out,
        domain,
        &name,
        &name,
        &event.parameters,
        recursive_edges,
    );
}

fn emit_struct(
    out: &mut String,
    domain: &Domain,
    rust_name: &str,
    type_name: &str,
    properties: &[Property],
    recursive_edges: &HashSet<(TypeKey, TypeKey)>,
) {
    out.push_str("#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]\n");
    out.push_str(&format!("pub struct {rust_name} {{\n"));
    for property in properties {
        let field = field_name(&property.name);
        let ty = rust_property_type(&domain.domain, type_name, property, recursive_edges);
        if property.optional {
            out.push_str(&format!(
                "    #[serde(rename = \"{}\", skip_serializing_if = \"Option::is_none\")]\n",
                property.name
            ));
        } else {
            out.push_str(&format!("    #[serde(rename = \"{}\")]\n", property.name));
        }
        out.push_str(&format!("    pub {field}: {ty},\n"));
    }
    out.push_str("}\n\n");
}

fn emit_command_fn(out: &mut String, domain: &Domain, command: &Command) {
    let fn_name = to_snake(&command.name);
    let method = format!("{}.{}", domain.domain, command.name);
    let base = to_pascal(&command.name);
    let params = if command.parameters.is_empty() {
        "EmptyParams".to_string()
    } else {
        format!("{base}Params")
    };
    let result = format!("{base}Result");
    let args = if command.parameters.is_empty() {
        "EmptyParams {}".to_string()
    } else {
        "params".to_string()
    };
    let params_arg = if command.parameters.is_empty() {
        String::new()
    } else {
        format!("params: {params}, ")
    };

    out.push_str(&format!(
        "pub async fn {fn_name}(client: &CdpClient, {params_arg}session: Option<&SessionId>) -> Result<{result}, CdpError> {{\n    client.send_typed(\"{method}\", {args}, session).await\n}}\n\n"
    ));
}

fn rust_property_type(
    domain: &str,
    type_name: &str,
    property: &Property,
    recursive_edges: &HashSet<(TypeKey, TypeKey)>,
) -> String {
    let mut ty = if let Some(ref_name) = &property.ref_name {
        rust_ref_type(domain, type_name, ref_name, recursive_edges)
    } else if property.kind.as_deref() == Some("array") {
        property
            .items
            .as_ref()
            .map(|item| rust_item_type(domain, type_name, item, recursive_edges))
            .map(|item| format!("Vec<{item}>"))
            .unwrap_or_else(|| "Vec<Value>".to_string())
    } else {
        rust_primitive(property.kind.as_deref().unwrap_or("object")).to_string()
    };

    if property.optional {
        ty = format!("Option<{ty}>");
    }
    ty
}

fn rust_item_type(
    domain: &str,
    type_name: &str,
    item: &Item,
    recursive_edges: &HashSet<(TypeKey, TypeKey)>,
) -> String {
    if let Some(ref_name) = &item.ref_name {
        return rust_ref_type(domain, type_name, ref_name, recursive_edges);
    }
    if item.kind.as_deref() == Some("array") {
        let inner = item
            .items
            .as_deref()
            .map(|inner| rust_item_type(domain, type_name, inner, recursive_edges))
            .unwrap_or_else(|| "Value".to_string());
        return format!("Vec<{inner}>");
    }
    rust_primitive(item.kind.as_deref().unwrap_or("object")).to_string()
}

fn rust_ref_type(
    current_domain: &str,
    current_type: &str,
    ref_name: &str,
    recursive_edges: &HashSet<(TypeKey, TypeKey)>,
) -> String {
    let target = resolve_type_key(current_domain, ref_name);
    let mut ty = if target.domain == current_domain {
        to_pascal(&target.name)
    } else {
        format!(
            "super::{}::{}",
            to_snake(&target.domain),
            to_pascal(&target.name)
        )
    };
    let source = TypeKey {
        domain: current_domain.to_string(),
        name: current_type.to_string(),
    };
    if recursive_edges.contains(&(source, target)) {
        ty = format!("Box<{ty}>");
    }
    ty
}

fn rust_primitive(kind: &str) -> &'static str {
    match kind {
        "string" => "String",
        "integer" => "i64",
        "number" => "f64",
        "boolean" => "bool",
        "object" => "Value",
        _ => "Value",
    }
}

fn to_pascal(value: &str) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut out = String::new();
    out.extend(first.to_uppercase());
    out.push_str(chars.as_str());
    out
}

fn to_snake(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::new();
    for (index, ch) in chars.iter().enumerate() {
        if ch.is_ascii_uppercase() {
            let prev = index.checked_sub(1).and_then(|prev| chars.get(prev));
            let next = chars.get(index + 1);
            let boundary = prev.is_some_and(|prev| {
                prev.is_ascii_lowercase()
                    || prev.is_ascii_digit()
                    || next.is_some_and(|next| next.is_ascii_lowercase())
            });
            if index > 0 && boundary {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else if *ch == '-' || *ch == ' ' {
            out.push('_');
        } else {
            out.push(*ch);
        }
    }
    out
}

fn field_name(value: &str) -> String {
    let name = to_snake(value);
    if matches!(name.as_str(), "crate" | "self" | "super") {
        return format!("{name}_");
    }
    if matches!(
        name.as_str(),
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
            | "union"
            | "unsafe"
            | "unsized"
            | "use"
            | "virtual"
            | "where"
            | "while"
            | "yield"
    ) {
        return format!("r#{name}");
    }
    name
}
