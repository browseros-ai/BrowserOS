import { z } from 'zod'
import type { SiteRuleAction } from '@/modules/api/site-rules.hooks'
import { normalizeDomainInput } from '@/screens/new-agent/new-agent.helpers'

export const SITE_RULE_ACTIONS = [
  'payments',
  'submit',
  'delete',
  'navigate',
  'upload',
  'admin',
] as const satisfies readonly SiteRuleAction[]

const ACTION_LABEL: Record<SiteRuleAction, string> = {
  payments: 'Payments',
  submit: 'Submit / send',
  delete: 'Delete',
  navigate: 'Navigate',
  upload: 'File upload',
  admin: 'Admin / settings',
}

const ACTION_BADGE_CLASS: Record<SiteRuleAction, string> = {
  payments: 'bg-red-tint text-red',
  submit: 'bg-amber-tint text-amber',
  delete: 'bg-red-tint text-red',
  navigate: 'bg-bg-sunken text-ink-2',
  upload: 'bg-amber-tint text-amber',
  admin: 'bg-red-tint text-red',
}

export function siteRuleActionLabel(action: SiteRuleAction): string {
  return ACTION_LABEL[action]
}

export function siteRuleActionBadgeClass(action: SiteRuleAction): string {
  return ACTION_BADGE_CLASS[action]
}

export const addSiteRuleSchema = z.object({
  label: z.string().trim().min(1, 'Give the rule a label'),
  domain: z
    .string()
    .trim()
    .min(1, 'Enter a domain')
    .transform(normalizeDomainInput),
  action: z.enum(SITE_RULE_ACTIONS),
})

export type AddSiteRuleValues = z.infer<typeof addSiteRuleSchema>

export const addSiteRuleDefaults: AddSiteRuleValues = {
  label: '',
  domain: '',
  action: 'payments',
}
