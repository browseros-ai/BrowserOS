/**
 * The drag handle between the runs and the sidebar. Dragging (or the arrow keys, when it has
 * focus) sets the sidebar's width; the width is remembered in this browser only.
 */

import { useCallback, useRef, useState } from 'react'

const STORAGE_KEY = 'neo-viewer.sidebar-width'
export const DEFAULT_SIDEBAR = 372
const MIN_SIDEBAR = 300
/** The runs keep at least this much room. */
const MIN_MAIN = 560
const KEY_STEP = 24

function clampWidth(width: number): number {
  const max = Math.max(MIN_SIDEBAR, window.innerWidth - MIN_MAIN)
  return Math.round(Math.min(Math.max(width, MIN_SIDEBAR), max))
}

function storedWidth(): number {
  try {
    const stored = Number(window.localStorage.getItem(STORAGE_KEY))
    return Number.isFinite(stored) && stored > 0 ? stored : DEFAULT_SIDEBAR
  } catch {
    return DEFAULT_SIDEBAR
  }
}

/** The sidebar's width and a setter that keeps it in bounds and remembers it. */
export function useSidebarWidth(): [number, (width: number) => void] {
  const [width, setWidth] = useState(storedWidth)
  const update = useCallback((next: number) => {
    const clamped = clampWidth(next)
    setWidth(clamped)
    try {
      window.localStorage.setItem(STORAGE_KEY, String(clamped))
    } catch {
      // Private windows and blocked storage: the width just isn't remembered.
    }
  }, [])
  return [width, update]
}

export function Splitter({
  width,
  onWidth,
}: {
  width: number
  onWidth: (width: number) => void
}) {
  const drag = useRef<{ x: number; width: number } | null>(null)
  return (
    // biome-ignore lint/a11y/useSemanticElements: a focusable window splitter (ARIA separator), not a rule.
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize the sidebar"
      aria-valuenow={width}
      tabIndex={0}
      onPointerDown={(event) => {
        event.currentTarget.setPointerCapture(event.pointerId)
        drag.current = { x: event.clientX, width }
      }}
      onPointerMove={(event) => {
        if (!drag.current) return
        // The sidebar is on the right: dragging left widens it.
        onWidth(drag.current.width - (event.clientX - drag.current.x))
      }}
      onPointerUp={() => {
        drag.current = null
      }}
      onPointerCancel={() => {
        drag.current = null
      }}
      onKeyDown={(event) => {
        if (event.key === 'ArrowLeft') onWidth(width + KEY_STEP)
        else if (event.key === 'ArrowRight') onWidth(width - KEY_STEP)
        else return
        event.preventDefault()
      }}
      className="relative flex w-[9px] shrink-0 cursor-col-resize touch-none select-none justify-center border-line border-x bg-panel focus-visible:outline-2 focus-visible:outline-neo"
    >
      <span className="block h-full w-0.5 bg-neo" />
      <span className="absolute top-1/2 left-0 flex h-9 w-[7px] -translate-y-1/2 flex-col items-center justify-center gap-[3px] rounded-[3px] bg-neo">
        <span className="block size-[3px] rounded-full bg-white" />
        <span className="block size-[3px] rounded-full bg-white" />
        <span className="block size-[3px] rounded-full bg-white" />
      </span>
    </div>
  )
}
