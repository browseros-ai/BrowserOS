/**
 * `page.keyboard` and `page.mouse`. Rust keeps their state per page (held modifiers and keys,
 * the pointer's position and buttons), as Playwright's server does, so these are thin: one op
 * per call, with no Playwright timeout.
 */
import { invoke } from './bridge'
import { markHandle, pick } from './options'
import type { Page } from './page'
import type { MouseButton } from './wire'

export type ButtonOptions = { button?: MouseButton; clickCount?: number }

export class Keyboard {
  readonly #page: Page

  constructor(page: Page) {
    this.#page = page
    markHandle(this)
  }

  async down(key: string): Promise<void> {
    const api = 'keyboard.down'
    await invoke(
      'keyboard.down',
      { api, timeoutMs: 0 },
      { page: await this.#page._brand(api), key },
    )
  }

  async up(key: string): Promise<void> {
    const api = 'keyboard.up'
    await invoke(
      'keyboard.up',
      { api, timeoutMs: 0 },
      { page: await this.#page._brand(api), key },
    )
  }

  async press(key: string, options: { delay?: number } = {}): Promise<void> {
    const api = 'keyboard.press'
    await invoke(
      'keyboard.press',
      { api, timeoutMs: 0 },
      {
        page: await this.#page._brand(api),
        key,
        options: pick(options, ['delay']),
      },
    )
  }

  async type(text: string, options: { delay?: number } = {}): Promise<void> {
    const api = 'keyboard.type'
    await invoke(
      'keyboard.type',
      { api, timeoutMs: 0 },
      {
        page: await this.#page._brand(api),
        text,
        options: pick(options, ['delay']),
      },
    )
  }

  async insertText(text: string): Promise<void> {
    const api = 'keyboard.insertText'
    await invoke(
      'keyboard.insertText',
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
    options: { steps?: number } = {},
  ): Promise<void> {
    const api = 'mouse.move'
    await invoke(
      'mouse.move',
      { api, timeoutMs: 0 },
      {
        page: await this.#page._brand(api),
        x,
        y,
        options: pick(options, ['steps']),
      },
    )
  }

  async down(options: ButtonOptions = {}): Promise<void> {
    const api = 'mouse.down'
    await invoke(
      'mouse.down',
      { api, timeoutMs: 0 },
      {
        page: await this.#page._brand(api),
        options: pick(options, ['button', 'clickCount']),
      },
    )
  }

  async up(options: ButtonOptions = {}): Promise<void> {
    const api = 'mouse.up'
    await invoke(
      'mouse.up',
      { api, timeoutMs: 0 },
      {
        page: await this.#page._brand(api),
        options: pick(options, ['button', 'clickCount']),
      },
    )
  }

  click(
    x: number,
    y: number,
    options: ButtonOptions & { delay?: number } = {},
  ): Promise<void> {
    return this.#clickAt('mouse.click', x, y, options)
  }

  /** upstream's client sends a click with `clickCount: 2`. */
  dblclick(
    x: number,
    y: number,
    options: Omit<ButtonOptions, 'clickCount'> & { delay?: number } = {},
  ): Promise<void> {
    return this.#clickAt('mouse.dblclick', x, y, { ...options, clickCount: 2 })
  }

  async wheel(deltaX: number, deltaY: number): Promise<void> {
    const api = 'mouse.wheel'
    await invoke(
      'mouse.wheel',
      { api, timeoutMs: 0 },
      { page: await this.#page._brand(api), deltaX, deltaY },
    )
  }

  async #clickAt(
    api: string,
    x: number,
    y: number,
    options: ButtonOptions & { delay?: number },
  ): Promise<void> {
    await invoke(
      'mouse.click',
      { api, timeoutMs: 0 },
      {
        page: await this.#page._brand(api),
        x,
        y,
        options: pick(options, ['button', 'clickCount', 'delay']),
      },
    )
  }
}
