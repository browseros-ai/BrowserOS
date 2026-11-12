# CAPTCHA test bed

End-to-end checks of neo-captcha through the Neo SDK: neo-server's `browseros-claw-sdk`, its one
MCP tool `run`, and `neo.solveCaptcha(page)`.

- `fixtures/`: public test widgets, the page catalog, and the fixture server that checks each
  submission with the vendor itself.
- `e2e/`: the runner, its preflight, the SDK page driver and a local stub endpoint.
- `docker/`: the Neo SDK image in an isolated container ([its README](docker/README.md)).
- `provider-check/`: provider key and billing diagnostics.

From `packages/browseros-agent`:

```sh
bun run test:captcha-e2e -- --mcp-url http://127.0.0.1:<port>/mcp --pages all   # a running Neo SDK
bun run test:captcha-e2e -- --dev-watch --pages fixtures                        # this worktree's SDK
bun run test:captcha-docker-e2e -- --keys placeholder                           # the SDK image
bun test crates/neo-sdk/neo-captcha/testbed                                     # unit tests
```

Without `--mcp-url` or `--dev-watch`, set `BROWSEROS_BINARY`: the runner builds
`cargo build -p neo-server --bin browseros-claw-sdk` (or runs `CLAW_SDK_BINARY`), launches the
browser with a fresh profile, and starts the server against it with private state.
`--dev-watch` runs `bun run dev:claw:watch:new` with `BROWSERCLAW_USE_SDK=1`, random ports, a fresh
profile and `BROWSERCLAW_DIR`; it refuses the owner's ports (9010, 9011, 9121, 9211–9215) and inherits
`CARGO_TARGET_DIR` (default `/Volumes/extssd/cargo-target/captcha-e2e-dev-watch`). Allow ten
minutes for a cold build, or set `--startup-timeout`.

## Pages

`--pages fixtures` (default) runs the eight local fixtures and the eleven fake-key GeeTest, AWS WAF
and FunCaptcha pages, which include cross-site frames and closed shadow roots; `spotting` runs only
those eleven; `demo` runs the eight public demos; `all` runs all 27. `--only id,id` narrows any
group. Fixtures use two listeners on different sites, `127.0.0.1:A` for top pages and `localhost:B`
for embedded widgets; `--fixture-frame-host` changes the frame host. See
[`fixtures/README.md`](fixtures/README.md).

Each page gets its own task (`neo.createTask`) and runs as separate `run` calls: open the tab, go
to the page, wait for the widget (`page.waitForFunction`), `neo.solveCaptcha(page)`, read the token
length, click submit, wait for the site's own verdict, and close the task. Each script starts with a
`/* captcha-e2e:<step> {…} */` tag, which the stub reads instead of running JavaScript.

## Outcomes

A result keeps the SDK's `verdict`, `type`, `reason`, `message`, `durationMs` and `paidTasks` exactly
as returned, and `site`: what the page's own server said about the submission. Only token lengths
are kept, never tokens.

- A page lists the verdicts that pass (`expect`), and may pin the type, the reason and a paid-task
  cap. `unsolved` is the SDK's verdict for every definite failure: hCaptcha and the Cloudflare wall
  pass only as `unsolved` / `unsupported_type` with the right type.
- `solved`, and `already_clear` on a CAPTCHA page, pass only if the site accepted the submission.
  `token_applied` passes a `solved` page only if the site accepted. A site that accepts a page the
  solver called `unsolved` fails it.
- The `Check` column says what a pass proves: `solve` (the site accepted), `deliver` (a token with
  no site check, as on the Arkose demo; never counted as solved) or `detect` (the right type, no
  solve claimed). The summary counts site-accepted submissions, deliveries, detections and paid tasks.
- A page that may need a paid solve is UNVERIFIED instead of FAIL when it ends `unsolved` or
  `unknown` without `--paid`.

## Preflight and keys

Before any page, the runner requires `/system/health` ok with the browser connected, an endpoint
whose only tool is `run` (a claw-server endpoint is refused), and `neo.solveCaptcha` listed in the
`run` description. A Neo without a readable provider key hides the method, and every call fails
`host/captcha_unavailable`: that is a **setup blocker**, exit 2, with no page run. To prove the
absence instead, pass `--expect-tool-missing`: the method must be unlisted and every page's call
must fail `captcha_unavailable`.

`--keys` sets the keys of a server the runner starts. `placeholder` (default) writes one fake
CapSolver key and clears inherited key variables, so nothing is charged. `real` uses
`BROWSEROS_CAPSOLVER_API_KEY_FILE` and `BROWSEROS_2CAPTCHA_API_KEY_FILE`, else the files in
`~/.config/browseros/captcha/`, without reading them, and caps the server at 12 paid tasks
(`BROWSEROS_CAPTCHA_PAID_TASK_LIMIT`; change it with `--paid-task-limit`). `none` clears every key.
CapSolver and 2Captcha are the SDK's only providers. `--paid` makes paid paths required. An
`--mcp-url` endpoint owns its keys and budget.

## Output

`--out DIR` (default a temporary directory) receives `preflight.json`, `results.json` (one record
per page) and `results.md`. Each finished page also prints one line. Exit codes: 0 when nothing
failed, 1 on a FAIL or an unexpected solver during `--expect-tool-missing`, 2 for a setup blocker or
a usage error.

## claw-server

The production Neo image still runs claw-server, whose release check (`release-docker.yml`) passes
`--server claw-server --mcp-url <url>`. That driver uses claw-server's `tabs`, `evaluate_in_page` and
`solve_captcha`; the catalog and rules are the same, and its `status` is kept beside the verdict
neo-captcha maps it to. The runner only ever launches the SDK.

For a local proof, `bun crates/neo-sdk/neo-captcha/testbed/e2e/stub-mcp.ts --scenario contract`
prints a stub URL; pass it with `--mcp-url` and `--fixture-origin http://fixtures.invalid`.
