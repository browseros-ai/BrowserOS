/**
 * Playwright: `client/download.ts` `Download`
 *
 * A download the run's `download` event handed out: its page, URL and suggested name, known at
 * once, and the file, which Rust serves on the download's guid (`download@<guid>`) as
 * Playwright's `Artifact` commands (neo-runtime `downloads.rs`). `path`, `failure` and `delete`
 * wait for the download to end, as Playwright's do, bounded only by the run.
 *
 * The browser saves the file where it saves the user's downloads, under its own name, so
 * `path()` names that file on the host, and it stays after the run unless `delete()` removes it.
 *
 * Deviation: `client/artifact.ts` `Artifact.createReadStream`: `const result = await this._channel.stream({}, kNoTimeout);`
 * Playwright answers with a Node `Readable` over the file. A Neo script has no streams, so
 * `createReadStream` throws an Error saying so.
 */

import type { Connection } from './connection'
import type { Page } from './page'

/** Neo: why `createReadStream` throws. */
const NO_STREAMS =
  'Neo cannot stream a download: its scripts have no streams; download.path() names the file'

export class Download {
  readonly #page: Page
  readonly #url: string
  readonly #suggestedFilename: string
  readonly #guid: string
  readonly #connection: Connection

  /** `guid` is the one Rust serves the download's file on (`download@<guid>`). */
  constructor(
    page: Page,
    url: string,
    suggestedFilename: string,
    guid: string,
    connection: Connection,
  ) {
    this.#page = page
    this.#url = url
    this.#suggestedFilename = suggestedFilename
    this.#guid = guid
    this.#connection = connection
  }

  page(): Page {
    return this.#page
  }

  url(): string {
    return this.#url
  }

  suggestedFilename(): string {
    return this.#suggestedFilename
  }

  async path(): Promise<string> {
    const { value } = await this.#connection.sendArtifact(
      'download.path',
      this.#guid,
      'pathAfterFinished',
      {},
      0,
    )
    return value
  }

  async saveAs(path: string): Promise<void> {
    await this.#connection.sendArtifact(
      'download.saveAs',
      this.#guid,
      'saveAs',
      { path },
      0,
    )
  }

  async failure(): Promise<string | null> {
    const { error } = await this.#connection.sendArtifact(
      'download.failure',
      this.#guid,
      'failure',
      {},
      0,
    )
    return error || null
  }

  async createReadStream(): Promise<never> {
    throw new Error(NO_STREAMS)
  }

  async cancel(): Promise<void> {
    await this.#connection.sendArtifact(
      'download.cancel',
      this.#guid,
      'cancel',
      {},
      0,
    )
  }

  async delete(): Promise<void> {
    await this.#connection.sendArtifact(
      'download.delete',
      this.#guid,
      'delete',
      {},
      0,
    )
  }
}
