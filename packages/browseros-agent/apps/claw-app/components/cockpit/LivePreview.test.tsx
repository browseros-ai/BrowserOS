import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'
import * as auditHooks from '@/modules/api/audit.hooks'

mock.module('@/modules/api/audit.hooks', () => ({
  ...auditHooks,
  useApiBaseUrl: () => 'http://127.0.0.1:9210',
}))

/** Track the real component's connection ownership without recording any page data. */
class PreviewSource extends EventTarget {
  static sources: PreviewSource[] = []
  closed = false
  constructor(readonly url: string) {
    super()
    PreviewSource.sources.push(this)
  }
  close() {
    this.closed = true
  }
}

const globalNames = [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Event',
  'EventSource',
  'IS_REACT_ACT_ENVIRONMENT',
]
const originals = new Map(
  globalNames.map((name) => [
    name,
    Object.getOwnPropertyDescriptor(globalThis, name),
  ]),
)
const { LivePreview } = await import('./LivePreview')
let root: Root
let visibility: DocumentVisibilityState

beforeEach(async () => {
  PreviewSource.sources = []
  visibility = 'visible'
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
  const globals = {
    window: dom.window,
    document: dom.document,
    navigator: dom.window.navigator,
    HTMLElement: dom.window.HTMLElement,
    Node: dom.window.Node,
    Event: dom.window.Event,
    EventSource: PreviewSource,
    IS_REACT_ACT_ENVIRONMENT: true,
  }
  for (const [name, value] of Object.entries(globals)) {
    Object.defineProperty(globalThis, name, {
      configurable: true,
      writable: true,
      value,
    })
  }
  Object.defineProperty(document, 'visibilityState', {
    configurable: true,
    get: () => visibility,
  })
  const { createRoot } = await import('react-dom/client')
  const container = document.getElementById('root')
  if (!container) throw new Error('Missing test container')
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  for (const [name, descriptor] of originals) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
})

async function renderPreviews() {
  await act(async () => {
    const sessions = Array.from({ length: 6 }, (_, i) => `session-${i}`)
    root.render(
      sessions.map((sessionId) => (
        <LivePreview key={sessionId} sessionId={sessionId} site="example.com" />
      )),
    )
  })
}

async function setVisibility(next: DocumentVisibilityState) {
  await act(async () => {
    visibility = next
    document.dispatchEvent(new Event('visibilitychange'))
  })
}

const openSources = () =>
  PreviewSource.sources.filter((source) => !source.closed)

describe('LivePreview connection lifecycle', () => {
  it('releases all preview connections when hidden and reconnects when visible', async () => {
    await renderPreviews()
    expect(openSources()).toHaveLength(6)
    await setVisibility('hidden')
    expect(openSources()).toHaveLength(0)
    await setVisibility('visible')
    expect(openSources()).toHaveLength(6)
    expect(PreviewSource.sources).toHaveLength(12)
    await act(async () => root.render(null))
    expect(openSources()).toHaveLength(0)
    await setVisibility('hidden')
    await setVisibility('visible')
    expect(openSources()).toHaveLength(0)
  })

  it('does not consume connections when a cockpit mounts in a background tab', async () => {
    visibility = 'hidden'
    await renderPreviews()
    expect(PreviewSource.sources).toHaveLength(0)
    await setVisibility('visible')
    expect(openSources()).toHaveLength(6)
  })
})
