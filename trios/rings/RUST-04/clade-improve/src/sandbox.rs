use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;

/// Filtered dev copy — secret-redacting source clone, NOT yet OS-isolated.
/// Copies source tree excluding secrets/keys, runs in /tmp.
///
/// OS-level isolation is being added incrementally: `generate_seatbelt_profile`
/// and `sandbox_exec_argv` below produce a deny-by-default macOS Seatbelt policy
/// for wrapping the build/test exec. They are built and unit-tested but NOT yet
/// wired into the live pipeline — see `.trinity/docs/p4-sandbox-isolation.md`
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
        let dev_root = PathBuf::from(format!("/tmp/clade-dev/{}", ticket_id));
        
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
                            fs::write(&dst_path, "# REDACTED — secrets removed by clade-improve\n")?;
                            continue;
                        }
                    }
                    Err(e) => {
                        fs::write(
                            &dst_path,
                            format!("# REDACTED — unreadable, treated as secret ({e})\n"),
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
(allow file-read-metadata)
(allow file-read*
    (subpath "/usr")
    (subpath "/bin")
    (subpath "/sbin")
    (subpath "/System")
    (subpath "/Library")
    (subpath "/private/var/db")
    (subpath "/opt")
    (subpath "{dev}"))
(allow file-write*
    (subpath "{dev}")
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
    fn rejects_path_traversal_ticket_id() {
        let src = PathBuf::from("/tmp/clade-test-src-traversal");
        fs::create_dir_all(&src).ok();
        let result = SandboxedDev::create_from_staging("../evil", &src);
        assert!(result.is_err());
        let _ = fs::remove_dir_all(&src);
    }
}
