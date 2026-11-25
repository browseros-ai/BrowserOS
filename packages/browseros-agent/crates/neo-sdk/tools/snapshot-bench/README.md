# snapshot-bench

How fast `neo_snapshot::render` prints a snapshot's text, and how much memory it takes, beside
today's TypeScript renderer in QuickJS, in one process. It is the bench of the Rust printing
design's measurement plan: a report run by hand at the switch to Rust and again after the old
renderer is removed. Nothing in a gate runs it or asserts a time.

## What it does

1. Builds trees shaped like the three pages the targets were measured on, from a seed
   (`src/shapes.rs`): a discussion thread (about 49,900 nodes, 4.2 MB of JSON), an article
   (about 7,350 nodes, 0.9 MB) and a data grid (about 600 nodes). Only the shape is copied
   (node count, size, role mix, depth, string lengths), never a page's text, so no real page
   reaches the machine that runs it.
2. Writes each tree as JSON and decodes it with serde, as the server reads a probe reply.
3. For each tree it runs the default call, `mode: 'full'` and Playwright's format:
   - **new**: `neo_snapshot::render` on the decoded tree;
   - **old**: `todaysAnswer(call, result)` from `neo-snapshot/tests/data/todays-renderer.js`
     (today's renderer, built by neo-playwright-js `scripts/build.ts`) in QuickJS, with a run's
     limits (128 MiB heap, 512 KiB stack). `JSON.parse` of the result is timed apart from the
     render.
   Every old run's text is compared with Rust's. A row whose bytes differ, or whose old engine
   failed, says so, and the process exits 1.
4. The delta's worst case: a page of 33,301 one-line nodes and a copy with 500 links renamed
   from the first line to the last (1,000 changed lines), under `maxTokens: 1e9`. Once with
   every line distinct, once with every unchanged line one of two, so the search meets the most
   ties.
5. Prints a Markdown report: first run and median per row, old over new, memory, and the
   design's targets.

Memory is read two ways. The binary's global allocator counts live bytes and their peak; with
rquickjs's `rust-alloc`, QuickJS allocates through it too, so one counter reads both engines.
QuickJS's own count (`memory_usage().malloc_size`) is the number its 128 MiB limit is checked
against.

## Running it

From this folder, with the toolchain the SDK uses:

```sh
cargo run --release --locked                       # every shape, 9 runs a row
cargo run --release --locked -- --runs 3 --only thread,delta --out report.md
```

On the remote pool, from `packages/browseros-agent`:

```sh
bun run remote run -- sh -c 'cd crates/neo-sdk/tools/snapshot-bench && cargo run --release --locked -- --out /tmp/snapshot-bench.md'
```

It is its own Cargo workspace, outside the SDK's lint set and `check.sh`, like the spec add-on.
Its lockfile was seeded from the SDK's, so QuickJS, serde and serde_json are the versions the
server builds with. If neo-snapshot gains a dependency, `--locked` fails here: refresh the lock
with `cargo update -p neo-snapshot` and commit it.

Once the TypeScript renderer and its bundle are removed, the old half goes with them and the
bench times Rust alone.
