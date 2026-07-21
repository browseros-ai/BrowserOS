use std::path::Path;
use std::process::{Command, Stdio};

const DEFAULT_TRIOS_ROOT: &str = "/Users/playra/BrowserOS-full/trios";
const WORKTREE_BRANCH: &str = "canary";
const DIRTY_THRESHOLD: usize = 5;
const SYNC_BEHIND_THRESHOLD: usize = 3;

fn project_dir_with(root: Option<&str>) -> String {
    root.map(|s| s.to_string())
        .or_else(|| std::env::var("TRIOS_ROOT").ok())
        .unwrap_or_else(|| DEFAULT_TRIOS_ROOT.to_string())
}

fn project_dir() -> String {
    project_dir_with(None)
}

fn worktree_dir_with(root: Option<&str>) -> String {
    format!("{}/.worktrees/staging", project_dir_with(root))
}

fn worktree_dir() -> String {
    worktree_dir_with(None)
}

fn validate_worktree_path_with(root: Option<&str>) -> bool {
    let wt = worktree_dir_with(root);
    let proj = project_dir_with(root);

    let canonical_wt = match std::fs::canonicalize(&wt) {
        Ok(p) => p,
        Err(_) => return false,
    };
    let canonical_proj = match std::fs::canonicalize(&proj) {
        Ok(p) => p,
        Err(_) => return false,
    };

    if !canonical_wt.starts_with(&canonical_proj) {
        eprintln!("[CladeWorktree] SECURITY: worktree {} is outside project {}", canonical_wt.display(), canonical_proj.display());
        return false;
    }

    match git_wt(&["rev-parse", "--git-dir"]) {
        Ok(git_dir) => {
            if !git_dir.contains(".worktrees") && !git_dir.contains("worktrees") {
                eprintln!("[CladeWorktree] SECURITY: git-dir {} does not look like a worktree", git_dir);
                return false;
            }
            true
        }
        Err(_) => false,
    }
}

fn validate_worktree_path() -> bool {
    validate_worktree_path_with(None)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let action = args.first().map(|s| s.as_str()).unwrap_or("status");

    println!("[CladeWorktree] Action: {} | path: {}", action, worktree_dir());

    match action {
        "status" => status(),
        "guard" => guard(),
        "commit" => commit(),
        "sync" => sync(),
        "ensure" => ensure(),
        "reset" => reset(),
        _ => {
            println!("Usage: cargo run --bin clade-worktree -- [status|guard|commit|sync|ensure|reset]");
            std::process::exit(1);
        }
    }
}

fn git_base(args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(project_dir())
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn git_wt(args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(worktree_dir())
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn worktree_exists() -> bool {
    let wt = worktree_dir();
    Path::new(&wt).exists()
        && Path::new(&format!("{}/.git", wt)).exists()
}

fn ensure() {
    println!("   [ensure] Checking Canary worktree...");

    // 1. Check worktree exists
    if !worktree_exists() {
        println!("   ⚠️  Worktree missing — creating...");
        let wt = worktree_dir();
        if let Err(e) = git_base(&[
            "worktree", "add", &wt, WORKTREE_BRANCH,
        ]) {
            println!("   ❌ Failed to create worktree: {}", e);
            std::process::exit(1);
        }
        println!("   ✅ Created worktree at {} (branch: {})", wt, WORKTREE_BRANCH);
    } else {
        println!("   ✅ Worktree exists");
    }

    // 2. Ensure branch exists
    let branch_check = git_base(&["show-ref", "--verify", &format!("refs/heads/{}", WORKTREE_BRANCH)]);
    if branch_check.is_err() {
        println!("   ⚠️  Branch '{}' not found — creating from HEAD", WORKTREE_BRANCH);
        if let Err(e) = git_base(&["branch", WORKTREE_BRANCH]) {
            println!("   ❌ Failed to create branch: {}", e);
            std::process::exit(1);
        }
    }

    // 3. Sync with origin/main (or current HEAD branch)
    println!("   [ensure] Syncing with upstream...");
    sync();

    // 4. Verify dirty threshold
    let files = dirty_files();
    if !files.is_empty() {
        println!("   ⚠️  Worktree dirty ({} files) — auto-committing...", files.len());
        commit();
    }

    // 5. Sync .claude/ agents/rules/skills into worktree
    sync_claude_artifacts();

    // 6. Verify buildable
    println!("   [ensure] Verifying build...");
    let build_ok = Command::new("cargo")
        .args(["check", "--manifest-path", &format!("{}/Cargo.toml", worktree_dir())])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if build_ok {
        println!("   ✅ Cargo check passed");
    } else {
        println!("   ⚠️  Cargo check failed — worktree may need manual fix");
    }

    println!("\n   🟢 Worktree ENSURED: {}", worktree_dir());
}

fn reset() {
    println!("   [reset] Hard-resetting Canary to origin/main...");

    if !worktree_exists() {
        ensure();
        return;
    }

    // Fetch latest
    if let Err(e) = git_wt(&["fetch", "origin"]) {
        println!("   ❌ Fetch failed: {}", e);
        return;
    }

    // Reset hard
    match git_wt(&["reset", "--hard", "origin/feat/zai-provider"]) {
        Ok(_) => println!("   ✅ Hard-reset to origin/feat/zai-provider"),
        Err(e) => {
            println!("   ❌ Reset failed: {}", e);
            return;
        }
    }

    // Validate worktree is inside project before destructive clean
    if validate_worktree_path() {
        if let Err(e) = git_wt(&["clean", "-fd"]) {
            println!("   ⚠️  Clean untracked failed: {}", e);
        }
    } else {
        println!("   ❌ Worktree path validation failed — skipping git clean");
    }

    println!("   ✅ Canary reset complete");
}

fn sync_claude_artifacts() {
    println!("   [ensure] Syncing .claude/ artifacts...");
    use std::fs;

    let sources = [
        format!("{}/.claude/agents", &project_dir()),
        format!("{}/.claude/rules", &project_dir()),
        format!("{}/.claude/skills", &project_dir()),
    ];
    let target_base = format!("{}/.claude", worktree_dir());

    for src in &sources {
        if !Path::new(src).exists() {
            continue;
        }
        let name = match Path::new(src).file_name() {
            Some(n) => n.to_string_lossy(),
            None => continue,
        };
        let dst = format!("{}/{}", target_base, name);

        if Path::new(&dst).exists() {
            let _ = fs::remove_dir_all(&dst);
        }

        let status = Command::new("cp")
            .args(["-R", src, &dst])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if status {
            println!("      ✅ {}", name);
        } else {
            println!("      ⚠️  {} (cp failed, may be stale)", name);
        }
    }
}

fn dirty_files() -> Vec<String> {
    match git_wt(&["status", "--porcelain"]) {
        Ok(out) => out.lines().map(|s| s.to_string()).collect(),
        Err(_) => vec![],
    }
}

fn current_branch() -> String {
    git_wt(&["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_else(|_| "unknown".to_string())
}

fn status() {
    let files = dirty_files();
    let branch = current_branch();
    let ahead = git_wt(&["rev-list", "--count", "@{u}..HEAD"]).unwrap_or_else(|_| "?".to_string());
    let behind = git_wt(&["rev-list", "--count", "HEAD..@{u}"]).unwrap_or_else(|_| "?".to_string());

    let exists = worktree_exists();
    println!("   Exists: {}", if exists { "✅" } else { "❌" });
    println!("   Branch: {}", branch);
    println!("   Dirty files: {} {}", files.len(), if files.len() > DIRTY_THRESHOLD { "⚠️  exceeds threshold" } else { "" });
    println!("   Ahead: {} | Behind: {}", ahead, behind);
    if !files.is_empty() {
        println!("   Files:");
        for f in files.iter().take(10) {
            println!("      {}", f);
        }
        if files.len() > 10 {
            println!("      ... and {} more", files.len() - 10);
        }
    }
}

fn guard() {
    if !worktree_exists() {
        println!("   ❌ Worktree missing — run 'ensure' first");
        std::process::exit(1);
    }

    let files = dirty_files();
    if files.len() > DIRTY_THRESHOLD {
        println!("   ⚠️  Dirty files {} > {} — triggering auto-commit", files.len(), DIRTY_THRESHOLD);
        commit();
    } else {
        println!("   ✅ Dirty files {} within threshold", files.len());
    }

    let behind = git_wt(&["rev-list", "--count", "HEAD..@{u}"]).unwrap_or_else(|_| "0".to_string());
    if behind.parse::<usize>().unwrap_or(0) > SYNC_BEHIND_THRESHOLD {
        println!("   ⚠️  Worktree is {} commits behind upstream (> {}) — auto-syncing", behind, SYNC_BEHIND_THRESHOLD);
        sync();
    } else if behind != "0" && behind != "?" {
        println!("   ⚠️  Worktree is {} commits behind upstream — run sync", behind);
    }
}

fn commit() {
    let files = dirty_files();
    if files.is_empty() {
        println!("   ✅ Nothing to commit");
        return;
    }

    let msg = format!("[clade-worktree] Auto-commit {} dirty files", files.len());
    if let Err(e) = git_wt(&["add", "-A"]) {
        println!("   ❌ git add failed: {}", e);
        return;
    }
    match git_wt(&["commit", "-m", &msg]) {
        Ok(_) => println!("   ✅ Committed: {}", msg),
        Err(e) => println!("   ❌ git commit failed: {}", e),
    }
}

fn sync() {
    println!("   Fetching origin...");
    match git_wt(&["fetch", "origin"]) {
        Ok(_) => println!("   ✅ Fetch complete"),
        Err(e) => {
            println!("   ❌ Fetch failed: {}", e);
            return;
        }
    }

    let behind = git_wt(&["rev-list", "--count", "HEAD..@{u}"]).unwrap_or_else(|_| "0".to_string());
    if behind == "0" || behind == "?" {
        println!("   ✅ Already up to date");
    } else {
        println!("   ⚠️  {} commits behind — fast-forwarding...", behind);
        match git_wt(&["merge", "--ff-only", "@{u}"]) {
            Ok(_) => println!("   ✅ Fast-forwarded"),
            Err(e) => println!("   ❌ Merge failed (diverged?): {}", e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worktree_branch_is_canary() {
        assert_eq!(WORKTREE_BRANCH, "canary");
    }

    #[test]
    fn dirty_threshold_is_5() {
        assert_eq!(DIRTY_THRESHOLD, 5);
    }

    #[test]
    fn sync_behind_threshold_is_3() {
        assert_eq!(SYNC_BEHIND_THRESHOLD, 3);
    }

    #[test]
    fn worktree_dir_contains_staging() {
        let dir = worktree_dir_with(Some(DEFAULT_TRIOS_ROOT));
        assert!(dir.contains(".worktrees/staging"), "worktree_dir should point to .worktrees/staging, got: {}", dir);
    }

    #[test]
    fn project_dir_has_fallback() {
        let dir = project_dir_with(Some(DEFAULT_TRIOS_ROOT));
        assert!(!dir.is_empty());
    }

    #[test]
    fn worktree_dir_is_inside_project() {
        // Validate against the canonical (resolved) paths so env-var overrides do
        // not break the assertion when the directory does not exist.
        let wt = std::fs::canonicalize(worktree_dir_with(Some(DEFAULT_TRIOS_ROOT)))
            .unwrap_or_else(|_| Path::new(&worktree_dir_with(Some(DEFAULT_TRIOS_ROOT))).to_path_buf());
        let proj = std::fs::canonicalize(project_dir_with(Some(DEFAULT_TRIOS_ROOT)))
            .unwrap_or_else(|_| Path::new(&project_dir_with(Some(DEFAULT_TRIOS_ROOT))).to_path_buf());
        assert!(wt.starts_with(&proj), "worktree {} must be under project {}", wt.display(), proj.display());
    }

    #[test]
    fn validate_worktree_rejects_outside_path() {
        // Point validation at a nonexistent project root and ensure it fails
        // because the worktree path cannot be canonicalized. This test deliberately
        // does not mutate the process environment so parallel test execution stays
        // deterministic.
        let result = validate_worktree_path_with(Some("/tmp/nonexistent-trios-validate-test"));
        assert!(!result);
    }
}
