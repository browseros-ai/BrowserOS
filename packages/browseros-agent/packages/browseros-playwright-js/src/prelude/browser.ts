/**
 * The browser a run is connected to: Neo's BrowserOS, with the owner's one profile as its only
 * context. Neo owns the browser's lifetime, so a script can neither close it nor make another
 * context (those methods are in the unsupported table).
 */
import type { BrowserContext } from './context'
import { EventEmitter } from './emitter'
import { markHandle } from './options'

export class Browser extends EventEmitter {
  readonly #context: BrowserContext
  readonly #versionValue: string

  constructor(context: BrowserContext, version: string) {
    super()
    this.#context = context
    this.#versionValue = version
    context._browser = this
    markHandle(this)
  }

  contexts(): BrowserContext[] {
    return [this.#context]
  }

  version(): string {
    return this.#versionValue
  }

  isConnected(): boolean {
    return true
  }

  toJSON(): { version: string } {
    return { version: this.#versionValue }
  }
}
