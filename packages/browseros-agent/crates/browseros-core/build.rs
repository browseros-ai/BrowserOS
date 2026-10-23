use std::path::{Path, PathBuf};

fn cursor_source(crate_dir: &Path, override_path: Option<&Path>) -> PathBuf {
    override_path
        .filter(|path| path.is_file())
        .map(Path::to_path_buf)
        .or_else(|| {
            let package =
                crate_dir.join("../../node_modules/@browseros/agent-cursor/dist/index.iife.js");
            package.is_file().then_some(package)
        })
        .unwrap_or_else(|| crate_dir.join("src/assets/agent-cursor-stub.js"))
}

fn watched_paths(crate_dir: &Path, override_path: Option<&Path>) -> Vec<PathBuf> {
    [
        Some(crate_dir.join("src/assets/agent-cursor-stub.js")),
        Some(crate_dir.join("../../bun.lock")),
        override_path.map(Path::to_path_buf),
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
    std::fs::copy(
        cursor_source(&crate_dir, override_path.as_deref()),
        out_dir.join("agent-cursor.js"),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{cursor_source, watched_paths};
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
            let path = self.0.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&path, "// cursor fixture")?;
            Ok(path)
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
        let override_path = fixture.write("custom.js")?;
        fixture.write("node_modules/@browseros/agent-cursor/dist/index.iife.js")?;
        assert_eq!(
            cursor_source(&fixture.crate_dir(), Some(&override_path)),
            override_path
        );
        Ok(())
    }

    #[test]
    fn tests_that_the_package_is_used_when_the_override_is_missing() -> io::Result<()> {
        let fixture = Fixture::new()?;
        fixture.write("node_modules/@browseros/agent-cursor/dist/index.iife.js")?;
        let source = cursor_source(&fixture.crate_dir(), Some(Path::new("missing.js")));
        assert_eq!(
            fs::canonicalize(source)?,
            fs::canonicalize(
                fixture
                    .0
                    .join("node_modules/@browseros/agent-cursor/dist/index.iife.js")
            )?
        );
        Ok(())
    }

    #[test]
    fn tests_that_missing_optional_bundles_fall_back_to_the_stub() -> io::Result<()> {
        let fixture = Fixture::new()?;
        let stub = fixture.write("crates/browseros-core/src/assets/agent-cursor-stub.js")?;
        for override_path in [None, Some(Path::new("missing.js"))] {
            assert_eq!(cursor_source(&fixture.crate_dir(), override_path), stub);
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
}
