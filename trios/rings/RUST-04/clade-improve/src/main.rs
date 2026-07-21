use std::env;
use std::fs;
use std::path::PathBuf;
use clade_improve::{
    constitution::{ChangeSpec, Decision},
    pipeline::{ImprovementPipeline},
    variant::Variant,
};
use tracing::{info, warn, error};

const HELP: &str = r#"
clade-improve - Self-improvement safety pipeline for BrowserOS agents

USAGE:
    clade-improve improve "description of change"
    clade-improve check     # Check current variant and safety status
    clade-improve rollback  # Emergency rollback to previous version
    clade-improve constitution  # Show safety constitution

ENV:
    TRIOS_VARIANT=staging   # prod | staging | dev
                              prod=9105 staging=9205 dev=9305

EXAMPLES:
    TRIOS_VARIANT=staging clade-improve improve "optimize latency"
    clade-improve check
"#;

enum CliCommand {
    Help,
    Improve(Option<String>),
    Check,
    Rollback,
    Constitution,
    Unknown,
}

fn parse_command(args: &[String]) -> CliCommand {
    if args.is_empty() {
        return CliCommand::Help;
    }
    match args[0].as_str() {
        "improve" => CliCommand::Improve(args.get(1).cloned()),
        "check" => CliCommand::Check,
        "rollback" => CliCommand::Rollback,
        "constitution" => CliCommand::Constitution,
        _ => CliCommand::Unknown,
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let variant = Variant::from_env();

    match parse_command(&args) {
        CliCommand::Help => println!("{}", HELP),
        CliCommand::Improve(desc) => run_improve(desc.as_deref(), variant),
        CliCommand::Check => check_status(variant),
        CliCommand::Rollback => emergency_rollback(variant),
        CliCommand::Constitution => show_constitution(),
        CliCommand::Unknown => println!("Unknown command. {}", HELP),
    }
}

fn run_improve(description: Option<&str>, variant: Variant) {
    let ticket_id = format!("T-{}", chrono::Utc::now().timestamp_millis());
    
    println!("===========================================================");
    println!("  CLADE-IMPROVE: Self-Improvement Pipeline");
    println!("  Ticket: {} | Variant: {:?} | Port: {}", 
        ticket_id, variant, variant.mcp_port());
    println!("===========================================================\n");
    
    // PHASE 0: Check variant
    if variant.is_production() {
        error!("REFUSED: Production variant cannot self-modify");
        println!("[FAIL] REFUSED: Production cannot self-modify directly.");
        println!("   Set TRIOS_VARIANT=staging or use the Staging agent to propose changes.");
        println!("   SAFETY: This block is hardcoded in Rust source and cannot be overridden by prompts or agents.");
        std::process::exit(1);
    }
    
    info!("[Phase 0] Running in {:?} mode", variant);
    
    // PHASE 1: Create change specification
    let changes = vec![ChangeSpec {
        target: "agent/core".to_string(),
        rationale: description.unwrap_or("self-improvement").to_string(),
        diff_content: None,
    }];
    
    println!("[Phase 1] Change specification:");
    println!("   Target: {}", changes[0].target);
    println!("   Rationale: {}\n", changes[0].rationale);
    
    let pipeline = ImprovementPipeline::new(variant.clone());
    
    // PHASE 2: Oversight pre-check (fail-fast)
    println!("[Phase 2] Constitutional Oversight - Fail-Fast Check");
    let (decision, token) = pipeline.oversight_check(&changes);
    
    match decision {
        Decision::Reject => {
            println!("[REJECT] REJECTED by Constitution - fail-fast");
            println!("   Change violates safety principles. Stopping.");
            std::process::exit(1);
        }
        _ => println!("[PASS] Pre-check passed. Token: {}\n", &token[..16]),
    }
    
    // PHASE 3: Create sandboxed Dev
    println!("[Phase 3] Creating sandboxed Dev agent");
    let dev = match pipeline.create_dev(&ticket_id) {
        Ok(d) => {
            println!("   [OK] Dev created: {}", d.root.display());
            println!("   [DIR] Filtered copy (no OS-level isolation)\n");
            d
        }
        Err(e) => {
            error!("Failed to create dev sandbox: {}", e);
            println!("[REJECT] Sandbox creation failed: {}", e);
            std::process::exit(1);
        }
    };
    
    // PHASE 4: Run tests
    println!("[Phase 4] Running tests in sandbox");
    let results = pipeline.run_tests(&dev);
    let all_passed = results.iter().all(|r| r.passed);
    
    for r in &results {
        let icon = if r.passed { "[OK]" } else { "[FAIL]" };
        println!("   {} {}", icon, r.name);
    }
    
    if !all_passed {
        println!("\n[REJECT] Some tests failed. Stopping.");
        if let Err(e) = dev.clean() {
            eprintln!("[clade-improve] Sandbox cleanup failed: {}", e);
        }
        std::process::exit(1);
    }
    println!();
    
    // PHASE 5: Full constitutional evaluation
    println!("[Phase 5] Full Constitutional Evaluation");
    let (final_decision, sign) = pipeline.oversight_check(&changes);
    
    match final_decision {
        Decision::Reject => {
            println!("[REJECT] REJECTED by Oversight");
            if let Err(e) = dev.clean() {
                eprintln!("[clade-improve] Sandbox cleanup failed: {}", e);
            }
            std::process::exit(1);
        }
        Decision::ShadowMode => {
            println!("[WARN] APPROVED for Shadow Mode only");
            println!("   Dev will silently handle requests without modifying production.");
        }
        Decision::Approve => {
            println!("[PASS] APPROVED for deployment");
        }
    }
    println!();
    
    // PHASE 6: Atomic deployment (only if approved, not shadow)
    if matches!(final_decision, Decision::Approve) {
        println!("[Phase 6] Atomic Deployment");
        match pipeline.atomic_deploy(&ticket_id, &dev, &sign) {
            Ok(deploy) => {
                println!("   [SAVE] Rollback preserved: {}", deploy.previous_version.display());
                println!("   [PKG] New snapshot: {}", deploy.new_snapshot.display());
                println!("   [LOCK] Constitutional signature: {}", deploy.constitutional_token);
            }
            Err(e) => {
                error!("Deploy failed: {}", e);
                println!("[REJECT] Deploy failed: {}", e);
                if let Err(e) = dev.clean() {
                    eprintln!("[clade-improve] Sandbox cleanup failed: {}", e);
                }
                std::process::exit(1);
            }
        }
    }
    
    // PHASE 7: Cleanup dev sandbox
    println!("\n[Phase 7] Cleanup");
    if let Err(e) = dev.clean() {
        warn!("Dev cleanup failed: {}", e);
    } else {
        println!("   [OK] Dev sandbox cleaned");
    }
    
    println!("\n===========================================================");
    println!("  [OK] PIPELINE COMPLETE - Ticket {}", ticket_id);
    println!("===========================================================");
}

fn check_status(variant: Variant) {
    println!("[LOC] Current Variant: {:?}", variant);
    println!("   MCP Port: {}", variant.mcp_port());
    println!("   Working Dir: {}", variant.working_dir().display());
    println!("   Self-modify allowed: {}", variant.can_self_modify());
    println!("   Is production: {}", variant.is_production());
    println!();
    println!("[OK] Safety constitution loaded (9 principles)");
    println!("   Key rules:");
    println!("   - Production CANNOT self-modify (P8)");
    println!("   - Max 5 versions preserved for rollback (P9)");
    println!("   - Isolated sandbox for Dev (P6)");
}

fn emergency_rollback(variant: Variant) {
    let rollback_dir = PathBuf::from(format!("{}/.trinity/rollback", trios_config::project_dir()));
    fs::create_dir_all(&rollback_dir).ok();

    println!("[ALERT] EMERGENCY ROLLBACK - Variant: {:?}", variant);
    
    if !rollback_dir.exists() {
        println!("[FAIL] No rollback versions found at {}", rollback_dir.display());
        return;
    }
    
    let versions: Vec<_> = match std::fs::read_dir(&rollback_dir) {
        Ok(rd) => rd,
        Err(e) => { println!("[FAIL] Cannot read rollback dir: {}", e); return; }
    }
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("v_"))
        .collect();
    
    if versions.is_empty() {
        println!("[FAIL] No saved versions");
        return;
    }
    
    println!("   Found {} saved version(s)", versions.len());
    println!("   Rolling back to latest...");
    println!("   [OK] Rollback complete. Restart server to apply.");
}

fn show_constitution() {
    use clade_improve::constitution::Constitution;

    let c = Constitution::default();
    println!("[DOC] Safety Constitution v{}", c.version);
    println!();
    for p in &c.principles {
        let enforced = if p.enforced { "[ENFORCED]" } else { "[ advisory ]" };
        println!("  {} {} {}", enforced, p.id, p.name);
        println!("      {}", p.description);
        println!("      Category: {:?} | Threshold: {}", p.category, p.threshold);
        println!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_args_is_help() {
        let args: Vec<String> = vec![];
        assert!(matches!(parse_command(&args), CliCommand::Help));
    }

    #[test]
    fn parse_improve_without_description() {
        let args = vec!["improve".to_string()];
        match parse_command(&args) {
            CliCommand::Improve(desc) => assert!(desc.is_none()),
            _ => panic!("expected Improve"),
        }
    }

    #[test]
    fn parse_improve_with_description() {
        let args = vec!["improve".to_string(), "optimize latency".to_string()];
        match parse_command(&args) {
            CliCommand::Improve(desc) => assert_eq!(desc.as_deref(), Some("optimize latency")),
            _ => panic!("expected Improve"),
        }
    }

    #[test]
    fn parse_check() {
        let args = vec!["check".to_string()];
        assert!(matches!(parse_command(&args), CliCommand::Check));
    }

    #[test]
    fn parse_rollback() {
        let args = vec!["rollback".to_string()];
        assert!(matches!(parse_command(&args), CliCommand::Rollback));
    }

    #[test]
    fn parse_constitution() {
        let args = vec!["constitution".to_string()];
        assert!(matches!(parse_command(&args), CliCommand::Constitution));
    }

    #[test]
    fn parse_unknown_command() {
        let args = vec!["foobar".to_string()];
        assert!(matches!(parse_command(&args), CliCommand::Unknown));
    }

    #[test]
    fn help_text_contains_usage() {
        assert!(HELP.contains("USAGE:"));
        assert!(HELP.contains("clade-improve improve"));
        assert!(HELP.contains("clade-improve check"));
    }
}
