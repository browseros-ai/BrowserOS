# bpatch

## What It Is

`bpatch` manages the BrowserOS Chromium patch store with one model: a checkout's patched state is `base + store`. `apply` converges a checkout to that state with minimal file writes, preserving unchanged file mtimes so incremental Chromium builds survive ordinary patch updates. `extract` folds checkout commits back into net per-file diffs against the store base, so create/delete and modify/revert history noise does not become store state. Checkout state lives in git commit trailers; there is no long-lived `.bpatch` state file.

The tool is built for three operators: a human doing the daily loop, cron applying unattended store updates, and an agent walking a Chromium base upgrade through conflicts and repin.

## Quick Start

Configure the store once:

```console
$ bpatch init /Users/shadowfax/code/browseros-project/packages/browseros/chromium_patches
initialized store /Users/shadowfax/code/browseros-project/packages/browseros/chromium_patches
config      /Users/shadowfax/.config/bpatch/config.toml
```

Run `bpatch init` from inside `chromium_patches/` to use the current directory. `--store <STORE>` overrides the config file for commands that read the store.

Requirements: Git 2.40 or newer; base-bump conflict sessions use `git merge-tree --write-tree --merge-base`.

Install from this directory:

```console
$ make install
```

`make install PREFIX=/usr/local/bin` installs somewhere else; `PREFIX` is the final binary directory.

Daily loop:

```console
$ bpatch status
base     148.0.7204.1 (9f8e7d6)
store    ~/code/browseros-project/.../chromium_patches @ 8c1f2ab   (2 revs ahead)
applied  store @ 55e0d3c  ·  41 feature commits  ·  last: feat: llmchat
tree     clean — no drift

$ bpatch diff
apply would touch 3 files · 1 feature:
  llmchat   M chrome/browser/ui/llmchat/panel.cc
            M chrome/browser/ui/llmchat/panel.h
            A chrome/browser/ui/llmchat/resize_util.cc
rebuild scope: no BUILD.gn / *.gni / include-fanout files touched → small incremental

$ bpatch apply
apply: store 8c1f2ab (delta vs applied 55e0d3c: 3 files)
  ✓ 3 files written · 3,161 store-managed files untouched (content + mtime preserved)
  ✓ commit 7ab19c2 "feat: llmchat #2"   [Bpatch-Store-Rev: 8c1f2ab]
converged. → incremental build will recompile ~1 target dir
```

## Verbs

Global flags:

| Flag | Meaning |
| --- | --- |
| `--store <STORE>` | Use this `chromium_patches` directory instead of `~/.config/bpatch/config.toml` for store-reading commands. |
| `--json` | Emit one JSON object, suppress progress and prompts. |

| Verb | Flags | Exit codes | Use |
| --- | --- | --- | --- |
| `bpatch init [STORE]` | global flags | `0`, `1` | Write `store = "<abs path>"` to `~/.config/bpatch/config.toml`, preserving other config keys and comments. |
| `bpatch status` | global flags | `0`, `3`, `1` | Show checkout base, store rev, applied trailers, and drift. |
| `bpatch diff` | global flags | `0`, `3`, `1` | Show what `apply` would touch, grouped by feature, with rebuild-scope hint. |
| `bpatch apply` | `--pull`, global flags | `0`, `2`, `3`, `1` | Optionally fast-forward the store repo, then converge the checkout or report conflicts/drift. |
| `bpatch extract [SPEC]` | `--feature <FEATURE>`, `--commit`, `--repin`, global flags | `0`, `3`, `1` | Extract `<rev>` or `<rev1>..<rev2>` into the store, or repin existing store patches to the checkout base. |
| `bpatch feature list` | global flags | `0`, `1` | List features, owned patch counts, and last applied sequence numbers. |
| `bpatch feature add <NAME> --path <PATH>` | `--description <DESCRIPTION>`, global flags | `0`, `1` | Append a new feature block to `features.yaml`. |
| `bpatch abort` | global flags | `0`, `1` | Remove a pending conflict session. |
| `bpatch continue` | `--materialize`, global flags | `0`, `2`, `1` | Materialize conflict markers or finish a conflict session after resolution. |

`extract` also has a hidden `--accept-suggestions` flag used by integration tests and scripted TTY bypasses; normal non-interactive routing should use `--feature <FEATURE>` or accept the `needs-feature` JSON result.

## Exit Codes

| Code | Meaning |
| --- | --- |
| `0` | Initialized, converged, applied, extracted, repinned, listed, added, aborted, or completed. |
| `2` | Conflicts are pending or conflict files remain unresolved. |
| `3` | Drift/refusal or `extract` needs a feature decision. |
| `1` | CLI, git, lock, config, or unexpected error. |

With `--json`, every command emits a single object carrying `result` and `exit`.

## Troubleshooting

If `status`, `diff`, or `apply` says the current path does not look like a Chromium checkout, you are probably running `bpatch` from the BrowserOS repo that contains `chromium_patches`, or from another git repo that is not the Chromium checkout. `cd` into the Chromium checkout that has the store base commit and re-run with the same `--store` path.

## Cron Recipe

```cron
# crontab (per checkout)
*/30 * * * * cd /Users/shadowfax/ch1-src && bpatch apply --pull --json >> ~/logs/bpatch-ch1.jsonl 2>&1; \
             tail -1 ~/logs/bpatch-ch1.jsonl | jq -e '.files_changed > 0' >/dev/null && \
             autoninja -C out/Default_arm64 chrome

# night 1 — store moved
{"result":"applied","store_rev":"8c1f2ab","base":"148.0.7204.1","files_changed":3,
 "commits":[{"feature":"llmchat","seq":2,"sha":"7ab19c2"}],"exit":0}

# night 2 — nothing new: no build triggered, no writes, lock released in <1s
{"result":"converged","store_rev":"8c1f2ab","files_changed":0,"exit":0}

# night 3 — a second bpatch already running on this checkout
{"result":"error","reason":"lock held by pid 4127 (started 02:00:03)","exit":1}
```

The checkout lock is per checkout and fails fast for a second invocation. Converged nights are exact no-ops: no commits, no writes, and `files_changed:0`. JSON mode writes one log line per run, so cron can gate `autoninja` with `jq` without parsing progress text.

## Agent Upgrade Guide

```console
[agent] ❯ git checkout 149.0.7250.0 && gclient sync        # new base, tool-agnostic
[agent] ❯ bpatch apply --json
{"result":"conflicts","base":"149.0.7250.0","merged":3161,"conflicts":[
  {"file":"chrome/browser/ui/browser_command_controller.cc","feature":"claw-commands","kind":"content"},
  {"file":"chrome/app/chrome_main_delegate.cc","feature":"bootstrap","kind":"content"}],
 "worktree_touched":false,"exit":2}

# nothing on disk changed yet — abort here would cost zero.
[agent] ❯ bpatch continue --materialize
2 files written with conflict markers; 3,161 clean files staged for convergence

[agent edits the 2 files, resolves markers]

[agent] ❯ bpatch continue
  ✓ converged on base 149.0.7250.0 · 41 feature commits authored
    [Bpatch-Store-Rev: 8c1f2ab · Bpatch-Base: 149.0.7250.0]

[agent] ❯ bpatch extract --repin
re-diffed 3,163 patches against base 149.0.7250.0 (2 content changes from conflict fixes)
store base pin: 148.0.7204.1 → 149.0.7250.0
next: bpatch extract --commit   (store repo commit: "chore: repin to 149.0.7250.0")
```

Before `continue --materialize`, `bpatch abort` only deletes the session file; the worktree has not been touched. A pending conflict session blocks `bpatch apply`; finish with `bpatch continue` or clear it with `bpatch abort`. When git plumbing fails during materialization or conflict completion, error messages include the exact `git read-tree`, `git update-index`, or recovery command to run manually.

## Store Layout

`chromium_patches/` contains:

| Path | Meaning |
| --- | --- |
| `<chromium/path>` | One git-style unified diff per Chromium-relative path, mirrored under the Chromium tree layout. |
| `features.yaml` | Canonical feature registry inside the store. Mutations append blocks so existing comments and bytes are preserved. |
| `store.yaml` | Base pin metadata: `base_commit` and `base_version`. |

`features.yaml` entries can own exact files or directory prefixes ending in `/`. `extract` matches exact paths first, then prefixes, then reports or creates the nearest feature decision.

## Cutover Checklist Vs `tools/patch`

The old Go tool stays in the repo until cutover; removing it is a separate effort. `packages/browseros/bos_build/features.yaml` also still exists for `bos_build`'s reader while consolidation is pending.

| Old `tools/patch` behavior | New `bpatch` path |
| --- | --- |
| `status` | `bpatch status` |
| `diff` | `bpatch diff` |
| `apply` / checkout catch-up | `bpatch apply` |
| store refresh by pulling first | `bpatch apply --pull` |
| `extract` | `bpatch extract <rev>` or `bpatch extract <rev1>..<rev2>` |
| feature inventory | `bpatch feature list` |
| feature creation | `bpatch feature add <name> --path <path> [--description ...]` |
| conflict abort | `bpatch abort` |
| conflict continue | `bpatch continue` or `bpatch continue --materialize` |
| base repin/rebase-store flow | `bpatch extract --repin` |

Not carried over in v1:

- `.bpatch` state files; state is recovered from commit trailers and git trees.
- Checkout aliases; run from the checkout directory and use config/`--store` for the store path.
- `publish`, `add`, `remove`, and `skip` flows.
- Branch-rebuild refresh machinery; convergence is tree-based and minimal-touch on same-base updates.
- `feature move`; feature mutation is append-only for now.

## Known V1 Limits

- Renames materialize as delete+add.
- Delete-kind merge conflicts refuse loudly at materialization.
- First extract/repin normalizes legacy abbreviated `index` lines per file as content changes touch them; change counts ignore `index` lines.
