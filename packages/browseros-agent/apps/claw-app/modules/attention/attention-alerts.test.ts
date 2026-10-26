import { afterEach, describe, expect, it } from 'bun:test'
import type { OpenHelp } from './attention.helpers'
import { createAttentionAlerts } from './attention-alerts'

const help: OpenHelp = {
  requestId: 'req-1',
  sessionId: 's1',
  browserTabId: 7,
  title: 'Codex needs you to sign in to netflix.com',
  message:
    'Open the netflix.com page, do what it needs, then hand control back.',
  requestedAt: 1,
}

const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'chrome')
afterEach(() => {
  if (descriptor) Object.defineProperty(globalThis, 'chrome', descriptor)
  else Reflect.deleteProperty(globalThis, 'chrome')
})

/** A browser with the extension APIs the alerts use, recording each call. */
function stubChrome(withNative: boolean) {
  const calls: string[] = []
  const chrome = {
    runtime: { getURL: (path: string) => path, lastError: undefined },
    notifications: {
      create: async (id: string) => calls.push(`notify ${id}`),
      clear: async (id: string) => calls.push(`clear ${id}`),
    },
    tabs: { get: async () => ({ windowId: 3 }) },
    windows: {
      update: async (_id: number, info: { drawAttention?: boolean }) =>
        calls.push(`drawAttention ${info.drawAttention}`),
    },
    action: {
      setBadgeBackgroundColor: async () => {},
      setBadgeTextColor: async () => {},
      setBadgeText: async ({ text }: { text: string }) =>
        calls.push(`toolbar "${text}"`),
      setTitle: async () => {},
    },
    browserOS: withNative
      ? {
          requestAttention: (
            options: { critical?: boolean },
            done: () => void,
          ) => {
            calls.push(`dock bounce critical=${options.critical}`)
            done()
          },
          cancelAttention: (done: () => void) => {
            calls.push('dock bounce stop')
            done()
          },
          setDockBadge: (text: string, done: () => void) => {
            calls.push(`dock badge "${text}"`)
            done()
          },
        }
      : {},
  }
  Object.defineProperty(globalThis, 'chrome', {
    configurable: true,
    writable: true,
    value: chrome,
  })
  return calls
}

describe('createAttentionAlerts', () => {
  it('keeps the dock bouncing and badged through BrowserOS when it can', async () => {
    const calls = stubChrome(true)
    const alerts = createAttentionAlerts()

    await alerts.show(help)
    await alerts.setCount(1)
    await alerts.setCount(0)

    expect(calls).toEqual([
      'notify req-1',
      'dock bounce critical=true',
      'dock badge "1"',
      'toolbar "1"',
      'dock badge ""',
      'dock bounce stop',
      'toolbar ""',
    ])
  })

  it('falls back to one bounce of the window on older builds', async () => {
    const calls = stubChrome(false)
    const alerts = createAttentionAlerts()

    await alerts.show(help)
    await alerts.clear(help.requestId, help.browserTabId)

    expect(calls).toEqual([
      'notify req-1',
      'drawAttention true',
      'clear req-1',
      'drawAttention false',
    ])
  })
})
