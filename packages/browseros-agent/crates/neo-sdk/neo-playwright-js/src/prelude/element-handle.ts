/**
 * Playwright's ElementHandle: one element pinned in one document (`{"$neo":"element"}`), where
 * a Locator re-resolves. Its methods send the same ops as Locator's with the element as the
 * target, so Rust acts on that node or fails `element_detached` once its document is gone.
 * `$`, `$$`, `$eval` and `$$eval` send a locator rooted at the handle (`root`), which Rust
 * queries from that node (`$eval` and `$$eval` adapted from b45f541b0). Its scripts run on the
 * node through the main-world gateway, in its document or out of it, as upstream's do.
 *
 * `dispose()` ends the handle as upstream's ends its remote object: every later call fails
 * "Target page, context or browser has been closed", and as another script's argument it fails
 * the channel's check by its guid, which each handle takes at random as upstream's does.
 */

import type { Rect } from '../probe/protocol'
import { invoke } from './bridge'
import { type CallSite, captureSite } from './call-site'
import { EventEmitter } from './emitter'
import { clientError } from './errors'
import { type EvaluateOptions, fromPageValue, pageCall } from './evaluation'
import { inputFiles, withClientErrors } from './files'
import {
  type ClickOptions,
  type ScreenshotOptionsInput,
  screenshotOptions,
} from './frame'
import { OPTION_KEYS, type Strictness, send } from './generated'
import type { OpName } from './generated/ops'
import { markHandle, pick, selectOptionValues } from './options'
import type { Page } from './page'
import { saveScreenshot, screenshotType } from './screenshot'
import { locatorBrand } from './selectors'
import { signalOf } from './signal'
import { type TimeoutOption, timeoutMs } from './timeouts'
import type { KeyList } from './upstream-check'
import type {
  ElementBrand,
  ElementStateName,
  FillOptions,
  KeyOptions,
  LocatorBrand,
  OpInputs,
  OpOutputs,
} from './wire'

/** These generated senders all take only an element target and strictness. */
type ElementReadOp =
  | 'locator.textContent'
  | 'locator.innerText'
  | 'locator.innerHTML'
  | 'locator.inputValue'
  | 'locator.isChecked'
  | 'locator.isDisabled'
  | 'locator.isEditable'
  | 'locator.isEnabled'
  | 'locator.isVisible'
  | 'locator.isHidden'
  | 'locator.boundingBox'

type ElementOp = {
  [Op in OpName]: OpInputs[Op] extends { target: unknown; strict: Strictness }
    ? Op
    : never
}[OpName]

/** Reads a handle from an op result, revived by the bootstrap or as a raw brand. */
export function elementBrand(value: unknown): ElementBrand | null {
  if (value instanceof ElementHandle) return value._brand
  if (typeof value !== 'object' || value === null) return null
  if (Reflect.get(value, '$neo') !== 'element') return null
  return value as ElementBrand
}

/** A guid as upstream's are made: `handle@` and 32 hex digits. */
function handleGuid(): string {
  let digits = ''
  for (let i = 0; i < 32; i++)
    digits += Math.floor(Math.random() * 16).toString(16)
  return `handle@${digits}`
}

export class ElementHandle extends EventEmitter {
  readonly _page: Page
  readonly _brand: ElementBrand
  readonly _guid = handleGuid()
  _disposed = false

  constructor(page: Page, brand: ElementBrand) {
    super()
    this._page = page
    this._brand = brand
    markHandle(this)
  }

  asElement(): ElementHandle {
    return this
  }

  async dispose(): Promise<void> {
    this._disposed = true
  }

  /** upstream's error for a call on a handle whose object it disposed. */
  #live(api: string): void {
    if (this._disposed)
      throw clientError(api, 'Target page, context or browser has been closed')
  }

  override toString(): string {
    return this._brand.display
  }

  toJSON(): ElementBrand {
    return this._brand
  }

  async #op<Op extends ElementOp>(
    op: Op,
    site: CallSite,
    api: string,
    timeout: number,
    args: Omit<OpInputs[Op], 'target' | 'strict'>,
  ): Promise<OpOutputs[Op]> {
    this.#live(api)
    const input = {
      target: this._brand,
      strict: 'strict',
      ...args,
    } as OpInputs[Op]
    return await invoke(
      op,
      site,
      { api, timeoutMs: timeoutMs(api, timeout) },
      input,
    )
  }

  #read<Op extends ElementReadOp>(
    op: Op,
    site: CallSite,
    api: string,
  ): Promise<OpOutputs[Op]>
  async #read(
    op: ElementReadOp,
    site: CallSite,
    api: string,
  ): Promise<OpOutputs[ElementReadOp]> {
    this.#live(api)
    const target = this._brand
    return await send[op](
      site,
      { api, timeoutMs: timeoutMs(api, 0) },
      { target, strict: 'strict' },
    )
  }

  #timeout(options: TimeoutOption | undefined): number {
    return this._page._timeoutSettings.timeout(options)
  }

  /** `selector` as a locator rooted at this handle's node, as upstream's handle queries scope it. */
  #rooted(api: string, selector: string): LocatorBrand {
    this.#live(api)
    if (typeof selector !== 'string')
      throw clientError(
        api,
        `selector: expected string, got ${typeof selector}`,
      )
    return {
      ...locatorBrand(api, this._brand.page, null, selector),
      root: this._brand,
    }
  }

  /** The first match under this handle's node now, or null; never waits, never strict. */
  async $(selector: string): Promise<ElementHandle | null> {
    const site = captureSite()
    const api = 'elementHandle.$'
    const handle = await send['locator.elementHandle'](
      site,
      { api, timeoutMs: timeoutMs(api, 0) },
      {
        target: this.#rooted(api, selector),
        strict: 'firstMatch',
      },
    )
    return this._page._handle(handle)
  }

  /** Every match under this handle's node now, each a handle; never waits. */
  async $$(selector: string): Promise<ElementHandle[]> {
    const site = captureSite()
    const api = 'elementHandle.$$'
    const handles = await send['locator.elementHandles'](
      site,
      { api, timeoutMs: timeoutMs(api, 0) },
      { target: this.#rooted(api, selector) },
    )
    return handles.flatMap((handle) => {
      const element = this._page._handle(handle)
      return element ? [element] : []
    })
  }

  async click(options: ClickOptions = {}): Promise<void> {
    const site = captureSite()
    const api = 'elementHandle.click'
    const timeout = this.#timeout(options)
    const picked = pick(options, OPTION_KEYS['locator.click'])
    this.#live(api)
    await send['locator.click'](
      site,
      { api, timeoutMs: timeoutMs(api, timeout) },
      { target: this._brand, strict: 'strict', ...picked },
    )
  }

  async dblclick(options: ClickOptions = {}): Promise<void> {
    const site = captureSite()
    const api = 'elementHandle.dblclick'
    const timeout = this.#timeout(options)
    const picked = pick(options, OPTION_KEYS['locator.dblclick'])
    this.#live(api)
    await send['locator.dblclick'](
      site,
      { api, timeoutMs: timeoutMs(api, timeout) },
      { target: this._brand, strict: 'strict', ...picked },
    )
  }

  async hover(options: ClickOptions = {}): Promise<void> {
    const site = captureSite()
    const api = 'elementHandle.hover'
    const timeout = this.#timeout(options)
    const picked = pick(options, OPTION_KEYS['locator.hover'])
    this.#live(api)
    await send['locator.hover'](
      site,
      { api, timeoutMs: timeoutMs(api, timeout) },
      { target: this._brand, strict: 'strict', ...picked },
    )
  }

  async check(options: ClickOptions = {}): Promise<void> {
    const site = captureSite()
    const api = 'elementHandle.check'
    const timeout = this.#timeout(options)
    const picked = pick(options, OPTION_KEYS['locator.check'])
    this.#live(api)
    await send['locator.check'](
      site,
      { api, timeoutMs: timeoutMs(api, timeout) },
      { target: this._brand, strict: 'strict', ...picked },
    )
  }

  async uncheck(options: ClickOptions = {}): Promise<void> {
    const site = captureSite()
    const api = 'elementHandle.uncheck'
    const timeout = this.#timeout(options)
    const picked = pick(options, OPTION_KEYS['locator.uncheck'])
    this.#live(api)
    await send['locator.uncheck'](
      site,
      { api, timeoutMs: timeoutMs(api, timeout) },
      { target: this._brand, strict: 'strict', ...picked },
    )
  }

  async setChecked(
    checked: boolean,
    options: ClickOptions = {},
  ): Promise<void> {
    const site = captureSite()
    const op = checked ? 'locator.check' : 'locator.uncheck'
    const api = 'elementHandle.setChecked'
    const timeout = this.#timeout(options)
    const picked = pick(options, OPTION_KEYS[op])
    this.#live(api)
    await send[op](
      site,
      { api, timeoutMs: timeoutMs(api, timeout) },
      { target: this._brand, strict: 'strict', ...picked },
    )
  }

  async fill(
    value: string,
    options: FillOptions & TimeoutOption = {},
  ): Promise<void> {
    const site = captureSite()
    const api = 'elementHandle.fill'
    const timeout = this.#timeout(options)
    const picked = pick(options, OPTION_KEYS['locator.fill'])
    this.#live(api)
    await send['locator.fill'](
      site,
      { api, timeoutMs: timeoutMs(api, timeout) },
      { target: this._brand, strict: 'strict', value, ...picked },
    )
  }

  async press(
    key: string,
    options: KeyOptions & TimeoutOption = {},
  ): Promise<void> {
    const site = captureSite()
    const api = 'elementHandle.press'
    const timeout = this.#timeout(options)
    const picked = pick(options, OPTION_KEYS['locator.press'])
    this.#live(api)
    await send['locator.press'](
      site,
      { api, timeoutMs: timeoutMs(api, timeout) },
      { target: this._brand, strict: 'strict', key, ...picked },
    )
  }

  async type(
    text: string,
    options: KeyOptions & TimeoutOption = {},
  ): Promise<void> {
    const site = captureSite()
    const api = 'elementHandle.type'
    const timeout = this.#timeout(options)
    const picked = pick(options, OPTION_KEYS['locator.pressSequentially'])
    this.#live(api)
    await send['locator.pressSequentially'](
      site,
      { api, timeoutMs: timeoutMs(api, timeout) },
      { target: this._brand, strict: 'strict', text, ...picked },
    )
  }

  async selectOption(
    values: unknown,
    options: TimeoutOption & { force?: boolean } = {},
  ): Promise<string[]> {
    const site = captureSite()
    const api = 'elementHandle.selectOption'
    const timeout = this.#timeout(options)
    const converted = selectOptionValues(api, values)
    const picked = pick(options, ['force'] satisfies KeyList<
      OpInputs['locator.selectOption']
    >)
    this.#live(api)
    return await send['locator.selectOption'](
      site,
      { api, timeoutMs: timeoutMs(api, timeout) },
      {
        target: this._brand,
        strict: 'strict',
        options: converted.options.length ? converted.options : undefined,
        elements: converted.elements.length ? converted.elements : undefined,
        ...picked,
      },
    )
  }

  async selectText(
    options: TimeoutOption & { force?: boolean } = {},
  ): Promise<void> {
    const site = captureSite()
    const api = 'elementHandle.selectText'
    const timeout = this.#timeout(options)
    const picked = pick(options, ['force'] satisfies KeyList<
      OpInputs['locator.selectText']
    >)
    this.#live(api)
    await send['locator.selectText'](
      site,
      { api, timeoutMs: timeoutMs(api, timeout) },
      { target: this._brand, strict: 'strict', ...picked },
    )
  }

  async setInputFiles(
    files: unknown,
    options: TimeoutOption = {},
  ): Promise<void> {
    const site = captureSite()
    const api = 'elementHandle.setInputFiles'
    const input = inputFiles(api, files)
    await withClientErrors(api, async () => {
      const timeout = this.#timeout(options)
      this.#live(api)
      return await send['locator.setInputFiles'](
        site,
        { api, timeoutMs: timeoutMs(api, timeout) },
        { target: this._brand, strict: 'strict', files: input },
      )
    })
  }

  async focus(): Promise<void> {
    const site = captureSite()
    const api = 'elementHandle.focus'
    const timeout = 0
    this.#live(api)
    await send['locator.focus'](
      site,
      { api, timeoutMs: timeoutMs(api, timeout) },
      { target: this._brand, strict: 'strict' },
    )
  }

  async dispatchEvent(type: string, eventInit: unknown = {}): Promise<void> {
    const site = captureSite()
    const api = 'elementHandle.dispatchEvent'
    const timeout = 0
    this.#live(api)
    await send['locator.dispatchEvent'](
      site,
      { api, timeoutMs: timeoutMs(api, timeout) },
      { target: this._brand, strict: 'strict', type, eventInit },
    )
  }

  async scrollIntoViewIfNeeded(options: TimeoutOption = {}): Promise<void> {
    const site = captureSite()
    const api = 'elementHandle.scrollIntoViewIfNeeded'
    const timeout = this.#timeout(options)
    this.#live(api)
    await send['locator.scrollIntoViewIfNeeded'](
      site,
      { api, timeoutMs: timeoutMs(api, timeout) },
      { target: this._brand, strict: 'strict' },
    )
  }

  async waitForElementState(
    state: ElementStateName,
    options: TimeoutOption = {},
  ): Promise<void> {
    const site = captureSite()
    const api = 'elementHandle.waitForElementState'
    const timeout = this.#timeout(options)
    this.#live(api)
    const target = this._brand
    await send['locator.waitFor'](
      site,
      { api, timeoutMs: timeoutMs(api, timeout) },
      { target, strict: 'strict', state, omitReturnValue: true },
    )
  }

  textContent(): Promise<string | null> {
    const site = captureSite()
    return this.#read('locator.textContent', site, 'elementHandle.textContent')
  }

  innerText(): Promise<string> {
    const site = captureSite()
    return this.#read('locator.innerText', site, 'elementHandle.innerText')
  }

  innerHTML(): Promise<string> {
    const site = captureSite()
    return this.#read('locator.innerHTML', site, 'elementHandle.innerHTML')
  }

  inputValue(): Promise<string> {
    const site = captureSite()
    return this.#read('locator.inputValue', site, 'elementHandle.inputValue')
  }

  async getAttribute(name: string): Promise<string | null> {
    const site = captureSite()
    const api = 'elementHandle.getAttribute'
    this.#live(api)
    const target = this._brand
    return await send['locator.getAttribute'](
      site,
      { api, timeoutMs: timeoutMs(api, 0) },
      { target, strict: 'strict', name },
    )
  }

  isChecked(): Promise<boolean> {
    const site = captureSite()
    return this.#read('locator.isChecked', site, 'elementHandle.isChecked')
  }

  isDisabled(): Promise<boolean> {
    const site = captureSite()
    return this.#read('locator.isDisabled', site, 'elementHandle.isDisabled')
  }

  isEditable(): Promise<boolean> {
    const site = captureSite()
    return this.#read('locator.isEditable', site, 'elementHandle.isEditable')
  }

  isEnabled(): Promise<boolean> {
    const site = captureSite()
    return this.#read('locator.isEnabled', site, 'elementHandle.isEnabled')
  }

  isHidden(): Promise<boolean> {
    const site = captureSite()
    return this.#read('locator.isHidden', site, 'elementHandle.isHidden')
  }

  isVisible(): Promise<boolean> {
    const site = captureSite()
    return this.#read('locator.isVisible', site, 'elementHandle.isVisible')
  }

  boundingBox(): Promise<Rect | null> {
    const site = captureSite()
    return this.#read('locator.boundingBox', site, 'elementHandle.boundingBox')
  }

  /** upstream's `JSHandle.evaluate`: its checks first, then the script on this node. */
  async evaluate(
    pageFunction: unknown,
    arg?: unknown,
    options?: EvaluateOptions,
  ): Promise<unknown> {
    const site = captureSite()
    const api = 'elementHandle.evaluate'
    this.#live(api)
    const script = await this._page._evaluationScript(
      api,
      site,
      'handle',
      pageFunction,
      arg,
      options,
    )
    this.#live(api)
    return fromPageValue(
      api,
      await send['locator.evaluate'](
        site,
        { api, timeoutMs: timeoutMs(api, 0) },
        { target: this._brand, strict: 'strict', ...script },
      ),
    )
  }

  /**
   * `pageFunction(element, arg)` on the first match under this handle's node now, or upstream's
   * failure: upstream's `ElementHandle.evalOnSelector`, whose params `page.evalOnSelector` carries.
   */
  async $eval(
    selector: string,
    pageFunction: unknown,
    arg?: unknown,
  ): Promise<unknown> {
    const site = captureSite()
    const api = 'elementHandle.$eval'
    const script = pageCall(api, 'query', pageFunction, arg)
    const value = await send['page.evalOnSelector'](
      site,
      { api, timeoutMs: timeoutMs(api, 0) },
      {
        target: this.#rooted(api, selector),
        strict: 'firstMatch',
        selector,
        ...script,
      },
    )
    return fromPageValue(api, value)
  }

  /** `pageFunction(elements, arg)` on every match under this handle's node now. */
  async $$eval(
    selector: string,
    pageFunction: unknown,
    arg?: unknown,
  ): Promise<unknown> {
    const site = captureSite()
    const api = 'elementHandle.$$eval'
    const script = pageCall(api, 'query', pageFunction, arg)
    const value = await send['locator.evaluateAll'](
      site,
      { api, timeoutMs: timeoutMs(api, 0) },
      { target: this.#rooted(api, selector), ...script },
    )
    return fromPageValue(api, value)
  }

  async screenshot(options: ScreenshotOptionsInput = {}): Promise<unknown> {
    const site = captureSite()
    const api = 'elementHandle.screenshot'
    const wire = await screenshotOptions(api, options)
    if (!wire.type && options.path)
      wire.type = screenshotType({ api, path: options.path })
    const signal = signalOf(api, options)
    const image = await send['page.screenshot'](
      site,
      { api, timeoutMs: timeoutMs(api, this.#timeout(options)) },
      {
        page: await this._page._brand(api),
        target: this._brand,
        strict: 'strict',
        type: wire.type,
        quality: wire.quality,
        omitBackground: wire.omitBackground,
        fullPage: wire.fullPage,
        clip: wire.clip,
        animations: wire.animations,
        caret: wire.caret,
        style: wire.style,
        scale: wire.scale,
        maskColor: wire.maskColor,
        mask: wire.mask,
      },
      signal,
    )
    await saveScreenshot({
      api,
      site,
      bytes: image,
      path: options.path,
      signal,
    })
    return image
  }
}
