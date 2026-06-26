# Vendored third-party scripts

Files in this directory are committed verbatim from upstream npm releases. They
are injected as inline JavaScript into agent-driven BrowserOS pages via CDP
`Page.addScriptToEvaluateOnNewDocument`, so the cockpit must own the exact
bytes that run inside those pages. Pinning to source control makes the
recorder behaviour deterministic across machines and prevents a transient
`bun install` from shifting recorder semantics.

## rrweb.umd.min.js

- **Source:** [rrweb 2.0.1](https://www.npmjs.com/package/rrweb/v/2.0.1) `dist/rrweb.umd.min.cjs`
- **License:** MIT (see https://github.com/rrweb-io/rrweb/blob/master/LICENSE)
- **Size:** ~260 KB
- **Exposes:** the `rrweb` UMD global with `.record(...)` and `.Replayer`

The same UMD ships both the recorder and the replayer. We only invoke
`rrweb.record` from the injected init script; the replayer is reached from
the cockpit UI via the `rrweb-player` npm package (not from this file).

## Refreshing a vendored bundle

Run the refresh script with the desired npm version. The script fetches the
tarball, verifies the contents, copies the chosen file into this directory,
and updates the version line in this README. Honour the package's bunfig
minimum-release-age policy (do not pull a version under seven days old) so
recorder updates land deliberately, not by accident.

```bash
bun run scripts/refresh-rrweb-bundle.ts --version 2.0.1
```
