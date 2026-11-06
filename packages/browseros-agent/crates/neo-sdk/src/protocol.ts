/**
 * Browser-call params as Playwright's client validator leaves them (`validatorPrimitives.ts`
 * `tObject`): only the keys the method's schema declares, in schema order, without `undefined`
 * values. So `timeout` and unknown option keys never reach Rust, and Neo's messages carry the
 * same params as Playwright's.
 *
 * Values are not validated here: Rust decodes params with the spec's types and replies `Error`
 * when they do not fit, worded by serde rather than by Playwright's validator.
 */

import type { FrameClickParams } from '@browseros/neo-runtime-spec/serve'

/** `Frame.click`'s params in Playwright's schema order (`spec/playwright/frame.yml`). */
export const FRAME_CLICK_KEYS = [
  'selector',
  'strict',
  'force',
  'scroll',
  'noWaitAfter',
  'modifiers',
  'position',
  'delay',
  'button',
  'clickCount',
  'trial',
  'steps',
] as const

// A type cannot carry key order, so the order is a list, and the build fails when the list and
// the generated `FrameClickParams` disagree on the key set in either direction. The alias is
// never used; declaring it is the check.
type SameKeys<A, B> = [keyof A] extends [keyof B]
  ? [keyof B] extends [keyof A]
    ? true
    : false
  : false
type Assert<T extends true> = T
type _FrameClickKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_CLICK_KEYS)[number], true>,
    Required<FrameClickParams>
  >
>

/** `options` reduced to `keys`, in `keys` order, without `undefined` values. */
export function pickParams<Keys extends readonly string[]>(
  keys: Keys,
  options: Record<string, unknown>,
): { [K in Keys[number]]?: unknown } {
  const params: { [K in Keys[number]]?: unknown } = {}
  for (const key of keys as readonly Keys[number][]) {
    const value = options[key]
    if (value !== undefined) params[key] = value
  }
  return params
}
