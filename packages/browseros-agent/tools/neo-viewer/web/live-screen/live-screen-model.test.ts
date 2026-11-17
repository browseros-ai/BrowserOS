import { expect, test } from 'bun:test'
import {
  displayClose,
  type LiveStatus,
  type LiveTakeover,
} from '../../shared/live-screen'
import {
  displayRequestFor,
  initialLiveScreen,
  type LiveScreenAction,
  type LiveScreenState,
  liveScreenReducer,
  retryDelayMs,
  screenNotice,
  takeOverAvailability,
} from './live-screen-model'

const TAKEOVER: LiveTakeover = {
  id: 'tk_1',
  tabId: 12,
  state: 'active',
  display: '/v1/display?mode=control&release=tk_1',
  createdAt: 1,
  expiresAt: 2,
}
const TWO = { tabId: 12, url: 'https://two.test/', title: 'Two' }
/** The agent named no task; its latest run last acted on tab 12. */
const UP: LiveStatus = {
  host: { state: 'up' },
  agent: { tab: TWO, taskId: null },
  takeover: null,
}

function run(...actions: LiveScreenAction[]): LiveScreenState {
  return actions.reduce(liveScreenReducer, initialLiveScreen)
}

test('follow keeps the agent’s tab in front: by tab, or by task when its run named one', () => {
  expect(displayRequestFor(initialLiveScreen)).toEqual({ mode: 'view' })
  const byTab = run({ type: 'status', status: UP })
  expect(displayRequestFor(byTab)).toEqual({
    mode: 'view',
    follow: { tabId: 12 },
  })
  const byTask = liveScreenReducer(byTab, {
    type: 'status',
    status: { ...UP, agent: { tab: TWO, taskId: 'host-7' } },
  })
  expect(displayRequestFor(byTask)).toEqual({
    mode: 'view',
    follow: { taskId: 'host-7' },
  })
  expect(displayRequestFor(run({ type: 'mode', mode: 'control' }))).toEqual({
    mode: 'control',
  })
  expect(displayRequestFor(run({ type: 'taken', takeover: TAKEOVER }))).toEqual(
    { mode: 'control', release: 'tk_1' },
  )
})

test('the follow target moves with the agent and survives a status from an unreachable Neo', () => {
  const onTwo = run({ type: 'status', status: UP })
  // The same target keeps its object: an unchanged status opens no new display.
  const again = liveScreenReducer(onTwo, { type: 'status', status: UP })
  expect(again.follow).toBe(onTwo.follow)
  const onOne = liveScreenReducer(onTwo, {
    type: 'status',
    status: { ...UP, agent: { tab: { ...TWO, tabId: 11 }, taskId: null } },
  })
  expect(onOne.follow).toEqual({ tabId: 11 })
  const down = liveScreenReducer(onOne, {
    type: 'status',
    status: {
      host: { state: 'unreachable', message: 'nothing answered' },
      agent: null,
      takeover: null,
    },
  })
  expect(down.follow).toBe(onOne.follow)
  // Neo answered: the agent's tab closed, so there is nothing to follow.
  expect(
    liveScreenReducer(down, { type: 'status', status: { ...UP, agent: null } })
      .follow,
  ).toBeNull()
})

test('each new display is a new attempt', () => {
  const control = run(
    { type: 'connected', screen: null },
    { type: 'mode', mode: 'control' },
  )
  expect(control.attempt).toBe(1)
  expect(control.connection).toEqual({ kind: 'connecting' })
  // Choosing the mode already shown opens nothing new.
  expect(liveScreenReducer(control, { type: 'mode', mode: 'control' })).toBe(
    control,
  )
})

test('while taken, the mode is locked to control until the takeover ends', () => {
  const taken = run({ type: 'busy' }, { type: 'taken', takeover: TAKEOVER })
  expect(taken).toMatchObject({ mode: 'control', busy: false, attempt: 1 })
  expect(liveScreenReducer(taken, { type: 'mode', mode: 'follow' })).toBe(taken)

  // Neo closes the takeover's display with 4002 when it is released, however it ended.
  const released = liveScreenReducer(taken, {
    type: 'closed',
    code: displayClose.released,
    reason: 'released',
  })
  expect(released).toMatchObject({
    takeover: null,
    mode: 'follow',
    attempt: 2,
    connection: { kind: 'connecting' },
  })
  // The release's answer after that changes nothing more.
  expect(liveScreenReducer(released, { type: 'given-back' })).toEqual({
    ...released,
    busy: false,
  })
})

test('a takeover Neo no longer knows is dropped instead of retried', () => {
  const taken = run({ type: 'taken', takeover: TAKEOVER })
  for (const reason of [
    'not_found: no takeover has this id',
    'takeover_released: the takeover was released',
  ]) {
    const next = liveScreenReducer(taken, {
      type: 'closed',
      code: displayClose.displayRefused,
      reason,
    })
    expect(next).toMatchObject({ takeover: null, mode: 'follow' })
  }
})

test('a reloaded page adopts the takeover the viewer still holds', () => {
  const adopted = run({ type: 'status', status: { ...UP, takeover: TAKEOVER } })
  expect(adopted).toMatchObject({ takeover: TAKEOVER, mode: 'control' })
})

test('a failed display backs off and says what is wrong', () => {
  const noHost = run({
    type: 'closed',
    code: displayClose.hostUnreachable,
    reason: "Neo's host API is not reachable: Unable to connect",
  })
  expect(noHost.connection).toEqual({
    kind: 'down',
    why: 'no-host',
    message: "Neo's host API is not reachable: Unable to connect",
    retryMs: 500,
  })
  expect(screenNotice(noHost)).toEqual({
    headline: 'No container',
    detail: "Neo's host API is not reachable: Unable to connect",
    next: 'Trying again in 1 s.',
  })

  const refused = run({
    type: 'closed',
    code: displayClose.displayRefused,
    reason: "display_unavailable: Neo's screen did not open",
  })
  expect(screenNotice(refused)?.headline).toBe('No display')

  const lost = run(
    { type: 'connected', screen: null },
    { type: 'closed', code: displayClose.displayLost, reason: '' },
  )
  expect(screenNotice(lost)).toMatchObject({
    headline: 'Screen lost',
    detail: `the display closed (${displayClose.displayLost})`,
  })

  // The retry opens a new attempt; connecting resets the backoff.
  const retried = liveScreenReducer(lost, { type: 'retry' })
  expect(retried.attempt).toBe(lost.attempt + 1)
  expect(retried.failures).toBe(1)
  expect(
    liveScreenReducer(retried, { type: 'connected', screen: null }).failures,
  ).toBe(0)
  expect([1, 2, 3, 4, 5, 9].map(retryDelayMs)).toEqual([
    500, 1000, 2000, 4000, 5000, 5000,
  ])
})

test('no container shows even before a display has failed', () => {
  const state = run({
    type: 'status',
    status: {
      host: { state: 'unreachable', message: 'Unable to connect' },
      agent: null,
      takeover: null,
    },
  })
  expect(screenNotice(state)?.headline).toBe('No container')
  expect(screenNotice(run({ type: 'connected', screen: null }))).toBeNull()
})

test('take over is offered once the agent has an open tab', () => {
  expect(takeOverAvailability(initialLiveScreen)).toMatchObject({
    label: 'Take over',
    enabled: false,
  })
  expect(
    takeOverAvailability(
      run({ type: 'status', status: { ...UP, agent: null } }),
    ).enabled,
  ).toBe(false)
  const ready = run({ type: 'status', status: UP })
  expect(takeOverAvailability(ready)).toMatchObject({
    label: 'Take over',
    enabled: true,
  })
  expect(
    takeOverAvailability(liveScreenReducer(ready, { type: 'busy' })).enabled,
  ).toBe(false)
  expect(
    takeOverAvailability(
      liveScreenReducer(ready, { type: 'taken', takeover: TAKEOVER }),
    ),
  ).toMatchObject({ label: 'Give back', enabled: true })
  const failed = liveScreenReducer(liveScreenReducer(ready, { type: 'busy' }), {
    type: 'action-failed',
    message: 'no open tab has this id',
  })
  expect(failed).toMatchObject({ busy: false, actionError: expect.any(String) })
})

test('the screen size from the last display is kept for the takeover’s fit', () => {
  const size = { width: 1600, height: 1000 }
  const state = run({ type: 'connected', screen: size })
  expect(state.screen).toEqual(size)
  // A display that could not read it keeps the size already known.
  expect(
    liveScreenReducer(state, { type: 'connected', screen: null }).screen,
  ).toEqual(size)
})
