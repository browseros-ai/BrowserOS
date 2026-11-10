/**
 * Neo: the page globals Playwright's vendored `isomorphic/utilityScriptSerializers.ts` probes
 * before using them (`typeof globalThis.Window === 'function' && value instanceof …`), declared
 * for the type checker only. The realm has none of them, which those `typeof` checks handle at
 * run time; the SDK itself must not use them.
 */

declare var Window: (abstract new (...args: never[]) => unknown) | undefined
declare var Document: (abstract new (...args: never[]) => unknown) | undefined
declare var Node: (abstract new (...args: never[]) => unknown) | undefined
