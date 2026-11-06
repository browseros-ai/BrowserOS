//! The translator's tests: Playwright's real spec through the dialect, and each way a spec is
//! rejected.

use std::fs;
use std::path::Path;

use crate::dialect::{DefKind, DialectFile, Grammar, Prop, Protocol, Type};
use crate::output::{Mode, REGENERATE};
use crate::spec_files::{self, SpecFile};
use crate::{Outcome, UPSTREAM, check_upstream, translate, upstream_text, yaml};

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
            drift.lines()
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
