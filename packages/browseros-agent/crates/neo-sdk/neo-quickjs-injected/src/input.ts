/**
 * Playwright: `client/input.ts` `Keyboard`
 * Playwright: `client/input.ts` `Mouse`
 * Playwright: `client/input.ts` `Touchscreen`
 *
 * `page.keyboard`, `page.mouse` and `page.touchscreen`: each method sends Playwright's own
 * Page command (`keyboardDown` … `touchscreenTap`) to the page's tab, with no timeout, as
 * Playwright's client sends them (`kNoTimeout`); the run's deadline still bounds them. Rust
 * keeps the tab's keyboard and mouse state between calls (neo-runtime `mouse_and_touch_input.rs`), so a
 * key from `down` stays held for later calls until `up`, or until the run ends, when Neo
 * releases what the run left held.
 *
 * What differs from Playwright:
 * - `touchscreen.tap` needs no `hasTouch` context: Neo's context is the user's BrowserOS
 *   context, which has no such option, and its tab takes touch input as a `hasTouch` one does.
 * - Keys and buttons a run leaves held are released when it ends; Playwright keeps them held
 *   for the page's life.
 */

import type {
  PageKeyboardDownParams,
  PageKeyboardInsertTextParams,
  PageKeyboardPressParams,
  PageKeyboardTypeParams,
  PageKeyboardUpParams,
  PageMouseClickParams,
  PageMouseDownParams,
  PageMouseMoveParams,
  PageMouseUpParams,
  PageMouseWheelParams,
  PageTouchscreenTapParams,
} from '@browseros/neo-runtime-spec/serve'
import type { Page } from './page'
import {
  PAGE_KEYBOARD_DOWN_KEYS,
  PAGE_KEYBOARD_INSERT_TEXT_KEYS,
  PAGE_KEYBOARD_PRESS_KEYS,
  PAGE_KEYBOARD_TYPE_KEYS,
  PAGE_KEYBOARD_UP_KEYS,
  PAGE_MOUSE_CLICK_KEYS,
  PAGE_MOUSE_DOWN_KEYS,
  PAGE_MOUSE_MOVE_KEYS,
  PAGE_MOUSE_UP_KEYS,
  PAGE_MOUSE_WHEEL_KEYS,
  PAGE_TOUCHSCREEN_TAP_KEYS,
  pickParams,
} from './protocol'

/** `keyboard.type`'s options, as Playwright's `PageKeyboardTypeOptions`. */
export type KeyboardTypeOptions = Omit<PageKeyboardTypeParams, 'text'>

/** `keyboard.press`'s options, as Playwright's `PageKeyboardPressOptions`. */
export type KeyboardPressOptions = Omit<PageKeyboardPressParams, 'key'>

/** `mouse.down` and `mouse.up`'s options. */
export type MouseButtonOptions = PageMouseDownParams

/** `mouse.click`'s options, as Playwright's `PageMouseClickOptions`. */
export type MouseClickOptions = Omit<PageMouseClickParams, 'x' | 'y'>

/** Playwright: `client/input.ts` `Keyboard` */
export class Keyboard {
  readonly #page: Page

  constructor(page: Page) {
    this.#page = page
  }

  async down(key: string): Promise<void> {
    await this.#page._sendInput(
      'keyboard.down',
      'keyboardDown',
      pickParams(PAGE_KEYBOARD_DOWN_KEYS, { key }) as PageKeyboardDownParams,
    )
  }

  async up(key: string): Promise<void> {
    await this.#page._sendInput(
      'keyboard.up',
      'keyboardUp',
      pickParams(PAGE_KEYBOARD_UP_KEYS, { key }) as PageKeyboardUpParams,
    )
  }

  async insertText(text: string): Promise<void> {
    await this.#page._sendInput(
      'keyboard.insertText',
      'keyboardInsertText',
      pickParams(PAGE_KEYBOARD_INSERT_TEXT_KEYS, {
        text,
      }) as PageKeyboardInsertTextParams,
    )
  }

  async type(text: string, options: KeyboardTypeOptions = {}): Promise<void> {
    await this.#page._sendInput(
      'keyboard.type',
      'keyboardType',
      pickParams(PAGE_KEYBOARD_TYPE_KEYS, {
        text,
        ...options,
      }) as PageKeyboardTypeParams,
    )
  }

  async press(key: string, options: KeyboardPressOptions = {}): Promise<void> {
    await this.#page._sendInput(
      'keyboard.press',
      'keyboardPress',
      pickParams(PAGE_KEYBOARD_PRESS_KEYS, {
        key,
        ...options,
      }) as PageKeyboardPressParams,
    )
  }
}

/** Playwright: `client/input.ts` `Mouse` */
export class Mouse {
  readonly #page: Page

  constructor(page: Page) {
    this.#page = page
  }

  async move(
    x: number,
    y: number,
    options: { steps?: number } = {},
  ): Promise<void> {
    await this.#page._sendInput(
      'mouse.move',
      'mouseMove',
      pickParams(PAGE_MOUSE_MOVE_KEYS, {
        x,
        y,
        ...options,
      }) as PageMouseMoveParams,
    )
  }

  async down(options: MouseButtonOptions = {}): Promise<void> {
    await this.#page._sendInput(
      'mouse.down',
      'mouseDown',
      pickParams(PAGE_MOUSE_DOWN_KEYS, { ...options }) as PageMouseDownParams,
    )
  }

  async up(options: MouseButtonOptions = {}): Promise<void> {
    await this.#page._sendInput(
      'mouse.up',
      'mouseUp',
      pickParams(PAGE_MOUSE_UP_KEYS, { ...options }) as PageMouseUpParams,
    )
  }

  async click(
    x: number,
    y: number,
    options: MouseClickOptions = {},
  ): Promise<void> {
    await this._click('mouse.click', x, y, options)
  }

  /** Playwright's client-side double click: a `mouseClick` of two, under its own name. */
  async dblclick(
    x: number,
    y: number,
    options: Omit<MouseClickOptions, 'clickCount'> = {},
  ): Promise<void> {
    await this._click('mouse.dblclick', x, y, { ...options, clickCount: 2 })
  }

  async wheel(deltaX: number, deltaY: number): Promise<void> {
    await this.#page._sendInput(
      'mouse.wheel',
      'mouseWheel',
      pickParams(PAGE_MOUSE_WHEEL_KEYS, {
        deltaX,
        deltaY,
      }) as PageMouseWheelParams,
    )
  }

  /** Sends `Page.mouseClick` for `click` and `dblclick`. */
  async _click(
    apiName: 'mouse.click' | 'mouse.dblclick',
    x: number,
    y: number,
    options: MouseClickOptions,
  ): Promise<void> {
    await this.#page._sendInput(
      apiName,
      'mouseClick',
      pickParams(PAGE_MOUSE_CLICK_KEYS, {
        x,
        y,
        ...options,
      }) as PageMouseClickParams,
    )
  }
}

/** Playwright: `client/input.ts` `Touchscreen` */
export class Touchscreen {
  readonly #page: Page

  constructor(page: Page) {
    this.#page = page
  }

  async tap(x: number, y: number): Promise<void> {
    await this.#page._sendInput(
      'touchscreen.tap',
      'touchscreenTap',
      pickParams(PAGE_TOUCHSCREEN_TAP_KEYS, {
        x,
        y,
      }) as PageTouchscreenTapParams,
    )
  }
}
