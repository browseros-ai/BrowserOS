import { createQuery } from 'react-query-kit'

export interface ApprovalItem {
  id: string
  agentLabel: string
  kind: 'submit' | 'navigate' | 'send' | 'delete'
  title: string
  detail: string
  domain: string
}

export interface HandoffItem {
  id: string
  agentLabel: string
  kind: 'captcha' | 'mfa' | 'security'
  title: string
  detail: string
  domain: string
}

/**
 * Mock pending approvals + handoffs that need user attention. The real
 * backend will derive these from the active turn registry; this stub
 * lets the WaitingStrip render the live shape without the backend.
 */
const MOCK_APPROVALS: ApprovalItem[] = [
  {
    id: 'apv-1',
    agentLabel: 'Cowork . File expenses',
    kind: 'submit',
    title: 'Claude wants to submit this report',
    detail: 'Submit "May 2026 Engineering" for $1,284.50 across 4 line items',
    domain: 'concur.com',
  },
]

const MOCK_HANDOFFS: HandoffItem[] = [
  {
    id: 'hdf-1',
    agentLabel: 'Codex . Book table',
    kind: 'captcha',
    title: 'Resy wants to confirm it is you',
    detail: 'Finish the human check, then the agent will keep going',
    domain: 'resy.com',
  },
]

export const useApprovals = createQuery<ApprovalItem[]>({
  queryKey: ['approvals'],
  fetcher: () =>
    new Promise((resolve) => setTimeout(() => resolve(MOCK_APPROVALS), 60)),
})

export const useHandoffs = createQuery<HandoffItem[]>({
  queryKey: ['handoffs'],
  fetcher: () =>
    new Promise((resolve) => setTimeout(() => resolve(MOCK_HANDOFFS), 60)),
})
