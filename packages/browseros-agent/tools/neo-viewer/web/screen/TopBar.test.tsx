/**
 * The top bar rendered to HTML (as `web/live-screen/LiveScreen.test.tsx` does): the read-only
 * container line, and Take over on the live screen's controller.
 */

import { describe, expect, it } from 'bun:test'
import { createRef } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import type { TelemetryHealth, ViewerInfo } from '../../shared/api'
import type { LiveTakeover } from '../../shared/live-screen'
import {
  displayRequestFor,
  initialLiveScreen,
  type LiveScreenAction,
  liveScreenReducer,
} from '../live-screen/live-screen-model'
import type { LiveScreenController } from '../live-screen/use-live-screen'
import { installDom } from '../test-support/dom'
import { health } from '../test-support/telemetry'
import { errorRate, HealthLine } from './HealthLine'
import { ExportNotice, TopBar } from './TopBar'

const INFO: ViewerInfo = {
  mcpUrl: 'http://127.0.0.1:7310/mcp',
  upstreamUrl: 'http://127.0.0.1:39010/mcp',
  dataDir: '/tmp/viewer',
  startedAt: '2026-11-11T10:00:00.000Z',
  container: null,
  otlp: {
    endpoint: 'http://127.0.0.1:7310/otlp',
    containerEndpoint: 'http://host.docker.internal:7310/otlp',
  },
}

const AGENT_ON_A_TAB: LiveScreenAction = {
  type: 'status',
  at: 0,
  status: {
    host: { state: 'up' },
    agent: {
      tab: {
        tabId: 12,
        url: 'http://127.0.0.1:8800/files.html',
        title: 'Files',
      },
      taskId: null,
    },
    takeover: null,
  },
}

const TAKEOVER: LiveTakeover = {
  id: 'tk_1',
  tabId: 12,
  state: 'active',
  display: '/v1/display?mode=control&release=tk_1',
  createdAt: 1,
  expiresAt: 2,
}

function controller(...actions: LiveScreenAction[]): LiveScreenController {
  const state = actions.reduce(liveScreenReducer, initialLiveScreen)
  const noop = () => {}
  return {
    state,
    display: displayRequestFor(state),
    setMode: noop,
    takeOver: noop,
    giveBack: noop,
    connected: noop,
    closed: noop,
  }
}

function text(markup: string): string {
  return markup
    .replace(/<[^>]+>/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
}

const bar = (
  info: ViewerInfo,
  live = controller(AGENT_ON_A_TAB),
  exporting = false,
) =>
  renderToStaticMarkup(
    <TopBar
      info={info}
      onClear={() => {}}
      onExport={() => {}}
      exporting={exporting}
      live={live}
    />,
  )

describe('the top bar', () => {
  it('shows the container it started, read-only: up or not, and its image tag', () => {
    const running = text(
      bar({
        ...INFO,
        container: {
          name: 'neo-viewer-sdk',
          image: 'neo-sdk:0.52.1.2-55e78dd52-arm64',
          up: true,
        },
      }),
    )
    expect(running).toContain('Running neo-sdk:0.52.1.2-55e78dd52-arm64')
    const stopped = text(
      bar({
        ...INFO,
        container: { name: 'neo-viewer-sdk', image: 'neo-sdk:x', up: false },
      }),
    )
    expect(stopped).toContain('Stopped neo-sdk:x')
    // No Start, Stop or Restart: the command that starts the viewer is the control.
    expect(stopped).not.toMatch(/Restart|Start\b|Stop\b/)
  })

  it('says the image is managed elsewhere when the viewer forwards to a Neo it did not start', () => {
    expect(text(bar(INFO))).toContain(
      'Image managed outside the viewer → http://127.0.0.1:39010/mcp',
    )
  })

  it("offers Take over for the agent's tab, and Give back while it is held", () => {
    const ready = bar(INFO)
    expect(ready).toMatch(
      /<button[^>]*title="Take the agent[^"]*"[^>]*>Take over<\/button>/,
    )
    expect(ready).not.toMatch(/<button[^>]*disabled=""[^>]*>Take over/)
    const held = bar(
      INFO,
      controller(AGENT_ON_A_TAB, { type: 'taken', takeover: TAKEOVER }),
    )
    expect(text(held)).toContain('Give back')
    // Before the viewer knows the agent's tab, there is nothing to take.
    expect(bar(INFO, controller())).toMatch(
      /<button[^>]*disabled=""[^>]*>Take over<\/button>/,
    )
  })

  it('offers Export beside Clear, and waits while an export is written', () => {
    expect(text(bar(INFO))).toMatch(/Export Clear/)
    expect(bar(INFO, controller(AGENT_ON_A_TAB), true)).toMatch(
      /<button[^>]*disabled=""[^>]*>Exporting…<\/button>/,
    )
  })

  it("shows the export's folder with Copy path until it is dismissed", () => {
    const notice = text(
      renderToStaticMarkup(
        <ExportNotice
          exported={{
            dir: '/tmp/neo-viewer/default/export-20261111-224501',
            files: ['README.md'],
            runs: 3,
          }}
          onDismiss={() => {}}
        />,
      ),
    )
    expect(notice).toBe(
      'Exported 3 runs to /tmp/neo-viewer/default/export-20261111-224501 Copy path ×',
    )
  })
})

describe("the top bar's health line", () => {
  const NOW = 1_000_000
  const line = (shown: TelemetryHealth, now = NOW, open = false) =>
    renderToStaticMarkup(
      <HealthLine health={shown} open={open} onToggle={() => {}} now={now} />,
    )

  it('is not there without telemetry: the bar is as it always was', () => {
    const markup = bar(INFO)
    expect(markup).not.toContain('In flight')
    expect(
      renderToStaticMarkup(
        <TopBar
          info={INFO}
          onClear={() => {}}
          onExport={() => {}}
          exporting={false}
          live={controller(AGENT_ON_A_TAB)}
          health={null}
          onHealth={() => {}}
        />,
      ),
    ).toBe(markup)
  })

  it('reads runs in flight, the error rates since the Clear, tabs and crashes, and that it is live', () => {
    expect(text(line(health()))).toBe(
      'In flight 1 Run errors 0% Op errors 13% Tabs 3 Crashes 0 live',
    )
    // Only what failed is in the error tone.
    const markup = line(
      health({
        sinceClear: { runs: 4, runErrors: 1, ops: 9, opErrors: 0, crashes: 2 },
      }),
    )
    expect(markup).toMatch(/text-bad">25%</)
    expect(markup).toMatch(/text-ink">0%</)
    expect(markup).toMatch(/text-bad">2</)
  })

  it('says how fresh it is: live, then how long ago, then that no metrics come', () => {
    expect(line(health(), NOW + 2_000)).toContain('data-freshness="live"')
    const late = line(health(), NOW + 7_000)
    expect(late).toContain('data-freshness="late"')
    expect(text(late)).toContain('7 s ago')
    // No source for 30 s: every figure is unknown, not zero.
    const quiet = text(
      line(
        health({
          source: null,
          observedAt: null,
          runsInFlight: null,
          tabsOpen: null,
          sinceClear: null,
        }),
      ),
    )
    expect(quiet).toBe(
      'In flight – Run errors – Op errors – Tabs – Crashes – no metrics',
    )
  })

  it('is named by its figures, so a screen reader reads them, with the sentence as its description', () => {
    const markup = line(health())
    expect(markup).not.toContain('aria-label')
    expect(markup).toMatch(
      /<button[^>]*title="Neo&#x27;s health\. Since the last Clear/,
    )
  })

  it('hands its button to the screen, which gives it focus back when the Health view closes', async () => {
    const dom = await installDom()
    const ref = createRef<HTMLButtonElement>()
    await dom.render(
      <HealthLine
        health={health()}
        open={false}
        onToggle={() => {}}
        now={NOW}
        buttonRef={ref}
      />,
    )
    expect(ref.current?.textContent).toContain('In flight')
    await dom.restore()
  })

  it('opens the Health view, and reads as pressed while it is open', () => {
    expect(line(health(), NOW, true)).toContain('aria-pressed="true"')
    expect(line(health())).toContain('aria-pressed="false"')
    const title = /title="([^"]*)"/
      .exec(line(health()))?.[1]
      ?.replaceAll('&#x27;', "'")
    expect(title).toBe(
      "Neo's health. Since the last Clear: 0 of 15 runs and 16 of 120 operations failed; no renderer crashes. Now: 1 run in flight, 3 tabs open. Scraped from Neo's GET /v1/metrics 0 s ago. Click for the Health view.",
    )
  })

  it('writes failure rates so they keep their slot', () => {
    expect(errorRate(undefined, undefined)).toEqual({ text: '–', bad: false })
    expect(errorRate(0, 0)).toEqual({ text: '–', bad: false })
    expect(errorRate(0, 12)).toEqual({ text: '0%', bad: false })
    expect(errorRate(1, 500)).toEqual({ text: '<1%', bad: true })
    expect(errorRate(4, 7)).toEqual({ text: '57%', bad: true })
    expect(errorRate(9, 9)).toEqual({ text: '100%', bad: true })
  })
})
