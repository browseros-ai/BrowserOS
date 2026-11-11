# claw-sdk

claw-sdk is hosted BrowserOS Neo's sidecar. Chromium launches it beside the browser, and agents talk to it instead of the desktop claw-server. It serves one stateless MCP tool, `run`, which executes an agent's JavaScript against the browser. Every SDK operation that script performs goes through one dispatcher, which owns the rules, delivery, and the trace.

The binary is `browseros-claw-sdk`. Code style and review rules are in [STYLE.md](STYLE.md).

## Hosting the real SDK in Node

The default-off Cargo feature `hosted-run` exports `hosted::{HostedRuntime, HostedOptions,
HostedRun}`. It builds the real trace store, browser link, Playwright
facade, built-ins, dispatcher and tab janitor. It binds no listeners and disables the trace
screenshot queue. Feature-off builds retain the product path.

Each connection owns an `App::runtime()` executor with the product's worker stack size.
`open_run(timeout)` starts a real `RunCtx` and seeds the same tab listing as MCP runs.
`invoke(name, json, site)` enters that executor and calls `Bridge::submit` synchronously; its
returned future always resolves to the bridge envelope. `log` and `sleep` use the same bridge.
`finish()` stops, drains operations, builds the reply and records the end exactly once; a
deadline or dropped run also ends through that path. `close()` drains runs, flushes traces
and releases the private executor, including core's background tasks and CDP socket.

`state_dir` is required; the store is `<state_dir>/claw-sdk/trace.db`. Pass Chromium's
`<user-data-dir>/.browseros` as `execution_dir` to locate the browser profile, and the installed
app's version as `browseros_version` for native capability gates. Without an execution directory,
the product refuses path uploads. An initial run has `init.page = null`; `init.pages` includes
all existing tabs, including the launch tab. The lazy default page opens when the SDK needs it.

The isolated N-API crate is `../neo-playwright-tests/node/claw-addon` (its own workspace and
lockfile, seeded from the agent lock). From `packages/browseros-agent`:

```sh
cargo build --release --locked --manifest-path crates/neo-sdk/neo-playwright-tests/node/claw-addon/Cargo.toml
cp "$CARGO_TARGET_DIR/release/libclaw_node.dylib" crates/neo-sdk/neo-playwright-tests/node/claw-addon/claw_node.node
node --test crates/neo-sdk/neo-playwright-tests/node/claw-smoke.test.mjs
```

The loader `openClaw(cdpPort, { timeoutMs, stateDir, executionDir, browserosVersion },
{ onUncaught })` returns the real SDK parameters, the native run, runtime, source hashes and
an idempotent `finish()`. `CLAW_NODE_ADDON` overrides the add-on path. Omitted `stateDir`
uses a temporary directory removed at close. The default run timeout is 24 hours.

The loader reads bootstrap, Playwright prelude and Neo prelude verbatim, adds the production
strict-mode prefix and evaluates them with `vm.compileFunction` in Node's main realm. A
`globalThis` parameter holds the natives and `__neo` privately. The SDK's Promise, Error and
Uint8Array have Node's identity; QuickJS's memory and CPU limits do not apply. `__neo.run`
keeps every provided parameter and stays pending until finish. Listener throws surface through
Node's `unhandledRejection` event and are forwarded to `onUncaught` for the worker's lifetime;
existing Node listeners still observe them. Use one loader run per worker.

Web globals (`URL`, `URLSearchParams`, `atob`, `btoa`, `DOMException`) also come from Node's
main realm, as in the prelude's VM test rig. The loader does not evaluate `web_globals.js`:
its factory requires QuickJS's Rust natives. Production URL parsing uses Rust's `url` 2.5.8,
which retains older opaque-path trailing-space rules and exposes no `URL.createObjectURL`
or `URL.revokeObjectURL`. Node uses its own URL implementation and blob URL methods, so the
comparison does not prove parity for these globals. `sources` still hashes the three
production bootstrap and prelude files.

The smoke skips without `NEO_E2E_BROWSER`. It owns a headless browser on a throwaway profile,
verifies its process group before cleanup, and requires its Node child to exit naturally.
The primary launch leaves `BROWSERCLAW_HOSTED_MODE` unset to match the comparison runner;
`CLAW_SMOKE_HOSTED=1` repeats it with BrowserOS hosted mode and reports console, popup and
`setContent` differences. `NEO_NODE_OUTPUT_DIR` retains proof, logs and cleanup records.

## How it runs

To run it beside the browser in a container, use the Neo SDK image in [docker/](docker/README.md).

Chromium starts the binary with its launch JSON and reads the exit code.

| Invocation or event | Result |
|---|---|
| `--version` | Prints the version alone on stdout and exits 0. Reads no config and opens no state. |
| `--config <launch.json>` | Serves until SIGTERM or SIGINT, then exits 0. |
| The agent port is taken | Exits 2, and Chromium relaunches Neo on the next port. Nothing else exits 2. |
| Any other failure | Exits 1, bad arguments included. |

From the launch JSON it reads `ports.server` (default 9200), `ports.cdp` (default 49337), `directories.resources`, `directories.execution` (Chromium's `<user-data-dir>/.browseros`), and `instance.browseros_version`. It ignores every other key. claw-sdk has no config file of its own, so it logs and ignores `BROWSERCLAW_CONFIG_FILE`.

`src/config/env_config.rs` reads the environment once, at start. A blank value counts as unset. A malformed address or screen size fails the start with exit 1.

| Variable | Default | Effect |
|---|---|---|
| `BROWSERCLAW_DIR` | `$HOME/.browserclaw` | The state directory. The start fails when both it and `HOME` are unset. |
| `CLAW_LOG` | `info,rmcp=warn` | The `tracing` filter. A filter that does not parse falls back to the default with a warning. |
| `BROWSERCLAW_HOSTED_MODE` | unset | Must be `1`. Any other value logs an error, because popups then open as windows Neo cannot drive. Neo still starts. |
| `CLAW_NATIVE_BROWSEROS_VERSION` | the launch JSON's | Overrides the browser version that gates core's native features. |
| `CLAW_HOST_SOCKET` | `$BROWSERCLAW_DIR/host.sock` | The host API's Unix socket. |
| `CLAW_HOST_TOKEN_FILE` | unset | The host's bearer token. Without a usable token (32 or more bytes of visible ASCII) there is no host listener, and `run` still works. |
| `CLAW_HOST_TCP` | unset | Also serves the host API on this TCP address. |
| `CLAW_XVNC_ADDR` | `127.0.0.1:5900` | Xvnc, which a takeover fit resizes. |
| `NEO_SCREEN` | `1600x1000` | Neo's screen size between fits. |
| `BROWSEROS_CAPSOLVER_API_KEY_FILE`, `BROWSEROS_2CAPTCHA_API_KEY_FILE` | unset | CAPTCHA provider keys. Without a readable one, `neo.solveCaptcha` is unlisted and fails `host / captcha_unavailable`. neo-captcha also reads `BROWSEROS_CAPSOLVER_API_URL`, `BROWSEROS_2CAPTCHA_API_URL`, and `BROWSEROS_CAPTCHA_PAID_TASK_LIMIT` (default 100). |

Neo binds two listeners:

- The agent listener is `127.0.0.1:<ports.server>`, with no auth. Chromium's proxy forwards to it. It serves `/mcp`, plus `GET /system/health` and `GET /system/ready` for Chromium's supervisor. `/mcp` refuses any request that carries `Origin` or `Sec-Fetch-Site`, so a web page in the driven browser cannot call `run`.
- The host listener is a Unix socket with mode 0660, plus TCP when `CLAW_HOST_TCP` is set. It serves neo-service's `/v1` routes (runs, operations, screenshots, events, locks, tabs, and fits) behind the service's bearer and peer-uid checks. The service starts when the browser link first connects.

State lives under `$BROWSERCLAW_DIR`:

- `claw-sdk/trace.db` is the SQLite trace store. Supported older files are upgraded in place. If it does not open (including an unknown newer migration or the earliest session-era schema), Neo moves it aside to `trace.db.broken-<millis>` and starts a fresh one. Rolling back across a newer migration loses the live trace and request-id history.
- `logs/claw-sdk.log.<date>` is the daily log. The same lines go to stderr. `logs/claw-sdk-panics.log` gets each panic, written synchronously.
- `host.sock` is the host socket, unless `CLAW_HOST_SOCKET` moves it.

`setInputFiles` uploads any file Neo's process can read, as Playwright does; the caller sandboxes the filesystem. A relative path is made absolute against Neo's working directory. A call's payloads total under 10 MiB, and a directory upload needs its tab in front.

## The `run` contract

The realm provides `URL`, `URLSearchParams`, `atob`, `btoa` and `DOMException`. Known URL differences: `url` 2.5.8 retains the older opaque-path trailing-space rules; `URL.createObjectURL` and `URL.revokeObjectURL` are absent.

The model sends `run { code, timeout? }`. `code` is the body of an async function whose parameters are the SDK: Playwright's `page`, `context`, `browser` and `expect`, and Node's `Buffer` (neo-playwright's prelude), `neo`, `console`, the timers, and the error classes. `timeout` is in milliseconds, from 1 to 900000, and defaults to 300000. Unknown fields are refused. Each run gets a fresh QuickJS realm. `src/mcp/run_description.rs` builds the text the model reads about all of this.

The host sends its ids in `_meta["com.browseros.neo/run"]` as `{ requestId?, taskId?, hostData? }`. Neo refuses unknown keys as `host / invalid_meta`. `taskId` makes the task's last tab the run's `page`. Without it, `page` opens a background tab on the first call that needs one. `hostData` is up to 2048 bytes of JSON that Neo stores with the run and never reads.

The result carries `_meta["com.browseros.neo/run"]` as `{ id, status, effects, error?, unknown }`. `id` is the run's id in the trace. `status` is `ok`, `error`, `cancelled`, or `timeout`, and `isError` is set unless it is `ok`. `effects` is `none`, `done`, or `unknown`. `unknown` lists the writes that were sent and never confirmed.

The script's `page`, `context`, `browser` and `expect` behave as Playwright 1.64.1's, except where Neo says otherwise in the `run` description, which lists each difference a script would trip on. A call that waits gives up after 5 s unless given a `timeout`, and a navigation after 30 s. For the main-world scripts (`page.evaluate`, `page.waitForFunction`, `locator.evaluate`, `locator.evaluateAll`, `locator.waitForFunction`, `elementHandle.evaluate`, `$eval` and `$$eval`, each `frame` form in its own frame) and `setContent`, whose HTML's scripts run in the main world too, those are:

- `waitForFunction` polls every 100 ms for its default `polling: 'raf'`, because animation frames never fire in Neo's background tabs. The handle it resolves to holds a snapshot of the value, so `jsonValue()` reads it and `evaluate` on it fails `neo / unsupported`.
- A result nested more than about 50 objects deep fails its call with `Maximum call stack size exceeded` (`page.evaluate: Maximum call stack size exceeded`, an `Error` with no owner or code, since the op answered), where Playwright allows 98. The prelude's result parser recurses on the realm's 512 KiB stack (`Defaults::JS_STACK_BYTES`); 50 was measured in a debug build.
- `setContent` refuses `waitUntil: 'networkidle'` as `neo / unsupported`, with nothing sent. Chromium never fires `networkIdle` for a written document, and Neo does not count the page's requests itself, so the wait could only time out. Use `load`.
- An owner's lock that blocks reads, taken while a script runs, withholds what the script read: the call fails `host / owner_has_control`, delivery `done`.

`page.addInitScript` and `context.addInitScript` register a script that every new document runs in its main world, before the page's own scripts, as Playwright's do. They differ in these ways:

- A script lasts as long as the run that registered it. Neo's tabs outlive a run, so when the run ends, however it ends, Neo removes the script and the tab's later documents no longer run it. Within the run it survives every navigation, and the returned disposable removes it sooner.
- Neo's context is the owner's whole profile, so a context's script does not go to every tab at once. It reaches each tab at the run's first write to that tab, after the door's rules on the tab, before the write: every document the run loads runs it, and a popup's first document, which loads before the run acts on the popup, does not.
- Neo adds a script to the tab's page session, which covers the main frame and its same-process frames. A cross-site iframe has a session of its own and runs without it.
- `exposeFunctions` passes the script's functions to the page as bindings, below.

`page.exposeBinding`, `page.exposeFunction`, their `context` forms and `addInitScript`'s `exposeFunctions` let the page call a function of the script and get its result back, as Playwright's do. The page's call goes to CDP's `__playwright__binding__` (in the main world only), Neo records it, the prelude's event pump runs the function in the run's realm, and Neo answers in the execution context that called. They differ in these ways:

- A binding lasts as long as the run that exposed it, since the realm that answers it ends with the run. When the run ends Neo removes it from the tab's later documents and from the ones loaded then, so the page can no longer call into a run that is gone. `dispose()` does the same sooner.
- A context's binding reaches each tab as a context's init script does, at the run's first write to it. A cross-site iframe has no bindings.
- A call reaches the script through the event pump, which is not ordered against op replies. A call the page awaits (`await page.evaluate(() => window.add(1, 2))`) answers before the op returns; one the page fires and does not await, such as from an event handler, can run after the op that caused it returned.
- A binding answers with values; a handle in its result fails the page's call. `evaluate`'s `exposeFunctions` is refused as `script / method_unavailable`.

A `requestId` never runs twice. A repeat while its run is going joins it and gets the same reply. A repeat within 10 minutes of the end gets the stored reply, from memory or from the trace store, so this holds across a restart. Later repeats are refused as `host / duplicate_request`, naming the first run. If the store cannot be read, Neo refuses the call as `neo / trace_unavailable` rather than risk a second run. Neo notices a gone client at the next SSE keep-alive, within 5 seconds. A run with a `requestId` then keeps going for 15 seconds, so a retry can join it. A run without one stops at once.

## Module map

| Module | Owns |
|---|---|
| `main.rs` | The process contract: flags and exit codes. |
| `app` | Startup in dependency order, shutdown on SIGTERM, and housekeeping (idle tabs and trace retention). |
| `config` | The launch JSON, the environment, and `Defaults`, which holds every fixed value. Nothing else reads an env var. |
| `logging` | The daily log file, stderr, and the panic log. |
| `http` | The two listeners, the request hygiene in front of `/mcp`, and `/system/*`. |
| `mcp` | The `run` tool: parsing its arguments and `_meta`, its description, and rendering a reply. |
| `run` | One `run` call from admission to reply: replay, joins, the orphan grace, and the deadline. |
| `runtime` | The per-run QuickJS realm, the JS bootstrap and the `neo` prelude in `runtime/js/` plus neo-playwright's prelude, and the strings-only bridge to the dispatcher. The only module that names rquickjs. |
| `dispatch` | The door: `Dispatcher`, the `Method` trait, the `Registry`, the policy rules, and the error model. |
| `builtins` | Registers neo-playwright's ops on the process's one `Playwright`, `neo.solveCaptcha`, and the idle-tab janitor. `op_marks.rs` maps capability declarations to dispatch metadata; main-world answers and pulled events pass the takeover rules on their targeted tab. File uploads use Playwright's path handling, and navigation delegates URL handling to Chromium. |
| `browser` | The CDP link to Chromium, its reconnects, and what Neo knows about each tab. |
| `host` | The host API's lifecycle and the takeover lock the door checks. |
| `test_rig` | The unit tests' rig: a fake browser, a real trace store, and test methods. |
| `test_support` | `FakeCdp` and a test config, behind the `test-support` feature that only test builds turn on. |

Every SDK operation goes through one door. A script's call reaches `Dispatcher::submit` through the bridge, and the idle-tab janitor submits its closes the same way. A method that needs another action calls `OpCtx::invoke`, which runs it through the same door as a child op. The door owns the op id, the trace records, the rules, and delivery, so a method does its work and reports facts. `neo.solveCaptcha` is the one method that drives a page outside the door, and it holds a claim on the tab while it does.

## Add a built-in method

A built-in is a private type in `src/builtins/` that implements `dispatch::Method`. Copy `src/builtins/solve_captcha.rs`. The Playwright API is not written this way: neo-playwright owns its ops, and `src/builtins/playwright_ops.rs` registers each one from the crate's table.

1. Write an input type with `#[serde(deny_unknown_fields)]` and an output type that implements `Serialize`.
2. Implement `Method`:
   - `spec` returns a `MethodSpec` with the name scripts call, its `Effect` (`Read` or `Write`), and its `Listing` in the `run` description.
   - `target` returns the input's `PageTarget`, or `None` for a method with no page. There is no default, so the takeover lock always sees the tab a method acts on.
   - `call` does the work through its `OpCtx<Self>`. Await browser calls through `op.race`. A write calls `op.recheck()?` and then `op.mark_sent()` right before it sends, and the door derives delivery from that.
3. Register it with `builder.define` from `builtin_set` in `src/builtins/builtin_set.rs`. A duplicate name fails the start.
4. Expose it to scripts from a prelude in `src/runtime/js/` with `__neo.invoke('<name>', input)`. A new parameter of the agent's function goes through `__neo.provide`, and a test in `run_description.rs` fails until the description names it.

## Test it

`check.sh` is the gate, and the package's `lint:rust` script. It runs rustfmt with Google's options, the forbidden-dependency check, clippy with `-D warnings`, and `cargo test`, all on CI's pinned toolchain. Run it from `packages/browseros-agent`. In a Grove worktree, give it a private build directory, or it waits on the build lock that other worktrees hold:

```sh
CARGO_BUILD_BUILD_DIR="$(git rev-parse --show-toplevel)/packages/browseros-agent/target/build-private" \
  crates/neo-sdk/check.sh
```

Unit and integration tests run against `FakeCdp`. The `*_browseros.rs` integration tests drive a real headless BrowserOS instead. Stock Chrome does not work, because core uses BrowserOS-only CDP methods. The tests pass without doing anything unless `CLAW_SDK_E2E_BROWSER` names the browser binary:

```sh
CLAW_SDK_E2E_BROWSER=/Applications/BrowserOS.app/Contents/MacOS/BrowserOS \
  cargo test -p neo-server --locked --test e2e_browseros --test acceptance_browseros \
    --test playwright_browseros --test uploads_browseros --test served_methods_browseros \
    --test main_world_browseros
```

- `tests/e2e_browseros.rs` launches the built binary the way Chromium does, runs scripts over MCP, and reads them back over the host socket.
- `tests/playwright_browseros.rs` runs one Playwright script over MCP, the way a model trained on Playwright writes it. The script loads a page with a same-origin iframe into the run's own `page`, reads its `ariaSnapshot({ mode: 'ai' })`, reads elements by a printed ref through `getByRef` and `aria-ref=`, and reads through locators and inside the frame.
- `tests/uploads_browseros.rs` uploads through `setInputFiles` over MCP: host-owned files and a symlink to one, a file of the agent's own, a payload and `[]` each reach the page.
- `tests/served_methods_browseros.rs` checks that the prelude's refusal of a method Neo does not serve is one row of the run's trace.
- `tests/main_world_browseros.rs` runs each main-world script API (`page.evaluate`, `waitForFunction`, `locator.evaluate`, `evaluateAll`, `elementHandle.evaluate`, `$eval` and `$$eval`, and the `frame` forms in a child frame) and `setContent` as Playwright does: its values, arguments, handles and errors, and the document `setContent` writes, whose inline script runs once, and whose password field reads as the page holds it in every read. An owner's lock that blocks reads withholds what a running script read. A unit test, `every_capability_op_has_a_browseros_case` in `src/builtins/op_marks.rs`, fails until every op that reaches the main world, Neo's files or Neo's screen names a test in one of these BrowserOS suites that proves it served or held.
- `tests/acceptance_browseros.rs` runs one task end to end. The agent drives a tab, and the host reads its trace, locks the tab, sees the agent's next write refused, releases the lock, and finds the tab's picture after that write.

`packages/browseros/docker/neo/smoke_sdk.py` checks a running claw-sdk Neo container from the host. It checks health and readiness, that `/mcp` lists the one `run` tool without issuing a session id, and that one run loads a page and reads its title. Run it as `python3 smoke_sdk.py --mcp-port <port>`. The port defaults to 9010.
