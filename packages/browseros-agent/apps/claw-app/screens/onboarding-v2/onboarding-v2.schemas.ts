/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Form schema for the v2 onboarding flow. Phase 3 wires only the
 * profile picker through react-hook-form; every other "input" in the
 * flow is a stateless local-state advancer. When the wiring person
 * replaces the fake handlers, they read `form.getValues()` here and
 * post to the real import service.
 */

import { z } from 'zod'

export const CHROME_PROFILE_IDS = ['work', 'personal', 'testing'] as const
export type ChromeProfileId = (typeof CHROME_PROFILE_IDS)[number]

export const chromeProfileIdEnum = z.enum(CHROME_PROFILE_IDS)

export const onboardingFormSchema = z.object({
  selectedProfileIds: z
    .array(chromeProfileIdEnum)
    .min(1, 'Pick at least one profile to import.'),
})

export type OnboardingFormValues = z.infer<typeof onboardingFormSchema>

export const onboardingFormDefaults: OnboardingFormValues = {
  selectedProfileIds: ['work', 'personal'],
}
