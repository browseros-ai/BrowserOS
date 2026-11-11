/**
 * The panel's states as the owner sees them, rendered to HTML without a browser: effects do
 * not run, so no socket opens and each state is exactly the controller passed in.
 */

import { expect, test } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { displayClose, type LiveTakeover } from '../../shared/live-screen'
import { LiveScreen, LiveScreenPopup, TakeOverButton } from './LiveScreen'
import {
  displayRequestFor,
  initialLiveScreen,
  type LiveScreenAction,
  liveScreenReducer,
} from './live-screen-model'
import type { LiveScreenController } from './use-live-screen'

const TAKEOVER: LiveTakeover = {
  id: 'tk_1',
  tabId: 12,
  state: 'active',
  display: '/v1/display?mode=control&release=tk_1',
  createdAt: 1,
  expiresAt: 2,
}
const STATUS_UP: LiveScreenAction = {
  type: 'status',
  status: {
    host: { state: 'up' },
    agent: {
      tab: { tabId: 12, url: 'https://two.test/', title: 'Statements' },
      taskId: null,
    },
    takeover: null,
  },
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

/** The page's visible text, one string, so tests read like what the owner reads. */
function text(markup: string): string {
  return markup
    .replace(/<[^>]+>/g, ' ')
    .replace(/&#x27;/g, "'")
    .replace(/\s+/g, ' ')
    .trim()
}

test('docked, following: the screen follows the agent’s tab', () => {
  const live = controller(STATUS_UP, { type: 'connected', screen: null })
  const html = renderToStaticMarkup(
    <LiveScreen live={live} size="docked" onPopOut={() => {}} />,
  )
  expect(html).toContain('data-display="mode=view&amp;followTab=12"')
  expect(html).toMatch(/aria-pressed="true"[^>]*>Follow agent</)
  expect(text(html)).toContain('Pop out')
  expect(text(html)).toContain('Following the agent’s tab.')
  expect(html).not.toContain('role="status"')
})

test('docked, before it is measured: the box takes Neo’s screen ratio, so it does not jump', () => {
  const unknown = renderToStaticMarkup(
    <LiveScreen live={controller(STATUS_UP)} size="docked" />,
  )
  expect(unknown).toContain('data-fit="ratio"')
  expect(unknown).toContain('aspect-ratio:1.6')
  // Once the server has reported its framebuffer, its own ratio.
  const reported = renderToStaticMarkup(
    <LiveScreen
      live={controller(STATUS_UP, {
        type: 'connected',
        screen: { width: 1280, height: 1024 },
      })}
      size="docked"
    />,
  )
  expect(reported).toContain('aspect-ratio:1.25')
  expect(reported).not.toContain('h-[200px]')
})

test('docked while popped out: no screen, a note where it went', () => {
  const live = controller(STATUS_UP)
  const html = renderToStaticMarkup(
    <LiveScreen live={live} size="docked" popped onPopOut={() => {}} />,
  )
  expect(html).not.toContain('data-display')
  expect(text(html)).toContain(
    'Open in a pop-up. Close it to dock it here again.',
  )
})

test('no container: says so and that it reconnects', () => {
  const live = controller({
    type: 'closed',
    code: displayClose.hostUnreachable,
    reason: "Neo's host API is not reachable: Unable to connect",
  })
  const shown = text(
    renderToStaticMarkup(<LiveScreen live={live} size="docked" />),
  )
  expect(shown).toContain('No container')
  expect(shown).toContain("Neo's host API is not reachable: Unable to connect")
  expect(shown).toContain('Trying again in 1 s.')
})

test('no display: says Neo did not open its screen and why', () => {
  const live = controller(STATUS_UP, {
    type: 'closed',
    code: displayClose.displayRefused,
    reason: 'display_unavailable: Neo’s screen did not open',
  })
  const shown = text(
    renderToStaticMarkup(<LiveScreen live={live} size="large" />),
  )
  expect(shown).toContain('No display')
  expect(shown).toContain('display_unavailable')
})

test('control: input reaches the browser, the agent is not held', () => {
  const live = controller(STATUS_UP, { type: 'mode', mode: 'control' })
  const html = renderToStaticMarkup(<LiveScreen live={live} size="large" />)
  expect(html).toContain('data-display="mode=control"')
  expect(html).toMatch(/aria-pressed="true"[^>]*>Control</)
  expect(text(html)).toContain('The agent is not held')
  expect(text(html)).toContain('Statements tab 12')
})

test('taken over: control display bound to the takeover, Give back, the agent held', () => {
  const live = controller(STATUS_UP, {
    type: 'taken',
    takeover: TAKEOVER,
  })
  const html = renderToStaticMarkup(
    <LiveScreenPopup live={live} onClose={() => {}} />,
  )
  expect(html).toContain('data-display="mode=control&amp;release=tk_1"')
  expect(text(html)).toContain('Give back')
  expect(text(html)).toContain(
    'fail with owner_has_control until you give it back',
  )
  // Follow agent is locked while the owner holds the tab.
  expect(html).toMatch(
    /disabled="" title="You have taken over[^"]*"[^>]*>Follow agent/,
  )
  expect(text(html)).toContain('Close to dock it back in the sidebar')
})

test('Take over waits for the agent to have an open tab', () => {
  const none = renderToStaticMarkup(<TakeOverButton live={controller()} />)
  expect(none).toContain('disabled=""')
  expect(none).toContain('The agent has no open tab yet')
  const ready = renderToStaticMarkup(
    <TakeOverButton live={controller(STATUS_UP)} />,
  )
  expect(ready).not.toContain('disabled=""')
  expect(text(ready)).toBe('Take over')
  const busy = renderToStaticMarkup(
    <TakeOverButton live={controller(STATUS_UP, { type: 'busy' })} />,
  )
  expect(text(busy)).toBe('Taking over…')
})
