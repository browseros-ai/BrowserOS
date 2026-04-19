/**
 * DesktopKeyboardService — native keyboard control via @jitsi/robotjs.
 *
 * Provides text typing with configurable delay and key press with
 * modifier support (e.g. Ctrl+C, Alt+Tab).
 *
 * @module desktop-control/keyboard
 */

import type { KeyCombination, KeyModifier, TypeTextOptions } from './types'

/** Lazy-loaded robotjs instance. */
let robot: typeof import('@jitsi/robotjs') | null = null

async function loadRobot() {
  if (robot !== undefined) return robot
  try {
    robot = await import('@jitsi/robotjs')
    return robot
  } catch {
    robot = null
    return null
  }
}

/**
 * Map from our KeyModifier enum to robotjs key string.
 */
const MODIFIER_MAP: Record<KeyModifier, string> = {
  alt: 'alt',
  control: 'control',
  shift: 'shift',
  meta: 'command',
}

/**
 * Common key name mappings from human-readable to robotjs key names.
 */
const KEY_MAP: Record<string, string> = {
  enter: 'enter',
  return: 'enter',
  escape: 'escape',
  esc: 'escape',
  tab: 'tab',
  backspace: 'backspace',
  delete: 'delete',
  space: 'space',
  up: 'up',
  down: 'down',
  left: 'left',
  right: 'right',
  home: 'home',
  end: 'end',
  pageup: 'pageup',
  pagedown: 'pagedown',
  capslock: 'caps_lock',
  f1: 'f1',
  f2: 'f2',
  f3: 'f3',
  f4: 'f4',
  f5: 'f5',
  f6: 'f6',
  f7: 'f7',
  f8: 'f8',
  f9: 'f9',
  f10: 'f10',
  f11: 'f11',
  f12: 'f12',
}

/**
 * DesktopKeyboardService controls the native keyboard.
 * @public
 */
export class DesktopKeyboardService {
  /**
   * Type a string of text character by character.
   * Uses configurable delay between keystrokes for reliability.
   */
  async typeText(text: string, options?: TypeTextOptions): Promise<void> {
    const r = await loadRobot()
    if (!r) return

    const keyDelay = options?.keyDelay ?? 10

    if (keyDelay <= 0) {
      // Fast path: type the entire string at once
      r.typeString(text)
      return
    }

    // Character-by-character with delay
    for (const char of text) {
      r.typeString(char)
      await this.sleep(keyDelay)
    }
  }

  /**
   * Press a key, optionally with modifiers held.
   *
   * Examples:
   *   pressKey({ key: 'c', modifiers: ['control'] })  → Ctrl+C
   *   pressKey({ key: 'tab', modifiers: ['alt'] })     → Alt+Tab
   *   pressKey({ key: 'enter' })                       → Enter
   */
  async pressKey(combination: KeyCombination): Promise<void> {
    const r = await loadRobot()
    if (!r) return

    const robotKey = this.resolveKeyName(combination.key)
    const modifiers = (combination.modifiers ?? []).map((m) => MODIFIER_MAP[m])

    r.keyTap(robotKey, modifiers)
  }

  // ─── Helpers ───────────────────────────────────────────────────────

  /**
   * Resolve a human key name to a robotjs key name.
   */
  private resolveKeyName(key: string): string {
    const lower = key.toLowerCase()
    return KEY_MAP[lower] ?? key
  }

  private sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }
}
