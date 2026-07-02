import { createQuery } from 'react-query-kit'
import type { RunStatus } from '@/lib/status'

export interface ActivityRow {
  id: string
  agentLabel: string
  /** Color the agent dot. Matches the per-agent color on AgentRow. */
  color: string
  status: Extract<
    RunStatus,
    'running' | 'blocked' | 'needs-human' | 'needs-ok' | 'done'
  > extends infer S
    ? S | 'allowed'
    : never
  action: string
  site?: string
  when: string
  /**
   * Run id used to route a done row's Replay link to
   * `/governance/audit/:runId/replay`. Only required on done rows.
   */
  runId?: string
  /** Total tool dispatches recorded against this tab. Surfaces as a badge. */
  toolCount?: number
  /** Short trail of recent tool names, e.g. `navigate -> snapshot -> act`. */
  trail?: string
}

/**
 * Mock cross-agent recent activity. Mirrors the design's ACTIVITY
 * array: blocked + needs-human + allowed + done rows so the cockpit
 * shows the full spectrum of status surfaces in one screenshot.
 */
const MOCK_ACTIVITY: ActivityRow[] = [
  {
    id: 'ac1',
    agentLabel: 'Codex . Book table',
    color: '#7A5AF8',
    status: 'needs-human',
    action: 'Hit a human check on Resy and handed control back to you',
    site: 'resy.com',
    when: '4 days ago',
  },
  {
    id: 'ac2',
    agentLabel: 'Cowork . Add to cart',
    color: '#0f3e17',
    status: 'blocked',
    action: 'Add-to-cart blocked by your ACL rule on this site',
    site: 'amazon.com',
    when: '2 days ago',
  },
  {
    id: 'ac3',
    agentLabel: 'Cowork . Recruiter reply',
    color: '#1FA463',
    status: 'allowed',
    action: 'You allowed once and the recruiter reply was sent',
    site: 'mail.google.com',
    when: '2 days ago',
  },
  {
    id: 'ac4',
    agentLabel: 'Codex . Log calls',
    color: '#2F6FE0',
    status: 'done',
    action: 'Exported Q2 pipeline, 42 deals, read-only',
    site: 'app.hubspot.com',
    when: '3 days ago',
    runId: 'run-concur-may',
  },
]

export const useRecentActivity = createQuery<ActivityRow[]>({
  queryKey: ['activity', 'recent'],
  fetcher: () =>
    new Promise((resolve) => setTimeout(() => resolve(MOCK_ACTIVITY), 60)),
})
