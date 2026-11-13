/**
 * The conformance switch. Playwright's own spec tests pin its exact output, so the spec
 * harness (neo-playwright-tests/node/claw-loader.mjs) sets `__neo_upstream_snapshots` on the
 * prelude's globalThis: then `ariaSnapshot()` with no mode means `'default'` and `mode: 'ai'`
 * prints Playwright's text, both as they did before the v1 format. Production hosts never set
 * it. The common call header also carries this bypass to assertion comparisons, failure
 * trees, previews and received-value diagnostics. Read once, as the prelude starts and before the agent's function exists, so a script that
 * sets the global changes nothing.
 */
export const upstreamSnapshots: boolean =
  Reflect.get(globalThis, '__neo_upstream_snapshots') === true
