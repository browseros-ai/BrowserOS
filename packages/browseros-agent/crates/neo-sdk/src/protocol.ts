/**
 * Browser-call params as Playwright's client validator leaves them (`validatorPrimitives.ts`
 * `tObject`): only the keys the method's schema declares, in schema order, without `undefined`
 * values. So `timeout` and unknown option keys never reach Rust, and Neo's messages carry the
 * same params as Playwright's.
 *
 * Values are not validated here: Rust decodes params with the spec's types and replies `Error`
 * when they do not fit, worded by serde rather than by Playwright's validator.
 */

import type {
  FrameAriaSnapshotParams,
  FrameCheckParams,
  FrameClickParams,
  FrameFillParams,
  FrameGotoParams,
  FrameInputValueParams,
  FramePressParams,
  FrameQueryCountParams,
  FrameSelectOptionParams,
  FrameTextContentParams,
  FrameUncheckParams,
  PageCloseParams,
} from '@browseros/neo-runtime-spec/serve'

/** `Frame.goto`'s params in Playwright's schema order (`spec/playwright/frame.yml`). */
export const FRAME_GOTO_KEYS = ['url', 'waitUntil', 'referer'] as const

/** `Frame.textContent`'s params in Playwright's schema order. */
export const FRAME_TEXT_CONTENT_KEYS = ['selector', 'strict'] as const

/** `Frame.queryCount`'s params in Playwright's schema order. */
export const FRAME_QUERY_COUNT_KEYS = ['selector'] as const

/** `Frame.ariaSnapshot`'s params in Playwright's schema order. */
export const FRAME_ARIA_SNAPSHOT_KEYS = [
  'mode',
  'selector',
  'depth',
  'boxes',
] as const

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

/** `Frame.fill` in Playwright schema order. */
export const FRAME_FILL_KEYS = ['selector', 'strict', 'value', 'force'] as const

/** `Frame.press` in Playwright schema order. */
export const FRAME_PRESS_KEYS = [
  'selector',
  'strict',
  'key',
  'delay',
  'noWaitAfter',
] as const

/** `Frame.inputValue` in Playwright schema order. */
export const FRAME_INPUT_VALUE_KEYS = ['selector', 'strict'] as const

/** `Frame.check` in Playwright schema order. */
export const FRAME_CHECK_KEYS = [
  'selector',
  'strict',
  'force',
  'scroll',
  'position',
  'trial',
] as const

/** `Frame.uncheck` in Playwright schema order. */
export const FRAME_UNCHECK_KEYS = [
  'selector',
  'strict',
  'force',
  'scroll',
  'position',
  'trial',
] as const

/** `Frame.selectOption` in Playwright schema order. */
export const FRAME_SELECT_OPTION_KEYS = [
  'selector',
  'strict',
  'elements',
  'options',
  'force',
] as const

// A type cannot carry key order, so the order is a list, and the build fails when a list and
// its generated params type disagree on the key set in either direction. The aliases are never
// used; declaring them is the check.
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
type _FrameGotoKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_GOTO_KEYS)[number], true>,
    Required<FrameGotoParams>
  >
>
type _FrameTextContentKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_TEXT_CONTENT_KEYS)[number], true>,
    Required<FrameTextContentParams>
  >
>
type _FrameAriaSnapshotKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_ARIA_SNAPSHOT_KEYS)[number], true>,
    Required<FrameAriaSnapshotParams>
  >
>

type _FrameQueryCountKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_QUERY_COUNT_KEYS)[number], true>,
    Required<FrameQueryCountParams>
  >
>

type _FrameFillKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_FILL_KEYS)[number], true>,
    Required<FrameFillParams>
  >
>
type _FramePressKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_PRESS_KEYS)[number], true>,
    Required<FramePressParams>
  >
>
type _FrameInputValueKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_INPUT_VALUE_KEYS)[number], true>,
    Required<FrameInputValueParams>
  >
>
type _FrameCheckKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_CHECK_KEYS)[number], true>,
    Required<FrameCheckParams>
  >
>
type _FrameUncheckKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_UNCHECK_KEYS)[number], true>,
    Required<FrameUncheckParams>
  >
>
type _FrameSelectOptionKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_SELECT_OPTION_KEYS)[number], true>,
    Required<FrameSelectOptionParams>
  >
>

/** `Page.close`'s params (`spec/playwright/page.yml`). */
export const PAGE_CLOSE_KEYS = ['reason'] as const

type _PageCloseKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof PAGE_CLOSE_KEYS)[number], true>,
    Required<PageCloseParams>
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
