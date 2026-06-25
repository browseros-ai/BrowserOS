/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Static fixtures for the Phase 3 onboarding shell. Numbers exist so
 * the import-progress bar has something to climb toward and the
 * success summary has a concrete count; the wiring person replaces
 * these with real Chrome profile reads later.
 */

import type { ChromeProfileId } from './onboarding-v2.schemas'

export interface ChromeProfile {
  id: ChromeProfileId
  name: string
  email: string
  sites: number
  logins: number
}

export const CHROME_PROFILES: readonly ChromeProfile[] = [
  { id: 'work', name: 'Work', email: 'you@example.com', sites: 31, logins: 9 },
  {
    id: 'personal',
    name: 'Personal',
    email: 'you.personal@example.com',
    sites: 16,
    logins: 3,
  },
  {
    id: 'testing',
    name: 'Testing',
    email: 'qa@example.com',
    sites: 8,
    logins: 2,
  },
]

export function sumSitesFor(ids: readonly ChromeProfileId[]): number {
  return CHROME_PROFILES.filter((p) => ids.includes(p.id)).reduce(
    (sum, p) => sum + p.sites,
    0,
  )
}

export function sumLoginsFor(ids: readonly ChromeProfileId[]): number {
  return CHROME_PROFILES.filter((p) => ids.includes(p.id)).reduce(
    (sum, p) => sum + p.logins,
    0,
  )
}

export function profilesByIds(
  ids: readonly ChromeProfileId[],
): readonly ChromeProfile[] {
  return CHROME_PROFILES.filter((p) => ids.includes(p.id))
}

/**
 * Two starter prompts the Ready step suggests. Hard-coded for Phase 3;
 * the wiring person can swap this for a real source later.
 */
export const STARTER_PROMPTS: readonly string[] = [
  'Find me a coffee shop within walking distance and save it to my Maps.',
  'Apply for the SF visa for me, you have my passport scan in iCloud.',
]
