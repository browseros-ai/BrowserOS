/**
 * `page.keyboard` and `page.mouse`. Rust keeps their state per page (held modifiers and keys,
 * the pointer's position and buttons), as Playwright's server does, so these are thin: one op
 * per call, with no Playwright timeout.
 */

import { captureSite } from './call-site'
import type {
  KeyboardPressInput,
  MouseButtonInput,
  MouseClickInput,
  MouseMoveInput,
} from './generated'
import { OPTION_KEYS, send } from './generated'
import { markHandle, pick } from './options'
import type { Page } from './page'

// Script option types project the generated wire and its non-positional keys. No option
// fields are restated here, and positional call arguments cannot become script options.
type KeyboardDelayOptions = Pick<
  KeyboardPressInput,
  (typeof OPTION_KEYS)['keyboard.press'][number]
>
type MouseStepOptions = Pick<
  MouseMoveInput,
  (typeof OPTION_KEYS)['mouse.move'][number]
>
export type ButtonOptions = Pick<
  MouseButtonInput,
  (typeof OPTION_KEYS)['mouse.down'][number]
>
type MouseClickOptions = Pick<
  MouseClickInput,
  (typeof OPTION_KEYS)['mouse.click'][number]
>

// The former nested JSON emitted only the picked object's own fields. Reading a missing
// option while flattening must not invoke an inherited getter. Capture the intrinsic before
// agent code can replace Object.hasOwn.
const hasOwn = Object.hasOwn
function ownOption<T extends object, K extends keyof T>(
  picked: T,
  key: K,
): T[K] | undefined {
  return hasOwn(picked, key) ? picked[key] : undefined
}

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
    const page = await this.#page._brand(api)
    const picked = pick(options, OPTION_KEYS['keyboard.press'])
    await send['keyboard.press'](
      site,
      { api, timeoutMs: 0 },
      { page, key, delay: ownOption(picked, 'delay') },
    )
  }

  async type(text: string, options: KeyboardDelayOptions = {}): Promise<void> {
    const site = captureSite()
    const api = 'keyboard.type'
    const page = await this.#page._brand(api)
    const picked = pick(options, OPTION_KEYS['keyboard.type'])
    await send['keyboard.type'](
      site,
      { api, timeoutMs: 0 },
      { page, text, delay: ownOption(picked, 'delay') },
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
    const page = await this.#page._brand(api)
    const picked = pick(options, OPTION_KEYS['mouse.move'])
    await send['mouse.move'](
      site,
      { api, timeoutMs: 0 },
      { page, x, y, steps: ownOption(picked, 'steps') },
    )
  }

  async down(options: ButtonOptions = {}): Promise<void> {
    const site = captureSite()
    const api = 'mouse.down'
    const page = await this.#page._brand(api)
    const picked = pick(options, OPTION_KEYS['mouse.down'])
    await send['mouse.down'](
      site,
      { api, timeoutMs: 0 },
      {
        page,
        button: ownOption(picked, 'button'),
        clickCount: ownOption(picked, 'clickCount'),
      },
    )
  }

  async up(options: ButtonOptions = {}): Promise<void> {
    const site = captureSite()
    const api = 'mouse.up'
    const page = await this.#page._brand(api)
    const picked = pick(options, OPTION_KEYS['mouse.up'])
    await send['mouse.up'](
      site,
      { api, timeoutMs: 0 },
      {
        page,
        button: ownOption(picked, 'button'),
        clickCount: ownOption(picked, 'clickCount'),
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
    const page = await this.#page._brand(api)
    const picked = pick(options, OPTION_KEYS['mouse.click'])
    await send['mouse.click'](
      site,
      { api, timeoutMs: 0 },
      {
        page,
        x,
        y,
        button: ownOption(picked, 'button'),
        clickCount: ownOption(picked, 'clickCount'),
        delay: ownOption(picked, 'delay'),
      },
    )
  }
}
