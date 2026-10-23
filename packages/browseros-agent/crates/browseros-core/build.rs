use std::{
    fs, io,
    path::{Path, PathBuf},
};

const PACKAGE: &str = "../../node_modules/@browseros/agent-cursor";
const STUB: &str = "src/assets/agent-cursor-stub.js";

struct CursorBundle {
    script: String,
    warning: Option<String>,
}

fn cursor_bundle(crate_dir: &Path, override_path: Option<&Path>) -> io::Result<CursorBundle> {
    if let Some(path) = override_path.filter(|path| path.is_file()) {
        return Ok(CursorBundle {
            script: fs::read_to_string(path)?,
            warning: None,
        });
    }
    let package = crate_dir.join(PACKAGE);
    let classic = package.join("dist/index.iife.js");
    let module = package.join("dist/index.js");
    let source = if classic.is_file() {
        Some(&classic)
    } else {
        module.is_file().then_some(&module)
    };
    let mut warning = None;
    if let Some(source) = source {
        let derived = (|| {
            let script = fs::read_to_string(source).map_err(|error| error.to_string())?;
            let license =
                fs::read_to_string(package.join("LICENSE")).map_err(|error| error.to_string())?;
            let notice =
                fs::read_to_string(package.join("NOTICE")).map_err(|error| error.to_string())?;
            if source == &classic {
                Ok(format!("{}{script}", license_header(&license, &notice)))
            } else {
                derive_classic(&script, &license, &notice).map_err(str::to_string)
            }
        })();
        match derived {
            Ok(script) => {
                return Ok(CursorBundle {
                    script,
                    warning: None,
                });
            }
            Err(reason) => {
                warning = Some(format!(
                    "agent cursor {}: {reason}; using the stub",
                    source.display()
                ))
            }
        }
    }
    Ok(CursorBundle {
        script: fs::read_to_string(crate_dir.join(STUB))?,
        warning,
    })
}

fn license_header(license: &str, notice: &str) -> String {
    format!(
        "/*\n{}\n\n{}\n*/\n\"use strict\";\n",
        license.trim_end().replace("*/", "* /"),
        notice.trim_end().replace("*/", "* /")
    )
}

fn derive_classic(source: &str, license: &str, notice: &str) -> Result<String, &'static str> {
    let mut statements = Vec::new();
    scan_code(source.as_bytes(), 0, false, &mut statements)?;
    let [start] = statements.as_slice() else {
        return Err("expected exactly one top-level export/import statement");
    };
    let exports = source[*start..]
        .trim_end()
        .strip_suffix(';')
        .and_then(|tail| tail.trim_end().strip_suffix('}'))
        .and_then(|tail| tail.strip_prefix("export"))
        .and_then(|tail| tail.trim_start().strip_prefix('{'))
        .ok_or("expected a trailing export { ... }; statement")?;
    let mut motion_player = false;
    for entry in exports
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        let names: Vec<_> = entry.split_whitespace().collect();
        let local = match names.as_slice() {
            [local] if identifier(local) => *local,
            [local, "as", exported] if identifier(local) && identifier(exported) => *local,
            _ => return Err("unsupported trailing export list"),
        };
        motion_player |= local == "MotionPlayer";
    }
    if !motion_player {
        return Err("trailing export list must include local MotionPlayer");
    }
    Ok(format!(
        "{}{}\nvar BrowserOSAgentCursor = {{ MotionPlayer }};\n",
        license_header(license, notice),
        source[..*start].trim_end()
    ))
}

fn identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes.next().is_some_and(identifier_start) && bytes.all(identifier_part)
}

fn identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b'_' | b'$')
}

fn identifier_part(byte: u8) -> bool {
    identifier_start(byte) || byte.is_ascii_digit()
}

// Lex only enough JavaScript to find module statements outside nested code,
// comments and literals. The installed-package shape guard rejects drift.
fn scan_code(
    source: &[u8],
    mut index: usize,
    interpolation: bool,
    statements: &mut Vec<usize>,
) -> Result<usize, &'static str> {
    let mut nesting = Vec::new();
    let mut regex_allowed = true;
    let mut property = false;
    while let Some(&byte) = source.get(index) {
        match byte {
            byte if byte.is_ascii_whitespace() => index += 1,
            b'/' if source.get(index + 1) == Some(&b'/') => {
                index += 2;
                while source.get(index).is_some_and(|byte| *byte != b'\n') {
                    index += 1;
                }
            }
            b'/' if source.get(index + 1) == Some(&b'*') => {
                let end = source[index + 2..]
                    .windows(2)
                    .position(|pair| pair == b"*/")
                    .ok_or("unterminated JavaScript comment")?;
                index += end + 4;
            }
            b'\'' | b'"' | b'`' => {
                index = skip_literal(source, index)?;
                regex_allowed = false;
                property = false;
            }
            b'/' if regex_allowed => {
                index = skip_regex(source, index)?;
                regex_allowed = false;
                property = false;
            }
            b'(' | b'[' | b'{' => {
                nesting.push(byte);
                regex_allowed = true;
                property = false;
                index += 1;
            }
            b'}' if interpolation && nesting.is_empty() => return Ok(index + 1),
            b')' | b']' | b'}' => {
                let expected = match byte {
                    b')' => b'(',
                    b']' => b'[',
                    _ => b'{',
                };
                if nesting.pop() != Some(expected) {
                    return Err("unbalanced JavaScript delimiters");
                }
                regex_allowed = false;
                property = false;
                index += 1;
            }
            byte if identifier_start(byte) => {
                let start = index;
                index += 1;
                while source.get(index).is_some_and(|byte| identifier_part(*byte)) {
                    index += 1;
                }
                let word = &source[start..index];
                if !interpolation
                    && nesting.is_empty()
                    && !property
                    && matches!(word, b"export" | b"import")
                {
                    statements.push(start);
                }
                regex_allowed = matches!(
                    word,
                    b"return"
                        | b"throw"
                        | b"case"
                        | b"void"
                        | b"typeof"
                        | b"delete"
                        | b"yield"
                        | b"await"
                        | b"in"
                        | b"of"
                        | b"instanceof"
                );
                property = false;
            }
            b'.' => {
                property = true;
                regex_allowed = false;
                index += 1;
            }
            _ => {
                property = false;
                regex_allowed = !byte.is_ascii_digit();
                index += 1;
            }
        }
    }
    if interpolation || !nesting.is_empty() {
        Err("unterminated JavaScript expression")
    } else {
        Ok(index)
    }
}

fn skip_literal(source: &[u8], start: usize) -> Result<usize, &'static str> {
    let quote = source[start];
    let mut index = start + 1;
    while let Some(&byte) = source.get(index) {
        if byte == b'\\' {
            index += 2;
        } else if byte == quote {
            return Ok(index + 1);
        } else if quote == b'`' && byte == b'$' && source.get(index + 1) == Some(&b'{') {
            index = scan_code(source, index + 2, true, &mut Vec::new())?;
        } else {
            index += 1;
        }
    }
    Err("unterminated JavaScript literal")
}

fn skip_regex(source: &[u8], start: usize) -> Result<usize, &'static str> {
    let mut index = start + 1;
    let mut class = false;
    while let Some(&byte) = source.get(index) {
        match byte {
            b'\\' => index += 2,
            b'[' => {
                class = true;
                index += 1;
            }
            b']' => {
                class = false;
                index += 1;
            }
            b'/' if !class => {
                index += 1;
                while source.get(index).is_some_and(u8::is_ascii_alphabetic) {
                    index += 1;
                }
                return Ok(index);
            }
            b'\n' | b'\r' => return Err("unterminated JavaScript regex"),
            _ => index += 1,
        }
    }
    Err("unterminated JavaScript regex")
}

fn watched_paths(crate_dir: &Path, override_path: Option<&Path>) -> Vec<PathBuf> {
    [
        Some(crate_dir.join(STUB)),
        Some(crate_dir.join("../../bun.lock")),
        override_path.map(Path::to_path_buf),
        Some(crate_dir.join(PACKAGE).join("dist/index.js")),
        Some(crate_dir.join(PACKAGE).join("dist/index.iife.js")),
        Some(crate_dir.join(PACKAGE).join("LICENSE")),
        Some(crate_dir.join(PACKAGE).join("NOTICE")),
    ]
    .into_iter()
    .flatten()
    .filter(|path| path.is_file())
    .collect()
}

#[cfg(not(test))]
#[allow(clippy::print_stdout)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let crate_dir =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").ok_or("missing manifest dir")?);
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").ok_or("missing build output dir")?);
    let override_path = std::env::var_os("BROWSEROS_AGENT_CURSOR_JS").map(PathBuf::from);
    println!("cargo:rerun-if-env-changed=BROWSEROS_AGENT_CURSOR_JS");
    for path in watched_paths(&crate_dir, override_path.as_deref()) {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let bundle = cursor_bundle(&crate_dir, override_path.as_deref())?;
    if let Some(warning) = bundle.warning {
        println!("cargo:warning={warning}");
    }
    fs::write(out_dir.join("agent-cursor.js"), bundle.script)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{cursor_bundle, derive_classic, watched_paths};
    use std::{
        fs, io,
        path::{Path, PathBuf},
        sync::atomic::{AtomicUsize, Ordering},
    };

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> io::Result<Self> {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "browseros-cursor-build-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(path.join("crates/browseros-core/src/assets"))?;
            Ok(Self(path))
        }

        fn crate_dir(&self) -> PathBuf {
            self.0.join("crates/browseros-core")
        }

        fn write(&self, relative: &str) -> io::Result<PathBuf> {
            self.write_text(relative, "// cursor fixture")
        }

        fn write_text(&self, relative: &str, text: &str) -> io::Result<PathBuf> {
            let path = self.0.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&path, text)?;
            Ok(path)
        }

        fn package(&self, source: &str) -> io::Result<()> {
            self.write_text("node_modules/@browseros/agent-cursor/dist/index.js", source)?;
            self.write_text(
                "node_modules/@browseros/agent-cursor/LICENSE",
                "MIT License\nfixture */ license",
            )?;
            self.write_text(
                "node_modules/@browseros/agent-cursor/NOTICE",
                "fixture */ notice",
            )?;
            Ok(())
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn tests_that_an_existing_override_wins() -> io::Result<()> {
        let fixture = Fixture::new()?;
        let override_path = fixture.write_text("custom.js", "override cursor")?;
        fixture.write_text(
            "node_modules/@browseros/agent-cursor/dist/index.iife.js",
            "classic cursor",
        )?;
        let bundle = cursor_bundle(&fixture.crate_dir(), Some(&override_path))?;
        assert_eq!(bundle.script, "override cursor");
        assert!(bundle.warning.is_none());
        Ok(())
    }

    #[test]
    fn tests_that_the_package_is_used_when_the_override_is_missing() -> io::Result<()> {
        let fixture = Fixture::new()?;
        fixture.package("unsupported ESM shape")?;
        fixture.write_text(
            "node_modules/@browseros/agent-cursor/dist/index.iife.js",
            "classic cursor",
        )?;
        let bundle = cursor_bundle(&fixture.crate_dir(), Some(Path::new("missing.js")))?;
        assert!(bundle.script.ends_with("classic cursor"));
        assert!(bundle.script.contains("MIT License"));
        assert!(bundle.warning.is_none());
        Ok(())
    }

    #[test]
    fn tests_that_missing_optional_bundles_fall_back_to_the_stub() -> io::Result<()> {
        let fixture = Fixture::new()?;
        fixture.write_text(
            "crates/browseros-core/src/assets/agent-cursor-stub.js",
            "stub cursor",
        )?;
        for override_path in [None, Some(Path::new("missing.js"))] {
            let bundle = cursor_bundle(&fixture.crate_dir(), override_path)?;
            assert_eq!(bundle.script, "stub cursor");
            assert!(bundle.warning.is_none());
        }
        Ok(())
    }

    #[test]
    fn tests_that_only_existing_paths_are_watched() -> io::Result<()> {
        let fixture = Fixture::new()?;
        let crate_dir = fixture.crate_dir();
        assert!(watched_paths(&crate_dir, Some(Path::new("missing.js"))).is_empty());
        let stub = fixture.write("crates/browseros-core/src/assets/agent-cursor-stub.js")?;
        fixture.write("bun.lock")?;
        let override_path = fixture.write("custom.js")?;
        assert_eq!(
            watched_paths(&crate_dir, Some(&override_path)),
            vec![stub, crate_dir.join("../../bun.lock"), override_path]
        );
        Ok(())
    }

    #[test]
    fn tests_that_derivation_preserves_notices_and_strict_mode_without_exports() -> io::Result<()> {
        let script = derive_classic(
            "const MotionPlayer = class {};\nexport { MotionPlayer };\n",
            "MIT License\nfixture */ license",
            "fixture */ notice",
        )
        .map_err(io::Error::other)?;
        assert!(script.starts_with(
            "/*\nMIT License\nfixture * / license\n\nfixture * / notice\n*/\n\"use strict\";\n"
        ));
        assert!(script.contains("const MotionPlayer = class {};"));
        assert!(!script.contains("export {"));
        assert!(script.ends_with("\nvar BrowserOSAgentCursor = { MotionPlayer };\n"));
        Ok(())
    }

    #[test]
    fn tests_that_the_esm_package_is_derived_before_the_stub() -> io::Result<()> {
        let fixture = Fixture::new()?;
        fixture.package("const MotionPlayer = class {};\nexport { MotionPlayer };")?;
        fixture.write_text(
            "crates/browseros-core/src/assets/agent-cursor-stub.js",
            "stub cursor",
        )?;
        let bundle = cursor_bundle(&fixture.crate_dir(), None)?;
        assert!(bundle.script.contains("const MotionPlayer = class {};"));
        assert!(
            bundle
                .script
                .contains("var BrowserOSAgentCursor = { MotionPlayer };")
        );
        assert!(bundle.warning.is_none());
        Ok(())
    }

    #[test]
    fn tests_that_unsupported_module_shapes_fall_back_with_a_reason() -> io::Result<()> {
        for (source, reason) in [
            ("const MotionPlayer = class {};", "export"),
            (
                "const MotionPlayer = class {}; export { MotionPlayer };\nvoid 0;",
                "trailing",
            ),
            (
                "export const Other = 1;\nconst MotionPlayer = class {};\nexport { MotionPlayer };",
                "export/import",
            ),
            (
                "import Other from './other.js';\nconst MotionPlayer = class {};\nexport { MotionPlayer };",
                "export/import",
            ),
            (
                "const Player = class {};\nexport { Player };",
                "MotionPlayer",
            ),
            (
                "const Player = class {};\nexport { Player as MotionPlayer };",
                "MotionPlayer",
            ),
        ] {
            let fixture = Fixture::new()?;
            fixture.package(source)?;
            fixture.write_text(
                "crates/browseros-core/src/assets/agent-cursor-stub.js",
                "stub cursor",
            )?;
            let bundle = cursor_bundle(&fixture.crate_dir(), None)?;
            assert_eq!(bundle.script, "stub cursor");
            assert!(
                bundle
                    .warning
                    .as_deref()
                    .is_some_and(|warning| warning.contains(reason)),
                "{source}: {:?}",
                bundle.warning
            );
        }
        Ok(())
    }

    #[test]
    fn tests_that_module_validation_ignores_nested_syntax_and_literal_keywords() -> io::Result<()> {
        let script = derive_classic(
            r#"// import ignored from 'comment';
const text = "export { Other };";
const pattern = /export\s*\{import\}/;
const template = `outer ${`nested ${"export"}`} import`;
const MotionPlayer = class { method() { return { import: 1, export: 2 }; } };
/* export { Nope }; */
export { MotionPlayer, text as Text };
"#,
            "MIT License",
            "notice",
        )
        .map_err(io::Error::other)?;
        assert!(script.contains("const MotionPlayer = class"));
        assert!(!script.contains("export { MotionPlayer, text as Text }"));
        assert!(derive_classic(
            "const value = `outer ${`inner`}`; import value2 from 'other'; export { MotionPlayer };",
            "MIT License", "notice"
        ).is_err());
        Ok(())
    }

    #[test]
    fn tests_that_existing_package_inputs_are_watched() -> io::Result<()> {
        let fixture = Fixture::new()?;
        fixture.package("const MotionPlayer = class {}; export { MotionPlayer };")?;
        fixture.write("node_modules/@browseros/agent-cursor/dist/index.iife.js")?;
        let crate_dir = fixture.crate_dir();
        let watched = watched_paths(&crate_dir, None);
        for name in ["dist/index.js", "dist/index.iife.js", "LICENSE", "NOTICE"] {
            assert!(
                watched.contains(
                    &crate_dir
                        .join("../../node_modules/@browseros/agent-cursor")
                        .join(name)
                )
            );
        }
        assert!(watched.iter().all(|path| path.is_file()));
        Ok(())
    }

    #[test]
    fn tests_that_the_installed_cursor_package_matches_the_supported_esm_shape() -> io::Result<()> {
        let package = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../node_modules/@browseros/agent-cursor");
        if !package.exists() {
            return Ok(());
        }
        let script = derive_classic(
            &fs::read_to_string(package.join("dist/index.js"))?,
            &fs::read_to_string(package.join("LICENSE"))?,
            &fs::read_to_string(package.join("NOTICE"))?,
        )
        .map_err(io::Error::other)?;
        assert!(script.contains("var BrowserOSAgentCursor = { MotionPlayer };"));
        assert!(script.contains("MIT License"));
        Ok(())
    }
}
