# Recorded AnnotatedPageContent fixtures and their oracles

Each fixture is two files recorded from one page load:

- `<name>.pb`: the real `Page.getAnnotatedPageContent` (`includeActionableInformation: true`) protobuf payload. `tests/fixtures.rs` decodes the original seven offline.
- `<name>.oracle.json`: the oracle, what Playwright's own ARIA engine says about that same page. `tests/oracles.rs` checks its shape. The Rust normalizer is later compared with it node by node, which makes functional parity a number per fixture (design 3.8, kind 2). The oracle is recorded data, so it stays the parity target after the JavaScript engine leaves the bundle.

The registry is `FIXTURES` in `tests/support/fixture_pages.rs`. A fixture that is not in it is not recorded or tested.

## Recording environment

- Browser binary: `BrowserOS Dev` (`/Users/shadowfax/code/chromium-3/src/out/Default_browseros_arm64/BrowserOS Dev.app/Contents/MacOS/BrowserOS Dev`).
- Browser version: Chromium `158.0.8127.18`, stock (unpatched APC). Each oracle records `Browser.getVersion`'s product in `browser`.

Run from `packages/browseros-agent` (in a heavy slot on the shared machine). Pass fixture names after `--` to re-record a subset:

```sh
NEO_E2E_BROWSER="/Users/shadowfax/code/chromium-3/src/out/Default_browseros_arm64/BrowserOS Dev.app/Contents/MacOS/BrowserOS Dev" \
  bun run test:browser -- cargo run -p neo-snapshot-apc --example record_fixtures
```

The recorder prints one line per fixture and exits non-zero when any fixture failed. A failed fixture writes neither file.

## One page load per fixture

```text
new tab ─▶ Emulation.setFocusEmulationEnabled ─▶ navigate ─▶ ready predicate
        ─▶ type secrets ─▶ oracle ─▶ before-capture step ─▶ APC ─▶ close
```

- Focus emulation is on before navigation, so `document.hasFocus()` and the `active` state do not depend on which tab the browser last raised.
- The oracle runs before the before-capture step. `hung-subframe`'s step makes its `[::1]` child spin forever so APC hits its subframe timeout and leaves that frame empty. A spinning renderer answers no script, so the oracle reads every frame, the hung one included, before the spin starts. In the recording the oracle has all three frames (the hung one with its heading and button), while the `.pb` has the hung frame's `<iframe>` node with no `FrameData` and no children. That node's `common_ancestor_dom_node_id` is still the hung frame's `owner`.

## The oracle

- **Engine.** `crates/neo-sdk/neo-playwright/assets/injected.js`, unchanged: Playwright's injected engine plus Neo's probe, as Neo ships it. `oracle.js` (`tests/support/`) is spliced in right after the bundle's one `globalThis.__neoProbe ??= createProbe();` statement, inside its IIFE, where it sees the bundle's top-level bindings by name. The recorder fails if that anchor is not there exactly once, and `oracle.js` throws, naming the binding, if one it reads is gone. `engine` in each oracle is the bundle's banner line (Playwright's version and commit).
- **World.** The patched bundle runs in an isolated world named `neo-oracle`, one per frame, so its prototype patches reach neither the page nor Neo's probe world.
- **Closed shadow roots.** Opened as Neo's marked snapshots open them: every element inside a closed root of the frame's document (from `DOMSnapshot.captureSnapshot` on the frame's session) is handed to `openClosedRoots` before the engine is built.
- **Masking scope.** Everything is read inside `withMaskedFieldValues(..., { declared: true })`, the scope Neo's snapshots use (`failure-output.ts` `tree`). A password field or a field with a private `autocomplete` token reads as bullets, exactly as Neo prints it today.
- **Contents per frame.** Every element `getAriaRole` gives a role (tree order, shadow root before light children): role, name, description, error message, hidden for ARIA, visible, and the states `toAriaNode` offers that role, plus `active`. `aria.ai` and `aria.default` are `ariaSnapshotJSON` from the frame's body, exactly as returned. A child frame's refs are `f<seq>e<n>`, where seq is the frame's index in `frames`.
- **Refs.** `refs` maps every ai-mode ref the engine minted on the read to its element. Playwright's distiller drops some nodes from the printed tree but keeps their refs resolvable, so `refs` can hold more than `aria.ai` prints: 12 more on `roles`, 9 on `names`. An iframe or a `generic` node has a ref but no entry in `elements`, which lists only elements with a role.
- **Frames.** Main first, then depth first, each frame's children sorted by URL then frame id. `frame` is the CDP frame id (the devtools frame token). URLs have the fixture server's ephemeral port removed.

### How it joins the `.pb`

- An element joins by `backendNodeId`: it is the APC node's `common_ancestor_dom_node_id` (both are Blink's `DOMNodeIds`). Ids are per renderer, so the join is within one frame's document.
- A child frame joins by `owner`: the backendNodeId of its `<iframe>` in the parent document, which is the APC iframe node's `common_ancestor_dom_node_id`. `tests/oracles.rs` checks that every child frame's owner is an iframe node of its parent's document in the same fixture's `.pb`.

## Inventory

| Fixture | Design 3.8 row it pins | Frames | APC nodes | Oracle elements | Refs | `.pb` bytes | Oracle bytes |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `nested-cross-site-frames` | Nested cross-site frames (8 frames, 3 sites) | 8 | 71 | 32 | 39 | 5,450 | 21,359 |
| `hung-subframe` | Nested cross-site frames, `?hang` | 3 | 16 | 9 | 11 | 1,494 | 6,449 |
| `shadow-dom` | Open and closed shadow roots (one level) | 1 | 23 | 9 | 13 | 1,924 | 6,495 |
| `shadow-roots` | Open and closed shadow roots, two levels, slots, a frame inside a closed root | 2 | 69 | 24 | 32 | 6,024 | 19,597 |
| `zero-size-checkbox` | Zero-size checkboxes with labels | 1 | 13 | 4 | 5 | 1,136 | 3,505 |
| `zero-size-controls` | Zero-size checkbox, radio and file input, labels by `for` and by nesting | 1 | 59 | 13 | 22 | 4,888 | 12,762 |
| `nosize` | Zero-size controls (`data/prelude/nosize.html`) | 1 | 71 | 30 | 14 | 5,141 | 20,650 |
| `ua-shadow` | Browser-made shadow content | 1 | 88 | 18 | 23 | 7,095 | 13,640 |
| `roles` | Roles | 1 | 350 | 178 | 218 | 25,771 | 125,646 |
| `names` | Names | 1 | 213 | 77 | 107 | 18,381 | 62,352 |
| `states` | States | 1 | 307 | 147 | 174 | 25,827 | 121,159 |
| `private-forms` | Private forms | 1 | 84 | 21 | 50 | 6,790 | 23,525 |
| `sensitive-form` | Private forms (values set in markup, not typed) | 1 | 24 | 6 | 10 | 1,989 | 5,599 |
| `covered-button` | A covered button | 1 | 16 | 8 | 9 | 1,418 | 5,308 |
| `modal-inert` | A modal dialog, an `inert` subtree | 1 | 27 | 13 | 15 | 2,322 | 9,298 |
| `deep-nesting` | None: 120 nested groups, past prost's and serde's default depth limits | 1 | 127 | 123 | 123 | 8,269 | 388,209 |

Frames is the oracle's count; APC's frame count (main plus iframe nodes) is the same on every fixture.

Total: 16 fixtures, 32 files, 969,472 bytes (123,919 in `.pb` files, 845,553 in oracles). `deep-nesting.oracle.json` is 388,209 of them: pretty-printing indents its 120-level tree. Compressed, as git stores it, the directory is about 81 KB (`deep-nesting.oracle.json` about 5.5 KB), so the oracles stay pretty-printed for review.

## Secrets and the canary guard

Fields carry `data-typed` (always typed) or `data-typed="contract"` (typed only under `--contract`). The recorder focuses each allowed field and types a canary, `CANARY_PREFIX` (`tests/support/fixture_pages.rs`) then `-<fixture>-<index>`, with `Input.insertText`, then calls `window.__afterTyping?.()` (on `private-forms` it flips the show-password field to text) and blurs the field. Before writing anything, it searches both files' bytes for the prefix and for every canary typed. On any hit it writes neither file and fails, naming the fixture and file. `tests/oracles.rs` repeats the prefix search over every file in this directory, this one included, which is why it never spells the prefix out.

On stock Chromium 158 APC withholds a typed value only for `type=password` and for a field toggled from password to text after typing. Every `autocomplete` token field (`current-password`, `new-password`, `one-time-code`, `cc-number`, `cc-csc`, on `input` and `textarea`) and a plain field write the typed value into the `.pb`, twice. So on stock the token fields are `data-typed="contract"`, are not typed into, and are empty in both files: the contract fields are absent from these recordings. This recording typed 2 canaries on `private-forms` (the password field and the show-password field) and 1 on `names` (the password box inside a labelledby chain). The oracle prints each as bullets of the canary's length (33 and 25), which shows the typing happened and Neo's masking scope applied.

## Re-recording after Chromium PRs C3 and C4

Once a local BrowserOS build carries contract PRs C1 to C4:

1. Record with the contract flag, so the token fields are typed into too:

   ```sh
   NEO_E2E_BROWSER="<that build's binary>" \
     bun run test:browser -- cargo run -p neo-snapshot-apc --example record_fixtures -- --contract
   ```

2. Run `cargo test -p neo-snapshot-apc` and check that `no_recorded_fixture_holds_a_typed_secret` passed.
   `tests/contract.rs`'s `stock_chromium_replies_decode_with_no_contract` then fails by design: the fixtures are no longer stock replies, so that test moves to the patched recordings' contract facts.
3. Update the recording environment above and the inventory table.
