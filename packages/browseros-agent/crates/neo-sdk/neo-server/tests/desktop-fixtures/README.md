# Run the SDK desktop fixtures

This suite executes 212 expanded desktop contracts through the SDK's real MCP `run` tool. The expected result is 207 passes and five strict expected failures. `capture-io-017` is a passing iframe download contract. `read-eval-021` passes by verifying the intended script exception.

From `packages/browseros-agent`, run the full suite on the Linux pool:

```sh
bun run remote run -- sh -c 'bun ci && bun crates/neo-sdk/neo-server/tests/desktop-fixtures/check.ts --run'
```

The runner builds `browseros-claw-sdk` from the checkout before it starts a browser. It invokes the existing `test:browser` launcher once for 211 ordinary cases, then once with native `--force-device-scale-factor=2` for `screenshot-scale-002`. The SDK connects to the launcher's CDP port. A prebuilt binary override cannot bypass the build.

To select one expanded case, retain the public runner:

```sh
bun run remote run -- sh -c 'bun ci && bun crates/neo-sdk/neo-server/tests/desktop-fixtures/run.ts --case act-001'
```

`bun ci` installs the workspace dependencies before the suite starts. CI performs that installation before invoking `check.ts --run` too.

Set `--out <directory>` to choose the artifact directory. The default is a unique directory beneath `target/sdk-desktop-fixtures`. For local execution, run `bun ci` once, then invoke `run.ts` directly with an installed browser. The browser selector accepts `BROWSEROS_BINARY` or `NEO_E2E_BROWSER`; CI browser arguments pass through the suite's adapter.

Inspect `run.json`, `build.json`, aggregate `junit.xml`, and each partition's `receipt.json`. Receipts separate the source build, browser and SDK startup, case execution, case cleanup, and launcher cleanup. Each expanded ID has a fresh task and its own raw MCP evidence beneath the partition's `cases` directory. These runtime artifacts can contain private browser paths and returned data.

One Linux amd64 pool run completed all 212 cases with 207 passes and five strict expected failures in 4m06s. That includes a 1m15s source build, 2m46s for the ordinary partition, and 3.6s for the native DPR2 partition. Both launcher cleanups passed. Dependency installation and offline preflight are excluded.

The five expected failures remain visible in console output, receipts, and JUnit. Their matchers require the known script owner, code, message, and payload. A successful result is an XPASS and fails the suite. Setup, transport, host assertions, and cleanup failures also fail. Uncertain requests or failed cleanup stop browser state reuse.

After dependency setup, validate offline without Cargo or a browser:

```sh
bun crates/neo-sdk/neo-server/tests/desktop-fixtures/check.ts
```

This is the same offline preflight CI runs before the live suite. It validates the inventory, runs focused Bun checks, and typechecks every suite module with the installed TypeScript. `run.ts --check` performs only the inventory validation.

Offline checks need only committed files. They verify the exact expanded membership, typed registry serialization, 155 raw body hashes, shared helper hash, source syntax, strict failure signatures, request ordering, and lifecycle guards. The fixture server serves pages directly from `contracts/claw-mcp/fixtures/pages`.

To regenerate the registry and function bodies from a compatible external bundle, supply its path explicitly:

```sh
bun crates/neo-sdk/neo-server/tests/desktop-fixtures/import.ts <external-bundle>
bun crates/neo-sdk/neo-server/tests/desktop-fixtures/import.ts --check
```

The importer validates the original classification boundary and emits only the selected contracts, shared assertions, and referenced bodies. Saved replies, reports, bootstrap code, and fixture pages stay outside the generated output. Every alias executes again during a live run, even when aliases share a body.
