import {
  afterAll,
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  mock,
} from 'bun:test'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'
import * as api from '@/modules/api/client'

const calls: string[] = []

// Every request is recorded and left unanswered; only which ones start matters.
mock.module('@/modules/api/client', () => ({
  ...api,
  apiClient: async () =>
    new Proxy(
      {},
      {
        // Not thenable, so awaiting the client does not treat it as a promise.
        get: (_target, name) =>
          name === 'then'
            ? undefined
            : () => {
                calls.push(String(name))
                return new Promise(() => {})
              },
      },
    ),
}))

const { useReplayData } = await import('./replay.data')

afterAll(() => mock.restore())

const globalNames = ['window', 'document', 'navigator', 'HTMLElement', 'Node']
const globalDescriptors = new Map(
  globalNames.map((name) => [
    name,
    Object.getOwnPropertyDescriptor(globalThis, name),
  ]),
)

let root: Root
let queryClient: QueryClient
let recordingState: string | undefined

function Probe({ enabled }: { enabled: boolean }) {
  recordingState = useReplayData('session-1', { enabled }).recordingState
  return null
}

beforeEach(async () => {
  calls.length = 0
  recordingState = undefined
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
  for (const [name, value] of Object.entries({
    window: dom.window,
    document: dom.document,
    navigator: dom.window.navigator,
    HTMLElement: dom.window.HTMLElement,
    Node: dom.window.Node,
  })) {
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
  queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const { createRoot } = await import('react-dom/client')
  root = createRoot(
    dom.document.getElementById('root') as unknown as HTMLElement,
  )
})

afterEach(async () => {
  await act(async () => root.unmount())
  queryClient.clear()
  for (const [name, descriptor] of globalDescriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
  Reflect.deleteProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
})

async function renderProbe(enabled: boolean) {
  await act(async () => {
    root.render(
      <QueryClientProvider client={queryClient}>
        <Probe enabled={enabled} />
      </QueryClientProvider>,
    )
  })
  await act(async () => {
    await Bun.sleep(0)
  })
}

describe('useReplayData loading', () => {
  it('tests that a task with replays on asks for its recording', async () => {
    await renderProbe(true)
    expect(calls).toContain('getRecording')
    expect(recordingState).toBe('checking')
  })

  it('tests that a task with replays off never asks for its recording', async () => {
    await renderProbe(false)
    expect(calls).toContain('getSession')
    expect(calls).not.toContain('getRecording')
    expect(calls).not.toContain('downloadRecordingEvents')
    expect(recordingState).toBe('settled')
  })
})
