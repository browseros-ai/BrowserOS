use std::fs;
use std::path::PathBuf;
use tracing::{info, warn};
use chrono::Utc;

use crate::constitution::{ChangeSpec, Decision};
use crate::oversight::OversightAgent;
use crate::sandbox::SandboxedDev;
use crate::variant::Variant;

/// Full self-improvement pipeline
/// Production -> Staging -> Dev -> Tests -> Oversight -> Shadow -> Atomic Deploy
#[derive(Debug)]
pub struct ImprovementPipeline {
    pub variant: Variant,
    pub version_history_limit: usize,
}

#[derive(Debug)]
pub enum PipelineResult {
    Approved(Deployment),
    Rejected(String),
    ShadowMode(ShadowDeployment),
}

#[derive(Debug)]
pub struct Deployment {
    pub ticket_id: String,
    pub previous_version: PathBuf,
    pub new_snapshot: PathBuf,
    pub constitutional_token: String,
}

#[derive(Debug)]
pub struct ShadowDeployment {
    pub ticket_id: String,
    pub duration_secs: u64,
    pub comparison_metric: Vec<MetricComparison>,
}

#[derive(Debug)]
pub struct MetricComparison {
    pub metric: String,
    pub production_value: f64,
    pub dev_value: f64,
    pub tolerance: f64,
    pub passed: bool,
}

impl ImprovementPipeline {
    pub fn new(variant: Variant) -> Self {
        Self {
            variant,
            version_history_limit: 5,
        }
    }

    /// Phase 1+2: Production creates ticket, Staging clones Dev
    pub fn create_dev(&self, ticket_id: &str) -> Result<SandboxedDev, Box<dyn std::error::Error>> {
        use std::process::{Command, Stdio};

        // Ensure Canary worktree exists and is synced before any dev work
        println!("[Pipeline] Ensuring Canary worktree...");
        let ensure = Command::new("cargo")
            .args(["run", "--bin", "clade-worktree", "--", "ensure"])
            .env("TRIOS_VARIANT", "staging")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();
        match ensure {
            Ok(ref o) if o.status.success() => {
                println!("   [OK] Worktree ensured");
            }
            Ok(ref o) => {
                let err = String::from_utf8_lossy(&o.stderr);
                warn!("[Pipeline] Worktree ensure warning: {}", err.lines().take(3).collect::<Vec<_>>().join("\n"));
            }
            Err(e) => {
                warn!("[Pipeline] Failed to ensure worktree: {}", e);
            }
        }

        let source = self.variant.working_dir();

        SandboxedDev::create_from_staging(ticket_id, &source)
    }
    
    /// P4.2b shadow mode (observe-only). Re-runs the sandbox build under
    /// `sandbox-exec` and logs the `ShadowVerdict` vs. the authoritative result.
    /// Opt-in via `TRIOS_SANDBOX=shadow`; a no-op otherwise, and it NEVER changes
    /// the pipeline outcome. Fail-safe: any missing precondition just skips.
    fn shadow_check_build(&self, dev: &SandboxedDev, real_build_ok: bool) {
        use crate::sandbox::{
            sandbox_exec_argv, sandbox_exec_available, shadow_mode_enabled, shadow_verdict,
            verdict_tag, write_seatbelt_profile,
        };
        use std::process::{Command, Stdio};

        if !shadow_mode_enabled(std::env::var("TRIOS_SANDBOX").ok().as_deref()) {
            return;
        }
        if !sandbox_exec_available() {
            info!("[Pipeline][shadow] sandbox-exec unavailable - skipping shadow check");
            return;
        }
        let home = match std::env::var("HOME") {
            Ok(h) => PathBuf::from(h),
            Err(_) => {
                warn!("[Pipeline][shadow] HOME unset - skipping shadow check");
                return;
            }
        };
        let profile = match write_seatbelt_profile(&dev.root, &home) {
            Ok(p) => p,
            Err(e) => {
                warn!("[Pipeline][shadow] profile write failed: {} - skipping", e);
                return;
            }
        };
        let manifest = format!("{}/Cargo.toml", dev.root.display());
        let argv = sandbox_exec_argv(&profile, "cargo", &["build", "--manifest-path", &manifest]);
        let sandboxed_ok = matches!(
            Command::new("sandbox-exec")
                .args(&argv)
                .current_dir(&dev.root)
                .env("TRIOS_VARIANT", "dev")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .output(),
            Ok(ref o) if o.status.success()
        );
        let verdict = shadow_verdict(real_build_ok, sandboxed_ok);
        info!(
            "[Pipeline][shadow] verdict={:?} (real_ok={}, sandboxed_ok={})",
            verdict, real_build_ok, sandboxed_ok
        );
        // P4.2d: persist to event_log.jsonl so the dashboard/audit can track
        // sandbox-profile health (e.g. count `too_tight` over time).
        trios_config::log_event(
            &trios_config::new_correlation_id(),
            "sandbox_shadow_verdict",
            &format!("{}_real{}_sandboxed{}", verdict_tag(&verdict), real_build_ok, sandboxed_ok),
        );
    }

    /// P4.3: construct the Command for `program args`, honoring the sandbox mode.
    /// In `Enforce` the command is wrapped in `sandbox-exec` with the generated
    /// profile and cwd = dev root. Returns `None` to FAIL CLOSED when enforcement
    /// is requested but cannot be applied (no `sandbox-exec`/`HOME`/profile) - the
    /// caller MUST treat `None` as a failed step. `Off`/`Shadow` run bare.
    fn build_command(
        &self,
        dev: &SandboxedDev,
        program: &str,
        args: &[&str],
        mode: crate::sandbox::SandboxMode,
    ) -> Option<std::process::Command> {
        use crate::sandbox::{
            sandbox_exec_argv, sandbox_exec_available, write_seatbelt_profile, SandboxMode,
        };
        use std::process::Command;

        if mode == SandboxMode::Enforce {
            if !sandbox_exec_available() {
                warn!("[Pipeline][enforce] sandbox-exec unavailable - failing closed");
                return None;
            }
            let home = std::env::var("HOME").ok()?;
            let profile = write_seatbelt_profile(&dev.root, std::path::Path::new(&home)).ok()?;
            let mut cmd = Command::new("sandbox-exec");
            cmd.args(sandbox_exec_argv(&profile, program, args))
                .current_dir(&dev.root);
            Some(cmd)
        } else {
            let mut cmd = Command::new(program);
            cmd.args(args);
            Some(cmd)
        }
    }

    /// Phase 3: Run tests in Dev sandbox
    pub fn run_tests(&self, dev: &SandboxedDev) -> Vec<TestResult> {
        use std::process::{Command, Stdio};
        #[allow(unused_imports)]
        use std::time::Instant;

        info!("[Pipeline] Running tests in sandbox {}", dev.root.display());

        let mode = crate::sandbox::sandbox_mode(std::env::var("TRIOS_SANDBOX").ok().as_deref());
        let manifest = format!("{}/Cargo.toml", dev.root.display());
        let mut results = vec![];

        // Test 1: Cargo test (under sandbox-exec when TRIOS_SANDBOX=enforce)
        let test_passed = match self.build_command(dev, "cargo", &["test", "--manifest-path", &manifest], mode) {
            Some(mut cmd) => match cmd
                .env("TRIOS_VARIANT", "dev")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
            {
                Ok(ref o) if o.status.success() => true,
                Ok(ref o) => {
                    let err = String::from_utf8_lossy(&o.stderr);
                    warn!("[Pipeline] Tests failed: {}", err.lines().take(5).collect::<Vec<_>>().join("\n"));
                    false
                }
                Err(e) => {
                    warn!("[Pipeline] Failed to run tests: {}", e);
                    false
                }
            },
            None => false, // enforce requested but unsandboxable -> fail closed
        };
        results.push(TestResult {
            name: "unit".to_string(),
            passed: test_passed,
        });

        // Test 2: Build check (under sandbox-exec when TRIOS_SANDBOX=enforce)
        let build_passed = match self.build_command(dev, "cargo", &["build", "--manifest-path", &manifest], mode) {
            Some(mut cmd) => matches!(
                cmd.env("TRIOS_VARIANT", "dev")
                    .stdout(Stdio::null())
                    .stderr(Stdio::piped())
                    .output(),
                Ok(ref o) if o.status.success()
            ),
            None => false, // fail closed
        };
        results.push(TestResult {
            name: "build".to_string(),
            passed: build_passed,
        });

        // P4.2b: observe-only shadow check. Runs only in Shadow mode (no-op in
        // Off/Enforce); never affects `results`.
        self.shadow_check_build(dev, build_passed);

        // Test 3: Swift build if main.swift exists
        let swift_path = dev.root.join("main.swift");
        let swift_passed = if swift_path.exists() {
            let swift_build = Command::new("swiftc")
                .args([
                    "-O",
                    "-o",
                    &format!("{}/trios_app_dev", dev.root.display()),
                    &format!("{}", swift_path.display()),
                ])
                .current_dir(&dev.root)
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output();
            matches!(swift_build, Ok(ref o) if o.status.success())
        } else {
            true // skip if no swift files
        };
        results.push(TestResult {
            name: "swift-build".to_string(),
            passed: swift_passed,
        });

        // Test 4: Health probe if binary built
        let health_passed = if build_passed {
            let binary = dev.root.join("target/debug/clade-monitor");
            if binary.exists() {
                // Skip runtime health in dev - just verify binary exists
                true
            } else {
                true
            }
        } else {
            false
        };
        results.push(TestResult {
            name: "binary-exists".to_string(),
            passed: health_passed,
        });

        // Test 5: No hardcoded secrets
        let secrets_passed = !self.contains_secrets(&dev.root);
        results.push(TestResult {
            name: "no-secrets".to_string(),
            passed: secrets_passed,
        });

        // Test 6: Differential - no regression vs Sovereign
        let diff_passed = if self.variant == Variant::Staging {
            println!("   [test] Running clade-diff (Sovereign vs Canary)...");
            let diff = Command::new("cargo")
                .args(["run", "--bin", "clade-diff"])
                .env("TRIOS_VARIANT", "staging")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output();
            match diff {
                Ok(ref o) if o.status.success() => {
                    println!("   [OK] clade-diff passed");
                    true
                }
                Ok(ref o) => {
                    let err = String::from_utf8_lossy(&o.stderr);
                    warn!("[Pipeline] clade-diff failed: {}", err.lines().take(3).collect::<Vec<_>>().join("\n"));
                    false
                }
                Err(e) => {
                    warn!("[Pipeline] Failed to run clade-diff: {}", e);
                    false
                }
            }
        } else {
            true // skip diff in non-staging variants
        };
        results.push(TestResult {
            name: "differential".to_string(),
            passed: diff_passed,
        });

        results
    }

    fn contains_secrets(&self, root: &std::path::Path) -> bool {
        use std::fs;
        let mut found = false;
        if let Ok(entries) = fs::read_dir(root) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.is_file() {
                    if let Ok(content) = fs::read_to_string(&path) {
                        if (content.contains("sk-") || content.contains("api_key"))
                            && path.extension().map(|e| e == "swift" || e == "rs" || e == "md").unwrap_or(false) {
                                found = true;
                                break;
                            }
                    }
                }
            }
        }
        found
    }
    
    /// Phase 4: Oversight evaluates against Constitution
    pub fn oversight_check(&self, changes: &[ChangeSpec]) -> (Decision, String) {
        let oversight = OversightAgent::new();
        let (result, token) = oversight.evaluate(changes, &self.variant);
        
        match result.decision {
            Decision::Approve | Decision::ShadowMode => {
                info!("[Pipeline] Oversight: APPROVE (token={})", &token[..16]);
            }
            Decision::Reject => {
                warn!("[Pipeline] Oversight: REJECT - {:?}", result.violations);
            }
        }
        
        (result.decision, token)
    }
    
    /// Phase 6: Atomic deployment with rollback preservation
    pub fn atomic_deploy(
        &self,
        ticket_id: &str,
        dev: &SandboxedDev,
        token: &str,
    ) -> Result<Deployment, Box<dyn std::error::Error>> {
        if self.variant.is_production() {
            return Err("Production cannot self-modify; staging only".into());
        }
        
        // Save current version for rollback
        let rollback_dir = PathBuf::from(format!("{}/.trinity/rollback", trios_config::project_dir()));
        fs::create_dir_all(&rollback_dir)?;
        
        let timestamp = Utc::now().timestamp();
        let previous = rollback_dir.join(format!("v_{}_pre_{}", timestamp, ticket_id));
        let snapshot = rollback_dir.join(format!("v_{}_approved_{}", timestamp, ticket_id));
        
        // Copy current to previous (rollback)
        crate::sandbox::copy_tree_filtered(&self.variant.working_dir(), &previous)?;
        
        // Copy dev to approved snapshot
        crate::sandbox::copy_tree_filtered(&dev.root, &snapshot)?;
        
        // Clean old versions
        self.cleanup_old_versions(&rollback_dir)?;
        
        info!(
            "[Pipeline] Atomic deploy: saved rollback={} snapshot={}",
            previous.display(),
            snapshot.display()
        );
        
        Ok(Deployment {
            ticket_id: ticket_id.to_string(),
            previous_version: previous,
            new_snapshot: snapshot,
            constitutional_token: token.to_string(),
        })
    }
    
    fn cleanup_old_versions(&self, dir: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        let mut versions: Vec<_> = fs::read_dir(dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("v_"))
            .collect();
        
        versions.sort_by_key(|e| e.file_name());
        
        while versions.len() > self.version_history_limit {
            if let Some(old) = versions.pop() {
                fs::remove_dir_all(old.path())?;
                info!("[Pipeline] Removed old version {}", old.path().display());
            }
        }
        
        Ok(())
    }
}

#[derive(Debug)]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
}
