# Recorded AnnotatedPageContent fixtures and their oracles

Each fixture is two files recorded from one page load:

- `<name>.pb`: the real `Page.getAnnotatedPageContent` (`includeActionableInformation: true`) protobuf payload. `tests/fixtures.rs` decodes the original seven offline.
- `<name>.oracle.json`: the oracle, what Playwright's own ARIA engine says about that same page. `tests/oracles.rs` checks its shape. `tests/tree_roles_states_oracle.rs` and `tests/tree_names_oracle.rs` compare the Rust `SnapshotTree` with it node by node, which makes functional parity a number per fixture (design 3.8, kind 2). Every element where the two still disagree is pinned, with its owner, in `tests/support/parity_ledger.rs`. The oracle is recorded data, so it stays the parity target after the JavaScript engine leaves the bundle.

The registry is `FIXTURES` in `tests/support/fixture_pages.rs`. A fixture that is not in it is not recorded or tested.

## Recording environment

- Browser binary: `BrowserOS Dev` (`/Users/shadowfax/code/chromium-1/src/out/Default_browseros_arm64/BrowserOS Dev.app/Contents/MacOS/BrowserOS Dev`).
- Browser version: BrowserOS Chromium `158.0.8127.18` at `735d57e27` (BrowserOS-internal #66 to #71, Chromium contract PRs C1 to C6, APC contract v6) for 15 fixtures, and at `9bbc6ad5d` (BrowserOS-internal #72, Chromium contract PR C7 / P15 inline whitespace breaks, APC contract v7) for `names`. `names.pb` reports `contract_version: 7`; the other 15 `.pb` files report `contract_version: 6`. Each oracle records `Browser.getVersion`'s product in `browser`, which does not tell a patched build from stock.
- What v5 to v7 added to the captures: an `<html>` node in every document (P12: 4 on `nested-cross-site-frames`, 1 each on `shadow-roots` and `modal-inert`), a node per `display: contents` element (P11: 5 on `roles`), `svg_title` (P13), `break_before` at a non-inline `::before` or `::after` box (P14), and (from v7) `break_before` at a skipped collapsible-whitespace text box between inline boxes (P15, verified on `names`, which now matches Playwright's default-mode tree outright). The oracles are the same as on v4 apart from the two span-built lines added to `names.html` and fresh CDP frame ids and `backendNodeId`s.

Run from `packages/browseros-agent` (in a heavy slot on the shared machine), with `--contract` so the contract's private fields are typed into too. Pass fixture names after the flag to re-record a subset:

```sh
NEO_E2E_BROWSER="/Users/shadowfax/code/chromium-1/src/out/Default_browseros_arm64/BrowserOS Dev.app/Contents/MacOS/BrowserOS Dev" \
  bun run test:browser -- cargo run -p neo-snapshot-apc --example record_fixtures -- --contract
```

The oracle tests refuse a recording below contract v4, so a recording made on stock Chromium fails them.

The recorder prints one line per fixture and exits non-zero when any fixture failed. A failed fixture writes neither file.

### Option places, merged in from a re-read

The reader learned to write each listed `<option>`'s `option` place after these files were recorded. `--option-places` adds the places without re-recording a `.pb`:

```sh
NEO_E2E_BROWSER="<a Chromium 158 build; it needs no APC patches>" \
  bun run test:browser -- cargo run -p neo-snapshot-apc --example record_fixtures -- --contract --option-places
```

Per fixture it loads and types as a recording does, reads the oracle and closes the tab. It runs no before-capture step and no APC, and never writes a `.pb`. The fresh read must equal the committed oracle in every value except the engine banner, each frame's CDP `frame` and `parent` ids, and the `option` fields. A different backendNodeId, owner, ref, fact, order, URL or ARIA tree fails the fixture, names the first differing JSON path, and writes nothing. On equality it writes the committed oracle (its banner and frame ids kept) with the read's places, after the canary guard, so the file's diff is the `option` fields alone. A fixture with no place to add is reported unchanged and not written.

It exists because the oracle is Playwright's engine reading the DOM, so a browser without the APC patches reads the same oracle, while a fresh `.pb` from the patched browser would carry whatever its newer contract changed. The equal backendNodeIds, owners and refs are what keep every join to the committed `.pb` intact without a shared page load.

Run on 2026-11-26 on the stock-158 reference build (`/Users/shadowfax/code/chromium-3/src/out/Default_browseros_arm64/BrowserOS Dev.app`, `Browser.getVersion` product `Chrome/158.0.8127.18`), headless with a throwaway profile, with `--contract`. Five oracles gained places: `nosize` 2, `ua-shadow` 3, `roles` 7, `names` 3, `states` 8. Every other value of those oracles and every `.pb` file is unchanged. Nine fixtures were unchanged (no `<select>`). `nested-cross-site-frames` and `shadow-roots` failed the equality check (a backendNodeId one or two off, alone and in the full run) and were left as recorded. Neither has a `<select>`, so neither had a place to add.

## One page load per fixture

```text
new tab ─▶ Emulation.setFocusEmulationEnabled ─▶ navigate ─▶ ready predicate
        ─▶ type secrets ─▶ oracle ─▶ before-capture step ─▶ APC ─▶ close
```

- Focus emulation is on before navigation, so `document.hasFocus()` and the `active` state do not depend on which tab the browser last raised.
- The oracle runs before the before-capture step. `hung-subframe`'s step makes its `[::1]` child spin forever so APC hits its subframe timeout and leaves that frame empty. A spinning renderer answers no script, so the oracle reads every frame, the hung one included, before the spin starts. In the recording the oracle has all three frames (the hung one with its heading and button), while the `.pb` has the hung frame's `<iframe>` node with no children and, instead of `FrameData`, a `RedactedFrameMetadata` the contract stamps with `BrowserOsTimeout` and the frame's CDP id. That node's `common_ancestor_dom_node_id` is still the hung frame's `owner`.

## The oracle

- **Engine.** `crates/neo-sdk/neo-playwright/assets/injected.js`, unchanged: Playwright's injected engine plus Neo's probe, as Neo ships it. `oracle.js` (`tests/support/`) is spliced in right after the bundle's one `globalThis.__neoProbe ??= createProbe();` statement, inside its IIFE, where it sees the bundle's top-level bindings by name. The recorder fails if that anchor is not there exactly once, and `oracle.js` throws, naming the binding, if one it reads is gone. `engine` in each oracle is the bundle's banner line (Playwright's version and commit).
- **World.** The patched bundle runs in an isolated world named `neo-oracle`, one per frame, so its prototype patches reach neither the page nor Neo's probe world.
- **Closed shadow roots.** Opened as Neo's marked snapshots open them: every element inside a closed root of the frame's document (from `DOMSnapshot.captureSnapshot` on the frame's session) is handed to `openClosedRoots` before the engine is built.
- **Masking scope.** Everything is read inside `withMaskedFieldValues(..., { declared: true })`, the scope Neo's snapshots use (`failure-output.ts` `tree`). A password field or a field with a private `autocomplete` token reads as bullets, exactly as Neo prints it today.
- **Contents per frame.** Every element `getAriaRole` gives a role (tree order, shadow root before light children): role, name, description, error message, hidden for ARIA, visible, and the states `toAriaNode` offers that role, plus `active`. `aria.ai` and `aria.default` are `ariaSnapshotJSON` from the frame's body, exactly as returned. A child frame's refs are `f<seq>e<n>`, where seq is the frame's index in `frames`.
- **Refs.** `refs` maps every ai-mode ref the engine minted on the read to its element. Playwright's distiller drops some nodes from the printed tree but keeps their refs resolvable, so `refs` can hold more than `aria.ai` prints: 12 more on `roles`, 9 on `names`. An iframe or a `generic` node has a ref but no entry in `elements`, which lists only elements with a role.
- **Frames.** Main first, then depth first, each frame's children sorted by URL then frame id. `frame` is the CDP frame id (the devtools frame token). URLs have the fixture server's ephemeral port removed.
- **Option places.** Every `<option>` a select lists carries `option: { select, index, count }`: its select's backendNodeId, its index in `select.options`, and that list's length, read from the select by `oracle.js`, separately from the probe's own option rule. An option of no select, or one in a `<datalist>`, has none.

### How it joins the `.pb`

- An element joins by `backendNodeId`: it is the APC node's `common_ancestor_dom_node_id` (both are Blink's `DOMNodeIds`). Ids are per renderer, so the join is within one frame's document.
- A child frame joins by `owner`: the backendNodeId of its `<iframe>` in the parent document, which is the APC iframe node's `common_ancestor_dom_node_id`. `tests/oracles.rs` checks that every child frame's owner is an iframe node of its parent's document in the same fixture's `.pb`.
- An `<option>` of a collapsed `<select>` has no APC node: APC sends its options as `select_options` on the select. It joins the select's option slot by its `option` place, through `SnapshotTree::node_at` (`tests/support/oracle_join.rs`), which the roles and states, names and emit-refs oracle tests and `zero_conflict_live` all use. A slot count other than the place's `count` fails as a broken recording. `tests/oracles.rs` checks that every place names a `<select>` node of the same frame's document in the `.pb`, with the option count it sent.

## Inventory

| Fixture | Design 3.8 row it pins | Frames | APC nodes | Oracle elements | Refs | `.pb` bytes | Oracle bytes |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `nested-cross-site-frames` | Nested cross-site frames (8 frames, 3 sites) | 8 | 75 | 32 | 39 | 6,580 | 21,357 |
| `hung-subframe` | Nested cross-site frames, `?hang` | 3 | 16 | 9 | 11 | 1,751 | 6,449 |
| `shadow-dom` | Open and closed shadow roots (one level) | 1 | 23 | 9 | 13 | 2,148 | 6,495 |
| `shadow-roots` | Open and closed shadow roots, two levels, slots, a frame inside a closed root | 2 | 70 | 24 | 32 | 6,581 | 19,597 |
| `zero-size-checkbox` | Zero-size checkboxes with labels | 1 | 13 | 4 | 5 | 1,266 | 3,505 |
| `zero-size-controls` | Zero-size checkbox, radio and file input, labels by `for` and by nesting | 1 | 59 | 13 | 22 | 5,351 | 12,762 |
| `nosize` | Zero-size controls (`data/prelude/nosize.html`) | 1 | 72 | 30 | 14 | 5,977 | 20,866 |
| `ua-shadow` | Browser-made shadow content | 1 | 88 | 18 | 23 | 7,734 | 13,961 |
| `roles` | Roles | 1 | 384 | 178 | 218 | 30,643 | 126,402 |
| `names` | Names | 1 | 229 | 79 | 110 | 21,991 | 64,167 |
| `states` | States | 1 | 314 | 147 | 174 | 30,181 | 122,019 |
| `private-forms` | Private forms | 1 | 70 | 21 | 50 | 6,523 | 26,237 |
| `sensitive-form` | Private forms (values set in markup, not typed) | 1 | 20 | 6 | 10 | 1,854 | 5,599 |
| `covered-button` | A covered button | 1 | 16 | 8 | 9 | 1,590 | 5,308 |
| `modal-inert` | A modal dialog, an `inert` subtree | 1 | 28 | 13 | 15 | 2,642 | 9,298 |
| `deep-nesting` | None: 120 nested groups, past prost's and serde's default depth limits | 1 | 127 | 123 | 123 | 9,555 | 388,209 |

Frames is the oracle's count; APC's frame count (main plus iframe nodes) is the same on every fixture.

Total: 16 fixtures, 32 files, 994,598 bytes (142,367 in `.pb` files, 852,231 in oracles). `deep-nesting.oracle.json` is 388,209 of them: pretty-printing indents its 120-level tree. Compressed, as git stores it, the directory is about 82 KB (`deep-nesting.oracle.json` about 5.5 KB), so the oracles stay pretty-printed for review.

## Secrets and the canary guard

Fields carry `data-typed` (always typed) or `data-typed="contract"` (typed only under `--contract`). The recorder focuses each allowed field and types a canary, `CANARY_PREFIX` (`tests/support/fixture_pages.rs`) then `-<fixture>-<index>`, with `Input.insertText`, then calls `window.__afterTyping?.()` (on `private-forms` it flips the show-password field to text) and blurs the field. Before writing anything, it searches both files' bytes for the prefix and for every canary typed. On any hit it writes neither file and fails, naming the fixture and file. `tests/oracles.rs` repeats the prefix search over every file in this directory, this one included, which is why it never spells the prefix out.

Stock Chromium 158 withholds a typed value only for `type=password` and for a field toggled from password to text after typing, so the stock recordings left the `data-typed="contract"` fields untyped. The contract withholds every private field's value: the `autocomplete` token fields (`current-password`, `new-password`, `one-time-code`, `cc-number`, `cc-csc`, on `input` and `textarea`) as well. This recording ran with `--contract` and typed 12 canaries on `private-forms` (every `data-typed` field) and 1 on `names` (the password box inside a labelledby chain). The recorder's guard found none of them in either file, and `no_recorded_fixture_holds_a_typed_secret` (`tests/oracles.rs`) passes. The oracle prints each as bullets of the canary's length (33 or 34 on `private-forms`, 25 on `names`), which shows the typing happened and Neo's masking scope applied. On `sensitive-form` the contract also withholds the card number and one-time code set in markup, which stock wrote into the `.pb`, so that page lost 4 APC nodes (the two value nodes under each of those fields).
