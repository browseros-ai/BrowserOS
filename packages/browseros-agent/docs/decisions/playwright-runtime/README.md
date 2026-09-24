# Playwright runtime for BrowserOS neo — design record

Companion material for the decision in
[`../2026-09-23-playwright-runtime.md`](../2026-09-23-playwright-runtime.md)
and the build report in
[`../2026-09-24-playwright-runtime-morning-report.md`](../2026-09-24-playwright-runtime-morning-report.md).
Everything here was produced during the overnight design-off and build on
2026-09-23/24; it is kept so the team can see the alternatives that lost and
the evidence behind the choice.

| Document | What it is |
|---|---|
| [architecture-review.md](architecture-review.md) | Scan of the script-execution path (run tool, script hook, effects, audit) with deepening candidates and the seam a second engine would plug into. |
| [host-seam-script-engine-port.md](host-seam-script-engine-port.md) | The additive `ScriptHost` / `ScriptEngine` / `ScriptDispatch` contract, deferred until a second engine exists. |
| [design-a-inprocess-rustwright.md](design-a-inprocess-rustwright.md) | Design A: in-process facade over a rustwright-core fork. Lost on fidelity and public-surface gaps. |
| [design-b-sidecar-playwright-core.md](design-b-sidecar-playwright-core.md) | Design B: real `playwright-core` in a Bun-compiled sidecar with a host CDP relay. Lost on shipping cost, pin fragility and sandboxing. |
| [design-c-vendored-injected-script.md](design-c-vendored-injected-script.md) | Design C: Playwright's own injected script vendored into an isolated world over our CDP session. **Chosen.** |
| [spike-rustwright.md](spike-rustwright.md), [spike-rustwright-gap-closing.md](spike-rustwright-gap-closing.md) | Measured rustwright in-process: works with caveats; closing the gaps needs a fork of a 64k-line file. |
| [spike-playwright-core-sidecar.md](spike-playwright-core-sidecar.md) | Measured `playwright-core` over CDP under Node and Bun: works with caveats; 64 MB sidecar, focus stealing, `node:vm` escape. |
| [spike-injected-script-standalone.md](spike-injected-script-standalone.md) | Proved the injected script runs standalone in a Chromium 151 isolated world; exact bootstrap, constructor and `expect` contracts. |
| [conformance-corpus.md](conformance-corpus.md), [conformance-expected-fidelity.md](conformance-expected-fidelity.md) | The agent-style scripts used as the acceptance bar (now `contracts/claw-mcp/tests/cases-playwright.ts`) and the fidelity checklist. |
| `images/` | Cockpit screenshots from a real session on the built branch. |
