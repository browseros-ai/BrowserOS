use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;

/// Dev agent sandboxing via Capability Control (Bostrom et al.)
/// Production: chroot, network namespaces, cgroups
#[derive(Debug)]
pub struct SandboxedDev {
    pub root: PathBuf,
    pub port: u16,
    pub network_isolated: bool,
    pub resource_caps: ResourceCaps,
}

#[derive(Debug, Clone)]
pub struct ResourceCaps {
    pub max_memory_mb: usize,
    pub max_cpu_cores: usize,
    pub max_execution_secs: u64,
}

impl Default for ResourceCaps {
    fn default() -> Self {
        Self {
            max_memory_mb: 512,
            max_cpu_cores: 1,
            max_execution_secs: 300,
        }
    }
}

impl SandboxedDev {
    pub fn create_from_staging(ticket_id: &str, source: &Path) -> Result<Self, Box<dyn std::error::Error>> {
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
            network_isolated: true,
            resource_caps: ResourceCaps::default(),
        })
    }
    
    pub fn clean(self) -> Result<(), Box<dyn std::error::Error>> {
        if self.root.exists() {
            fs::remove_dir_all(&self.root)?;
            info!("[Sandbox] Cleaned {}", self.root.display());
        }
        Ok(())
    }
}

pub fn copy_tree_filtered(src: &Path, dst: &Path) -> std::io::Result<()> {
    let ignore = [
        ".env", ".env.local", "node_modules", "sandbox", 
        ".git", "__pycache__", "*.key", "*.pem",
        "browseros-server.log", "trios-server.log" // logs may contain secrets
    ];
    
    fs::create_dir_all(dst)?;
    
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        
        if ignore.iter().any(|p| name.contains(p)) {
            continue;
        }
        
        let src_path = entry.path();
        let dst_path = dst.join(&name);
        
        if src_path.is_dir() {
            copy_tree_filtered(&src_path, &dst_path)?;
        } else {
            // Skip files with tokens (heuristic: line contains 'sk-')
            if name.ends_with(".env") || name.ends_with(".toml") {
                let content = fs::read_to_string(&src_path).unwrap_or_default();
                if content.contains("sk-") || content.contains("api_key") {
                    fs::write(&dst_path, "# REDACTED — secrets removed by clade-improve\n")?;
                    continue;
                }
            }
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}
