/**
 * Selector strings and their wire form.
 *
 * Builders are pure: a Locator is a frame plus a selector string, composed exactly as
 * playwright-core 1.64.1's client/locator.ts composes it on top of the vendored locatorUtils.
 * The string becomes wire data only when an op runs, as in Playwright, where a bad selector
 * fails the action that uses it rather than the line that built it.
 */
import { clientError, unsupportedError } from './errors'
import {
  asLocator,
  escapeForTextSelector,
  type ParsedSelector,
  splitSelectorByFrame,
} from './vendored'
import type { LocatorBrand } from './wire'

/** What a locator's frame must provide for `has` checks and wire handles. */
export type FrameLike = object

export type LocatorOptions = {
  hasText?: string | RegExp
  hasNotText?: string | RegExp
  has?: { _frame: FrameLike; _selector: string }
  hasNot?: { _frame: FrameLike; _selector: string }
  visible?: boolean
}

/** Upstream's Locator constructor: each option appends one selector part, in this order. */
export function withLocatorOptions(
  frame: FrameLike,
  selector: string,
  options: LocatorOptions | undefined,
): string {
  let result = selector
  if (options?.hasText)
    result += ` >> internal:has-text=${escapeForTextSelector(options.hasText, false)}`
  if (options?.hasNotText)
    result += ` >> internal:has-not-text=${escapeForTextSelector(options.hasNotText, false)}`
  if (options?.has) {
    if (options.has._frame !== frame)
      throw new Error('Inner "has" locator must belong to the same frame.')
    result += ` >> internal:has=${JSON.stringify(options.has._selector)}`
  }
  if (options?.hasNot) {
    if (options.hasNot._frame !== frame)
      throw new Error('Inner "hasNot" locator must belong to the same frame.')
    result += ` >> internal:has-not=${JSON.stringify(options.hasNot._selector)}`
  }
  if (options?.visible !== undefined)
    result += ` >> visible=${options.visible ? 'true' : 'false'}`
  return result
}

/**
 * The locator handle Rust decodes (target.rs `LocatorTarget`). Parsing the whole selector once
 * with the vendored `splitSelectorByFrame` is what strips a nested selector's frame prefix when
 * it equals the outer one; the chunks are copied to plain JSON, which is all the bridge carries.
 */
export function locatorBrand(
  api: string,
  page: number,
  frame: string | null,
  selector: string,
): LocatorBrand {
  let split: { anyFrame: boolean; chunks: ParsedSelector[] }
  try {
    split = splitSelectorByFrame(selector)
  } catch (error) {
    throw clientError(
      api,
      error instanceof Error ? error.message : String(error),
    )
  }
  if (split.anyFrame)
    throw unsupportedError(
      api,
      "frameLocator() needs a selector in Neo, such as frameLocator('iframe') or locator('#pay').contentFrame()",
    )
  return {
    $neo: 'locator',
    page,
    frame,
    chunks: JSON.parse(JSON.stringify(split.chunks)),
    display: asLocator('javascript', selector),
  }
}
