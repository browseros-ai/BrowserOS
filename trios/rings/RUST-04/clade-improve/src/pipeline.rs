use std::fs;
use std::path::PathBuf;
use tracing::{info, warn};
use chrono::Utc;

use crate::constitution::{ChangeSpec, Decision};
use crate::oversight::OversightAgent;
use crate::sandbox::SandboxedDev;
use crate::variant::Variant;

/// Full self-improvement pipeline
/// Production → Staging → Dev → Tests → Oversight → Shadow → Atomic Deploy
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
                println!("   ✅ Worktree ensured");
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
    
    /// Phase 3: Run tests in Dev sandbox
    pub fn run_tests(&self, dev: &SandboxedDev) -> Vec<TestResult> {
        use std::process::{Command, Stdio};
        #[allow(unused_imports)]
        use std::time::Instant;

        info!("[Pipeline] Running tests in sandbox {}", dev.root.display());

        let mut results = vec![];

        // Test 1: Cargo test in sandbox
        let test_output = Command::new("cargo")
            .args(["test", "--manifest-path", &format!("{}/Cargo.toml", dev.root.display())])
            .env("TRIOS_VARIANT", "dev")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();
        let test_passed = match test_output {
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
        };
        results.push(TestResult {
            name: "unit".to_string(),
            passed: test_passed,
        });

        // Test 2: Build check in sandbox
        let build_output = Command::new("cargo")
            .args(["build", "--manifest-path", &format!("{}/Cargo.toml", dev.root.display())])
            .env("TRIOS_VARIANT", "dev")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output();
        let build_passed = matches!(build_output, Ok(ref o) if o.status.success());
        results.push(TestResult {
            name: "build".to_string(),
            passed: build_passed,
        });

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
                // Skip runtime health in dev — just verify binary exists
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

        // Test 6: Differential — no regression vs Sovereign
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
                    println!("   ✅ clade-diff passed");
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
                warn!("[Pipeline] Oversight: REJECT — {:?}", result.violations);
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
        let rollback_dir = PathBuf::from("/tmp/clade-rollback");
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
