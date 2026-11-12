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

## External Chromium patch packages

`--patch-root` selects another `packages/browseros` directory as the source of
`CHROMIUM_VERSION`, `BASE_COMMIT`, `chromium_patches` (including ownership and
store metadata), optional `v8_patches`, and `series_patches`. The version and
canonical store base must agree. V8 patches run before Chromium patches, and
their base must match the V8 gitlink in the selected Chromium base. Optional
V8 store metadata and ownership are validated before execution; the gitlink
check runs as soon as the Chromium commit is available, before source selection
or reset. Symlinks inside external patch inputs are rejected so resume can
identify all consumed bytes; a symlink to the package root itself is supported.

Build code, profiles, GN flags, branding overlays, resource manifests, product
version, and build offset still come from the repository running the command.
Omitting `--patch-root` keeps the existing single-repository behavior.

```bash
# Inspect the selected inputs without a Chromium checkout.
browseros build --preset debug --patch-root /path/to/internal/packages/browseros --show-plan

# Use the selected package with an existing source checkout.
browseros build --preset debug --chromium-src /path/to/chromium/src \
  --patch-root /path/to/internal/packages/browseros
```

The option works with presets, modules profiles, and direct `--modules`/phase
flags. It is a CLI-only path; profiles remain portable. When provisioning source
separately, pass the same package's `CHROMIUM_VERSION` to `browseros source ensure` with
`--version-file`.

Resume checkpoints include the external pin files and patch-store contents,
including V8 and platform series. Editing those inputs invalidates a prior
checkpoint even when the build repository has not changed. External patch
packages cannot be combined with `--lane-manifest`: release-lane evidence
currently attests one source repository and cannot represent this split.

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
`upload`, `resource_mode`, `skip`) and the `--skip` / `--from` flags are
rejected alongside `modules:`. You own the list; edit it directly. Shipped
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
uv run browseros source ensure --root "$CHROMIUM_ROOT" --reset
uv run browseros source clean-outputs --root "$CHROMIUM_ROOT" --all-products --arch x64
uv run browseros build --profile release-ci --chromium-src "$CHROMIUM_ROOT/src" --arch x64
```

The complete preparation command holds the checkout lock, fetches the exact
missing pin, resets source and managed dependencies, then runs sync/hooks.
Omit `--reset` to preserve local changes; conflicting checkout/sync operations
fail without silently discarding work. `--step checkout` and `--step sync` remain
partial-operation compatibility commands. Cache restore/save also hold the lock;
archives exclude lock files. Hosted roots remain exclusive to their workflow job.

macOS CI uses `source workspace` through `macos-chromium-workspace.sh`: it refreshes
the persistent base once and holds its lock through every requested product copy.
Per-product output/resource cleanup follows; `release-ci` and `nightly-macos`
then skip source cleanup and provisioning. `source cleanup-workspaces` removes
owned copy contents but retains small ownership markers and lock directories so
waiting processes cannot acquire a replacement lock inode. Local release and
explicit `--modules clean` retain their reset behavior.

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
