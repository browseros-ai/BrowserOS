import { useQueryClient } from '@tanstack/react-query'
import {
  type Grant,
  useGrants,
  useRevokeGrant,
} from '@/modules/api/grants.hooks'

/**
 * Aggregates the Grants ledger's server state. Revoke writes back
 * to the `grants` cache so the row disappears immediately.
 */
export function useGrantsData() {
  const queryClient = useQueryClient()
  const { data: grants = [], isLoading } = useGrants()

  const revokeGrant = useRevokeGrant({
    onSuccess: ({ id }) => {
      queryClient.setQueryData<Grant[]>(useGrants.getKey(), (prev) =>
        (prev ?? []).filter((grant) => grant.id !== id),
      )
    },
  })

  return { grants, isLoading, revokeGrant }
}
