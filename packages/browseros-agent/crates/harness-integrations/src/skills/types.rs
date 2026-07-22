use std::{collections::BTreeMap, path::PathBuf};

use crate::Error;

/// Product-owned skill content and the directory name used by each harness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillSpec {
    pub(crate) name: String,
    pub(crate) content: String,
}

impl SkillSpec {
    pub fn new(name: impl Into<String>, content: impl Into<String>) -> Result<Self, Error> {
        let name = name.into();
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(Error::InvalidSkillSpec {
                reason: "name must contain only lowercase ASCII letters, digits, and hyphens"
                    .to_string(),
            });
        }
        Ok(Self {
            name,
            content: content.into(),
        })
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetPlatform {
    Darwin,
    Linux,
    Windows,
}

impl TargetPlatform {
    #[must_use]
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Darwin
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

/// Explicit path inputs for portable, deterministic global-root resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillEnvironment {
    pub home_dir: PathBuf,
    pub platform: TargetPlatform,
    variables: BTreeMap<String, PathBuf>,
}

impl SkillEnvironment {
    pub fn new(home_dir: impl Into<PathBuf>, platform: TargetPlatform) -> Self {
        Self {
            home_dir: home_dir.into(),
            platform,
            variables: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn current(home_dir: impl Into<PathBuf>) -> Self {
        let mut environment = Self::new(home_dir, TargetPlatform::current());
        for variable in ["CLAUDE_CONFIG_DIR", "XDG_CONFIG_HOME"] {
            if let Some(value) = std::env::var_os(variable).filter(|value| !value.is_empty()) {
                environment
                    .variables
                    .insert(variable.to_string(), PathBuf::from(value));
            }
        }
        environment
    }

    #[must_use]
    pub fn with_variable(mut self, name: impl Into<String>, value: impl Into<PathBuf>) -> Self {
        self.variables.insert(name.into(), value.into());
        self
    }

    pub(crate) fn variable(&self, name: &str) -> Option<PathBuf> {
        match name {
            "HOME" | "USERPROFILE" => Some(self.home_dir.clone()),
            _ => self.variables.get(name).cloned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillWarning {
    pub target: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillReconcileOutcome {
    pub installed: usize,
    pub updated: usize,
    pub removed: usize,
    pub unchanged: usize,
    pub warnings: Vec<SkillWarning>,
}
