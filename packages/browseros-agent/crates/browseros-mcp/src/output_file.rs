//! Compatibility helpers for tool producers; storage, ownership and lifetime live in OutputStore.
use crate::{framework::OutputFileAccess, output_store::OutputStore};
use std::{
    env,
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub const TOOL_OUTPUT_DIR_MODE: u32 = 0o700;
pub const TOOL_OUTPUT_FILE_MODE: u32 = 0o600;

#[must_use]
pub fn create_browser_output_file_access() -> OutputFileAccess {
    OutputStore::new(output_root(), &Uuid::new_v4().to_string())
}

#[must_use]
pub fn output_root() -> PathBuf {
    env::var_os("BROWSEROS_OUTPUT_DIR")
        .filter(|value| !value.is_empty())
        .map_or_else(|| get_browseros_dir().join("tool-output"), PathBuf::from)
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

pub async fn write_temp_tool_output_file(
    access: &OutputFileAccess,
    tool_name: &str,
    extension: &str,
    content: &str,
) -> std::io::Result<PathBuf> {
    access
        .write_part(tool_name, extension, content.as_bytes())
        .await
}

pub async fn write_temp_tool_output_binary_file(
    access: &OutputFileAccess,
    tool_name: &str,
    extension: &str,
    content: &[u8],
) -> std::io::Result<PathBuf> {
    access.write_part(tool_name, extension, content).await
}

pub async fn record_browser_output_file(
    access: &OutputFileAccess,
    path: PathBuf,
) -> std::io::Result<()> {
    access
        .copy_file("download", Path::new(&path))
        .await
        .map(|_| ())
}
