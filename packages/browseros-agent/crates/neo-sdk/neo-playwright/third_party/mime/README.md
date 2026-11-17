# Vendored mime

The [`mime`](https://github.com/broofa/mime) package's TypeScript sources, copied unmodified
from its npm tarball under MIT. `VERSION` holds the version and the tarball's npm integrity,
which `sync.sh` checks before it copies anything. `LICENSE` is the package's own.

Playwright's server types an in-memory `setInputFiles` payload that names no `mimeType` with
`mime.getType(name)` from this package (`prepareFilesForUpload`, server/fileUploadUtils.ts).
Playwright 1.64.1 bundles `mime@4.1.0` into `playwright-core/lib/utilsBundle.js`, as that
bundle's LICENSE file says, so this is the same version. The prelude's `files.ts` builds the
same instance from these files and makes the same call, so a payload's File gets the type
Playwright would give it.

No file here is edited by hand. `sync.sh` writes `SHA256SUMS`, and the crate's
`tests/engine_sources.rs` fails when a file differs from it or is missing from it.
`crates/neo-sdk/neo-playwright-js/tests/mime-golden.test.ts` checks the table these files build
against the one Playwright's own bundle built (`tests/data/mime-golden.json`, recorded by
`scripts/capture-mime-golden.mjs`), so a version that drifts from Playwright's fails there.

## What is here

- `src/Mime.ts`: the `Mime` class, with `getType`, which reads the extension after the last
  dot of the name's last path segment.
- `src/index.ts`: the default instance, `new Mime(standardTypes, otherTypes)._freeze()`, the one
  Playwright's bundle builds.
- `types/standard.ts` and `types/other.ts`: the two tables that instance is built from.

## What is left out

`src/index_lite.ts` (an instance from the standard table alone, which Playwright does not use),
`src/mime_cli.ts` and `bin/` (the command line tool), and the compiled `dist/` copies of the
files above.
