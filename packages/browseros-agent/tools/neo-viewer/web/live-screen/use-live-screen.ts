/**
 * The live screen's controller: one per page, shared by every view of it (the docked panel, the
 * pop-up, a Take over button in a top bar), so they agree on the mode and the takeover. It polls
 * the viewer for the task's tab and the host's state, reopens the display after a failure, and
 * calls the viewer's takeover routes. `live-screen-model.ts` holds the rules.
 *
 * The poll shares the browser's few connections to the viewer with everything else the page and
 * the viewer's other tabs load, so a status can be slow to come. One read is in flight at a time,
 * and the panel's clock ticks once the follow target has gone `FOLLOW_STALE_MS` unconfirmed, so
 * the line under the screen says so instead of claiming to follow.
 */

import { useCallback, useEffect, useMemo, useReducer } from 'react'
import {
  type DisplayRequest,
  type LiveError,
  type LiveStatus,
  liveRoutes,
  type ScreenSize,
  type TakeoverRequest,
  type TakeoverResponse,
} from '../../shared/live-screen'
import {
  displayRequestFor,
  FOLLOW_STALE_MS,
  initialLiveScreen,
  type LiveScreenState,
  liveScreenReducer,
  type ScreenMode,
} from './live-screen-model'

/**
 * How often the panel asks where the agent is and whether Neo answers. Following by tab, a move
 * to another tab shows on the screen after the next poll.
 */
const STATUS_POLL_MS = 1_500

export interface LiveScreenController {
  state: LiveScreenState
  /** The display to have open now; `state.attempt` changes when it must be opened again. */
  display: DisplayRequest
  setMode(mode: ScreenMode): void
  takeOver(): void
  giveBack(): void
  /** From the screen: the display is showing Neo's screen of this size. */
  connected(screen: ScreenSize | null): void
  /** From the screen: the display's socket closed by itself, with this code and reason. */
  closed(code: number, reason: string): void
}

export function useLiveScreen(): LiveScreenController {
  const [state, dispatch] = useReducer(liveScreenReducer, initialLiveScreen)

  useEffect(
    () =>
      startStatusPoll(
        readStatus,
        (status) => dispatch({ type: 'status', status, at: Date.now() }),
        STATUS_POLL_MS,
      ),
    [],
  )

  const { confirmedAt } = state
  useEffect(() => {
    if (confirmedAt === null) return
    // Every status that reaches Neo moves `confirmedAt` and so reschedules this: it fires only
    // when none has come for the whole bound.
    const timer = setTimeout(
      () => dispatch({ type: 'tick', at: Date.now() }),
      confirmedAt + FOLLOW_STALE_MS + 1 - Date.now(),
    )
    return () => clearTimeout(timer)
  }, [confirmedAt])

  const { connection } = state
  useEffect(() => {
    if (connection.kind !== 'down') return
    const timer = setTimeout(
      () => dispatch({ type: 'retry' }),
      connection.retryMs,
    )
    return () => clearTimeout(timer)
  }, [connection])

  const takeoverId = state.takeover?.id ?? null
  const { screen } = state
  // The tab the panel shows as the agent's is the one taken, so the owner gets what they see.
  const agentTab = state.status?.agent?.tab.tabId ?? null
  const takeOver = useCallback(() => {
    if (agentTab === null) return
    dispatch({ type: 'busy' })
    // Neo's own screen size: its fit brings the tab to the front and leaves the screen as is.
    const request: TakeoverRequest = screen
      ? { tabId: agentTab, viewer: screen }
      : { tabId: agentTab }
    void post<TakeoverResponse>(liveRoutes.takeovers, request).then((answer) =>
      dispatch(
        'error' in answer
          ? { type: 'action-failed', message: answer.error.message }
          : { type: 'taken', takeover: answer.takeover },
      ),
    )
  }, [agentTab, screen])

  const giveBack = useCallback(() => {
    if (!takeoverId) return
    dispatch({ type: 'busy' })
    void post<TakeoverResponse>(liveRoutes.release(takeoverId), {}).then(
      (answer) => {
        // A takeover Neo already ended (its TTL, its tab closed) is given back all the same.
        const gone =
          'error' in answer &&
          ['not_found', 'expired', 'takeover_released'].includes(
            answer.error.code,
          )
        dispatch(
          'error' in answer && !gone
            ? { type: 'action-failed', message: answer.error.message }
            : { type: 'given-back' },
        )
      },
    )
  }, [takeoverId])

  const setMode = useCallback(
    (mode: ScreenMode) => dispatch({ type: 'mode', mode }),
    [],
  )
  const connected = useCallback(
    (screen: ScreenSize | null) => dispatch({ type: 'connected', screen }),
    [],
  )
  const closed = useCallback(
    (code: number, reason: string) =>
      dispatch({ type: 'closed', code, reason }),
    [],
  )
  const display = useMemo(() => displayRequestFor(state), [state])

  return {
    state,
    display,
    setMode,
    takeOver,
    giveBack,
    connected,
    closed,
  }
}

/**
 * Reads the status now and every `everyMs`, one read at a time: a tick while a read is pending
 * starts nothing, so a read stuck waiting for a connection does not queue more behind it. The
 * returned function stops; a read still pending then delivers nothing.
 */
export function startStatusPoll(
  read: () => Promise<LiveStatus>,
  deliver: (status: LiveStatus) => void,
  everyMs: number,
): () => void {
  let current = true
  let pending = false
  const tick = () => {
    if (pending) return
    pending = true
    void read()
      .then((status) => {
        if (current) deliver(status)
      })
      .finally(() => {
        pending = false
      })
  }
  tick()
  const timer = setInterval(tick, everyMs)
  return () => {
    current = false
    clearInterval(timer)
  }
}

/** The viewer's answer; a viewer that does not answer is reported as an unreachable host. */
async function readStatus(): Promise<LiveStatus> {
  try {
    const response = await fetch(liveRoutes.status)
    if (!response.ok) throw new Error(`HTTP ${response.status}`)
    return (await response.json()) as LiveStatus
  } catch (cause) {
    return {
      host: {
        state: 'unreachable',
        message: `The viewer did not answer (${String(cause)})`,
      },
      agent: null,
      takeover: null,
    }
  }
}

async function post<T>(path: string, body: unknown): Promise<T | LiveError> {
  try {
    const response = await fetch(path, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
    })
    const answer = (await response.json().catch(() => null)) as
      | T
      | LiveError
      | null
    if (answer && (response.ok || 'error' in (answer as object))) return answer
    return { error: { code: 'http_error', message: `HTTP ${response.status}` } }
  } catch (cause) {
    return { error: { code: 'unreachable', message: String(cause) } }
  }
}
