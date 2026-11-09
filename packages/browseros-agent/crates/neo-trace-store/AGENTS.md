# neo-trace-store

Owns operation-trace persistence: SQLite schema, typed reads, a synchronous runtime sink, one background writer, committed session heads, retention and restart recovery. Read `../NEO_STYLE.md` first. Runtime event types are consumed from `neo-runtime`; HTTP and hosted API mapping belong to feeds.

- SeaORM entities, active enums, migrations and connections are crate-private. Public signatures use owned views, runtime IDs and `StoreError`; never expose SeaORM or hosted API types. Each table has an entity and relations to its run.
- `ingest.rs` must work on a plain thread with no tokio runtime: no wait, lock, I/O or await. Each run owns atomic loss counters. Queued event payloads own byte permits through transaction retries; only commit and post-commit head publication release them. Control fences use the same FIFO.
- `apply.rs` and `apply/` are transaction-local projections. Preserve replay idempotence, first-end wins and the independent settled-delivery guard. The writer owns ordering, retry, settlement deadlines and notifications.
- Migrations use only SchemaManager builders, frozen per-migration idens and guarded DDL. Never edit an applied migration after release. Keep the private `neo_trace_migrations` ledger and run all pending steps in one transaction. Add a fixture for every new prefix; the array is sized by `MIGRATION_NAMES`.
- Retention deletes children first in transactions of at most 10,000 child rows. Never use a parameter per run or cascading deletes that defeat that bound. Privacy tombstones must survive captures that complete late.

- Shutdown order: stop admitting runs and cancel/await active runs, await `runtime.drain()`, then call `traces.close(budget)`. Outcomes may precede late input acknowledgements; closing ingest before the runtime drains would lose their final delivery and settlement events.

From `packages/browseros-agent`, use private Cargo directories and run `scripts/neo-check.sh`, `cargo test -p neo-trace-store --locked`, and `cargo check --workspace --locked`. Unit tests build public events by hand; `tests/runtime_store.rs` drives a real Runtime against fake CDP. `tests/store_e2e.rs` uses an isolated browser and skips without `NEO_E2E_BROWSER`.
