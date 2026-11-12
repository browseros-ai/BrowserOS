/**
 * `page.keyboard` and `page.mouse`. Rust keeps their state per page (held modifiers and keys,
 * the pointer's position and buttons), as Playwright's server does, so these are thin: one op
 * per call, with no Playwright timeout.
 */

import { captureSite } from './call-site'
import type {
  KeyboardDelayOptions,
  MouseButtonOptions,
  MouseClickOptions,
  MouseStepOptions,
} from './generated'
import { OPTION_KEYS, send } from './generated'
import { markHandle, pick } from './options'
import type { Page } from './page'

export type ButtonOptions = MouseButtonOptions

export class Keyboard {
  readonly #page: Page

  constructor(page: Page) {
    this.#page = page
    markHandle(this)
  }

  async down(key: string): Promise<void> {
    const site = captureSite()
    const api = 'keyboard.down'
    await send['keyboard.down'](
      site,
      { api, timeoutMs: 0 },
      { page: await this.#page._brand(api), key },
    )
  }

  async up(key: string): Promise<void> {
    const site = captureSite()
    const api = 'keyboard.up'
    await send['keyboard.up'](
      site,
      { api, timeoutMs: 0 },
      { page: await this.#page._brand(api), key },
    )
  }

  async press(key: string, options: KeyboardDelayOptions = {}): Promise<void> {
    const site = captureSite()
    const api = 'keyboard.press'
    await send['keyboard.press'](
      site,
      { api, timeoutMs: 0 },
      {
        page: await this.#page._brand(api),
        key,
        options: pick(options, OPTION_KEYS['keyboard.press']),
      },
    )
  }

  async type(text: string, options: KeyboardDelayOptions = {}): Promise<void> {
    const site = captureSite()
    const api = 'keyboard.type'
    await send['keyboard.type'](
      site,
      { api, timeoutMs: 0 },
      {
        page: await this.#page._brand(api),
        text,
        options: pick(options, OPTION_KEYS['keyboard.type']),
      },
    )
  }

  async insertText(text: string): Promise<void> {
    const site = captureSite()
    const api = 'keyboard.insertText'
    await send['keyboard.insertText'](
      site,
      { api, timeoutMs: 0 },
      { page: await this.#page._brand(api), text },
    )
  }
}

export class Mouse {
  readonly #page: Page

  constructor(page: Page) {
    this.#page = page
    markHandle(this)
  }

  async move(
    x: number,
    y: number,
    options: MouseStepOptions = {},
  ): Promise<void> {
    const site = captureSite()
    const api = 'mouse.move'
    await send['mouse.move'](
      site,
      { api, timeoutMs: 0 },
      {
        page: await this.#page._brand(api),
        x,
        y,
        options: pick(options, OPTION_KEYS['mouse.move']),
      },
    )
  }

  async down(options: ButtonOptions = {}): Promise<void> {
    const site = captureSite()
    const api = 'mouse.down'
    await send['mouse.down'](
      site,
      { api, timeoutMs: 0 },
      {
        page: await this.#page._brand(api),
        options: pick(options, OPTION_KEYS['mouse.down']),
      },
    )
  }

  async up(options: ButtonOptions = {}): Promise<void> {
    const site = captureSite()
    const api = 'mouse.up'
    await send['mouse.up'](
      site,
      { api, timeoutMs: 0 },
      {
        page: await this.#page._brand(api),
        options: pick(options, OPTION_KEYS['mouse.up']),
      },
    )
  }

  click(x: number, y: number, options: MouseClickOptions = {}): Promise<void> {
    return this.#clickAt('mouse.click', x, y, options)
  }

  /** upstream's client sends a click with `clickCount: 2`. */
  dblclick(
    x: number,
    y: number,
    options: Omit<MouseClickOptions, 'clickCount'> = {},
  ): Promise<void> {
    return this.#clickAt('mouse.dblclick', x, y, { ...options, clickCount: 2 })
  }

  async wheel(deltaX: number, deltaY: number): Promise<void> {
    const site = captureSite()
    const api = 'mouse.wheel'
    await send['mouse.wheel'](
      site,
      { api, timeoutMs: 0 },
      { page: await this.#page._brand(api), deltaX, deltaY },
    )
  }

  async #clickAt(
    api: string,
    x: number,
    y: number,
    options: MouseClickOptions,
  ): Promise<void> {
    const site = captureSite()
    await send['mouse.click'](
      site,
      { api, timeoutMs: 0 },
      {
        page: await this.#page._brand(api),
        x,
        y,
        options: pick(options, OPTION_KEYS['mouse.click']),
      },
    )
  }
}
