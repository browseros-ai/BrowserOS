use crate::framework::OutputFileAccess;
use std::{
    collections::HashSet,
    env,
    path::{Path, PathBuf},
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
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(dir_name)
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

pub async fn create_download_output_dir() -> std::io::Result<PathBuf> {
    let output_dir = get_tool_output_dir().await?;
    for _attempt in 0..10 {
        let path = output_dir.join(format!("download-{}", Uuid::new_v4()));
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
        "could not create unique download output directory",
    ))
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
