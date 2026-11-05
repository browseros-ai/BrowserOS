//! The spec as bytes: which files it is, their order, and their SHA-256.
//!
//! `build.rs` includes this file with `#[path]` to hash the spec as it is now. The translator
//! reads the spec through it once and then parses and hashes those same bytes. Both sides
//! define the input the same way, so the hash stamped into the generated code and the hash
//! `build.rs` computes compare like for like.
//!
//! The spec is every `*.yml` under `spec/`, ordered by its `/`-separated path relative to
//! `spec/`, compared as bytes. The digest covers, per file: the path, a NUL byte, the length in
//! decimal ASCII, a NUL byte, then the raw bytes. `spec/.gitattributes` keeps Git from
//! rewriting those bytes. The same digest from a shell, run in `spec/`:
//!
//! ```text
//! find . -name '*.yml' | sed 's|^\./||' | LC_ALL=C sort | while read -r f; do
//!   printf '%s\0%s\0' "$f" "$(wc -c < "$f" | tr -d ' ')"; cat "$f"; done | shasum -a 256
//! ```

use std::fs;
use std::io;
use std::path::Path;

use sha2::{Digest, Sha256};

/// One spec file: its `/`-separated path relative to `spec/`, and its raw bytes.
pub struct SpecFile {
    pub path: String,
    pub bytes: Vec<u8>,
}

/// Reads every `*.yml` under `spec_dir`, in spec order.
pub fn read(spec_dir: &Path) -> io::Result<Vec<SpecFile>> {
    let mut paths = Vec::new();
    collect_yml(spec_dir, spec_dir, &mut paths)?;
    paths.sort();
    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        let bytes = fs::read(spec_dir.join(&path))?;
        files.push(SpecFile { path, bytes });
    }
    Ok(files)
}

/// The lowercase hex SHA-256 of `files`, which must be in spec order, as [`read`] returns them.
pub fn sha256(files: &[SpecFile]) -> String {
    let mut hasher = Sha256::new();
    for file in files {
        hasher.update(file.path.as_bytes());
        hasher.update([0]);
        hasher.update(file.bytes.len().to_string().as_bytes());
        hasher.update([0]);
        hasher.update(&file.bytes);
    }
    format!("{:x}", hasher.finalize())
}

/// Appends the `/`-separated path, relative to `root`, of every `*.yml` under `dir`.
fn collect_yml(root: &Path, dir: &Path, paths: &mut Vec<String>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_yml(root, &path, paths)?;
        } else if path.extension().is_some_and(|extension| extension == "yml") {
            let relative = path.strip_prefix(root).map_err(io::Error::other)?;
            let relative = relative.to_str().ok_or_else(|| {
                io::Error::other(format!("non-UTF-8 spec path {}", path.display()))
            })?;
            paths.push(relative.replace('\\', "/"));
        }
    }
    Ok(())
}
