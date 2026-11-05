/**
 * Playwright's ElementHandle: one element pinned in one document (`{"$neo":"element"}`), where
 * a Locator re-resolves. Its methods send the same ops as Locator's with the element as the
 * target, so Rust acts on that node or fails `element_detached` once its document is gone.
 * `$` and `$$` send a locator rooted at the handle (`root`), which Rust queries from that node.
 */
import type { Rect } from '../probe/protocol'
import { invoke } from './bridge'
import { EventEmitter } from './emitter'
import { clientError, unsupportedError } from './errors'
import {
  type ClickOptions,
  refusedScreenshotOption,
  type ScreenshotOptionsInput,
} from './frame'
import type { OpName } from './ops.generated'
import {
  CHECK_KEYS,
  DBLCLICK_KEYS,
  evaluation,
  FILL_KEYS,
  HOVER_KEYS,
  inputFilePaths,
  KEY_KEYS,
  markHandle,
  POINTER_KEYS,
  pick,
  SCREENSHOT_KEYS,
  selectOptionValues,
} from './options'
import type { Page } from './page'
import { locatorBrand } from './selectors'
import { type TimeoutOption, timeoutMs } from './timeouts'
import type {
  ElementBrand,
  ElementStateName,
  FillOptions,
  KeyOptions,
  LocatorBrand,
  OpInputs,
  OpOutputs,
  ScreenshotOptions,
  Strictness,
} from './wire'

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

export class ElementHandle extends EventEmitter {
  readonly _page: Page
  readonly _brand: ElementBrand

  constructor(page: Page, brand: ElementBrand) {
    super()
    this._page = page
    this._brand = brand
    markHandle(this)
  }

  asElement(): ElementHandle {
    return this
  }

  async dispose(): Promise<void> {}

  override toString(): string {
    return this._brand.display
  }

  toJSON(): ElementBrand {
    return this._brand
  }

  async #op<Op extends ElementOp>(
    op: Op,
    api: string,
    timeout: number,
    args: Omit<OpInputs[Op], 'target' | 'strict'>,
  ): Promise<OpOutputs[Op]> {
    const input = {
      target: this._brand,
      strict: 'strict',
      ...args,
    } as OpInputs[Op]
    return await invoke(op, { api, timeoutMs: timeoutMs(api, timeout) }, input)
  }

  #timeout(options: TimeoutOption | undefined): number {
    return this._page._timeoutSettings.timeout(options)
  }

  /** `selector` as a locator rooted at this handle's node, as upstream's handle queries scope it. */
  #rooted(api: string, selector: string): LocatorBrand {
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
    const api = 'elementHandle.$'
    const handle = await invoke(
      'locator.elementHandle',
      { api, timeoutMs: timeoutMs(api, 0) },
      {
        target: this.#rooted(api, selector),
        strict: 'firstMatch',
        noWait: true,
      },
    )
    return this._page._handle(handle)
  }

  /** Every match under this handle's node now, each a handle; never waits. */
  async $$(selector: string): Promise<ElementHandle[]> {
    const api = 'elementHandle.$$'
    const handles = await invoke(
      'locator.elementHandles',
      { api, timeoutMs: timeoutMs(api, 0) },
      { target: this.#rooted(api, selector) },
    )
    return handles.flatMap((handle) => {
      const element = this._page._handle(handle)
      return element ? [element] : []
    })
  }

  async click(options: ClickOptions = {}): Promise<void> {
    await this.#op(
      'locator.click',
      'elementHandle.click',
      this.#timeout(options),
      {
        options: pick(options, POINTER_KEYS),
      },
    )
  }

  async dblclick(options: ClickOptions = {}): Promise<void> {
    await this.#op(
      'locator.dblclick',
      'elementHandle.dblclick',
      this.#timeout(options),
      {
        options: pick(options, DBLCLICK_KEYS),
      },
    )
  }

  async hover(options: ClickOptions = {}): Promise<void> {
    await this.#op(
      'locator.hover',
      'elementHandle.hover',
      this.#timeout(options),
      {
        options: pick(options, HOVER_KEYS),
      },
    )
  }

  async check(options: ClickOptions = {}): Promise<void> {
    await this.#op(
      'locator.check',
      'elementHandle.check',
      this.#timeout(options),
      {
        options: pick(options, CHECK_KEYS),
      },
    )
  }

  async uncheck(options: ClickOptions = {}): Promise<void> {
    await this.#op(
      'locator.uncheck',
      'elementHandle.uncheck',
      this.#timeout(options),
      {
        options: pick(options, CHECK_KEYS),
      },
    )
  }

  async setChecked(
    checked: boolean,
    options: ClickOptions = {},
  ): Promise<void> {
    await this.#op(
      checked ? 'locator.check' : 'locator.uncheck',
      'elementHandle.setChecked',
      this.#timeout(options),
      { options: pick(options, CHECK_KEYS) },
    )
  }

  async fill(
    value: string,
    options: FillOptions & TimeoutOption = {},
  ): Promise<void> {
    await this.#op(
      'locator.fill',
      'elementHandle.fill',
      this.#timeout(options),
      {
        value,
        options: pick(options, FILL_KEYS),
      },
    )
  }

  async press(
    key: string,
    options: KeyOptions & TimeoutOption = {},
  ): Promise<void> {
    await this.#op(
      'locator.press',
      'elementHandle.press',
      this.#timeout(options),
      {
        key,
        options: pick(options, KEY_KEYS),
      },
    )
  }

  async type(
    text: string,
    options: KeyOptions & TimeoutOption = {},
  ): Promise<void> {
    await this.#op(
      'locator.pressSequentially',
      'elementHandle.type',
      this.#timeout(options),
      {
        text,
        options: pick(options, KEY_KEYS),
      },
    )
  }

  async selectOption(
    values: unknown,
    options: TimeoutOption & { force?: boolean } = {},
  ): Promise<string[]> {
    const api = 'elementHandle.selectOption'
    return await this.#op('locator.selectOption', api, this.#timeout(options), {
      ...selectOptionValues(api, values),
      ...pick(options, ['force']),
    })
  }

  async selectText(
    options: TimeoutOption & { force?: boolean } = {},
  ): Promise<void> {
    await this.#op(
      'locator.selectText',
      'elementHandle.selectText',
      this.#timeout(options),
      pick(options, ['force']),
    )
  }

  async setInputFiles(
    files: unknown,
    options: TimeoutOption = {},
  ): Promise<void> {
    const api = 'elementHandle.setInputFiles'
    await this.#op('locator.setInputFiles', api, this.#timeout(options), {
      files: inputFilePaths(api, files),
    })
  }

  async focus(): Promise<void> {
    await this.#op('locator.focus', 'elementHandle.focus', 0, {})
  }

  async dispatchEvent(type: string, eventInit: unknown = {}): Promise<void> {
    await this.#op('locator.dispatchEvent', 'elementHandle.dispatchEvent', 0, {
      type,
      eventInit,
    })
  }

  async scrollIntoViewIfNeeded(options: TimeoutOption = {}): Promise<void> {
    await this.#op(
      'locator.scrollIntoViewIfNeeded',
      'elementHandle.scrollIntoViewIfNeeded',
      this.#timeout(options),
      {},
    )
  }

  async waitForElementState(
    state: ElementStateName,
    options: TimeoutOption = {},
  ): Promise<void> {
    await this.#op(
      'locator.waitFor',
      'elementHandle.waitForElementState',
      this.#timeout(options),
      {
        state,
        omitReturnValue: true,
      },
    )
  }

  textContent(): Promise<string | null> {
    return this.#op('locator.textContent', 'elementHandle.textContent', 0, {})
  }

  innerText(): Promise<string> {
    return this.#op('locator.innerText', 'elementHandle.innerText', 0, {})
  }

  innerHTML(): Promise<string> {
    return this.#op('locator.innerHTML', 'elementHandle.innerHTML', 0, {})
  }

  inputValue(): Promise<string> {
    return this.#op('locator.inputValue', 'elementHandle.inputValue', 0, {})
  }

  getAttribute(name: string): Promise<string | null> {
    return this.#op('locator.getAttribute', 'elementHandle.getAttribute', 0, {
      name,
    })
  }

  isChecked(): Promise<boolean> {
    return this.#op('locator.isChecked', 'elementHandle.isChecked', 0, {})
  }

  isDisabled(): Promise<boolean> {
    return this.#op('locator.isDisabled', 'elementHandle.isDisabled', 0, {})
  }

  isEditable(): Promise<boolean> {
    return this.#op('locator.isEditable', 'elementHandle.isEditable', 0, {})
  }

  isEnabled(): Promise<boolean> {
    return this.#op('locator.isEnabled', 'elementHandle.isEnabled', 0, {})
  }

  isHidden(): Promise<boolean> {
    return this.#op('locator.isHidden', 'elementHandle.isHidden', 0, {})
  }

  isVisible(): Promise<boolean> {
    return this.#op('locator.isVisible', 'elementHandle.isVisible', 0, {})
  }

  boundingBox(): Promise<Rect | null> {
    return this.#op('locator.boundingBox', 'elementHandle.boundingBox', 0, {})
  }

  async evaluate(pageFunction: unknown, arg?: unknown): Promise<unknown> {
    const api = 'elementHandle.evaluate'
    return await this.#op(
      'locator.evaluate',
      api,
      0,
      evaluation(api, pageFunction, arg),
    )
  }

  async screenshot(options: ScreenshotOptionsInput = {}): Promise<unknown> {
    const api = 'elementHandle.screenshot'
    if (options.path !== undefined)
      throw unsupportedError(
        api,
        'Neo returns the image instead of writing a file; omit path',
      )
    const refused = refusedScreenshotOption(options)
    if (refused !== undefined) throw unsupportedError(api, refused)
    const wire: ScreenshotOptions = pick(options, SCREENSHOT_KEYS)
    return await invoke(
      'page.screenshot',
      { api, timeoutMs: timeoutMs(api, this.#timeout(options)) },
      {
        page: await this._page._brand(api),
        target: this._brand,
        strict: 'strict',
        ...wire,
      },
    )
  }
}
