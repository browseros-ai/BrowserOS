---
name: panic-hardening
description: Convert Rust panic surfaces (unwrap/expect) in daemon code into Result propagation and clippy-clean error paths.
argument-hint: [crate-path]
---

# Panic Hardening Skill (trios Rust rings)

Convert daemon production code from `unwrap`/`expect` panic surfaces to `Result` propagation, satisfying `clippy::unwrap_used` and `clippy::expect_used` at `deny` level while keeping tests ergonomic.

## When to Invoke

- A `cargo clippy --workspace --all-targets --all-features` run reports `unwrap_used`/`expect_used` in production code.
- A ring that runs as a daemon (e.g. `clade-*`, `trios-meshd`) contains `expect("...")` on I/O, crypto, or config parsing.
- Before sealing a wave that touches Rust rings.

## Pattern Library

### 1. Add a domain-internal error variant

For "should never happen" primitive failures (HKDF expand, cipher init, key parsing):

```rust
#[derive(Debug, PartialEq, Eq)]
pub enum MeshError {
    Auth,
    // ... existing variants ...
    /// Internal crypto primitive failed on an input that should be valid.
    /// Treat as auth-equivalent failure; do not crash the daemon.
    CryptoInternal,
}
```

Name it `<Domain>Internal` so callers know it is not an attacker-controlled failure.

### 2. Centralize infallible-looking operations in fallible helpers

```rust
fn hkdf_expand_32(hk: &Hkdf<Sha256>, info: &[u8], out: &mut [u8; 32]) -> Result<(), MeshError> {
    hk.expand(info, out).map_err(|_| MeshError::CryptoInternal)
}
```

- One helper per invariant operation.
- Return `Result<T, DomainError>`; never `expect` inside the helper.

### 3. Propagate `Result` through the public API

Before:

```rust
fn combine_dh_shares(...) -> [u8; 32] { ... hk.expand(...).expect("...") }
```

After:

```rust
fn combine_dh_shares(...) -> Result<[u8; 32], MeshError> { ... hkdf_expand_32(&hk, ..., &mut out)?; Ok(out) }
```

Cascading callers (`Session::from_shared`, `NoiseXX::complete_initiator`, `StaticKey::session_with`, `Handshake::complete`) must also return `Result`.

### 4. Safe byte extraction instead of `try_into().expect()`

```rust
fn read_u32_be(bytes: &[u8]) -> Option<u32> {
    bytes.get(..4)?.try_into().ok().map(u32::from_be_bytes)
}
fn read_u64_be(bytes: &[u8]) -> Option<u64> {
    bytes.get(..8)?.try_into().ok().map(u64::from_be_bytes)
}
```

Use `let Some(value) = read_u32_be(buf) else { return Err(MeshError::ShortFrame); };`.

### 5. AEAD operations

```rust
self.cipher.encrypt(nonce, payload).map_err(|_| MeshError::Auth)?;
```

Encryption failure is almost always a nonce reuse bug - treat as auth failure and return, never panic.

### 6. Mutex poison recovery in daemon hot paths

```rust
let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
```

Use this when a daemon thread must survive a panicking peer thread. In tests, `.expect("mutex poison")` is acceptable under the test exemption (see below).

### 7. Config parsing in binaries

Convert `parse_cfg` from panicking to `Result<Cfg, String>` with line numbers:

```rust
fn parse_cfg(path: &Path) -> Result<Cfg, String> {
    let s = std::fs::read_to_string(path)
        .map_err(|e| format!("{}: read error: {}", path.display(), e))?;
    let mut cfg = Cfg::default();
    for (n, line) in s.lines().enumerate() {
        // ...
        if id.is_none() { return Err(format!("{}:{}: missing id", path.display(), n + 1)); }
    }
    Ok(cfg)
}
```

`main` prints the error and exits 1:

```rust
fn main() {
    if let Err(e) = run() {
        eprintln!("trios-meshd: {}", e);
        std::process::exit(1);
    }
}
```

### 8. Workspace lint setup

In workspace `Cargo.toml`:

```toml
[workspace.lints.clippy]
unwrap_used = "deny"
expect_used = "deny"
```

In each affected crate's `Cargo.toml`:

```toml
[lints]
workspace = true
```

In the crate root, allow tests only:

```rust
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
```

This keeps CI strict while letting tests stay concise.

## Verification Checklist

- [ ] `cargo clippy -p <crate> --all-targets --all-features` reports zero `unwrap_used`/`expect_used` violations.
- [ ] `cargo test -p <crate> --all-features` passes.
- [ ] `./build.sh` passes (Swift app still links).
- [ ] Changed source files are ASCII-only (`grep -RIn '[^\x00-\x7F]' <paths>`).
- [ ] Binary startup paths (config, bind, drop file) return errors instead of panicking.

## Backlog Extensions

- `tmp-zero`: replace remaining test `/tmp` paths with `tempfile` or project-relative dirs.
- `seal-automation`: add a `clade-seal` ring that gates promotion on clippy + test + ASCII.
- `promotion-lock`: prevent concurrent `clade-promote` runs across rings.
