/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { z } from 'zod'

export const credentialSchema = z.object({
  credential: z
    .string()
    .trim()
    .min(1, 'Paste the key from your provider.'),
})

export const budgetsSchema = z.object({
  maxSteps: z.coerce
    .number()
    .int('Whole steps only.')
    .min(1, 'At least one step.')
    .max(200, 'At most 200 steps.'),
  maxSeconds: z.coerce
    .number()
    .int('Whole seconds only.')
    .min(5, 'At least 5 seconds.')
    .max(600, 'At most 600 seconds.'),
})

export type CredentialInput = z.infer<typeof credentialSchema>
export type BudgetsInput = z.infer<typeof budgetsSchema>
