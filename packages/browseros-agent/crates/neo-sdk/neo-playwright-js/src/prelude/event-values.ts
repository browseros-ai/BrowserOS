/**
 * The values page events carry: Dialog, Download and ConsoleMessage, following
 * playwright-core 1.64.1's client classes. Each is built from one record of Rust's event log
 * (wire.ts `EventRecord`); dialogs and downloads keep the id Rust answers them by.
 */
import { captureSite } from './call-site'
import type { ElementHandle } from './element-handle'
import { send } from './generated'
import { inspectCustom, markHandle } from './options'
import type { Page } from './page'
import type {
  ConsoleInfo,
  DialogInfo,
  DownloadInfo,
  FileChooserInfo,
  PageErrorInfo,
} from './wire'
import type { Worker } from './worker'

/** One intercepted input. All upload forms share ElementHandle.setInputFiles' validation. */
export class FileChooser {
  readonly #pageValue: Page
  readonly #element: ElementHandle
  readonly #multiple: boolean

  constructor(page: Page, info: FileChooserInfo) {
    this.#pageValue = page
    const element = page._handle(info.element)
    if (!element) throw new Error('Rust returned a chooser without its input')
    this.#element = element
    this.#multiple = info.isMultiple
    markHandle(this)
  }

  element(): ElementHandle {
    return this.#element
  }

  isMultiple(): boolean {
    return this.#multiple
  }

  page(): Page {
    return this.#pageValue
  }

  async setFiles(
    files: unknown,
    options?: Parameters<ElementHandle['setInputFiles']>[1],
  ): Promise<void> {
    await this.#element.setInputFiles(files, options)
  }
}

export class Dialog {
  readonly #pageValue: Page
  readonly #info: DialogInfo

  constructor(page: Page, info: DialogInfo) {
    this.#pageValue = page
    this.#info = info
    markHandle(this)
  }

  page(): Page | null {
    return this.#pageValue
  }

  type(): string {
    return this.#info.type
  }

  message(): string {
    return this.#info.message
  }

  defaultValue(): string {
    return this.#info.defaultValue
  }

  async accept(promptText?: string): Promise<void> {
    const site = captureSite()
    const api = 'dialog.accept'
    // An undefined `promptText` leaves the input, as the bootstrap drops undefined keys.
    await send['dialog.accept'](
      site,
      { api, timeoutMs: 0 },
      {
        page: await this.#pageValue._brand(api),
        dialog: this.#info.id,
        promptText,
      },
    )
  }

  /** As upstream, a dialog whose page already closed counts as dismissed. */
  async dismiss(): Promise<void> {
    const site = captureSite()
    const api = 'dialog.dismiss'
    // Outside the try, as page.close's: the catch reads the op's code from the door.
    const page = await this.#pageValue._brand(api)
    try {
      await send['dialog.dismiss'](
        site,
        { api, timeoutMs: 0 },
        { page, dialog: this.#info.id },
      )
    } catch (error) {
      if (Reflect.get(Object(error), 'code') === 'target_closed') return
      throw error
    }
  }
}

export class Download {
  readonly #pageValue: Page
  readonly #info: DownloadInfo

  constructor(page: Page, info: DownloadInfo) {
    this.#pageValue = page
    this.#info = info
    markHandle(this)
  }

  page(): Page {
    return this.#pageValue
  }

  url(): string {
    return this.#info.url
  }

  suggestedFilename(): string {
    return this.#info.suggestedFilename
  }

  /** Waits for the download to finish; its host path, or its failure as an error. */
  async path(): Promise<string> {
    const site = captureSite()
    const api = 'download.path'
    const done = await send['download.path'](
      site,
      { api, timeoutMs: 0 },
      { download: this.#info.id },
    )
    if (done.path === null)
      throw new Error(`${api}: ${done.failure ?? 'download failed'}`)
    return done.path
  }

  /** Waits for the download to finish; null, or why it failed. */
  async failure(): Promise<string | null> {
    const site = captureSite()
    const api = 'download.failure'
    const done = await send['download.path'](
      site,
      { api, timeoutMs: 0 },
      { download: this.#info.id },
    )
    return done.failure
  }

  async saveAs(path: string): Promise<void> {
    const site = captureSite()
    const api = 'download.saveAs'
    await send['download.saveAs'](
      site,
      { api, timeoutMs: 0 },
      { download: this.#info.id, path },
    )
  }
}

export class ConsoleMessage {
  readonly #pageValue: Page
  readonly #workerValue: Worker | null
  readonly #info: ConsoleInfo

  constructor(page: Page, info: ConsoleInfo) {
    this.#workerValue = info.worker ? page._worker(info.worker) : null
    this.#pageValue = page
    this.#info = info
    markHandle(this)
  }

  page(): Page | null {
    return this.#pageValue
  }

  worker(): Worker | null {
    return this.#workerValue
  }

  type(): string {
    return this.#info.type
  }

  text(): string {
    return this.#info.text
  }

  location(): {
    url: string
    line: number
    column: number
    lineNumber: number
    columnNumber: number
  } {
    const { url, lineNumber, columnNumber } = this.#info.location
    return {
      url,
      line: lineNumber,
      column: columnNumber,
      lineNumber,
      columnNumber,
    }
  }

  timestamp(): number {
    return this.#info.timestamp
  }

  toString(): string {
    return this.#info.text
  }

  [inspectCustom](): string {
    return this.text()
  }
}

/** `pageerror` carries the page's exception as an Error, as upstream's client parses it. */
export function pageError(info: PageErrorInfo): Error {
  const error = new Error(info.message)
  error.name = info.name
  error.stack = info.stack
  return error
}
