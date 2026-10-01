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
#[must_use]
pub fn get_user_download_dir() -> PathBuf {
    for key in ["BROWSEROS_DOWNLOAD_DIR", "XDG_DOWNLOAD_DIR"] {
        if let Some(dir) = env::var_os(key).filter(|value| !value.is_empty()) {
            return PathBuf::from(dir);
        }
    }
    home_dir().join("Downloads")
}

/// Checks a caller-supplied download directory before anything is written to it.
///
/// Absolute, no `..`, and inside the user's home directory. The home bound is
/// not about the loopback-only endpoint it sits behind today; it is so that a
/// caller cannot aim a download at a system location if that ever changes. It
/// still admits an agent's own working directory, which is the case this
/// argument exists for.
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
    let home = home_dir();
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

    #[test]
    fn accepts_a_directory_inside_the_users_home() {
        let project = home_dir().join("projects").join("my-app");

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
        let traversal = home_dir().join("..").join("elsewhere");

        let Err(error) = validate_download_dir(&traversal.to_string_lossy()) else {
            panic!("traversal hides the real destination and must be refused");
        };

        assert!(error.contains(".."));
    }

    #[test]
    fn rejects_a_system_directory_outside_home() {
        let Err(error) = validate_download_dir("/Library/LaunchDaemons") else {
            panic!("a download must not be aimed at a system location");
        };

        assert!(error.contains("must be inside"));
    }
}
