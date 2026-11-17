/**
 * Stands in for the vendored yaml's `log.js` in the prelude's bundle. Its `debug` and `warn`
 * write to `console`, which Neo's QuickJS realm has no global for: the bootstrap hands `console`
 * to the agent's function as a parameter. yaml logs only from `parse()` and `toJS`, and the aria
 * template parser (src/prelude/aria-template.ts) calls neither, so there is nothing to log.
 */
export function debug(): void {}

export function warn(): void {}
