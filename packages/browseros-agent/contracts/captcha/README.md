# CAPTCHA test bed

- `provider-check/`: key validation and provider diagnostics.
- `fixtures/`: public test widgets, a page catalog, and independent server-side verification.
- `e2e/`: MCP runner and local contract stub. Run `bun run test:captcha-e2e -- --pages all`.
- `docker/`: isolated browser/server test environment.

The runner defaults to one temporary CapSolver placeholder key file and clears other inherited provider keys. `--keys real` expands `BROWSEROS_CAPSOLVER_API_KEY_FILE` (default `~/.config/browseros/captcha/capsolver-api-key`) and forwards any other inherited `BROWSEROS_*_API_KEY_FILE` path as-is, without reading key contents. `--keys none` clears all matching key variables. Use `--paid` to require paid paths to succeed. Without it, eligible provider failures remain UNVERIFIED. Only token lengths are saved.

The dev browser runs headed by default; use `--headless` to opt in to headless testing. `--headed` remains an alias for the default.

Run the standard dev stack and the fixtures in one command, from `packages/browseros-agent`:

```sh
bun run test:captcha-e2e -- --dev-watch --pages fixtures --keys placeholder
```

`--dev-watch` runs `bun run dev:claw:watch:new` from this worktree. It uses random ports, a fresh browser profile and `BROWSERCLAW_DIR`, and the same key modes described above. It inherits `CARGO_TARGET_DIR`, defaulting to `/Volumes/extssd/cargo-target/captcha-e2e-dev-watch`. Allow ten minutes for startup, or set `--startup-timeout <milliseconds>` for a cold build.

Before opening pages, it requires healthy, connected browser status and `solve_captcha` in `tools/list`. With `--expect-tool-missing`, it requires the tool to be absent. It refuses the owner's ports (9010, 9011, 9121, 9211–9215). `--mcp-url`, `--headless`, and `--browser-arg` cannot be combined with this mode.

The output directory contains `dev-watch.log`, `results.md`, and `results.json`. In this mode JSON is `{ "preflight": { "url", "port", "toolPresent", "health", "ok" }, "results": [...] }`; other modes keep the results array. Preflight is printed and saved before pages run, including failures. Shutdown and Ctrl+C stop only the spawned process group, check for surviving processes referencing its temporary directories, and remove those directories. A leak fails the run and preserves the directory for diagnosis; Cargo output remains cached.

Use `--mcp-url <url> --fixture-origin <url>` for existing endpoints. That endpoint controls its own credentials and browser. Otherwise set `BROWSEROS_BINARY`; `CLAW_SERVER_BINARY` selects a prebuilt server, and `CARGO_TARGET_DIR` controls Cargo output. Results go to a temporary directory unless `--out` is set.

For a local proof: `bun contracts/captcha/e2e/stub-mcp.ts --scenario contract`, then pass its URL with `--mcp-url` and `--fixture-origin http://fixtures.invalid`.
