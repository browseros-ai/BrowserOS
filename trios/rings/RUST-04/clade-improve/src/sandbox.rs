use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;

/// Filtered dev copy — NOT a security sandbox.
/// Copies source tree excluding secrets/keys, runs in /tmp. No OS-level isolation.
/// TODO: enforce real isolation via sandbox-exec (macOS) before running untrusted code.
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

#[cfg(test)]
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
    fn rejects_path_traversal_ticket_id() {
        let src = PathBuf::from("/tmp/clade-test-src-traversal");
        fs::create_dir_all(&src).ok();
        let result = SandboxedDev::create_from_staging("../evil", &src);
        assert!(result.is_err());
        let _ = fs::remove_dir_all(&src);
    }
}
