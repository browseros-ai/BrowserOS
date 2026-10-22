# CAPTCHA test bed

- `provider-check/`: key validation and provider diagnostics.
- `fixtures/`: public test widgets, a page catalog, and independent server-side verification.
- `e2e/`: MCP runner and local contract stub. Run `bun run test:captcha-e2e -- --pages all`.
- `docker/`: isolated browser/server test environment.

The runner defaults to temporary placeholder key files. `--keys real` forwards key-file paths without reading their contents; `--keys none` clears them. Use `--paid` to require paid paths to succeed. Without it, eligible provider failures remain UNVERIFIED. Only token lengths are saved.

The dev browser runs headed by default; use `--headless` to opt in to headless testing. `--headed` remains an alias for the default.

Use `--mcp-url <url> --fixture-origin <url>` for existing endpoints. That endpoint controls its own credentials and browser. Otherwise set `BROWSEROS_BINARY`; `CLAW_SERVER_BINARY` selects a prebuilt server, and `CARGO_TARGET_DIR` controls Cargo output. Results go to a temporary directory unless `--out` is set.

For a local proof: `bun contracts/captcha/e2e/stub-mcp.ts --scenario contract`, then pass its URL with `--mcp-url` and `--fixture-origin http://fixtures.invalid`.
