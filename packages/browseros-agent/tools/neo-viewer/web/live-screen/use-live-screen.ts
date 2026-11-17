/**
 * The live screen's controller: one per page, shared by every view of it (the docked panel, the
 * pop-up, a Take over button in a top bar), so they agree on the mode and the takeover. It polls
 * the viewer for the task's tab and the host's state, reopens the display after a failure, and
 * calls the viewer's takeover routes. `live-screen-model.ts` holds the rules.
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

  useEffect(() => {
    let current = true
    const poll = async () => {
      try {
        const response = await fetch(liveRoutes.status)
        if (!response.ok) throw new Error(`HTTP ${response.status}`)
        const status = (await response.json()) as LiveStatus
        if (current) dispatch({ type: 'status', status })
      } catch (cause) {
        if (current)
          dispatch({
            type: 'status',
            status: {
              host: {
                state: 'unreachable',
                message: `The viewer did not answer (${String(cause)})`,
              },
              agent: null,
              takeover: null,
            },
          })
      }
    }
    void poll()
    const timer = setInterval(poll, STATUS_POLL_MS)
    return () => {
      current = false
      clearInterval(timer)
    }
  }, [])

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
