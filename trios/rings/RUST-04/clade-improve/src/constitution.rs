use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Constitution {
    pub version: String,
    pub principles: Vec<Principle>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Principle {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: PrincipleCategory,
    pub threshold: f64,
    pub enforced: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PrincipleCategory {
    Accuracy,
    Safety,
    Privacy,
    Autonomy,
    Performance,
    Isolation,
    Explainability,
    NoSelfModify,
    PreserveRollback,
}

impl Default for Constitution {
    fn default() -> Self {
        Self {
            version: "1.0.0".to_string(),
            principles: vec![
                Principle {
                    id: "P1".to_string(),
                    name: "Accuracy Preservation".to_string(),
                    description: "Must not degrade accuracy on known queries".to_string(),
                    category: PrincipleCategory::Accuracy,
                    threshold: 0.95,
                    enforced: true,
                },
                Principle {
                    id: "P2".to_string(),
                    name: "Safety Invariants".to_string(),
                    description: "Must not bypass safety constraints".to_string(),
                    category: PrincipleCategory::Safety,
                    threshold: 1.0,
                    enforced: true,
                },
                Principle {
                    id: "P3".to_string(),
                    name: "Privacy Protection".to_string(),
                    description: "Must not expose system prompts or API keys".to_string(),
                    category: PrincipleCategory::Privacy,
                    threshold: 1.0,
                    enforced: true,
                },
                Principle {
                    id: "P4".to_string(),
                    name: "Autonomy Boundaries".to_string(),
                    description: "Must not act without user request".to_string(),
                    category: PrincipleCategory::Autonomy,
                    threshold: 1.0,
                    enforced: true,
                },
                Principle {
                    id: "P5".to_string(),
                    name: "Performance Bounds".to_string(),
                    description: "Latency must not exceed +20%".to_string(),
                    category: PrincipleCategory::Performance,
                    threshold: 1.2,
                    enforced: true,
                },
                Principle {
                    id: "P6".to_string(),
                    name: "Network Isolation".to_string(),
                    description: "No external network access in Dev sandbox".to_string(),
                    category: PrincipleCategory::Isolation,
                    threshold: 1.0,
                    enforced: true,
                },
                Principle {
                    id: "P7".to_string(),
                    name: "Explainability".to_string(),
                    description: "Changes must be explainable (chain-of-thought)".to_string(),
                    category: PrincipleCategory::Explainability,
                    threshold: 1.0,
                    enforced: false,
                },
                Principle {
                    id: "P8".to_string(),
                    name: "No Self-Modification in Production".to_string(),
                    description: "Production agent must not modify its own code".to_string(),
                    category: PrincipleCategory::NoSelfModify,
                    threshold: 1.0,
                    enforced: true,
                },
                Principle {
                    id: "P9".to_string(),
                    name: "Rollback Preservation".to_string(),
                    description: "Must keep at least N=5 recent working versions".to_string(),
                    category: PrincipleCategory::PreserveRollback,
                    threshold: 5.0,
                    enforced: true,
                },
            ],
        }
    }
}

impl Constitution {
    pub fn evaluate(&self, changes: &[ChangeSpec]) -> OversightResult {
        let mut violations = vec![];
        let mut evidence_parts = vec![];

        for p in &self.principles {
            if !p.enforced { continue; }

            let passed = match p.category {
                PrincipleCategory::Accuracy => {
                    let ok = changes.iter().all(|c| !c.degrades_accuracy());
                    if !ok { evidence_parts.push("accuracy-sensitive files modified".to_string()); }
                    ok
                }
                PrincipleCategory::Safety => {
                    let ok = changes.iter().all(|c| !c.bypasses_safety());
                    if !ok { evidence_parts.push("safety-critical guard removed or weakened".to_string()); }
                    ok
                }
                PrincipleCategory::Privacy => {
                    let ok = changes.iter().all(|c| !c.exposes_secrets());
                    if !ok { evidence_parts.push("secrets or system prompts exposed".to_string()); }
                    ok
                }
                PrincipleCategory::Autonomy => {
                    let ok = changes.iter().all(|c| !c.removes_consent_gate());
                    if !ok { evidence_parts.push("user consent gate removed".to_string()); }
                    ok
                }
                PrincipleCategory::Performance => {
                    let ok = changes.iter().all(|c| !c.adds_blocking_operation());
                    if !ok { evidence_parts.push("blocking operation added to hot path".to_string()); }
                    ok
                }
                PrincipleCategory::Isolation => {
                    let ok = changes.iter().all(|c| !c.adds_network_access());
                    if !ok { evidence_parts.push("network access added to sandbox".to_string()); }
                    ok
                }
                PrincipleCategory::Explainability => true,
                PrincipleCategory::NoSelfModify => {
                    let ok = changes.iter().all(|c| !c.targets_production_code());
                    if !ok { evidence_parts.push("production code self-modification attempted".to_string()); }
                    ok
                }
                PrincipleCategory::PreserveRollback => {
                    let ok = changes.iter().all(|c| c.preserve_rollback());
                    if !ok { evidence_parts.push("rollback history deletion attempted".to_string()); }
                    ok
                }
            };

            if !passed {
                violations.push(p.clone());
            }
        }

        let all_passed = violations.is_empty();
        let evidence = if evidence_parts.is_empty() {
            format!("Checked {} principles — all passed", self.principles.len())
        } else {
            format!("Checked {} principles — {} violations: {}",
                self.principles.len(), violations.len(), evidence_parts.join("; "))
        };

        OversightResult {
            decision: if all_passed { Decision::Approve } else { Decision::Reject },
            violations,
            evidence,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Decision {
    Approve,
    Reject,
    ShadowMode,
}

#[derive(Debug, Clone)]
pub struct OversightResult {
    pub decision: Decision,
    pub violations: Vec<Principle>,
    pub evidence: String,
}

#[derive(Debug, Clone)]
pub struct ChangeSpec {
    pub target: String,
    pub rationale: String,
    pub diff_content: Option<String>,
}

impl ChangeSpec {
    pub fn targets_production_code(&self) -> bool {
        self.target.starts_with("/prod/") || self.target.contains("production")
    }

    pub fn preserve_rollback(&self) -> bool {
        !self.target.contains("delete_history") && !self.target.contains("remove_snapshot")
    }

    pub fn degrades_accuracy(&self) -> bool {
        let sensitive = ["prompt", "model", "temperature", "system_message", "constitution"];
        let t = self.target.to_lowercase();
        sensitive.iter().any(|s| t.contains(s))
            && self.diff_content.as_ref().is_some_and(|d| d.contains('-') && !d.contains('+'))
    }

    pub fn bypasses_safety(&self) -> bool {
        let guards = ["safety_budget", "oversight", "constitution", "guard", "allowlist", "recursion"];
        let t = self.target.to_lowercase();
        if !guards.iter().any(|g| t.contains(g)) { return false; }
        self.diff_content.as_ref().is_some_and(|d| {
            d.contains("- ") && (d.contains("check") || d.contains("validate") || d.contains("guard"))
        })
    }

    pub fn exposes_secrets(&self) -> bool {
        self.diff_content.as_ref().is_some_and(|d| {
            let d_lower = d.to_lowercase();
            d_lower.contains("api_key") || d_lower.contains("secret") || d_lower.contains("password")
                || d_lower.contains("token=") || d_lower.contains("sk-")
        })
    }

    pub fn removes_consent_gate(&self) -> bool {
        self.diff_content.as_ref().is_some_and(|d| {
            d.contains("- ") && (d.contains("confirm") || d.contains("user_approve") || d.contains("consent"))
        })
    }

    pub fn adds_blocking_operation(&self) -> bool {
        self.diff_content.as_ref().is_some_and(|d| {
            d.contains("+ ") && (d.contains("sleep(") || d.contains("thread::sleep") || d.contains("Thread.sleep"))
        })
    }

    pub fn adds_network_access(&self) -> bool {
        let t = self.target.to_lowercase();
        if !t.contains("sandbox") && !t.contains("dev") { return false; }
        self.diff_content.as_ref().is_some_and(|d| {
            d.contains("+ ") && (d.contains("reqwest") || d.contains("curl") || d.contains("URLSession")
                || d.contains("http://") || d.contains("https://"))
        })
    }
}

#[cfg(test)]
// Tests legitimately use expect()/unwrap() for fixtures and invariants; the
// workspace deny/warn policy targets production code paths, not test setup.
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn safe_change() -> ChangeSpec {
        ChangeSpec {
            target: "agent/core".to_string(),
            rationale: "improvement".to_string(),
            diff_content: None,
        }
    }

    #[test]
    fn default_constitution_has_9_principles() {
        let c = Constitution::default();
        assert_eq!(c.principles.len(), 9);
        assert_eq!(c.version, "1.0.0");
    }

    #[test]
    fn safe_change_passes_all_principles() {
        let c = Constitution::default();
        let result = c.evaluate(&[safe_change()]);
        assert!(matches!(result.decision, Decision::Approve));
        assert!(result.violations.is_empty());
    }

    #[test]
    fn production_self_modify_rejected() {
        let c = Constitution::default();
        let change = ChangeSpec {
            target: "/prod/main.rs".to_string(),
            rationale: "self-modify".to_string(),
            diff_content: None,
        };
        let result = c.evaluate(&[change]);
        assert!(matches!(result.decision, Decision::Reject));
        assert!(result.violations.iter().any(|v| v.category == PrincipleCategory::NoSelfModify));
    }

    #[test]
    fn rollback_deletion_rejected() {
        let c = Constitution::default();
        let change = ChangeSpec {
            target: "delete_history/snapshots".to_string(),
            rationale: "cleanup".to_string(),
            diff_content: None,
        };
        let result = c.evaluate(&[change]);
        assert!(matches!(result.decision, Decision::Reject));
        assert!(result.violations.iter().any(|v| v.category == PrincipleCategory::PreserveRollback));
    }

    #[test]
    fn accuracy_degradation_detected() {
        let change = ChangeSpec {
            target: "model/prompt_template".to_string(),
            rationale: "simplify".to_string(),
            diff_content: Some("- detailed instructions\n- careful handling".to_string()),
        };
        assert!(change.degrades_accuracy());
    }

    #[test]
    fn accuracy_improvement_passes() {
        let change = ChangeSpec {
            target: "model/prompt_template".to_string(),
            rationale: "improve".to_string(),
            diff_content: Some("+ detailed instructions\n+ careful handling".to_string()),
        };
        assert!(!change.degrades_accuracy());
    }

    #[test]
    fn safety_bypass_detected() {
        let change = ChangeSpec {
            target: "oversight/guard.rs".to_string(),
            rationale: "simplify".to_string(),
            diff_content: Some("- if !check_safety() { return; }".to_string()),
        };
        assert!(change.bypasses_safety());
    }

    #[test]
    fn secret_exposure_detected() {
        let change = ChangeSpec {
            target: "config.rs".to_string(),
            rationale: "add config".to_string(),
            diff_content: Some("let key = \"sk-abc123\"".to_string()),
        };
        assert!(change.exposes_secrets());
    }

    #[test]
    fn consent_removal_detected() {
        let change = ChangeSpec {
            target: "ui/dialog.swift".to_string(),
            rationale: "streamline".to_string(),
            diff_content: Some("- if user_approve() { proceed() }".to_string()),
        };
        assert!(change.removes_consent_gate());
    }

    #[test]
    fn blocking_operation_detected() {
        let change = ChangeSpec {
            target: "handler.rs".to_string(),
            rationale: "add delay".to_string(),
            diff_content: Some("+ thread::sleep(Duration::from_secs(10))".to_string()),
        };
        assert!(change.adds_blocking_operation());
    }

    #[test]
    fn network_access_in_sandbox_detected() {
        let change = ChangeSpec {
            target: "sandbox/dev/fetch.rs".to_string(),
            rationale: "add API call".to_string(),
            diff_content: Some("+ let resp = reqwest::get(url)".to_string()),
        };
        assert!(change.adds_network_access());
    }

    #[test]
    fn network_access_outside_sandbox_allowed() {
        let change = ChangeSpec {
            target: "prod/api.rs".to_string(),
            rationale: "add API call".to_string(),
            diff_content: Some("+ let resp = reqwest::get(url)".to_string()),
        };
        assert!(!change.adds_network_access());
    }

    #[test]
    fn unenforced_principle_skipped() {
        let c = Constitution::default();
        let p7 = c.principles.iter().find(|p| p.id == "P7").expect("P7 exists");
        assert!(!p7.enforced);
        assert_eq!(p7.category, PrincipleCategory::Explainability);
    }

    #[test]
    fn evidence_describes_violations() {
        let c = Constitution::default();
        let change = ChangeSpec {
            target: "/prod/main.rs".to_string(),
            rationale: "hack".to_string(),
            diff_content: None,
        };
        let result = c.evaluate(&[change]);
        assert!(result.evidence.contains("violations"));
        assert!(result.evidence.contains("production code self-modification"));
    }
}
