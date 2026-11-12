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
