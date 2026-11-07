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
  BrowserContextCloseParams,
  FrameAriaSnapshotJSONParams,
  FrameAriaSnapshotParams,
  FrameBlurParams,
  FrameCheckParams,
  FrameClickParams,
  FrameContentParams,
  FrameDblclickParams,
  FrameDispatchEventParams,
  FrameDragAndDropParams,
  FrameFillParams,
  FrameFocusParams,
  FrameGetAttributeParams,
  FrameGotoParams,
  FrameHideHighlightParams,
  FrameHighlightParams,
  FrameHoverParams,
  FrameInnerHTMLParams,
  FrameInnerTextParams,
  FrameInputValueParams,
  FrameIsCheckedParams,
  FrameIsDisabledParams,
  FrameIsEditableParams,
  FrameIsEnabledParams,
  FrameIsHiddenParams,
  FrameIsVisibleParams,
  FramePressParams,
  FrameQueryCountParams,
  FrameResolveSelectorParams,
  FrameSelectOptionParams,
  FrameSetContentParams,
  FrameTapParams,
  FrameTextContentParams,
  FrameTypeParams,
  FrameUncheckParams,
  FrameWaitForSelectorParams,
  FrameWaitForTimeoutParams,
  PageCloseParams,
  PageConsoleMessagesParams,
  PageGoBackParams,
  PageGoForwardParams,
  PageKeyboardDownParams,
  PageKeyboardInsertTextParams,
  PageKeyboardPressParams,
  PageKeyboardTypeParams,
  PageKeyboardUpParams,
  PageMouseClickParams,
  PageMouseDownParams,
  PageMouseMoveParams,
  PageMouseUpParams,
  PageMouseWheelParams,
  PagePageErrorsParams,
  PagePdfParams,
  PageRegisterLocatorHandlerParams,
  PageReloadParams,
  PageResolveLocatorHandlerNoReplyParams,
  PageScreenshotParams,
  PageTouchscreenTapParams,
  PageUnregisterLocatorHandlerParams,
} from '@browseros/neo-runtime-spec/serve'
import type {
  ElementScreenshotParams,
  NeoScreenshotParams,
} from './neo-commands'

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

/** `Frame.content` in Playwright schema order. */
export const FRAME_CONTENT_KEYS = ['includeShadow'] as const

/** `Frame.setContent` in Playwright schema order. */
export const FRAME_SET_CONTENT_KEYS = ['html', 'waitUntil'] as const

/** `Frame.waitForTimeout` in Playwright schema order. */
export const FRAME_WAIT_FOR_TIMEOUT_KEYS = ['waitTimeout'] as const

/** `Frame.resolveSelector` in Playwright schema order. */
export const FRAME_RESOLVE_SELECTOR_KEYS = ['selector', 'sdkLanguage'] as const

/** `Frame.ariaSnapshotJSON` in Playwright schema order. */
export const FRAME_ARIA_SNAPSHOT_JSON_KEYS = [
  'mode',
  'selector',
  'depth',
  'boxes',
] as const

type _FrameContentKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_CONTENT_KEYS)[number], true>,
    Required<FrameContentParams>
  >
>
type _FrameSetContentKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_SET_CONTENT_KEYS)[number], true>,
    Required<FrameSetContentParams>
  >
>
type _FrameWaitForTimeoutKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_WAIT_FOR_TIMEOUT_KEYS)[number], true>,
    Required<FrameWaitForTimeoutParams>
  >
>
type _FrameResolveSelectorKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_RESOLVE_SELECTOR_KEYS)[number], true>,
    Required<FrameResolveSelectorParams>
  >
>
type _FrameAriaSnapshotJsonKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_ARIA_SNAPSHOT_JSON_KEYS)[number], true>,
    Required<FrameAriaSnapshotJSONParams>
  >
>

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

/** `Page.keyboardDown` in Playwright schema order (`spec/playwright/page.yml`). */
export const PAGE_KEYBOARD_DOWN_KEYS = ['key'] as const

/** `Page.keyboardUp` in Playwright schema order. */
export const PAGE_KEYBOARD_UP_KEYS = ['key'] as const

/** `Page.keyboardInsertText` in Playwright schema order. */
export const PAGE_KEYBOARD_INSERT_TEXT_KEYS = ['text'] as const

/** `Page.keyboardType` in Playwright schema order. */
export const PAGE_KEYBOARD_TYPE_KEYS = ['text', 'delay'] as const

/** `Page.keyboardPress` in Playwright schema order. */
export const PAGE_KEYBOARD_PRESS_KEYS = ['key', 'delay'] as const

/** `Page.mouseMove` in Playwright schema order. */
export const PAGE_MOUSE_MOVE_KEYS = ['x', 'y', 'steps'] as const

/** `Page.mouseDown` in Playwright schema order. */
export const PAGE_MOUSE_DOWN_KEYS = ['button', 'clickCount'] as const

/** `Page.mouseUp` in Playwright schema order. */
export const PAGE_MOUSE_UP_KEYS = ['button', 'clickCount'] as const

/** `Page.mouseClick` in Playwright schema order: no `steps`. */
export const PAGE_MOUSE_CLICK_KEYS = [
  'x',
  'y',
  'delay',
  'button',
  'clickCount',
] as const

/** `Page.mouseWheel` in Playwright schema order. */
export const PAGE_MOUSE_WHEEL_KEYS = ['deltaX', 'deltaY'] as const

/** `Page.touchscreenTap` in Playwright schema order. */
export const PAGE_TOUCHSCREEN_TAP_KEYS = ['x', 'y'] as const

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

/** `Frame.type` in Playwright schema order. */
export const FRAME_TYPE_KEYS = ['selector', 'strict', 'text', 'delay'] as const

/** `Frame.dragAndDrop` in Playwright schema order. */
export const FRAME_DRAG_AND_DROP_KEYS = [
  'source',
  'target',
  'force',
  'scroll',
  'trial',
  'sourcePosition',
  'targetPosition',
  'strict',
  'steps',
] as const

/** `Frame.inputValue` in Playwright schema order. */
export const FRAME_INPUT_VALUE_KEYS = ['selector', 'strict'] as const

/** `Frame.isChecked` in Playwright schema order. */
export const FRAME_IS_CHECKED_KEYS = ['selector', 'strict'] as const

/** `Frame.isVisible` in Playwright schema order. */
export const FRAME_IS_VISIBLE_KEYS = ['selector', 'strict'] as const

/** `Frame.isHidden` in Playwright schema order. */
export const FRAME_IS_HIDDEN_KEYS = ['selector', 'strict'] as const

/** `Frame.isEnabled` in Playwright schema order. */
export const FRAME_IS_ENABLED_KEYS = ['selector', 'strict'] as const

/** `Frame.isDisabled` in Playwright schema order. */
export const FRAME_IS_DISABLED_KEYS = ['selector', 'strict'] as const

/** `Frame.isEditable` in Playwright schema order. */
export const FRAME_IS_EDITABLE_KEYS = ['selector', 'strict'] as const

/** `Frame.innerHTML` in Playwright schema order. */
export const FRAME_INNER_HTML_KEYS = ['selector', 'strict'] as const

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
type _PageKeyboardDownKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof PAGE_KEYBOARD_DOWN_KEYS)[number], true>,
    Required<PageKeyboardDownParams>
  >
>
type _PageKeyboardUpKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof PAGE_KEYBOARD_UP_KEYS)[number], true>,
    Required<PageKeyboardUpParams>
  >
>
type _PageKeyboardInsertTextKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof PAGE_KEYBOARD_INSERT_TEXT_KEYS)[number], true>,
    Required<PageKeyboardInsertTextParams>
  >
>
type _PageKeyboardTypeKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof PAGE_KEYBOARD_TYPE_KEYS)[number], true>,
    Required<PageKeyboardTypeParams>
  >
>
type _PageKeyboardPressKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof PAGE_KEYBOARD_PRESS_KEYS)[number], true>,
    Required<PageKeyboardPressParams>
  >
>
type _PageMouseMoveKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof PAGE_MOUSE_MOVE_KEYS)[number], true>,
    Required<PageMouseMoveParams>
  >
>
type _PageMouseDownKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof PAGE_MOUSE_DOWN_KEYS)[number], true>,
    Required<PageMouseDownParams>
  >
>
type _PageMouseUpKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof PAGE_MOUSE_UP_KEYS)[number], true>,
    Required<PageMouseUpParams>
  >
>
type _PageMouseClickKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof PAGE_MOUSE_CLICK_KEYS)[number], true>,
    Required<PageMouseClickParams>
  >
>
type _PageMouseWheelKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof PAGE_MOUSE_WHEEL_KEYS)[number], true>,
    Required<PageMouseWheelParams>
  >
>
type _PageTouchscreenTapKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof PAGE_TOUCHSCREEN_TAP_KEYS)[number], true>,
    Required<PageTouchscreenTapParams>
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
type _FrameDragAndDropKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_DRAG_AND_DROP_KEYS)[number], true>,
    Required<FrameDragAndDropParams>
  >
>
type _FrameTypeKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_TYPE_KEYS)[number], true>,
    Required<FrameTypeParams>
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
type _FrameIsHiddenKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_IS_HIDDEN_KEYS)[number], true>,
    Required<FrameIsHiddenParams>
  >
>
type _FrameIsEnabledKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_IS_ENABLED_KEYS)[number], true>,
    Required<FrameIsEnabledParams>
  >
>
type _FrameIsDisabledKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_IS_DISABLED_KEYS)[number], true>,
    Required<FrameIsDisabledParams>
  >
>
type _FrameIsEditableKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_IS_EDITABLE_KEYS)[number], true>,
    Required<FrameIsEditableParams>
  >
>
type _FrameInnerHtmlKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_INNER_HTML_KEYS)[number], true>,
    Required<FrameInnerHTMLParams>
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

/** `Frame.focus` in Playwright schema order. */
export const FRAME_FOCUS_KEYS = ['selector', 'strict'] as const

/** `Frame.blur` in Playwright schema order. */
export const FRAME_BLUR_KEYS = ['selector', 'strict'] as const

/** `Frame.dispatchEvent` in Playwright schema order. */
export const FRAME_DISPATCH_EVENT_KEYS = [
  'selector',
  'strict',
  'type',
  'eventInit',
] as const

/** `Frame.highlight` in Playwright schema order. */
export const FRAME_HIGHLIGHT_KEYS = ['selector', 'style'] as const

/** `Frame.hideHighlight` in Playwright schema order. */
export const FRAME_HIDE_HIGHLIGHT_KEYS = ['selector'] as const

type _FrameFocusKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_FOCUS_KEYS)[number], true>,
    Required<FrameFocusParams>
  >
>
type _FrameBlurKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_BLUR_KEYS)[number], true>,
    Required<FrameBlurParams>
  >
>
type _FrameDispatchEventKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_DISPATCH_EVENT_KEYS)[number], true>,
    Required<FrameDispatchEventParams>
  >
>
type _FrameHighlightKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_HIGHLIGHT_KEYS)[number], true>,
    Required<FrameHighlightParams>
  >
>
type _FrameHideHighlightKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof FRAME_HIDE_HIGHLIGHT_KEYS)[number], true>,
    Required<FrameHideHighlightParams>
  >
>

/** `BrowserContext.close`'s params (`spec/playwright/browserContext.yml`). */
export const BROWSER_CONTEXT_CLOSE_KEYS = ['reason'] as const

type _BrowserContextCloseKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof BROWSER_CONTEXT_CLOSE_KEYS)[number], true>,
    Required<BrowserContextCloseParams>
  >
>

/** `Page.close`'s params (`spec/playwright/page.yml`). */
export const PAGE_CLOSE_KEYS = ['reason'] as const

/** `Page.screenshot` in Playwright schema order (`spec/playwright/page.yml`, then the
 * `CommonScreenshotOptions` mixin). */
export const PAGE_SCREENSHOT_KEYS = [
  'type',
  'quality',
  'fullPage',
  'clip',
  'omitBackground',
  'caret',
  'animations',
  'scale',
  'mask',
  'maskColor',
  'style',
] as const

/** `Page.pdf` in Playwright schema order. */
export const PAGE_PDF_KEYS = [
  'scale',
  'displayHeaderFooter',
  'headerTemplate',
  'footerTemplate',
  'printBackground',
  'landscape',
  'pageRanges',
  'format',
  'width',
  'height',
  'preferCSSPageSize',
  'margin',
  'tagged',
  'outline',
] as const

/** `ElementHandle.screenshot` in Playwright schema order (`spec/playwright/handles.yml`);
 * Neo's `path` is added after. */
export const ELEMENT_SCREENSHOT_KEYS = [
  'type',
  'quality',
  'omitBackground',
  'caret',
  'animations',
  'scale',
  'mask',
  'maskColor',
  'style',
] as const

/** `neo:screenshot`: the selector, then `ElementHandle.screenshot`'s params. */
export const NEO_SCREENSHOT_KEYS = [
  'selector',
  ...ELEMENT_SCREENSHOT_KEYS,
] as const

/** `Page.registerLocatorHandler` in Playwright schema order. */
export const PAGE_REGISTER_LOCATOR_HANDLER_KEYS = [
  'selector',
  'noWaitAfter',
] as const

/** `Page.resolveLocatorHandlerNoReply` in Playwright schema order. */
export const PAGE_RESOLVE_LOCATOR_HANDLER_KEYS = ['uid', 'remove'] as const

/** `Page.unregisterLocatorHandler` in Playwright schema order. */
export const PAGE_UNREGISTER_LOCATOR_HANDLER_KEYS = ['uid'] as const

type _PageRegisterLocatorHandlerKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof PAGE_REGISTER_LOCATOR_HANDLER_KEYS)[number], true>,
    Required<PageRegisterLocatorHandlerParams>
  >
>
type _PageResolveLocatorHandlerKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof PAGE_RESOLVE_LOCATOR_HANDLER_KEYS)[number], true>,
    Required<PageResolveLocatorHandlerNoReplyParams>
  >
>
type _PageUnregisterLocatorHandlerKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof PAGE_UNREGISTER_LOCATOR_HANDLER_KEYS)[number], true>,
    Required<PageUnregisterLocatorHandlerParams>
  >
>

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

/** `Page.consoleMessages`'s params (`spec/playwright/page.yml`). */
export const PAGE_CONSOLE_MESSAGES_KEYS = ['filter'] as const

type _PageConsoleMessagesKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof PAGE_CONSOLE_MESSAGES_KEYS)[number], true>,
    Required<PageConsoleMessagesParams>
  >
>

/** `Page.pageErrors`'s params (`spec/playwright/page.yml`). */
export const PAGE_PAGE_ERRORS_KEYS = ['filter'] as const

type _PagePageErrorsKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof PAGE_PAGE_ERRORS_KEYS)[number], true>,
    Required<PagePageErrorsParams>
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
type _PageScreenshotKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof PAGE_SCREENSHOT_KEYS)[number], true>,
    Required<PageScreenshotParams>
  >
>
type _PagePdfKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof PAGE_PDF_KEYS)[number], true>,
    Required<PagePdfParams>
  >
>
type _ElementScreenshotKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof ELEMENT_SCREENSHOT_KEYS)[number] | 'path', true>,
    Required<ElementScreenshotParams>
  >
>
type _NeoScreenshotKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof NEO_SCREENSHOT_KEYS)[number] | 'path', true>,
    Required<NeoScreenshotParams>
  >
>
