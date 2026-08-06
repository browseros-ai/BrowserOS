import { useCallback, useEffect, useRef, useState } from 'react'
import { Outlet } from 'react-router'
import { AppSidebar } from '@/components/sidebar/AppSidebar'

const COLLAPSE_DELAY = 150

export function CockpitShell() {
  const [expanded, setExpanded] = useState(false)
  const collapseTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const handleMouseEnter = useCallback(() => {
    if (collapseTimeoutRef.current) {
      clearTimeout(collapseTimeoutRef.current)
      collapseTimeoutRef.current = null
    }
    setExpanded(true)
  }, [])

  const handleMouseLeave = useCallback(() => {
    collapseTimeoutRef.current = setTimeout(() => {
      setExpanded(false)
    }, COLLAPSE_DELAY)
  }, [])

  useEffect(() => {
    // Snapshot the ref object so the unmount cleanup closes over a stable
    // reference (the React docs' canonical pattern for refs in effects).
    const timeoutRef = collapseTimeoutRef
    return () => {
      const id = timeoutRef.current
      if (id !== null) clearTimeout(id)
    }
  }, [])

  return (
    <div className="relative min-h-screen pl-14">
      {/* biome-ignore lint/a11y/noStaticElementInteractions: hover-driven sidebar expansion */}
      <div
        className="fixed inset-y-0 left-0 z-40"
        onMouseEnter={handleMouseEnter}
        onMouseLeave={handleMouseLeave}
      >
        <AppSidebar expanded={expanded} />
      </div>
      <main className="min-h-screen overflow-y-auto">
        <Outlet />
      </main>
    </div>
  )
}
