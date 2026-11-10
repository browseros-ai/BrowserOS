/**
 * Playwright: `client/fileChooser.ts` `FileChooser`
 *
 * The file chooser a page opened while the run listened for one (`page.on('filechooser')`):
 * the page, whether the input takes several files, and the input as an `ElementHandle`, which
 * Rust makes when `element()` first reads it (neo-runtime `tabs.rs` `file_chooser_element`).
 *
 * Deviation: `client/fileChooser.ts` `FileChooser.setFiles`: `return await this._elementHandle.setInputFiles(files, options);`
 * Playwright sets the files through the input's `setInputFiles`, which Neo does not serve yet
 * (med-pw-dom), so `setFiles` throws an Error naming that instead of choosing nothing.
 */

import type { Connection } from './connection'
import type { ElementHandle } from './element-handle'
import { handleFrom } from './js-handle'
import type { Page } from './page'
import { get } from './runtime-bridge'

/** Neo: why `setFiles` refuses until `setInputFiles` is served. */
const SET_FILES_UNSERVED =
  'Neo does not serve FileChooser.setFiles yet: it sets the input through setInputFiles, which Neo does not serve yet'

export class FileChooser {
  readonly #page: Page
  readonly #chooser: string
  readonly #isMultiple: boolean
  readonly #connection: Connection
  #element: ElementHandle | undefined

  /** `chooser` is the guid Rust names it by (`filechooser@<targetId>:<n>`). */
  constructor(
    page: Page,
    chooser: string,
    isMultiple: boolean,
    connection: Connection,
  ) {
    this.#page = page
    this.#chooser = chooser
    this.#isMultiple = isMultiple
    this.#connection = connection
  }

  /** The input the chooser opened for, the same object at every read. */
  element(): ElementHandle {
    if (!this.#element)
      // Timed by the chooser's page, as every frame of a page shares its `TimeoutSettings`.
      this.#element = handleFrom(
        this.#connection,
        get(this.#chooser, 'element'),
        this.#page.mainFrame(),
      ) as ElementHandle
    return this.#element
  }

  isMultiple(): boolean {
    return this.#isMultiple
  }

  page(): Page {
    return this.#page
  }

  async setFiles(files: unknown, options?: unknown): Promise<void> {
    void files
    void options
    throw new Error(SET_FILES_UNSERVED)
  }
}
