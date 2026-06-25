/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * System-wide default approval catalog. This is the source for
 * `GET /permissions/catalog` and the fallback `permissions.check()`
 * uses when neither a site rule nor an agent verdict applies.
 *
 * Keep in sync with the UI's local catalog at
 * apps/claw-app/screens/new-agent/new-agent.schemas.ts
 * (`APPROVAL_CATEGORIES`). The UI keeps its own copy as the
 * fetch-failure fallback path for the Permissions tab; this server
 * copy is the source of truth at the wire boundary.
 */

import { z } from 'zod'

export const approvalVerdictEnum = z.enum(['Auto', 'Ask', 'Block'])
export type ApprovalVerdict = z.infer<typeof approvalVerdictEnum>

export const approvalCategorySchema = z.object({
  id: z.string(),
  name: z.string(),
  defaultVerdict: approvalVerdictEnum,
  allowAuto: z.boolean(),
})
export type ApprovalCategory = z.infer<typeof approvalCategorySchema>

export const APPROVAL_CATEGORIES: readonly ApprovalCategory[] = [
  {
    id: 'submit',
    name: 'Submit / send / post',
    defaultVerdict: 'Ask',
    allowAuto: true,
  },
  {
    id: 'payment',
    name: 'Payments & checkout',
    defaultVerdict: 'Block',
    allowAuto: false,
  },
  {
    id: 'delete',
    name: 'Delete / destructive',
    defaultVerdict: 'Ask',
    allowAuto: true,
  },
  { id: 'upload', name: 'File upload', defaultVerdict: 'Ask', allowAuto: true },
  {
    id: 'navigate',
    name: 'Navigate to a new site',
    defaultVerdict: 'Ask',
    allowAuto: true,
  },
  {
    id: 'input',
    name: 'Click & type',
    defaultVerdict: 'Auto',
    allowAuto: true,
  },
]
