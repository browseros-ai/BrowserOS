use std::fmt;

pub const CAPSOLVER_KEY_FILE_ENV: &str = "BROWSEROS_CAPSOLVER_API_KEY_FILE";
pub const TWOCAPTCHA_KEY_FILE_ENV: &str = "BROWSEROS_2CAPTCHA_API_KEY_FILE";
pub const TWOCAPTCHA_URL_ENV: &str = "BROWSEROS_2CAPTCHA_API_URL";
pub const CAPSOLVER_URL_ENV: &str = "BROWSEROS_CAPSOLVER_API_URL";
pub const PAID_TASK_LIMIT_ENV: &str = "BROWSEROS_CAPTCHA_PAID_TASK_LIMIT";
pub const TEST_FORCE_METHOD_ENV: &str = "BROWSEROS_CAPTCHA_TEST_FORCE_METHOD";

/// Test-only: skip free paths. Vendors reject localhost websiteURLs, so use public pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForcedMethod {
    Token,
}

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

impl fmt::Display for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub key: ApiKey,
    pub base_url: url::Url,
}

#[derive(Debug, Clone)]
pub struct CaptchaConfig {
    pub capsolver: Option<ProviderConfig>,
    pub twocaptcha: Option<ProviderConfig>,
    pub paid_task_limit: u32,
    pub force_method: Option<ForcedMethod>,
}

impl Default for CaptchaConfig {
    fn default() -> Self {
        Self {
            capsolver: None,
            twocaptcha: None,
            paid_task_limit: 100,
            force_method: None,
        }
    }
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
            twocaptcha: provider(
                &lookup,
                TWOCAPTCHA_KEY_FILE_ENV,
                TWOCAPTCHA_URL_ENV,
                "https://api.2captcha.com",
            ),
            paid_task_limit: lookup(PAID_TASK_LIMIT_ENV)
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or(100),
            force_method: lookup(TEST_FORCE_METHOD_ENV).and_then(|value| {
                if value.trim() == "token" {
                    tracing::warn!(target: "captcha", "test-only: CAPTCHA free paths are skipped");
                    Some(ForcedMethod::Token)
                } else {
                    tracing::warn!(target: "captcha", variable = TEST_FORCE_METHOD_ENV, "Unsupported CAPTCHA test force method ignored; only token is supported");
                    None
                }
            }),
        }
    }

    pub fn from_env() -> Self {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    pub fn has_provider(&self) -> bool {
        self.capsolver.is_some() || self.twocaptcha.is_some()
    }
}

fn provider(
    lookup: &impl Fn(&str) -> Option<String>,
    key_env: &'static str,
    url_env: &'static str,
    default_url: &str,
) -> Option<ProviderConfig> {
    let path = lookup(key_env)?;
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
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
    let configured_url = configured_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty());
    let base_url = match url::Url::parse(configured_url.unwrap_or(default_url)) {
        Ok(url)
            if matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none() =>
        {
            url
        }
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

    #[derive(Clone, Default)]
    struct LogWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for LogWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn config_and_logs(vars: &[(&str, String)]) -> (CaptchaConfig, String) {
        let writer = LogWriter::default();
        let output = writer.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::WARN)
            .with_writer(move || writer.clone())
            .finish();
        let config = tracing::subscriber::with_default(subscriber, || from_vars(vars));
        let bytes = output.0.lock().unwrap_or_else(|p| p.into_inner());
        (config, String::from_utf8_lossy(&bytes).into_owned())
    }

    #[test]
    fn tests_that_force_method_is_unset_by_default_without_a_warning() {
        let (config, logs) = config_and_logs(&[]);
        assert_eq!(config.force_method, None);
        assert_eq!(CaptchaConfig::default().force_method, None);
        assert!(logs.is_empty(), "{logs}");
    }

    #[test]
    fn tests_that_token_force_method_warns_once_at_startup() {
        for value in ["token", " token \n"] {
            let (config, logs) =
                config_and_logs(&[("BROWSEROS_CAPTCHA_TEST_FORCE_METHOD", value.into())]);
            assert_eq!(config.force_method, Some(ForcedMethod::Token));
            assert!(!config.has_provider());
            assert_eq!(logs.lines().count(), 1, "{logs}");
            assert!(logs.contains("WARN"), "{logs}");
            assert!(
                logs.contains("test-only: CAPTCHA free paths are skipped"),
                "{logs}"
            );
        }
    }

    #[test]
    fn tests_that_unsupported_force_methods_are_ignored_with_a_warning() {
        for value in ["tiles", "junk", "TOKEN", "", " \n"] {
            let (config, logs) =
                config_and_logs(&[("BROWSEROS_CAPTCHA_TEST_FORCE_METHOD", value.into())]);
            assert_eq!(config.force_method, None);
            assert_eq!(logs.lines().count(), 1, "{logs}");
            assert!(logs.contains("WARN"), "{logs}");
            assert!(
                logs.contains("BROWSEROS_CAPTCHA_TEST_FORCE_METHOD"),
                "{logs}"
            );
            assert!(logs.contains("ignored"), "{logs}");
            assert!(!logs.contains("free paths are skipped"), "{logs}");
        }
    }

    fn from_vars(vars: &[(&str, String)]) -> CaptchaConfig {
        CaptchaConfig::from_lookup(|name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.clone())
        })
    }
    #[test]
    fn tests_that_unset_keys_disable_the_provider_and_default_the_limit() {
        let config = from_vars(&[]);
        assert!(!config.has_provider());
        assert_eq!(config.paid_task_limit, 100);
        assert_eq!(CaptchaConfig::default().paid_task_limit, 100);
    }
    #[test]
    fn tests_that_key_files_are_trimmed_and_formats_are_redacted() -> anyhow::Result<()> {
        let file = tempfile::NamedTempFile::new()?;
        std::fs::write(file.path(), " \n unique-test-key \t\n")?;
        let config = from_vars(&[(
            CAPSOLVER_KEY_FILE_ENV,
            file.path().to_string_lossy().into_owned(),
        )]);
        assert!(config.has_provider());
        let provider = config
            .capsolver
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing provider"))?;
        assert_eq!(provider.key.expose(), "unique-test-key");
        assert_eq!(format!("{:?}", provider.key), "ApiKey(<redacted>)");
        assert_eq!(format!("{}", provider.key), "<redacted>");
        assert!(!format!("{config:?}").contains("unique-test-key"));
        assert_eq!(provider.base_url.as_str(), "https://api.capsolver.com/");
        Ok(())
    }
    #[test]
    fn tests_that_empty_and_missing_files_disable_the_provider() -> anyhow::Result<()> {
        let file = tempfile::NamedTempFile::new()?;
        std::fs::write(file.path(), " \n\t")?;
        let dir = tempfile::tempdir()?;
        for path in [file.path(), dir.path()] {
            assert!(
                !from_vars(&[(CAPSOLVER_KEY_FILE_ENV, path.to_string_lossy().into_owned())])
                    .has_provider()
            );
        }
        Ok(())
    }
    #[test]
    fn tests_that_blank_settings_are_unset_and_numbers_are_trimmed() -> anyhow::Result<()> {
        let file = tempfile::NamedTempFile::new()?;
        std::fs::write(file.path(), "test-key")?;
        let config = from_vars(&[
            (
                CAPSOLVER_KEY_FILE_ENV,
                format!(" {} \n", file.path().display()),
            ),
            (CAPSOLVER_URL_ENV, " \n".into()),
            (PAID_TASK_LIMIT_ENV, " 7 \n".into()),
        ]);
        assert_eq!(config.paid_task_limit, 7);
        assert_eq!(
            config.capsolver.map(|p| p.base_url.to_string()),
            Some("https://api.capsolver.com/".into())
        );
        assert!(!from_vars(&[(CAPSOLVER_KEY_FILE_ENV, " \n".into())]).has_provider());
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
            (CAPSOLVER_URL_ENV, " http://127.0.0.1:9876/mock/ ".into()),
            (PAID_TASK_LIMIT_ENV, "7".into()),
        ]);
        assert_eq!(
            config.capsolver.map(|p| p.base_url.to_string()),
            Some("http://127.0.0.1:9876/mock/".into())
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
        for url in [
            "not a url",
            "https://user:secret@example.test/",
            "https://example.test/?key=secret",
        ] {
            assert!(
                !from_vars(&[
                    (
                        CAPSOLVER_KEY_FILE_ENV,
                        file.path().to_string_lossy().into_owned()
                    ),
                    (CAPSOLVER_URL_ENV, url.into())
                ])
                .has_provider()
            );
        }
        Ok(())
    }
}
