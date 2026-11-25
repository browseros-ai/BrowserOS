/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import {
  afterAll,
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  mock,
} from 'bun:test'
import type { Plan, TelemetryState } from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'
import { MemoryRouter } from 'react-router'

const api = await import('@/modules/api/client')
const posthog = await import('./posthog')

const captured: { event: string; properties: Record<string, unknown> }[] = []
mock.module('./posthog', () => ({
  ...posthog,
  applyTelemetry: () => {},
  isCapturing: () => true,
  // The real sanitizer adds the plan the way a sent event carries it.
  capture: (event: string, properties?: Record<string, unknown>) => {
    captured.push({
      event,
      properties: posthog.sanitizeProperties(properties ?? {}),
    })
  },
}))

let resolvePlan: (plan: Plan) => void = () => {}

mock.module('@/modules/api/client', () => ({
  ...api,
  apiClient: async () => ({
    getTelemetry: async (): Promise<TelemetryState> => ({
      distinctId: 'install-1',
      enabled: true,
      consent: true,
    }),
    getPlan: () =>
      new Promise<Plan>((resolve) => {
        resolvePlan = resolve
      }),
  }),
}))

const { Analytics } = await import('./Analytics')

afterAll(() => mock.restore())

const ULTRAFAST: Plan = {
  available: true,
  plan: 'ultrafast',
  planName: 'ultrafast',
  features: ['claw.ultrafast'],
  status: 'active',
  periodEnd: null,
  endsAt: null,
  graceUntil: null,
  onKey: true,
  checkedAt: null,
  stale: false,
}

const globalNames = ['window', 'document', 'navigator', 'HTMLElement'] as const
const globalDescriptors = new Map(
  globalNames.map((name) => [
    name,
    Object.getOwnPropertyDescriptor(globalThis, name),
  ]),
)

let root: Root

function define(name: string, value: unknown) {
  Object.defineProperty(globalThis, name, {
    configurable: true,
    writable: true,
    value,
  })
}

async function settle() {
  for (let tick = 0; tick < 5; tick += 1) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
  }
}

beforeEach(async () => {
  captured.length = 0
  posthog.setPlanContext(null)
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
  define('window', dom.window)
  define('document', dom.document)
  define('navigator', { userAgent: 'test' })
  define('HTMLElement', dom.window.HTMLElement)
  define('IS_REACT_ACT_ENVIRONMENT', true)
  const { createRoot } = await import('react-dom/client')
  root = createRoot(
    dom.document.getElementById('root') as unknown as HTMLElement,
  )
})

afterEach(async () => {
  await act(async () => root.unmount())
  posthog.setPlanContext(null)
  for (const [name, descriptor] of globalDescriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
  Reflect.deleteProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
})

describe('the first events of an app load', () => {
  it('tests that a cold open sends app_opened and the first view once the plan is known, with that plan', async () => {
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    })
    await act(async () => {
      root.render(
        <QueryClientProvider client={client}>
          <MemoryRouter initialEntries={['/pro']}>
            <Analytics />
          </MemoryRouter>
        </QueryClientProvider>,
      )
    })
    await settle()
    expect(captured).toEqual([])

    await act(async () => resolvePlan(ULTRAFAST))
    await settle()

    expect(captured).toEqual([
      { event: 'app_opened', properties: { plan: 'ultrafast' } },
      { event: 'pro_page_viewed', properties: { plan: 'ultrafast' } },
    ])
  })
})
