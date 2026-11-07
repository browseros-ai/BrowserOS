/**
 * Playwright: `client/page.ts` `Page.screenshot`
 * Playwright: `client/page.ts` `Page.pdf`
 * Playwright: `client/elementHandle.ts` `determineScreenshotType`
 *
 * The client half of `page.screenshot`, `locator.screenshot` and `page.pdf`: Playwright's
 * option mapping before the call (the image type a `path` names, mask locators as
 * `{frame, selector}`, PDF sizes in `px`), and the `Buffer` the call resolves.
 *
 * What differs from Playwright:
 * - Deviation: `client/page.ts` `Page.screenshot`: `await fs.promises.writeFile(options.path, result.binary);`
 *   A Neo script has no file system: `path` travels with the call and neo-runtime writes the
 *   bytes into the run's artifact directory (`artifacts.rs`), relative paths only. A run
 *   without one refuses `path` with an Error naming the option.
 *
 * `page.screenshot` and `locator.screenshot` resolve their timeout through `TimeoutSettings`,
 * so their `signal` aborts the capture as Playwright's does (slice S); `page.pdf`, which
 * Playwright sends with `kNoTimeout`, reads none and ignores one.
 */

import type {
  PagePdfParams,
  PageScreenshotParams,
} from '@browseros/neo-runtime-spec/serve'
import { getMimeTypeForPath } from '../vendor/playwright/isomorphic/mimeType'
import { Buffer } from './buffer'
import type { ApiName } from './connection'
import type { Locator } from './locator'
import type { NeoScreenshotParams } from './neo-commands'
import {
  NEO_SCREENSHOT_KEYS,
  PAGE_PDF_KEYS,
  PAGE_SCREENSHOT_KEYS,
  pickParams,
} from './protocol'

/** Playwright's `Rect`. */
export type Rect = { x: number; y: number; width: number; height: number }

/** The options `page.screenshot` and `locator.screenshot` share (`LocatorScreenshotOptions`). */
export type ScreenshotOptions = {
  type?: 'png' | 'jpeg' | 'webp'
  quality?: number
  omitBackground?: boolean
  caret?: 'hide' | 'initial'
  animations?: 'disabled' | 'allow'
  scale?: 'css' | 'device'
  mask?: Locator[]
  maskColor?: string
  style?: string
  /** Relative to the run's artifact directory. */
  path?: string
  timeout?: number
}

/** `page.screenshot`'s options (`PageScreenshotOptions`). */
export type PageScreenshotOptions = ScreenshotOptions & {
  fullPage?: boolean
  clip?: Rect
}

/** `page.pdf`'s options (`PDFOptions`). Numbers are CSS pixels. */
export type PDFOptions = {
  scale?: number
  displayHeaderFooter?: boolean
  headerTemplate?: string
  footerTemplate?: string
  printBackground?: boolean
  landscape?: boolean
  pageRanges?: string
  format?: string
  width?: string | number
  height?: string | number
  margin?: {
    top?: string | number
    right?: string | number
    bottom?: string | number
    left?: string | number
  }
  preferCSSPageSize?: boolean
  tagged?: boolean
  outline?: boolean
  /** Relative to the run's artifact directory. */
  path?: string
}

/** Neo's `path` beside Playwright's params (`capture.rs`). */
type WithPath<Params> = Params & { path?: string }

/**
 * Playwright's `determineScreenshotType`: a `path`'s extension names the type, and one that is
 * not an image Chromium writes throws.
 * Playwright: `client/elementHandle.ts` `determineScreenshotType`
 */
export function determineScreenshotType(options: {
  path?: string
  type?: 'png' | 'jpeg' | 'webp'
}): 'png' | 'jpeg' | 'webp' | undefined {
  if (options.path) {
    const mimeType = getMimeTypeForPath(options.path)
    if (mimeType === 'image/png') return 'png'
    else if (mimeType === 'image/jpeg') return 'jpeg'
    else if (mimeType === 'image/webp') return 'webp'
    throw new Error(`path: unsupported mime type "${mimeType}"`)
  }
  return options.type
}

/**
 * `page.screenshot`'s params: Playwright's, in schema order, then Neo's `path`.
 * Playwright: `client/page.ts` `Page.screenshot`
 */
export function pageScreenshotParams(
  apiName: ApiName,
  options: PageScreenshotOptions,
): WithPath<PageScreenshotParams> {
  const params = pickParams(
    PAGE_SCREENSHOT_KEYS,
    screenshotFields(apiName, options),
  ) as WithPath<PageScreenshotParams>
  return withPath(params, options.path)
}

/**
 * `neo:screenshot`'s params for `locator.screenshot`: the selector, then
 * `ElementHandle.screenshot`'s params, then `path`.
 * Playwright: `client/locator.ts` `Locator.screenshot`
 */
export function locatorScreenshotParams(
  apiName: ApiName,
  selector: string,
  options: ScreenshotOptions,
): NeoScreenshotParams {
  const params = pickParams(NEO_SCREENSHOT_KEYS, {
    ...screenshotFields(apiName, options),
    selector,
  }) as NeoScreenshotParams
  return withPath(params, options.path)
}

/**
 * Playwright's copy of the options, `copy.type` from the path when unset, and each mask
 * locator as its frame's channel and selector.
 * Playwright: `client/page.ts` `Page.screenshot`: `const copy: channels.PageScreenshotParams = { ...options, mask: undefined };`
 */
function screenshotFields(
  apiName: ApiName,
  options: ScreenshotOptions,
): Record<string, unknown> {
  const fields: Record<string, unknown> = { ...options, mask: undefined }
  if (!fields.type) {
    try {
      fields.type = determineScreenshotType(options)
    } catch (error) {
      ;(error as Error).message = `${apiName}: ${(error as Error).message}`
      throw error
    }
  }
  if (options.mask) {
    fields.mask = options.mask.map((locator) => ({
      frame: { guid: locator._frame._guid },
      selector: locator._selector,
    }))
  }
  return fields
}

/**
 * Playwright's `Page.pdf` option mapping: number sizes and margins become `px` strings, and
 * `margin` is copied so the caller's object stays as it was.
 * Playwright: `client/page.ts` `Page.pdf`
 */
export function pdfParams(
  apiName: ApiName,
  options: PDFOptions,
): WithPath<PagePdfParams> {
  const transportOptions: Record<string, unknown> = { ...options }
  const margin = options.margin ? { ...options.margin } : undefined
  if (margin) transportOptions.margin = margin
  if (typeof options.width === 'number')
    transportOptions.width = `${options.width}px`
  if (typeof options.height === 'number')
    transportOptions.height = `${options.height}px`
  if (margin) {
    for (const side of ['top', 'right', 'bottom', 'left'] as const) {
      const value = margin[side]
      if (typeof value === 'number') margin[side] = `${value}px`
    }
  }
  const params = pickParams(
    PAGE_PDF_KEYS,
    transportOptions,
  ) as WithPath<PagePdfParams>
  return withPath(params, options.path)
}

/**
 * Only a non-empty path is written.
 * Playwright: `client/page.ts` `Page.screenshot`: `if (options.path) {`
 */
function withPath<Params extends { path?: string }>(
  params: Params,
  path: string | undefined,
): Params {
  if (path) params.path = path
  return params
}

/**
 * The bytes of a `{binary}` or `{pdf}` result.
 * Neo: the realm's `Buffer` for the wire's base64 (`buffer.ts`).
 */
export function resultBuffer(binary: string): Buffer {
  return Buffer.fromBinary(binary)
}
