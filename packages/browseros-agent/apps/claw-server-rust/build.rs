use std::fmt::Write;

fn main() {
    println!("cargo:rerun-if-env-changed=CLAW_POSTHOG_KEY");
    let Some(key) = std::env::var_os("CLAW_POSTHOG_KEY").and_then(|value| value.into_string().ok())
    else {
        return;
    };
    let mut encoded = String::with_capacity(key.len() * 2);
    for byte in key.as_bytes() {
        write!(&mut encoded, "{byte:02x}").expect("writing to a string cannot fail");
    }
    println!("cargo:rustc-env=CLAW_POSTHOG_KEY_MARKER=browseros-claw-posthog-key={encoded};");
}
