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

`--pages fixtures` (default) runs the fourteen local fixtures (including three stale-token pages)
and the eleven fake-key GeeTest, AWS WAF and FunCaptcha pages, which include cross-site frames and
closed shadow roots; `spotting` runs only those eleven; `demo` runs the thirteen public demos;
`all` runs all 38; `owner` runs the owner's six-site check (`neo-server/docker/CLAUDE.md`) in its
order, with invisible reCAPTCHA twice: untouched, and after its own submit has shown the image
challenge. `--only id,id` narrows any group. Fixtures use two listeners on different sites,
`127.0.0.1:A` for top pages and `localhost:B` for embedded widgets; `--fixture-frame-host` changes
the frame host. See [`fixtures/README.md`](fixtures/README.md).

Each page gets its own task (`neo.createTask`) and runs as separate `run` calls: open the tab, go
to the page, wait for the widget (`page.waitForFunction`), `neo.solveCaptcha(page)`, read the token
length, click the submit button inside the page (as claw-server's runner did, so a challenge frame
left over the button cannot swallow the click), wait for the site's own verdict, and close the task. Each script starts with a
`/* captcha-e2e:<step> {…} */` tag, which the stub reads instead of running JavaScript.

A page may click something first (`trigger`), as a visitor would, and wait for the state it
tests; if that state never comes (say, Google shows no challenge to a trusted IP), a failing page
is UNVERIFIED. A page may also record one value after the site check (`reading`), such as the
v3 demo's score, Turnstile's siteverify answer, or the title once the Cloudflare page lets the
browser through; the reading is printed in the note and never decides the outcome. The site check
clicks nothing when the page has no submit control or already shows its acceptance (an
invisible reCAPTCHA callback submits the form itself).

## Outcomes

`neo.solveCaptcha(page)` returns `{ attempted, type, method, reason, message, durationMs,
paidTasks, provider }` — no `verdict`. A value that still carries a `verdict` key is rejected as
malformed (`neo.solveCaptcha returned a verdict`). The test bed keeps `verdict` as its own local
classification of the SDK's `reason` (or claw-server's `status`), alongside `attempted`, `method`,
and `site` (what the page's own server said about the submission). `results.json`, `results.md`
and the one-line progress output record `attempted` and `method`. Only token lengths are kept,
never tokens.

- A page lists the test-bed verdicts that pass (`expect`), and may pin the type, the reason,
  `attempted` (`expectAttempted`, ignored for claw-server where `attempted` is `null`), and a
  paid-task cap. `unsolved` is the test bed's classification for every definite failure reason, and
  only `reason` keeps claw-server's `unsupported` apart: `unsolved` / `unsupported_type` fails, and
  is never UNVERIFIED, unless the page allows it. hCaptcha and PerimeterX's check pass only that
  way, with the right type and `attempted: false` (PerimeterX's is `unknown` on purpose:
  claw-server's closed `type` enum does not list it); the spotting pages accept it beside any other
  `unsolved`.
- Neo always solves a visible widget fresh instead of answering `already_passed` when a response
  token is already present. On Cloudflare's always-pass test key (`turnstile-pass`,
  `cloudflare-turnstile-demo`), `neo.solveCaptcha` usually lands before the widget's token and
  passes free (`auto_passed`, `mayNeedPaid: false`, `expectAttempted: true`). When the widget wins
  the race, Neo asks a paid provider, but both providers refuse that test key (`provider_error`, 0
  paid tasks) while the page's own token still passes the site; `providerRefusesKey: true` marks
  that `UNVERIFIED` regardless of `--paid` instead of treating it as a solver contradiction. Three
  stale-token fixtures (`turnstile-stale-interactive`, `turnstile-stale`, `recaptcha-v2-stale`)
  pre-populate an obvious fake token that the fixture server rejects (`REJECTED: stale-token`), so
  a pass proves Neo wrote a replacement token the vendor accepted: the interactive Turnstile and
  reCAPTCHA pages replace it for free via checkbox click, while `turnstile-stale` is a static
  failed-widget box on Turnstile's interactive test key (`3x00000000000000000000FF`, which 2Captcha
  mints) so Neo must mint a paid token (`mayNeedPaid: true`).
- Neo waits up to 10 seconds for Cloudflare's full-page check. It classifies `auto_passed` as
  `solved` when the replacement page has cleared and carries the site's clearance cookie. A
  persistent wall returns `challenge_not_passed` (`unsolved`, `attempted: true`), which the static
  wall fixture expects. The ScrapingCourse demo passes as `solved` or `already_clear` once the page
  itself says the browser got through.
- `solved`, and `already_clear` on a CAPTCHA page, pass only if the site accepted the submission.
  `token_applied` (`awaiting_submit`) passes a `solved` page only if the site accepted (and any
  pinned `expectReason` / `expectAttempted` hold). A site that accepts a page classified as
  `unsolved` fails it.
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
per page) and `results.md`. Each finished page also prints one line. The summary's paid tasks count
what each solve reported; a solve that failed may still have paid, so it is listed as a result
without a count. `--solve-timeout` (default 150000) is each solve's `run` budget, at most the SDK's
900000; under the solver's own 120000 it can cut a solve short. Exit codes: 0 when nothing failed,
1 on a FAIL or an unexpected solver during `--expect-tool-missing`, 2 for a setup blocker or a usage
error.

## claw-server

The production Neo image still runs claw-server, whose release check (`release-docker.yml`) passes
`--server claw-server --mcp-url <url>`. That driver uses claw-server's `tabs`, `evaluate_in_page` and
`solve_captcha`; the catalog and rules are the same, claw-server's `status` is kept beside the test
bed's local `verdict` classification (`STATUS_VERDICT`), and `attempted` is `null` (so
`expectAttempted` is ignored). The runner only ever launches the SDK.

For a local proof, `bun crates/neo-sdk/neo-captcha/testbed/e2e/stub-mcp.ts --scenario contract`
prints a stub URL; pass it with `--mcp-url` and `--fixture-origin http://fixtures.invalid`.
