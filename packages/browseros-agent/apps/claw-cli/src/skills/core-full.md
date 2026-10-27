# browseros: the complete reference

Everything the CLI can do, every flag, and an honest list of what it cannot.
For the short version, use `browseros skills get core`.

## The model

One command is one tool call against claw-server's MCP endpoint. The CLI owns
no browser logic: the server is the daemon, the CDP client, the snapshot
builder and the ref resolver.

Three things are remembered per directory so commands carry no plumbing: the
current page, the MCP session id and the tab group id. They live in
`<browserclaw_dir>/cli/<scope>.json`, where the scope is the git worktree root
and `<browserclaw_dir>` is the same state directory the server uses, so a dev
server and a release server never share a session.

`BROWSEROS_SESSION=<name>` pins an explicit scope. `--page` overrides the
remembered page for one command. `browseros use <id>` changes it.

## Global flags

```text
--page <id>          act on this page instead of the remembered one
--endpoint <url>     the server, otherwise resolved and health checked
--agent <name>       the product driving this CLI, for tab labelling
--json               wrap the result in a JSON envelope on stdout
--quiet, -q          suppress the CLI's own notices on stderr
--client-timeout <ms>  client read timeout, default 180000
--no-auto-session    fail instead of pairing a session automatically
```

`--agent` is also read from `BROWSEROS_AGENT`, which is how a harness sets it
once. It accepts `claude`, `codex`, `cursor`, `opencode`, `antigravity`,
`vscode`, `zed`, or another product's short name, and decides the prefix on the
tab group the user sees.

## Endpoint resolution

Each candidate is probed with `GET /system/health` before it is used, in this
order:

```text
1  --endpoint <url>
2  $BROWSEROS_URL
3  the endpoint cached in the scope file
4  $BROWSERCLAW_DIR/runtime.json
5  ~/.browserclaw/runtime.json           (release)
6  ~/.browserclaw-dev/runtime.json       (dev)
7  http://127.0.0.1:9200                 (the compiled default)
```

The server writes `runtime.json` at startup and never cleans it up, so a stale
one is normal rather than a fault. If both a release and a dev server answer,
the release one is used and the CLI says so once.

## Every command

### Session and state

```bash
browseros status                     # endpoint, link, session, page, optional commands
browseros session start [--task "..."] [--category C] [--summary S]
browseros session status
browseros session list               # every scope file on this machine
browseros session name "<task>" [--category C] [--summary S]
browseros session end [--close-tabs] [--all]
browseros use <page>
```

`session start` performs the handshake once, lists tabs (which is how a group
left behind by a previous session for this directory is adopted rather than a
second one started), and names the task. Pairing is otherwise implicit: any
command with no usable session pairs itself and says so on stderr.

`session end` sends `DELETE /mcp` and clears the scope file. It deliberately
leaves tabs open; `--close-tabs` closes the task's tab group as well.

### Tabs, groups, windows

```bash
browseros tab list | tab new [url] | tab close [page] | tab active
browseros group list
browseros group create <page...> [--title T] [--group ID]
browseros group update <id> [--title T] [--color C] [--collapsed true|false]
browseros group ungroup <page...>
browseros group close <id>
browseros window list | window create | window close <id>
```

Group colors: grey, blue, red, yellow, green, pink, purple, cyan, orange.

There is no `window activate` and there will not be: the tool omits it so a
caller cannot steal the user's focus. New tabs are always background for the
same reason.

### Profiles (only where the native supports it)

```bash
browseros profile list
browseros profile load [name] [--profile-id ID]
browseros target get
browseros target set [name] [--profile-id ID]
browseros target set-window <id> [--strict]
browseros target clear-window | target clear
```

With no selection every call uses the browser's original Default profile.
Selection affects later calls; changing it inside a `run` script is refused.

### Navigating

```bash
browseros open <url> [--keep-page]
browseros goto <url>
browseros back | forward | reload
browseros history [--max N]
```

`open` is one call, not two: `tabs action=new` navigates and returns once the
page has stopped changing. It remembers the new page unless `--keep-page`.

### Seeing

```bash
browseros snapshot [-i] [--depth N]
browseros diff
browseros read [--selector CSS] [--text|--links|--console]
             [--viewport] [--include-links] [--include-images]
browseros grep <pattern> [--over ax|content] [--limit N]
browseros screenshot [path] [--full] [--annotate] [--format jpeg|png|webp] [--quality N]
browseros pdf [path] [--landscape] [--no-background]
```

`--depth` is clamped to 1..=100. `--quality` is 0..=100. A screenshot comes back
inline as an image, which a terminal cannot render, so the CLI writes it to the
path given, or to `~/.browseros/tool-output/`, and prints where it went.

### Acting

Every verb below is one `act` call with the kind set by the verb.

```bash
browseros click [@ref] [--at X,Y] [--button left|middle|right] [--count N]
browseros fill @ref "value" [--append] [--sensitive]
browseros fill @ref --value-stdin | --value-env VAR
browseros fill --fields @e1="a" @e2="b" [--append]
browseros type "text" [--value-stdin] [--value-env VAR] [--sensitive]
browseros press <key>                # Enter, Tab, Control+a, ...
browseros hover [@ref] [--at X,Y]
browseros focus @ref
browseros check @ref | uncheck @ref
browseros select @ref "Option"
browseros scroll [up|down|left|right] [amount] [@ref]
browseros drag @from @to
browseros dialog accept | dialog dismiss
browseros upload @ref <file...>
browseros download @ref
```

Every acting verb takes `--diff none|summary|full|<N>`, which caps the automatic
readback. Use it on a large page, where a full diff floods context.

An action that opens a JavaScript dialog comes back saying so, and the page
stays blocked until the dialog is accepted or dismissed, so do that before
anything else on the page.

### Waiting

```bash
browseros wait --text "..."    [--timeout MS]
browseros wait --selector "..." [--timeout MS]
browseros wait --time MS
```

`--timeout` is capped at 30000 for text and selector. `--time` is honored up to
90000 and a larger value is rejected rather than shortened, so the CLI refuses
it locally. A text or selector wait that times out returns `matched: false`
rather than failing, so read the result.

### Scripting

```bash
browseros run '<js>'
browseros run -f script.js
browseros run - <<'JS' … JS
browseros run ... [--no-page] [--script-timeout MS]

browseros eval '<js>' [--settle] [--max-chars N] [--script-timeout MS]
browseros eval -f snippet.js

browseros helpers list
browseros helpers read <name>
browseros helpers save <name> -f helper.js
```

`run` executes against the browser SDK in the server runtime. `eval` runs inside
the page. `--max-chars` is capped at 200000 and counts UTF-8 bytes rather than
characters.

The `run` runtime is a bare engine, neither Node nor the page. It has `browser`,
`console`, `setTimeout`/`clearTimeout` and `sleep`, and nothing else: no
`fetch`, `require`, `process`, `window`, `document` or `localStorage`. Reach
into the page through `page.evaluate`, which runs there.

Helpers are saved per host and are hot-loaded as `helpers.<name>(browser, page)`
where page is the id number. They are only callable inside `run`, even though
the roster is advertised in other tools' output.

### Skills, humans and the decision loop

```bash
browseros skill save <name> --description "..." [--steps S] [--site X] [--learned N]
browseros skill ran <name>
browseros help request "<reason>" [--kind login|captcha|approval|other]
                                  [--details D] [--resume-hint R]
browseros help wait [<id>]
browseros pursue "<goal>" [--value name=value]... [--destination label=url]...
```

`help request` blocks for a while and returns a status. If it says `waiting`,
call `help wait` again and do nothing else on the page until it says `resolved`.
This is the right answer for a sign-in, a one-time code, a captcha or an
approval: a human does it in the real browser.

`pursue` takes a whole outcome and decides every step on the server. It is
offered only where the server holds a decision-model credential; `browseros
status` says whether this one does. Never put a secret in `--value`: the text is
typed into the page and recorded with the call like any other argument.

## Exit codes

```text
0    success
1    the tool ran and reported a failure, including a refusal this server makes
2    usage error: bad flag, unknown command, malformed ref, out-of-range value
3    no browser: nothing answering any endpoint, or the browser link is down
4    timeout, including a run killed at the 30 second cap
5    a human pressed Stop in the cockpit. Do not retry.
130  interrupted. The CLI tells the server to stop the call before exiting.
```

Exit 5 exists because the server distinguishes a client cancellation from a
human pressing Stop, and only one of those is worth retrying.

## Output

Default output is the tool's text block, verbatim, because that text is already
written for an agent to read. `--json` wraps it:

```json
{ "ok": true, "command": "click", "tool": "act", "page": 3,
  "text": "...", "structured": { }, "elapsedMs": 341 }
```

The CLI's own notices go to stderr so stdout stays pipeable. `--quiet` silences
them. Colour is off when output is piped and whenever `NO_COLOR` is set.

## Gaps versus agent-browser

These exist in agent-browser and do not exist here. An agent that assumes
otherwise wastes turns, so the list is explicit. `browseros <name>` for any of
them prints the nearest working command.

| Missing | Nearest thing here |
|---|---|
| cookies | `browseros run "return await page.cdp('Network.getCookies')"` |
| localStorage and sessionStorage | `browseros eval "return JSON.stringify(localStorage)"` |
| saved auth state, the auth vault | nothing: neo drives the user's real profile, already signed in |
| network interception, HAR, request inspection | call the site's own API from inside `eval` |
| route, abort, waitForRequest, waitForResponse | nothing |
| device and viewport emulation | `setViewport` exists only on a hosted server |
| geolocation, offline, extra headers, color scheme | nothing |
| locators by role, label, placeholder, testid | `snapshot -i`, then the `[ref=eN]` handle |
| `get html\|value\|attr\|box\|styles\|count` | `eval` |
| pixel screenshot diff, url-to-url diff | `diff` compares accessibility trees, not pixels |
| video, trace, profiler | the cockpit already records sessions |
| React devtools, WebMCP | nothing |
| frame scoping | iframe content is stitched into the snapshot inline |
| clipboard | `eval "return await navigator.clipboard.readText()"` |
| highlight | `screenshot --annotate` |
| iOS Safari, cloud browser providers | nothing |
| `batch` | `run`, which is strictly better |

And the other direction, which is the reason this exists at all: `run` with a
page-object SDK and `Promise.all`, self-healing per-host helpers, server-side
`grep`, an action readback that needs no follow-up call, readiness that is
handled rather than waited on, saved skills, human handoff, tab groups and
ownership, an audit trail, `pursue`, and a browser that is already signed in.
