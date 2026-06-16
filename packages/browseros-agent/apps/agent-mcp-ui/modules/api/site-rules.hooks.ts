import { nanoid } from 'nanoid'
import { createMutation, createQuery } from 'react-query-kit'

export type SiteRuleAction =
  | 'payments'
  | 'submit'
  | 'delete'
  | 'navigate'
  | 'upload'
  | 'admin'

export interface SiteRule {
  id: string
  /** Human label, e.g. "Wire transfers". */
  label: string
  /** Domain pattern, e.g. "stripe.com" or "admin.*". */
  domain: string
  /** Action category this rule clamps down on. */
  action: SiteRuleAction
}

const MOCK_SITE_RULES: SiteRule[] = [
  {
    id: 'rule-mercury',
    label: 'Wire transfers',
    domain: 'mercury.com',
    action: 'payments',
  },
  {
    id: 'rule-stripe-pay',
    label: 'Edit payment methods',
    domain: 'stripe.com',
    action: 'payments',
  },
  {
    id: 'rule-admin',
    label: 'Org billing settings',
    domain: 'admin.*',
    action: 'admin',
  },
  {
    id: 'rule-workspace',
    label: 'User management',
    domain: 'workspace.google.com',
    action: 'admin',
  },
  {
    id: 'rule-delete',
    label: 'Delete account',
    domain: '*',
    action: 'delete',
  },
]

export const useSiteRules = createQuery<SiteRule[]>({
  queryKey: ['site-rules'],
  fetcher: () =>
    new Promise((resolve) => setTimeout(() => resolve(MOCK_SITE_RULES), 60)),
})

export interface AddSiteRuleVariables {
  label: string
  domain: string
  action: SiteRuleAction
}

export const useAddSiteRule = createMutation<SiteRule, AddSiteRuleVariables>({
  mutationFn: async (variables) => {
    await new Promise((resolve) => setTimeout(resolve, 350))
    return { id: `rule-${nanoid(6).toLowerCase()}`, ...variables }
  },
})

interface DeleteSiteRuleVariables {
  id: string
}

export const useDeleteSiteRule = createMutation<
  DeleteSiteRuleVariables,
  DeleteSiteRuleVariables
>({
  mutationFn: async (variables) => {
    await new Promise((resolve) => setTimeout(resolve, 300))
    return variables
  },
})
