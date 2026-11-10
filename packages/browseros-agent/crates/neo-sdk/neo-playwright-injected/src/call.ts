/**
 * The bundle entry `dist/call.js`: the calling convention of every page call. neo-runtime passes
 * it as `Runtime.callFunctionOn`'s `functionDeclaration` with the glue object as `objectId`, so
 * `this` is the glue object and the arguments are `[name, params, ...handles]` (handles as
 * `{objectId}` arguments, which CDP turns back into the page objects they name).
 *
 * It lives here, not as a string in Rust, so every byte of JavaScript that runs in the page is
 * built, typed and checked in this package.
 */

import type { GlueObject } from './install'

export default function call(
  this: GlueObject,
  name: keyof GlueObject,
  params: unknown,
  ...handles: unknown[]
): unknown {
  return this[name](params, ...handles)
}
