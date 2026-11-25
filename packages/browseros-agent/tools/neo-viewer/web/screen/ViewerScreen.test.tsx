/**
 * The whole screen against a fake viewer, for what only the screen decides: the Health view
 * opens from the top bar's health line and closes with "← Runs" or Escape, giving focus back to
 * the line (its button is the one thing left of what the user was in), and a Health view closed
 * because health went away (a viewer restarted without telemetry) does not come back by itself.
 */

import { afterEach, describe, expect, it } from 'bun:test'
import {
  apiRoutes,
  type RunsResponse,
  type TelemetryHealth,
  type ViewerInfo,
} from '../../shared/api'
import { liveRoutes } from '../../shared/live-screen'
import { fire, installDom, type TestDom } from '../test-support/dom'
import { FakeOrigin } from '../test-support/tabs'
import { health, metricsView } from '../test-support/telemetry'
import { ViewerScreen } from './ViewerScreen'

const INFO: ViewerInfo = {
  mcpUrl: 'http://127.0.0.1:7392/mcp',
  upstreamUrl: 'http://127.0.0.1:39010/mcp',
  dataDir: '/tmp/viewer',
  startedAt: '2026-11-20T10:00:00.000Z',
  container: null,
  otlp: {
    endpoint: 'http://127.0.0.1:7392/otlp',
    containerEndpoint: 'http://host.docker.internal:7392/otlp',
  },
}

describe('the screen with Neo health', () => {
  let dom: TestDom
  let uninstall = () => {}
  afterEach(async () => {
    uninstall()
    await dom.restore()
    for (const name of [
      'ResizeObserver',
      'getComputedStyle',
      'MutationObserver',
    ])
      delete (globalThis as Record<string, unknown>)[name]
  })

  /** The screen on a fake viewer whose run snapshots carry `snapshots` in turn (then the last). */
  async function screen(
    snapshots: Array<TelemetryHealth | null>,
    withAgent = false,
  ) {
    dom = await installDom()
    const origin = new FakeOrigin()
    uninstall = origin.tab().install()
    // linkedom has no layout: the sidebar's size watcher has nothing to watch or measure.
    Object.assign(globalThis, {
      ResizeObserver: class {
        observe() {}
        unobserve() {}
        disconnect() {}
      },
      getComputedStyle: () => ({ paddingTop: '0', paddingBottom: '0' }),
      // noVNC, which the live screen loads, makes one when its module loads.
      MutationObserver: class {
        observe() {}
        disconnect() {}
        takeRecords() {
          return []
        }
      },
    })
    Object.assign(window, { innerWidth: 1440 })
    globalThis.fetch = (async (input: string | URL) => {
      const path = String(input)
      if (withAgent && path === liveRoutes.status)
        return Response.json({
          host: { state: 'up' },
          agent: {
            tab: { tabId: 12, title: 'Test', url: 'about:blank' },
            taskId: null,
          },
          takeover: null,
        })
      if (withAgent && path === liveRoutes.takeovers)
        return Response.json({
          takeover: {
            id: 'tk1',
            tabId: 12,
            state: 'active',
            display: '/v1/display?mode=control&release=tk1',
            createdAt: 1,
            expiresAt: Date.now() + 60000,
          },
        })
      if (path === apiRoutes.info) return Response.json(INFO)
      if (path === apiRoutes.metrics)
        return Response.json({ metrics: metricsView() })
      if (path === apiRoutes.runs) {
        const answer: Omit<RunsResponse, 'log'> & { log: null } = {
          log: null,
          runs: [],
          version: 0,
          health:
            (snapshots.length > 1 ? snapshots.shift() : snapshots[0]) ?? null,
        }
        return Response.json(answer)
      }
      return Response.json({ error: 'not here' }, { status: 404 })
    }) as typeof fetch
    await dom.render(<ViewerScreen />)
    const [stream] = origin.openStreams()
    await dom.interact(() => stream?.open())
    return stream
  }

  const healthLine = () =>
    [...dom.container.querySelectorAll('button[aria-pressed]')].find((button) =>
      button.textContent?.startsWith('In flight'),
    ) as HTMLElement | undefined
  const healthView = () =>
    dom.container.querySelector('section[aria-label="Neo health"]')
  const button = (text: string) =>
    [...dom.container.querySelectorAll('button')].find(
      (candidate) => candidate.textContent === text,
    ) as Element

  it('opens Runs first and returns to the same run table after viewing metrics', async () => {
    await screen([health()])
    const nav = dom.container.querySelector('nav[aria-label="Viewer sections"]')
    const current = () =>
      nav?.querySelector('[aria-current="page"]')?.textContent
    const table = dom.container.querySelector('[aria-label="Sort by Neo time"]')
    expect(current()).toBe('Runs')
    expect(table).not.toBeNull()
    await dom.interact(() => fire(button('Metrics'), 'click'))
    expect(current()).toBe('Metrics')
    await dom.interact(() => fire(button('Runs'), 'click'))
    expect(current()).toBe('Runs')
    expect(dom.container.querySelector('[aria-label="Sort by Neo time"]')).toBe(
      table,
    )
  })

  it('shows the control screen when taking over from Metrics and keeps it across sections', async () => {
    await screen([health()], true)
    await dom.interact(() => fire(button('Metrics'), 'click'))
    const display = () =>
      dom.container.querySelector('section[aria-label="Live screen"]')
    expect(display()).toBeNull()
    await dom.interact(() => fire(button('Take over'), 'click'))
    const held = display()
    expect(held).not.toBeNull()
    expect(button('Give back')).toBeDefined()
    await dom.interact(() => fire(button('Runs'), 'click'))
    expect(display()).toBe(held)
  })

  it('closes with ← Runs or Escape and gives focus back to the health line', async () => {
    await screen([health()])
    const line = healthLine()
    if (!line) throw new Error('no health line')
    let focused = 0
    line.focus = () => {
      focused++
    }
    await dom.interact(() => fire(line, 'click'))
    expect(healthView()).not.toBeNull()
    expect(line.getAttribute('aria-pressed')).toBe('true')

    await dom.interact(() => fire(button('← Runs'), 'click'))
    expect(healthView()).toBeNull()
    expect(focused).toBe(1)

    await dom.interact(() => fire(line, 'click'))
    expect(healthView()).not.toBeNull()
    await dom.interact(() => {
      const key = new window.Event('keydown')
      Object.assign(key, { key: 'Escape' })
      window.dispatchEvent(key)
    })
    expect(healthView()).toBeNull()
    expect(focused).toBe(2)
  })

  it('does not reopen by itself when health comes back after a viewer restarted without it', async () => {
    const stream = await screen([health({ rev: 5 }), null])
    const line = healthLine()
    if (!line) throw new Error('no health line')
    await dom.interact(() => fire(line, 'click'))
    expect(healthView()).not.toBeNull()
    // The viewer restarts with no telemetry: the reopened stream's snapshot has no health.
    await dom.interact(() => {
      stream?.drop()
      stream?.open()
    })
    expect(healthLine()).toBeUndefined()
    expect(healthView()).toBeNull()
    // Neo's telemetry starts again: the line is back, the view stays closed until clicked.
    await dom.interact(() =>
      stream?.send({ type: 'health', health: health({ rev: 1 }) }),
    )
    expect(healthLine()).toBeDefined()
    expect(healthView()).toBeNull()
  })
})
