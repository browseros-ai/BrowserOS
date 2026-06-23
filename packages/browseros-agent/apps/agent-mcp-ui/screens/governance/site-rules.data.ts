import { useQueryClient } from '@tanstack/react-query'
import {
  type SiteRule,
  useAddSiteRule,
  useDeleteSiteRule,
  useSiteRules,
} from '@/modules/api/site-rules.hooks'

/**
 * Aggregates Site Rules server state. Add + delete mutations write
 * back to the `site-rules` cache so the list stays the source of
 * truth and re-renders without a refetch.
 */
export function useSiteRulesData() {
  const queryClient = useQueryClient()
  const { data: rules = [], isLoading } = useSiteRules()

  const addRule = useAddSiteRule({
    onSuccess: (rule) => {
      queryClient.setQueryData<SiteRule[]>(useSiteRules.getKey(), (prev) => [
        ...(prev ?? []),
        rule,
      ])
    },
  })

  const deleteRule = useDeleteSiteRule({
    onSuccess: ({ id }) => {
      queryClient.setQueryData<SiteRule[]>(useSiteRules.getKey(), (prev) =>
        (prev ?? []).filter((rule) => rule.id !== id),
      )
    },
  })

  return { rules, isLoading, addRule, deleteRule }
}
