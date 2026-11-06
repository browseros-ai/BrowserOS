import { get } from './runtime-bridge'

/** Playwright's `Page`, backed by live reads from the tab this run is attached to. */
export class Page {
  readonly #guid: string
  /**
   * The only state the SDK keeps. Playwright still answers `url()` after the page closes,
   * with the last URL, while every CDP source errors; seeded with the URL at run start.
   */
  #lastUrl: string

  constructor(guid: string, url: string) {
    this.#guid = guid
    this.#lastUrl = url
  }

  url(): string {
    const url = get(this.#guid, 'url')
    if (url !== null) this.#lastUrl = url
    return this.#lastUrl
  }

  isClosed(): boolean {
    return get(this.#guid, 'isClosed')
  }
}
