use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;

/// Filtered dev copy - secret-redacting source clone, NOT yet OS-isolated.
/// Copies source tree excluding secrets/keys, runs in /tmp.
///
/// OS-level isolation is being added incrementally: `generate_seatbelt_profile`
/// and `sandbox_exec_argv` below produce a deny-by-default macOS Seatbelt policy
/// for wrapping the build/test exec. They are built and unit-tested but NOT yet
/// wired into the live pipeline - see `.trinity/docs/p4-sandbox-isolation.md`
/// for the staged rollout (off -> shadow -> enforce). Enforcing blindly is unsafe
/// because Seatbelt fails *silently* and `sandbox-exec` is deprecated (still
/// functional on macOS 14+); the profile must be validated against real builds first.
#[derive(Debug)]
pub struct SandboxedDev {
    pub root: PathBuf,
    pub port: u16,
    cleaned: bool,
}

impl SandboxedDev {
    pub fn create_from_staging(ticket_id: &str, source: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        if !ticket_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(format!("invalid ticket_id: must be alphanumeric/dash/underscore, got '{}'", ticket_id).into());
        }
        let dev_root = PathBuf::from(format!("{}/.trinity/dev/{}", trios_config::project_dir(), ticket_id));
        
        if dev_root.exists() {
            fs::remove_dir_all(&dev_root)?;
        }
        
        // Full clone excluding tokens and keys
        crate::sandbox::copy_tree_filtered(source, &dev_root)?;
        
        info!(
            "[Sandbox] Dev created for ticket={ticket_id}, root={root}",
            ticket_id = ticket_id,
            root = dev_root.display()
        );
        
        Ok(SandboxedDev {
            root: dev_root,
            port: 9305,
            cleaned: false,
        })
    }
    
    pub fn clean(mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.root.exists() {
            fs::remove_dir_all(&self.root)?;
            info!("[Sandbox] Cleaned {}", self.root.display());
        }
        self.cleaned = true;
        Ok(())
    }
}

impl Drop for SandboxedDev {
    fn drop(&mut self) {
        if !self.cleaned && self.root.exists() {
            if let Err(e) = fs::remove_dir_all(&self.root) {
                eprintln!("[Sandbox] Drop cleanup failed for {}: {}", self.root.display(), e);
            }
        }
    }
}

pub fn copy_tree_filtered(src: &Path, dst: &Path) -> std::io::Result<()> {
    let ignore_exact = [
        ".env", ".env.local", "node_modules", "sandbox",
        ".git", "__pycache__",
        "browseros-server.log", "trios-server.log",
    ];
    let ignore_extensions = [".key", ".pem"];

    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();

        if ignore_exact.iter().any(|p| name.contains(p))
            || ignore_extensions.iter().any(|ext| name.ends_with(ext))
        {
            continue;
        }
        
        let src_path = entry.path();
        let dst_path = dst.join(&name);
        
        if src_path.is_dir() {
            copy_tree_filtered(&src_path, &dst_path)?;
        } else {
            // Skip files with tokens (heuristic: line contains 'sk-')
            if name.ends_with(".env") || name.ends_with(".toml") {
                // Fail closed: if the file can't be read we cannot prove it is
                // secret-free, so redact rather than copy it verbatim.
                match fs::read_to_string(&src_path) {
                    Ok(content) => {
                        if content.contains("sk-") || content.contains("api_key") {
                            fs::write(&dst_path, "# REDACTED - secrets removed by clade-improve\n")?;
                            continue;
                        }
                    }
                    Err(e) => {
                        fs::write(
                            &dst_path,
                            format!("# REDACTED - unreadable, treated as secret ({e})\n"),
                        )?;
                        continue;
                    }
                }
            }
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

/// Generate a deny-by-default macOS Seatbelt profile for running the build/test
/// of an untrusted variant. Seatbelt evaluates rules last-match-wins, so the
/// credential denies are emitted AFTER the broad read allows to override them.
///
/// Policy: deny all; allow process exec/fork + sysctl reads; allow reads of
/// system toolchain paths and the dev root; allow writes only inside the dev
/// root and the temp dirs toolchains require (/tmp, /var/folders); explicitly
/// deny credential stores (~/.ssh, Keychains) even though they are outside the
/// read allowlist (defense in depth); restrict network to localhost.
pub fn generate_seatbelt_profile(dev_root: &Path, home: &Path) -> String {
    let dev = dev_root.display();
    let home = home.display();
    format!(
        r#"(version 1)
(deny default)
(allow process-fork)
(allow process-exec*)
(allow sysctl-read)
(allow mach-lookup)
(allow ipc-posix-shm)
(allow signal (target self))
(allow file-read-metadata)
(allow file-read*
    ;; root-dir traversal: opening ~/.cargo, ~/.rustup and the dev root requires
    ;; read access to each ancestor directory ENTRY (not its contents). Literals
    ;; expose only the listing of /, /Users and $HOME, never file contents - the
    ;; credential denies below still override.
    (literal "/")
    (literal "/Users")
    (literal "{home}")
    (subpath "/usr")
    (subpath "/bin")
    (subpath "/sbin")
    (subpath "/System")
    (subpath "/Library")
    (subpath "/opt")
    (subpath "/dev")
    (subpath "/etc")
    (subpath "/private/etc")
    (subpath "/private/var/db")
    (subpath "/private/var/folders")
    (subpath "/private/tmp")
    (subpath "{home}/.cargo")
    (subpath "{home}/.rustup")
    (subpath "{dev}"))
(allow file-write*
    (subpath "{dev}")
    (subpath "/dev")
    (subpath "/private/tmp")
    (subpath "/private/var/folders")
    (subpath "/tmp"))
(deny file-read*
    (subpath "{home}/.ssh")
    (subpath "{home}/Library/Keychains")
    (subpath "/Library/Keychains"))
(allow network* (local ip) (remote ip "localhost:*"))
(deny network-outbound (remote ip))
"#
    )
}

/// Build the argv for invoking a program under a Seatbelt profile via
/// `sandbox-exec -f <profile> <program> <args...>`. Returned as owned Strings so
/// the caller can feed a `Command::new("sandbox-exec").args(...)` without
/// lifetime juggling. Pure: does not spawn anything.
pub fn sandbox_exec_argv(profile_path: &Path, program: &str, program_args: &[&str]) -> Vec<String> {
    let mut argv = vec![
        "-f".to_string(),
        profile_path.display().to_string(),
        program.to_string(),
    ];
    argv.extend(program_args.iter().map(|a| a.to_string()));
    argv
}

/// Whether `sandbox-exec` exists on this host. It is deprecated but still ships
/// on macOS 14+. Shadow/enforce modes no-op when it is absent (fail-safe).
pub fn sandbox_exec_available() -> bool {
    std::process::Command::new("which")
        .arg("sandbox-exec")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Write the Seatbelt profile for `dev_root` to `<dev_root>/.clade-sandbox.sb`
/// and return its path, for use as the `-f` argument to `sandbox-exec`.
pub fn write_seatbelt_profile(dev_root: &Path, home: &Path) -> std::io::Result<PathBuf> {
    let path = dev_root.join(".clade-sandbox.sb");
    fs::write(&path, generate_seatbelt_profile(dev_root, home))?;
    Ok(path)
}

/// Outcome of comparing the authoritative (un-sandboxed) build result with the
/// shadow (sandboxed) result. In shadow mode the authoritative result always
/// wins; this verdict only drives profile tuning before enforcement (P4.3).
#[derive(Debug, PartialEq, Eq)]
pub enum ShadowVerdict {
    /// Both agree - the profile neither over- nor under-restricts this build.
    Match,
    /// Real build passed but sandboxed failed - profile is TOO TIGHT and would
    /// break the build if enforced; needs an allowlist addition before P4.3.
    TooTight,
    /// Real build failed but sandboxed passed - anomalous; investigate before
    /// trusting the shadow signal.
    Inconsistent,
}

/// Whether shadow mode is enabled, from the `TRIOS_SANDBOX` env value. Default
/// (unset / any other value) is OFF - the live pipeline is unaffected unless a
/// caller explicitly opts in with `TRIOS_SANDBOX=shadow`. Extracted as a pure
/// function so the default-off guarantee is unit-tested.
pub fn shadow_mode_enabled(env_val: Option<&str>) -> bool {
    matches!(sandbox_mode(env_val), SandboxMode::Shadow)
}

/// Sandbox enforcement level, from the `TRIOS_SANDBOX` env value.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum SandboxMode {
    /// Default. No sandboxing; builds run bare (current behavior).
    Off,
    /// Build runs bare (authoritative) plus an observe-only sandboxed re-run.
    Shadow,
    /// Build runs ONLY under `sandbox-exec` and its result is authoritative.
    Enforce,
}

/// Parse the sandbox mode. Default (unset / unrecognized) is `Off`, so the live
/// pipeline is unaffected unless a caller explicitly opts in.
pub fn sandbox_mode(env_val: Option<&str>) -> SandboxMode {
    match env_val {
        Some("shadow") => SandboxMode::Shadow,
        Some("enforce") => SandboxMode::Enforce,
        _ => SandboxMode::Off,
    }
}

/// Pure comparison of authoritative vs. sandboxed build success.
pub fn shadow_verdict(real_ok: bool, sandboxed_ok: bool) -> ShadowVerdict {
    match (real_ok, sandboxed_ok) {
        (true, true) | (false, false) => ShadowVerdict::Match,
        (true, false) => ShadowVerdict::TooTight,
        (false, true) => ShadowVerdict::Inconsistent,
    }
}

/// Stable lowercase tag for a verdict, for the `event_log.jsonl` `details` field
/// the dashboard/audit parse (avoids depending on Debug formatting).
pub fn verdict_tag(v: &ShadowVerdict) -> &'static str {
    match v {
        ShadowVerdict::Match => "match",
        ShadowVerdict::TooTight => "too_tight",
        ShadowVerdict::Inconsistent => "inconsistent",
    }
}

#[cfg(test)]
// Tests legitimately use expect()/unwrap() for fixtures and invariants; the
// workspace deny/warn policy targets production code paths, not test setup.
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn drop_cleans_up_directory() {
        let dir = PathBuf::from("/tmp/clade-dev-test-drop");
        fs::create_dir_all(&dir).ok();
        fs::write(dir.join("test.txt"), "data").ok();
        assert!(dir.exists());

        {
            let dev = SandboxedDev {
                root: dir.clone(),
                port: 9305,
                cleaned: false,
            };
            drop(dev);
        }
        assert!(!dir.exists());
    }

    #[test]
    fn clean_marks_as_cleaned_and_drop_skips() {
        let dir = PathBuf::from("/tmp/clade-dev-test-clean-drop");
        fs::create_dir_all(&dir).ok();
        assert!(dir.exists());

        let dev = SandboxedDev {
            root: dir.clone(),
            port: 9305,
            cleaned: false,
        };
        let result = dev.clean();
        assert!(result.is_ok());
        assert!(!dir.exists());
    }

    #[test]
    fn seatbelt_profile_is_deny_by_default() {
        let p = generate_seatbelt_profile(Path::new("/tmp/clade-dev/t1"), Path::new("/Users/x"));
        assert!(p.starts_with("(version 1)"));
        assert!(p.contains("(deny default)"));
    }

    #[test]
    fn seatbelt_profile_allows_dev_root_write_and_temp() {
        let p = generate_seatbelt_profile(Path::new("/tmp/clade-dev/t1"), Path::new("/Users/x"));
        // dev root writable; temp dirs toolchains need are present.
        assert!(p.contains("(subpath \"/tmp/clade-dev/t1\")"));
        assert!(p.contains("/private/var/folders"));
    }

    #[test]
    fn seatbelt_profile_allows_rust_toolchain_reads() {
        // P4.2c: real dependency builds read the cargo registry and rustc std
        // libs; without these the sandboxed build is instantly TooTight.
        let p = generate_seatbelt_profile(Path::new("/tmp/d"), Path::new("/Users/x"));
        assert!(p.contains("/Users/x/.cargo"));
        assert!(p.contains("/Users/x/.rustup"));
    }

    #[test]
    fn seatbelt_profile_allows_root_traversal_and_openssl() {
        // P4.2c Match recipe: ancestor-dir literals (to traverse into ~/.cargo)
        // and /private/etc (rustup shim's OpenSSL reads openssl.cnf). Without
        // these rustc is SIGABRT-killed at startup before any output.
        let p = generate_seatbelt_profile(Path::new("/tmp/d"), Path::new("/Users/x"));
        assert!(p.contains("(literal \"/\")"));
        assert!(p.contains("(literal \"/Users\")"));
        assert!(p.contains("(literal \"/Users/x\")"));
        assert!(p.contains("/private/etc"));
    }

    #[test]
    fn seatbelt_profile_denies_credentials_after_read_allow() {
        let p = generate_seatbelt_profile(Path::new("/tmp/clade-dev/t1"), Path::new("/Users/x"));
        assert!(p.contains("/Users/x/.ssh"));
        assert!(p.contains("Library/Keychains"));
        // Last-match-wins: the credential deny must appear AFTER the read allow,
        // otherwise the broad read allow would win and expose ~/.ssh.
        let read_allow = p.find("(allow file-read*").expect("read allow present");
        let cred_deny = p.find("(deny file-read*").expect("credential deny present");
        assert!(cred_deny > read_allow, "credential deny must follow the read allow");
    }

    #[test]
    fn seatbelt_profile_restricts_network_to_localhost() {
        let p = generate_seatbelt_profile(Path::new("/tmp/d"), Path::new("/Users/x"));
        assert!(p.contains("localhost"));
        assert!(p.contains("(deny network-outbound (remote ip))"));
    }

    #[test]
    fn sandbox_exec_argv_builds_wrapped_command() {
        let argv = sandbox_exec_argv(Path::new("/tmp/p.sb"), "swiftc", &["-O", "main.swift"]);
        assert_eq!(argv, vec!["-f", "/tmp/p.sb", "swiftc", "-O", "main.swift"]);
    }

    #[test]
    fn shadow_verdict_agreement_is_match() {
        assert_eq!(shadow_verdict(true, true), ShadowVerdict::Match);
        assert_eq!(shadow_verdict(false, false), ShadowVerdict::Match);
    }

    #[test]
    fn shadow_verdict_real_pass_sandbox_fail_is_too_tight() {
        assert_eq!(shadow_verdict(true, false), ShadowVerdict::TooTight);
    }

    #[test]
    fn shadow_verdict_real_fail_sandbox_pass_is_inconsistent() {
        assert_eq!(shadow_verdict(false, true), ShadowVerdict::Inconsistent);
    }

    #[test]
    fn verdict_tag_is_stable_lowercase() {
        assert_eq!(verdict_tag(&ShadowVerdict::Match), "match");
        assert_eq!(verdict_tag(&ShadowVerdict::TooTight), "too_tight");
        assert_eq!(verdict_tag(&ShadowVerdict::Inconsistent), "inconsistent");
    }

    #[test]
    fn write_seatbelt_profile_writes_deny_default_file() {
        let dir = PathBuf::from("/tmp/clade-sb-write-test");
        fs::create_dir_all(&dir).ok();
        let path = write_seatbelt_profile(&dir, Path::new("/Users/x")).expect("write ok");
        assert!(path.ends_with(".clade-sandbox.sb"));
        let body = fs::read_to_string(&path).expect("read back");
        assert!(body.contains("(deny default)"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn shadow_mode_off_by_default() {
        assert!(!shadow_mode_enabled(None));
        assert!(!shadow_mode_enabled(Some("")));
        assert!(!shadow_mode_enabled(Some("enforce")));
        assert!(shadow_mode_enabled(Some("shadow")));
    }

    #[test]
    fn sandbox_mode_parses_and_defaults_off() {
        assert_eq!(sandbox_mode(None), SandboxMode::Off);
        assert_eq!(sandbox_mode(Some("")), SandboxMode::Off);
        assert_eq!(sandbox_mode(Some("nonsense")), SandboxMode::Off);
        assert_eq!(sandbox_mode(Some("shadow")), SandboxMode::Shadow);
        assert_eq!(sandbox_mode(Some("enforce")), SandboxMode::Enforce);
    }

    #[test]
    fn sandbox_exec_available_is_callable() {
        // Smoke: must not panic. On macOS hosts (trios's target) this is true.
        let _ = sandbox_exec_available();
    }

    #[test]
    fn rejects_path_traversal_ticket_id() {
        let src = PathBuf::from("/tmp/clade-test-src-traversal");
        fs::create_dir_all(&src).ok();
        let result = SandboxedDev::create_from_staging("../evil", &src);
        assert!(result.is_err());
        let _ = fs::remove_dir_all(&src);
    }
}
