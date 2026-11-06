import { get } from './runtime-bridge'

/**
 * Playwright's `Page`, backed by live reads from the tab this run is attached to.
 *
 * Known limitation: once the tab is closed, `url()` returns the last URL this object returned,
 * not the last URL the tab committed. Navigate `/a` to `/final`, close the tab before the
 * script calls `url()` again, and `url()` returns `/a`; Playwright returns `/final`. No CDP
 * source answers after a close, so matching Playwright here needs event tracking, which the
 * no-cache design leaves out (sync-getters-design.md, "Known limitation").
 */
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
