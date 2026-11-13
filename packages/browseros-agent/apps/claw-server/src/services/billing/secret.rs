use std::fmt;

/// A credential held in memory: the install secret, or a pasted licence key on its way to the
/// billing gateway. Its `Debug` and `Display` never show the value, so it cannot reach a log
/// line or an error message by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(value)
    }

    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[cfg(test)]
mod tests {
    use super::Secret;

    #[test]
    fn formatting_never_shows_the_value() {
        let secret = Secret::new("NEO-7K2QM-AAAAA-BBBBB-CCCCC-DDDDD".to_string());
        assert!(!format!("{secret}").contains("7K2QM"));
        assert!(!format!("{secret:?}").contains("7K2QM"));
        assert!(!format!("{:?}", Some(&secret)).contains("7K2QM"));
        assert_eq!(secret.expose(), "NEO-7K2QM-AAAAA-BBBBB-CCCCC-DDDDD");
    }
}
