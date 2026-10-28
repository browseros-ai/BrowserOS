import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import type {
  AuditAgentList,
  SessionList,
  SessionSummary,
} from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'
import { MemoryRouter } from 'react-router'
import * as api from '@/modules/api/client'

const agents: AuditAgentList = {
  items: [
    {
      slug: 'ultrafast-bench',
      label: 'ultrafast-bench',
      taskCount: 30,
      lastStartedAt: 3,
    },
    { slug: 'claude', label: 'Claude', taskCount: 12, lastStartedAt: 2 },
    { slug: 'clitest', label: 'clitest', taskCount: 1, lastStartedAt: 1 },
  ],
}

function task(slug: string, label: string): SessionSummary {
  return {
    sessionId: `${slug}-1`,
    slug,
    label,
    name: 'Browsed www.amazon.in',
    startedAt: 100,
    durationMs: 1000,
    dispatchCount: 1,
    toolSequence: ['navigate'],
    status: 'done',
    errorCount: 0,
  }
}

// The server filters by slug, so a filtered page only holds that agent's tasks.
const sessionsBySlug: Record<string, SessionSummary> = {
  'ultrafast-bench': task('ultrafast-bench', 'ultrafast-bench'),
  claude: task('claude', 'Claude'),
  clitest: task('clitest', 'clitest'),
}

mock.module('@/modules/api/client', () => ({
  ...api,
  apiClient: async () => ({
    listAuditAgents: async () => agents,
    listSessions: async ({
      slug,
    }: {
      slug?: string
    }): Promise<SessionList> => ({
      items: slug
        ? [sessionsBySlug[slug]].filter((item) => item !== undefined)
        : Object.values(sessionsBySlug),
    }),
  }),
}))

const { useAuditScreenData } = await import('./audit.data')

const globalDescriptors = new Map(
  ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event'].map(
    (name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)],
  ),
)

function Probe() {
  const data = useAuditScreenData()
  return (
    <ul>
      {data.agentOptions.map((agent) => (
        <li key={agent.slug}>{agent.name}</li>
      ))}
    </ul>
  )
}

let root: Root
let container: HTMLElement

beforeEach(async () => {
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
})

async function renderHistory(url: string): Promise<string[]> {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <MemoryRouter initialEntries={[url]}>
          <Probe />
        </MemoryRouter>
      </QueryClientProvider>,
    ),
  )
  // Let both queries resolve and render.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
  return [...container.querySelectorAll('li')].map((li) => li.textContent ?? '')
}

describe('History agent filter', () => {
  it('lists every agent in History, not only those on the loaded page', async () => {
    expect(await renderHistory('/audit')).toEqual([
      'ultrafast-bench',
      'Claude',
      'clitest',
    ])
  })

  it('keeps the other agents listed after picking one', async () => {
    expect(await renderHistory('/audit?agent=claude')).toEqual([
      'ultrafast-bench',
      'Claude',
      'clitest',
    ])
  })
})
