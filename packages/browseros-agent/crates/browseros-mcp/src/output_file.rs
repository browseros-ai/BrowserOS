use crate::framework::OutputFileAccess;
use std::{
    collections::HashSet,
    env,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{fs, io::AsyncWriteExt, sync::Mutex};
use uuid::Uuid;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub const TOOL_OUTPUT_DIR_MODE: u32 = 0o700;
pub const TOOL_OUTPUT_FILE_MODE: u32 = 0o600;

#[must_use]
pub fn create_browser_output_file_access() -> OutputFileAccess {
    Arc::new(Mutex::new(HashSet::new()))
}

/// The user's home directory, or `None` when the environment does not say.
///
/// `USERPROFILE` as well as `HOME`, so Windows resolves somewhere real instead
/// of falling through to a path relative to the process.
fn user_home() -> Option<PathBuf> {
    for key in ["HOME", "USERPROFILE"] {
        if let Some(value) = env::var_os(key).filter(|value| !value.is_empty()) {
            return Some(PathBuf::from(value));
        }
    }
    None
}

fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[must_use]
pub fn get_browseros_dir() -> PathBuf {
    if let Some(override_dir) = env::var_os("BROWSEROS_DIR")
        && !override_dir.is_empty()
    {
        return PathBuf::from(override_dir);
    }
    let dir_name = if cfg!(debug_assertions) {
        ".browseros-dev"
    } else {
        ".browseros"
    };
    home_dir().join(dir_name)
}

/// Where the browser puts downloads, which is where the user looks for them.
///
/// Read from the environment rather than from the browser's own
/// `download.default_directory` preference: the server is never told the
/// browser's profile path, so a relocated download folder still resolves to the
/// platform default. `BROWSEROS_DOWNLOAD_DIR` is the override until the browser
/// passes its profile through.
/// Falls back to the tool output directory rather than to a relative path. A
/// download has to land somewhere the user can find, and "./Downloads" would be
/// wherever the server process happened to start.
pub async fn get_user_download_dir() -> std::io::Result<PathBuf> {
    for key in ["BROWSEROS_DOWNLOAD_DIR", "XDG_DOWNLOAD_DIR"] {
        if let Some(dir) = env::var_os(key).filter(|value| !value.is_empty()) {
            return Ok(PathBuf::from(dir));
        }
    }
    match user_home() {
        Some(home) => Ok(home.join("Downloads")),
        None => get_tool_output_dir().await,
    }
}

/// Checks a caller-supplied download directory before anything is written to it.
///
/// Absolute, no `..`, and inside the user's home directory. The home bound is
/// not about the loopback-only endpoint it sits behind today; it is so that a
/// caller cannot aim a download at a system location if that ever changes. It
/// still admits an agent's own working directory, which is the case this
/// argument exists for.
///
/// Text only. A symlink pointing out of home passes here and is caught by
/// `ensure_download_dir_inside_home` once the path can be canonicalised.
pub fn validate_download_dir(requested: &str) -> Result<PathBuf, String> {
    let path = Path::new(requested);
    if !path.is_absolute() {
        return Err(format!(
            "download dir must be an absolute path, got \"{requested}\""
        ));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("download dir must not contain \"..\"".to_string());
    }
    let Some(home) = user_home() else {
        return Err("download dir cannot be checked: no home directory is set".to_string());
    };
    if !path.starts_with(&home) {
        return Err(format!(
            "download dir must be inside {}, got \"{requested}\"",
            home.display()
        ));
    }
    Ok(path.to_path_buf())
}

/// Creates the destination if needed and returns it resolved.
pub async fn prepare_download_dir(dir: PathBuf) -> std::io::Result<PathBuf> {
    fs::create_dir_all(&dir).await?;
    fs::canonicalize(dir).await
}

/// The destination with its existing ancestors resolved.
///
/// A destination that does not exist yet cannot be canonicalised, so the deepest
/// ancestor that does exist is resolved and the remainder reattached. That is
/// enough to see through a symlink before anything is created.
async fn resolve_existing_prefix(dir: &Path) -> PathBuf {
    let mut existing = dir;
    loop {
        if let Ok(real) = fs::canonicalize(existing).await {
            return match dir.strip_prefix(existing) {
                Ok(remainder) => real.join(remainder),
                Err(_) => real,
            };
        }
        match existing.parent() {
            Some(parent) => existing = parent,
            None => return dir.to_path_buf(),
        }
    }
}

/// Re-checks a resolved destination against the home directory.
///
/// `validate_download_dir` reads the path as text and so cannot see a symlink
/// inside home that points out of it. This resolves the path first, which is the
/// only form that answers where writes land, and runs before the destination is
/// created so a refused request leaves no directories behind.
pub async fn ensure_download_dir_inside_home(dir: &Path) -> Result<(), String> {
    let Some(home) = user_home() else {
        return Err("download dir cannot be checked: no home directory is set".to_string());
    };
    let home = fs::canonicalize(&home).await.unwrap_or(home);
    let resolved = resolve_existing_prefix(dir).await;
    if resolved.starts_with(&home) {
        return Ok(());
    }
    Err(format!(
        "download dir resolves to {}, which is outside {}",
        resolved.display(),
        home.display()
    ))
}

/// A fresh empty directory inside `parent`, for a download to land in alone.
///
/// A managed download writes straight to the path it is given and replaces
/// whatever is there, because overriding the behaviour is what skips the
/// browser's own target determination, and that is the part that renames around
/// a collision. Giving it an empty directory of its own is what makes the write
/// safe; the file is moved to its final name afterwards. Inside the destination
/// so that move is a rename on the same filesystem rather than a copy.
pub async fn create_download_staging_dir(parent: &Path) -> std::io::Result<PathBuf> {
    for _attempt in 0..10 {
        let path = parent.join(format!(".browseros-download-{}", Uuid::new_v4()));
        match fs::create_dir(&path).await {
            Ok(()) => {
                #[cfg(unix)]
                fs::set_permissions(&path, std::fs::Permissions::from_mode(TOOL_OUTPUT_DIR_MODE))
                    .await?;
                return Ok(path);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not create a unique download staging directory",
    ))
}

/// A name in `dir` that is free, renaming around a collision the way the browser
/// would: "report.pdf", then "report (1).pdf", and so on.
///
/// The counter goes before the whole extension, so "archive.tar.gz" becomes
/// "archive (1).tar.gz". That matches what the browser does, which was checked
/// against it rather than assumed.
pub async fn free_name_in(dir: &Path, name: &str) -> std::io::Result<String> {
    if !fs::try_exists(dir.join(name)).await? {
        return Ok(name.to_string());
    }
    let (stem, extension) = split_download_name(name);
    for counter in 1..10_000 {
        let candidate = format!("{stem} ({counter}){extension}");
        if !fs::try_exists(dir.join(&candidate)).await? {
            return Ok(candidate);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        format!("no free name for \"{name}\" in {}", dir.display()),
    ))
}

/// Splits a download name so a counter can go between the halves.
///
/// The extension starts at the first dot of the trailing run of extensions, so
/// "archive.tar.gz" splits into "archive" and ".tar.gz" and a leading dot stays
/// part of the stem.
fn split_download_name(name: &str) -> (String, String) {
    let trimmed = name.trim_start_matches('.');
    let leading_dots = name.len() - trimmed.len();
    match trimmed.find('.') {
        Some(index) if index > 0 => {
            let split = leading_dots + index;
            (name[..split].to_string(), name[split..].to_string())
        }
        _ => (name.to_string(), String::new()),
    }
}

/// The file names directly inside a directory, for spotting what a download added.
///
/// Partial downloads are skipped so a `.crdownload` seen mid-write is never
/// mistaken for the finished file.
pub async fn read_download_dir_names(dir: &Path) -> std::io::Result<HashSet<String>> {
    let mut names = HashSet::new();
    let mut entries = fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".crdownload") {
            names.insert(name);
        }
    }
    Ok(names)
}

pub async fn get_tool_output_dir() -> std::io::Result<PathBuf> {
    let output_dir = get_browseros_dir().join("tool-output");
    fs::create_dir_all(&output_dir).await?;
    let metadata = fs::symlink_metadata(&output_dir).await?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(std::io::Error::other(
            "BrowserOS tool output directory must be a real directory.",
        ));
    }
    let real = fs::canonicalize(output_dir).await?;
    #[cfg(unix)]
    fs::set_permissions(&real, std::fs::Permissions::from_mode(TOOL_OUTPUT_DIR_MODE)).await?;
    Ok(real)
}

pub async fn write_temp_tool_output_file(
    access: &OutputFileAccess,
    tool_name: &str,
    extension: &str,
    content: &str,
) -> std::io::Result<PathBuf> {
    let path = unique_output_path(&get_tool_output_dir().await?, tool_name, extension);
    write_tool_output_file(&path, content.as_bytes()).await?;
    record_browser_output_file(access, path.clone()).await;
    Ok(path)
}

pub async fn write_temp_tool_output_binary_file(
    access: &OutputFileAccess,
    tool_name: &str,
    extension: &str,
    content: &[u8],
) -> std::io::Result<PathBuf> {
    let path = unique_output_path(&get_tool_output_dir().await?, tool_name, extension);
    write_tool_output_file(&path, content).await?;
    record_browser_output_file(access, path.clone()).await;
    Ok(path)
}

pub async fn record_browser_output_file(access: &OutputFileAccess, path: PathBuf) {
    access.lock().await.insert(path);
}

fn sanitize_segment(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if sanitized.is_empty() {
        "browser-tool-output".to_string()
    } else {
        sanitized
    }
}

fn unique_output_path(output_dir: &Path, tool_name: &str, extension: &str) -> PathBuf {
    let epoch_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    output_dir.join(format!(
        "{}-{epoch_ms}-{}.{}",
        sanitize_segment(tool_name),
        Uuid::new_v4(),
        sanitize_segment(extension),
    ))
}

async fn write_tool_output_file(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(TOOL_OUTPUT_FILE_MODE);
    let mut file = options.open(path).await?;
    file.write_all(content).await?;
    file.flush().await?;
    drop(file);
    #[cfg(unix)]
    fs::set_permissions(path, std::fs::Permissions::from_mode(TOOL_OUTPUT_FILE_MODE)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn test_home() -> PathBuf {
        user_home().unwrap_or_else(|| panic!("these tests need a home directory"))
    }

    #[test]
    fn accepts_a_directory_inside_the_users_home() {
        let project = test_home().join("projects").join("my-app");

        let resolved = validate_download_dir(&project.to_string_lossy())
            .unwrap_or_else(|err| panic!("a directory under home should be accepted: {err}"));

        assert_eq!(resolved, project);
    }

    #[test]
    fn rejects_a_relative_directory() {
        let Err(error) = validate_download_dir("Downloads") else {
            panic!("a relative path does not name one destination and must be refused");
        };

        assert!(error.contains("absolute"));
    }

    #[test]
    fn rejects_traversal_even_when_it_lands_inside_home() {
        let traversal = test_home().join("..").join("elsewhere");

        let Err(error) = validate_download_dir(&traversal.to_string_lossy()) else {
            panic!("traversal hides the real destination and must be refused");
        };

        assert!(error.contains(".."));
    }

    #[tokio::test]
    async fn accepts_a_resolved_directory_inside_home() {
        let inside = test_home();

        if let Err(err) = ensure_download_dir_inside_home(&inside).await {
            panic!("the home directory itself should be accepted: {err}");
        }
    }

    #[tokio::test]
    async fn rejects_a_resolved_directory_that_escaped_home() {
        // What the text check cannot see: a symlink under home whose target is
        // outside it. Only the canonical path answers where writes land.
        let Err(error) = ensure_download_dir_inside_home(Path::new("/tmp")).await else {
            panic!("a destination resolving outside home must be refused");
        };

        assert!(error.contains("outside"));
    }

    #[tokio::test]
    async fn sees_through_a_symlink_before_the_destination_exists() {
        // A path under home whose parent is a symlink out of it. The text check
        // passes, so this is the one that has to catch it, and it has to catch it
        // without creating anything.
        let Some(home) = user_home() else {
            panic!("these tests need a home directory");
        };
        let link = home.join(format!("browseros-escape-{}", Uuid::new_v4()));
        let outside = std::env::temp_dir().join(format!("browseros-out-{}", Uuid::new_v4()));
        if let Err(err) = fs::create_dir_all(&outside).await {
            panic!("test target directory should be creatable: {err}");
        }
        #[cfg(unix)]
        if let Err(err) = tokio::fs::symlink(&outside, &link).await {
            panic!("test symlink should be creatable: {err}");
        }
        let through_link = link.join("nested").join("deeper");

        let verdict = ensure_download_dir_inside_home(&through_link).await;

        let _ = fs::remove_file(&link).await;
        let _ = fs::remove_dir_all(&outside).await;
        #[cfg(unix)]
        {
            let Err(error) = verdict else {
                panic!("a destination reached through a symlink out of home must be refused");
            };
            assert!(error.contains("outside"));
            assert!(
                !through_link.exists(),
                "a refused destination must not have been created"
            );
        }
    }

    #[tokio::test]
    async fn picks_a_free_name_the_way_the_browser_would() {
        let dir = std::env::temp_dir().join(format!("free-{}", Uuid::new_v4()));
        if let Err(err) = fs::create_dir_all(&dir).await {
            panic!("scratch directory should be creatable: {err}");
        }

        let first = free_name_in(&dir, "report.pdf").await;
        assert_eq!(first.unwrap_or_default(), "report.pdf");

        if let Err(err) = fs::write(dir.join("report.pdf"), b"taken").await {
            panic!("scratch file should be writable: {err}");
        }
        let second = free_name_in(&dir, "report.pdf").await;
        assert_eq!(second.unwrap_or_default(), "report (1).pdf");

        if let Err(err) = fs::write(dir.join("report (1).pdf"), b"taken").await {
            panic!("scratch file should be writable: {err}");
        }
        let third = free_name_in(&dir, "report.pdf").await;
        assert_eq!(third.unwrap_or_default(), "report (2).pdf");

        let _ = fs::remove_dir_all(&dir).await;
    }

    #[test]
    fn keeps_a_compound_extension_whole() {
        // Checked against the browser: "archive.tar.gz" becomes
        // "archive (1).tar.gz", not "archive.tar (1).gz".
        assert_eq!(
            split_download_name("archive.tar.gz"),
            ("archive".to_string(), ".tar.gz".to_string())
        );
        assert_eq!(
            split_download_name("report.pdf"),
            ("report".to_string(), ".pdf".to_string())
        );
        assert_eq!(
            split_download_name("no-extension"),
            ("no-extension".to_string(), String::new())
        );
        // A leading dot belongs to the name, not to an extension.
        assert_eq!(
            split_download_name(".bashrc"),
            (".bashrc".to_string(), String::new())
        );
    }

    #[test]
    fn rejects_a_system_directory_outside_home() {
        let Err(error) = validate_download_dir("/Library/LaunchDaemons") else {
            panic!("a download must not be aimed at a system location");
        };

        assert!(error.contains("must be inside"));
    }
}
