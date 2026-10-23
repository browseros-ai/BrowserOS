/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { motion, useReducedMotion } from 'motion/react'
import { useEffect } from 'react'
import { Outlet, useLocation } from 'react-router'
import {
  PAGE_ENTER_DURATION_S,
  PAGE_ENTER_EASE,
  PAGE_ENTER_OFFSET_PX,
} from './layout.constants'

/**
 * Identity of a page for the enter animation. A task and its replay deep link
 * are the same page, and search params (filters) never change the page.
 */
export function pageKeyFor(pathname: string): string {
  return pathname.replace(/\/replay\/?$/, '')
}

/**
 * Renders the routed page with the prototype's enter transition: a short fade
 * and 8px rise on page change only. There is no exit animation, so navigation
 * never waits, and the sidebar outside this wrapper never re-mounts.
 */
export function PageTransition() {
  const { pathname } = useLocation()
  const reduce = useReducedMotion() ?? false
  const pageKey = pageKeyFor(pathname)

  // Scroll position is browser state outside React; a new page starts at the
  // top. The replay deep link keeps its own scroll target.
  useEffect(() => {
    if (pageKey === pathname) window.scrollTo(0, 0)
  }, [pageKey, pathname])

  return (
    <motion.div
      key={pageKey}
      data-page-key={pageKey}
      initial={{ opacity: 0, y: reduce ? 0 : PAGE_ENTER_OFFSET_PX }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: PAGE_ENTER_DURATION_S, ease: PAGE_ENTER_EASE }}
      className="flex min-w-0 flex-1 flex-col"
    >
      <Outlet />
    </motion.div>
  )
}
