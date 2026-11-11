/**
 * `webStorage`: `page.localStorage` and `page.sessionStorage`, one Storage call per request,
 * as upstream's server evaluates `localStorage.getItem(...)` and the rest in the main frame's
 * utility world (server/page.ts `webStorage*` at v1.64.1). This isolated world shares the
 * document's Storage, and the page's own script cannot replace the methods it calls. A
 * document that may not use storage (an opaque origin) throws Chromium's `SecurityError`,
 * which reaches the script as upstream's does.
 */
import type { Value, WebStorageRequest } from './protocol'
import { ProbeError, type World } from './world'

export function webStorage(world: World, request: WebStorageRequest): Value {
  try {
    return call(world, request)
  } catch (error) {
    // Chromium's DOMException (`SecurityError`, `QuotaExceededError`) is the page's answer,
    // printed as upstream's evaluation prints it.
    if (error instanceof DOMException)
      throw new ProbeError('script', `${error.name}: ${error.message}`)
    throw error
  }
}

function call(world: World, request: WebStorageRequest): Value {
  const { window } = world.injected
  const storage =
    request.kind === 'local' ? window.localStorage : window.sessionStorage
  switch (request.action) {
    case 'items': {
      // Upstream's loop: by index, skipping a key that went away under it.
      const items: { name: string; value: string }[] = []
      for (let index = 0; index < storage.length; index++) {
        const name = storage.key(index)
        if (name !== null)
          items.push({ name, value: storage.getItem(name) ?? '' })
      }
      return { kind: 'value', value: items }
    }
    case 'getItem':
      return { kind: 'value', value: storage.getItem(named(request)) }
    case 'setItem':
      storage.setItem(named(request), request.value ?? '')
      return { kind: 'value', value: null }
    case 'removeItem':
      storage.removeItem(named(request))
      return { kind: 'value', value: null }
    case 'clear':
      storage.clear()
      return { kind: 'value', value: null }
  }
}

function named(request: WebStorageRequest): string {
  if (request.name === undefined)
    throw new ProbeError(
      'internal',
      `webStorage ${request.action} needs a name`,
    )
  return request.name
}
