/**
 * Browser-call params as Playwright's client validator leaves them (`validatorPrimitives.ts`
 * `tObject`): only the keys the method's schema declares, in schema order, without `undefined`
 * values. So `timeout` and unknown option keys never reach Rust, and Neo's messages carry the
 * same params as Playwright's.
 *
 * Values are not validated here, except `null`: Rust decodes params with the spec's types and
 * replies `Error` when they do not fit, worded by serde rather than by Playwright's validator.
 */

import type {
  FrameAriaSnapshotParams,
  FrameCheckParams,
  FrameClickParams,
  FrameDblclickParams,
  FrameFillParams,
  FrameGetAttributeParams,
  FrameGotoParams,
  FrameHoverParams,
  FrameInnerTextParams,
  FrameInputValueParams,
  FrameIsCheckedParams,
  FrameIsVisibleParams,
  FramePressParams,
  FrameQueryCountParams,
  FrameSelectOptionParams,
  FrameTapParams,
  FrameTextContentParams,
  FrameUncheckParams,
  FrameWaitForSelectorParams,
  PageCloseParams,
  PageGoBackParams,
  PageGoForwardParams,
  PageReloadParams,
} from '@browseros/neo-runtime-spec/serve'

/** `Frame.goto`'s params in Playwright's schema order (`spec/playwright/frame.yml`). */
export const FRAME_GOTO_KEYS = ['url', 'waitUntil', 'referer'] as const

/** `Frame.textContent`'s params in Playwright's schema order. */
export const FRAME_TEXT_CONTENT_KEYS = ['selector', 'strict'] as const

/** `Frame.innerText`'s params in Playwright's schema order. */
export const FRAME_INNER_TEXT_KEYS = ['selector', 'strict'] as const

/** `Frame.getAttribute`'s params in Playwright's schema order. */
export const FRAME_GET_ATTRIBUTE_KEYS = ['selector', 'strict', 'name'] as const

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

/** `Frame.dblclick` in Playwright schema order: no `clickCount` and no `noWaitAfter`. */
export const FRAME_DBLCLICK_KEYS = [
  'selector',
  'strict',
  'force',
  'scroll',
  'modifiers',
  'position',
  'delay',
  'button',
  'trial',
  'steps',
] as const

/** `Frame.hover` in Playwright schema order: no `noWaitAfter`. */
export const FRAME_HOVER_KEYS = [
  'selector',
  'strict',
  'force',
  'scroll',
  'modifiers',
  'position',
  'trial',
] as const

/** `Frame.tap` in Playwright schema order: no `noWaitAfter`. */
export const FRAME_TAP_KEYS = [
  'selector',
  'strict',
  'force',
  'scroll',
  'modifiers',
  'position',
  'trial',
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

/** `Frame.isChecked` in Playwright schema order. */
export const FRAME_IS_CHECKED_KEYS = ['selector', 'strict'] as const

/** `Frame.isVisible` in Playwright schema order. */
export const FRAME_IS_VISIBLE_KEYS = ['selector', 'strict'] as const

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

/** `Frame.waitForSelector` in Playwright schema order. */
export const FRAME_WAIT_FOR_SELECTOR_KEYS = [
  'selector',
  'strict',
  'state',
  'omitReturnValue',
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
type _FrameDblclickKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_DBLCLICK_KEYS)[number], true>,
    Required<FrameDblclickParams>
  >
>
type _FrameHoverKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_HOVER_KEYS)[number], true>,
    Required<FrameHoverParams>
  >
>
type _FrameTapKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_TAP_KEYS)[number], true>,
    Required<FrameTapParams>
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
type _FrameInnerTextKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_INNER_TEXT_KEYS)[number], true>,
    Required<FrameInnerTextParams>
  >
>
type _FrameGetAttributeKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_GET_ATTRIBUTE_KEYS)[number], true>,
    Required<FrameGetAttributeParams>
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
type _FrameIsCheckedKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_IS_CHECKED_KEYS)[number], true>,
    Required<FrameIsCheckedParams>
  >
>
type _FrameIsVisibleKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_IS_VISIBLE_KEYS)[number], true>,
    Required<FrameIsVisibleParams>
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
type _FrameWaitForSelectorKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_WAIT_FOR_SELECTOR_KEYS)[number], true>,
    Required<FrameWaitForSelectorParams>
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

/** `Page.reload`'s params (`spec/playwright/page.yml`). */
export const PAGE_RELOAD_KEYS = ['waitUntil'] as const

/** `Page.goBack`'s params (`spec/playwright/page.yml`). */
export const PAGE_GO_BACK_KEYS = ['waitUntil'] as const

/** `Page.goForward`'s params (`spec/playwright/page.yml`). */
export const PAGE_GO_FORWARD_KEYS = ['waitUntil'] as const

type _PageReloadKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof PAGE_RELOAD_KEYS)[number], true>,
    Required<PageReloadParams>
  >
>
type _PageGoBackKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof PAGE_GO_BACK_KEYS)[number], true>,
    Required<PageGoBackParams>
  >
>
type _PageGoForwardKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof PAGE_GO_FORWARD_KEYS)[number], true>,
    Required<PageGoForwardParams>
  >
>

/**
 * `options` reduced to `keys`, in `keys` order, without `undefined` values. A key set to `null`
 * throws a plain `Error` before anything is sent, as Playwright's validator does: each of these
 * keys is required or `tOptional`, which lets only `undefined` through. Rust's serde would read
 * the `null` as an absent option and run the call.
 */
export function pickParams<Keys extends readonly string[]>(
  keys: Keys,
  options: Record<string, unknown>,
): { [K in Keys[number]]?: unknown } {
  const params: { [K in Keys[number]]?: unknown } = {}
  for (const key of keys as readonly Keys[number][]) {
    const value = options[key]
    if (value === null) throw new Error(`${key}: expected a value, got null`)
    if (value !== undefined) params[key] = value
  }
  return params
}
