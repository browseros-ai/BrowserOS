//! Filesystem authorization at the host boundary. The library owns path semantics; the app
//! supplies live read roots and fixed locations that scripts must never read or overwrite.

use std::collections::VecDeque;
use std::error::Error;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::{env, fmt, io};

use tokio::fs;

/// A host's file policy. Roots are read on each use, including after a locator has waited,
/// while protected locations are independent of settings and cannot be allowed by `/`.
pub struct FileAccess {
    roots: Arc<dyn Fn() -> Vec<PathBuf> + Send + Sync>,
    own: Vec<ProtectedPath>,
}

impl fmt::Debug for FileAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileAccess").finish_non_exhaustive()
    }
}

/// Neo-owned locations. A directory protects its entire tree; a file protects exactly its
/// target, including when the configured location has not been created yet.
#[derive(Clone, Debug)]
pub enum ProtectedPath {
    /// The state directory or browser profile and everything underneath it.
    Directory(PathBuf),
    /// A host token or CAPTCHA key file.
    File(PathBuf),
    /// The home of the user Neo's server and browser run as, and its whole tree. The browser,
    /// its crash reporter, fontconfig and GTK read files there without being asked (the
    /// certificate store `~/.pki/nssdb`, the crash reporter's upload settings, font settings),
    /// so a script must not plant or replace one.
    ///
    /// A folder the host allows lifts this rule for its own tree, judged by real path, when it
    /// lies inside the home and its first name below the home does not start with a dot. The
    /// allowed folders are a live read root, or one of `open`. Lifting it changes nothing else:
    /// a read still needs a read root, and Neo's other places stay refused inside the folder.
    Home {
        /// The home, as the processes' `HOME` names it.
        path: PathBuf,
        /// Folders the host allows besides the read roots: the downloads folder.
        open: Vec<PathBuf>,
    },
}

/// An authorized read's IO spelling. Canonical paths are private authorization facts, so
/// Chromium receives a symlink alias's original basename rather than the target's name.
#[derive(Debug)]
pub struct ReadPath {
    original: PathBuf,
    directory: bool,
}

impl ReadPath {
    /// The spelling authorized for the filesystem operation.
    #[must_use]
    pub fn original(&self) -> &Path {
        &self.original
    }

    /// Whether the existing target is a directory.
    #[must_use]
    pub fn is_dir(&self) -> bool {
        self.directory
    }

    pub(crate) async fn unchecked(path: &Path) -> Self {
        Self {
            original: path.to_owned(),
            directory: fs::metadata(path).await.is_ok_and(|meta| meta.is_dir()),
        }
    }
}

/// A destination checked before the operation marks delivery or creates parent directories.
#[derive(Debug)]
pub struct WritePath {
    original: PathBuf,
}

impl WritePath {
    /// The spelling authorized for the filesystem operation.
    #[must_use]
    pub fn original(&self) -> &Path {
        &self.original
    }

    pub(crate) fn unchecked(path: &Path) -> Self {
        Self {
            original: path.to_owned(),
        }
    }
}

/// A path refusal deliberately carries no path or OS error. These messages reach untrusted
/// scripts, which must not learn a protected target through an error instead of a file read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileAccessError {
    /// The current read roots do not contain this path.
    OutsideReadDirs,
    /// The path would expose or replace a Neo-owned location.
    OwnFiles,
    /// The path cannot be resolved safely, such as a dangling or cyclic symlink.
    Unresolvable,
}

impl fmt::Display for FileAccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::OutsideReadDirs => "file path is outside files.readDirs; use an allowed folder or a { name, mimeType, buffer } payload",
            Self::OwnFiles => "scripts cannot read or write Neo's own files",
            Self::Unresolvable => "file path cannot be safely checked against files.readDirs or Neo's own files",
        })
    }
}

impl Error for FileAccessError {}

impl FileAccess {
    /// Builds one policy without reading the process environment or depending on a settings
    /// crate. The callback should clone the live roots under the host's settings lock.
    #[must_use]
    pub fn new(
        roots: impl Fn() -> Vec<PathBuf> + Send + Sync + 'static,
        own: Vec<ProtectedPath>,
    ) -> Self {
        Self {
            roots: Arc::new(roots),
            own,
        }
    }

    /// Authorizes the exact path the gateway will read. Missing ordinary files still pass
    /// authorization inside a root, letting the existing gateway preserve Playwright's IO
    /// errors. A missing root itself never grants access.
    pub async fn read(&self, path: &Path) -> Result<ReadPath, FileAccessError> {
        let roots = (self.roots)();
        let resolved = resolve(path, ResolveMode::Candidate).await?;
        let target = resolved
            .target
            .as_deref()
            .ok_or(FileAccessError::Unresolvable)?;
        let directory = match fs::metadata(path).await {
            Ok(metadata) => metadata.is_dir(),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                false
            }
            Err(_error) => return Err(FileAccessError::Unresolvable),
        };
        let own = self.protected_paths(&roots).await;
        check_own(target, Checked::Read { directory }, &own)?;
        for root in roots {
            let Ok(root) = fs::canonicalize(root).await else {
                continue;
            };
            if fs::metadata(&root).await.is_ok_and(|meta| meta.is_dir()) && target.starts_with(root)
            {
                return Ok(ReadPath {
                    original: path.to_owned(),
                    directory,
                });
            }
        }
        Err(FileAccessError::OutsideReadDirs)
    }

    /// Authorizes a destination, including missing tails and final symlinks. Writes outside
    /// Neo's own locations remain unrestricted; read roots are not a write allowlist.
    ///
    /// The check judges path names, and the save then makes its folders and writes by path name.
    /// It protects against a script, whose only writes are regular files and the folders their
    /// saves make, and assumes that only Neo and the host create links under a listed folder or
    /// the home: it does not cover a hostile process in the container that plants a link between
    /// this check and the write.
    pub async fn write(&self, path: &Path) -> Result<WritePath, FileAccessError> {
        let resolved = resolve(path, ResolveMode::Candidate).await?;
        let target = resolved
            .target
            .as_deref()
            .ok_or(FileAccessError::Unresolvable)?;
        let own = self.protected_paths(&(self.roots)()).await;
        check_own(target, Checked::Write, &own)?;
        // mkdir walks the original spelling too. A missing component before `..` may be
        // created even though it disappears from the final target, so authorize those writes
        // separately. Existing protected parents can still be traversed without changing them.
        for parent in &resolved.prospective_parents {
            check_own(parent, Checked::MadeParent, &own)?;
        }
        Ok(WritePath::unchecked(path))
    }

    /// Every protected spelling, resolved now. A home carries, beside each of its spellings,
    /// the real paths of the allowed folders that lift its rule (`opens_home`), taken from the
    /// same `roots` snapshot the read checks against.
    async fn protected_paths(&self, roots: &[PathBuf]) -> Vec<ProtectedPath> {
        let mut paths = Vec::new();
        for own in &self.own {
            let configured = match own {
                ProtectedPath::Directory(path)
                | ProtectedPath::File(path)
                | ProtectedPath::Home { path, .. } => path,
            };
            let mut allowed = Vec::new();
            if let ProtectedPath::Home { open, .. } = own {
                // A folder not made yet is judged by the real path it will have, so the first
                // save into the downloads folder can make it. A read still needs an existing
                // read root. A file, or a link that resolves nowhere, lifts nothing.
                for folder in roots.iter().chain(open) {
                    let Ok(Resolution {
                        target: Some(real), ..
                    }) = resolve(folder, ResolveMode::Protected).await
                    else {
                        continue;
                    };
                    if fs::metadata(&real).await.is_ok_and(|meta| !meta.is_dir()) {
                        continue;
                    }
                    allowed.push(real);
                }
            }
            // An unusable credential must not disable unrelated file operations. Protection
            // resolution follows dangling links to their future target and retains every known
            // alias even if a cycle or unreadable link leaves no resolvable final target.
            let resolved = resolve(configured, ResolveMode::Protected)
                .await
                .unwrap_or_else(|_error| Resolution {
                    target: None,
                    prospective_parents: Vec::new(),
                    aliases: vec![configured.clone()],
                });
            for path in resolved.aliases.into_iter().chain(resolved.target) {
                paths.push(match own {
                    ProtectedPath::Directory(_) => ProtectedPath::Directory(path),
                    ProtectedPath::File(_) => ProtectedPath::File(path),
                    ProtectedPath::Home { .. } => ProtectedPath::Home {
                        open: allowed
                            .iter()
                            .filter(|folder| opens_home(&path, folder))
                            .cloned()
                            .collect(),
                        path,
                    },
                });
            }
        }
        paths
    }
}

/// Whether an allowed folder's real path lifts the home rule: it lies inside `home`, and its
/// first name below `home` is not a dot-entry (`.config`, `.pki`, `.cache`), where the browser
/// and its helpers keep what they read. So `/`, `/home` or the home itself lift nothing, and a
/// dot name deeper down, inside a lifted folder, belongs to that folder.
fn opens_home(home: &Path, folder: &Path) -> bool {
    folder
        .strip_prefix(home)
        .ok()
        .and_then(|below| below.components().next())
        .is_some_and(|first| match first {
            Component::Normal(name) => !name.as_encoded_bytes().starts_with(b"."),
            _ => false,
        })
}

/// What a checked path is for: the home rule lifts differently for each.
#[derive(Clone, Copy)]
enum Checked {
    /// A read; `directory` when it is a folder, whose whole tree an upload hands the browser.
    Read { directory: bool },
    /// The path a write lands on.
    Write,
    /// A missing folder a write makes on its way (`Resolution::prospective_parents`).
    MadeParent,
}

fn check_own(
    candidate: &Path,
    checked: Checked,
    own: &[ProtectedPath],
) -> Result<(), FileAccessError> {
    let directory = matches!(checked, Checked::Read { directory: true });
    for own in own {
        let (path, tree, open) = match own {
            ProtectedPath::Directory(path) => (path, true, &[][..]),
            ProtectedPath::File(path) => (path, false, &[][..]),
            ProtectedPath::Home { path, open } => (path, true, open.as_slice()),
        };
        // Inside an open folder the home rule is lifted. A folder a write must make on its way
        // to one is too (the file-access design's rule for a nested downloads or listed
        // folder): a missing parent strictly inside the home that is an ancestor of an open
        // folder. Only the home has open folders, and a file never takes this way, so Neo's
        // other places stay refused and nothing else can be written beside that path.
        let lifted = open.iter().any(|folder| {
            candidate.starts_with(folder)
                || (matches!(checked, Checked::MadeParent)
                    && candidate != path
                    && candidate.starts_with(path)
                    && folder.starts_with(candidate))
        });
        // Chromium receives a whole directory upload and cannot omit ordinary protected
        // descendants. Writes check containment/equality only, including prospective parents.
        let tree_hit = tree && candidate.starts_with(path) && !lifted;
        if tree_hit || candidate == path || (directory && path.starts_with(candidate)) {
            return Err(FileAccessError::OwnFiles);
        }
    }
    Ok(())
}

/// Candidate IO fails closed on unusable links. Configured protections instead follow link
/// contents through missing tails, since an absent token/key may be created after startup.
#[derive(Clone, Copy)]
enum ResolveMode {
    Candidate,
    Protected,
}

/// One component walk records both the final target and transient mkdir effects. The latter
/// survive `..` cancellation. A protected path also retains configured and intermediate alias
/// names; its target is absent only when an unusable link prevents complete resolution.
struct Resolution {
    target: Option<PathBuf>,
    prospective_parents: Vec<PathBuf>,
    aliases: Vec<PathBuf>,
}

impl Resolution {
    fn incomplete(
        mut self,
        mode: ResolveMode,
        prefix: &Path,
        pending: &VecDeque<PathBuf>,
    ) -> Result<Self, FileAccessError> {
        match mode {
            ResolveMode::Candidate => Err(FileAccessError::Unresolvable),
            ResolveMode::Protected => {
                self.aliases.push(with_tail(prefix, pending));
                Ok(self)
            }
        }
    }
}

fn with_tail(prefix: &Path, pending: &VecDeque<PathBuf>) -> PathBuf {
    let mut path = prefix.to_owned();
    for part in pending {
        path.push(part);
    }
    path
}

/// Walks in OS order instead of normalizing `..` ahead of symlinks. Missing components are
/// recorded before they can be cancelled, and each later component is probed again. This
/// catches an existing symlink reached after `missing/..` and protects mkdir's transient tail.
async fn resolve(path: &Path, mode: ResolveMode) -> Result<Resolution, FileAccessError> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        env::current_dir()
            .map_err(|_error| FileAccessError::Unresolvable)?
            .join(path)
    };
    let mut resolution = Resolution {
        target: None,
        prospective_parents: Vec::new(),
        aliases: vec![absolute.clone()],
    };
    let mut pending: VecDeque<_> = absolute
        .components()
        .map(|part| PathBuf::from(part.as_os_str()))
        .collect();
    let mut resolved = PathBuf::new();
    let mut links = 0;
    while let Some(part) = pending.pop_front() {
        match part.components().next() {
            Some(Component::Prefix(prefix)) => resolved.push(prefix.as_os_str()),
            Some(Component::RootDir) => resolved.push(part.as_os_str()),
            Some(Component::CurDir) | None => {}
            Some(Component::ParentDir) => {
                resolved.pop();
            }
            Some(Component::Normal(name)) => {
                resolved.push(name);
                match fs::symlink_metadata(&resolved).await {
                    Ok(metadata)
                        if metadata.file_type().is_symlink()
                            && matches!(mode, ResolveMode::Protected) =>
                    {
                        resolution.aliases.push(with_tail(&resolved, &pending));
                        links += 1;
                        if links > 40 {
                            return resolution.incomplete(mode, &resolved, &pending);
                        }
                        let target = match fs::read_link(&resolved).await {
                            Ok(target) => target,
                            Err(_error) => return resolution.incomplete(mode, &resolved, &pending),
                        };
                        resolved.pop();
                        for part in target.components().rev() {
                            pending.push_front(PathBuf::from(part.as_os_str()));
                        }
                    }
                    Ok(_) => {
                        resolved = match fs::canonicalize(&resolved).await {
                            Ok(target) => target,
                            Err(_error) => return resolution.incomplete(mode, &resolved, &pending),
                        };
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                        ) =>
                    {
                        if !pending.is_empty() {
                            resolution.prospective_parents.push(resolved.clone());
                        }
                    }
                    Err(_error) => return resolution.incomplete(mode, &resolved, &pending),
                }
            }
        }
    }
    resolution.target = Some(resolved);
    Ok(resolution)
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, PoisonError, RwLock};

    use tempfile::TempDir;
    use tokio::fs;

    use super::{FileAccess, FileAccessError, ProtectedPath};

    #[tokio::test]
    async fn roots_are_live_component_scoped_and_resolve_symlinks() -> Result<(), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let allowed = dir.path().join("allowed");
        let sibling = dir.path().join("allowed-old");
        fs::create_dir(&allowed).await?;
        fs::create_dir(&sibling).await?;
        let file = allowed.join("original.txt");
        let outside = sibling.join("outside.txt");
        fs::write(&file, "inside").await?;
        fs::write(&outside, "outside").await?;
        symlink(&file, allowed.join("alias.txt"))?;
        symlink(&outside, allowed.join("escape.txt"))?;
        let roots = Arc::new(RwLock::new(vec![allowed.clone()]));
        let live = Arc::clone(&roots);
        let policy = FileAccess::new(
            move || live.read().unwrap_or_else(PoisonError::into_inner).clone(),
            Vec::new(),
        );
        let alias = allowed.join("alias.txt");
        let read = policy.read(&alias).await?;
        assert_eq!(read.original(), alias);
        assert!(!read.is_dir());
        assert!(policy.read(&allowed).await?.is_dir());
        for path in [
            outside,
            allowed.join("../allowed-old/outside.txt"),
            allowed.join("escape.txt"),
        ] {
            assert_eq!(
                policy.read(&path).await.err(),
                Some(FileAccessError::OutsideReadDirs)
            );
        }
        // Authorization preserves the ordinary missing-file error at the gateway.
        assert!(policy.read(&allowed.join("missing.txt")).await.is_ok());
        *roots.write().unwrap_or_else(PoisonError::into_inner) =
            vec![dir.path().join("missing-root")];
        assert_eq!(
            policy.read(&file).await.err(),
            Some(FileAccessError::OutsideReadDirs)
        );
        roots
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        assert_eq!(
            policy.read(&file).await.err(),
            Some(FileAccessError::OutsideReadDirs)
        );
        Ok(())
    }

    #[tokio::test]
    async fn own_files_and_their_directory_ancestors_are_never_allowed()
    -> Result<(), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let state = dir.path().join("state");
        let profile = dir.path().join("profile");
        fs::create_dir(&state).await?;
        fs::create_dir(&profile).await?;
        let token = dir.path().join("token");
        let key = dir.path().join("key-not-created");
        let settings = state.join("settings.json");
        let local_state = profile.join("Local State");
        for file in [&settings, &local_state, &token] {
            fs::write(file, "private").await?;
        }
        symlink(&state, dir.path().join("state-link"))?;
        symlink(&token, dir.path().join("token-link"))?;
        let policy = FileAccess::new(
            || vec!["/".into()],
            vec![
                ProtectedPath::Directory(state.clone()),
                ProtectedPath::Directory(profile.clone()),
                ProtectedPath::File(token.clone()),
                ProtectedPath::File(key.clone()),
            ],
        );
        for path in [
            settings,
            local_state,
            token,
            key,
            state.join("new/sub/file"),
            dir.path().join("state-link/new-file"),
            dir.path().join("token-link"),
            profile.join("../profile/Local State"),
        ] {
            assert_eq!(
                policy.read(&path).await.err(),
                Some(FileAccessError::OwnFiles)
            );
            assert_eq!(
                policy.write(&path).await.err(),
                Some(FileAccessError::OwnFiles)
            );
        }
        for path in [
            Path::new("/"),
            dir.path(),
            state.as_path(),
            profile.as_path(),
        ] {
            assert_eq!(
                policy.read(path).await.err(),
                Some(FileAccessError::OwnFiles)
            );
        }
        assert!(
            policy
                .write(&dir.path().join("ordinary-copy"))
                .await
                .is_ok()
        );
        assert!(
            !state.join("new").exists(),
            "authorization never creates parent folders"
        );
        Ok(())
    }

    #[tokio::test]
    async fn missing_tails_resume_symlink_resolution_after_parent_components()
    -> Result<(), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let state = dir.path().join("state");
        fs::create_dir(&state).await?;
        symlink(&state, dir.path().join("into-state"))?;
        symlink(dir.path().join("absent"), dir.path().join("dangling"))?;
        symlink(dir.path().join("cycle"), dir.path().join("cycle"))?;
        let policy = FileAccess::new(
            || vec!["/".into()],
            vec![
                ProtectedPath::Directory(state.clone()),
                ProtectedPath::Directory(dir.path().join("not-created/state")),
            ],
        );
        for path in [
            dir.path().join("missing/../into-state/new/sub/file"),
            dir.path().join("not-created/state/nested/file"),
        ] {
            assert_eq!(
                policy.write(&path).await.err(),
                Some(FileAccessError::OwnFiles)
            );
        }
        for path in [dir.path().join("dangling/new"), dir.path().join("cycle")] {
            assert_eq!(
                policy.read(&path).await.err(),
                Some(FileAccessError::Unresolvable)
            );
            assert_eq!(
                policy.write(&path).await.err(),
                Some(FileAccessError::Unresolvable)
            );
        }
        assert!(!dir.path().join("missing").exists());
        assert!(!dir.path().join("not-created").exists());
        Ok(())
    }
    #[tokio::test]
    async fn transient_parents_are_checked_without_restricting_other_writes()
    -> Result<(), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let state = dir.path().join("data/browserclaw/claw-sdk");
        let future_key = dir.path().join("keys/token");
        fs::create_dir_all(state.join("existing")).await?;
        let policy = FileAccess::new(
            || vec!["/".into()],
            vec![
                ProtectedPath::Directory(state.clone()),
                ProtectedPath::File(future_key.clone()),
            ],
        );
        let escaped = state.join("settings.json.tmp/../../../../tmp/proof.png");
        assert_eq!(
            policy.write(&escaped).await.err(),
            Some(FileAccessError::OwnFiles)
        );
        let key_escape = future_key.join("../../public.png");
        assert_eq!(
            policy.write(&key_escape).await.err(),
            Some(FileAccessError::OwnFiles)
        );
        let ordinary = dir.path().join("ordinary/new/../../public.png");
        assert_eq!(policy.write(&ordinary).await?.original(), ordinary);
        let existing_parent = state.join("existing/../../../../public.png");
        assert!(
            policy.write(&existing_parent).await.is_ok(),
            "traversal through existing parents makes no protected directory"
        );
        assert!(
            policy
                .write(&dir.path().join("keys/ordinary.png"))
                .await
                .is_ok(),
            "creating an ancestor of a future exact key is allowed"
        );
        assert!(!state.join("settings.json.tmp").exists());
        assert!(!future_key.exists());
        Ok(())
    }

    #[tokio::test]
    async fn dangling_configured_credentials_protect_their_names_and_future_targets()
    -> Result<(), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let allowed = dir.path().join("downloads");
        let keys = dir.path().join("keys");
        fs::create_dir(&allowed).await?;
        fs::create_dir(&keys).await?;
        let file = allowed.join("ordinary.txt");
        fs::write(&file, "ordinary").await?;
        let token = keys.join("token-link");
        let key = keys.join("key-link");
        let future_token = dir.path().join("future-token/value");
        let future_key = dir.path().join("future-key/value");
        symlink(&future_token, &token)?;
        symlink("../future-key/value", &key)?;
        let cycle = keys.join("cycle");
        symlink(&cycle, &cycle)?;
        let root = allowed.clone();
        let policy = FileAccess::new(
            move || vec![root.clone()],
            vec![
                ProtectedPath::File(token.clone()),
                ProtectedPath::File(key.clone()),
                ProtectedPath::File(cycle.clone()),
            ],
        );
        assert!(
            policy.read(&file).await.is_ok(),
            "an unusable configured credential does not disable an ordinary download read"
        );
        assert!(policy.write(&dir.path().join("ordinary.png")).await.is_ok());
        for alias in [&token, &key, &cycle] {
            assert!(policy.read(alias).await.is_err());
            assert!(policy.write(alias).await.is_err());
        }
        for target in [&future_token, &future_key] {
            assert_eq!(
                policy.read(target).await.err(),
                Some(FileAccessError::OwnFiles)
            );
            assert_eq!(
                policy.write(target).await.err(),
                Some(FileAccessError::OwnFiles)
            );
        }
        let all = FileAccess::new(|| vec!["/".into()], vec![ProtectedPath::File(token)]);
        assert_eq!(
            all.read(&keys).await.err(),
            Some(FileAccessError::OwnFiles),
            "a directory containing the configured alias is protected too"
        );
        Ok(())
    }

    /// A home laid out as the SDK image's: dotfiles and caches, a `Downloads` link to the
    /// downloads folder outside it, and folders a host may list.
    struct HomeFixture {
        dir: TempDir,
        home: PathBuf,
        downloads: PathBuf,
    }

    impl HomeFixture {
        async fn new() -> Result<Self, Box<dyn Error>> {
            let dir = tempfile::tempdir()?;
            let home = dir.path().join("home/neo");
            let downloads = dir.path().join("data/downloads");
            for folder in ["shared/state", ".hidden", "plain", ".cache/fontconfig"] {
                fs::create_dir_all(home.join(folder)).await?;
            }
            fs::create_dir_all(&downloads).await?;
            for file in [
                ".bashrc",
                ".cache/fontconfig/fonts.cache-9",
                "shared/readable.txt",
                "shared/state/settings.json",
                ".hidden/readable.txt",
                "plain/readable.txt",
            ] {
                fs::write(home.join(file), "private").await?;
            }
            symlink(&downloads, home.join("Downloads"))?;
            symlink(&home, dir.path().join("home-link"))?;
            Ok(Self {
                dir,
                home,
                downloads,
            })
        }

        fn policy(&self, roots: Vec<PathBuf>) -> FileAccess {
            FileAccess::new(
                move || roots.clone(),
                vec![
                    ProtectedPath::Directory(self.home.join("shared/state")),
                    ProtectedPath::Home {
                        path: self.home.clone(),
                        open: vec![self.downloads.clone()],
                    },
                ],
            )
        }
    }

    #[tokio::test]
    async fn the_home_is_refused_except_inside_a_listed_folder_below_it()
    -> Result<(), Box<dyn Error>> {
        let fixture = HomeFixture::new().await?;
        let (home, outside) = (&fixture.home, fixture.dir.path());
        let policy = fixture.policy(vec![
            fixture.downloads.clone(),
            home.join("shared"),
            home.join(".hidden"),
        ]);
        for path in [
            home.join(".bashrc"),
            home.join(".cache/fontconfig/fonts.cache-9"),
            home.join(".pki/nssdb/cert9.db"),
            home.join(".config/fontconfig/fonts.conf"),
            home.join("new.png"),
            home.join("plain/readable.txt"),
            // A listed folder whose first name below the home is a dot-entry lifts nothing.
            home.join(".hidden/readable.txt"),
            // One of Neo's places inside a listed folder stays Neo's.
            home.join("shared/state/settings.json"),
            home.join("shared/../.bashrc"),
            outside.join("home-link/.bashrc"),
            outside.join("missing/../home-link/.pki/nssdb/cert9.db"),
        ] {
            assert_eq!(
                policy.read(&path).await.err(),
                Some(FileAccessError::OwnFiles),
                "read {}",
                path.display()
            );
            assert_eq!(
                policy.write(&path).await.err(),
                Some(FileAccessError::OwnFiles),
                "write {}",
                path.display()
            );
        }
        for path in [home.clone(), outside.join("home"), outside.to_owned()] {
            assert_eq!(
                policy.read(&path).await.err(),
                Some(FileAccessError::OwnFiles),
                "a folder upload of {} would carry the home",
                path.display()
            );
        }
        assert!(policy.read(&home.join("shared/readable.txt")).await.is_ok());
        assert!(
            policy
                .write(&home.join("shared/deep/.dot/shot.png"))
                .await
                .is_ok(),
            "a dot name inside a listed folder belongs to that folder"
        );
        for path in ["Downloads/saved.csv", "Downloads/new/saved.csv"] {
            assert!(
                policy.write(&home.join(path)).await.is_ok(),
                "~/Downloads is judged by its real path, the downloads folder"
            );
        }
        assert!(policy.read(&home.join("Downloads/saved.csv")).await.is_ok());
        assert!(!home.join(".pki").exists() && !home.join(".config").exists());
        assert!(!outside.join("missing").exists() && !home.join("shared/deep").exists());
        Ok(())
    }

    /// An open folder two or more levels below the home, whose parents are not made yet, takes
    /// its first save: a missing parent inside the home may be made when it is an ancestor of an
    /// open folder (the file-access design's rule), and no other. A file there never may, and
    /// Neo's other places stay refused below such a parent.
    #[tokio::test]
    async fn a_missing_parent_of_an_open_folder_may_be_made_and_no_other()
    -> Result<(), Box<dyn Error>> {
        let fixture = HomeFixture::new().await?;
        let home = fixture.home.clone();
        let listed = home.join("listed/deep/in");
        let policy = FileAccess::new(
            move || vec![listed.clone()],
            vec![
                ProtectedPath::Directory(home.join("work/state")),
                ProtectedPath::Home {
                    path: home.clone(),
                    open: vec![home.join("work/downloads")],
                },
            ],
        );
        for path in [
            "work/downloads/shot.png",
            "work/downloads/new/shot.png",
            "listed/deep/in/saved.csv",
        ] {
            assert!(policy.write(&home.join(path)).await.is_ok(), "{path}");
        }
        for path in [
            // A file beside the open folder's path, or below a missing folder off it.
            "work/notes.txt",
            "work/other/shot.png",
            "listed/deep/shot.png",
            "listed/side/in/shot.png",
            // A spelling that lands in the open folder but makes a folder off its path.
            "work/other/../downloads/shot.png",
            // Neo's own place below a missing parent stays Neo's.
            "work/state/shot.png",
            "work/downloads/../../.bashrc",
        ] {
            assert_eq!(
                policy.write(&home.join(path)).await.err(),
                Some(FileAccessError::OwnFiles),
                "{path}"
            );
        }
        assert!(
            !home.join("work").exists() && !home.join("listed").exists(),
            "a check makes nothing"
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_root_at_or_above_the_home_lifts_nothing() -> Result<(), Box<dyn Error>> {
        let fixture = HomeFixture::new().await?;
        let home = &fixture.home;
        for root in ["/".into(), home.clone(), fixture.dir.path().join("home")] {
            let policy = fixture.policy(vec![root.clone()]);
            for path in [home.join("plain/readable.txt"), home.join(".bashrc")] {
                assert_eq!(
                    policy.read(&path).await.err(),
                    Some(FileAccessError::OwnFiles),
                    "{} listed",
                    root.display()
                );
                assert_eq!(
                    policy.write(&path).await.err(),
                    Some(FileAccessError::OwnFiles)
                );
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn an_open_folder_lifts_only_the_home_rule() -> Result<(), Box<dyn Error>> {
        let fixture = HomeFixture::new().await?;
        let home = &fixture.home;
        let inside = home.join("Inside");
        fs::create_dir(&inside).await?;
        fs::write(inside.join("saved.csv"), "ordinary").await?;
        symlink(home.join(".cache/fontconfig"), home.join("into-cache"))?;
        let listed = home.join("listed-not-made");
        let policy = FileAccess::new(
            move || vec![listed.clone()],
            vec![ProtectedPath::Home {
                path: home.clone(),
                open: vec![
                    inside.clone(),
                    home.join("Downloads-not-made"),
                    home.join(".config/not-made"),
                    home.join("into-cache"),
                    home.join(".bashrc"),
                ],
            }],
        );
        assert!(policy.write(&inside.join("saved.csv")).await.is_ok());
        assert_eq!(
            policy.read(&inside.join("saved.csv")).await.err(),
            Some(FileAccessError::OutsideReadDirs),
            "a read still needs files.readDirs"
        );
        for path in [
            "Downloads-not-made/new/file.png",
            "listed-not-made/file.png",
        ] {
            assert!(
                policy.write(&home.join(path)).await.is_ok(),
                "a folder not made yet is judged by its real path: {path}"
            );
        }
        assert_eq!(
            policy
                .read(&home.join("listed-not-made/file.png"))
                .await
                .err(),
            Some(FileAccessError::OutsideReadDirs),
            "a missing read root grants no read"
        );
        for path in [
            ".config/not-made/file.png",
            "into-cache/fonts.cache-9",
            ".bashrc",
        ] {
            assert_eq!(
                policy.write(&home.join(path)).await.err(),
                Some(FileAccessError::OwnFiles),
                "{path}: a dot-entry, a link into one, or a file lifts nothing"
            );
        }
        assert!(!home.join("Downloads-not-made").exists() && !home.join(".pki").exists());
        Ok(())
    }
}
