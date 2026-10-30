//! The name this computer starts with in the computers list: its own name, so people recognise
//! it. It is sent once, when the install registers, and never logged.

use std::time::Duration;
use tokio::{process::Command, sync::OnceCell};

/// The billing gateway's limit, which it counts in UTF-16 units.
const MAX_LABEL_UNITS: usize = 40;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);

static LABEL: OnceCell<Option<String>> = OnceCell::const_new();

/// Worked out once per process.
pub async fn computer_label() -> Option<String> {
    LABEL.get_or_init(work_out).await.clone()
}

async fn work_out() -> Option<String> {
    let own = match std::env::consts::OS {
        "macos" => run("scutil", &["--get", "ComputerName"]).await,
        "windows" => std::env::var("COMPUTERNAME").ok(),
        _ => linux_hostname().await,
    };
    match own.as_deref().and_then(clean) {
        Some(label) => Some(label),
        None => clean(&hardware_label().await),
    }
}

/// Trimmed and cut to the gateway's limit; `None` when nothing is left.
#[must_use]
pub fn clean(raw: &str) -> Option<String> {
    let mut label = String::new();
    let mut units = 0;
    for character in raw.trim().chars() {
        units += character.len_utf16();
        if units > MAX_LABEL_UNITS {
            break;
        }
        label.push(character);
    }
    let label = label.trim_end().to_string();
    (!label.is_empty()).then_some(label)
}

async fn hardware_label() -> String {
    match std::env::consts::OS {
        "macos" => mac_model_name(
            run("sysctl", &["-n", "hw.model"])
                .await
                .as_deref()
                .unwrap_or_default(),
        )
        .to_string(),
        "windows" => "Windows PC".to_string(),
        _ => "Linux computer".to_string(),
    }
}

/// `hw.model` names a Mac's family before its version, such as `MacBookAir10,1`. Newer Macs
/// report only `Mac14,2`, which says nothing more than "Mac".
#[must_use]
pub fn mac_model_name(model: &str) -> &'static str {
    const FAMILIES: &[(&str, &str)] = &[
        ("MacBookAir", "MacBook Air"),
        ("MacBookPro", "MacBook Pro"),
        ("MacBook", "MacBook"),
        ("Macmini", "Mac mini"),
        ("MacPro", "Mac Pro"),
        ("iMac", "iMac"),
    ];
    FAMILIES
        .iter()
        .find(|(prefix, _)| model.trim().starts_with(prefix))
        .map_or("Mac", |(_, name)| name)
}

async fn linux_hostname() -> Option<String> {
    for path in ["/proc/sys/kernel/hostname", "/etc/hostname"] {
        if let Ok(name) = tokio::fs::read_to_string(path).await
            && !name.trim().is_empty()
        {
            return Some(name);
        }
    }
    None
}

async fn run(program: &str, args: &[&str]) -> Option<String> {
    let mut command = Command::new(program);
    command.args(args).kill_on_drop(true);
    let output = tokio::time::timeout(COMMAND_TIMEOUT, command.output())
        .await
        .ok()?
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_trimmed_and_capped_in_utf16_units() {
        assert_eq!(clean("  Sam’s MacBook Air \n").as_deref(), Some("Sam’s MacBook Air"));
        assert_eq!(clean("   "), None);
        let long = "a".repeat(60);
        assert_eq!(clean(&long).map(|label| label.len()), Some(40));
        // Each of these takes two UTF-16 units, so only 20 fit.
        let wide = "😀".repeat(30);
        assert_eq!(clean(&wide).map(|label| label.chars().count()), Some(20));
    }

    #[test]
    fn mac_models_name_their_family() {
        assert_eq!(mac_model_name("MacBookAir10,1"), "MacBook Air");
        assert_eq!(mac_model_name("MacBookPro18,3"), "MacBook Pro");
        assert_eq!(mac_model_name("Macmini9,1"), "Mac mini");
        assert_eq!(mac_model_name("iMac21,1"), "iMac");
        assert_eq!(mac_model_name("Mac14,2"), "Mac");
        assert_eq!(mac_model_name(""), "Mac");
    }
}
