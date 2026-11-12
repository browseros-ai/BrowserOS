/**
 * Every wire op is checked against Playwright 1.64.1 or classified below as client/Neo work.
 * The reasons are the migration inventory for Rust-owned generated checks; every remaining
 * deviation describes an intentional difference from upstream rather than a loose declaration.
 */
import type * as pw from '../../../neo-playwright/third_party/playwright/packages/playwright-core/src/client/channels'
import type { GeneratedInputs } from './generated'
import type { OpName } from './generated/ops'
import type { Assert } from './upstream-check'

/**
 * Hand checks against upstream commands: none since the events family moved; every op is
 * checked by its generated route row. The close-out deletes this file.
 */
export type UpstreamChecks = Record<never, never>

/** Ops implemented as client work, an event pump, or several upstream commands together. */
type NeoOnlyOps = Record<never, never>

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
