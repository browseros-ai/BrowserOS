# zero-conflict sites

The site half of the zero-conflict fuzzer (APC engine design, section 10.3). An agent shown
`button "Pay" [ref=e5]` writes `getByRole('button', { name: 'Pay', exact: true })`, and that query
must reach the element `e5` names. The offline gate (`neo-snapshot-apc/tests/zero_conflict.rs`)
proves it for the recorded fixtures and generated trees. This runner proves it on real pages.

`run.sh` loads every URL of `sites.txt`, one tab at a time, in a fresh headless BrowserOS that
carries the APC contract. For each site the live test
`neo-snapshot-apc/tests/zero_conflict_live.rs` (`site_lines_reach_their_elements`) does this:

1. It waits for `document.readyState === 'complete'`, at most 30 s, then 3 s more.
2. It reads the page's oracle, Playwright's vendored engine run in each frame, within 20 s a
   step and 5 minutes in all. When the oracle fails, the site's recall is not measured and the
   rest still is. An oracle step that overran its bound leaves the engine running in the page,
   so the runner stops that script (`Runtime.terminateExecution`) before it captures.
3. It captures `Page.getAnnotatedPageContent` twice and checks every line with a ref in Neo's
   format and in Playwright's.
4. It hydrates every printed ref in its frame's CDP session.
5. It compares the tree's role sets with the oracle's (recall).

The gate is 99.5% of role lines over the measured sites. A role line is a line with a ref that
is not one of the snapshot's own labels (`generic`, `iframe`). Stability, hydration and recall
are reported and not gated.

## Running it

From `packages/browseros-agent`, on this machine (the script finds its own way there):

```sh
crates/neo-sdk/tools/zero-conflict/run.sh \
  --browser "/Users/shadowfax/code/chromium-1/src/out/Default_browseros_arm64/BrowserOS Dev.app/Contents/MacOS/BrowserOS Dev"
```

| Flag | Default |
| --- | --- |
| `--sites FILE` | `sites.txt` beside the script |
| `--out DIR` | `$HOME/.neo-zero-conflict/<UTC timestamp>` |
| `--browser PATH` | the launcher's own default (`NEO_E2E_BROWSER`) |

Cargo and the toolchain come from your environment. A run needs the network.

The test runs optimized (`cargo test --release`). Each printed line runs its own query over its
whole document, so the check grows with lines times nodes. On 2026-11-25, against a debug
BrowserOS build, the 50 sites took 62 minutes. The Hacker News discussion page alone took 23
of them: 87,049 role lines, each checked in both captures.

## Where the private data goes

Every measured site writes these files to the private folder:

| File | What it holds |
| --- | --- |
| `<id>.pb` | Capture #1's protobuf bytes, checked through a fresh ref table |
| `<id>.2.pb` | Capture #2's bytes, for a line only the second capture missed |
| `<id>.oracle.json` | The page's oracle, when it could be read |
| `report.md` | Every miss with its line text and query, each difference between the captures, each hydration failure, and recall per frame and role |

A site's id is its 1-based position in the list and its host, such as `07-mui.com`.

These files are real pages' text, cookie banners and account names included. They are never
committed and never sent to the remote pool. The remote client uploads `packages/browseros-agent`
as the working tree has it, so `run.sh` refuses a folder inside this worktree or inside the main
checkout it belongs to. The test refuses one too when it is run without `run.sh`. Standard
output holds no page text. It shows a row per site and the totals, and lists every miss by site
and line number with its role and verdict kind. Use the line number to find the miss in
`report.md`.

## Replaying a miss offline

Capture #1 replays without the browser or the network. From `packages/browseros-agent`:

```sh
NEO_ZERO_CONFLICT_REPLAY="$HOME/.neo-zero-conflict/<run>" \
  cargo test --release -p neo-snapshot-apc --test zero_conflict_live \
  private_site_captures_replay_offline -- --nocapture
```

It checks each `<id>.pb` with `check_apc`, the offline gate's entry point. It prints the same
counts and miss lines as the live row for capture #1.

## The list

`sites.txt` holds 50 public pages that load without signing in. It covers the plan's nine spike
pages and then checkouts, data grids, editors, forms, single-page apps, cookie banners, nested
frames and shadow DOM. One URL per line; blank lines and `#` lines are skipped. A site that
fails to load is listed as not measured, with its reason, and the run goes on. A site still
loading after 30 s is measured as it stands, and its row names the state it was in.
