# Vendored yaml

The [`yaml`](https://github.com/eemeli/yaml) package's ES module build, copied unmodified from
its npm tarball under ISC. `VERSION` holds the version and the tarball's npm integrity, which
`sync.sh` checks before it copies anything. `LICENSE` is the package's own.

`toMatchAriaSnapshot` takes its template as YAML. Playwright's server parses it before the page
sees it: `Frame.expect` (server/frames.ts) calls the vendored isomorphic `parseAriaSnapshotUnsafe`
with this library, and the page matches the parsed template. Playwright 1.64.1 bundles
`yaml@2.9.0` into `playwright-core/lib/utilsBundle.js`, as that bundle's LICENSE file says, so
this is the same version. Neo's prelude makes the same call on these files
(`crates/neo-sdk/neo-playwright-js/src/prelude/aria-template.ts`), so a template parses, and
fails to parse, with the same messages as in Playwright.

The prelude's sources import `yaml` by its package name, as upstream does, and type it from the
npm package's own declarations (`neo-playwright-js` lists the repository's `yaml` as a
devDependency, for its types only; it is a patch release past these files). The build (`crates/neo-sdk/neo-playwright-js/scripts/build.ts`)
resolves that import to `browser/dist/index.js` here, so the bundle never reads `node_modules`.

No file here is edited by hand. `sync.sh` writes `SHA256SUMS`, and the crate's
`tests/engine_sources.rs` fails when a file differs from it or is missing from it. The
repository ignores `dist` directories; `packages/browseros-agent/.gitignore` re-includes this one.

## What is here

`browser/dist/`, every module of it: `index.js` re-exports them all, and the bundler keeps the
ones `parseDocument`, `LineCounter`, `Scalar`, `YAMLMap` and `YAMLSeq` reach. The build swaps
`browser/dist/log.js` for `crates/neo-sdk/neo-playwright-js/src/stubs/yaml-log.ts` at
resolution, as it swaps Playwright's inspector modules: its logger writes to a global `console`
that the prelude's QuickJS realm does not have, and the aria parser never reaches it.

## What is left out

`dist/` (the CommonJS build for Node, with its type declarations), `browser/index.js` (a
default-export wrapper around `browser/dist/index.js`), `bin.mjs` and `util.js`.
