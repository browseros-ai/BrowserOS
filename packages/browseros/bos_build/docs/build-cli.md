# Build CLI Reference

Everything `browseros build` can do beyond the common invocations in
[`../README.md`](../README.md). Run these from `packages/browseros`.

## Seeing and tweaking a plan

The composed plan is always a projection of `plan()` in `core/planner.py` —
generated, never hand-copied, so it cannot drift from what actually runs.

```bash
# Print the composed steps + required env vars and exit
# (works without a chromium checkout)
browseros build --preset release --show-plan

# Comment out steps, as an operation: subtract from the composed plan
browseros build --preset release --skip upload,series_patches

# Resume the tail after a failure without recompiling
browseros build --preset release --from sign_macos

# One-off GN overrides while iterating (appended last, so they win)
browseros build --preset debug --gn-arg symbol_level=2 --gn-arg dcheck_always_on=true
```

### `--skip`

Subtracts **after** composition, so it never re-triggers composition rules —
skipping `sign_windows` will not add `mini_installer`. CLI `--skip` and a
profile's `skip:` list union. Unknown step names fail loudly; a valid step
absent from this plan is a no-op, so a saved `skip:` keeps working as presets
evolve. It is subtraction from the canonical plan, never a copy of it.

### `--from`

Resumes the composed (post-skip) run timeline at a step. Earlier runs are
dropped, the first run containing the step is sliced, later runs stay whole. A
failed universal merge resumes with just
`--arch universal --from merge_universal` — no recompiles.

CLI-only: a resume is a one-off, so there is no `from:` profile key.

### `--gn-arg`

Repeatable, valid in any mode. Appends GN overrides **after** the flags file and
the product args, so last write wins and `configure` honors them without editing
the committed `config/gn/*.gn` files.

Values are verbatim GN: bools and ints bare, strings with embedded quotes
(`--gn-arg 'target_cpu="arm64"'`). CLI-only by design — a profile that wants
different flags should use a different flags file.

Only the `configure` step writes `args.gn`. A plan that skips it (for example
`--from compile`) reuses the existing file untouched, including any overrides a
previous invocation wrote there.

## Modules profiles — "you own this list now"

For the rare run that genuinely wants an arbitrary sequence, a profile may carry
`modules:` as an explicit opt-in — a local, commentable file that bypasses the
planner entirely:

```yaml
# my-tail.yaml (local only — never shipped)
modules: [compile, sign_macos, package_macos]
build_type: release   # only valid with modules:; defaults to debug
arch: arm64           # single arch only
```

Planner-owned keys (`preset`, `clean`, `provision`, `download`, `sign`,
`upload`, `bundle_local_extensions`, `skip`) and the `--skip` / `--from` flags
are rejected alongside `modules:`. You own the list; edit it directly. Shipped
profiles stay switch-based, and a golden test keeps them from drifting.

The same escape hatch exists on the command line for one-off runs:

```bash
browseros build --modules clean,compile,sign_macos --product browseros
browseros build --list                                # every available step
```

Phase flags are the middle ground — auto-ordered, no explicit step list:

```bash
browseros build --setup --build --sign --package
browseros build --build --sign                        # skip setup
```

Note that `--prep` does **not** apply `series_patches`. Run
`browseros build -m series_patches` separately if you need them.

## Remote and ephemeral runners

A fresh machine needs nothing outside this package:

```bash
uv sync
uv run browseros source ensure --root "$CHROMIUM_ROOT" --step checkout
uv run browseros build --modules clean --chromium-src "$CHROMIUM_ROOT/src" -t release
uv run browseros source ensure --root "$CHROMIUM_ROOT" --step sync
uv run browseros build --profile nightly-ci --chromium-src "$CHROMIUM_ROOT/src"
```

Checkout and sync are split because `clean` must run between them: it deletes
the hook-managed toolchains that sync then restores. On runners without
WarpCache, `browseros source cache restore|save` handles the R2 checkout cache.

## Concurrency

Every build takes an exclusive lock on its Chromium checkout, keyed by the
resolved `src` path and held regardless of product. A second build against the
same checkout fails fast and tells you who holds it. Pass `--lock-wait` to queue
behind it instead.

Release builds run `git reset`, `git clean`, and patch application in one shared
checkout, which is why the lock exists. The lock file sits next to the gclient
root rather than under `src/`, so `git clean` cannot delete it mid-build.

## Products

A product is one file: `products/<id>/product.py` with a
`ProductDescriptor.define()` call — about five irreducible inputs, roughly forty
fields derived by convention, keyword overrides for deviations — plus its server
bundle definitions. Verify with:

```bash
browseros product doctor          # identity uniqueness + branding assets
```
