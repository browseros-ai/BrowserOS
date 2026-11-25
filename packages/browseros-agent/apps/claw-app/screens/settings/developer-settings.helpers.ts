/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import {
  DISMISS_KEY as PRO_BAR_KEY,
  TRIAL_DISMISS_KEY,
} from '@/components/cockpit/ProBanner'
import { DISMISS_KEY as TRIAL_ENDING_BAR_KEY } from '@/components/cockpit/TrialEndingBar'
import { SEEN_PREFIX as SKILL_UPGRADE_SEEN } from '@/components/skills/NowUltrafast'
import { MARK_KEY as TRIAL_WATCH_KEY } from '@/modules/analytics/trial-outcome'

/** Whether this is a development build, the only kind that may show developer tools. */
export const DEVELOPMENT_BUILD = import.meta.env.MODE === 'development'

const PLAN_MEMORY = [
  PRO_BAR_KEY,
  TRIAL_DISMISS_KEY,
  TRIAL_ENDING_BAR_KEY,
  TRIAL_WATCH_KEY,
]

/**
 * Forgets what this browser remembers about the plan, so a computer taken off its plan sees the
 * free trial bar, tags and upgrade notes again, as a new computer would.
 */
export function forgetPlanMemory(storage: Storage = localStorage): void {
  try {
    const keys = Array.from({ length: storage.length }, (_, index) =>
      storage.key(index),
    )
    for (const key of keys) {
      if (
        key !== null &&
        (PLAN_MEMORY.includes(key) || key.startsWith(`${SKILL_UPGRADE_SEEN}:`))
      ) {
        storage.removeItem(key)
      }
    }
  } catch {
    // Without storage there is nothing remembered to forget.
  }
}
