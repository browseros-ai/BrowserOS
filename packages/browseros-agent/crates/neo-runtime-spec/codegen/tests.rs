//! The translator's tests: Playwright's real spec through the dialect, and each way a spec is
//! rejected.

use std::fs;
use std::path::Path;

use crate::contract::{field_ident, title_case, variant_ident};
use crate::dialect::{DefKind, DialectFile, Grammar, Prop, Protocol, Type};
use crate::output::{Mode, REGENERATE, Rendered};
use crate::spec_files::{self, SpecFile};
use crate::{Outcome, UPSTREAM, check_upstream, generate, translate, upstream_text, yaml};

/// The spec files on disk.
fn committed_spec() -> Vec<SpecFile> {
    spec_files::read(&Path::new(env!("CARGO_MANIFEST_DIR")).join("spec"))
        .expect("spec/ is readable")
}

/// The drift gate CI runs through plain `cargo test`: the committed files are exactly what the
/// spec on disk generates, compared as `--check` compares them.
#[test]
fn committed_output_matches_spec() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    match translate(crate_dir, Mode::Check) {
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
            DefKind::Union(_) => panic!("Playwright's files cannot define unions"),
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
    let spec_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("spec");
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
    let element = PLAYWRIGHT.replace(
        "      returns:\n        response: Response\n",
        "      returns: Element?\n",
    );
    let error = rejection("playwright/a.yml", &element);
    assert!(
        error.contains("Page.commands.reload.returns: uses `returns: Element?`, a Neo extension"),
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
        object.contains("is a `Page` object; a page call crosses plain JSON or one element"),
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
