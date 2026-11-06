use std::error::Error;
#[cfg(unix)]
use std::os::unix::fs as unix_fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, fs, process};

use super::{Secrets, Verdict};
use crate::op::Op;
use crate::op_scope::OpScope;
use crate::target::CallHeader;
use crate::test_env::TestEnv;

type TestResult = Result<(), Box<dyn Error>>;

/// A Neo host's disk in a temp dir of its own, removed when the test ends:
///
/// ```text
///  run/neo-host-token              named: a secret file
///  profile/Default/{Cookies, Login Data, Cache/data_0}   named: profile/, a secret directory
///  store/Cookies                   named alone; store/Cookies-backup beside it is not
///  trace/trace.db                  named, but absent until a test makes it
///  home/notes.txt                  an ordinary file
/// ```
struct Disk(PathBuf);

impl Disk {
    fn new(name: &str) -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root = env::temp_dir().join(format!("neo-secrets-{name}-{}-{stamp}", process::id()));
        // The temp dir is itself a symlink on macOS (/var -> /private/var), which every path
        // below then runs through.
        let disk = Self(root);
        for (file, body) in [
            ("run/neo-host-token", "token"),
            ("profile/Default/Cookies", "cookies"),
            ("profile/Default/Login Data", "logins"),
            ("profile/Default/Cache/data_0", "cache"),
            ("store/Cookies", "cookies"),
            ("store/Cookies-backup", "backup"),
            ("home/notes.txt", "notes"),
        ] {
            let path = disk.at(file);
            fs::create_dir_all(path.parent().ok_or("a file has a directory")?)?;
            fs::write(&path, body)?;
        }
        fs::create_dir_all(disk.at("trace"))?;
        Ok(disk)
    }

    fn at(&self, path: &str) -> PathBuf {
        self.0.join(path)
    }

    /// What the host names: the token, the profile, the one store file and the trace store.
    fn named(&self) -> Vec<PathBuf> {
        [
            "run/neo-host-token",
            "profile",
            "store/Cookies",
            "trace/trace.db",
        ]
        .iter()
        .map(|path| self.at(path))
        .collect()
    }
}

impl Drop for Disk {
    fn drop(&mut self) {
        let _removed = fs::remove_dir_all(&self.0);
    }
}

/// What one round of a `setInputFiles` op whose host named `named` says of each path.
async fn verdicts(named: Vec<PathBuf>, paths: &[&Path]) -> Result<Vec<Verdict>, Box<dyn Error>> {
    let env = TestEnv::new();
    let upload = Op::from_name("locator.setInputFiles").ok_or("setInputFiles is an op")?;
    let scope = OpScope::new(&env, upload, CallHeader::default());
    let on_disk = Secrets::named_by(Some(named)).on_disk(&scope).await?;
    let mut verdicts = Vec::new();
    for path in paths {
        verdicts.push(on_disk.check(path.to_str().ok_or("a UTF-8 path")?).await);
    }
    Ok(verdicts)
}

fn refused(verdicts: &[Verdict]) -> Vec<bool> {
    verdicts
        .iter()
        .map(|verdict| matches!(verdict, Verdict::Refused))
        .collect()
}

#[cfg(unix)]
#[tokio::test]
async fn tests_that_a_secret_file_is_refused_by_every_name_that_reaches_it() -> TestResult {
    let disk = Disk::new("names")?;
    fs::create_dir_all(disk.at("elsewhere"))?;
    unix_fs::symlink(disk.at("run/neo-host-token"), disk.at("home/token-link"))?;
    unix_fs::symlink(disk.at("run"), disk.at("home/run-link"))?;
    fs::hard_link(disk.at("run/neo-host-token"), disk.at("elsewhere/token"))?;
    let names = [
        disk.at("run/neo-host-token"),
        disk.at("home/token-link"),
        disk.at("home/run-link/neo-host-token"),
        disk.at("home/../run/./neo-host-token"),
        // `..` after a symlink leaves the symlink's target, as the kernel resolves it.
        disk.at("home/run-link/../run/neo-host-token"),
        disk.at("elsewhere/token"),
        disk.at("store/Cookies"),
    ];
    let paths: Vec<&Path> = names.iter().map(PathBuf::as_path).collect();
    assert_eq!(
        refused(&verdicts(disk.named(), &paths).await?),
        [true; 7],
        "its own path, a symlink to it, a symlink to its directory, `..`, a hard link: {names:?}"
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn tests_that_anything_inside_a_secret_directory_is_refused() -> TestResult {
    let disk = Disk::new("inside")?;
    unix_fs::symlink(disk.at("profile"), disk.at("home/profile-link"))?;
    fs::hard_link(disk.at("profile/Default/Cookies"), disk.at("home/cookies"))?;
    fs::hard_link(
        disk.at("profile/Default/Cache/data_0"),
        disk.at("home/data_0"),
    )?;
    let names = [
        disk.at("profile/Default/Cookies"),
        disk.at("profile/Default/Login Data"),
        disk.at("profile/Default/Cache/data_0"),
        disk.at("home/profile-link/Default/Cookies"),
        disk.at("home/cookies"),
        disk.at("home/data_0"),
    ];
    let paths: Vec<&Path> = names.iter().map(PathBuf::as_path).collect();
    assert_eq!(
        refused(&verdicts(disk.named(), &paths).await?),
        [true; 6],
        "a file inside, through a symlink to the directory, a hard link from outside: {names:?}"
    );
    Ok(())
}

/// Only a file whose identity a secret shares is refused: a name that merely starts like an
/// entry's, an ordinary file with a second link, and a path where nothing is are not.
#[cfg(unix)]
#[tokio::test]
async fn tests_that_ordinary_files_and_look_alike_names_are_clear() -> TestResult {
    let disk = Disk::new("clear")?;
    fs::hard_link(disk.at("home/notes.txt"), disk.at("home/notes-link.txt"))?;
    let names = [
        disk.at("home/notes.txt"),
        disk.at("store/Cookies-backup"),
        disk.at("home/notes-link.txt"),
    ];
    let paths: Vec<&Path> = names.iter().map(PathBuf::as_path).collect();
    let verdicts = verdicts(disk.named(), &paths).await?;
    assert!(
        verdicts
            .iter()
            .all(|verdict| matches!(verdict, Verdict::Clear)),
        "{verdicts:?}"
    );
    let missing = disk.at("home/missing.txt");
    let verdicts = self::verdicts(disk.named(), &[missing.as_path()]).await?;
    assert!(
        matches!(verdicts.as_slice(), [Verdict::Missing(_)]),
        "nothing there now is nothing to read: {verdicts:?}"
    );
    Ok(())
}

/// The list is resolved again by each round, so an entry that does not exist is skipped while
/// the others hold, and is refused once it exists.
#[tokio::test]
async fn tests_that_an_absent_entry_is_skipped_until_it_exists() -> TestResult {
    let disk = Disk::new("absent")?;
    let token = disk.at("run/neo-host-token");
    let notes = disk.at("home/notes.txt");
    assert_eq!(
        refused(&verdicts(disk.named(), &[token.as_path(), notes.as_path()]).await?),
        [true, false],
        "the absent trace store takes nothing from the rest"
    );
    let trace = disk.at("trace/trace.db");
    fs::write(&trace, "trace")?;
    assert_eq!(
        refused(&verdicts(disk.named(), &[trace.as_path()]).await?),
        [true]
    );
    Ok(())
}

/// A case-insensitive disk (macOS's default) gives the same file under another case, and its
/// inode says so. Skipped on a case-sensitive disk, where the other case names nothing.
#[tokio::test]
async fn tests_that_another_case_of_a_secret_is_refused_on_a_case_insensitive_disk() -> TestResult {
    let disk = Disk::new("case")?;
    let upper = disk.at("RUN/NEO-HOST-TOKEN");
    if !upper.exists() {
        return Ok(());
    }
    let inside = disk.at("PROFILE/default/cookies");
    assert_eq!(
        refused(&verdicts(disk.named(), &[upper.as_path(), inside.as_path()]).await?),
        [true, true]
    );
    Ok(())
}

/// A hard link's other names can only be found by walking the secret directories. When a walk
/// meets a directory Neo cannot read, a file with more than one link is refused, since it may
/// be a secret's other name; a file with one link is its only name, and is still cleared.
#[cfg(unix)]
#[tokio::test]
async fn tests_that_a_hard_link_is_refused_when_a_secret_directory_cannot_be_walked() -> TestResult
{
    use std::os::unix::fs::PermissionsExt;

    let disk = Disk::new("locked")?;
    fs::hard_link(disk.at("home/notes.txt"), disk.at("home/notes-link.txt"))?;
    let locked = disk.at("profile/Default/Cache");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000))?;
    let readable = fs::read_dir(&locked).is_ok();
    let linked = disk.at("home/notes-link.txt");
    let single = disk.at("store/Cookies-backup");
    let verdicts = verdicts(disk.named(), &[linked.as_path(), single.as_path()]).await;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755))?;
    // Root reads through any mode, so the case holds only for an ordinary user.
    if readable {
        return Ok(());
    }
    assert_eq!(refused(&verdicts?), [true, false]);
    Ok(())
}

/// A directory upload hands Chromium the whole tree, so it is refused when it is or sits in a
/// secret directory, when a secret sits anywhere below it, and when a file in its tree is a
/// secret's other name: a hard link to a secret file, or to a file inside a secret directory.
#[cfg(unix)]
#[tokio::test]
async fn tests_that_a_directory_holding_or_inside_a_secret_is_refused() -> TestResult {
    let disk = Disk::new("dirs")?;
    unix_fs::symlink(disk.at("profile"), disk.at("home/profile-link"))?;
    fs::create_dir_all(disk.at("bundle/deep"))?;
    fs::hard_link(disk.at("run/neo-host-token"), disk.at("bundle/deep/token"))?;
    fs::create_dir_all(disk.at("export"))?;
    fs::hard_link(
        disk.at("profile/Default/Cookies"),
        disk.at("export/cookies"),
    )?;
    let names = [
        disk.at("run"),
        disk.0.clone(),
        disk.at("store"),
        disk.at("profile"),
        disk.at("profile/Default/Cache"),
        disk.at("home/profile-link"),
        disk.at("bundle"),
        disk.at("export"),
    ];
    let paths: Vec<&Path> = names.iter().map(PathBuf::as_path).collect();
    assert_eq!(
        refused(&verdicts(disk.named(), &paths).await?),
        [true; 8],
        "holding a secret file, holding them all, the directory itself, inside it, a symlink \
         to it, a hard link to a secret below, a hard link into a secret directory: {names:?}"
    );
    Ok(())
}

/// Chromium's directory upload drops the symlinks inside the directory (measured on macOS
/// BrowserOS neo, and in Neo's Linux image, spikes-docker S3), so a symlink in the tree to a
/// secret uploads nothing of it, and the directory is cleared. A directory in the tree Neo
/// cannot read might hold anything, so the upload is refused.
#[cfg(unix)]
#[tokio::test]
async fn tests_that_a_directory_is_clear_when_only_its_symlinks_reach_a_secret() -> TestResult {
    use std::os::unix::fs::PermissionsExt;

    let disk = Disk::new("links")?;
    fs::create_dir_all(disk.at("links/sub"))?;
    fs::write(disk.at("links/sub/a.txt"), "a")?;
    unix_fs::symlink(disk.at("run/neo-host-token"), disk.at("links/token"))?;
    unix_fs::symlink(disk.at("profile"), disk.at("links/sub/profile"))?;
    let (links, home) = (disk.at("links"), disk.at("home"));
    assert_eq!(
        refused(&verdicts(disk.named(), &[links.as_path(), home.as_path()]).await?),
        [false, false]
    );
    let locked = disk.at("home/locked");
    fs::create_dir(&locked)?;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000))?;
    let readable = fs::read_dir(&locked).is_ok();
    let verdicts = verdicts(disk.named(), &[home.as_path()]).await;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755))?;
    if readable {
        return Ok(());
    }
    assert_eq!(refused(&verdicts?), [true]);
    Ok(())
}

/// Chromium opens the string it is handed in its own process, where `/dev/fd/N`, `/dev/stdin`
/// and `/proc/self/…` name Chromium's own files, not the ones Neo resolved. So a path whose
/// resolution enters `/dev` or `/proc` is refused, whether or not the host names any secret:
/// by its own name, through a symlink on the way, by `..`, and as a directory. An ordinary file
/// and an ordinary directory, even one holding a symlink into `/dev`, are still cleared.
#[cfg(unix)]
#[tokio::test]
async fn tests_that_a_path_through_dev_or_proc_is_refused_whatever_the_host_names() -> TestResult {
    let disk = Disk::new("device")?;
    unix_fs::symlink("/dev/null", disk.at("home/null-link"))?;
    // Relative, so it climbs from the link's own directory, by more `..` than the temp dir is
    // deep; `..` stops at `/`.
    let climb = format!("{}dev/null", "../".repeat(64));
    unix_fs::symlink(&climb, disk.at("home/climb-link"))?;
    let mut through = vec![
        PathBuf::from("/dev/null"),
        disk.at("home/null-link"),
        disk.at("home").join(&climb),
        disk.at("home/climb-link"),
        PathBuf::from("/dev/fd/0"),
        PathBuf::from("/dev"),
    ];
    if cfg!(target_os = "linux") {
        through.push(PathBuf::from("/proc/self/environ"));
    }
    // A case-insensitive root (macOS's default) gives `/dev` as `/DEV` too.
    if Path::new("/DEV").exists() {
        through.push(PathBuf::from("/DEV/null"));
    }
    let ordinary = [disk.at("home/notes.txt"), disk.at("home")];
    let paths: Vec<&Path> = through
        .iter()
        .chain(&ordinary)
        .map(PathBuf::as_path)
        .collect();
    for named in [disk.named(), Vec::new()] {
        let verdicts = verdicts(named, &paths).await?;
        let (device, cleared) = verdicts.split_at(through.len());
        assert_eq!(refused(device), vec![true; through.len()], "{through:?}");
        assert!(
            cleared
                .iter()
                .all(|verdict| matches!(verdict, Verdict::Clear)),
            "an ordinary file and directory: {cleared:?}"
        );
    }
    Ok(())
}

/// A directory upload hands Chromium its whole tree, so a directory above `/dev` or `/proc` is
/// refused whatever the host names: `/`, and a symlink to it. An ordinary directory is walked
/// with no entry named too, and is still cleared, a symlink to `/` inside it skipped as
/// Chromium skips it. A bind mount of `/dev` in a tree is met by the same walk; no test makes
/// one, since mounting needs root.
#[cfg(unix)]
#[tokio::test]
async fn tests_that_a_directory_holding_a_device_tree_is_refused_whatever_the_host_names()
-> TestResult {
    let disk = Disk::new("holding")?;
    unix_fs::symlink("/", disk.at("home/root-link"))?;
    let names = [
        PathBuf::from("/"),
        disk.at("home/root-link"),
        disk.at("home"),
    ];
    let paths: Vec<&Path> = names.iter().map(PathBuf::as_path).collect();
    for named in [disk.named(), Vec::new()] {
        let verdicts = verdicts(named, &paths).await?;
        assert_eq!(
            refused(&verdicts),
            [true, true, false],
            "/, a symlink to it, an ordinary directory: {verdicts:?}"
        );
    }
    Ok(())
}

/// With no device tree to meet, a walk of `/` still ends refused, at an entry it cannot stat
/// (one in `/dev/fd` on macOS, measured), so the test above cannot tell what refused it. A
/// readable tree can: with a directory of the test's own standing in for `/dev`, the
/// directories above it are refused, and one beside it is cleared.
#[cfg(unix)]
#[test]
fn tests_that_a_readable_directory_above_a_device_tree_is_refused() -> TestResult {
    use std::sync::atomic::AtomicBool;

    use super::Entries;

    let disk = Disk::new("stand-in")?;
    let tree = disk.at("home/dev");
    fs::create_dir_all(&tree)?;
    let entries = Entries::resolve(&[tree.as_path()], &[]);
    let stop = AtomicBool::new(false);
    let verdicts: Vec<Verdict> = [disk.at("home"), disk.0.clone(), disk.at("store")]
        .iter()
        .map(|path| entries.check(path, &stop))
        .collect();
    assert_eq!(
        refused(&verdicts),
        [true, true, false],
        "its directory, the disk above it, a directory beside it: {verdicts:?}"
    );
    Ok(())
}
