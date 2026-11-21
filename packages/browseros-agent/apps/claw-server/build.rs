fn main() {
    println!("cargo:rerun-if-env-changed=CLAW_PLAYWRIGHT_RUN");
    let value = match std::env::var("CLAW_PLAYWRIGHT_RUN") {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => String::new(),
        Err(error) => panic!("CLAW_PLAYWRIGHT_RUN must be \"0\" or \"1\": {error}"),
    };
    let marker = match value.as_str() {
        "1" => "1",
        "0" => "0",
        raw if raw.trim().is_empty() => "0",
        raw => panic!("CLAW_PLAYWRIGHT_RUN must be \"0\" or \"1\", got {raw:?}"),
    };
    // Release tooling reads the baked selection from the binary before publishing it.
    println!("cargo:rustc-env=CLAW_PLAYWRIGHT_RUN_MARKER=browseros-claw-playwright-run={marker};");

    println!("cargo:rerun-if-env-changed=CLAW_POSTHOG_KEY");
    let Some(key) = std::env::var_os("CLAW_POSTHOG_KEY").and_then(|value| value.into_string().ok())
    else {
        return;
    };
    let mut encoded = String::with_capacity(key.len() * 2);
    let hex = b"0123456789abcdef";
    for byte in key.bytes() {
        encoded.push(char::from(hex[usize::from(byte >> 4)]));
        encoded.push(char::from(hex[usize::from(byte & 0x0f)]));
    }
    println!("cargo:rustc-env=CLAW_POSTHOG_KEY_MARKER=browseros-claw-posthog-key={encoded};");
}
