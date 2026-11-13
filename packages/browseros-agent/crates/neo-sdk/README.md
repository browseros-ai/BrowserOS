# Neo SDK

The hosted Neo implementation lives here. This is a grouping folder, not a Cargo or npm
package: each child owns its manifest, and the agent workspace lists the eight Rust members
explicitly. Keep tool configuration inside the packages that need it so it cannot change
the lint or formatting rules of sibling crates.

Give agents [agent.d.ts](agent.d.ts) with the MCP URL for the task API, script globals, and a four-run example.

Each `run` starts with fresh JavaScript. With no task, built-in `page` and
`context.newPage()` each open a new tab with no task, never an existing tab.
To keep tabs across runs, create a task with `await neo.createTask(name)`, return `task.id`,
and start later scripts with `await neo.task(id)`. The latest task pick wins: built-in
`page` uses its last-used open tab, else another open tab, else a new one;
`context.newPage()` opens in the picked task, and `task.newPage()` opens in that task.
Popups join their opener's task. `task.pages()` lists its tabs oldest first;
`task.page(tabId)` picks one. Finish with `await task.close()` to close its tabs and end it.

Any script may use any tab or task. `context.pages()` lists every tab with `tabId` and
`taskId` (`null` without a task). Recover a lost id with
`context.pages().find(p => p.url().includes('ycombinator'))?.taskId`.
An unknown id throws `task_not_found`, whose hint points to `context.pages()` and `page.taskId`.
The reply is exactly what the script returned, plus console lines and existing notes.
Hosts get touched tasks in `_meta["com.browseros.neo/run"].tasks`; host task ids and the
neo-viewer proxy keep their existing behavior. Tasks live in memory (at most 4096);
a restart forgets them while tabs stay. The janitor closes agent-opened tabs nobody used for the host's `tabs.idleCloseSeconds`, 30 minutes by default.

```text
host / MCP run
      |
   neo-server -----> neo-playwright <--- neo-playwright-js (builds its assets)
      |                    |
      +----> neo-http -----+----> neo-trace
      |          |                    ^
      |          +----> neo-takeover --+
      |          +----> neo-sdk-api (spec + generated types)
      |          +----> neo-screencast <----- claw-server (desktop)
      +---------------> neo-captcha <----- claw-server (desktop)
```

| Package | Responsibility |
|---|---|
| `neo-server` | Stateless MCP `run`, JavaScript runtime, dispatch and application wiring. Builds `browseros-claw-sdk`. |
| `neo-playwright` | Browser operations, locators, snapshots and the isolated-world engine boundary. |
| `neo-playwright-js` | JavaScript probe and agent prelude; builds the engine's committed assets and fixtures. |
| `neo-http` | Authenticated host API, traces, live views and takeover routes. |
| `neo-takeover` | Tab locks, display control, viewport fitting and owner handoff. |
| `neo-trace` | Run and operation records, storage and bounded trace values. |
| `neo-sdk-api` | Host API specification, generated Rust models and migration notes. |
| `neo-captcha` | Shared CAPTCHA solver, also used by desktop. `testbed/` checks it end to end through `run` ([README](neo-captcha/testbed/README.md)). |
| `neo-screencast` | Shared capture hub, also used by desktop. |

`neo-server/docker/` builds the Neo SDK image, the browser plus this server, for hosts such as
zuzu: see [its README](neo-server/docker/README.md).

The pre-existing browser infrastructure (`browseros-cdp`, `browseros-core` and the other
workspace crates) stays outside this folder. Desktop depends on `neo-captcha` and
`neo-screencast`; the SDK server and its other libraries do not enter desktop's dependency graph.

From `packages/browseros-agent`, run the existing SDK gate with private build state:

```sh
CARGO_BUILD_BUILD_DIR="$PWD/target/build-private" crates/neo-sdk/check.sh
```

The gate covers every Rust crate in this folder: rustfmt, clippy `-D warnings` and tests for each
(not yet `neo-captcha`'s tests, which are flaky; `check.sh` says why), plus Google's rustfmt
options and the forbidden-dependency check for `neo-server` and `neo-playwright`. Follow [STYLE.md](STYLE.md) and the workspace's
contributor rules. Build the server with `cargo build -p neo-server --bin browseros-claw-sdk`.
Use Bun 1.4.2 in `neo-playwright-js` for `bun run build`, `bun run build:check` and its tests.
Regenerate the host contract from the agent root with `bun run codegen:neo-sdk-api`; never
edit `neo-sdk-api/src/generated` by hand.

Release and protocol identities remain `claw-sdk`, `browseros-claw-sdk`, the existing version
marker, MCP name and `CLAW_*` environment variables. The package rename changes tracing
targets to `neo_captcha::…` and `neo_screencast::…`, including in desktop logs. Custom `CLAW_LOG`
directives naming either crate must use its new target; the default filter is unchanged.

The server crate now uses `neo_server::…` tracing targets. Custom `CLAW_LOG` directives
for its former crate target should use `neo_server`; the variable and default filter stay the same.
