//! Launch resolution matrix (port of the TS launcher tests): bundled Bun with the node shim and
//! native PATH, host npx fallback, custom commands with the login-shell PATH, and command splitting.

#![cfg(unix)]

use std::{collections::BTreeMap, os::unix::fs::PermissionsExt, path::Path};

use acp_agents::{
    AgentKind, CLAUDE_ADAPTER, CODEX_ADAPTER, CustomAgentConfig, HostEnvironment, LaunchError,
    LaunchSource, resolve_launch, split_command_line,
};
use tempfile::TempDir;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn host(dir: &TempDir, resources: Option<&Path>) -> HostEnvironment {
    HostEnvironment {
        resources_dir: resources.map(Path::to_path_buf),
        browseros_dir: dir.path().join("browseros"),
        inherited_env: BTreeMap::from([
            ("PATH".into(), "/usr/bin:/bin".into()),
            // A shell that cannot run keeps custom-launch tests off the developer's profile.
            ("SHELL".into(), "/nonexistent/shell".into()),
        ]),
    }
}

fn write_executable(path: &Path, body: &str, mode: u32) -> TestResult {
    std::fs::create_dir_all(path.parent().ok_or("no parent")?)?;
    std::fs::write(path, body)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[test]
fn bundled_bun_runs_the_adapter_with_shim_native_path_and_cache() -> TestResult {
    let dir = TempDir::new()?;
    let resources = dir.path().join("resources");
    let third_party = resources.join("bin/third_party");
    let bun = third_party.join("bun");
    write_executable(&bun, "#!/bin/sh\n", 0o755)?;
    let host = host(&dir, Some(&resources));
    let cwd = dir.path().join("work");
    let extra = BTreeMap::from([("CODEX_CONFIG".to_owned(), "{}".to_owned())]);

    let launch = resolve_launch(AgentKind::Codex, None, &extra, &cwd, &host)?;

    assert_eq!(launch.source, LaunchSource::BundledBun);
    assert_eq!(launch.spec.program, bun);
    assert_eq!(
        launch.spec.args,
        [
            "x",
            "--bun",
            "--silent",
            "--package",
            CODEX_ADAPTER.package_spec,
            CODEX_ADAPTER.bin
        ]
    );
    assert_eq!(launch.spec.cwd, cwd);
    let shim_dir = host.browseros_dir.join("cache/acp-node-shim");
    // The native dir is prepended last, so it leads even though bun's own dir came after the shim.
    assert_eq!(
        launch.spec.env.get("PATH"),
        Some(&format!(
            "{}:{}:/usr/bin:/bin",
            third_party.display(),
            shim_dir.display()
        ))
    );
    assert_eq!(
        launch.spec.env.get("BUN_INSTALL_CACHE_DIR"),
        Some(
            &host
                .browseros_dir
                .join("cache/bun-install")
                .display()
                .to_string()
        )
    );
    assert_eq!(
        launch.spec.env.get("CODEX_CONFIG").map(String::as_str),
        Some("{}")
    );
    let shim = shim_dir.join("node");
    assert_eq!(
        std::fs::read_to_string(&shim)?,
        format!("#!/bin/sh\nexec '{}' \"$@\"\n", bun.display())
    );
    assert_eq!(
        std::fs::metadata(&shim)?.permissions().mode() & 0o777,
        0o755
    );
    Ok(())
}

#[test]
fn non_executable_bun_falls_back_to_npx_with_the_native_dir_on_path() -> TestResult {
    let dir = TempDir::new()?;
    let resources = dir.path().join("resources");
    let third_party = resources.join("bin/third_party");
    write_executable(&third_party.join("bun"), "", 0o644)?;
    let host = host(&dir, Some(&resources));

    let launch = resolve_launch(AgentKind::Claude, None, &BTreeMap::new(), dir.path(), &host)?;

    assert_eq!(launch.source, LaunchSource::HostNpx);
    assert_eq!(launch.spec.program, Path::new("npx"));
    assert_eq!(launch.spec.args, ["-y", CLAUDE_ADAPTER.package_spec]);
    assert_eq!(
        launch.spec.env,
        BTreeMap::from([(
            "PATH".to_owned(),
            format!("{}:/usr/bin:/bin", third_party.display())
        )])
    );
    Ok(())
}

#[test]
fn npx_without_resources_inherits_path_and_adds_only_spawn_env() -> TestResult {
    let dir = TempDir::new()?;
    let host = host(&dir, None);
    let extra = BTreeMap::from([(
        "INITIAL_AGENT_MODE".to_owned(),
        "agent-full-access".to_owned(),
    )]);

    let launch = resolve_launch(AgentKind::Codex, None, &extra, dir.path(), &host)?;

    assert_eq!(launch.source, LaunchSource::HostNpx);
    assert_eq!(launch.spec.args, ["-y", CODEX_ADAPTER.package_spec]);
    assert_eq!(launch.spec.env, extra);
    Ok(())
}

#[test]
fn custom_command_is_split_and_gets_the_login_shell_path() -> TestResult {
    let dir = TempDir::new()?;
    let shell = dir.path().join("login-shell");
    write_executable(
        &shell,
        "#!/bin/sh\nprintf '%s' '/login/bin:/usr/bin'\n",
        0o755,
    )?;
    let mut host = host(&dir, None);
    host.inherited_env
        .insert("SHELL".into(), shell.display().to_string());
    let custom = CustomAgentConfig {
        command: "  'my agent' --flag \"two words\" ".into(),
        ..CustomAgentConfig::default()
    };
    let env = BTreeMap::from([
        ("TOKEN".to_owned(), "x".to_owned()),
        ("PATH".to_owned(), "/ignored".to_owned()),
    ]);

    let launch = resolve_launch(AgentKind::Custom, Some(&custom), &env, dir.path(), &host)?;

    assert_eq!(launch.source, LaunchSource::Custom);
    assert_eq!(launch.spec.program, Path::new("my agent"));
    assert_eq!(launch.spec.args, ["--flag", "two words"]);
    assert_eq!(launch.spec.env.get("TOKEN").map(String::as_str), Some("x"));
    // TS quirk kept: the merged login PATH replaces a PATH from the custom env.
    assert_eq!(
        launch.spec.env.get("PATH").map(String::as_str),
        Some("/login/bin:/usr/bin:/bin")
    );
    Ok(())
}

#[test]
fn custom_without_a_usable_login_shell_keeps_its_env() -> TestResult {
    let dir = TempDir::new()?;
    let host = host(&dir, None);
    let custom = CustomAgentConfig {
        command: "agent acp".into(),
        ..CustomAgentConfig::default()
    };
    let env = BTreeMap::from([("PATH".to_owned(), "/custom".to_owned())]);
    let launch = resolve_launch(AgentKind::Custom, Some(&custom), &env, dir.path(), &host)?;
    assert_eq!(launch.spec.env, env);
    Ok(())
}

#[test]
fn a_custom_command_wins_over_a_built_in_kind() -> TestResult {
    let dir = TempDir::new()?;
    let custom = CustomAgentConfig {
        command: "probe-me --acp".into(),
        ..CustomAgentConfig::default()
    };
    let launch = resolve_launch(
        AgentKind::Claude,
        Some(&custom),
        &BTreeMap::new(),
        dir.path(),
        &host(&dir, None),
    )?;
    assert_eq!(launch.source, LaunchSource::Custom);
    assert_eq!(launch.spec.program, Path::new("probe-me"));
    Ok(())
}

#[test]
fn custom_launch_errors_use_the_ts_messages() -> TestResult {
    let dir = TempDir::new()?;
    let host = host(&dir, None);
    let none = resolve_launch(AgentKind::Custom, None, &BTreeMap::new(), dir.path(), &host);
    assert_eq!(none, Err(LaunchError::MissingCommand));
    let blank = CustomAgentConfig {
        command: "   ".into(),
        ..CustomAgentConfig::default()
    };
    let error = resolve_launch(
        AgentKind::Custom,
        Some(&blank),
        &BTreeMap::new(),
        dir.path(),
        &host,
    );
    assert_eq!(error, Err(LaunchError::MissingCommand));
    assert_eq!(
        LaunchError::MissingCommand.to_string(),
        "Custom ACP agent is missing a launch command"
    );
    let quote = CustomAgentConfig {
        command: "agent 'oops".into(),
        ..CustomAgentConfig::default()
    };
    let error = resolve_launch(
        AgentKind::Custom,
        Some(&quote),
        &BTreeMap::new(),
        dir.path(),
        &host,
    );
    assert_eq!(error, Err(LaunchError::UnterminatedQuote));
    assert_eq!(
        LaunchError::UnterminatedQuote.to_string(),
        "Command has an unterminated quote"
    );
    Ok(())
}

#[test]
fn split_command_line_matches_parse_command_ts() -> TestResult {
    let cases: [(&str, &[&str]); 8] = [
        ("agent acp", &["agent", "acp"]),
        ("  agent\t--x\r\n--y  ", &["agent", "--x", "--y"]),
        ("'a b' \"c d\"", &["a b", "c d"]),
        ("pre'fix suf'fix", &["prefix suffix"]),
        (
            r"C:\Users\me\agent.exe --dir C:\tmp",
            &[r"C:\Users\me\agent.exe", "--dir", r"C:\tmp"],
        ),
        ("say \"it's\" 'a \"b\"'", &["say", "it's", "a \"b\""]),
        ("empty ''", &["empty", ""]),
        ("", &[]),
    ];
    for (input, expected) in cases {
        assert_eq!(split_command_line(input)?, expected, "input: {input:?}");
    }
    assert_eq!(
        split_command_line("a \"b"),
        Err(LaunchError::UnterminatedQuote)
    );
    Ok(())
}
