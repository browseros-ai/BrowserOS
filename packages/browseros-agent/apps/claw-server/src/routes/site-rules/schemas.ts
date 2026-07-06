/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Zod shapes for stored site rules. The live MCP permission chain
 * reads these rules directly; no site-rules HTTP route is mounted.
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

/** Input accepted by the site-rule service. */
export const addSiteRuleSchema = z.object({
  label: z.string().trim().min(1),
  domain: z.string().trim().min(1),
  action: siteRuleActionEnum,
})
export type AddSiteRuleVariables = z.infer<typeof addSiteRuleSchema>

/** On-disk site-rule row shape. */
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

/** Acknowledges a service mutation by id. */
export const idAckSchema = z.object({ id: z.string() })
export type IdAck = z.infer<typeof idAckSchema>
