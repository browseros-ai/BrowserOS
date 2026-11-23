/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * What this computer's plan unlocks, read from its features. No React, no side effects.
 */

import type { Plan } from '@browseros/claw-api'

export const ULTRAFAST_FEATURE = 'claw.ultrafast'

/** Whether the plan includes ultrafast. */
export function hasUltrafast(
  plan: Pick<Plan, 'features'> | undefined,
): boolean {
  return plan?.features.includes(ULTRAFAST_FEATURE) === true
}

/**
 * Whether to tell this person that ultrafast runs their skills in seconds: only where ultrafast is
 * on sale and the plan does not have it yet, so the link never leads to a plan that cannot be bought.
 */
export function showsUltrafastNudge(
  plan: Pick<Plan, 'available' | 'features'> | undefined,
): boolean {
  return plan?.available === true && !hasUltrafast(plan)
}
