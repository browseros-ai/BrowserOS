use std::fs;
use std::process::Command;

fn project_dir() -> String { trios_config::project_dir() }
const MONITOR_LABEL: &str = "com.browseros.clade-monitor";
const DASHBOARD_LABEL: &str = "com.browseros.clade-dashboard";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let action = args.first().map(|s| s.as_str()).unwrap_or("status");

    println!("[CladeLaunchd] Action: {}", action);

    match action {
        "install" => install(),
        "uninstall" => uninstall(),
        _ => status(),
    }
}

fn install() {
    let launch_agents = launch_agents_dir();
    if let Err(e) = fs::create_dir_all(&launch_agents) {
        println!("   [FAIL] Failed to create LaunchAgents dir: {}", e);
        return;
    }
    if let Err(e) = fs::create_dir_all(format!("{}/.trinity/logs", &project_dir())) {
        println!("   [WARN]  Failed to create logs dir: {}", e);
    }

    for (label, bin) in [
        (MONITOR_LABEL, "clade-monitor"),
        (DASHBOARD_LABEL, "clade-dashboard"),
    ] {
        let path = launch_agents.join(format!("{}.plist", label));
        let xml = plist_xml(label, &format!("{}/target/release/{}", &project_dir(), bin), &project_dir());
        if let Err(e) = fs::write(&path, xml) {
            println!("   [FAIL] Failed to write {}: {}", path.display(), e);
            continue;
        }
        match Command::new("launchctl")
            .args(["unload", &path.to_string_lossy()])
            .status()
        {
            Ok(s) if !s.success() => {} // not loaded - expected on first install
            Err(e) => println!("   [WARN]  launchctl unload failed: {}", e),
            _ => {}
        }
        let status = Command::new("launchctl")
            .args(["load", &path.to_string_lossy()])
            .status();
        match status {
            Ok(s) if s.success() => println!("   [OK] Loaded {}", label),
            Ok(s) => println!("   [WARN]  load exit={:?} for {}", s.code(), label),
            Err(e) => println!("   [FAIL] load failed for {}: {}", label, e),
        }
    }
}

fn uninstall() {
    let launch_agents = launch_agents_dir();
    for label in [MONITOR_LABEL, DASHBOARD_LABEL] {
        let path = launch_agents.join(format!("{}.plist", label));
        match Command::new("launchctl")
            .args(["unload", &path.to_string_lossy()])
            .status()
        {
            Ok(s) if !s.success() => println!("   [WARN]  {} was not loaded", label),
            Err(e) => println!("   [WARN]  launchctl unload {}: {}", label, e),
            _ => {}
        }
        if let Err(e) = fs::remove_file(&path) {
            println!("   [WARN]  Failed to remove plist for {}: {}", label, e);
        }
        println!("   [BIN]  Removed {}", label);
    }
}

fn status() {
    for label in [MONITOR_LABEL, DASHBOARD_LABEL] {
        let output = Command::new("launchctl")
            .args(["list", label])
            .output();
        match output {
            Ok(o) if o.status.success() => println!("   [PASS] {}: loaded", label),
            _ => println!("   [REJECT] {}: not loaded", label),
        }
    }
}

fn launch_agents_dir() -> std::path::PathBuf {
    std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join("Library/LaunchAgents"))
        .unwrap_or_else(|_| std::path::PathBuf::from("/Library/LaunchAgents"))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn plist_xml(label: &str, program: &str, working_dir: &str) -> String {
    let label = xml_escape(label);
    let program = xml_escape(program);
    let working_dir = xml_escape(working_dir);
    let proj = xml_escape(&project_dir());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
    </array>
    <key>WorkingDirectory</key>
    <string>{}</string>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>ThrottleInterval</key>
    <integer>60</integer>
    <key>StandardOutPath</key>
    <string>{}/.trinity/logs/{}.stdout.log</string>
    <key>StandardErrorPath</key>
    <string>{}/.trinity/logs/{}.stderr.log</string>
</dict>
</plist>"#,
        label, program, working_dir, proj, label, proj, label
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_contains_label() {
        let xml = plist_xml("com.test.label", "/usr/bin/test", "/tmp");
        assert!(xml.contains("<string>com.test.label</string>"));
    }

    #[test]
    fn plist_contains_program() {
        let xml = plist_xml("com.test.label", "/usr/bin/test", "/tmp");
        assert!(xml.contains("<string>/usr/bin/test</string>"));
    }

    #[test]
    fn plist_contains_working_dir() {
        let xml = plist_xml("com.test.label", "/usr/bin/test", "/tmp/workdir");
        assert!(xml.contains("<string>/tmp/workdir</string>"));
    }

    #[test]
    fn plist_has_keepalive() {
        let xml = plist_xml("x", "y", "z");
        assert!(xml.contains("<key>KeepAlive</key>"));
        assert!(xml.contains("<true/>"));
    }

    #[test]
    fn plist_has_throttle_interval() {
        let xml = plist_xml("x", "y", "z");
        assert!(xml.contains("<integer>60</integer>"));
    }

    #[test]
    fn plist_is_valid_xml_structure() {
        let xml = plist_xml("com.test", "/bin/test", "/tmp");
        assert!(xml.starts_with("<?xml"));
        assert!(xml.contains("<plist version=\"1.0\">"));
        assert!(xml.contains("</plist>"));
    }

    #[test]
    fn xml_escape_ampersand() {
        assert_eq!(xml_escape("test&value"), "test&amp;value");
    }

    #[test]
    fn xml_escape_angle_brackets() {
        assert_eq!(xml_escape("<tag>"), "&lt;tag&gt;");
    }

    #[test]
    fn xml_escape_clean_string_unchanged() {
        assert_eq!(xml_escape("/usr/bin/test"), "/usr/bin/test");
    }

    #[test]
    fn xml_escape_double_quote() {
        // A label/path containing a double quote must not break out of the
        // <string> context in the generated plist.
        assert_eq!(xml_escape("a\"b"), "a&quot;b");
    }

    #[test]
    fn xml_escape_apostrophe() {
        assert_eq!(xml_escape("a'b"), "a&apos;b");
    }

    #[test]
    fn xml_escape_ampersand_before_entities() {
        // Ampersand must be escaped first so already-escaped entities are not
        // double-encoded into &amp;lt; etc.
        assert_eq!(xml_escape("<&>"), "&lt;&amp;&gt;");
    }

    #[test]
    fn plist_escapes_special_chars_in_path() {
        let xml = plist_xml("com.test", "/tmp/test&prog", "/tmp");
        assert!(xml.contains("test&amp;prog"));
        assert!(!xml.contains("test&prog"));
    }
}
