/**
 * The browser a run is connected to: Neo's BrowserOS, with the owner's one profile as its only
 * context. Neo owns the browser's lifetime, so a script can neither close it nor make another
 * context (those methods are in the unsupported table). claw-sdk's bootstrap gives a run no
 * browser version, so `version()` is empty.
 */
import type { BrowserContext } from './context'
import { EventEmitter } from './emitter'
import { markHandle } from './options'

export class Browser extends EventEmitter {
  readonly #context: BrowserContext

  constructor(context: BrowserContext) {
    super()
    this.#context = context
    context._browser = this
    markHandle(this)
  }

  contexts(): BrowserContext[] {
    return [this.#context]
  }

  version(): string {
    return ''
  }

  isConnected(): boolean {
    return true
  }

  toJSON(): { version: string } {
    return { version: this.version() }
  }
}
