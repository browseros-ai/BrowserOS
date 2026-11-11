/**
 * How tall the docked screen is. Its width is the sidebar's, which the owner drags; its height
 * follows at Neo's own screen ratio, so the picture always fills the width and widening the
 * sidebar zooms in. Only when that would be taller than the sidebar can show is the height
 * capped, and then the picture letterboxes (noVNC scales to fit the box).
 *
 *   width ──▶ height = ⌈width ÷ ratio⌉ ──▶ capped at the sidebar's visible height less the
 *             panel's header row (the whole picture stays on screen)
 */

import { type RefObject, useEffect, useLayoutEffect, useState } from 'react'
import type { ScreenSize } from '../../shared/live-screen'

/**
 * Neo's screen before its server says otherwise: the image's default `NEO_SCREEN`, 1600×1000. A
 * box sized with it before the first frame does not jump when the real ratio arrives.
 */
export const DEFAULT_SCREEN_RATIO = 1600 / 1000

/** Below this the picture is too small to read; a very short window scrolls instead. */
export const MIN_SCREEN_HEIGHT = 120

/** Width over height of Neo's screen, from the framebuffer size its RFB server announced. */
export function screenRatio(screen: ScreenSize | null): number {
  return screen && screen.width > 0 && screen.height > 0
    ? screen.width / screen.height
    : DEFAULT_SCREEN_RATIO
}

export interface ScreenFit {
  height: number
  /** True when the cap applied: the picture is narrower than the box (bars at the sides). */
  capped: boolean
}

/**
 * The height for a screen box `width` wide. Rounded up, so the picture's scale is set by the
 * width and fills it with no side bars (at most a sub-pixel band below). `maxHeight` null: no cap.
 */
export function fitScreen(
  width: number,
  ratio: number,
  maxHeight: number | null,
): ScreenFit {
  const natural = Math.ceil(width / ratio)
  if (maxHeight === null || natural <= maxHeight)
    return { height: natural, capped: false }
  return {
    height: Math.max(MIN_SCREEN_HEIGHT, Math.floor(maxHeight)),
    capped: true,
  }
}

/**
 * The fitted height of the element `box` refers to, kept current as its width changes (the
 * splitter drag) and as `viewHeight` changes (the window). `viewHeight` is the visible height
 * the whole panel sits in; `reserved` is what the panel needs above and around the screen.
 * Null until the box has been measured once.
 */
export function useScreenFit(
  box: RefObject<HTMLElement | null>,
  ratio: number,
  viewHeight: number | null,
  reserved: () => number,
): ScreenFit | null {
  const [width, setWidth] = useState<number | null>(null)
  const windowHeight = useWindowHeight(viewHeight === null)
  // Before paint, so the box never shows at the wrong height.
  useLayoutEffect(() => {
    const element = box.current
    if (!element) return
    setWidth(element.clientWidth)
    const observer = new ResizeObserver(([entry]) => {
      if (entry) setWidth(entry.contentRect.width)
    })
    observer.observe(element)
    return () => observer.disconnect()
  }, [box])
  if (width === null) return null
  const visible = viewHeight ?? windowHeight
  return fitScreen(width, ratio, visible === null ? null : visible - reserved())
}

function useWindowHeight(wanted: boolean): number | null {
  const [height, setHeight] = useState<number | null>(null)
  useEffect(() => {
    if (!wanted) return
    const update = () => setHeight(window.innerHeight)
    update()
    window.addEventListener('resize', update)
    return () => window.removeEventListener('resize', update)
  }, [wanted])
  return wanted ? height : null
}
