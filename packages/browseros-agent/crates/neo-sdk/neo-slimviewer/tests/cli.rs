use std::process::Command;

#[test]
fn tests_that_help_and_version_need_no_display() {
    for (arg, expected) in [
        ("--help", "--max-viewers"),
        ("--version", "slimviewer 0.1.0"),
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_slimviewer"))
            .arg(arg)
            .env_remove("DISPLAY")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{arg}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(String::from_utf8_lossy(&out.stdout).contains(expected));
    }
}

#[test]
fn tests_that_bad_options_fail_before_connecting() {
    for (arg, value) in [
        ("--fps", "0"),
        ("--fps", "NaN"),
        ("--fps", "inf"),
        ("--fps", "-1"),
        ("--crf", "NaN"),
        ("--crf-range", "-1"),
        ("--sharpen-qp", "52"),
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_slimviewer"))
            .args([arg, value])
            .output()
            .unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success());
        assert!(err.contains(arg), "{arg}={value}: {err}");
        assert!(!err.contains("panicked"), "{err}");
        assert_eq!(err.lines().count(), 1, "{err}");
    }
}

#[cfg(feature = "openh264")]
#[test]
fn tests_that_openh264_rejects_x264_presets_before_connecting() {
    let out = Command::new(env!("CARGO_BIN_EXE_slimviewer"))
        .args(["--encoder", "openh264", "--preset", "ultrafast"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("--preset is only available with x264"),
        "{err}"
    );
}

#[test]
fn tests_that_help_reports_the_compiled_default_encoder() {
    let out = Command::new(env!("CARGO_BIN_EXE_slimviewer"))
        .arg("--help")
        .output()
        .unwrap();
    let expected = if cfg!(feature = "openh264") {
        "Default encoder: openh264"
    } else {
        "Default encoder: x264"
    };
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains(expected));
}

#[cfg(not(feature = "x264"))]
#[test]
fn tests_that_openh264_only_build_refuses_x264_before_connecting() {
    let out = Command::new(env!("CARGO_BIN_EXE_slimviewer"))
        .args(["--encoder", "x264"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("encoder x264 is not built in"));
}

#[test]
fn tests_that_sigterm_exits_successfully_during_blocked_startup() {
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_slimviewer"))
        .args(["--rfb", &listener.local_addr().unwrap().to_string()])
        .spawn()
        .unwrap();
    let start = Instant::now();
    let _connection = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if start.elapsed() > Duration::from_secs(5) {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("process did not reach RFB startup");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("accept: {e}"),
        }
    };
    unsafe {
        assert_eq!(libc::kill(child.id() as i32, libc::SIGTERM), 0);
    }
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert_eq!(status.code(), Some(0));
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("SIGTERM did not stop process within one second");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
