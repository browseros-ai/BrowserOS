import { afterEach, expect, it, mock } from 'bun:test'
import { parseDiagnostics } from '@browseros/diagnostics/contract'
import { requestDiagnostics } from '@browseros/diagnostics/extension'
import fixture from '../../../../packages/diagnostics/tests/fixture.json'

const listeners = new Set<(message: unknown) => unknown>()
mock.module('webextension-polyfill', () => ({
  default: {
    runtime: {
      onMessage: {
        addListener: (listener: (message: unknown) => unknown) =>
          listeners.add(listener),
        removeListener: (listener: (message: unknown) => unknown) =>
          listeners.delete(listener),
      },
    },
  },
}))
const { defineExtensionMessaging } = await import('@webext-core/messaging')
const originalChrome = globalThis.chrome
afterEach(() => {
  globalThis.chrome = originalChrome
  listeners.clear()
})

it('receives fresh diagnostics alongside the app messaging handlers', async () => {
  const snapshot = parseDiagnostics(fixture)
  if (!snapshot) throw new Error('Invalid diagnostics fixture')
  const messaging = defineExtensionMessaging<{ unrelated(): void }>()
  messaging.onMessage('unrelated', () => {})
  globalThis.chrome = {
    runtime: {
      sendMessage: async (request: unknown) => {
        // Chrome delivers to every handler. An unrelated handler's protocol
        // error can win the response race before diagnostics finishes reading.
        for (const listener of listeners) listener(request)
        return { snapshot, cached: false }
      },
    },
    storage: { local: { get: async () => ({}) } },
  } as unknown as typeof chrome

  await expect(requestDiagnostics(0)).resolves.toEqual({
    snapshot,
    cached: false,
  })
  messaging.removeAllListeners()
})
