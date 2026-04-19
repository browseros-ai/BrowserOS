/**
 * DesktopMouseService — native mouse control via @jitsi/robotjs.
 *
 * Provides mouse move, click, drag, scroll, and position query.
 * Falls back to a no-op stub when robotjs is unavailable.
 *
 * @module desktop-control/mouse
 */

import type {
  MouseClickOptions,
  MouseDragOptions,
  MouseMoveOptions,
  MouseScrollOptions,
  Point,
} from './types'

/** Lazy-loaded robotjs instance. */
let robot: typeof import('@jitsi/robotjs') | null = null

async function loadRobot() {
  if (robot !== undefined) return robot
  try {
    robot = await import('@jitsi/robotjs')
    return robot
  } catch {
    console.warn('[DesktopMouse] @jitsi/robotjs not available — mouse operations will be no-ops')
    robot = null
    return null
  }
}

/**
 * DesktopMouseService controls the native mouse cursor.
 * @public
 */
export class DesktopMouseService {
  /**
   * Move the mouse to the given screen coordinates.
   * Optionally smooth the movement over a duration.
   */
  async mouseMove(point: Point, options?: MouseMoveOptions): Promise<void> {
    const r = await loadRobot()
    if (!r) return

    if (options?.smoothDuration && options.smoothDuration > 0) {
      const current = r.getMousePos()
      const steps = Math.max(1, Math.ceil(options.smoothDuration / 2))
      const dx = point.x - current.x
      const dy = point.y - current.y
      const stepDelay = options.smoothDuration / steps

      for (let i = 1; i <= steps; i++) {
        const t = i / steps
        r.moveMouse(
          Math.round(current.x + dx * t),
          Math.round(current.y + dy * t),
        )
        if (i < steps) {
          await this.sleep(stepDelay)
        }
      }
    } else {
      r.moveMouse(point.x, point.y)
    }
  }

  /**
   * Click at the given position.
   * Moves first, then performs the click.
   */
  async mouseClick(point: Point, options?: MouseClickOptions): Promise<void> {
    const r = await loadRobot()
    if (!r) return

    await this.mouseMove(point)

    const button = this.mapButton(options?.button ?? 'left')
    const double = options?.clickType === 'double'

    if (double) {
      r.mouseClick(button, true)
    } else {
      r.mouseClick(button, false)
    }
  }

  /**
   * Drag from one point to another.
   * Moves to start, presses mouse down, slides to end, releases.
   */
  async mouseDrag(
    from: Point,
    to: Point,
    options?: MouseDragOptions,
  ): Promise<void> {
    const r = await loadRobot()
    if (!r) return

    const duration = options?.duration ?? 300
    const steps = options?.steps ?? 20
    const stepDelay = duration / steps

    // Move to start position
    r.moveMouse(from.x, from.y)

    // Press mouse down
    r.mouseToggle('down')

    try {
      // Smoothly move to destination
      for (let i = 1; i <= steps; i++) {
        const t = i / steps
        r.moveMouse(
          Math.round(from.x + (to.x - from.x) * t),
          Math.round(from.y + (to.y - from.y) * t),
        )
        if (i < steps) {
          await this.sleep(stepDelay)
        }
      }
    } finally {
      // Always release
      r.mouseToggle('up')
    }
  }

  /**
   * Scroll at a given position.
   */
  async mouseScroll(
    point: Point,
    options?: MouseScrollOptions,
  ): Promise<void> {
    const r = await loadRobot()
    if (!r) return

    await this.mouseMove(point)

    const amount = options?.amount ?? 3
    const direction = options?.direction ?? 'down'

    // robotjs uses positive = scroll up, negative = scroll down
    switch (direction) {
      case 'up':
        r.scrollMouse(0, amount)
        break
      case 'down':
        r.scrollMouse(0, -amount)
        break
      case 'left':
        r.scrollMouse(-amount, 0)
        break
      case 'right':
        r.scrollMouse(amount, 0)
        break
    }
  }

  /**
   * Get the current mouse position.
   */
  async getMousePosition(): Promise<Point> {
    const r = await loadRobot()
    if (!r) return { x: 0, y: 0 }

    const pos = r.getMousePos()
    return { x: pos.x, y: pos.y }
  }

  // ─── Helpers ───────────────────────────────────────────────────────

  private mapButton(
    button: 'left' | 'right' | 'middle',
  ): 'left' | 'right' | 'middle' {
    return button
  }

  private sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }
}
