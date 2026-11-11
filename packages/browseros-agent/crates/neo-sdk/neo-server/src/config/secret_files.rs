//! Neo's own secret files, which `setInputFiles` never hands a page (decisions 33b and 35d): its
//! host token, its CAPTCHA keys, its trace store and the browser profile. They are located once
//! at start, from the launch JSON and the environment, for neo-playwright's deny list
//! (`PlaywrightHooks::secret_paths`). Every other file Neo's process can read uploads, as
//! Playwright does, and the caller sandboxes the filesystem.
//!
//! Each entry is the narrowest path that holds a secret in the shipped layouts, and none holds a
//! download, which an agent uploads again:
//!
//! ```text
//!  HostToken       CLAW_HOST_TOKEN_FILE             file  /run/mirror-neo/token, <rig>/host-token
//!  CaptchaKey      BROWSEROS_*_API_KEY_FILE         file  <keys>/capsolver-api-key, <rig>/…-key
//!  CaptchaKeyDir   $HOME/.config/browseros/captcha  dir   <keys>, the image's 0700 keys-only dir
//!                  (HOME unset, as in an `env -i` launch: the passwd entry's home)
//!  TraceStore      $BROWSERCLAW_DIR/claw-sdk        dir   trace.db, -wal, -shm,
//!                                                         broken stores, screenshots
//!  BrowserProfile  parent(directories.execution)    dir   /data/profile, <rig>/profile
//!  not listed      logs/, host.sock, downloads (/data/downloads, beside every entry)
//! ```

use std::path::PathBuf;
use std::{fs, io};

use super::{Defaults, EnvConfig, LaunchConfig};

/// Where Neo keeps its secrets on disk, or why it cannot say. Built once in `AppConfig::load` and
/// never changed: the crate resolves every entry again on each upload, so a file that appears
/// later inside an entry directory (the trace store's `-wal`) is covered with no second look at
/// the config.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SecretFiles {
    /// Every secret Neo keeps on disk, each at an absolute path. A configured file that does not
    /// exist (yet) stays listed, and the crate skips it until it appears.
    Named(Vec<SecretFile>),
    /// A secret's place is unknown, so no file on Neo's disk may upload: the crate refuses every
    /// upload from a path as `neo / unsupported`. Payloads, which read no file, still upload.
    Unlocated {
        /// The secret Neo cannot place.
        missing: SecretKind,
        /// Why, for the startup log.
        why: String,
    },
}

/// One of Neo's secrets on disk. A plain data bag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SecretFile {
    pub(crate) kind: SecretKind,
    /// Absolute, as the crate takes it: a file, or a directory that denies everything inside it.
    pub(crate) path: PathBuf,
}

/// What a secret file holds, for the startup log and the tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SecretKind {
    /// The `CLAW_HOST_TOKEN_FILE` file itself. Its directory can be shared: zuzu's
    /// `/run/mirror-neo/` also holds the host socket.
    HostToken,
    /// A CAPTCHA provider's key file, from neo-captcha's variable. The file, not its
    /// directory, which can be shared too (the BrowserOS rig's root holds the profile and the
    /// token); the crate's `(dev, inode)` match still catches a link to it.
    CaptchaKey,
    /// The image's CAPTCHA key directory under the user's home (`EnvConfig::home`), which covers
    /// a key baked into the image that no variable names. Skipped only when neither `HOME` nor
    /// the passwd entry names a home.
    CaptchaKeyDir,
    /// The trace store's directory: `trace.db` and its `-wal` and `-shm`, which come and go, the
    /// broken stores moved aside, and the trace screenshots.
    TraceStore,
    /// The browser's user-data directory, the parent of `directories.execution`: the cookies,
    /// saved logins and `Local State` of every profile under it.
    BrowserProfile,
}

impl SecretFiles {
    /// Every entry, from the config. `cwd` is Neo's working directory at start: a relative value
    /// is joined onto it, as the OS reads that value and as the crate resolves a script's
    /// relative paths. A parameter, so the tests lay out the image and the rig as values.
    #[must_use]
    pub(crate) fn locate(
        env: &EnvConfig,
        launch: &LaunchConfig,
        cwd: &io::Result<PathBuf>,
    ) -> Self {
        let Some(profile) = launch.profile_dir() else {
            return Self::Unlocated {
                missing: SecretKind::BrowserProfile,
                why: "the launch config names no `directories.execution`".to_owned(),
            };
        };
        let mut entries = Vec::new();
        if let Some(token) = &env.host_api.token_file {
            entries.push((SecretKind::HostToken, token.clone()));
        }
        for key in &env.captcha_key_files {
            entries.push((SecretKind::CaptchaKey, key.clone()));
        }
        if let Some(home) = &env.home {
            entries.push((
                SecretKind::CaptchaKeyDir,
                home.join(Defaults::CAPTCHA_KEY_DIR),
            ));
        }
        entries.push((SecretKind::TraceStore, env.trace_dir()));
        entries.push((SecretKind::BrowserProfile, profile.to_path_buf()));

        let mut files = Vec::with_capacity(entries.len());
        for (kind, path) in entries {
            let path = match cwd {
                // `join` keeps an absolute path as it is, so only a relative one moves.
                Ok(cwd) => cwd.join(path),
                Err(_) if path.is_absolute() => path,
                Err(error) => {
                    return Self::Unlocated {
                        missing: kind,
                        why: format!(
                            "{} is relative, and Neo has no working directory to place it in: \
                             {error}",
                            path.display()
                        ),
                    };
                }
            };
            files.push(SecretFile { kind, path });
        }
        Self::Named(files)
    }

    /// What the crate's deny list gets (`PlaywrightHooks::secret_paths`): every entry's path, or
    /// `None` while one has no known place, so the crate refuses every upload from a path.
    #[must_use]
    pub(crate) fn paths(&self) -> Option<Vec<PathBuf>> {
        match self {
            Self::Named(files) => Some(files.iter().map(|file| file.path.clone()).collect()),
            Self::Unlocated { .. } => None,
        }
    }
}

impl SecretFile {
    /// Why the crate cannot resolve this entry now, when the reason is not that it is absent. The
    /// crate then refuses every upload from a path, since it cannot tell what the entry names,
    /// and the startup log is the operator's only word on why. An absent entry is fine: the crate
    /// skips it until it appears.
    #[must_use]
    pub(crate) fn unresolvable(&self) -> Option<io::Error> {
        // The crate's own resolution, a realpath and then a stat, and its own test of absence
        // (neo-playwright's `input_files/secrets.rs`).
        let error = fs::canonicalize(&self.path).and_then(fs::metadata).err()?;
        let absent = matches!(
            error.kind(),
            io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
        );
        (!absent).then_some(error)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::error::Error;
    use std::io;
    use std::path::{Path, PathBuf};

    use super::SecretKind::{BrowserProfile, CaptchaKey, CaptchaKeyDir, HostToken, TraceStore};
    use super::{SecretFile, SecretFiles, SecretKind};
    use crate::config::{Defaults, EnvConfig, LaunchConfig};

    /// The environment `vars` would give Neo, for a user with no home in the passwd database.
    fn env(vars: &[(&str, &str)]) -> Result<EnvConfig, Box<dyn Error>> {
        env_for_user(vars, None)
    }

    /// The environment `vars` would give Neo, for a user whose passwd entry's home is
    /// `passwd_home`.
    fn env_for_user(
        vars: &[(&str, &str)],
        passwd_home: Option<&str>,
    ) -> Result<EnvConfig, Box<dyn Error>> {
        let vars: HashMap<&str, &str> = vars.iter().copied().collect();
        Ok(EnvConfig::from_lookup(
            &|name| vars.get(name).map(|value| (*value).to_owned()),
            &|| passwd_home.map(PathBuf::from),
        )?)
    }

    /// The launch facts Chromium writes, with `directories.execution` as given.
    fn launch(execution: Option<&str>) -> LaunchConfig {
        LaunchConfig {
            server_port: Defaults::SERVER_PORT,
            cdp_port: Defaults::CDP_PORT,
            resources_dir: PathBuf::from("/opt/neo/resources"),
            execution_dir: execution.map(PathBuf::from),
            browseros_version: None,
        }
    }

    fn named(entries: &[(SecretKind, &str)]) -> SecretFiles {
        SecretFiles::Named(
            entries
                .iter()
                .map(|&(kind, path)| SecretFile {
                    kind,
                    path: PathBuf::from(path),
                })
                .collect(),
        )
    }

    /// Whether an upload of `path` would meet an entry, as a path inside a directory entry, or
    /// as a directory upload that holds one: lexically, which the layouts' absolute paths allow.
    fn meets_an_entry(files: &SecretFiles, path: &Path) -> bool {
        let SecretFiles::Named(files) = files else {
            return true;
        };
        files
            .iter()
            .any(|file| path.starts_with(&file.path) || file.path.starts_with(path))
    }

    /// The Neo image's layout (Dockerfile and entrypoint.sh), with zuzu's token beside its host
    /// socket. Its cwd is `/`, which no entry needs.
    #[test]
    fn the_image_layout_denies_each_secret_and_no_download() -> Result<(), Box<dyn Error>> {
        let image = env(&[
            ("HOME", "/home/neo"),
            ("BROWSERCLAW_DIR", "/data/browserclaw"),
            ("CLAW_HOST_SOCKET", "/run/mirror-neo/host.sock"),
            ("CLAW_HOST_TOKEN_FILE", "/run/mirror-neo/token"),
            (
                "BROWSEROS_CAPSOLVER_API_KEY_FILE",
                "/home/neo/.config/browseros/captcha/capsolver-api-key",
            ),
            (
                "BROWSEROS_2CAPTCHA_API_KEY_FILE",
                "/home/neo/.config/browseros/captcha/2captcha-api-key",
            ),
        ])?;
        let files = SecretFiles::locate(
            &image,
            &launch(Some("/data/profile/.browseros")),
            &Ok(PathBuf::from("/")),
        );
        assert_eq!(
            files,
            named(&[
                (HostToken, "/run/mirror-neo/token"),
                (
                    CaptchaKey,
                    "/home/neo/.config/browseros/captcha/capsolver-api-key"
                ),
                (
                    CaptchaKey,
                    "/home/neo/.config/browseros/captcha/2captcha-api-key"
                ),
                (CaptchaKeyDir, "/home/neo/.config/browseros/captcha"),
                (TraceStore, "/data/browserclaw/claw-sdk"),
                (BrowserProfile, "/data/profile"),
            ]),
            "each secret by the narrowest path that holds it"
        );
        for kept in [
            "/data/downloads",
            "/data/downloads/report.pdf",
            "/data/browserclaw/logs",
            "/run/mirror-neo/host.sock",
        ] {
            assert!(
                !meets_an_entry(&files, Path::new(kept)),
                "{kept} is outside every entry, so an agent uploads what it downloaded"
            );
        }
        Ok(())
    }

    /// The BrowserOS rig (`tests/support/browseros_rig.rs`) keeps everything in one temp root: the
    /// profile, the state, the token and the CAPTCHA placeholder key. Naming the key's or the
    /// token's directory would deny the whole root.
    #[test]
    fn the_rig_layout_denies_its_token_and_key_but_not_its_root() -> Result<(), Box<dyn Error>> {
        let rig = env(&[
            ("HOME", "/tmp/rig/home"),
            ("BROWSERCLAW_DIR", "/tmp/rig/state"),
            ("CLAW_HOST_SOCKET", "/tmp/rig/host.sock"),
            ("CLAW_HOST_TOKEN_FILE", "/tmp/rig/host-token"),
            ("BROWSEROS_CAPSOLVER_API_KEY_FILE", "/tmp/rig/capsolver-key"),
        ])?;
        let files = SecretFiles::locate(
            &rig,
            &launch(Some("/tmp/rig/profile/.browseros")),
            &Ok(PathBuf::from("/srv/neo")),
        );
        assert_eq!(
            files,
            named(&[
                (HostToken, "/tmp/rig/host-token"),
                (CaptchaKey, "/tmp/rig/capsolver-key"),
                (CaptchaKeyDir, "/tmp/rig/home/.config/browseros/captcha"),
                (TraceStore, "/tmp/rig/state/claw-sdk"),
                (BrowserProfile, "/tmp/rig/profile"),
            ])
        );
        assert!(
            !meets_an_entry(&files, Path::new("/tmp/rig/upload.txt")),
            "a file beside the secrets in the root still uploads"
        );
        Ok(())
    }

    #[test]
    fn a_relative_entry_is_made_absolute_against_the_working_directory()
    -> Result<(), Box<dyn Error>> {
        let relative = env(&[
            ("HOME", "/home/neo"),
            ("BROWSERCLAW_DIR", "state"),
            ("CLAW_HOST_TOKEN_FILE", "secrets/token"),
            ("BROWSEROS_CAPSOLVER_API_KEY_FILE", "./keys/capsolver"),
        ])?;
        let profile = launch(Some("udd/.browseros"));
        assert_eq!(
            SecretFiles::locate(&relative, &profile, &Ok(PathBuf::from("/srv/neo"))),
            named(&[
                (HostToken, "/srv/neo/secrets/token"),
                (CaptchaKey, "/srv/neo/keys/capsolver"),
                (CaptchaKeyDir, "/home/neo/.config/browseros/captcha"),
                (TraceStore, "/srv/neo/state/claw-sdk"),
                (BrowserProfile, "/srv/neo/udd"),
            ]),
            "the files the OS opens for those values, from Neo's working directory"
        );

        let gone = Err(io::Error::from(io::ErrorKind::NotFound));
        let unlocated = SecretFiles::locate(&relative, &profile, &gone);
        assert!(
            matches!(
                &unlocated,
                SecretFiles::Unlocated { missing: HostToken, why } if why.contains("secrets/token")
            ),
            "without a working directory Neo cannot say which file a relative value names, so no \
             path uploads: {unlocated:?}"
        );
        let absolute = env(&[
            ("HOME", "/home/neo"),
            ("BROWSERCLAW_DIR", "/data/browserclaw"),
        ])?;
        assert!(
            matches!(
                SecretFiles::locate(&absolute, &launch(Some("/data/profile/.browseros")), &gone),
                SecretFiles::Named(_)
            ),
            "absolute entries need no working directory"
        );
        Ok(())
    }

    /// A launch through `env -i` has no `HOME`, yet the image's keys still sit under its user's
    /// home, where a key no variable names would otherwise upload.
    #[test]
    fn an_unset_home_falls_back_to_the_users_passwd_home() -> Result<(), Box<dyn Error>> {
        let bare = [
            ("BROWSERCLAW_DIR", "/data/browserclaw"),
            ("BROWSEROS_CAPSOLVER_API_KEY_FILE", "/run/keys/capsolver"),
        ];
        let profile = launch(Some("/data/profile/.browseros"));
        let root = Ok(PathBuf::from("/"));
        assert_eq!(
            SecretFiles::locate(&env_for_user(&bare, Some("/home/neo"))?, &profile, &root),
            named(&[
                (CaptchaKey, "/run/keys/capsolver"),
                (CaptchaKeyDir, "/home/neo/.config/browseros/captcha"),
                (TraceStore, "/data/browserclaw/claw-sdk"),
                (BrowserProfile, "/data/profile"),
            ]),
            "the key dir under the passwd entry's home"
        );
        assert_eq!(
            SecretFiles::locate(&env_for_user(&bare, None)?, &profile, &root),
            named(&[
                (CaptchaKey, "/run/keys/capsolver"),
                (TraceStore, "/data/browserclaw/claw-sdk"),
                (BrowserProfile, "/data/profile"),
            ]),
            "with no home anywhere there is no key dir to name, and the named key is still denied"
        );
        Ok(())
    }

    #[test]
    fn without_the_execution_dir_no_path_uploads() -> Result<(), Box<dyn Error>> {
        let image = env(&[
            ("HOME", "/home/neo"),
            ("BROWSERCLAW_DIR", "/data/browserclaw"),
            ("CLAW_HOST_TOKEN_FILE", "/run/mirror-neo/token"),
        ])?;
        let files = SecretFiles::locate(&image, &launch(None), &Ok(PathBuf::from("/")));
        assert!(
            matches!(
                &files,
                SecretFiles::Unlocated {
                    missing: BrowserProfile,
                    ..
                }
            ),
            "the profile's cookies and logins could be anywhere, so no file on Neo's disk \
             uploads, rather than one the list missed: {files:?}"
        );
        Ok(())
    }
}
