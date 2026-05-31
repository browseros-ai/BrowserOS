use std::fs;
use std::process::Command;

fn project_dir() -> String { std::env::var("TRIOS_ROOT").unwrap_or_else(|_| "/Users/playra/BrowserOS-full/trios".to_string()) }
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
        println!("   ❌ Failed to create LaunchAgents dir: {}", e);
        return;
    }
    if let Err(e) = fs::create_dir_all(format!("{}/.trinity/logs", &project_dir())) {
        println!("   ⚠️  Failed to create logs dir: {}", e);
    }

    for (label, bin) in [
        (MONITOR_LABEL, "clade-monitor"),
        (DASHBOARD_LABEL, "clade-dashboard"),
    ] {
        let path = launch_agents.join(format!("{}.plist", label));
        let xml = plist_xml(label, &format!("{}/target/release/{}", &project_dir(), bin), &project_dir());
        if let Err(e) = fs::write(&path, xml) {
            println!("   ❌ Failed to write {}: {}", path.display(), e);
            continue;
        }
        match Command::new("launchctl")
            .args(["unload", &path.to_string_lossy()])
            .status()
        {
            Ok(s) if !s.success() => {} // not loaded — expected on first install
            Err(e) => println!("   ⚠️  launchctl unload failed: {}", e),
            _ => {}
        }
        let status = Command::new("launchctl")
            .args(["load", &path.to_string_lossy()])
            .status();
        match status {
            Ok(s) if s.success() => println!("   ✅ Loaded {}", label),
            Ok(s) => println!("   ⚠️  load exit={:?} for {}", s.code(), label),
            Err(e) => println!("   ❌ load failed for {}: {}", label, e),
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
            Ok(s) if !s.success() => println!("   ⚠️  {} was not loaded", label),
            Err(e) => println!("   ⚠️  launchctl unload {}: {}", label, e),
            _ => {}
        }
        if let Err(e) = fs::remove_file(&path) {
            println!("   ⚠️  Failed to remove plist for {}: {}", label, e);
        }
        println!("   🗑  Removed {}", label);
    }
}

fn status() {
    for label in [MONITOR_LABEL, DASHBOARD_LABEL] {
        let output = Command::new("launchctl")
            .args(["list", label])
            .output();
        match output {
            Ok(o) if o.status.success() => println!("   🟢 {}: loaded", label),
            _ => println!("   🔴 {}: not loaded", label),
        }
    }
}

fn launch_agents_dir() -> std::path::PathBuf {
    std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join("Library/LaunchAgents"))
        .unwrap_or_else(|_| std::path::PathBuf::from("/Library/LaunchAgents"))
}

fn plist_xml(label: &str, program: &str, working_dir: &str) -> String {
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
        label, program, working_dir, &project_dir(), label, &project_dir(), label
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
}
