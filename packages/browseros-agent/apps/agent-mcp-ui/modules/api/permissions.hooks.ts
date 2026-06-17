import { createQuery } from 'react-query-kit'
import {
  APPROVAL_CATEGORIES,
  type ApprovalCategory,
} from '@/screens/new-agent/new-agent.schemas'
import { api } from './client'
import { parseResponse } from './parseResponse'

export type { ApprovalCategory } from '@/screens/new-agent/new-agent.schemas'

/**
 * System-wide approval catalog. The backend ships the source of truth
 * baked into `lib/approval-catalog.ts`; the local constant stays as a
 * silent fallback so the Permissions tab still renders if the cockpit
 * lost its connection to `agent-mcp-interface`.
 */
export const useApprovalCatalog = createQuery<readonly ApprovalCategory[]>({
  queryKey: ['permissions', 'catalog'],
  fetcher: async () => {
    try {
      const response = await api.permissions.catalog.$get()
      return await parseResponse<ApprovalCategory[]>(response)
    } catch {
      return APPROVAL_CATEGORIES
    }
  },
})
