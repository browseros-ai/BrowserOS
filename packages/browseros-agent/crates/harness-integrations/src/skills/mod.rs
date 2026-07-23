mod manifest;
mod reconciler;
mod types;

pub use reconciler::{SkillReconciler, resolve_agent_skill_target};
pub use types::{SkillEnvironment, SkillReconcileOutcome, SkillSpec, SkillWarning, TargetPlatform};
