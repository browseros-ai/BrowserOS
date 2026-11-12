/**
 * Every wire op is checked against Playwright 1.64.1 or classified below as client/Neo work.
 * The reasons are the migration inventory for Rust-owned generated checks; every remaining
 * deviation describes an intentional difference from upstream rather than a loose declaration.
 */
import type * as pw from '../../../neo-playwright/third_party/playwright/packages/playwright-core/src/client/channels'
import type { GeneratedInputs } from './generated'
import type { OpName } from './generated/ops'
import type { Assert, UpstreamCheck } from './upstream-check'
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
  // Neo carries the original selector for diagnostics after brand resolution; it always formats JavaScript, without sdkLanguage.
  // This op also serves waitForElementState, so state includes stable/enabled/disabled/editable.
  // noWait selects the immediate page.$ query instead of the locator wait.
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
  'locator.waitForFunction': 'Neo probes the element predicate; upstream composes evaluation and waiting.'
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

// All exported option lists now carry generated checks; retain the empty assertion map
// while the remaining hand op checks still use this module.
export type OptionChecks = Record<never, never>

/** Private/inline lists refer to these vendored options only through erased type imports. */
export type PickOptions = {
  'locator.selectOption': pw.ElementHandleSelectOptionOptions
  'locator.selectText': pw.ElementHandleSelectTextOptions
  'page.goto': pw.FrameGotoOptions
  'page.pdf': pw.PagePdfOptions
  // path is read on the Neo host; upstream's client converts it to content before sending.
  'page.addScriptTag': pw.FrameAddScriptTagOptions & { path?: string }
  'page.addStyleTag': pw.FrameAddStyleTagOptions & { path?: string }
}
