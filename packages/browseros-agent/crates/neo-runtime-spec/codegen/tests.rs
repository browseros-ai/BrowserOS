//! The translator's tests: Playwright's real spec through the dialect, and each way a spec is
//! rejected.

use std::fs;
use std::path::PathBuf;

use crate::contract::{field_ident, title_case, variant_ident};
use crate::dialect::{DefKind, DialectFile, Grammar, Prop, Protocol, Type};
use crate::output::{Mode, REGENERATE, Rendered};
use crate::spec_files::{self, SpecFile};
use crate::{Outcome, UPSTREAM, check_upstream, generate, translate, upstream_text, yaml};

/// This crate's directory, read at run time for the reason on [`crate::crate_dir`].
fn crate_dir() -> PathBuf {
    crate::crate_dir().unwrap_or_else(|error| panic!("{error}"))
}

/// The spec files on disk.
fn committed_spec() -> Vec<SpecFile> {
    spec_files::read(&crate_dir().join("spec")).expect("spec/ is readable")
}

/// The drift gate CI runs through plain `cargo test`: the committed files are exactly what the
/// spec on disk generates, compared as `--check` compares them.
#[test]
fn committed_output_matches_spec() {
    match translate(&crate_dir(), Mode::Check) {
        Ok(Outcome::Translated { drift, .. }) => assert!(
            drift.is_empty(),
            "generated files are stale: {:?}; run `{REGENERATE}`",
            drift.lines(Mode::Check)
        ),
        Ok(Outcome::Pinned { .. }) => panic!("check mode never pins"),
        Err(error) => panic!("{error}"),
    }
}

fn playwright_protocol() -> Protocol {
    let mut dialect = Vec::new();
    for file in committed_spec()
        .iter()
        .filter(|file| file.path.starts_with("playwright/"))
    {
        let root = yaml::parse(file).unwrap_or_else(|error| panic!("{error}"));
        dialect.push(DialectFile {
            path: file.path.clone(),
            root,
            grammar: Grammar::Playwright,
        });
    }
    Protocol::load(&dialect).unwrap_or_else(|error| panic!("{error}"))
}

fn props_of<'p>(protocol: &'p Protocol, interface: &str, command: &str) -> &'p [Prop] {
    let id = protocol.get(interface).expect("the interface exists");
    let interface = protocol.def(id).as_interface().expect("it is an interface");
    let command = interface
        .commands
        .iter()
        .find(|candidate| candidate.name == command);
    command
        .and_then(|command| command.params.as_deref())
        .expect("the command has parameters")
}

#[test]
fn playwright_spec_loads_with_every_name_resolved() {
    let protocol = playwright_protocol();
    let mut kinds = [0; 4];
    for (_, def) in protocol.defs() {
        let slot = match def.kind {
            DefKind::Object(_) => 0,
            DefKind::Enum(_) => 1,
            DefKind::Mixin => 2,
            DefKind::Interface(_) => 3,
            DefKind::Union(_) | DefKind::Handle => {
                panic!("Playwright's files cannot use Neo's extensions")
            }
        };
        kinds[slot] += 1;
    }
    assert_eq!(
        kinds,
        [33, 3, 5, 34],
        "objects, enums, mixins and interfaces at 26137face"
    );
}

#[test]
fn yaml_null_literal_reads_as_the_string_null() {
    let protocol = playwright_protocol();
    let id = protocol
        .get("SerializedValue")
        .expect("SerializedValue exists");
    let DefKind::Object(props) = &protocol.def(id).kind else {
        panic!("SerializedValue is an object");
    };
    let v = props
        .iter()
        .find(|prop| prop.name == "v")
        .expect("SerializedValue.v exists");
    let Type::Enum(literals) = &v.ty else {
        panic!("SerializedValue.v is an inline enum");
    };
    assert_eq!(
        literals[..2],
        ["null", "undefined"],
        "as Playwright's generator stringifies them"
    );
}

#[test]
fn render_param_y_stays_a_string_under_yaml_1_2() {
    let protocol = playwright_protocol();
    let id = protocol.get("Page").expect("Page exists");
    let page = protocol
        .def(id)
        .as_interface()
        .expect("Page is an interface");
    let mouse_move = page
        .commands
        .iter()
        .find(|command| command.name == "mouseMove");
    let render_params = &mouse_move
        .expect("Page.mouseMove exists")
        .meta
        .render_params;
    assert_eq!(render_params, &["x", "y"], "YAML 1.1 would read y as true");
}

#[test]
fn mixins_splice_at_their_position() {
    let protocol = playwright_protocol();
    let names: Vec<&str> = props_of(&protocol, "BrowserType", "launchPersistentContext")
        .iter()
        .map(|prop| prop.name.as_str())
        .collect();
    let launch = names
        .iter()
        .position(|name| *name == "channel")
        .expect("LaunchOptions spliced");
    let context = names
        .iter()
        .position(|name| *name == "noDefaultViewport")
        .expect("ContextOptions spliced");
    let own = names
        .iter()
        .position(|name| *name == "userDataDir")
        .expect("own parameter kept");
    assert!(
        launch < context && context < own,
        "$mixin1, then $mixin2, then userDataDir: {names:?}"
    );
}

#[test]
fn committed_upstream_pins_the_vendored_files() {
    let spec_dir = crate_dir().join("spec");
    let upstream = fs::read_to_string(spec_dir.join(UPSTREAM)).expect("UPSTREAM is readable");
    if let Err(error) = check_upstream(&committed_spec(), &upstream) {
        panic!("{error}");
    }
    let field = |key: &str| {
        let line = upstream.lines().find_map(|line| line.strip_prefix(key));
        line.expect("UPSTREAM has the field").to_owned()
    };
    assert_eq!(
        upstream_text(&field("commit: "), &field("sha256: ")),
        upstream,
        "sync-playwright.sh would rewrite UPSTREAM differently"
    );
}

/// A small Playwright file: a `Page` with two commands, a `Frame` that extends it, a `Response`,
/// and an enum.
const PLAYWRIGHT: &str = "\
Page:
  type: interface
  commands:
    title:
      title: Get page title
      returns:
        value: string
    reload:
      title: Reload
      returns:
        response: Response
Frame:
  type: interface
  extends: Page
Response:
  type: interface
Mode:
  type: enum
  literals:
  - fast
  - slow
";

const SERVE: &str = "interfaces:\n  Page: [title]\ntypes: [Mode]\n";

const INJECTED: &str = "\
Injected:
  type: interface
  commands:
    title:
      internal: true
      returns:
        value: string
";

/// `generate` over in-memory files, with `UPSTREAM` pinned to their Playwright files.
fn generate_from(files: &[(&str, &str)]) -> Result<Vec<Rendered>, String> {
    let mut spec: Vec<SpecFile> = files
        .iter()
        .map(|(path, text)| SpecFile {
            path: (*path).to_owned(),
            bytes: text.as_bytes().to_vec(),
        })
        .collect();
    spec.sort_by(|a, b| a.path.cmp(&b.path));
    let playwright = spec
        .iter()
        .filter(|file| file.path.starts_with("playwright/"));
    let upstream = upstream_text(&"0".repeat(40), &spec_files::sha256(playwright));
    generate(&spec, &upstream).map_err(|error| error.to_string())
}

/// The error for a spec built from the small files with one of them replaced.
fn rejection(path: &str, text: &str) -> String {
    let mut files = vec![
        ("playwright/a.yml", PLAYWRIGHT),
        ("serve.yml", SERVE),
        ("injected.yml", INJECTED),
    ];
    files.retain(|(existing, _)| *existing != path);
    files.push((path, text));
    match generate_from(&files) {
        Ok(_) => panic!("the spec was accepted"),
        Err(error) => error,
    }
}

fn rendered<'r>(outputs: &'r [Rendered], path: &str) -> &'r str {
    let output = outputs.iter().find(|output| output.path == path);
    &output.expect("the file is rendered").contents
}

#[test]
fn the_small_spec_generates() {
    let outputs = generate_from(&[
        ("playwright/a.yml", PLAYWRIGHT),
        ("serve.yml", SERVE),
        ("injected.yml", INJECTED),
    ])
    .unwrap_or_else(|error| panic!("{error}"));
    assert!(
        rendered(&outputs, "src/serve.rs").contains("pub enum PageCall {"),
        "the call enum is generated"
    );
}

#[test]
fn an_unknown_command_in_serve_yml_names_the_closest() {
    let error = rejection("serve.yml", "interfaces:\n  Page: [titel]\ntypes: []\n");
    assert_eq!(
        error,
        "spec/serve.yml: interfaces.Page.0: Page has no command \"titel\" (did you mean \"title\"?)"
    );
}

#[test]
fn an_unknown_interface_or_type_in_serve_yml_names_the_closest() {
    let interface = rejection("serve.yml", "interfaces:\n  Pgae: [title]\ntypes: []\n");
    assert!(
        interface.ends_with("is not a Playwright interface (did you mean \"Page\"?)"),
        "{interface}"
    );
    let ty = rejection("serve.yml", "interfaces: {}\ntypes: [Mdoe]\n");
    assert!(
        ty.contains("\"Mdoe\" is not a Playwright object or enum (did you mean \"Mode\"?)"),
        "{ty}"
    );
}

#[test]
fn an_inherited_command_is_not_served_yet() {
    let error = rejection("serve.yml", "interfaces:\n  Frame: [title]\ntypes: []\n");
    assert!(
        error.contains("Frame.title is declared on Page, which Frame extends"),
        "{error}"
    );
}

#[test]
fn a_required_reference_to_an_unimplemented_interface_names_the_fix() {
    let error = rejection("serve.yml", "interfaces:\n  Page: [reload]\ntypes: []\n");
    assert!(
        error.starts_with("spec/serve.yml: interfaces.Page.0: the result of `Page.reload`"),
        "{error}"
    );
    assert!(
        error.contains("add `Response: []` to `interfaces`"),
        "{error}"
    );
    // Implementing `Response` makes the result constructible.
    let served = "interfaces:\n  Page: [reload]\n  Response: []\ntypes: []\n";
    let outputs = generate_from(&[
        ("playwright/a.yml", PLAYWRIGHT),
        ("serve.yml", served),
        ("injected.yml", INJECTED),
    ])
    .unwrap_or_else(|error| panic!("{error}"));
    assert!(
        rendered(&outputs, "src/serve.rs").contains("pub struct ResponseChannel {"),
        "an inhabited channel"
    );
}

#[test]
fn tests_that_bare_channels_are_uninhabited_on_the_wire_and_rejected_in_page_calls() {
    let playwright = PLAYWRIGHT.replace(
        "        value: string\n",
        "        value: string\n        handles:\n          type: array\n          items: Channel\n",
    );
    let outputs = generate_from(&[
        ("playwright/a.yml", &playwright),
        ("serve.yml", SERVE),
        ("injected.yml", INJECTED),
    ])
    .unwrap_or_else(|error| panic!("{error}"));
    assert!(
        rendered(&outputs, "src/serve.rs").contains("pub enum Channel {}"),
        "bare channels have no Rust values"
    );
    assert!(
        rendered(&outputs, "src/serve.rs").contains("pub handles: Vec<Channel>"),
        "empty handle arrays remain constructible"
    );
    assert!(
        rendered(&outputs, "ts/serve.ts").contains("export type Channel = never"),
        "bare channels have no TypeScript values"
    );

    let injected = INJECTED.replace(
        "        value: string\n",
        "        value: string\n        handles:\n          type: array\n          items: Channel\n",
    );
    let error = rejection("injected.yml", &injected);
    assert!(error.contains("is `Channel`"), "{error}");
}

#[test]
fn an_unknown_construct_is_rejected() {
    let key = PLAYWRIGHT.replace(
        "      title: Reload\n",
        "      title: Reload\n      retries: 3\n",
    );
    let error = rejection("playwright/a.yml", &key);
    assert!(
        error.contains("Page.commands.reload.retries: is not part of the dialect here"),
        "{error}"
    );
    let node = PLAYWRIGHT.replace(
        "        value: string\n",
        "        value:\n          type: map\n",
    );
    let error = rejection("playwright/a.yml", &node);
    assert!(error.contains("unknown type node \"map\""), "{error}");
    let number = PLAYWRIGHT.replace("        value: string\n", "        value: number\n");
    assert!(
        rejection("playwright/a.yml", &number).contains("uses \"number\""),
        "number is rejected"
    );
}

#[test]
fn an_extension_outside_injected_yml_is_rejected() {
    let union =
        format!("{PLAYWRIGHT}Either:\n  type: union\n  members:\n    a: string\n    b: boolean\n");
    let error = rejection("playwright/a.yml", &union);
    assert!(
        error.starts_with("spec/playwright/a.yml: Either: uses `type: union`, a Neo extension"),
        "{error}"
    );
    let handle = format!("{PLAYWRIGHT}Interceptor:\n  type: handle\n");
    let error = rejection("playwright/a.yml", &handle);
    assert_eq!(
        error,
        "spec/playwright/a.yml: Interceptor: uses `type: handle`, a Neo extension that only Neo's \
         own spec files may use"
    );
    // Only Neo's files see the built-in handle type, so a Playwright command cannot cross one.
    let element = PLAYWRIGHT.replace("        response: Response\n", "        owner: Element\n");
    let error = rejection("playwright/a.yml", &element);
    assert!(
        error.starts_with(
            "spec/playwright/a.yml: Page.commands.reload.returns.owner: unknown type \"Element\""
        ),
        "{error}"
    );
}

#[test]
fn an_edited_playwright_copy_is_rejected() {
    let mut spec = vec![
        SpecFile {
            path: "playwright/a.yml".to_owned(),
            bytes: PLAYWRIGHT.as_bytes().to_vec(),
        },
        SpecFile {
            path: "serve.yml".to_owned(),
            bytes: SERVE.as_bytes().to_vec(),
        },
    ];
    let upstream = upstream_text(&"0".repeat(40), &spec_files::sha256(&spec[..1]));
    spec[0].bytes.extend_from_slice(b"# a local edit\n");
    let Err(error) = generate(&spec, &upstream) else {
        panic!("an edited copy was accepted");
    };
    let error = error.to_string();
    assert!(
        error.starts_with("spec/playwright/UPSTREAM: pins sha256"),
        "{error}"
    );
    assert!(error.contains("sync-playwright.sh"), "{error}");
}

#[test]
fn a_carriage_return_is_rejected() {
    let error = rejection("injected.yml", &INJECTED.replace('\n', "\r\n"));
    assert!(
        error.starts_with("spec/injected.yml: contains a carriage return"),
        "{error}"
    );
}

#[test]
fn a_name_defined_twice_across_files_is_rejected() {
    let error = rejection(
        "injected.yml",
        &format!("{INJECTED}Mode:\n  type: enum\n  literals: [a]\n"),
    );
    assert_eq!(
        error,
        "spec/injected.yml: Mode: is also defined in spec/playwright/a.yml"
    );
}

#[test]
fn page_calls_cross_plain_json_only() {
    let with_returns = |returns: &str| {
        format!("{INJECTED}    frame:\n      internal: true\n      returns:\n{returns}")
    };
    let object = rejection("injected.yml", &with_returns("        page: Page\n"));
    assert!(
        object.contains("is a `Page` object; a page call crosses plain JSON and handles"),
        "{object}"
    );
    let binary = rejection("injected.yml", &with_returns("        data: binary\n"));
    assert!(binary.contains("is `binary`"), "{binary}");
    let titled = INJECTED.replace("      internal: true\n", "      title: Title\n");
    assert!(
        rejection("injected.yml", &titled).contains("a page call carries no metainfo"),
        "metainfo is rejected"
    );
}

#[test]
fn every_injected_type_is_reachable_from_injected() {
    let error = rejection(
        "injected.yml",
        &format!("{INJECTED}Unused:\n  type: object\n  properties:\n    a: string\n"),
    );
    assert_eq!(
        error,
        "spec/injected.yml: Unused: is not reachable from `Injected`; use it in a page call or remove it"
    );
}

#[test]
fn union_members_differ_in_json_kind() {
    let union = "\
Injected:
  type: interface
  commands:
    pick:
      internal: true
      returns:
        value: Choice
Choice:
  type: union
  members:
    text: string
    mode: Mode
";
    let error = rejection("injected.yml", union);
    assert!(
        error.contains("Choice.members.mode: is a JSON string, like member \"text\""),
        "{error}"
    );
}

#[test]
fn names_that_would_collide_are_rejected() {
    let fields = PLAYWRIGHT.replace(
        "        value: string\n",
        "        baseURL: string\n        baseUrl: string\n",
    );
    let error = rejection("playwright/a.yml", &fields);
    assert!(
        error.contains("both become the Rust field `base_url`"),
        "{error}"
    );
    let hoisted = "\
Injected:
  type: interface
  commands:
    title:
      internal: true
      returns:
        node: Node
Node:
  type: object
  properties:
    kind:
      type: enum
      literals: [a]
    other: NodeKind
NodeKind:
  type: enum
  literals: [b]
";
    let error = rejection("injected.yml", hoisted);
    assert!(
        error.contains("would generate the type `NodeKind`, already generated for"),
        "{error}"
    );
}

#[test]
fn recursion_that_no_array_breaks_is_boxed() {
    let tree = "\
Injected:
  type: interface
  commands:
    tree:
      internal: true
      returns:
        root: Tree
Tree:
  type: object
  properties:
    child: Tree?
    children:
      type: array
      items: Tree
    weight: float
";
    let outputs = generate_from(&[
        ("playwright/a.yml", PLAYWRIGHT),
        ("serve.yml", SERVE),
        ("injected.yml", tree),
    ])
    .unwrap_or_else(|error| panic!("{error}"));
    let injected = rendered(&outputs, "src/injected.rs");
    assert!(
        injected.contains("pub child: Option<Box<Tree>>,"),
        "the direct cycle is boxed"
    );
    assert!(
        injected.contains("pub children: Vec<Tree>,"),
        "an array already breaks the cycle"
    );
    assert!(injected.contains("#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]\n#[serde(deny_unknown_fields)]\npub struct Tree {"), "a float rules out Eq");
}

#[test]
fn identifiers_follow_the_wire_names() {
    assert_eq!(field_ident("baseURL").as_deref(), Some("base_url"));
    assert_eq!(
        field_ident("ignoreHTTPSErrors").as_deref(),
        Some("ignore_https_errors")
    );
    assert_eq!(
        field_ident("_crHasCrossSiteAncestor").as_deref(),
        Some("cr_has_cross_site_ancestor")
    );
    assert_eq!(field_ident("type").as_deref(), Some("r#type"));
    assert_eq!(field_ident("self").as_deref(), Some("self_"));
    assert_eq!(field_ident("3d"), None);
    assert_eq!(variant_ident("-Infinity").as_deref(), Some("MinusInfinity"));
    assert_eq!(variant_ident("-0").as_deref(), Some("Minus0"));
    assert_eq!(
        variant_ident("no-preference").as_deref(),
        Some("NoPreference")
    );
    assert_eq!(variant_ident("2d").as_deref(), None);
    assert_eq!(title_case("textContent"), "TextContent");
}

/// `injected.yml` with one more page call, `pick`, declared by `body` (its keys under the
/// command), and `defs` appended as definitions.
fn with_pick(body: &str, defs: &str) -> String {
    format!("{INJECTED}    pick:\n      internal: true\n{body}{defs}")
}

const INTERCEPTOR: &str = "Interceptor:\n  type: handle\n";

#[test]
fn a_handle_crosses_only_at_the_top_level_of_a_page_call() {
    let nested = with_pick(
        "      returns:\n        box:\n          type: object\n          properties:\n            \
         element: Element\n",
        "",
    );
    assert_eq!(
        rejection("injected.yml", &nested),
        "spec/injected.yml: Injected.commands.pick.returns.box.element: is a handle, which \
         crosses only as a top-level parameter or result property of a page call"
    );
    let listed = with_pick(
        "      returns:\n        elements:\n          type: array\n          items: Element\n",
        "",
    );
    let error = rejection("injected.yml", &listed);
    assert!(
        error
            .starts_with("spec/injected.yml: Injected.commands.pick.returns.elements: is a handle"),
        "{error}"
    );
    let initializer = INJECTED.replace(
        "  type: interface\n",
        "  type: interface\n  initializer:\n    owner: Element\n",
    );
    let error = rejection("injected.yml", &initializer);
    assert!(
        error.starts_with("spec/injected.yml: Injected.initializer.owner: is a handle"),
        "{error}"
    );
    let union_param = with_pick(
        "      parameters:\n        target: Target\n",
        "Target:\n  type: union\n  members:\n    text: string\n    element: Element\n",
    );
    assert_eq!(
        rejection("injected.yml", &union_param),
        "spec/injected.yml: Injected.commands.pick.parameters.target: is a union with a handle \
         member, which crosses only as a top-level result property of a page call"
    );
}

#[test]
fn a_handle_argument_is_never_optional() {
    let optional = with_pick("      parameters:\n        element: Element?\n", "");
    let error = rejection("injected.yml", &optional);
    assert!(
        error.starts_with(
            "spec/injected.yml: Injected.commands.pick.parameters.element: is an optional handle"
        ),
        "{error}"
    );
}

#[test]
fn a_result_that_holds_a_handle_has_only_primitive_properties() {
    for (property, kind) in [
        (
            "rect",
            "          type: object\n          properties:\n            x: int\n",
        ),
        ("tags", "          type: array\n          items: string\n"),
    ] {
        let returns =
            format!("      returns:\n        element: Element?\n        {property}:\n{kind}");
        let error = rejection("injected.yml", &with_pick(&returns, ""));
        assert!(
            error.starts_with(&format!(
                "spec/injected.yml: Injected.commands.pick.returns.{property}: is not a primitive; a \
                 result that holds a handle is read with `Runtime.getProperties`"
            )),
            "{error}"
        );
    }
}

#[test]
fn a_union_holds_at_most_one_handle() {
    let two = with_pick(
        "      returns:\n        target: Target\n",
        &format!(
            "Target:\n  type: union\n  members:\n    element: Element\n    armed: Interceptor\n{INTERCEPTOR}"
        ),
    );
    let error = rejection("injected.yml", &two);
    assert!(
        error.starts_with(
            "spec/injected.yml: Target.members.armed: is a JSON object, like member \"element\""
        ),
        "{error}"
    );
}

#[test]
fn every_handle_definition_is_reachable_from_injected() {
    let error = rejection("injected.yml", &format!("{INJECTED}{INTERCEPTOR}"));
    assert_eq!(
        error,
        "spec/injected.yml: Interceptor: is not reachable from `Injected`; use it in a page call or \
         remove it"
    );
    let body = format!("{INJECTED}Interceptor:\n  type: handle\n  properties:\n    stop: string\n");
    let error = rejection("injected.yml", &body);
    assert!(
        error.starts_with(
            "spec/injected.yml: Interceptor.properties: is not part of the dialect here"
        ),
        "{error}"
    );
}

/// Every place a handle may cross, in one scratch spec: a handle argument, a result that holds an
/// element next to primitives, a result union of a string and a named handle, and a call that
/// consumes that handle.
const HANDLES: &str = "\
Injected:
  type: interface
  commands:
    resolve:
      internal: true
      parameters:
        selector: string
      returns:
        element: Element?
        count: int
        mode: Mode?
    check:
      internal: true
      parameters:
        element: Element
        force: boolean
    arm:
      internal: true
      parameters:
        element: Element
      returns:
        outcome: Setup
    stop:
      internal: true
      parameters:
        interceptor: Interceptor
      returns:
        done: boolean
Interceptor:
  type: handle
Setup:
  type: union
  members:
    description: string
    armed: Interceptor
";

#[test]
fn handles_cross_as_arguments_and_properties() {
    let outputs = generate_from(&[
        ("playwright/a.yml", PLAYWRIGHT),
        ("serve.yml", SERVE),
        ("injected.yml", HANDLES),
    ])
    .unwrap_or_else(|error| panic!("{error}"));
    let rust = rendered(&outputs, "src/injected.rs");
    for expected in [
        // A handle argument is skipped by serde and listed by `handles()`, in spec order.
        "#[derive(Clone, Debug, PartialEq, Eq, Serialize)]\npub struct InjectedCheckParams {\n    \
         /// Not in the JSON params: crosses as a `Runtime.callFunctionOn` argument after them, in \
         spec\n    /// order.\n    #[serde(skip)]\n    pub element: Handle<Element>,\n    \
         #[serde(rename = \"force\")]\n    pub force: bool,\n}",
        "    fn handles(&self) -> Vec<&HandleId> {\n        vec![self.element.id()]\n    }",
        "    fn handles(&self) -> Vec<&HandleId> {\n        vec![self.interceptor.id()]\n    }",
        // A result that holds a handle is decoded property by property.
        "#[derive(Clone, Debug, PartialEq, Eq)]\npub struct InjectedResolveResult {\n    pub element: \
         Option<Handle<Element>>,\n    pub count: i64,\n    pub mode: Option<Mode>,\n}",
        "    const BY_VALUE: bool = false;",
        "            count: properties.required(\"resolve\", \"count\", decode_property)?,",
        "        properties.finish(\"resolve\")?;",
        // The union of a string and a handle is told apart by where the property arrives.
        "pub enum Setup {\n    Description(String),\n    Armed(Handle<Interceptor>),\n}",
        "            Property::Object(..) => Handle::from_property(property).map(Self::Armed),",
        // A named handle kind is an uninhabited marker.
        "pub enum Interceptor {}",
        "impl HandleKind for Interceptor {",
        // A result that holds no handle still crosses by value.
        "impl PageResult for InjectedStopResult {\n    const BY_VALUE: bool = true;",
    ] {
        assert!(
            rust.contains(expected),
            "src/injected.rs lacks:\n{expected}\n\n{rust}"
        );
    }
    let ts = rendered(&outputs, "ts/injected.ts");
    for expected in [
        "export type InjectedHandleKinds = {\n  /** `Interceptor` (`spec/injected.yml`). */\n  \
         Interceptor: unknown\n}",
        "  check: {\n    params: InjectedCheckParams\n    handles: [element: Element]\n",
        "    handles: [interceptor: H['Interceptor']]\n",
        "      outcome: string | H['Interceptor']\n",
        "      element?: Element\n",
        // A handle argument is not part of the JSON params.
        "export type InjectedCheckParams = {\n  force: boolean\n}",
        "export type InjectedStopParams = Record<string, never>",
    ] {
        assert!(
            ts.contains(expected),
            "ts/injected.ts lacks:\n{expected}\n\n{ts}"
        );
    }
    assert!(
        !ts.contains("export type InjectedResolveResult"),
        "a result that holds a handle is written inline, where `H` is in scope"
    );
}

#[test]
fn a_void_command_replies_without_a_result() {
    let served = "interfaces:\n  Page: [title, ping]\ntypes: []\n";
    let ping = PLAYWRIGHT.replace(
        "    reload:\n",
        "    ping:\n      title: Ping\n    reload:\n",
    );
    let outputs = generate_from(&[
        ("playwright/a.yml", &ping),
        ("serve.yml", served),
        ("injected.yml", INJECTED),
    ])
    .unwrap_or_else(|error| panic!("{error}"));
    let rust = rendered(&outputs, "src/serve.rs");
    assert!(
        rust.contains("impl Command for PagePingParams {\n    type Result = PagePingResult;"),
        "{rust}"
    );
    let void = |params: &str| {
        let start = rust
            .find(&format!("impl Command for {params} {{"))
            .expect("the impl exists");
        rust[start..].find("const VOID_RESULT: bool = true;") < rust[start..].find("\n}\n")
    };
    assert!(void("PagePingParams"), "ping declares no returns");
    assert!(!void("PageTitleParams"), "title returns its value");
}

/// The small spec's `Page` with a command whose params hold one of each kind of value.
fn with_fill() -> String {
    PLAYWRIGHT.replace(
        "    reload:\n",
        "    fill:
      title: Fill
      subtitle: '{selector}'
      parameters:
        selector: string
        value: string
        url: string?
        force: boolean?
        mode: Mode?
        data: binary?
        options:
          type: array?
          items:
            type: object
            properties:
              label: string
    reload:\n",
    )
}

fn generate_fill(serve: &str) -> Result<Vec<Rendered>, String> {
    generate_from(&[
        ("playwright/a.yml", &with_fill()),
        ("serve.yml", serve),
        ("injected.yml", INJECTED),
    ])
}

#[test]
fn param_kinds_come_from_types_and_trace_overrides() {
    let serve = "interfaces:\n  Page: [title, fill]\ntypes: []\ntrace:\n  Page:\n    fill:\n      \"options[].label\": plain\n";
    let outputs = generate_fill(serve).unwrap_or_else(|error| panic!("{error}"));
    let rust = rendered(&outputs, "src/serve.rs");
    for (path, kind) in [
        ("selector", "Selector"),
        ("value", "Secret"),
        ("url", "Url"),
        ("force", "Plain"),
        ("mode", "Plain"),
        ("data", "Binary"),
        ("options[].label", "Plain"),
    ] {
        let shape = format!("ParamShape {{ path: {path:?}, kind: ParamKind::{kind} }},");
        assert!(rust.contains(&shape), "{path} is {kind}: {rust}");
    }
}

#[test]
fn a_trace_override_must_name_a_served_param_and_a_kind() {
    let serve =
        |trace: &str| format!("interfaces:\n  Page: [title, fill]\ntypes: []\ntrace:\n{trace}");
    assert!(
        generate_fill(&serve("  Page:\n    fill:\n      valeu: plain\n")).is_err(),
        "a missing field fails translation"
    );
    assert!(
        generate_fill(&serve("  Page:\n    reload:\n      response: plain\n")).is_err(),
        "an unserved command fails translation"
    );
    assert!(
        generate_fill(&serve("  Page:\n    fill:\n      value: hidden\n")).is_err(),
        "an unknown kind fails translation"
    );
}
