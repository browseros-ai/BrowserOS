/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The five states the page can be in, derived in one place so the copy and the
 * controls can never disagree about which one is showing.
 */

import type { JevModeState } from '@browseros/claw-api'

export type FastModeStatus = 'off' | 'on' | 'paused'

export function statusOf(state: JevModeState | undefined): FastModeStatus {
  if (!state?.configured) return 'off'
  return state.paused ? 'paused' : 'on'
}

/**
 * What agents see, which is the only thing that matters about this screen.
 * Phrased as the consequence rather than the setting.
 */
export function agentVisibility(status: FastModeStatus): string {
  switch (status) {
    case 'on':
      return 'Connected agents can use it. Every other tool is unchanged.'
    case 'paused':
      return 'Hidden from agents. Your key is kept, so you can switch it back on.'
    case 'off':
      return 'Nothing changes until you add a key. Agents see exactly what they see today.'
  }
}
