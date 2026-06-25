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
 * lost its connection to `claw-server`.
 */
export const useApprovalCatalog = createQuery<readonly ApprovalCategory[]>({
  queryKey: ['permissions', 'catalog'],
  fetcher: async () => {
    try {
      const response = await api.permissions.catalog.$get()
      return await parseResponse<ApprovalCategory[]>(response)
    } catch (err) {
      // Surface a single line so operators can tell "intentional
      // offline fallback" apart from "the backend changed schema
      // under us". Returning the local constant keeps the tab
      // usable; the warn is the diagnostic seam.
      console.warn(
        '[permissions.useApprovalCatalog] backend catalog fetch failed, using local fallback',
        err,
      )
      return APPROVAL_CATEGORIES
    }
  },
})
