/**
 * The container line's state, kept current. `/api/info` checks the container (`docker inspect`)
 * on every request, but the page reads it once at load; this asks again every few seconds while
 * the viewer has a container to report, so one that stops or comes back shows without a reload.
 */

import { useEffect, useState } from 'react'
import type { ContainerInfo, ViewerInfo } from '../../shared/api'
import { apiRoutes } from '../../shared/api'

export const CONTAINER_POLL_MS = 5_000

export function useContainer(
  info: ViewerInfo | null,
  everyMs = CONTAINER_POLL_MS,
): ContainerInfo | null {
  const [current, setCurrent] = useState<ContainerInfo | null>(null)
  const watched = info?.container != null
  useEffect(() => {
    if (!watched) return
    let live = true
    const ask = async () => {
      try {
        const response = await fetch(apiRoutes.info)
        if (!response.ok) return
        const latest = (await response.json()) as ViewerInfo
        if (live) setCurrent(latest.container)
      } catch {
        // The viewer itself went away: keep the last answer; the page shows the error elsewhere.
      }
    }
    const timer = setInterval(() => void ask(), everyMs)
    return () => {
      live = false
      clearInterval(timer)
    }
  }, [watched, everyMs])
  return watched ? (current ?? info?.container ?? null) : null
}
