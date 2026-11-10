# Neo SDK

The hosted Neo implementation lives here. This is a grouping folder, not a Cargo or npm
package: each child owns its manifest, and the agent workspace lists the eight Rust members
explicitly. Keep tool configuration inside the packages that need it so it cannot change
the lint or formatting rules of sibling crates.

```text
host / MCP run
      |
   neo-sdk --------> neo-playwright <--- neo-playwright-js (builds its assets)
      |                    |
      +----> neo-service --+----> neo-trace
      |          |                    ^
      |          +----> neo-takeover --+
      |          +----> neo-sdk-api (spec + generated types)
      |          +----> neo-capture <----- claw-server (desktop)
      +---------------> neo-captcha <----- claw-server (desktop)
```

| Package | Responsibility |
|---|---|
| `neo-sdk` | Stateless MCP `run`, JavaScript runtime, dispatch and application wiring. Builds `browseros-claw-sdk`. |
| `neo-playwright` | Browser operations, locators, snapshots and the sealed-world engine boundary. |
| `neo-playwright-js` | JavaScript probe and agent prelude; builds the engine's committed assets and fixtures. |
| `neo-service` | Authenticated host API, traces, live views and takeover routes. |
| `neo-takeover` | Tab locks, display control, viewport fitting and owner handoff. |
| `neo-trace` | Run and operation records, storage, masking and privacy. |
| `neo-sdk-api` | Host API specification, generated Rust models and migration notes. |
| `neo-captcha` | Shared CAPTCHA solver, also used by desktop. |
| `neo-capture` | Shared capture hub, also used by desktop. |

The pre-existing browser infrastructure (`browseros-cdp`, `browseros-core` and the other
workspace crates) stays outside this folder. Desktop depends on `neo-captcha` and
`neo-capture`; the SDK server and its other libraries do not enter desktop's dependency graph.

From `packages/browseros-agent`, run the existing SDK gate with private build state:

```sh
CARGO_BUILD_BUILD_DIR="$PWD/target/build-private" crates/neo-sdk/check.sh
```

The gate covers `neo-sdk` and `neo-playwright`. Follow [STYLE.md](STYLE.md) and the workspace's
contributor rules. Build the server with `cargo build -p neo-sdk --bin browseros-claw-sdk`.
Use Bun 1.4.2 in `neo-playwright-js` for `bun run build`, `bun run build:check` and its tests.
Regenerate the host contract from the agent root with `bun run codegen:neo-sdk-api`; never
edit `neo-sdk-api/src/generated` by hand.

Release and protocol identities remain `claw-sdk`, `browseros-claw-sdk`, the existing version
marker, MCP name and `CLAW_*` environment variables. The package rename changes tracing
targets to `neo_captcha::…` and `neo_capture::…`, including in desktop logs. Custom `CLAW_LOG`
directives naming either crate must use its new target; the default filter is unchanged.
