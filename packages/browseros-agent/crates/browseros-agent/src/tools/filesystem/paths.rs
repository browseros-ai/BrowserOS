//! The filesystem tools' path boundary (port of `tools/filesystem/path-boundary.ts`): every path a
//! model names is resolved here before anything touches the disk.
//!
//! Two roots exist. The **workspace** (the user's chosen folder) accepts only relative input; the
//! lexical join must stay inside it and so must the canonical (symlink-resolved) result, so `..`
//! and symlinks cannot escape. The **tool-output directory** (`<browseros dir>/tool-output`, where
//! the MCP host writes large browser-tool results) accepts only absolute input and only canonical
//! paths under it; sibling BrowserOS state (DB, OAuth tokens, sessions) is never readable.
//!
//! The error texts are model-visible and copied verbatim from TS.

use std::{
    io,
    path::{Component, Path, PathBuf},
};

pub(super) const OUTSIDE_WORKSPACE: &str = "Path is outside the selected workspace.";
const MUST_BE_RELATIVE: &str = "Path must be relative to the selected workspace.";
const MUST_BE_ABSOLUTE_OUTPUT: &str = "Path must be an absolute BrowserOS tool output path.";
const OUTSIDE_TOOL_OUTPUT: &str = "Path is outside BrowserOS tool output.";

/// A path-boundary or I/O failure, already rendered as the text the model sees (TS throws, and
/// `executeWithMetrics` turns the message into `{text, isError: true}`).
pub(super) type FsError = String;

/// `isAbsolute(p) || win32.isAbsolute(p)`: both POSIX and Windows absolute spellings are refused
/// as workspace input on every platform, so `C:\…` cannot slip through on macOS as a "relative"
/// file name either.
pub(super) fn is_absolute_input(input: &str) -> bool {
    let bytes = input.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\');
    input.starts_with('/') || input.starts_with('\\') || drive
}

/// `path.resolve(base, input)` for relative input: a purely lexical join that folds `.` and `..`
/// (never above the filesystem root), touching nothing on disk.
pub(super) fn lexical_join(base: &Path, input: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for component in base.join(input).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    out.components().next_back(),
                    Some(Component::RootDir) | None
                ) {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `isPathInside`: `candidate` is `root` or below it, compared component-wise on normalized
/// absolute paths (so `/ws-other` is not inside `/ws`, and a file named `..x` is).
pub(super) fn is_path_inside(root: &Path, candidate: &Path) -> bool {
    candidate.starts_with(root)
}

fn assert_inside_workspace(root: &Path, candidate: &Path) -> Result<(), FsError> {
    if is_path_inside(root, candidate) {
        Ok(())
    } else {
        Err(OUTSIDE_WORKSPACE.to_owned())
    }
}

fn assert_relative(input: &str) -> Result<(), FsError> {
    if is_absolute_input(input) {
        Err(MUST_BE_RELATIVE.to_owned())
    } else {
        Ok(())
    }
}

pub(super) fn realpath(path: &Path) -> Result<PathBuf, FsError> {
    std::fs::canonicalize(path).map_err(|error| node_error(&error, "realpath", Some(path)))
}

/// `resolveWorkspaceRoot`: the canonical workspace, re-resolved on every call as TS does, so a
/// workspace folder swapped for a symlink mid-session is judged by where it points now.
pub(super) fn workspace_root(cwd: &Path) -> Result<PathBuf, FsError> {
    realpath(cwd)
}

/// `resolveWorkspacePathFromRoot`: an existing path under the canonical root; both the lexical
/// candidate and its canonical form must stay inside.
pub(super) fn resolve_existing_from_root(root: &Path, input: &str) -> Result<PathBuf, FsError> {
    assert_relative(input)?;
    let candidate = lexical_join(root, input);
    assert_inside_workspace(root, &candidate)?;
    let canonical = realpath(&candidate)?;
    assert_inside_workspace(root, &canonical)?;
    Ok(canonical)
}

pub(super) fn resolve_existing(cwd: &Path, input: &str) -> Result<PathBuf, FsError> {
    resolve_existing_from_root(&workspace_root(cwd)?, input)
}

/// `resolveWorkspaceWritePath`: a write target that may not exist yet. An existing target is
/// judged canonically; otherwise the nearest existing ancestor is canonicalized and the missing
/// tail re-appended, so `mkdir -p` can never be steered outside through a symlinked parent.
///
/// DECIDED (deviation): a target that is itself a dangling symlink is refused. TS treats it as a
/// missing file and `writeFile` then follows the link wherever it points, outside the workspace
/// included.
pub(super) fn resolve_write_target(cwd: &Path, input: &str) -> Result<PathBuf, FsError> {
    assert_relative(input)?;
    let root = workspace_root(cwd)?;
    let candidate = lexical_join(&root, input);
    assert_inside_workspace(&root, &candidate)?;

    match std::fs::canonicalize(&candidate) {
        Ok(canonical) => {
            assert_inside_workspace(&root, &canonical)?;
            return Ok(canonical);
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(node_error(&error, "realpath", Some(&candidate))),
    }
    if std::fs::symlink_metadata(&candidate).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(OUTSIDE_WORKSPACE.to_owned());
    }

    let parent = find_existing_parent(&root, &candidate)?;
    let canonical_parent = realpath(&parent)?;
    assert_inside_workspace(&root, &canonical_parent)?;
    let tail = candidate
        .strip_prefix(&parent)
        .map_err(|_| OUTSIDE_WORKSPACE.to_owned())?;
    let resolved = canonical_parent.join(tail);
    assert_inside_workspace(&root, &resolved)?;
    Ok(resolved)
}

fn find_existing_parent(root: &Path, target: &Path) -> Result<PathBuf, FsError> {
    let mut parent = target.parent().map(Path::to_path_buf);
    while let Some(current) = parent {
        if !is_path_inside(root, &current) {
            break;
        }
        match std::fs::symlink_metadata(&current) {
            Ok(_) => return Ok(current),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(node_error(&error, "lstat", Some(&current))),
        }
        parent = current.parent().map(Path::to_path_buf);
    }
    Err(OUTSIDE_WORKSPACE.to_owned())
}

/// The BrowserOS state directory, by the same rule `browseros_mcp::output_file::get_browseros_dir`
/// uses. It must stay identical: the MCP host writes generated outputs under this directory and
/// `filesystem_read` reads them back from it (this crate may not depend on browseros-mcp).
pub(super) fn browseros_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("BROWSEROS_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
    home.join(if cfg!(debug_assertions) {
        ".browseros-dev"
    } else {
        ".browseros"
    })
}

/// `getToolOutputDir`: created on demand (0700), required to be a real directory, canonical.
pub(super) fn tool_output_dir() -> Result<PathBuf, FsError> {
    let dir = browseros_dir().join("tool-output");
    std::fs::create_dir_all(&dir).map_err(|error| node_error(&error, "mkdir", Some(&dir)))?;
    let meta =
        std::fs::symlink_metadata(&dir).map_err(|error| node_error(&error, "lstat", Some(&dir)))?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err("BrowserOS tool output directory must be a real directory.".to_owned());
    }
    let canonical = realpath(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&canonical, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| node_error(&error, "chmod", Some(&canonical)))?;
    }
    Ok(canonical)
}

/// `path.resolve(input)` for an input already known to be absolute input.
fn resolve_absolute(input: &str) -> PathBuf {
    let base = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    lexical_join(&base, input)
}

/// `isBrowserosStatePath`: whether an absolute input points into the BrowserOS state directory,
/// lexically or after resolving symlinks on both sides.
pub(super) fn is_browseros_state_path(input: &str) -> bool {
    if !is_absolute_input(input) {
        return false;
    }
    let state_root = resolve_absolute(&browseros_dir().to_string_lossy());
    let candidate = resolve_absolute(input);
    if is_path_inside(&state_root, &candidate) {
        return true;
    }
    match (
        std::fs::canonicalize(&state_root),
        std::fs::canonicalize(&candidate),
    ) {
        (Ok(root), Ok(candidate)) => is_path_inside(&root, &candidate),
        _ => false,
    }
}

/// `resolveBrowserToolOutputPath`: an existing file whose canonical path is under the canonical
/// tool-output directory.
pub(super) fn resolve_tool_output(input: &str) -> Result<PathBuf, FsError> {
    if !is_absolute_input(input) {
        return Err(MUST_BE_ABSOLUTE_OUTPUT.to_owned());
    }
    let output_root = tool_output_dir()?;
    let canonical = realpath(&resolve_absolute(input))?;
    if !is_path_inside(&output_root, &canonical) {
        return Err(OUTSIDE_TOOL_OUTPUT.to_owned());
    }
    Ok(canonical)
}

/// Node's `fs` error message (`ENOENT: no such file or directory, open '/x'`): these strings
/// reach the model as tool output, and TS tools surface them verbatim.
pub(super) fn node_error(error: &io::Error, syscall: &str, path: Option<&Path>) -> String {
    let described = match error.kind() {
        io::ErrorKind::NotFound => Some("ENOENT: no such file or directory"),
        io::ErrorKind::PermissionDenied => Some("EACCES: permission denied"),
        io::ErrorKind::AlreadyExists => Some("EEXIST: file already exists"),
        io::ErrorKind::IsADirectory => Some("EISDIR: illegal operation on a directory"),
        io::ErrorKind::NotADirectory => Some("ENOTDIR: not a directory"),
        _ if is_loop_error(error) => Some("ELOOP: too many symbolic links encountered"),
        _ => None,
    };
    match (described, path) {
        // Node reports a read of a directory without a path: `EISDIR: …, read`.
        (Some(text), _) if error.kind() == io::ErrorKind::IsADirectory && syscall == "read" => {
            format!("{text}, read")
        }
        (Some(text), Some(path)) => format!("{text}, {syscall} '{}'", path.display()),
        (Some(text), None) => format!("{text}, {syscall}"),
        (None, _) => error.to_string(),
    }
}

fn is_loop_error(error: &io::Error) -> bool {
    // ELOOP is 62 on Darwin/BSD and 40 on Linux.
    match error.raw_os_error() {
        Some(62) => cfg!(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "freebsd"
        )),
        Some(40) => cfg!(target_os = "linux"),
        _ => false,
    }
}
