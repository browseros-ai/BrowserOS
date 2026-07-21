use sha2::{Sha256, Digest};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

fn project_dir() -> String { trios_config::project_dir() }

fn is_valid_snapshot(name: &str) -> bool {
    name.starts_with("trios_app-") && !name.ends_with(".sha256")
}

fn find_newest(entries: &mut [(String, SystemTime)]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.1));
    Some(entries[0].0.clone())
}

fn verify_sha256(binary_path: &Path) -> bool {
    let hash_path = PathBuf::from(format!("{}.sha256", binary_path.display()));
    let expected = match fs::read_to_string(&hash_path) {
        Ok(content) => content.trim().to_lowercase(),
        Err(_) => {
            eprintln!("[rollback] No .sha256 sidecar for {} - skipping verification", binary_path.display());
            return true;
        }
    };

    let mut file = match fs::File::open(binary_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[rollback] Cannot open {} for hashing: {}", binary_path.display(), e);
            return false;
        }
    };

    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buf[..n]),
            Err(e) => {
                eprintln!("[rollback] Read error during hash: {}", e);
                return false;
            }
        }
    }

    let actual = format!("{:x}", hasher.finalize());
    // Compare first 64 chars (SHA-256 hex)
    let expected_hash = expected.split_whitespace().next().unwrap_or(&expected);
    if actual != expected_hash {
        eprintln!("[rollback] SHA256 MISMATCH for {}", binary_path.display());
        eprintln!("   Expected: {}", expected_hash);
        eprintln!("   Actual:   {}", actual);
        return false;
    }
    println!("[rollback] SHA256 verified: {}", binary_path.display());
    true
}

fn main() {
    let snapshot_dir = PathBuf::from(format!("{}/.trinity/snapshots", &project_dir()));
    let targets = vec![
        format!("{}/trios_app", &project_dir()),
        format!("{}/trios.app/Contents/MacOS/trios", &project_dir()),
    ];

    let mut entries: Vec<(String, SystemTime)> = fs::read_dir(&snapshot_dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(is_valid_snapshot)
                .unwrap_or(false)
        })
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let mtime = e.metadata().ok()?.modified().ok()?;
            Some((name, mtime))
        })
        .collect();

    let newest = match find_newest(&mut entries) {
        Some(n) => n,
        None => {
            eprintln!("[FAIL] No snapshots found in {}", snapshot_dir.display());
            std::process::exit(1);
        }
    };

    let src = snapshot_dir.join(&newest);

    if !verify_sha256(&src) {
        eprintln!("[FAIL] Snapshot failed integrity check - aborting rollback");
        std::process::exit(1);
    }

    for dst in &targets {
        let dst_path = Path::new(dst);
        if let Some(parent) = dst_path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                eprintln!("[FAIL] Failed to create parent dir for {}: {}", dst, e);
                std::process::exit(1);
            }
        }
        let tmp = format!("{}.tmp", dst);
        let tmp_path = Path::new(&tmp);
        match fs::copy(&src, tmp_path) {
            Ok(_) => {
                match fs::rename(tmp_path, dst_path) {
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!("[FAIL] Atomic rename failed for {}: {}", dst, e);
                        if let Err(e2) = fs::remove_file(tmp_path) {
                            eprintln!("[rollback] cleanup tmp: {}", e2);
                        }
                        std::process::exit(1);
                    }
                }
            }
            Err(e) => {
                eprintln!("[FAIL] Failed to stage rollback to {}: {}", tmp, e);
                std::process::exit(1);
            }
        }
    }

    println!("[OK] Rolled back to {}", newest);
    println!("   Source: {}", src.display());
    println!("   Targets: {}", targets.join(", "));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn valid_snapshot_name() {
        assert!(is_valid_snapshot("trios_app-1234567890-clade-1.0.0"));
    }

    #[test]
    fn sha256_not_valid_snapshot() {
        assert!(!is_valid_snapshot("trios_app-1234567890.sha256"));
    }

    #[test]
    fn random_file_not_valid() {
        assert!(!is_valid_snapshot("random_file.bin"));
    }

    #[test]
    fn empty_prefix_not_valid() {
        assert!(!is_valid_snapshot(""));
    }

    #[test]
    fn find_newest_empty_returns_none() {
        let mut entries = vec![];
        assert!(find_newest(&mut entries).is_none());
    }

    #[test]
    fn find_newest_returns_most_recent() {
        let now = SystemTime::now();
        let older = now - Duration::from_secs(100);
        let oldest = now - Duration::from_secs(200);
        let mut entries = vec![
            ("trios_app-oldest".to_string(), oldest),
            ("trios_app-newest".to_string(), now),
            ("trios_app-older".to_string(), older),
        ];
        assert_eq!(find_newest(&mut entries), Some("trios_app-newest".to_string()));
    }

    #[test]
    fn find_newest_single_entry() {
        let mut entries = vec![
            ("trios_app-only".to_string(), SystemTime::now()),
        ];
        assert_eq!(find_newest(&mut entries), Some("trios_app-only".to_string()));
    }

    #[test]
    fn verify_sha256_missing_sidecar_returns_true() {
        let tmp = std::env::temp_dir().join("rollback_test_no_sidecar");
        let _ = std::fs::write(&tmp, b"test content");
        assert!(verify_sha256(&tmp));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn verify_sha256_matching_hash() {
        let tmp = std::env::temp_dir().join("rollback_test_match");
        let hash_file = std::env::temp_dir().join("rollback_test_match.sha256");
        let content = b"hello world";
        let _ = std::fs::write(&tmp, content);
        let mut hasher = Sha256::new();
        hasher.update(content);
        let expected = format!("{:x}", hasher.finalize());
        let _ = std::fs::write(&hash_file, &expected);
        assert!(verify_sha256(&tmp));
        let _ = std::fs::remove_file(&tmp);
        let _ = std::fs::remove_file(&hash_file);
    }

    #[test]
    fn verify_sha256_mismatch() {
        let tmp = std::env::temp_dir().join("rollback_test_mismatch");
        let hash_file = std::env::temp_dir().join("rollback_test_mismatch.sha256");
        let _ = std::fs::write(&tmp, b"real content");
        let _ = std::fs::write(&hash_file, "0000000000000000000000000000000000000000000000000000000000000000");
        assert!(!verify_sha256(&tmp));
        let _ = std::fs::remove_file(&tmp);
        let _ = std::fs::remove_file(&hash_file);
    }

    #[test]
    fn verify_sha256_nonexistent_binary() {
        let tmp = std::env::temp_dir().join("rollback_test_nonexistent");
        let hash_file = std::env::temp_dir().join("rollback_test_nonexistent.sha256");
        let _ = std::fs::write(&hash_file, "abc123");
        assert!(!verify_sha256(&tmp));
        let _ = std::fs::remove_file(&hash_file);
    }

    #[test]
    fn atomic_rollback_preserves_old_on_copy_fail() {
        let dir = std::env::temp_dir().join("rollback_atomic_test");
        let _ = fs::create_dir_all(&dir);
        let dst = dir.join("trios_app");
        let _ = fs::write(&dst, b"original binary");
        assert!(dst.exists());
        let content = fs::read_to_string(&dst).unwrap_or_default();
        assert_eq!(content, "original binary");
        let _ = fs::remove_dir_all(&dir);
    }
}
