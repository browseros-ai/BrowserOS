//! Sidecar and executable protocol compatibility; process tests own only temporary state and ports.
use serde_json::{Value, json};
use server_rust::{
    VERSION,
    config::{CliAction, Config, EnvConfig, parse_cli},
};
use std::{ffi::OsString, process::Command};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn executable_version_and_general_errors() -> Result {
    let output = Command::new(env!("CARGO_BIN_EXE_browseros-server-rs"))
        .arg("--version")
        .output()?;
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout)?.trim(), VERSION);
    assert!(output.stderr.is_empty());
    for args in [
        vec![],
        vec!["--config"],
        vec!["--config", "/nonexistent/browseros-config.json"],
        vec!["--unknown"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_browseros-server-rs"))
            .args(args)
            .output()?;
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8(output.stderr)?.contains("[FATAL]"));
    }
    assert_eq!(
        parse_cli([OsString::from("--config=")]),
        Err(server_rust::config::CliError::ConfigPathMissing)
    );
    assert_eq!(
        parse_cli([OsString::from("--version")]),
        Ok(CliAction::Version)
    );
    Ok(())
}

#[test]
fn sidecar_normalization_and_required_validation() -> Result {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("config.json");
    let env = EnvConfig {
        home: Some(dir.path().to_path_buf()),
        node_env: Some("development".into()),
        ..Default::default()
    };
    let valid = json!({"ports":{"server":" 09200 ","cdp":9100,"proxy":9000},"directories":{"resources":" ./resources/../resources ","execution":" ./logs "},"flags":{"allow_remote_in_mcp":true},"replay":{"retentionDays":7},"extra":true});
    std::fs::write(&path, valid.to_string())?;
    let config = Config::load(&path, &env)?;
    assert_eq!(config.server_port, 9200);
    assert_eq!(config.proxy_port, Some(9000));
    assert_eq!(config.resources_dir, dir.path().join("resources"));
    assert_eq!(config.browseros_dir, dir.path().join(".browseros-dev"));
    assert!(config.allow_remote_in_mcp);
    for invalid in [
        json!(0),
        json!(65536),
        json!(1.5),
        json!("+9200"),
        json!("9e3"),
        json!(null),
        json!(true),
    ] {
        let mut value = valid.clone();
        value["ports"]["server"] = invalid;
        std::fs::write(&path, value.to_string())?;
        assert!(Config::load(&path, &env).is_err(), "{value}");
    }
    for invalid in [json!(" "), json!(null), json!(7)] {
        let mut value = valid.clone();
        value["directories"]["resources"] = invalid;
        std::fs::write(&path, value.to_string())?;
        assert!(Config::load(&path, &env).is_err(), "{value}");
    }
    std::fs::write(&path, Value::Object(Default::default()).to_string())?;
    assert!(Config::load(&path, &env).is_err());
    Ok(())
}
