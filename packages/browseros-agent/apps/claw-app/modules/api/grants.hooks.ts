import { createMutation, createQuery } from 'react-query-kit'

export interface Grant {
  id: string
  /** Action that was always-allowed, e.g. "Submit", "Navigate". */
  action: string
  /** Domain the grant applies to. */
  domain: string
  /** Relative time the grant was issued. */
  when: string
  /** Originating agent label, e.g. "Cowork . Finance ops". */
  grantedTo: string
  /** Optional context blurb, e.g. "May expense report". */
  note?: string
}

const MOCK_GRANTS: Grant[] = [
  {
    id: 'grant-concur-submit',
    action: 'Submit report',
    domain: 'concur.com',
    when: '2m ago',
    grantedTo: 'Cowork . Finance ops',
    note: 'May expense report',
  },
  {
    id: 'grant-concur-attach',
    action: 'Attach receipts',
    domain: 'concur.com',
    when: '5m ago',
    grantedTo: 'Cowork . Finance ops',
  },
  {
    id: 'grant-linkedin-post',
    action: 'Submit post',
    domain: 'linkedin.com',
    when: '4m ago',
    grantedTo: 'Cowork . Social posts',
    note: 'Launch announcement',
  },
  {
    id: 'grant-sheet-paste',
    action: 'Paste row',
    domain: 'docs.google.com',
    when: '12m ago',
    grantedTo: 'Codex . Pricing research',
  },
  {
    id: 'grant-hubspot-export',
    action: 'Export leads',
    domain: 'app.hubspot.com',
    when: 'Yesterday 17:42',
    grantedTo: 'Codex . Pipeline digest',
  },
]

export const useGrants = createQuery<Grant[]>({
  queryKey: ['grants'],
  fetcher: () =>
    new Promise((resolve) => setTimeout(() => resolve(MOCK_GRANTS), 60)),
})

interface RevokeGrantVariables {
  id: string
}

/**
 * Mock revoke mutation. Real surface clears the agent's
 * always-allow row server-side and a side-effect should also nuke
 * the matching row from the originating agent profile. The mock just
 * echoes back the id so the client can drop it from the cache.
 */
export const useRevokeGrant = createMutation<
  RevokeGrantVariables,
  RevokeGrantVariables
>({
  mutationFn: async (variables) => {
    await new Promise((resolve) => setTimeout(resolve, 350))
    return variables
  },
})
