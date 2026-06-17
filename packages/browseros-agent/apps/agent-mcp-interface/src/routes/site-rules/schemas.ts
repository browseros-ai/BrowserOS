/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Zod shapes for the /site-rules routes. Wire shape mirrors
 * apps/agent-mcp-ui/modules/api/site-rules.hooks.ts so the UI's
 * existing `SiteRule` / `AddSiteRuleVariables` consumers stay byte
 * identical after the hook swap.
 *
 * Storage shape is a single file holding an array; site-rule lookups
 * are always full-table scans (typical user has under 20 rules) and
 * Phase 5's `permissions.check` reads the whole set on every dispatch.
 */

import { z } from 'zod'

export const siteRuleActionEnum = z.enum([
  'payments',
  'submit',
  'delete',
  'navigate',
  'upload',
  'admin',
])
export type SiteRuleAction = z.infer<typeof siteRuleActionEnum>

/** Wire shape: POST body. */
export const addSiteRuleSchema = z.object({
  label: z.string().trim().min(1),
  domain: z.string().trim().min(1),
  action: siteRuleActionEnum,
})
export type AddSiteRuleVariables = z.infer<typeof addSiteRuleSchema>

/** Wire shape: GET / and POST / response item. Also the on-disk row shape. */
export const siteRuleSchema = z.object({
  id: z.string(),
  label: z.string().min(1),
  domain: z.string().min(1),
  action: siteRuleActionEnum,
})
export type SiteRule = z.infer<typeof siteRuleSchema>

/** Storage wrapper: site-rules.json holds an array. */
export const siteRulesFileSchema = z.array(siteRuleSchema)
export type SiteRulesFile = z.infer<typeof siteRulesFileSchema>

/** Wire shape: DELETE response. */
export const idAckSchema = z.object({ id: z.string() })
export type IdAck = z.infer<typeof idAckSchema>
