# browseros: the complete reference

Everything the CLI can do, every flag, and an honest list of what it cannot.
For the short version, use `browseros guide core`.

## The model

One command is one stateless tool call against claw-server's MCP endpoint. The
CLI owns no browser logic: the server is the daemon, the CDP client, the
snapshot builder and the ref resolver.

There is no session. A task is the tab group it works in: the first page opens
the group, every command presents it, and the server resolves the task from it
and names the group back in each result. Two things are remembered per
directory so commands carry no plumbing: the current page and the task's tab
group. They live in `<browserclaw_dir>/cli/<scope>.json`, where the scope is the
git worktree root and `<browserclaw_dir>` is the same state directory the
server uses, so a dev server and a release server never share a task.

`BROWSEROS_TASK=<name>` pins an explicit scope, so two agents in one checkout
can each work on a task of their own. `--page` overrides the remembered page for
one command. `browseros use <id>` changes it.

## Global flags

```text
--page <id>          act on this page instead of the remembered one
--endpoint <url>     the server, otherwise resolved and health checked
--agent <name>       the product driving this CLI, for tab labelling
--json               wrap the result in a JSON envelope on stdout
--quiet, -q          suppress the CLI's own notices on stderr
--client-timeout <ms>  client read timeout, default 180000
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

### Tasks and state

```bash
browseros status                     # endpoint, link, task, page, optional commands
browseros task start [--name "..."] [--category C] [--summary S]
browseros task status
browseros task list                  # every directory on this machine with a task
browseros task name "<task>" [--category C] [--summary S]
browseros task done [--keep-tabs] [--all]
browseros use <page>
```

A task starts with its first page, which is where the server opens its tab
group. `task start` only names it ahead of time; the name is applied once the
group exists. Every later command joins the task by presenting its group, and
a command run in a new process, or after the server restarts, lands in the same
task and the same history row.

`task done` is for when the user says the task is done. It closes the tab group,
which ends the task on the server and clears the scope file. `--keep-tabs` takes
the pages out of the group instead, which ends the task the same way and leaves
them open. If the user closes the group's tabs themselves, the task ends too,
and the CLI notices on the next command. A task is never ended by going quiet,
however long the agent waits between commands.

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
path given, or to the server's output folder, and prints where it went.
`browseros status` shows that folder on its `outputs` line; saved outputs too
large to inline go there as well.

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
`fetch`, `require`, `process`, `window`, `document` or `localStorage`. Read a
page with `page.snapshot`, `page.read` and `page.grep`. `page.evaluateInPage`
runs code inside the page itself, where the page can watch it, so use it only
when the user asks for that.

Helpers are saved per host and are hot-loaded as `helpers.<name>(browser, page)`
where page is the id number. They are only callable inside `run`, even though
the roster is advertised in other tools' output.

### Large results, and saved flows

A call whose result exceeds the server's inline budget saves the whole thing
and names an output id. Read it back without re-running the call:

```bash
browseros output <id>                     # list the parts
browseros output <id> --part read.txt     # the whole part, read to its end
browseros output <id> --part read.txt --offset 16384 --limit 65536   # one slice
```

Part names carry their extension, such as `result.json` or `read.txt`; list
the parts to see them. Asking for a slice prints, on stderr, how much is left
and the command for the next one.

The server's own advisory for this names the `read_output` tool, because it is
written for MCP callers. `browseros output` is the same thing.

```bash
browseros skill run <name> --inputs '{"query":"ram"}'
browseros skill run <name> --inputs -          # JSON on stdin
```

`skill run` executes a saved skill's verified flow and is offered only where
the server runs them; `browseros status` says whether it is.

### Saved passwords and cards, by reference only

```bash
browseros credentials list [--query Q] [--limit N] [--all] [--cursor C]
browseros credentials fill --id <credentialId>
browseros cards list
browseros cards fill --id <cardId>
```

`credentials list` matches the current page's site; `--all` lists the whole
profile. Listings never contain a password, a card number or a security code.
Filling acknowledges the fields and does not submit, confirm a login, or pay,
and a saved card keeps the browser's own payment checks, so it may report that
the user has to finish in the browser.

The tools behind these also support adding an entry and filling a supplied
username, password or card number. **The CLI does not expose those, and will
not.** They are the only ways to get a secret onto a command line, where it
lands in shell history and in `ps` output. A human saves credentials in the
browser; the CLI uses what is already there. For a secret that is not saved,
use `human request`.

### Skills, humans and the decision loop

```bash
browseros skill save <name> --description "..." [--steps S] [--site X] [--learned N]
browseros skill ran <name>
browseros human request "<reason>" [--kind login|captcha|approval|other]
                                   [--details D] [--resume-hint R]
browseros human wait [<id>]
browseros pursue "<goal>" [--value name=value]... [--destination label=url]...
```

`human request` blocks for a while and returns a status. If it says `waiting`,
call `human wait` again and do nothing else on the page until it says `resolved`.
This is the right answer for a sign-in, a one-time code, a captcha or an
approval: a human does it in the real browser.

`pursue` takes a whole outcome and decides every step on the server. It is
offered only where the server holds a decision-model credential; `browseros
status` says whether this one does. Never put a secret in `--value`: the text is
typed into the page and recorded with the call like any other argument.

## Exit codes

```text
0    success
1    the tool ran and reported a failure, including a page that did not load and a refusal this server makes
2    usage error: bad flag, unknown command, malformed ref, out-of-range value
3    no browser: nothing answering any endpoint, or the browser link is down
4    timeout, including a run killed at the server's wall-clock cap
5    a human pressed Stop in the cockpit. Do not retry.
6    this computer does not have the ultrafast plan, which the CLI is part of
130  interrupted. Abandoning the request stops the call on the server.
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
| saved auth state, the auth vault | not needed: neo drives the user's real profile, and `credentials list` / `credentials fill --id` use its saved passwords by reference |
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
