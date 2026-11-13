# Try claw-sdk by hand

This kit runs this checkout's Neo SDK image (claw-sdk) in Docker. You watch Neo's screen in a browser, point an MCP client at it, send scripts, and see for every operation the code that ran, what it was given, what came back, and a screenshot. It is for development only. The image and its entrypoint are the ones `build.sh` makes, and only the way `dev.sh` starts the container differs.

The same guide with screenshots of the viewer and the trace UI: https://docs.shadowfax92.workers.dev/9Hf3E3tN

## Quickstart

Run these from `packages/browseros-agent/crates/neo-sdk/neo-server/docker`, after the setup at the end of this section:

```sh
./dev.sh up
open http://127.0.0.1:16090/
open http://127.0.0.1:16090/trace
./dev.sh run --task demo -f devkit/examples/goto-title.js
./dev.sh down
```

1. `up` builds this checkout's Neo SDK image with `build.sh`, starts the container and the gateway, and prints every address. The first build takes about 7 minutes; later ones only what changed.
2. `http://127.0.0.1:16090/` is the viewer: Neo's whole screen, through claw-sdk's own display.
3. `http://127.0.0.1:16090/trace` is the trace UI. It lists every run and updates as runs happen.
4. `run` sends a script to claw-sdk's `run` tool, prints what came back, and logs the call.
5. `down` stops the container and the gateway. The browser profile and its logins stay.

`up` ends with this block:

```text
Viewer:   http://127.0.0.1:16090/   (?mode=control to type into it)
Trace UI: http://127.0.0.1:16090/trace
MCP:      http://127.0.0.1:19010/mcp
Host API: http://127.0.0.1:19300/v1   bearer token in /Users/<you>/.local/state/neo-sdk-dev/host-token

Claude Code:
  claude mcp add --transport http neo-sdk http://127.0.0.1:19010/mcp
```

After you change claw-sdk, run `./dev.sh up` again. It rebuilds only what changed and replaces the container and the gateway. To restart the gateway alone after editing `devkit/`, run `./dev.sh gateway`.

Setup, once: Docker, `brew install jq openssl`, `npx` (the gateway runs on Bun 1.4.2, which `npx` fetches), and `bun install` in `packages/browseros-agent`: the viewer page loads noVNC from the neo-viewer tool's `@novnc/novnc`, because the SDK image ships none. No Rust toolchain on the Mac: `build.sh` compiles the server inside Docker.

## Connect an MCP client

Paste the config that `up` prints into your MCP client, or run the `claude mcp add` line. claw-sdk serves one tool, `run`, whose input is `{ code, timeout? }`. `code` is the body of an async function that gets Playwright's `page`, `context`, `browser` and `expect`, plus `neo` and `console`.

A client can send its own ids in `_meta["com.browseros.neo/run"]` as `{ requestId, taskId }`. Always send a `taskId`. A run with one reuses that task's last tab as its `page`. A run without one drives the last tab any script touched, which can be another task's tab.

## Watch and drive the browser

The viewer opens in view mode. You see Neo's screen, and your keys and clicks go nowhere.

To type into Neo, pick **Control** or open `http://127.0.0.1:16090/?mode=control`. View displays and screenshots keep working while you type. Use a takeover when the agent must stop acting on the tab until you release it.


The status line says why a display ended:

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

### Read a snapshot, then act on a printed ref

`aria-ref.js` reads `page.ariaSnapshot()`, takes the ref printed for a button, and reads that element back through `page.getByRef(ref)` and `aria-ref=`.

`await locator.ref({ timeout: 1500 })` is Neo's own read. It waits for exactly one attached element, even without size, and returns its engine ref, an earlier minted ref, or a new `n<k>` ref with an `f<seq>` prefix in other documents. Two matches fail strictly; no match times out. `page.getByRef(ref)` keeps naming that node across runs until removal or navigation, and minting changes neither snapshot output nor action checks. Keep the ref with its task and use that task's page, because ref strings are tab-local and two tabs can each hold their own `n1`.

For a field printed without a ref, call `const ref = await field.ref()` on its role and exact-name locator, then check the attributes through `page.getByRef(ref)` so they belong to the node you keep. In a later run, `page.getByRef(ref).pressSequentially(text)` checks that the field takes focus before typing. A `click` or `fill` still waits for a visible target under Playwright's usual rules.

`page.ariaSnapshot()` (the same as `{ mode: 'ai' }`) lists what a script can act on, in Playwright's line syntax with Playwright's refs, and the page's text while the whole snapshot stays under about 3,000 tokens:

```text
- heading "Orders" [level=1]
- button "Save order" [ref=e3]
- button "Cancel" [ref=e4]
- iframe [ref=e5]:
  - text: Inside the frame
  - button "Frame action" [ref=f1e3]
```

On a bigger page its first line says how much text it left out. `{ text: true }` always includes the text and `{ text: false }` never does; `{ urls: true }` prints each link's url; `{ maxTokens: 20000 }` raises the 8,000-token cap. `page.getByRef('e5').ariaSnapshot()` reads one section. `{ mode: 'default' }` is Playwright's own default YAML, unchanged.

`page.getByRef('e12').ariaSnapshot({ mode: 'full' })` prints everything in one section as Playwright prints its `'ai'` snapshot: every element the engine keeps, all the text, each link's `/url`, and a ref on each visible element, including the ones the default list leaves without a ref (a plain `div` you want to hover). It carries Neo's clickable marks, so it prints every ref the default list prints. It obeys `maxTokens` (8,000 by default) with the same cut line, which offers a section in `{ mode: 'full' }`. A whole page that way is several times larger than the default list, and `text`, `urls` and `delta` are ignored.

What sits inside a closed shadow root (a Cloudflare Turnstile widget, a closed web component) prints and is acted on like any other element once a snapshot has read its page: `page.ariaSnapshot()` and `{ mode: 'full' }` find each closed root, and from then on every snapshot, locator and action in that document sees inside it. A locator used before any such snapshot of the document does not, so take a snapshot first. The page's own scripts still see the root as closed.

`page.ariaSnapshot({ delta: true })` returns only what changed since the last whole-page snapshot of the tab that a run gave you: one whose run ended ok with the snapshot (or its delta) inside a string it returned or a line it logged. A snapshot your script only inspected, or one from a run that failed, does not count, and neither does one taken earlier in the same run, or one whose page arrived after the call's timeout had passed (the call still returns it). Its first line says which case it is: `# whole page: no earlier snapshot of this page` (then the whole text), `# no change since your last snapshot of this tab`, `# whole page: too much differs from your last snapshot to list as changes` (then the whole text), `# whole page: no time was left to look for an earlier snapshot of this page` (then the whole text: the call's timeout passed before it could compare), or `# changes since your last snapshot of this tab: 1 added, 1 removed`, then the changes with three lines of context, each block headed by the section it is in:

```
@@ main [ref=e2]
 - main [ref=e2]:
   - heading "Cart" [level=2]
-  - button "Add to cart" [ref=e16]
+  - button "Remove from cart" [ref=e31]
   - link "Checkout" [ref=e17]
```

A new document in the tab (a navigation, a reload) starts over. A locator's snapshot, `{ mode: 'default' }` and `{ mode: 'full' }` ignore `delta` and never become the baseline. When a run takes a whole-page snapshot of a page you were already given, its reply ends with a one-line tip saying so, at most three times per task.

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

### Upload a file or a payload

`upload-file.js` uploads `/etc/os-release` from the container, then an in-memory text file. The page shows each file's name and first line. Allow the ordinary fixture before running it, then restore your prior folder list:

```sh
(
  prior=$(./dev.sh host GET /v1/settings | jq -c '.settings.files.readDirs')
  restore() {
    body=$(jq -cn --argjson dirs "$prior" '{files:{readDirs:$dirs}}')
    ./dev.sh host PATCH /v1/settings "$body" >/dev/null
  }
  trap restore EXIT
  body=$(jq -cn --argjson dirs "$prior" '{files:{readDirs:($dirs + ["/etc"] | unique)}}')
  ./dev.sh host PATCH /v1/settings "$body"
  ./dev.sh run --task howto-upload -f devkit/examples/upload-file.js
)
```

The result contains `uploaded` (starting with `os-release: `) and `payload: "note.txt: hello from Neo"`. Both `locator.setInputFiles` operations are delivered `done` in the trace. `devkit/examples/check.sh` makes the same temporary allowance and restores the host's prior list even after a failed example.

`files.readDirs` defaults to `["/data/downloads"]` in this image. A task can give its own `download.path()` straight to `setInputFiles`, or save a copy inside that folder and upload the copy. Owner files can arrive as `{ name, mimeType, buffer }` payloads, with no host staging folder. Payloads and clearing an input with `[]` need no path allowance.

Every path read resolves symlinks and must stay inside a listed folder. The list covers uploads, drops and script/style/init-script file paths. Neo always protects its state folder, browser profile, host token and CAPTCHA key files, even with `["/"]` listed. Directory uploads also refuse ancestors of those places. Download saves and screenshot paths cannot write to those protected places; `/tmp` remains writable. A refused path reports `script / invalid_input`, delivery `not_sent`, without repeating the denied path. Host settings changes apply to the next file operation, including inside a running script.

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

A takeover locks the agent out of the tab and gives the owner a control display bound to the takeover. The quickest way is the viewer's **Takeover** panel: enter the tab id, pick **Start takeover**, type into the page, then pick **Release**. From the shell:

```sh
./dev.sh host POST /v1/takeovers '{"tabId":1276814799}'
open 'http://127.0.0.1:16090/?mode=control&release=<takeover id>'
./dev.sh run --task howto "return await page.title()"           # refused: host/owner_has_control
./dev.sh host POST /v1/takeovers/<takeover id>/release
./dev.sh run --task howto "return await page.title()"           # "Example Domain"
./dev.sh run --task howto -f devkit/examples/close-page.js       # close the example tab
```

The takeover answers with the display to open:

```text
{"id":"tk_01a1acf4-f1fa-7751-a50a-47f6f6be4019","tabId":1276814799,"state":"active","lockId":"lk_01a1acf4-f1fa-7751-a50a-480a66e09075","display":"/v1/display?mode=control&release=tk_01a1acf4-f1fa-7751-a50a-47f6f6be4019",...}
```

Release closes that display (`4002 released`), waiting up to 2 s for Xvnc to read the owner's last
input, and lifts the lock. Only an answer with `inputDrained: true` guarantees that input landed; a
`false` after that wait means it does not hold for that release.

### Run them all

`devkit/examples/check.sh` runs every example, the lock and takeover flows, and the probes in the next section through `dev.sh run`. It checks each reply and Neo's own trace, prints `ok` or `FAIL` per step, and exits non-zero on a failure. It takes about 25 seconds.

## Read the trace

The trace UI joins two records of each run on its `requestId`:

- Neo's own trace, from the host API: the run's code and result, and every operation with its arguments, result, outcome, delivery (`not_sent`, `done` or `unknown`), error owner, duration and screenshot.
- The CLI log, `~/.local/state/neo-sdk-dev/traces/runs.jsonl`: the code `dev.sh run` sent, the MCP reply as the client saw it (console output included), and the round-trip time.

**Follow newest** selects each run as it starts. **Export JSON** downloads the selected run with its screenshots inlined and its CLI record. The list loads the newest 100 runs, and the filter searches only the runs it has loaded, so pick **Load older runs** to reach older ones.


To copy everything to disk, run `./dev.sh export`. It writes `~/.local/state/neo-sdk-dev/traces/export-<time>/` with `runs.json`, one `runs/<run id>.json` per run, `screenshots/<operation id>.jpg`, `events.json` (the events Neo still holds, up to 1024) and `cli-runs.jsonl`.

To call the host API yourself, use `./dev.sh host <METHOD> <path> [json]`, for example `./dev.sh host GET '/v1/runs?taskId=demo'`. It adds the bearer token, which never appears on the command line.

The settings are on the same API. `./dev.sh host GET /v1/settings` shows each one and its default, and `./dev.sh host PATCH /v1/settings '{"tabs":{"idleCloseSeconds":300}}'` changes one at once. The container keeps what you set across its restarts, under `NEO_DATA`; `./dev.sh host PUT /v1/settings '{}'` puts every setting back to its default.

Main-world evaluation (`page.evaluate` and element forms), `page.waitForFunction` and `page.setContent` are supported. The evaluate probe checks a successful result. `file:` navigation is refused before sending; it cannot bypass the file policy.

## What does not work yet

These are the supported probes and remaining gaps exercised by the kit. `devkit/examples/check.sh` re-checks every row, and most rows have a `probe-*.js` script you can run yourself. The requirements behind them are in the claw-sdk gap analysis (`claw-sdk/requirements/gap-analysis.md`, rows 5 to 9).

| Call | What you get today | Why |
|---|---|---|
| `page.bringToFront` | `script / script_error: page.bringToFront: Neo keeps agent tabs in the background; act on the page directly` | Refused on purpose, so the owner's screen never jumps. A person brings a tab forward with the viewer's **Follow** or a takeover. |
| Network: `page.waitForResponse`, `page.waitForRequest`, `page.on('request')`, `page.route` | `script / script_error: page.waitForResponse: Neo does not report network events; wait for what the page shows instead` (`route`: "network interception is not available in Neo") | Not built. The `Response` that `page.goto` returns works: `status()`, `ok()` and `url()`. |
| A tab parameter on `run` | `run` takes only `{ code, timeout }`, and refuses any other field | A run picks its tab through `_meta["com.browseros.neo/run"].taskId`: a task's runs share its last tab. Without a `taskId`, a run drives the last tab any script touched. |

`page.waitForTimeout` works, although the gap analysis lists it as missing from the op table. It is a timer in the SDK, not a browser operation: `waitForTimeout(1500)` returned after 1502 ms. It shows in the run's time, not as an operation in the trace.

`locator.clear` and `locator.setChecked` work too, although the gap analysis lists them as missing. The SDK runs them as `fill('')` and `check()` or `uncheck()`. `locator.screenshot` is an operation now. Screenshots keep working during owner input; Playwright's explicit `mask` option is supported.

The held calls report `script / method_unavailable`. The ones the SDK refuses itself report `script / script_error` with no delivery, because nothing reached the browser.

Use `./dev.sh host GET /v1/tabs` for every open tab's `tabId`, DevTools `targetId`, URL, title and touching `taskIds`. Trace rows are historical. `Operation.hasScreenshot` reports a stored image; missing or legacy withheld images return `404 not_found`, and retention returns `410 expired`.

## Gaps in Neo's trace

The trace UI needs a few things the host API does not record. Each row says what the kit does meanwhile and which field would close the gap in `crates/neo-sdk/neo-sdk-api/openapi.yaml`.

| Missing | Kit meanwhile | Field and place |
|---|---|---|
| The script's console output | Shows the CLI reply, which carries it | `Run.console`: the bounded lines, on `Run` |
| The code while a run is running (`Run.code` is stored when the run ends) | Shows the CLI log's `code` | `Run.code` from the start, on `Run` and `RunSummary` |
| An event when a run is complete and its screenshots are taken (every `run.ended` arrives with `complete: false`) | Re-reads `GET /v1/runs/<id>?wait=10` after `run.ended` | A `run.completed` value of `Event.type` |
| Which lock or takeover refused an operation | Shows the error message, which names the lock | `OperationError.lockId` and `OperationError.takeoverId` |
| The page Neo opens itself for a script's first call | Shows it as one of the script's own calls | `Operation.parentId` set to the call that needed the page |
| Which operations the script called, and which the SDK made for it (`page.frames` is about a quarter of all operations) | Shows them all alike | `Operation.origin` (`agent`, `prelude` or `door`) |
| Which line of the script made an operation | Nothing: a position appears only inside a script error's text, such as `(run:1:253)` | `Operation.callsite` (`{line, column}` into `Run.code`) |
| That a repeated `requestId` got the first run's stored reply | The CLI log keeps the code it sent, which can differ from what ran; prefer `Run.code` | `replayed: true` in the reply's `_meta["com.browseros.neo/run"]` |

## When something goes wrong

- **claw-sdk restarts in a loop with `setting the mode of <path>/host.sock`.** A folder that Docker Desktop shares from macOS cannot hold a Unix socket. The image's default socket, `/run/neo/host.sock`, is off the data folder, so this happens only when `CLAW_HOST_SOCKET` points into a shared folder. `compose.dev.yaml` keeps it in the container's `/tmp`.
- **A run drives the wrong tab.** It had no `--task`. Pass one.
- **A port is taken.** Set `NEO_GATEWAY_PORT` (16090), `NEO_HOST_PORT` (19300) or `NEO_MCP_PORT` (19010) before `up`.
- **The gateway did not come up.** `up` prints the end of `~/.local/state/neo-sdk-dev/gateway.log`. The usual cause is that `npx` could not fetch Bun.

## Where the kit lives

All of it is dev only and stays out of the image:

- `dev.sh` builds and starts everything. Its header lists every command and setting.
- `compose.dev.yaml` adds the host API to the container: the token file, the TCP listener on 127.0.0.1:19300, and the socket moved off the shared folder.
- `devkit/gateway.ts` is the browser's way in. It adds the bearer token, which a browser cannot send on a WebSocket, refuses requests from any other origin, and serves the viewer, the trace UI and the CLI log.
- `devkit/viewer.html`, `devkit/trace.html` and `devkit/web/` are the two pages. The viewer uses the noVNC 1.3.0 that ships in the image.
- `dev_mcp.py run` sends scripts and writes the CLI log. `devkit/export.py` is `dev.sh export`.
- `devkit/examples/` holds the example scripts and `check.sh`.

### Omit values from the trace

Use the host API before sending a script:

```sh
./dev.sh host PATCH /v1/settings '{"trace":{"values":"off","screenshots":"off"}}'
```

Neo then omits script text, arguments, results, notes, host data, stored replies and error
messages when records end. The records keep metadata and `values: "off"`. Replies still carry
the result. Fetch large result blobs within ten minutes; downloads stay until trace retention.
The viewer explains the omission, while its independent proxy log still keeps what it saw.
Restore ordinary recording with `{"trace":{"values":"on","screenshots":"perOperation"}}`.
