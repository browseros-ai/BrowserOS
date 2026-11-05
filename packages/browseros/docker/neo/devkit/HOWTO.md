# Try claw-sdk by hand

This kit runs Neo in Docker with this checkout's claw-sdk as its server. You watch Neo's screen in a browser, point an MCP client at it, send scripts, and see for every operation the code that ran, what it was given, what came back, and a screenshot. It is for development only. The image and its entrypoint are the release ones, and only the way `dev.sh` starts the container differs.

The same guide with screenshots of the viewer and the trace UI: https://docs.shadowfax92.workers.dev/9Hf3E3tN

## Quickstart

Run these from `packages/browseros/docker/neo`:

```sh
./dev.sh up --server claw-sdk
open http://127.0.0.1:16090/
open http://127.0.0.1:16090/trace
./dev.sh run --task demo -f devkit/examples/goto-title.js
./dev.sh down
```

1. `up --server claw-sdk` builds claw-sdk for Linux, bakes it into the Neo image, starts the container and the gateway, and prints every address. The first build takes about 2 minutes.
2. `http://127.0.0.1:16090/` is the viewer: Neo's whole screen, through claw-sdk's own display.
3. `http://127.0.0.1:16090/trace` is the trace UI. It lists every run and updates as runs happen.
4. `run` sends a script to claw-sdk's `run` tool, prints what came back, and logs the call.
5. `down` stops the container and the gateway. The browser profile and its logins stay.

`up` ends with this block:

```text
Server:   claw-sdk (read from the container)
Viewer:   http://127.0.0.1:16090/   (?mode=control to type into it)
Trace UI: http://127.0.0.1:16090/trace
MCP:      http://127.0.0.1:19010/mcp
Host API: http://127.0.0.1:19300/v1   bearer token in /Users/<you>/.local/state/neo-dev/host-token

MCP client config:
  {"mcpServers": {"neo-sdk": {"type": "http", "url": "http://127.0.0.1:19010/mcp"}}}
Claude Code:
  claude mcp add --transport http neo-sdk http://127.0.0.1:19010/mcp
```

After you change claw-sdk, run `./dev.sh up --server claw-sdk` again. It rebuilds only what changed and replaces the container and the gateway. To restart the gateway alone after editing `devkit/`, run `./dev.sh gateway`.

Setup, once: Docker, `brew install jq zig`, `cargo install --locked cargo-zigbuild`, `rustup toolchain install 1.98.0`, `rustup +1.98.0 target add aarch64-unknown-linux-gnu`, and `npx` (the gateway runs on Bun 1.4.2, which `npx` fetches). `dev.sh` builds with Rust 1.98.0, the release job's toolchain. Set `NEO_RUST_TOOLCHAIN=` to use your default instead.

## Connect an MCP client

Paste the config that `up` prints into your MCP client, or run the `claude mcp add` line. claw-sdk serves one tool, `run`, whose input is `{ code, timeout? }`. `code` is the body of an async function that gets Playwright's `page`, `context`, `browser` and `expect`, plus `neo` and `console`.

A client can send its own ids in `_meta["com.browseros.neo/run"]` as `{ requestId, taskId }`. Always send a `taskId`. A run with one reuses that task's last tab as its `page`. A run without one drives the last tab any script touched, which can be another task's tab.

## Watch and drive the browser

The viewer opens in view mode. You see Neo's screen, and your keys and clicks go nowhere.

To type into Neo, pick **Control** or open `http://127.0.0.1:16090/?mode=control`. Your first key, click or paste makes every open tab private. A private tab gets no screenshots, and view displays pause while one is in front of the screen. A tab stays private until it closes. To get the view back, close those tabs, for example with `./dev.sh run --task <task> -f devkit/examples/close-page.js`.


The status line says why a display ended:

- `4409 display_paused` means the owner has control or a private tab is in front. The viewer retries every 2 seconds.
- `4002 released` means the display's takeover was released.
- `4502` means the gateway cannot reach claw-sdk. Check that the container is up.

**Follow** keeps one task's current page, or one tab, in front of the screen while nobody has control. The **Takeover** panel starts and releases a takeover on a tab; see the takeover example below.

## Run the examples

Each example is a script in `devkit/examples/`. `run` prints the script's return value, then one status line with the run's id, status, effects, time and tabs.

### Load a page and read its title

```sh
./dev.sh run --task howto -f devkit/examples/goto-title.js
```

```text
{"title":"Example Domain","url":"https://example.com/"}
run 01a1acf4-cf1f-7102-8a53-f7b39569a66e ok effects=done 73ms tabs=[1276814799]
```

### Read an AI snapshot, then act on a printed ref

`aria-ref.js` reads `page.ariaSnapshot({ mode: 'ai' })`, takes the ref printed for a button, and reads that element back through `page.getByRef(ref)` and `aria-ref=`.

```sh
./dev.sh run --task howto-aria -f devkit/examples/aria-ref.js
```

```text
{"byAriaRef":"Save order","byRef":"Save order","frameByRef":"Frame action","frameRef":"f1e3","ref":"e3"}
run 01a1acf4-d049-7000-97d6-144d5f9b06ff ok effects=done 93ms tabs=[1276814800]
```

A main-frame ref is bare (`e3`) only on a tab's first page. After the tab loads another page it carries a frame prefix (`f2e3`), so match refs with `(?:f\d+)?e\d+`.

### Read inside a frame

```sh
./dev.sh run --task howto-frame -f devkit/examples/frame-read.js
```

```text
{"button":"Frame action","frames":2,"fromFrames":"Inside the frame","text":"Inside the frame"}
run 01a1acf4-d174-75c2-aed5-c5fa9ae847fc ok effects=done 82ms tabs=[1276814801]
```

### Fill a form, click, and assert what the page shows

`form-actions.js` fills two fields found by their labels, clicks **Send**, and waits with `expect(...).toHaveText` until the page shows the result.

```sh
./dev.sh run --task howto-actions -f devkit/examples/form-actions.js
```

```text
{"shown":"Thanks, Ada Lovelace (ada@example.com)"}
run 01a1ad11-0c76-7170-9f9b-e97820315853 ok effects=done 106ms tabs=[1276815223]
```

In the trace UI, each call is one operation with its own delivery: two `locator.fill`, a `locator.click`, and the `expect.locator` assertion.

`keyboard-submit.js` types with `page.keyboard.type`, submits with `page.keyboard.press('Enter')`, and asserts the same way:

```text
{"shown":"Results for claw sdk"}
run 01a1ad11-0d93-7030-96ae-f432286a6a94 ok effects=done 130ms tabs=[1276815223]
```

`clear-field.js` and `set-checked.js` show `locator.clear()` and `locator.setChecked()`. The SDK runs them as `fill('')` and as `check()` or `uncheck()`, so the trace shows `locator.fill` and `locator.check`.

### See a host lock refuse a write

A lock keeps the agent out of one tab. Take the tab id from the status line of the task's last run.

```sh
./dev.sh run --task howto -f devkit/examples/goto-title.js        # tabs=[1276814799]
./dev.sh host POST /v1/locks '{"tabId":1276814799,"ttlSeconds":60}'
./dev.sh run --task howto -f devkit/examples/task-goto.js
./dev.sh host DELETE /v1/locks/<lock id>
./dev.sh run --task howto -f devkit/examples/task-goto.js
```

While the lock is held, the write is refused before anything is sent:

```text
host / owner_has_control: The owner has control of this tab. Nothing was done. Wait for them to give control back. (lock lk_01a1acf4-e959-7621-b5ff-b29a0cd48a2a) (op 1, delivery not_sent)
run 01a1acf4-ebbe-7033-823e-78a54792b309 error effects=none 31ms tabs=[1276814799] error=host/owner_has_control
```

After `DELETE`, the same run works:

```text
{"url":"https://example.com/?from=task-goto"}
run 01a1acf4-f13e-7333-8a6b-157c1468dbfc ok effects=done 57ms tabs=[1276814799]
```


### Take over a tab, then give it back

A takeover makes the tab private, locks the agent out of it, and gives the owner a control display bound to the takeover. The quickest way is the viewer's **Takeover** panel: enter the tab id, pick **Start takeover**, type into the page, then pick **Release**. From the shell:

```sh
./dev.sh host POST /v1/takeovers '{"tabId":1276814799}'
open 'http://127.0.0.1:16090/?mode=control&release=<takeover id>'
./dev.sh run --task howto "return await page.title()"           # refused: host/owner_has_control
./dev.sh host POST /v1/takeovers/<takeover id>/release
./dev.sh run --task howto "return await page.title()"           # "Example Domain"
./dev.sh run --task howto -f devkit/examples/close-page.js       # the tab stays private until it closes
```

The takeover answers with the display to open:

```text
{"id":"tk_01a1acf4-f1fa-7751-a50a-47f6f6be4019","tabId":1276814799,"state":"active","lockId":"lk_01a1acf4-f1fa-7751-a50a-480a66e09075","display":"/v1/display?mode=control&release=tk_01a1acf4-f1fa-7751-a50a-47f6f6be4019",...}
```

Release closes that display (`4002 released`) and lifts the lock.

### Run them all

`devkit/examples/check.sh` runs every example, the lock and takeover flows, and the probes in the next section through `dev.sh run`. It checks each reply and Neo's own trace, prints `ok` or `FAIL` per step, and exits non-zero on a failure. It takes about 10 seconds.

## Read the trace

The trace UI joins two records of each run on its `requestId`:

- Neo's own trace, from the host API: the run's code and result, and every operation with its arguments, result, outcome, delivery (`not_sent`, `done` or `unknown`), error owner, duration and screenshot.
- The CLI log, `~/.local/state/neo-dev/traces/runs.jsonl`: the code `dev.sh run` sent, the MCP reply as the client saw it (console output included), and the round-trip time.

**Follow newest** selects each run as it starts. **Export JSON** downloads the selected run with its screenshots inlined and its CLI record. The list loads the newest 100 runs, and the filter searches only the runs it has loaded, so pick **Load older runs** to reach older ones.


To copy everything to disk, run `./dev.sh export`. It writes `~/.local/state/neo-dev/traces/export-<time>/` with `runs.json`, one `runs/<run id>.json` per run, `screenshots/<operation id>.jpg`, `events.json` (the events Neo still holds, up to 1024) and `cli-runs.jsonl`.

To call the host API yourself, use `./dev.sh host <METHOD> <path> [json]`, for example `./dev.sh host GET '/v1/runs?taskId=demo'`. It adds the bearer token, which never appears on the command line.

## What does not work yet

These are the gaps a tester hits first, on `feat/claw-sdk` at `12427d0b9`, which includes the actions and `expect`. `devkit/examples/check.sh` re-checks every row, and most rows have a `probe-*.js` script you can run yourself. The requirements behind them are in the claw-sdk gap analysis (`claw-sdk/requirements/gap-analysis.md`, rows 5 to 9).

| Call | What you get today | Why |
|---|---|---|
| `page.evaluate`, `page.waitForFunction`, `locator.evaluate`, `locator.evaluateAll`, `page.setContent` | `script / method_unavailable`, delivery `not_sent` | Held: they run script in the page's main world, and the privacy fix that lets them back is not merged. Read through locators, `page.content()` or `page.title()` instead. |
| `locator.setInputFiles` | `script / method_unavailable`, delivery `not_sent` | Held: it uploaded any file Neo's process could read from Neo's own disk. |
| `page.bringToFront` | `script / script_error: page.bringToFront: Neo keeps agent tabs in the background; act on the page directly` | Refused on purpose, so the owner's screen never jumps. A person brings a tab forward with the viewer's **Follow** or a takeover. |
| Network: `page.waitForResponse`, `page.waitForRequest`, `page.on('request')`, `page.route` | `script / script_error: page.waitForResponse: Neo does not report network events; wait for what the page shows instead` (`route`: "network interception is not available in Neo") | Not built. The `Response` that `page.goto` returns works: `status()`, `ok()` and `url()`. |
| A tab parameter on `run` | `run` takes only `{ code, timeout }`, and refuses any other field | A run picks its tab through `_meta["com.browseros.neo/run"].taskId`: a task's runs share its last tab. Without a `taskId`, a run drives the last tab any script touched. |
| `page.goto('file:///…')` | `script / invalid_input`: Neo opens only http, https, data and about URLs | By design: a file in the container could show Neo's own secrets. |

`page.waitForTimeout` works, although the gap analysis lists it as missing from the op table. It is a timer in the SDK, not a browser operation: `waitForTimeout(1500)` returned after 1502 ms. It shows in the run's time, not as an operation in the trace.

`locator.clear` and `locator.setChecked` work too, although the gap analysis lists them as missing. The SDK runs them as `fill('')` and `check()` or `uncheck()`. `locator.screenshot` is an operation now. On a private tab it is refused as `host / page_private`, as `page.screenshot` is.

The held calls report `script / method_unavailable`. The ones the SDK refuses itself report `script / script_error` with no delivery, because nothing reached the browser.

## Gaps in Neo's trace

The trace UI needs a few things the host API does not record. Each row says what the kit does meanwhile and which field would close the gap in `contracts/claw-sdk-api/openapi.yaml`.

| Missing | Kit meanwhile | Field and place |
|---|---|---|
| The script's console output | Shows the CLI reply, which carries it | `Run.console`: the masked, bounded lines, on `Run` |
| The code while a run is running (`Run.code` is stored when the run ends) | Shows the CLI log's `code` | `Run.code` from the start, on `Run` and `RunSummary` |
| An event when a run is complete and its screenshots are taken (every `run.ended` arrives with `complete: false`) | Re-reads `GET /v1/runs/<id>?wait=10` after `run.ended` | A `run.completed` value of `Event.type` |
| Whether an operation's missing screenshot was withheld (private tab) or never taken | Asks `/v1/operations/<id>/screenshot` for every operation with a tab: 410 means withheld, 404 means none | `Operation.screenshotWithheld` (`private` or `expired`); today only `TraceTail.visuals` says it |
| Which lock or takeover refused an operation | Shows the error message, which names the lock | `OperationError.lockId` and `OperationError.takeoverId` |
| The page Neo opens itself for a script's first call | Shows it as one of the script's own calls | `Operation.parentId` set to the call that needed the page |
| Which operations the script called, and which the SDK made for it (`page.frames` is about a quarter of all operations) | Shows them all alike | `Operation.origin` (`agent`, `prelude` or `door`) |
| Which line of the script made an operation | Nothing: a position appears only inside a script error's text, such as `(run:1:253)` | `Operation.callsite` (`{line, column}` into `Run.code`) |
| Which tabs are open | The viewer's recent tabs come from `/v1/trace`, so closed tabs still show | `GET /v1/tabs` with each tab's id, URL, title and privacy, next to `/v1/tabs/{tabId}/private` |
| That a repeated `requestId` got the first run's stored reply | The CLI log keeps the code it sent, which can differ from what ran; prefer `Run.code` | `replayed: true` in the reply's `_meta["com.browseros.neo/run"]` |

## When something goes wrong

- **claw-sdk restarts in a loop with `setting the mode of /data/browserclaw/host.sock`.** A folder that Docker Desktop shares from macOS cannot hold a Unix socket. `compose.sdk-dev.yaml` moves the socket to `/tmp` in the container. If you start the container another way, set `CLAW_HOST_SOCKET` the same way.
- **The viewer says `4409 display_paused` and keeps retrying.** A private tab is in front. Close the tabs that went private, or wait for the takeover to end.
- **A run drives the wrong tab.** It had no `--task`. Pass one.
- **A port is taken.** Set `NEO_GATEWAY_PORT` (16090), `NEO_HOST_PORT` (19300) or `NEO_MCP_PORT` (19010) before `up`.
- **The gateway did not come up.** `up` prints the end of `~/.local/state/neo-dev/gateway.log`. The usual cause is that `npx` could not fetch Bun.

## Where the kit lives

All of it is dev only and stays out of the image:

- `dev.sh` builds and starts everything. Its header lists every command and setting.
- `compose.sdk-dev.yaml` adds the host API to the container: the token file, the TCP listener on 127.0.0.1:19300, and the socket moved off the shared folder.
- `devkit/gateway.ts` is the browser's way in. It adds the bearer token, which a browser cannot send on a WebSocket, refuses requests from any other origin, and serves the viewer, the trace UI and the CLI log.
- `devkit/viewer.html`, `devkit/trace.html` and `devkit/web/` are the two pages. The viewer uses the noVNC 1.3.0 that ships in the image.
- `dev_mcp.py run` sends scripts and writes the CLI log. `devkit/export.py` is `dev.sh export`.
- `devkit/examples/` holds the example scripts and `check.sh`.
