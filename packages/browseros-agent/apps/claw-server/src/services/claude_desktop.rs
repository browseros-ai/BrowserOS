//! Installs the BrowserOS neo extension into the Claude desktop app in one step.
//! The server fetches the pinned `.mcpb`, checks its hash, and opens it with
//! Claude, which shows its own install sheet; nothing installs until the
//! person confirms there. Installed state is read from Claude's own record.

use crate::error::{AppError, AppResult};
use claw_api::models::{ClaudeDesktopExtension, ClaudeDesktopInstallStatus};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Claude derives this from the bundle's author and name.
pub const EXTENSION_ID: &str = "local.mcpb.browseros.browserclaw";

/// The extension release this server installs, pinned by hash.
#[derive(Clone, Debug)]
pub struct ExtensionBundle {
    pub version: String,
    pub url: String,
    pub sha256: String,
}

impl Default for ExtensionBundle {
    fn default() -> Self {
        Self {
            version: "0.5.0".to_string(),
            url: "https://github.com/browseros-ai/browserclaw-claude-desktop/releases/download/v0.5.0/browseros-neo-0.5.0.mcpb".to_string(),
            sha256: "824a909d2e3986fd9a7e7f727c9d1249a99e9c9271adb86b95fec543aeba8819"
                .to_string(),
        }
    }
}

/// Where Claude lives on this computer and where it records installed extensions.
#[derive(Clone, Debug)]
pub struct ClaudeLocations {
    /// The first of these that exists is the Claude app.
    pub app_candidates: Vec<PathBuf>,
    pub installations_file: Option<PathBuf>,
}

impl ClaudeLocations {
    pub fn for_home(home: &Path) -> Self {
        if cfg!(target_os = "macos") {
            let support = home.join("Library/Application Support/Claude");
            Self {
                app_candidates: vec![
                    PathBuf::from("/Applications/Claude.app"),
                    home.join("Applications/Claude.app"),
                ],
                installations_file: Some(support.join("extensions-installations.json")),
            }
        } else if cfg!(target_os = "windows") {
            Self {
                app_candidates: vec![home.join("AppData/Local/AnthropicClaude/claude.exe")],
                installations_file: Some(
                    home.join("AppData/Roaming/Claude/extensions-installations.json"),
                ),
            }
        } else {
            // There is no Claude desktop app for this platform.
            Self {
                app_candidates: Vec::new(),
                installations_file: None,
            }
        }
    }

    fn app(&self) -> Option<&Path> {
        self.app_candidates
            .iter()
            .map(PathBuf::as_path)
            .find(|path| path.exists())
    }
}

/// Hands a file to the Claude app at `app`.
pub type Opener = Arc<dyn Fn(&Path, &Path) -> std::io::Result<()> + Send + Sync>;

pub struct ClaudeDesktop {
    locations: ClaudeLocations,
    bundle: ExtensionBundle,
    cache_dir: PathBuf,
    client: reqwest::Client,
    opener: Opener,
}

impl ClaudeDesktop {
    pub fn new(home: &Path, cache_dir: PathBuf) -> Self {
        Self::with_parts(
            ClaudeLocations::for_home(home),
            ExtensionBundle::default(),
            cache_dir,
            Arc::new(open_with_claude),
        )
    }

    pub fn with_parts(
        locations: ClaudeLocations,
        bundle: ExtensionBundle,
        cache_dir: PathBuf,
        opener: Opener,
    ) -> Self {
        Self {
            locations,
            bundle,
            cache_dir,
            client: reqwest::Client::new(),
            opener,
        }
    }

    pub async fn status(&self) -> ClaudeDesktopExtension {
        let installed_version = match &self.locations.installations_file {
            Some(path) => installed_version(path).await,
            None => None,
        };
        let update_available = installed_version
            .as_deref()
            .is_some_and(|installed| is_older(installed, &self.bundle.version));
        ClaudeDesktopExtension {
            claude_installed: self.locations.app().is_some(),
            installed: installed_version.is_some(),
            installed_version,
            available_version: self.bundle.version.clone(),
            update_available,
        }
    }

    pub async fn install(&self) -> AppResult<ClaudeDesktopInstallStatus> {
        let Some(app) = self.locations.app().map(Path::to_path_buf) else {
            return Ok(ClaudeDesktopInstallStatus::Missing);
        };
        let bundle = self.ensure_bundle().await?;
        let opener = self.opener.clone();
        tokio::task::spawn_blocking(move || opener(&app, &bundle))
            .await?
            .map_err(|source| AppError::Io { path: None, source })?;
        Ok(ClaudeDesktopInstallStatus::Opened)
    }

    /// The cached bundle, fetched first when it is missing or does not match the pinned hash.
    async fn ensure_bundle(&self) -> AppResult<PathBuf> {
        let path = self
            .cache_dir
            .join(format!("browseros-neo-{}.mcpb", self.bundle.version));
        if let Ok(bytes) = tokio::fs::read(&path).await
            && sha256_hex(&bytes) == self.bundle.sha256
        {
            return Ok(path);
        }
        let bytes = self
            .client
            .get(&self.bundle.url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| {
                AppError::Internal(format!("could not download the Claude extension: {error}"))
            })?
            .bytes()
            .await
            .map_err(|error| {
                AppError::Internal(format!("could not download the Claude extension: {error}"))
            })?;
        if sha256_hex(&bytes) != self.bundle.sha256 {
            return Err(AppError::Internal(
                "the downloaded Claude extension did not match its expected hash".to_string(),
            ));
        }
        tokio::fs::create_dir_all(&self.cache_dir)
            .await
            .map_err(|source| AppError::Io {
                path: Some(self.cache_dir.clone()),
                source,
            })?;
        // Written beside the final name and renamed, so a half-written file is never opened.
        let partial = path.with_extension("mcpb.partial");
        tokio::fs::write(&partial, &bytes)
            .await
            .map_err(|source| AppError::Io {
                path: Some(partial.clone()),
                source,
            })?;
        tokio::fs::rename(&partial, &path)
            .await
            .map_err(|source| AppError::Io {
                path: Some(path.clone()),
                source,
            })?;
        Ok(path)
    }
}

#[derive(Deserialize)]
struct Installations {
    #[serde(default)]
    extensions: BTreeMap<String, InstalledExtension>,
}

#[derive(Deserialize)]
struct InstalledExtension {
    version: Option<String>,
}

/// The neo extension's version from Claude's record, or None when it is not installed.
async fn installed_version(path: &Path) -> Option<String> {
    let bytes = tokio::fs::read(path).await.ok()?;
    let installations: Installations = serde_json::from_slice(&bytes).ok()?;
    let extension = installations.extensions.get(EXTENSION_ID)?;
    Some(extension.version.clone().unwrap_or_default())
}

/// Compares dotted versions number by number; anything unreadable counts as older.
fn is_older(installed: &str, available: &str) -> bool {
    let parse = |version: &str| -> Vec<u64> {
        version
            .split(['.', '-'])
            .map_while(|part| part.parse().ok())
            .collect()
    };
    parse(installed) < parse(available)
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Opens `file` with the Claude app, which routes `.mcpb` files to its installer.
fn open_with_claude(app: &Path, file: &Path) -> std::io::Result<()> {
    let mut command = if cfg!(target_os = "windows") {
        let mut command = std::process::Command::new(app);
        command.arg(file);
        command
    } else {
        // `open -a` targets this Claude even if another app has claimed `.mcpb`.
        let mut command = std::process::Command::new("open");
        command.arg("-a").arg(app).arg(file);
        command
    };
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "opening the extension in Claude exited with {status}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    const BYTES: &[u8] = b"neo extension bundle";

    fn bundle() -> ExtensionBundle {
        ExtensionBundle {
            version: "0.5.0".to_string(),
            // Never reached: tests either cache the file or expect the fetch to fail.
            url: "http://127.0.0.1:9/browseros-neo.mcpb".to_string(),
            sha256: sha256_hex(BYTES),
        }
    }

    struct Fixture {
        _root: tempfile::TempDir,
        app: PathBuf,
        installations: PathBuf,
        cache: PathBuf,
        opened: Arc<Mutex<Vec<(PathBuf, PathBuf)>>>,
    }

    fn fixture() -> anyhow::Result<Fixture> {
        let root = tempfile::tempdir()?;
        let app = root.path().join("Claude.app");
        let installations = root.path().join("extensions-installations.json");
        let cache = root.path().join("cache");
        Ok(Fixture {
            app,
            installations,
            cache,
            opened: Arc::default(),
            _root: root,
        })
    }

    fn desktop(fixture: &Fixture) -> ClaudeDesktop {
        let opened = fixture.opened.clone();
        ClaudeDesktop::with_parts(
            ClaudeLocations {
                app_candidates: vec![fixture.app.clone()],
                installations_file: Some(fixture.installations.clone()),
            },
            bundle(),
            fixture.cache.clone(),
            Arc::new(move |app, file| {
                opened
                    .lock()
                    .map_err(|_| std::io::Error::other("poisoned"))?
                    .push((app.to_path_buf(), file.to_path_buf()));
                Ok(())
            }),
        )
    }

    #[tokio::test]
    async fn reports_claude_missing_without_touching_anything() -> anyhow::Result<()> {
        let fixture = fixture()?;
        let desktop = desktop(&fixture);

        assert_eq!(
            desktop.install().await?,
            ClaudeDesktopInstallStatus::Missing
        );
        let status = desktop.status().await;
        assert!(!status.claude_installed);
        assert!(!status.installed);
        assert!(fixture.opened.lock().is_ok_and(|opened| opened.is_empty()));
        Ok(())
    }

    #[tokio::test]
    async fn opens_the_cached_bundle_in_claude() -> anyhow::Result<()> {
        let fixture = fixture()?;
        std::fs::create_dir_all(&fixture.app)?;
        std::fs::create_dir_all(&fixture.cache)?;
        let cached = fixture.cache.join("browseros-neo-0.5.0.mcpb");
        std::fs::write(&cached, BYTES)?;

        assert_eq!(
            desktop(&fixture).install().await?,
            ClaudeDesktopInstallStatus::Opened
        );
        assert!(
            fixture
                .opened
                .lock()
                .is_ok_and(|opened| *opened == [(fixture.app.clone(), cached.clone())])
        );
        Ok(())
    }

    #[tokio::test]
    async fn refuses_a_bundle_it_cannot_verify() -> anyhow::Result<()> {
        let fixture = fixture()?;
        std::fs::create_dir_all(&fixture.app)?;
        std::fs::create_dir_all(&fixture.cache)?;
        std::fs::write(fixture.cache.join("browseros-neo-0.5.0.mcpb"), b"tampered")?;

        assert!(desktop(&fixture).install().await.is_err());
        assert!(fixture.opened.lock().is_ok_and(|opened| opened.is_empty()));
        Ok(())
    }

    #[tokio::test]
    async fn reads_the_installed_version_and_offers_updates() -> anyhow::Result<()> {
        let fixture = fixture()?;
        std::fs::create_dir_all(&fixture.app)?;
        std::fs::write(
            &fixture.installations,
            r#"{"extensions":{"local.mcpb.browseros.browserclaw":{"id":"local.mcpb.browseros.browserclaw","version":"0.4.0"},"local.mcpb.other.tool":{"version":"9.9.9"}}}"#,
        )?;

        let status = desktop(&fixture).status().await;
        assert!(status.claude_installed);
        assert!(status.installed);
        assert_eq!(status.installed_version.as_deref(), Some("0.4.0"));
        assert_eq!(status.available_version, "0.5.0");
        assert!(status.update_available);
        Ok(())
    }

    #[test]
    fn compares_versions_by_number() {
        assert!(is_older("0.4.0", "0.5.0"));
        assert!(is_older("0.9.0", "0.10.0"));
        assert!(!is_older("0.5.0", "0.5.0"));
        assert!(!is_older("1.0.0", "0.5.0"));
        assert!(is_older("", "0.5.0"));
    }
}
