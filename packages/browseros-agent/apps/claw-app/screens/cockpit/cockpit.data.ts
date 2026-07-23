import { useEffect, useRef } from 'react'
import { useLiveSessions } from '@/modules/api/audit.hooks'
import {
  type LiveSessionCardRecord,
  sessionsToLiveCards,
} from './cockpit.helpers'

export interface CockpitData {
  sessions: LiveSessionCardRecord[]
  isPending: boolean
}

/** Builds Running now exclusively from the complete live-session snapshot. */
export function useCockpitData(): CockpitData {
  const liveSessions = useLiveSessions()
  const selectedTabBySessionRef = useRef<Map<string, number>>(new Map())
  const sessions = sessionsToLiveCards(liveSessions.data?.items ?? [], {
    stickySelection: selectedTabBySessionRef.current,
  })

  useEffect(() => {
    selectedTabBySessionRef.current = new Map(
      sessions.flatMap((session) =>
        session.selectedTab
          ? [[session.sessionId, session.selectedTab.browserTabId] as const]
          : [],
      ),
    )
  }, [sessions])

  return { sessions, isPending: liveSessions.isPending }
}
