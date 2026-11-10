use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, fs, process};

use neo_trace::ErrorOwner;
use serde_json::{Value, json};

use super::{
    FilesInput, HostPath, InputFiles, Secrets, SetInputFilesInput, client_stat, inspect, recheck,
    resolve_like_node, set_input_files,
};
use crate::op_scope::OpScope;
use crate::target::CallHeader;
use crate::test_env::{TestEnv, offline_session};
use crate::test_support::RecordingHooks;
use crate::{ErrorCode, Op, Playwright, PlaywrightError};

type TestResult = Result<(), Box<dyn Error>>;

/// A directory of its own under the system's temp dir, removed when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let dir = env::temp_dir().join(format!("input-files-{name}-{}-{stamp}", process::id()));
        fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }

    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _removed = fs::remove_dir_all(&self.0);
    }
}

/// `files` classified in a `locator.setInputFiles` op whose host named `secrets`: what the
/// input gets, or the error the script does.
async fn classified_naming(
    secrets: &[PathBuf],
    files: FilesInput,
) -> Result<Result<InputFiles, PlaywrightError>, Box<dyn Error>> {
    let env = TestEnv::new();
    let upload = Op::from_name("locator.setInputFiles").ok_or("setInputFiles is an op")?;
    let scope = OpScope::new(&env, upload, CallHeader::default());
    let secrets = Secrets::named_by(Some(secrets.to_vec()));
    Ok(InputFiles::classify(&scope, &secrets, files).await)
}

/// `paths` classified by a host with no secrets to deny.
async fn classified(paths: &[&str]) -> Result<Result<InputFiles, PlaywrightError>, Box<dyn Error>> {
    let paths = paths.iter().map(|path| (*path).to_owned()).collect();
    classified_naming(&[], FilesInput::Paths(paths)).await
}

/// The client's error a classification fails with, as the script sees it.
async fn refusal(
    paths: &[&str],
) -> Result<(ErrorOwner, Option<ErrorCode>, String), Box<dyn Error>> {
    let error = classified(paths)
        .await?
        .err()
        .ok_or(format!("{paths:?} is refused"))?;
    Ok((
        error.owner(),
        error.error_code(),
        error.message().to_owned(),
    ))
}

/// `paths` checked again before the send, as `ready` checks them, by a host with no secrets
/// to deny: the access error of the first path gone now, or the script's error.
async fn rechecked(
    paths: &[&str],
) -> Result<Result<Option<String>, PlaywrightError>, Box<dyn Error>> {
    let env = TestEnv::new();
    let upload = Op::from_name("locator.setInputFiles").ok_or("setInputFiles is an op")?;
    let scope = OpScope::new(&env, upload, CallHeader::default());
    let paths: Vec<HostPath> = paths
        .iter()
        .map(|path| HostPath((*path).to_owned()))
        .collect();
    Ok(recheck(&scope, &Secrets::named_by(Some(Vec::new())), &paths).await)
}

fn cwd() -> Result<String, Box<dyn Error>> {
    Ok(env::current_dir()?
        .to_str()
        .ok_or("the test runs in a UTF-8 directory")?
        .to_owned())
}

/// Each case is Node 26's `path.resolve` (measured), with `/w/d` as the working directory.
#[test]
fn tests_that_paths_resolve_as_nodes_path_resolve() {
    for (path, resolved) in [
        ("a.txt", "/w/d/a.txt"),
        ("a//b/../c/", "/w/d/a/c"),
        ("./", "/w/d"),
        ("a/./b/.", "/w/d/a/b"),
        ("..", "/w"),
        ("../..", "/"),
        ("../../../x", "/x"),
        ("a/..", "/w/d"),
        ("/../x", "/x"),
        ("//x//y", "/x/y"),
        ("/", "/"),
        ("/a/b/../../..", "/"),
        ("a\\b", "/w/d/a\\b"),
        ("space name.txt", "/w/d/space name.txt"),
    ] {
        assert_eq!(
            resolve_like_node("/w/d", path),
            resolved,
            "path.resolve({path:?})"
        );
    }
}

/// Node quotes a path in its argument errors with `util.inspect`.
#[test]
fn tests_that_a_quoted_path_reads_as_nodes_inspect() {
    for (text, quoted) in [
        ("a\0b", "'a\\x00b'"),
        ("it's", "\"it's\""),
        ("'\"", "`'\"`"),
        ("a\\b\nc\td", "'a\\\\b\\nc\\td'"),
        ("\u{1b}\u{7f}\u{b}", "'\\x1B\\x7F\\x0B'"),
    ] {
        assert_eq!(inspect(text), quoted, "util.inspect({text:?})");
    }
}

/// The messages are Node 26's for the same calls on macOS (measured), which take libuv's words.
#[tokio::test]
async fn tests_that_a_failed_stat_reads_as_nodes_error() -> TestResult {
    let scratch = Scratch::new("stat")?;
    let file = scratch.path("file.txt");
    fs::write(&file, "hi")?;
    let missing = scratch.path("missing.txt");
    let through_file = format!("{file}/x");
    let long = scratch.path(&"a".repeat(300));
    for (path, message) in [
        (
            missing.clone(),
            format!("ENOENT: no such file or directory, stat '{missing}'"),
        ),
        (
            through_file.clone(),
            format!("ENOTDIR: not a directory, stat '{through_file}'"),
        ),
        (
            long.clone(),
            format!("ENAMETOOLONG: name too long, stat '{long}'"),
        ),
        (
            String::new(),
            "ENOENT: no such file or directory, stat ''".to_owned(),
        ),
    ] {
        let error = client_stat(&path)
            .await
            .err()
            .ok_or(format!("{path} fails"))?;
        assert_eq!(error.message(), message, "the client's stat of {path:?}");
    }
    let nul = client_stat("a\0b").await.err().ok_or("a NUL byte fails")?;
    assert_eq!(
        nul.message(),
        "The argument 'path' must be a string, Uint8Array, or URL without null bytes. Received \
         'a\\x00b'",
        "Node refuses the path before the system call"
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn tests_that_a_symlink_loop_and_a_locked_directory_read_as_nodes_errors() -> TestResult {
    use std::os::unix::fs::{self as unix_fs, PermissionsExt};

    let scratch = Scratch::new("unix")?;
    let first = scratch.path("loop1");
    unix_fs::symlink(scratch.path("loop2"), &first)?;
    unix_fs::symlink(&first, scratch.path("loop2"))?;
    let looped = client_stat(&first).await.err().ok_or("a loop fails")?;
    assert_eq!(
        looped.message(),
        format!("ELOOP: too many symbolic links encountered, stat '{first}'")
    );
    let locked = scratch.path("locked");
    fs::create_dir(&locked)?;
    fs::write(format!("{locked}/inner.txt"), "secret")?;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000))?;
    let inner = format!("{locked}/inner.txt");
    let denied = client_stat(&inner).await;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755))?;
    // Root reads through any mode, so the case holds only for an ordinary user.
    if fs::read_dir(&locked).is_ok() && denied.is_ok() {
        return Ok(());
    }
    assert_eq!(
        denied.err().ok_or("a locked directory fails")?.message(),
        format!("EACCES: permission denied, stat '{inner}'")
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_paths_classify_as_upstreams_client_resolves_them() -> TestResult {
    let cwd = cwd()?;
    let InputFiles::Paths(paths) = classified(&["Cargo.toml", "./src/../Cargo.toml"]).await??
    else {
        return Err("two files are paths".into());
    };
    let resolved: Vec<&str> = paths.iter().map(|path| path.0.as_str()).collect();
    let manifest = format!("{cwd}/Cargo.toml");
    assert_eq!(
        resolved,
        [manifest.as_str(), manifest.as_str()],
        "relative paths resolve against the working directory"
    );
    let InputFiles::Directory(directory) = classified(&["src/"]).await?? else {
        return Err("a directory is a directory".into());
    };
    assert_eq!(directory.0, format!("{cwd}/src"));
    let absolute = format!("{cwd}/Cargo.toml");
    let InputFiles::Paths(paths) = classified(&[&absolute]).await?? else {
        return Err("an absolute file is a path".into());
    };
    assert_eq!(
        paths.first().map(|path| path.0.as_str()),
        Some(absolute.as_str())
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_the_clients_refusals_are_unprefixed_and_in_its_order() -> TestResult {
    let client = |message: &str| {
        (
            ErrorOwner::Script,
            Some(ErrorCode::InvalidInput),
            message.to_owned(),
        )
    };
    assert_eq!(
        refusal(&["src", "tests"]).await?,
        client("Multiple directories are not supported")
    );
    assert_eq!(
        refusal(&["src", "Cargo.toml"]).await?,
        client("File paths must be all files or a single directory")
    );
    assert_eq!(
        refusal(&["Cargo.toml", "src"]).await?,
        client("File paths must be all files or a single directory")
    );
    // Each path is stat'ed in order: the first that fails is the error.
    assert_eq!(
        refusal(&["Cargo.toml", "nope.txt", "a\0b"]).await?,
        client("ENOENT: no such file or directory, stat 'nope.txt'")
    );
    assert_eq!(
        refusal(&["src", "tests", "nope.txt"]).await?,
        client("Multiple directories are not supported"),
        "a second directory fails at once, before the paths after it"
    );
    let payloads = classified_naming(
        &[],
        FilesInput::Payloads(serde_json::from_value(json!([
            { "name": "a.txt", "mimeType": "text/plain", "buffer": "YQ==" }
        ]))?),
    )
    .await??;
    assert!(
        matches!(&payloads, InputFiles::Payloads(payloads) if payloads.len() == 1),
        "payloads touch no disk"
    );
    Ok(())
}

/// A host serves an op unheld unless its spec declares `HostFiles`, so the file action
/// refuses any other op before it admits a page, while `locator.setInputFiles` goes on to
/// admit its page, which fails here for want of a browser. Paused: core waits out its own
/// attach timeout before it gives up on the page.
#[tokio::test(start_paused = true)]
async fn tests_that_only_an_op_declaring_host_files_reaches_the_page() -> TestResult {
    let playwright = Playwright::new(offline_session(), Arc::new(RecordingHooks::default()));
    let env = TestEnv::new();
    let input = || {
        serde_json::from_value::<SetInputFilesInput>(json!({
            "target": { "$neo": "locator", "page": { "$neo": "page", "id": 7, "tab": 1007 },
                        "display": "getByRef('f7e3')",
                        "chunks": [{ "parts": [{ "name": "aria-ref", "body": "f7e3",
                                                 "source": "f7e3" }] }] },
            "strict": "strict",
            "files": { "payloads": [{ "name": "a.txt", "mimeType": "text/plain",
                                      "buffer": "YQ==" }] },
        }))
    };
    let click = Op::from_name("locator.click").ok_or("locator.click is an op")?;
    let undeclared = OpScope::new(&env, click, CallHeader::default());
    let refused = set_input_files(&playwright, &undeclared, input()?)
        .await
        .err()
        .ok_or("an op that does not declare HostFiles must not reach the page")?;
    assert_eq!(
        (refused.owner(), refused.error_code()),
        (ErrorOwner::Neo, Some(ErrorCode::Internal)),
        "{refused}"
    );
    let upload = Op::from_name("locator.setInputFiles").ok_or("setInputFiles is an op")?;
    let declared = OpScope::new(&env, upload, CallHeader::default());
    let admitting = set_input_files(&playwright, &declared, input()?)
        .await
        .err()
        .ok_or("no browser, no page")?;
    assert_eq!(
        admitting.error_code(),
        Some(ErrorCode::BrowserDisconnected),
        "the declared op goes on to admit its page: {admitting}"
    );
    assert!(
        env.calls().is_empty(),
        "neither op noted, rechecked or marked anything sent"
    );
    Ok(())
}

/// `locator('#f').setInputFiles(files)` as the prelude sends it.
fn upload_of(files: &Value) -> Result<SetInputFilesInput, serde_json::Error> {
    serde_json::from_value(json!({
        "target": { "$neo": "locator", "page": { "$neo": "page", "id": 7, "tab": 1007 },
                    "display": "locator('#f')",
                    "chunks": [{ "parts": [{ "name": "css", "body": "#f", "source": "#f" }] }] },
        "strict": "strict",
        "files": files,
    }))
}

/// A missing path is refused before the page is admitted, so a host sees nothing sent and no
/// page asked for.
#[tokio::test]
async fn tests_that_a_missing_path_is_refused_before_any_page_work() -> TestResult {
    let hooks = RecordingHooks::naming_secrets(Vec::new());
    let playwright = Playwright::new(offline_session(), Arc::new(hooks));
    let env = TestEnv::new();
    let upload = Op::from_name("locator.setInputFiles").ok_or("setInputFiles is an op")?;
    let scope = OpScope::new(&env, upload, CallHeader::default());
    let input = upload_of(&json!({ "paths": ["missing.txt"] }))?;
    let refused = set_input_files(&playwright, &scope, input)
        .await
        .err()
        .ok_or("a missing path is refused")?;
    assert_eq!(
        refused.message(),
        "ENOENT: no such file or directory, stat 'missing.txt'",
        "upstream's client error: no prefix, no call log"
    );
    assert!(env.calls().is_empty(), "nothing was noted or sent");
    Ok(())
}

/// A host that has not named Neo's secret files has not said what on its disk must stay
/// there, so no path uploads: each is refused as `neo / unsupported` before it is stat'ed, a
/// missing one too, while payloads, which read no file, go on to admit their page (which fails
/// here for want of a browser). Paused, as above.
#[tokio::test(start_paused = true)]
async fn tests_that_with_no_secrets_named_no_path_uploads_but_payloads_do() -> TestResult {
    let playwright = Playwright::new(offline_session(), Arc::new(RecordingHooks::default()));
    let env = TestEnv::new();
    let upload = Op::from_name("locator.setInputFiles").ok_or("setInputFiles is an op")?;
    for paths in [
        json!(["Cargo.toml"]),
        json!(["missing.txt"]),
        json!(["src"]),
        json!(["/dev/fd/2147483647"]),
    ] {
        let scope = OpScope::new(&env, upload, CallHeader::default());
        let input = upload_of(&json!({ "paths": paths }))?;
        let refused = set_input_files(&playwright, &scope, input)
            .await
            .err()
            .ok_or(format!("{paths} is refused"))?;
        assert_eq!(
            (refused.owner(), refused.error_code(), refused.message()),
            (
                ErrorOwner::Neo,
                Some(ErrorCode::Unsupported),
                "locator.setInputFiles: Neo uploads no file from its disk until its host names \
                 Neo's own secret files; pass { name, mimeType, buffer } instead"
            ),
            "{paths}"
        );
    }
    let scope = OpScope::new(&env, upload, CallHeader::default());
    let payload = upload_of(&json!({ "payloads": [
        { "name": "a.txt", "mimeType": "text/plain", "buffer": "YQ==" }
    ] }))?;
    let admitting = set_input_files(&playwright, &scope, payload)
        .await
        .err()
        .ok_or("no browser, no page")?;
    assert_eq!(
        admitting.error_code(),
        Some(ErrorCode::BrowserDisconnected),
        "a payload goes on to admit its page: {admitting}"
    );
    assert!(env.calls().is_empty(), "nothing was noted or sent");
    Ok(())
}

/// A path into `/dev`, a file or the directory itself, names a device or Chromium's own
/// descriptor once Chromium opens it, so a host with no secrets to deny refuses it as one. So
/// does a directory upload of `/`, whose tree holds `/dev`.
#[cfg(unix)]
#[tokio::test]
async fn tests_that_a_path_into_dev_is_refused_as_a_secret_with_none_to_deny() -> TestResult {
    for path in ["/dev/null", "/dev", "/"] {
        assert_eq!(
            refusal(&[path]).await?,
            (
                ErrorOwner::Script,
                Some(ErrorCode::SecretUploadRefused),
                "locator.setInputFiles: Neo does not upload its own secret files".to_owned()
            ),
            "{path}"
        );
    }
    Ok(())
}

/// A descriptor Neo's process does not hold fails Neo's own stat (`EBADF` on macOS, measured),
/// yet names whatever Chromium's process holds there, so it is refused as a secret before
/// either check stats it: classify's, by its absolute path and by a relative one Node folds
/// into `/dev`, and the re-check's, before its access. No path is in the message.
#[cfg(unix)]
#[tokio::test]
async fn tests_that_a_device_path_neo_cannot_stat_is_refused_before_the_stat() -> TestResult {
    let unheld = "/dev/fd/2147483647";
    let relative = format!("{}dev/fd/2147483647", "../".repeat(64));
    let secret = (
        ErrorOwner::Script,
        Some(ErrorCode::SecretUploadRefused),
        "locator.setInputFiles: Neo does not upload its own secret files".to_owned(),
    );
    for path in [unheld, relative.as_str()] {
        assert_eq!(refusal(&[path]).await?, secret, "classify, {path}");
    }
    let refused = rechecked(&[unheld])
        .await?
        .err()
        .ok_or("the re-check refuses it")?;
    assert_eq!(
        (
            refused.owner(),
            refused.error_code(),
            refused.message().to_owned()
        ),
        secret,
        "the re-check"
    );
    Ok(())
}

/// The script sees `script / neo_secret_upload_refused` with the method's prefix, and nothing
/// that says which secret or where it is. Each path is checked right after its own stat, in
/// the script's order, so a missing path before a secret still fails as Node's stat, and a
/// secret before a missing path is refused.
#[tokio::test]
async fn tests_that_a_secret_is_refused_after_its_stat_and_never_named() -> TestResult {
    let scratch = Scratch::new("secret")?;
    let token = scratch.path("neo-host-token");
    let notes = scratch.path("notes.txt");
    let missing = scratch.path("missing.txt");
    fs::write(&token, "token")?;
    fs::write(&notes, "notes")?;
    let named = [PathBuf::from(&token)];
    let classify = |paths: &[&str]| {
        let paths = paths.iter().map(|path| (*path).to_owned()).collect();
        classified_naming(&named, FilesInput::Paths(paths))
    };
    for paths in [
        vec![token.as_str()],
        vec![notes.as_str(), token.as_str()],
        vec![token.as_str(), missing.as_str()],
    ] {
        let refused = classify(&paths)
            .await?
            .err()
            .ok_or(format!("{paths:?} is refused"))?;
        assert_eq!(
            (refused.owner(), refused.error_code(), refused.message()),
            (
                ErrorOwner::Script,
                Some(ErrorCode::SecretUploadRefused),
                "locator.setInputFiles: Neo does not upload its own secret files"
            ),
            "{paths:?}"
        );
        assert!(
            !refused
                .message()
                .contains(scratch.0.to_string_lossy().as_ref())
        );
    }
    let stat_first = classify(&[missing.as_str(), token.as_str()])
        .await?
        .err()
        .ok_or("a missing path is refused")?;
    assert_eq!(
        stat_first.message(),
        format!("ENOENT: no such file or directory, stat '{missing}'")
    );
    let InputFiles::Paths(paths) = classify(&[notes.as_str()]).await?? else {
        return Err("an ordinary file is a path".into());
    };
    assert_eq!(paths.len(), 1, "an ordinary file beside a secret uploads");
    Ok(())
}

/// Right before the send, each path is `access`ed again, in order: one still there passes, and
/// the first one gone by now is upstream's access error with its absolute path, which a
/// locator retries on.
#[tokio::test]
async fn tests_that_the_recheck_reads_a_path_gone_by_now_as_upstreams_access_error() -> TestResult {
    let scratch = Scratch::new("recheck")?;
    let present = scratch.path("present.txt");
    fs::write(&present, "here")?;
    let missing = scratch.path("missing.txt");
    assert_eq!(rechecked(&[&present]).await??, None);
    assert_eq!(
        rechecked(&[&present, &missing]).await??,
        Some(format!(
            "ENOENT: no such file or directory, access '{missing}'"
        ))
    );
    Ok(())
}
