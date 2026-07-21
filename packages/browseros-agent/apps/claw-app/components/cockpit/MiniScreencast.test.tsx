/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'
import { renderToStaticMarkup } from 'react-dom/server'
import * as _auditHooks from '@/modules/api/audit.hooks'

mock.module('@/modules/api/audit.hooks', () => ({
  ..._auditHooks,
  useSessionPreviewUrl: (sessionId: string, refresh: number) =>
    `/sessions/${sessionId}/preview?refresh=${refresh}`,
}))

const { MiniScreencast } = await import('./MiniScreencast')

class FakeImage {
  static instances: FakeImage[] = []

  onload: (() => void) | null = null
  onerror: (() => void) | null = null
  src = ''

  constructor() {
    FakeImage.instances.push(this)
  }
}

const globalDescriptors = new Map(
  [
    'window',
    'document',
    'navigator',
    'HTMLElement',
    'Node',
    'Event',
    'Image',
  ].map((name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)]),
)

let root: Root
let container: HTMLElement
let previewNow = 100
let refreshPreview: (() => void) | null = null
const realDateNow = Date.now

beforeEach(async () => {
  FakeImage.instances.length = 0
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
  previewNow = 100
  Date.now = () => previewNow
  Object.defineProperties(dom.window, {
    setInterval: {
      configurable: true,
      value: (callback: () => void) => {
        refreshPreview = callback
        return 1
      },
    },
    clearInterval: { configurable: true, value: () => undefined },
  })
  const globals = {
    window: dom.window,
    document: dom.document,
    navigator: dom.window.navigator,
    HTMLElement: dom.window.HTMLElement,
    Node: dom.window.Node,
    Event: dom.window.Event,
    Image: FakeImage,
  }
  for (const [name, value] of Object.entries(globals)) {
    Object.defineProperty(globalThis, name, {
      configurable: true,
      writable: true,
      value,
    })
  }
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })
  container = dom.document.getElementById('root') as unknown as HTMLElement
  const { createRoot } = await import('react-dom/client')
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  for (const [name, descriptor] of globalDescriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
  Reflect.deleteProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
  Date.now = realDateNow
  refreshPreview = null
})

describe('MiniScreencast', () => {
  it('renders the placeholder globe and host without a browser tab', () => {
    const html = renderToStaticMarkup(
      <MiniScreencast site="No browser activity" sessionId="session-empty" />,
    )
    expect(html).toContain('No browser activity')
    expect(html).not.toContain('data:image/jpeg;base64,')
    expect(html).not.toContain('data-preview-url')
  })

  it('keys the canonical JPEG URL by session and refresh token', () => {
    expect(
      _auditHooks.sessionPreviewUrl(
        'session / one',
        123,
        'http://127.0.0.1:9200',
      ),
    ).toBe(
      'http://127.0.0.1:9200/api/v1/sessions/session%20%2F%20one/preview?refresh=123',
    )
  })

  it('falls back to the placeholder until a preview decodes', () => {
    const html = renderToStaticMarkup(
      <MiniScreencast site="example.com" sessionId="session-1" />,
    )
    expect(html).not.toContain('data:image/jpeg;base64,')
    expect(html).not.toContain('data-preview-url')
    expect(html).toContain('example.com')
  })

  it('shows the live dot only when live=true', () => {
    const liveHtml = renderToStaticMarkup(
      <MiniScreencast site="example.com" sessionId="session-1" live />,
    )
    const idleHtml = renderToStaticMarkup(
      <MiniScreencast site="example.com" sessionId="session-1" />,
    )
    expect(liveHtml).toMatch(/animate-pulse-dot/)
    expect(idleHtml).not.toMatch(/animate-pulse-dot/)
  })

  it('retains a decoded frame across failed refreshes but clears it for a new session', async () => {
    await act(async () => {
      root.render(<MiniScreencast site="first.example" sessionId="session-1" />)
    })
    const firstFrame = FakeImage.instances.at(-1)
    if (!firstFrame) throw new Error('initial preview image was not created')
    await act(async () => firstFrame.onload?.())
    expect(
      container.querySelector('[data-preview-url]')?.getAttribute('src'),
    ).toContain('/sessions/session-1/preview?refresh=100')

    previewNow = 101
    await act(async () => {
      refreshPreview?.()
    })
    expect(
      container.querySelector('[data-preview-url]')?.getAttribute('src'),
    ).toContain('/sessions/session-1/preview?refresh=100')

    const failedReplacement = FakeImage.instances.at(-1)
    if (!failedReplacement) {
      throw new Error('replacement preview image was not created')
    }
    await act(async () => failedReplacement.onerror?.())
    expect(
      container.querySelector('[data-preview-url]')?.getAttribute('src'),
    ).toContain('/sessions/session-1/preview?refresh=100')

    previewNow = 102
    await act(async () => {
      refreshPreview?.()
    })
    const recoveredFrame = FakeImage.instances.at(-1)
    if (!recoveredFrame) {
      throw new Error('recovery image was not created')
    }
    await act(async () => recoveredFrame.onload?.())
    expect(
      container.querySelector('[data-preview-url]')?.getAttribute('src'),
    ).toContain('/sessions/session-1/preview?refresh=102')

    await act(async () => {
      root.render(
        <MiniScreencast site="second.example" sessionId="session-2" />,
      )
    })
    expect(container.querySelector('[data-preview-url]')).toBeNull()

    const replacement = FakeImage.instances.at(-1)
    if (!replacement) throw new Error('replacement image was not created')
    await act(async () => replacement.onerror?.())

    expect(container.querySelector('[data-preview-url]')).toBeNull()
    expect(container.textContent).toContain('second.example')
  })
})
