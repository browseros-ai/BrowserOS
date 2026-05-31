use std::path::Path;
use std::process::{Command, Stdio};

const PROJECT_DIR: &str = "/Users/playra/BrowserOS-full/trios";
const WORKTREE_DIR: &str = "/Users/playra/BrowserOS-full/trios/.worktrees/staging";
const WORKTREE_BRANCH: &str = "canary";
const DIRTY_THRESHOLD: usize = 5;
const SYNC_BEHIND_THRESHOLD: usize = 3;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let action = args.get(0).map(|s| s.as_str()).unwrap_or("status");

    println!("[CladeWorktree] Action: {} | path: {}", action, WORKTREE_DIR);

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
        .current_dir(PROJECT_DIR)
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
        .current_dir(WORKTREE_DIR)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn worktree_exists() -> bool {
    Path::new(WORKTREE_DIR).exists()
        && Path::new(&format!("{}/.git", WORKTREE_DIR)).exists()
}

fn ensure() {
    println!("   [ensure] Checking Canary worktree...");

    // 1. Check worktree exists
    if !worktree_exists() {
        println!("   ⚠️  Worktree missing — creating...");
        if let Err(e) = git_base(&[
            "worktree", "add", WORKTREE_DIR, WORKTREE_BRANCH,
        ]) {
            println!("   ❌ Failed to create worktree: {}", e);
            std::process::exit(1);
        }
        println!("   ✅ Created worktree at {} (branch: {})", WORKTREE_DIR, WORKTREE_BRANCH);
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
        .args(["check", "--manifest-path", &format!("{}/Cargo.toml", WORKTREE_DIR)])
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

    println!("\n   🟢 Worktree ENSURED: {}", WORKTREE_DIR);
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

    // Clean untracked
    let _ = git_wt(&["clean", "-fd"]); // remove untracked files

    println!("   ✅ Canary reset complete");
}

fn sync_claude_artifacts() {
    println!("   [ensure] Syncing .claude/ artifacts...");
    use std::fs;

    let sources = [
        format!("{}/.claude/agents", PROJECT_DIR),
        format!("{}/.claude/rules", PROJECT_DIR),
        format!("{}/.claude/skills", PROJECT_DIR),
    ];
    let target_base = format!("{}/.claude", WORKTREE_DIR);

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
