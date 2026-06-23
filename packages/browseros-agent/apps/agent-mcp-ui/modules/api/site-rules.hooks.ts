import { createMutation, createQuery } from 'react-query-kit'
import { api } from './client'
import { parseResponse } from './parseResponse'

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

export const useSiteRules = createQuery<SiteRule[]>({
  queryKey: ['site-rules'],
  fetcher: async () => {
    const response = await api['site-rules'].$get()
    return parseResponse<SiteRule[]>(response)
  },
})

export interface AddSiteRuleVariables {
  label: string
  domain: string
  action: SiteRuleAction
}

export const useAddSiteRule = createMutation<SiteRule, AddSiteRuleVariables>({
  mutationFn: async (variables) => {
    const response = await api['site-rules'].$post({ json: variables })
    return parseResponse<SiteRule>(response)
  },
})

interface DeleteSiteRuleVariables {
  id: string
}

export const useDeleteSiteRule = createMutation<
  DeleteSiteRuleVariables,
  DeleteSiteRuleVariables
>({
  mutationFn: async ({ id }) => {
    const response = await api['site-rules'][':id'].$delete({ param: { id } })
    return parseResponse<DeleteSiteRuleVariables>(response)
  },
})
