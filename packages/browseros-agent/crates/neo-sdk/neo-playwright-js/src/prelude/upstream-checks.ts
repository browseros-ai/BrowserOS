/**
 * Every wire op is checked against Playwright 1.64.1 or classified below as client/Neo work.
 * The reasons are the migration inventory for Rust-owned generated checks; every remaining
 * deviation describes an intentional difference from upstream rather than a loose declaration.
 */
import type * as pw from '../../../neo-playwright/third_party/playwright/packages/playwright-core/src/client/channels'
import type { GeneratedInputs } from './generated'
import type { OpName } from './generated/ops'
import type {
  FILL_KEYS,
  PRESS_KEYS,
  SCREENSHOT_KEYS,
  TYPE_KEYS,
} from './options'
import type { Assert, KeysCheck, UpstreamCheck } from './upstream-check'
import type { OpInputs } from './wire'

export type UpstreamChecks = {
  // Neo leases file-chooser interception per run; upstream subscribes by event name and enabled flag.
  'events.interceptFileChooser': Assert<
    UpstreamCheck<
      'events.interceptFileChooser',
      OpInputs['events.interceptFileChooser'],
      pw.PageUpdateSubscriptionParams,
      'lease' | 'on',
      'enabled' | 'event',
      never
    >
  >
  'locator.fill': Assert<
    UpstreamCheck<
      'locator.fill',
      OpInputs['locator.fill'],
      pw.ElementHandleFillParams,
      never,
      never,
      never
    >
  >
  'locator.press': Assert<
    UpstreamCheck<
      'locator.press',
      OpInputs['locator.press'],
      pw.ElementHandlePressParams,
      never,
      never,
      never
    >
  >
  'locator.pressSequentially': Assert<
    UpstreamCheck<
      'locator.pressSequentially',
      OpInputs['locator.pressSequentially'],
      pw.ElementHandleTypeParams,
      never,
      never,
      never
    >
  >
  // The probe receives option records directly and Neo element brands, instead of upstream channel handles.
  'locator.selectOption': Assert<
    UpstreamCheck<
      'locator.selectOption',
      OpInputs['locator.selectOption'],
      pw.ElementHandleSelectOptionParams,
      never,
      never,
      'elements'
    >
  >
  'locator.selectText': Assert<
    UpstreamCheck<
      'locator.selectText',
      OpInputs['locator.selectText'],
      pw.ElementHandleSelectTextParams,
      never,
      never,
      never
    >
  >
  'locator.focus': Assert<
    UpstreamCheck<
      'locator.focus',
      OpInputs['locator.focus'],
      pw.ElementHandleFocusParams,
      never,
      never,
      never
    >
  >
  'locator.blur': Assert<
    UpstreamCheck<
      'locator.blur',
      OpInputs['locator.blur'],
      pw.FrameBlurParams,
      never,
      never,
      never
    >
  >
  // Neo resolves paths or in-memory payloads on its host, replacing upstream transport-specific file fields.
  'locator.setInputFiles': Assert<
    UpstreamCheck<
      'locator.setInputFiles',
      OpInputs['locator.setInputFiles'],
      pw.ElementHandleSetInputFilesParams,
      'files',
      | 'directoryStream'
      | 'localDirectory'
      | 'localPaths'
      | 'payloads'
      | 'streams',
      never
    >
  >
  // v1 transports eventInit as JSON rather than upstream SerializedArgument; handles are unsupported here.
  'locator.dispatchEvent': Assert<
    UpstreamCheck<
      'locator.dispatchEvent',
      OpInputs['locator.dispatchEvent'],
      pw.ElementHandleDispatchEventParams,
      never,
      never,
      'eventInit'
    >
  >
  'locator.scrollIntoViewIfNeeded': Assert<
    UpstreamCheck<
      'locator.scrollIntoViewIfNeeded',
      OpInputs['locator.scrollIntoViewIfNeeded'],
      pw.ElementHandleScrollIntoViewIfNeededParams,
      never,
      never,
      never
    >
  >
  'locator.textContent': Assert<
    UpstreamCheck<
      'locator.textContent',
      OpInputs['locator.textContent'],
      pw.ElementHandleTextContentParams,
      never,
      never,
      never
    >
  >
  'locator.innerText': Assert<
    UpstreamCheck<
      'locator.innerText',
      OpInputs['locator.innerText'],
      pw.ElementHandleInnerTextParams,
      never,
      never,
      never
    >
  >
  'locator.innerHTML': Assert<
    UpstreamCheck<
      'locator.innerHTML',
      OpInputs['locator.innerHTML'],
      pw.ElementHandleInnerHTMLParams,
      never,
      never,
      never
    >
  >
  'locator.getAttribute': Assert<
    UpstreamCheck<
      'locator.getAttribute',
      OpInputs['locator.getAttribute'],
      pw.ElementHandleGetAttributeParams,
      never,
      never,
      never
    >
  >
  'locator.inputValue': Assert<
    UpstreamCheck<
      'locator.inputValue',
      OpInputs['locator.inputValue'],
      pw.ElementHandleInputValueParams,
      never,
      never,
      never
    >
  >
  'locator.isEnabled': Assert<
    UpstreamCheck<
      'locator.isEnabled',
      OpInputs['locator.isEnabled'],
      pw.ElementHandleIsEnabledParams,
      never,
      never,
      never
    >
  >
  'locator.isDisabled': Assert<
    UpstreamCheck<
      'locator.isDisabled',
      OpInputs['locator.isDisabled'],
      pw.ElementHandleIsDisabledParams,
      never,
      never,
      never
    >
  >
  'locator.isEditable': Assert<
    UpstreamCheck<
      'locator.isEditable',
      OpInputs['locator.isEditable'],
      pw.ElementHandleIsEditableParams,
      never,
      never,
      never
    >
  >
  'locator.isChecked': Assert<
    UpstreamCheck<
      'locator.isChecked',
      OpInputs['locator.isChecked'],
      pw.ElementHandleIsCheckedParams,
      never,
      never,
      never
    >
  >
  'locator.isVisible': Assert<
    UpstreamCheck<
      'locator.isVisible',
      OpInputs['locator.isVisible'],
      pw.ElementHandleIsVisibleParams,
      never,
      never,
      never
    >
  >
  'locator.isHidden': Assert<
    UpstreamCheck<
      'locator.isHidden',
      OpInputs['locator.isHidden'],
      pw.ElementHandleIsHiddenParams,
      never,
      never,
      never
    >
  >
  'locator.count': Assert<
    UpstreamCheck<
      'locator.count',
      OpInputs['locator.count'],
      pw.FrameQueryCountParams,
      never,
      never,
      never
    >
  >
  // Neo carries the original selector for diagnostics after brand resolution; it always formats JavaScript, without sdkLanguage.
  'locator.normalize': Assert<
    UpstreamCheck<
      'locator.normalize',
      OpInputs['locator.normalize'],
      pw.FrameResolveSelectorParams,
      'selector',
      'sdkLanguage',
      never
    >
  >
  'locator.boundingBox': Assert<
    UpstreamCheck<
      'locator.boundingBox',
      OpInputs['locator.boundingBox'],
      pw.ElementHandleBoundingBoxParams,
      never,
      never,
      never
    >
  >
  // This op also serves waitForElementState, so state includes stable/enabled/disabled/editable.
  'locator.waitFor': Assert<
    UpstreamCheck<
      'locator.waitFor',
      OpInputs['locator.waitFor'],
      pw.FrameWaitForSelectorParams,
      never,
      never,
      'state'
    >
  >
  // noWait selects the immediate page.$ query instead of the locator wait.
  'locator.elementHandle': Assert<
    UpstreamCheck<
      'locator.elementHandle',
      OpInputs['locator.elementHandle'],
      pw.FrameQuerySelectorParams,
      'noWait',
      never,
      never
    >
  >
  'locator.elementHandles': Assert<
    UpstreamCheck<
      'locator.elementHandles',
      OpInputs['locator.elementHandles'],
      pw.FrameQuerySelectorAllParams,
      never,
      never,
      never
    >
  >
  // Neo replaces Frame.expect with probe params and preformatted matcher diagnostics, including negation in params.
  'expect.locator': Assert<
    UpstreamCheck<
      'expect.locator',
      OpInputs['expect.locator'],
      pw.FrameExpectParams,
      'expectation' | 'expected' | 'matcher' | 'params' | 'templateError',
      | 'expectedNumber'
      | 'expectedText'
      | 'expectedValue'
      | 'expression'
      | 'expressionArg'
      | 'isNot'
      | 'pseudo'
      | 'useInnerText',
      never
    >
  >
  // Neo replaces Frame.expect with probe params and preformatted matcher diagnostics, including negation in params.
  'expect.page': Assert<
    UpstreamCheck<
      'expect.page',
      OpInputs['expect.page'],
      pw.FrameExpectParams,
      'expectation' | 'expected' | 'matcher' | 'params' | 'templateError',
      | 'expectedNumber'
      | 'expectedText'
      | 'expectedValue'
      | 'expression'
      | 'expressionArg'
      | 'isNot'
      | 'pseudo'
      | 'useInnerText',
      never
    >
  >
  'keyboard.down': Assert<
    UpstreamCheck<
      'keyboard.down',
      OpInputs['keyboard.down'],
      pw.PageKeyboardDownParams,
      never,
      never,
      never
    >
  >
  'keyboard.up': Assert<
    UpstreamCheck<
      'keyboard.up',
      OpInputs['keyboard.up'],
      pw.PageKeyboardUpParams,
      never,
      never,
      never
    >
  >
  'keyboard.press': Assert<
    UpstreamCheck<
      'keyboard.press',
      OpInputs['keyboard.press'],
      pw.PageKeyboardPressParams,
      never,
      never,
      never
    >
  >
  'keyboard.type': Assert<
    UpstreamCheck<
      'keyboard.type',
      OpInputs['keyboard.type'],
      pw.PageKeyboardTypeParams,
      never,
      never,
      never
    >
  >
  'keyboard.insertText': Assert<
    UpstreamCheck<
      'keyboard.insertText',
      OpInputs['keyboard.insertText'],
      pw.PageKeyboardInsertTextParams,
      never,
      never,
      never
    >
  >
  'mouse.move': Assert<
    UpstreamCheck<
      'mouse.move',
      OpInputs['mouse.move'],
      pw.PageMouseMoveParams,
      never,
      never,
      never
    >
  >
  'mouse.down': Assert<
    UpstreamCheck<
      'mouse.down',
      OpInputs['mouse.down'],
      pw.PageMouseDownParams,
      never,
      never,
      never
    >
  >
  'mouse.up': Assert<
    UpstreamCheck<
      'mouse.up',
      OpInputs['mouse.up'],
      pw.PageMouseUpParams,
      never,
      never,
      never
    >
  >
  'mouse.click': Assert<
    UpstreamCheck<
      'mouse.click',
      OpInputs['mouse.click'],
      pw.PageMouseClickParams,
      never,
      never,
      never
    >
  >
  'mouse.wheel': Assert<
    UpstreamCheck<
      'mouse.wheel',
      OpInputs['mouse.wheel'],
      pw.PageMouseWheelParams,
      never,
      never,
      never
    >
  >
  // publication correlates host navigation receipts with the action that produced them.
  'page.goto': Assert<
    UpstreamCheck<
      'page.goto',
      OpInputs['page.goto'],
      pw.FrameGotoParams,
      'publication',
      never,
      never
    >
  >
  // publication correlates host navigation receipts with the action that produced them.
  'page.reload': Assert<
    UpstreamCheck<
      'page.reload',
      OpInputs['page.reload'],
      pw.PageReloadParams,
      'publication',
      never,
      never
    >
  >
  // publication correlates host navigation receipts with the action that produced them.
  'page.goBack': Assert<
    UpstreamCheck<
      'page.goBack',
      OpInputs['page.goBack'],
      pw.PageGoBackParams,
      'publication',
      never,
      never
    >
  >
  // publication correlates host navigation receipts with the action that produced them.
  'page.goForward': Assert<
    UpstreamCheck<
      'page.goForward',
      OpInputs['page.goForward'],
      pw.PageGoForwardParams,
      'publication',
      never,
      never
    >
  >
  'page.requestGC': Assert<
    UpstreamCheck<
      'page.requestGC',
      OpInputs['page.requestGC'],
      pw.PageRequestGCParams,
      never,
      never,
      never
    >
  >
  'page.title': Assert<
    UpstreamCheck<
      'page.title',
      OpInputs['page.title'],
      pw.FrameTitleParams,
      never,
      never,
      never
    >
  >
  // Neo returns document HTML; shadow-root serialization (includeShadow) is not exposed.
  'page.content': Assert<
    UpstreamCheck<
      'page.content',
      OpInputs['page.content'],
      pw.FrameContentParams,
      never,
      'includeShadow',
      never
    >
  >
  'page.emulateMedia': Assert<
    UpstreamCheck<
      'page.emulateMedia',
      OpInputs['page.emulateMedia'],
      pw.PageEmulateMediaParams,
      never,
      never,
      never
    >
  >
  // path is read on the Neo host rather than by an upstream client before sending content.
  'page.addScriptTag': Assert<
    UpstreamCheck<
      'page.addScriptTag',
      OpInputs['page.addScriptTag'],
      pw.FrameAddScriptTagParams,
      'path',
      never,
      never
    >
  >
  // path is read on the Neo host rather than by an upstream client before sending content.
  'page.addStyleTag': Assert<
    UpstreamCheck<
      'page.addStyleTag',
      OpInputs['page.addStyleTag'],
      pw.FrameAddStyleTagParams,
      'path',
      never,
      never
    >
  >
  'page.setContent': Assert<
    UpstreamCheck<
      'page.setContent',
      OpInputs['page.setContent'],
      pw.FrameSetContentParams,
      never,
      never,
      never
    >
  >
  // The host owns the runBeforeUnload close policy; upstream reason has no Neo wire field.
  'page.close': Assert<
    UpstreamCheck<
      'page.close',
      OpInputs['page.close'],
      pw.PageCloseParams,
      'runBeforeUnload',
      'reason',
      never
    >
  >
  // worker selects a host-owned worker; script wraps normalized expression/argument or a preflight refusal.
  'worker.evaluate': Assert<
    UpstreamCheck<
      'worker.evaluate',
      OpInputs['worker.evaluate'],
      pw.WorkerEvaluateExpressionParams,
      'script' | 'worker',
      'arg' | 'expression' | 'isFunction',
      never
    >
  >
  // script wraps normalized expression/argument or a preflight refusal instead of upstream evaluation fields.
  'page.evaluate': Assert<
    UpstreamCheck<
      'page.evaluate',
      OpInputs['page.evaluate'],
      pw.FrameEvaluateExpressionParams,
      'script',
      'arg' | 'expression' | 'isFunction',
      never
    >
  >
  // script carries normalized evaluation; noWait and selector preserve $eval timing and no-match diagnostics.
  'locator.evaluate': Assert<
    UpstreamCheck<
      'locator.evaluate',
      OpInputs['locator.evaluate'],
      pw.FrameEvalOnSelectorParams,
      'noWait' | 'script' | 'selector',
      'arg' | 'expression' | 'isFunction',
      never
    >
  >
  // script wraps normalized expression/argument or a preflight refusal instead of upstream evaluation fields.
  'locator.evaluateAll': Assert<
    UpstreamCheck<
      'locator.evaluateAll',
      OpInputs['locator.evaluateAll'],
      pw.FrameEvalOnSelectorAllParams,
      'script',
      'arg' | 'expression' | 'isFunction',
      never
    >
  >
  // Neo wraps evaluation in script and uses polling milliseconds (absent means raf), replacing pollingInterval.
  'page.waitForFunction': Assert<
    UpstreamCheck<
      'page.waitForFunction',
      OpInputs['page.waitForFunction'],
      pw.FrameWaitForFunctionParams,
      'polling' | 'script',
      'arg' | 'expression' | 'isFunction' | 'pollingInterval',
      never
    >
  >
  // Masks travel as locator brands rather than upstream frame channel/selector pairs.
  'page.screenshot': Assert<
    UpstreamCheck<
      'page.screenshot',
      OpInputs['page.screenshot'],
      pw.PageScreenshotParams,
      never,
      never,
      'mask'
    >
  >
  // Rust also accepts numeric pixel lengths; upstream's protocol only accepts the client's normalized strings.
  'page.pdf': Assert<
    UpstreamCheck<
      'page.pdf',
      OpInputs['page.pdf'],
      pw.PagePdfParams,
      never,
      never,
      'width' | 'height' | 'margin'
    >
  >
  'page.setViewportSize': Assert<
    UpstreamCheck<
      'page.setViewportSize',
      OpInputs['page.setViewportSize'],
      pw.PageSetViewportSizeParams,
      never,
      never,
      never
    >
  >
  // script is source or a path read on the Neo host, replacing upstream source.
  'page.addInitScript': Assert<
    UpstreamCheck<
      'page.addInitScript',
      OpInputs['page.addInitScript'],
      pw.PageAddInitScriptParams,
      'script',
      'source',
      never
    >
  >
  'page.exposeBinding': Assert<
    UpstreamCheck<
      'page.exposeBinding',
      OpInputs['page.exposeBinding'],
      pw.PageExposeBindingParams,
      never,
      never,
      never
    >
  >
  // Neo names an execution context and sends the JSON binding completion, replacing upstream result.
  'bindingCall.resolve': Assert<
    UpstreamCheck<
      'bindingCall.resolve',
      OpInputs['bindingCall.resolve'],
      pw.BindingCallResolveParams,
      'arg' | 'context',
      'result',
      never
    >
  >
  // One Neo op handles getItem and items; name is only required for getItem.
  'webStorage.read': Assert<
    UpstreamCheck<
      'webStorage.read',
      OpInputs['webStorage.read'],
      pw.PageWebStorageGetItemParams,
      'action',
      never,
      'name'
    >
  >
  // One Neo op handles setItem, removeItem and clear; name/value are only needed by those actions.
  'webStorage.write': Assert<
    UpstreamCheck<
      'webStorage.write',
      OpInputs['webStorage.write'],
      pw.PageWebStorageSetItemParams,
      'action',
      never,
      'name' | 'value'
    >
  >
  'page.addLocatorHandler': Assert<
    UpstreamCheck<
      'page.addLocatorHandler',
      OpInputs['page.addLocatorHandler'],
      pw.PageRegisterLocatorHandlerParams,
      never,
      never,
      never
    >
  >
  'page.removeLocatorHandler': Assert<
    UpstreamCheck<
      'page.removeLocatorHandler',
      OpInputs['page.removeLocatorHandler'],
      pw.PageUnregisterLocatorHandlerParams,
      never,
      never,
      never
    >
  >
  'page.resolveLocatorHandler': Assert<
    UpstreamCheck<
      'page.resolveLocatorHandler',
      OpInputs['page.resolveLocatorHandler'],
      pw.PageResolveLocatorHandlerNoReplyParams,
      never,
      never,
      never
    >
  >
  'context.newPage': Assert<
    UpstreamCheck<
      'context.newPage',
      OpInputs['context.newPage'],
      pw.BrowserContextNewPageParams,
      never,
      never,
      never
    >
  >
  // script is source or a path read on the Neo host, replacing upstream source.
  'context.addInitScript': Assert<
    UpstreamCheck<
      'context.addInitScript',
      OpInputs['context.addInitScript'],
      pw.BrowserContextAddInitScriptParams,
      'script',
      'source',
      never
    >
  >
  'context.exposeBinding': Assert<
    UpstreamCheck<
      'context.exposeBinding',
      OpInputs['context.exposeBinding'],
      pw.BrowserContextExposeBindingParams,
      never,
      never,
      never
    >
  >
  // dialog identifies the host-owned dialog instead of an upstream channel receiver.
  'dialog.accept': Assert<
    UpstreamCheck<
      'dialog.accept',
      OpInputs['dialog.accept'],
      pw.DialogAcceptParams,
      'dialog',
      never,
      never
    >
  >
  // dialog identifies the host-owned dialog instead of an upstream channel receiver.
  'dialog.dismiss': Assert<
    UpstreamCheck<
      'dialog.dismiss',
      OpInputs['dialog.dismiss'],
      pw.DialogDismissParams,
      'dialog',
      never,
      never
    >
  >
  // download identifies the host-owned download instead of an upstream Artifact channel.
  'download.path': Assert<
    UpstreamCheck<
      'download.path',
      OpInputs['download.path'],
      pw.ArtifactPathAfterFinishedParams,
      'download',
      never,
      never
    >
  >
  // download identifies the host-owned download instead of an upstream Artifact channel.
  'download.saveAs': Assert<
    UpstreamCheck<
      'download.saveAs',
      OpInputs['download.saveAs'],
      pw.ArtifactSaveAsParams,
      'download',
      never,
      never
    >
  >
}

/** Ops implemented as client work, an event pump, or several upstream commands together. */
type NeoOnlyOps = {
  'locator.allTextContents': 'Upstream evaluates all matches in the client; Neo performs a probe read.'
  'locator.allInnerTexts': 'Upstream evaluates all matches in the client; Neo performs a probe read.'
  'page.waitForNavigation': 'Neo waits on its navigation journal instead of upstream frame events.'
  'page.waitForURL': 'Neo combines URL matching with a navigation wait.'
  'page.waitForLoadState': 'Neo waits on its lifecycle journal instead of upstream frame events.'
  'page.opener': 'Neo queries its tab bookkeeping instead of upstream initializer state.'
  'page.frames': 'Neo reads the frame tree instead of upstream client-side frame objects.'
  'locator.waitForFunction': 'Neo probes the element predicate; upstream composes evaluation and waiting.'
  'page.saveScreenshot': 'Host-side file write after capture; upstream performs this in its client.'
  'page.locatorHandlerTriggered': 'Neo background pull replacing upstream locatorHandlerTriggered events.'
  'context.pages': 'Neo lists live tabs instead of upstream client-side page objects.'
  'disposable.dispose': 'Neo disposes a host-owned registration rather than a channel object.'
  'events.listen': 'Neo run-scoped listener bookkeeping, including cleanup after a tab closes.'
  'events.next': 'Neo background event pump, with one cursor per listener.'
  'events.wait': 'Neo journal wait by event and timestamp instead of upstream channel events.'
}

// Exhaustive in both directions: a new op cannot silently miss this file, and a removed op
// cannot leave a check or a Neo-only reason behind. A migrated op is checked only by its
// generated route row, so hand checks must leave in the same change as the hand input.
export type AllOpsChecked = Assert<
  Exclude<
    OpName,
    keyof UpstreamChecks | keyof NeoOnlyOps | keyof GeneratedInputs
  > extends never
    ? Exclude<
        keyof UpstreamChecks | keyof NeoOnlyOps | keyof GeneratedInputs,
        OpName
      > extends never
      ?
          | Extract<keyof UpstreamChecks, keyof NeoOnlyOps>
          | Extract<
              keyof GeneratedInputs,
              keyof UpstreamChecks | keyof NeoOnlyOps
            > extends never
        ? true
        : false
      : false
    : false
>

// Shared exported lists are checked by their actual inferred keys. Private and inline lists
// use KeyList at their declaration/call site so their literal keys cannot drift from a copy.
export type OptionChecks = {
  'locator.fill': Assert<
    KeysCheck<
      'locator.fill',
      (typeof FILL_KEYS)[number],
      pw.ElementHandleFillOptions
    >
  >
  'locator.press': Assert<
    KeysCheck<
      'locator.press',
      (typeof PRESS_KEYS)[number],
      pw.ElementHandlePressOptions
    >
  >
  'locator.pressSequentially': Assert<
    KeysCheck<
      'locator.pressSequentially',
      (typeof TYPE_KEYS)[number],
      pw.ElementHandleTypeOptions
    >
  >
  'page.screenshot': Assert<
    KeysCheck<
      'page.screenshot',
      (typeof SCREENSHOT_KEYS)[number],
      pw.PageScreenshotOptions
    >
  >
}

/** Private/inline lists refer to these vendored options only through erased type imports. */
export type PickOptions = {
  'keyboard.press': pw.PageKeyboardPressOptions
  'keyboard.type': pw.PageKeyboardTypeOptions
  'mouse.move': pw.PageMouseMoveOptions
  'mouse.down': pw.PageMouseDownOptions
  'mouse.up': pw.PageMouseUpOptions
  'mouse.click': pw.PageMouseClickOptions
  'locator.selectOption': pw.ElementHandleSelectOptionOptions
  'locator.selectText': pw.ElementHandleSelectTextOptions
  'page.goto': pw.FrameGotoOptions
  'page.pdf': pw.PagePdfOptions
  // path is read on the Neo host; upstream's client converts it to content before sending.
  'page.addScriptTag': pw.FrameAddScriptTagOptions & { path?: string }
  'page.addStyleTag': pw.FrameAddStyleTagOptions & { path?: string }
}
