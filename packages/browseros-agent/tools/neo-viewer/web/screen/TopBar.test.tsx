/**
 * The top bar rendered to HTML (as `web/live-screen/LiveScreen.test.tsx` does): the read-only
 * container line, and Take over on the live screen's controller.
 */

import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import type { ViewerInfo } from '../../shared/api'
import type { LiveTakeover } from '../../shared/live-screen'
import {
  displayRequestFor,
  initialLiveScreen,
  type LiveScreenAction,
  liveScreenReducer,
} from '../live-screen/live-screen-model'
import type { LiveScreenController } from '../live-screen/use-live-screen'
import { ExportNotice, TopBar } from './TopBar'

const INFO: ViewerInfo = {
  mcpUrl: 'http://127.0.0.1:7310/mcp',
  upstreamUrl: 'http://127.0.0.1:39010/mcp',
  dataDir: '/tmp/viewer',
  startedAt: '2026-11-11T10:00:00.000Z',
  container: null,
}

const AGENT_ON_A_TAB: LiveScreenAction = {
  type: 'status',
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
