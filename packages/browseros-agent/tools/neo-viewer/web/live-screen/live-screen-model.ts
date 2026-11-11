/**
 * What the live-screen panel is doing, as plain data and one reducer, so every state the panel
 * can show is a value a test can build. `use-live-screen.ts` feeds it events (the status poll,
 * the owner's clicks, the display socket opening and closing); `LiveScreen.tsx` renders it.
 *
 * Three ways to look at Neo's screen, one display at a time:
 *   follow   view display with `followTask`: watch, and Neo keeps the agent's tab in front
 *   control  control display: the owner clicks and types; the front tab stops following,
 *            the agent is not blocked
 *   taken    control display bound to a takeover (`release=`): the agent's calls on that tab
 *            fail with `owner_has_control` until the owner gives it back
 */

import {
  type DisplayRequest,
  displayClose,
  type LiveStatus,
  type LiveTakeover,
  type ScreenSize,
} from '../../shared/live-screen'

export type ScreenMode = 'follow' | 'control'

export type Connection =
  | { kind: 'connecting' }
  | { kind: 'live' }
  | {
      kind: 'down'
      /** `no-host`: no container (or its host API is off); `refused`: Neo would not open the display; `lost`: it dropped. */
      why: 'no-host' | 'refused' | 'lost'
      message: string
      /** When the panel tries again. */
      retryMs: number
    }

export interface LiveScreenState {
  mode: ScreenMode
  /** The takeover the owner holds; while set, the display is its control display. */
  takeover: LiveTakeover | null
  status: LiveStatus | null
  connection: Connection
  /** Bumped to open the display again: a retry, or a different display. */
  attempt: number
  /** Failed attempts in a row, for the backoff. */
  failures: number
  /** Take over or Give back is in flight. */
  busy: boolean
  /** Why the owner's last Take over or Give back failed. */
  actionError: string | null
  /** Neo's screen size, from the last display that opened; a takeover fits its tab to it. */
  screen: ScreenSize | null
}

export type LiveScreenAction =
  | { type: 'status'; status: LiveStatus }
  | { type: 'mode'; mode: ScreenMode }
  | { type: 'busy' }
  | { type: 'taken'; takeover: LiveTakeover }
  | { type: 'given-back' }
  | { type: 'action-failed'; message: string }
  | { type: 'connected'; screen: ScreenSize | null }
  | { type: 'closed'; code: number; reason: string }
  | { type: 'retry' }

export const initialLiveScreen: LiveScreenState = {
  mode: 'follow',
  takeover: null,
  status: null,
  connection: { kind: 'connecting' },
  attempt: 0,
  failures: 0,
  busy: false,
  actionError: null,
  screen: null,
}

/** 0.5 s, 1 s, 2 s, 4 s, then every 5 s: quick after a blip, calm while a container is down. */
export function retryDelayMs(failures: number): number {
  return Math.min(500 * 2 ** Math.max(0, failures - 1), 5_000)
}

/** Open a fresh display: a new attempt, counted from zero failures. */
function reopen(state: LiveScreenState): LiveScreenState {
  return {
    ...state,
    connection: { kind: 'connecting' },
    attempt: state.attempt + 1,
    failures: 0,
  }
}

function giveUpTakeover(state: LiveScreenState): LiveScreenState {
  return reopen({ ...state, takeover: null, mode: 'follow', busy: false })
}

export function liveScreenReducer(
  state: LiveScreenState,
  action: LiveScreenAction,
): LiveScreenState {
  switch (action.type) {
    case 'status': {
      const next = { ...state, status: action.status }
      // A reloaded page learns from the viewer that it still holds a takeover.
      if (action.status.takeover && !state.takeover && !state.busy)
        return reopen({
          ...next,
          takeover: action.status.takeover,
          mode: 'control',
        })
      return next
    }
    case 'mode':
      if (state.takeover || state.mode === action.mode) return state
      return reopen({ ...state, mode: action.mode })
    case 'busy':
      return { ...state, busy: true, actionError: null }
    case 'taken':
      return reopen({
        ...state,
        takeover: action.takeover,
        mode: 'control',
        busy: false,
      })
    case 'given-back':
      // Neo closes the takeover's display (4002) before the release answers, so the takeover
      // may already be gone.
      return state.takeover ? giveUpTakeover(state) : { ...state, busy: false }
    case 'action-failed':
      return { ...state, busy: false, actionError: action.message }
    case 'connected':
      return {
        ...state,
        connection: { kind: 'live' },
        failures: 0,
        screen: action.screen ?? state.screen,
      }
    case 'closed':
      return closed(state, action.code, action.reason)
    case 'retry':
      return {
        ...state,
        connection: { kind: 'connecting' },
        attempt: state.attempt + 1,
      }
  }
}

function closed(
  state: LiveScreenState,
  code: number,
  reason: string,
): LiveScreenState {
  if (code === displayClose.released) return giveUpTakeover(state)
  // The takeover ended while no display was bound to it (the viewer restarted, or it expired).
  if (
    state.takeover &&
    code === displayClose.displayRefused &&
    /^(not_found|takeover_released):/.test(reason)
  )
    return giveUpTakeover(state)
  const failures = state.failures + 1
  const why =
    code === displayClose.hostUnreachable
      ? 'no-host'
      : code === displayClose.displayRefused || code === 1008 || code === 1003
        ? 'refused'
        : 'lost'
  return {
    ...state,
    failures,
    connection: {
      kind: 'down',
      why,
      message: reason || `the display closed (${code})`,
      retryMs: retryDelayMs(failures),
    },
  }
}

/** The display the panel should have open now. */
export function displayRequestFor(
  state: LiveScreenState,
  task: string | null,
): DisplayRequest {
  if (state.takeover) return { mode: 'control', release: state.takeover.id }
  if (state.mode === 'control') return { mode: 'control' }
  return task ? { mode: 'view', followTask: task } : { mode: 'view' }
}

export interface TakeOverAvailability {
  label: 'Take over' | 'Give back'
  enabled: boolean
  /** Why it is disabled, or what it will do. */
  hint: string
}

export function takeOverAvailability(
  state: LiveScreenState,
  task: string | null,
): TakeOverAvailability {
  if (state.takeover)
    return {
      label: 'Give back',
      enabled: !state.busy,
      hint: 'Give the tab back to the agent.',
    }
  const disabled = (hint: string) =>
    ({ label: 'Take over', enabled: false, hint }) as const
  if (!task) return disabled('Pick a task to take over its tab.')
  if (state.status && state.status.host.state !== 'up')
    return disabled('Neo is not reachable.')
  if (!state.status?.tab) return disabled('This task has no open tab yet.')
  return {
    label: 'Take over',
    enabled: !state.busy,
    hint: 'Take the agent’s tab: you click and type, the agent is held.',
  }
}

/** The overlay over the screen while it is not live: what is wrong, why, and what happens next. */
export interface ScreenNotice {
  headline: string
  detail: string
  next: string
}

export function screenNotice(state: LiveScreenState): ScreenNotice | null {
  const connection = state.connection
  if (state.status?.host.state === 'unreachable' && connection.kind !== 'live')
    return {
      headline: 'No container',
      detail: state.status.host.message,
      next: 'Start the Neo SDK image; the screen reconnects on its own.',
    }
  if (connection.kind === 'live') return null
  if (connection.kind === 'connecting')
    return {
      headline: 'Connecting…',
      detail: 'Opening Neo’s screen.',
      next: '',
    }
  const next = `Trying again in ${Math.ceil(connection.retryMs / 1000)} s.`
  const headline = {
    'no-host': 'No container',
    refused: 'No display',
    lost: 'Screen lost',
  }[connection.why]
  return { headline, detail: connection.message, next }
}

/** One line under the screen: what the owner's clicks do now, and whether the agent is held. */
export function modeLine(state: LiveScreenState, task: string | null): string {
  if (state.takeover)
    return 'You have the agent’s tab. Its calls on this tab fail with owner_has_control until you give it back.'
  if (state.mode === 'control')
    return 'Control: your clicks and keys reach the browser. The agent is not held; the front tab stops following it.'
  if (!task) return 'Watching the screen. Pick a task to follow its tab.'
  return 'Following the agent’s tab. Control lets you click and type; Take over also holds the agent on this tab.'
}
