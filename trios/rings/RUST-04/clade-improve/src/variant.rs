use std::path::PathBuf;

/// Each variant is a fully isolated environment (Capability Control boxing via filesystem isolation)
#[derive(Debug, Clone, PartialEq)]
pub enum Variant {
    /// Port 9105 - stable production
    Prod,
    /// Port 9205 - staging for testing
    Staging,
    /// Fixed ephemeral port - one-shot experiment
    Dev,
}

impl Variant {
    pub fn from_env() -> Self {
        match std::env::var("TRIOS_VARIANT").as_deref() {
            Ok("dev") => Self::Dev,
            Ok("staging") => Self::Staging,
            _ => Self::Prod,
        }
    }

    pub fn mcp_port(&self) -> u16 {
        match self {
            Variant::Prod => 9105,
            Variant::Staging => 9205,
            Variant::Dev => 9305, // fixed ephemeral port
        }
    }

    pub fn working_dir(&self) -> PathBuf {
        let root = trios_config::project_dir();
        match self {
            Variant::Prod => PathBuf::from(&root),
            Variant::Staging => PathBuf::from(format!("{}/.worktrees/staging", root)),
            Variant::Dev => PathBuf::from(format!("{}/.trinity/dev", root)),
        }
    }

    pub fn is_production(&self) -> bool {
        matches!(self, Variant::Prod)
    }

    pub fn can_self_modify(&self) -> bool {
        matches!(self, Variant::Dev)
    }
}
