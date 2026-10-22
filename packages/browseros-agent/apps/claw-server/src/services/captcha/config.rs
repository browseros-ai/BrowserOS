use std::fmt;

pub const CAPSOLVER_KEY_FILE_ENV: &str = "BROWSEROS_CAPSOLVER_API_KEY_FILE";
pub const CAPMONSTER_KEY_FILE_ENV: &str = "BROWSEROS_CAPMONSTER_API_KEY_FILE";
pub const CAPSOLVER_URL_ENV: &str = "BROWSEROS_CAPSOLVER_API_URL";
pub const CAPMONSTER_URL_ENV: &str = "BROWSEROS_CAPMONSTER_API_URL";
pub const PAID_TASK_LIMIT_ENV: &str = "BROWSEROS_CAPTCHA_PAID_TASK_LIMIT";

#[derive(Clone)]
pub struct ApiKey(String);

impl ApiKey {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(<redacted>)")
    }
}

#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub key: ApiKey,
    pub base_url: url::Url,
}

#[derive(Debug, Clone, Default)]
pub struct CaptchaConfig {
    pub capsolver: Option<ProviderConfig>,
    pub capmonster: Option<ProviderConfig>,
    pub paid_task_limit: u32,
}

impl CaptchaConfig {
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        Self {
            capsolver: provider(
                &lookup,
                CAPSOLVER_KEY_FILE_ENV,
                CAPSOLVER_URL_ENV,
                "https://api.capsolver.com",
            ),
            capmonster: provider(
                &lookup,
                CAPMONSTER_KEY_FILE_ENV,
                CAPMONSTER_URL_ENV,
                "https://api.capmonster.cloud",
            ),
            paid_task_limit: lookup(PAID_TASK_LIMIT_ENV)
                .and_then(|value| value.parse().ok())
                .unwrap_or(100),
        }
    }

    pub fn from_env() -> Self {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    pub fn has_provider(&self) -> bool {
        self.capsolver.is_some() || self.capmonster.is_some()
    }
}

fn provider(
    lookup: &impl Fn(&str) -> Option<String>,
    key_env: &'static str,
    url_env: &'static str,
    default_url: &str,
) -> Option<ProviderConfig> {
    let path = lookup(key_env)?;
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(_) => {
            tracing::warn!(target: "captcha", variable = key_env, "CAPTCHA key file could not be read; provider disabled");
            return None;
        }
    };
    let key = contents.trim();
    if key.is_empty() {
        tracing::warn!(target: "captcha", variable = key_env, "CAPTCHA key file is empty; provider disabled");
        return None;
    }
    let configured_url = lookup(url_env);
    let base_url = match url::Url::parse(configured_url.as_deref().unwrap_or(default_url)) {
        Ok(url) if matches!(url.scheme(), "http" | "https") && url.host_str().is_some() => url,
        _ => {
            tracing::warn!(target: "captcha", variable = url_env, "CAPTCHA provider URL is invalid; provider disabled");
            return None;
        }
    };
    Some(ProviderConfig {
        key: ApiKey(key.to_owned()),
        base_url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn from_vars(vars: &[(&str, String)]) -> CaptchaConfig {
        CaptchaConfig::from_lookup(|name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.clone())
        })
    }

    #[test]
    fn tests_that_unset_keys_disable_providers_and_default_the_limit() {
        let config = from_vars(&[]);
        assert!(!config.has_provider());
        assert_eq!(config.paid_task_limit, 100);
    }

    #[test]
    fn tests_that_key_files_are_trimmed_and_debug_is_redacted() -> anyhow::Result<()> {
        let file = tempfile::NamedTempFile::new()?;
        std::fs::write(file.path(), " \n unique-test-key \t\n")?;
        let config = from_vars(&[
            (
                CAPSOLVER_KEY_FILE_ENV,
                file.path().to_string_lossy().into_owned(),
            ),
            (
                CAPMONSTER_KEY_FILE_ENV,
                file.path().to_string_lossy().into_owned(),
            ),
        ]);
        assert!(config.has_provider());
        let capsolver = config
            .capsolver
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing CapSolver"))?;
        let capmonster = config
            .capmonster
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing CapMonster"))?;
        assert_eq!(capsolver.key.expose(), "unique-test-key");
        assert_eq!(capmonster.key.expose(), "unique-test-key");
        assert_eq!(format!("{:?}", capsolver.key), "ApiKey(<redacted>)");
        assert!(!format!("{config:?}").contains("unique-test-key"));
        assert_eq!(capsolver.base_url.as_str(), "https://api.capsolver.com/");
        assert_eq!(
            capmonster.base_url.as_str(),
            "https://api.capmonster.cloud/"
        );
        Ok(())
    }

    #[test]
    fn tests_that_empty_and_missing_files_disable_providers() -> anyhow::Result<()> {
        let file = tempfile::NamedTempFile::new()?;
        std::fs::write(file.path(), " \n\t")?;
        let dir = tempfile::tempdir()?;
        let config = from_vars(&[
            (
                CAPSOLVER_KEY_FILE_ENV,
                file.path().to_string_lossy().into_owned(),
            ),
            (
                CAPMONSTER_KEY_FILE_ENV,
                dir.path().join("missing").to_string_lossy().into_owned(),
            ),
        ]);
        assert!(!config.has_provider());
        Ok(())
    }

    #[test]
    fn tests_that_url_and_limit_overrides_are_parsed() -> anyhow::Result<()> {
        let file = tempfile::NamedTempFile::new()?;
        std::fs::write(file.path(), "test-key")?;
        let config = from_vars(&[
            (
                CAPSOLVER_KEY_FILE_ENV,
                file.path().to_string_lossy().into_owned(),
            ),
            (CAPSOLVER_URL_ENV, "http://127.0.0.1:9876/mock/".into()),
            (
                CAPMONSTER_KEY_FILE_ENV,
                file.path().to_string_lossy().into_owned(),
            ),
            (CAPMONSTER_URL_ENV, "http://127.0.0.1:9877/".into()),
            (PAID_TASK_LIMIT_ENV, "7".into()),
        ]);
        assert_eq!(
            config.capsolver.map(|p| p.base_url.to_string()),
            Some("http://127.0.0.1:9876/mock/".into())
        );
        assert_eq!(
            config.capmonster.map(|p| p.base_url.to_string()),
            Some("http://127.0.0.1:9877/".into())
        );
        assert_eq!(config.paid_task_limit, 7);
        assert_eq!(
            from_vars(&[(PAID_TASK_LIMIT_ENV, "invalid".into())]).paid_task_limit,
            100
        );
        assert_eq!(
            from_vars(&[(PAID_TASK_LIMIT_ENV, "0".into())]).paid_task_limit,
            0
        );
        Ok(())
    }

    #[test]
    fn tests_that_invalid_url_overrides_disable_the_provider() -> anyhow::Result<()> {
        let file = tempfile::NamedTempFile::new()?;
        std::fs::write(file.path(), "test-key")?;
        let config = from_vars(&[
            (
                CAPSOLVER_KEY_FILE_ENV,
                file.path().to_string_lossy().into_owned(),
            ),
            (CAPSOLVER_URL_ENV, "not a url".into()),
        ]);
        assert!(!config.has_provider());
        Ok(())
    }
}
