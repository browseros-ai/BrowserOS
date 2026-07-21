use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn project_dir() -> String {
    std::env::var("TRIOS_ROOT").unwrap_or_else(|_| {
        match std::env::current_dir() {
            Ok(p) => p.to_string_lossy().into_owned(),
            Err(e) => {
                eprintln!("[FAIL] TRIOS_ROOT not set and current_dir unavailable: {}", e);
                std::process::exit(1);
            }
        }
    })
}

struct Variant {
    name: &'static str,
    output: PathBuf,
    app_bundle: PathBuf,
    bundle_id: &'static str,
    mcp_port: &'static str,
    a2a_port: &'static str,
    build_root: PathBuf,
}

fn main() {
    let variant_name = env::var("TRIOS_VARIANT").unwrap_or_else(|_| "prod".into());
    let variant = resolve_variant(&variant_name);

    println!(
        "[CladeBuild] Variant={}, output={}",
        variant.name,
        variant.output.display()
    );

    // Collect Swift files
    let mut swift_files = vec![variant.build_root.join("main.swift")];
    collect_swift_files(&variant.build_root.join("rings"), &mut swift_files);
    collect_swift_files(&variant.build_root.join("BR-OUTPUT"), &mut swift_files);

    let file_count = swift_files.len();

    // Build
    let mut cmd = Command::new("swiftc");
    cmd.arg("-O")
        .arg("-o")
        .arg(&variant.output)
        .arg("-framework")
        .arg("SwiftUI")
        .arg("-framework")
        .arg("AppKit")
        .arg("-framework")
        .arg("WebKit")
        .arg("-framework")
        .arg("Combine");

    for f in &swift_files {
        cmd.arg(f);
    }

    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let output = match cmd.output() {
        Ok(o) => o,
        Err(e) => { eprintln!("[FAIL] swiftc failed to start: {}", e); std::process::exit(1); }
    };
    let log_path = format!("{}/.trinity/logs/clade-build_{}.log", project_dir(), variant.name);
    if let Err(e) = fs::write(&log_path, &output.stderr) {
        eprintln!("[build] Failed to write build log {}: {}", log_path, e);
    }

    if !output.status.success() {
        eprintln!("[FAIL] Build failed for variant={}", variant.name);
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        std::process::exit(1);
    }

    println!("[OK] Build successful: {}", variant.output.display());
    if let Err(e) = fs::set_permissions(&variant.output, std::fs::Permissions::from_mode(0o755)) {
        eprintln!("[CladeBuild] Failed to set binary permissions: {}", e);
    }

    // Ensure .app bundle structure
    let macos = variant.app_bundle.join("Contents/MacOS");
    let resources = variant.app_bundle.join("Contents/Resources");
    if let Err(e) = fs::create_dir_all(&macos) {
        eprintln!("[CladeBuild] Failed to create MacOS dir: {}", e);
    }
    if let Err(e) = fs::create_dir_all(&resources) {
        eprintln!("[CladeBuild] Failed to create Resources dir: {}", e);
    }

    // Generate Info.plist
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>trios</string>
    <key>CFBundleIdentifier</key><string>{}</string>
    <key>CFBundleName</key><string>Trios {}</string>
    <key>CFBundleVersion</key><string>1.0.0</string>
    <key>CFBundleShortVersionString</key><string>1.0.0</string>
    <key>LSMinimumSystemVersion</key><string>14.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>TRIOS_VARIANT</key><string>{}</string>
    <key>TRIOS_MCP_PORT</key><string>{}</string>
    <key>TRIOS_A2A_PORT</key><string>{}</string>
</dict>
</plist>"#,
        variant.bundle_id,
        capitalize(variant.name),
        variant.name,
        variant.mcp_port,
        variant.a2a_port
    );
    if let Err(e) = fs::write(variant.app_bundle.join("Contents/Info.plist"), plist) {
        eprintln!("[FAIL] Failed to write Info.plist: {}", e);
        std::process::exit(1);
    }

    println!(
        "[OK] Copied to .app bundle: {} (files={}, ports MCP={} A2A={})",
        variant.app_bundle.display(),
        file_count,
        variant.mcp_port,
        variant.a2a_port
    );
}

fn resolve_variant(name: &str) -> Variant {
    if name == "staging" {
        Variant {
            name: "staging",
            output: PathBuf::from(format!("{}/.worktrees/staging/trios_app", &project_dir())),
            app_bundle: PathBuf::from(format!("{}/.worktrees/staging/trios-staging.app", &project_dir())),
            bundle_id: "com.browseros.trios.staging",
            mcp_port: "9205",
            a2a_port: "9300",
            build_root: PathBuf::from(format!("{}/.worktrees/staging/trios", &project_dir())),
        }
    } else {
        Variant {
            name: "prod",
            output: PathBuf::from(format!("{}/trios_app", &project_dir())),
            app_bundle: PathBuf::from(format!("{}/trios.app", &project_dir())),
            bundle_id: "com.browseros.trios",
            mcp_port: "9105",
            a2a_port: "9200",
            build_root: PathBuf::from(&project_dir()),
        }
    }
}

fn collect_swift_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            collect_swift_files(&path, out);
        } else if path.extension().map(|e| e == "swift").unwrap_or(false) {
            out.push(path);
        }
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capitalize_empty() {
        assert_eq!(capitalize(""), "");
    }

    #[test]
    fn capitalize_single_char() {
        assert_eq!(capitalize("a"), "A");
    }

    #[test]
    fn capitalize_word() {
        assert_eq!(capitalize("staging"), "Staging");
    }

    #[test]
    fn capitalize_already_upper() {
        assert_eq!(capitalize("Prod"), "Prod");
    }

    #[test]
    fn resolve_variant_prod() {
        let v = resolve_variant("prod");
        assert_eq!(v.name, "prod");
        assert_eq!(v.mcp_port, "9105");
        assert_eq!(v.a2a_port, "9200");
        assert_eq!(v.bundle_id, "com.browseros.trios");
    }

    #[test]
    fn resolve_variant_staging() {
        let v = resolve_variant("staging");
        assert_eq!(v.name, "staging");
        assert_eq!(v.mcp_port, "9205");
        assert_eq!(v.a2a_port, "9300");
        assert_eq!(v.bundle_id, "com.browseros.trios.staging");
    }

    #[test]
    fn resolve_variant_unknown_defaults_to_prod() {
        let v = resolve_variant("anything");
        assert_eq!(v.name, "prod");
    }
}
